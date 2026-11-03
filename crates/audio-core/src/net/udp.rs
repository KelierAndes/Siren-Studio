//! UDP audio transport.
//!
//! One sender = one stream id. The receiver demultiplexes by
//! `(source address, stream id)`, creates an independent lock-free ring plus
//! drift-correcting channel per sender, conceals Opus loss, and retires
//! streams that have been silent for a few seconds.

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rtrb::Producer;

use super::codec::{AudioDecoder, AudioEncoder};
use crate::dsp::spectrum::SpectrumAnalyzer;
use crate::dsp::{MixerControl, StreamChannel};
use crate::format::{frames_for_ms, CHANNELS, SAMPLE_RATE};
use crate::metrics::{linear_to_db, now_us, store_f32, MetricsRegistry, StreamMetrics};
use crate::protocol::{Codec, FrameHeader};
use crate::{CoreError, Result};

pub(crate) const PEER_RING_SAMPLES: usize = SAMPLE_RATE as usize * CHANNELS; // one second
const MAX_PLC_PACKETS: u32 = 6;
const MAX_GAP_FILL_MS: f32 = 100.0;
/// Frozen streams (timed out but kept for a fast resume) are evicted after
/// this much total silence.
const FREEZE_GRACE: Duration = Duration::from_secs(30);
const PEER_TIMEOUT: Duration = Duration::from_secs(3);
const BT_PEER_TIMEOUT: Duration = Duration::from_secs(8);
/// RFCOMM has bursts and long stalls; a Bluetooth-only stream gets a wider
/// jitter buffer than the LAN default.
const BT_STREAM_TARGET_MS: f32 = 250.0;
/// Upper bound for the adaptive Bluetooth buffer. Measured: the link itself is
/// clean (p99 100 ms, no stall over 500 ms) and a 250 ms target cut latency but
/// doubled the underruns, because every overflow trim starves the player right
/// after. The loop only raises this for starvation that is *not* trim-correlated
/// (see the guard in the receiver), so the deep setting is reserved for real RF.
const BT_MAX_TARGET_MS: f32 = 400.0;
/// 10 ms of stereo audio at 48 kHz, the window a hub keeps per origin for
/// building N-1 mixes.
const MIX_TAP_SAMPLES: usize = 960;
/// While individual Wi-Fi streams are this fresh, a Bluetooth hub mix is
/// dropped (Wi-Fi wins).
const MIX_YIELD_TO_WIFI: Duration = Duration::from_millis(500);
/// A hub mix counts as active while a packet arrived this recently.
const MIX_ACTIVE: Duration = Duration::from_millis(600);

/// Whether any origin inside `mask` lacks a fresh direct copy.
fn mask_needs(mask: u64, known: &HashMap<u64, (Instant, bool)>, now: Instant) -> bool {
    if mask == 0 {
        return false;
    }
    let mut bits = mask;
    while bits != 0 {
        let bit = bits.trailing_zeros();
        bits &= bits - 1;
        let want = 1u64 << bit;
        let fresh = known.iter().any(|(origin, (at, wifi))| {
            *wifi && (1u64 << (origin % 64)) == want && now.duration_since(*at) < NORMAL_FRESH
        });
        if !fresh {
            return true;
        }
    }
    false
}
/// An individual stream counts as live for mix arbitration while it is this
/// fresh.
const NORMAL_FRESH: Duration = Duration::from_millis(400);
/// How long an origin stays part of the expected set after its last packet.
const KNOWN_NORMAL_TTL: Duration = Duration::from_secs(60);
/// A stream silent for longer than this is treated as stalled when it comes
/// back with a large sequence jump.
const CARRIER_STALL: Duration = Duration::from_millis(200);
/// Longest gap between accepted packets worth reporting: below this the hole is
/// smaller than one jitter buffer and is not heard as a mute.
const AUDIBLE_MUTE: Duration = Duration::from_millis(500);
const BT_TARGET_STREAK: u32 = 40;

pub struct UdpSender {
    socket: UdpSocket,
    target: SocketAddr,
    packer: FramePacker,
    bytes_sent: u64,
    packets_sent: u64,
    fail_streak: u32,
    dead_until: Option<Instant>,
    last_fail_log: Option<Instant>,
    // Heartbeat (F10): request every HB_TICKS ticks, echo comes back from
    // the peer's bind socket into this connected socket.
    hb_window: std::collections::VecDeque<u64>,
    hb_pending: Option<u64>,
    hb_sent: u64,
    hb_lost: u64,
}

/// One shared encoder/sequence per node. Dual-stack sends the *same* encoded
/// datagram over Wi-Fi and Bluetooth so the receiver can identify both copies
/// by `(origin, stream_id, seq)` and keep whichever arrives first.
pub struct FramePacker {
    stream_id: u16,
    seq: u32,
    frame_ms: u16,
    codec: Codec,
    origin: u64,
    encoder: AudioEncoder,
    payload: Vec<u8>,
    packet: Vec<u8>,
}

impl FramePacker {
    pub fn new(codec: Codec, frame_ms: u16, origin: u64) -> Result<Self> {
        Self::with_stream(codec, frame_ms, origin, stream_id_from_clock(), None)
    }

    /// Secondary carrier packer: shares the primary `stream_id` (so the
    /// receiver treats both as one logical stream) but encodes independently
    /// — the Bluetooth carrier uses a low-bitrate Opus to fit narrow links.
    pub fn with_stream(
        codec: Codec,
        frame_ms: u16,
        origin: u64,
        stream_id: u16,
        bitrate: Option<i32>,
    ) -> Result<Self> {
        Ok(Self {
            stream_id,
            seq: 0,
            frame_ms,
            codec,
            origin,
            encoder: AudioEncoder::with_bitrate(codec, bitrate)?,
            payload: Vec::with_capacity(2048),
            packet: Vec::with_capacity(4096),
        })
    }

    pub fn stream_id(&self) -> u16 {
        self.stream_id
    }

    /// Encode one interleaved stereo frame; the returned datagram stays valid
    /// until the next `build` call.
    pub fn build(&mut self, pcm: &[f32]) -> Result<&[u8]> {
        self.build_with(pcm, false)
    }

    /// Bluetooth framing: 20 B header instead of 28 B (no timestamp).
    pub fn build_compact(&mut self, pcm: &[f32]) -> Result<&[u8]> {
        self.build_with(pcm, true)
    }

    /// Build a hub N-1 mix frame (v4) that declares which origins it holds.
    pub fn build_mix(&mut self, pcm: &[f32], mask: u64) -> Result<&[u8]> {
        self.build_parts(pcm, false, None, Some(mask))
    }

    /// Build the secondary-carrier datagram with the primary carrier's
    /// sequence number. Both carriers must share one sequence timeline:
    /// a secondary counter that drifts behind makes the receiver reject
    /// every Bluetooth copy as a late duplicate, and after a Wi-Fi loss it
    /// stays silent until that drift has been consumed.
    pub fn build_compact_with_seq(&mut self, pcm: &[f32], seq: u32) -> Result<&[u8]> {
        self.build_parts(pcm, true, Some(seq), None)
    }

    fn build_with(&mut self, pcm: &[f32], compact: bool) -> Result<&[u8]> {
        self.build_parts(pcm, compact, None, None)
    }

    fn build_parts(
        &mut self,
        pcm: &[f32],
        compact: bool,
        seq_override: Option<u32>,
        mix_mask: Option<u64>,
    ) -> Result<&[u8]> {
        let seq = seq_override.unwrap_or(self.seq);
        let header = FrameHeader::new(
            self.codec,
            self.frame_ms,
            self.stream_id,
            seq,
            now_us(),
            self.origin,
        );
        self.packet.clear();
        if let Some(mask) = mix_mask {
            self.packet.extend_from_slice(&header.encode_mix(mask));
        } else if compact {
            self.packet.extend_from_slice(&header.encode_compact());
        } else {
            self.packet.extend_from_slice(&header.encode());
        }
        let written = self.encoder.encode(pcm, &mut self.payload)?;
        self.packet.extend_from_slice(&self.payload[..written]);
        if seq_override.is_none() {
            self.seq = self.seq.wrapping_add(1);
        }
        Ok(&self.packet)
    }

    /// Sequence number the next primary frame will carry.
    pub fn next_seq(&self) -> u32 {
        self.seq
    }
}

impl UdpSender {
    pub fn new(target: SocketAddr, codec: Codec, frame_ms: u16, origin: u64) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_write_timeout(Some(Duration::from_millis(200)))?;
        let socket_target = if target.ip().is_unspecified() {
            SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                target.port(),
            )
        } else {
            target
        };
        socket.connect(socket_target)?;
        socket.set_nonblocking(true).ok();
        Ok(Self {
            socket,
            target: socket_target,
            packer: FramePacker::new(codec, frame_ms, origin)?,
            bytes_sent: 0,
            packets_sent: 0,
            fail_streak: 0,
            dead_until: None,
            last_fail_log: None,
            hb_window: std::collections::VecDeque::new(),
            hb_pending: None,
            hb_sent: 0,
            hb_lost: 0,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    pub fn target(&self) -> SocketAddr {
        self.target
    }

    /// Send one heartbeat request; a still-pending previous request counts
    /// as lost (the peer stopped answering or the path ate both copies).
    pub fn send_heartbeat(&mut self) {
        if self.hb_pending.take().is_some() {
            self.hb_lost += 1;
        }
        let ts = crate::metrics::now_us();
        let mut buf = [0u8; HB_LEN];
        buf[..4].copy_from_slice(&HB_MAGIC);
        buf[4..12].copy_from_slice(&ts.to_le_bytes());
        buf[12] = HB_REQUEST;
        if self.socket.send(&buf).is_ok() {
            self.hb_pending = Some(ts);
            self.hb_sent += 1;
        }
    }

    /// Drain heartbeat echoes (non-blocking); returns true when an RTT
    /// sample was recorded.
    pub fn poll_heartbeat(&mut self) -> bool {
        let mut got = false;
        let mut buf = [0u8; HB_LEN];
        while let Ok(len) = self.socket.recv(&mut buf) {
            if len >= HB_LEN && buf[..4] == HB_MAGIC && buf[12] == HB_ECHO {
                let ts = u64::from_le_bytes(buf[4..12].try_into().unwrap_or([0; 8]));
                if self.hb_pending.map(|pending| pending == ts).unwrap_or(false) {
                    self.hb_window.push_back(crate::metrics::now_us().saturating_sub(ts));
                    if self.hb_window.len() > 32 {
                        self.hb_window.pop_front();
                    }
                    self.hb_pending = None;
                    got = true;
                }
            }
        }
        got
    }

    /// Current rolling health of this path (zeros until samples exist).
    /// Re-pin the connected socket's route (F13): a connected UDP socket
    /// caches the route chosen at connect time, so after Wi-Fi returns the
    /// sender kept riding the old interface and the audio stayed on
    /// Bluetooth. Re-connecting re-resolves source/route without touching
    /// the packer (stream id and sequence must not change).
    pub fn refresh_route(&mut self) {
        if self.socket.connect(self.target).is_err() {
            // Network momentarily unreachable (e.g. Wi-Fi mid-toggle); the
            // next tick's refresh retries.
            self.fail_streak += 1;
        }
    }

    pub fn path_health(&self) -> PathHealth {
        let window: Vec<u64> = self.hb_window.iter().copied().collect();
        PathHealth::from_window(&window, self.hb_sent, self.hb_lost)
    }

    pub fn stream_id(&self) -> u16 {
        self.packer.stream_id
    }

    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent
    }

    pub fn packets_sent(&self) -> u64 {
        self.packets_sent
    }

    /// Encode and send one interleaved stereo frame. Returns datagram size.
    pub fn send(&mut self, pcm: &[f32]) -> Result<usize> {
        let packet = self.packer.build(pcm)?.to_vec();
        self.send_packet(&packet)
    }

    /// Send a datagram produced by the shared [`FramePacker`]. Carries the
    /// same `(origin, stream_id, seq)` as every other carrier this tick.
    ///
    /// A sender whose destination vanished (Wi-Fi off, peer gone) fails on
    /// every 5 ms tick; retrying 200 times a second floods the log and burns
    /// CPU on phones. After a short run of failures the socket is parked for
    /// a second and failures are reported at most once per second.
    pub fn send_packet(&mut self, packet: &[u8]) -> Result<usize> {
        if let Some(at) = self.dead_until {
            if at > Instant::now() {
                return Ok(0);
            }
            self.dead_until = None;
        }
        match self.socket.send(packet) {
            Ok(sent) => {
                self.fail_streak = 0;
                self.packets_sent += 1;
                self.bytes_sent += sent as u64;
                Ok(sent)
            }
            Err(error) => {
                self.fail_streak = self.fail_streak.saturating_add(1);
                if self.fail_streak >= 20 {
                    self.fail_streak = 0;
                    self.dead_until = Some(Instant::now() + Duration::from_secs(1));
                }
                let now = Instant::now();
                let report = self
                    .last_fail_log
                    .map(|at| now.duration_since(at) >= Duration::from_secs(1))
                    .unwrap_or(true);
                if report {
                    self.last_fail_log = Some(now);
                    Err(CoreError::Backend(format!("udp send: {error}")))
                } else {
                    Ok(0)
                }
            }
        }
    }
}

fn stream_id_from_clock() -> u16 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let id = (nanos ^ pid.wrapping_mul(2654435761)) as u16;
    if id == crate::protocol::MIX_STREAM_ID {
        id.wrapping_add(1)
    } else {
        id
    }
}

/// Heartbeat datagram: magic + u64 send timestamp (us) + kind
/// (0 = request, 1 = echo). Sent every 500 ms per Wi-Fi peer; the peer's
/// receiver echoes from its bind socket, so the requester measures one-way
/// there and back on the exact carrier path the audio uses (F10).
pub const HB_MAGIC: [u8; 4] = *b"HB01";
pub const HB_REQUEST: u8 = 0;
pub const HB_ECHO: u8 = 1;
pub const HB_LEN: usize = 13;
/// One heartbeat every 100 mesh ticks = 500 ms at 5 ms frames.
pub const HB_TICKS: u64 = 100;

/// Rolling path health measured by the heartbeat requester.
#[derive(Clone, Copy, Debug, Default)]
pub struct PathHealth {
    pub rtt_p50_us: u64,
    pub rtt_p99_us: u64,
    /// Unanswered heartbeats over all sent, in percent.
    pub loss_pct: f64,
    pub samples: u64,
}

impl PathHealth {
    fn from_window(window: &[u64], sent: u64, lost: u64) -> Self {
        let mut sorted = window.to_vec();
        sorted.sort_unstable();
        let pick = |q: f64| {
            if sorted.is_empty() {
                0
            } else {
                sorted[((q * (sorted.len() - 1) as f64).round()) as usize]
            }
        };
        Self {
            rtt_p50_us: pick(0.5),
            rtt_p99_us: pick(0.99),
            loss_pct: if sent > 0 {
                100.0 * lost as f64 / sent as f64
            } else {
                0.0
            },
            samples: window.len() as u64,
        }
    }
}

/// Linear fade-in over the first `ramp` frames, in place. Returns how many
/// frames of ramp are left after this buffer (0 when the ramp finished).
pub fn apply_fade_in(samples: &mut [f32], ramp_left: usize) -> usize {
    let frames = samples.len() / CHANNELS;
    if frames == 0 || ramp_left == 0 {
        return ramp_left;
    }
    // The ramp always spans RAMP_FRAMES total; `ramp_left` is what is still
    // owed, so the gain continues where the previous buffer left off.
    let span = ramp_left.min(frames);
    for i in 0..span {
        let gain = (((RAMP_FRAMES - ramp_left) + i + 1) as f32 / RAMP_FRAMES as f32).min(1.0);
        for ch in 0..CHANNELS {
            samples[i * CHANNELS + ch] *= gain;
        }
    }
    ramp_left.saturating_sub(span)
}

/// A sample-to-sample jump larger than this is counted as a click. A full
/// scale sine at 20 kHz only moves ~0.13 per sample, so legitimate program
/// material rarely trips it; hard discontinuities reach 1.0+.
pub const CLICK_STEP: f32 = 0.5;

pub use crate::format::RAMP_FRAMES;

pub struct ReceiverConfig {
    pub bind: SocketAddr,
    pub target_ms: f32,
    pub max_streams: usize,
    /// Live copy of the user's buffer target; the UI may change it after the
    /// receiver started. Falls back to `target_ms` for fixed setups.
    pub live_target_ms: Option<Arc<AtomicU32>>,
    /// Decoded audio per origin, consumed by a hub that forwards N-1 mixes
    /// over Bluetooth.
    pub mix_tap: Option<Arc<MixTap>>,
    /// Loopback ports that carry Wi-Fi, not Bluetooth. The tunnel injects on
    /// loopback, so loopback sources classify as the Bluetooth carrier —
    /// which is what makes in-memory link simulators (whose fronts are also
    /// loopback) need this exemption. Empty in production.
    pub bt_exempt_ports: Vec<u16>,
}

/// Latest decoded audio per origin. A hub builds one mix per Bluetooth link
/// from this tap instead of forwarding every stream separately, halving the
/// downlink traffic on narrow links.
#[derive(Debug)]
pub struct MixTap {
    blocks: Mutex<HashMap<u64, TapBlock>>,
}

#[derive(Debug)]
struct TapBlock {
    samples: VecDeque<f32>,
    updated: Instant,
    /// Latest copy arrived over Bluetooth (the origin's Wi-Fi is gone).
    bt: bool,
}

impl MixTap {
    pub fn new() -> Self {
        Self {
            blocks: Mutex::new(HashMap::new()),
        }
    }

    /// Keep the most recent `keep` samples for one origin.
    pub fn push(&self, origin: u64, samples: &[f32], keep: usize, bt: bool) {
        if origin == 0 || samples.is_empty() {
            return;
        }
        let mut blocks = self.blocks.lock().unwrap();
        let block = blocks.entry(origin).or_insert_with(|| TapBlock {
            samples: VecDeque::with_capacity(keep),
            updated: Instant::now(),
            bt,
        });
        block.bt = bt;
        for &sample in samples {
            if block.samples.len() >= keep {
                block.samples.pop_front();
            }
            block.samples.push_back(sample);
        }
        block.updated = Instant::now();
    }

    /// Newest `want` samples of every origin that produced audio recently.
    pub fn snapshot(
        &self,
        want: usize,
        exclude: u64,
        max_age: Duration,
    ) -> Vec<(u64, Vec<f32>, bool)> {
        let now = Instant::now();
        let blocks = self.blocks.lock().unwrap();
        blocks
            .iter()
            .filter(|(origin, block)| {
                **origin != exclude
                    && !block.samples.is_empty()
                    && now.duration_since(block.updated) < max_age
            })
            .map(|(origin, block)| {
                let take = want.min(block.samples.len());
                let skip = block.samples.len() - take;
                (
                    *origin,
                    block.samples.iter().skip(skip).copied().collect(),
                    block.bt,
                )
            })
            .collect()
    }
}

impl Default for MixTap {
    fn default() -> Self {
        Self::new()
    }
}

/// Buffer target currently in force (the UI value wins over the start value).
fn target_now(config: &ReceiverConfig) -> f32 {
    config
        .live_target_ms
        .as_ref()
        .map(|value| f32::from_bits(value.load(Ordering::Relaxed)))
        .filter(|value| (5.0..=500.0).contains(value))
        .unwrap_or(config.target_ms)
}

#[derive(Default)]
pub struct ReceiverStats {
    pub datagrams: AtomicU64,
    pub bytes: AtomicU64,
    pub bad_packets: AtomicU64,
    pub timeouts: AtomicU64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceiverStatsSnapshot {
    pub datagrams: u64,
    pub bytes: u64,
    pub bad_packets: u64,
    pub timeouts: u64,
}

impl ReceiverStats {
    pub fn snapshot(&self) -> ReceiverStatsSnapshot {
        ReceiverStatsSnapshot {
            datagrams: self.datagrams.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            bad_packets: self.bad_packets.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
        }
    }
}

pub struct ReceiverHandle {
    pub local_addr: SocketAddr,
    pub registry: Arc<MetricsRegistry>,
    pub stats: Arc<ReceiverStats>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl ReceiverHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for ReceiverHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub fn spawn(config: ReceiverConfig, mixer: MixerControl) -> Result<ReceiverHandle> {
    let socket = UdpSocket::bind(config.bind)
        .map_err(|e| CoreError::Backend(format!("bind {}: {e}", config.bind)))?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    let local_addr = socket.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let registry = Arc::new(MetricsRegistry::default());
    let stats = Arc::new(ReceiverStats::default());

    let thread_stop = stop.clone();
    let thread_registry = registry.clone();
    let thread_stats = stats.clone();
    let join = thread::Builder::new()
        .name("siren-udp-receiver".into())
        .spawn(move || {
            run(
                socket,
                config,
                mixer,
                thread_registry,
                thread_stats,
                thread_stop,
            );
        })
        .map_err(|e| CoreError::Backend(format!("spawn receiver: {e}")))?;

    Ok(ReceiverHandle {
        local_addr,
        registry,
        stats,
        stop,
        join: Some(join),
    })
}

/// Logical stream identity. With a version 2 header the `origin` id is stable
/// across carriers, so the same frame arriving over Wi-Fi and Bluetooth maps
/// to one peer (and `seq` drops the second copy). Legacy version 1 packets
/// fall back to the source address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PeerKey {
    Origin(u64, u16),
    Addr(SocketAddr, u16),
}

impl PeerKey {
    fn new(origin: u64, addr: SocketAddr, stream_id: u16) -> Self {
        if origin == 0 {
            Self::Addr(addr, stream_id)
        } else {
            Self::Origin(origin, stream_id)
        }
    }
}

struct Peer {
    id: u64,
    origin: u64,
    codec: Codec,
    producer: Producer<f32>,
    /// Dual-stack delivers the same frame encoded differently per carrier
    /// (raw over Wi-Fi, Opus over Bluetooth); keep one decoder per codec and
    /// pick per packet. The sequence timeline is shared by both carriers.
    decoders: [Option<AudioDecoder>; 3],
    last_seq: Option<u32>,
    last_seen: Instant,
    metrics: Arc<StreamMetrics>,
    zero_buf: Vec<f32>,
    /// Consecutive frames delivered over the Bluetooth (loopback) carrier.
    bt_streak: u32,
    /// Latest copy arrived over the Bluetooth (loopback) carrier.
    last_path_bt: bool,
    /// Last copy seen over Wi-Fi, even when Bluetooth won the race. A hub uses
    /// this to decide whether the peer's Wi-Fi is still healthy.
    last_wifi_seen: Instant,
    /// Codec of the latest delivered copy; a change also signals a carrier
    /// switch (Wi-Fi raw <-> Bluetooth Opus).
    last_codec: Codec,
    /// Whether the buffer target was raised for a Bluetooth-only carrier.
    bt_target_raised: bool,
    /// Current adaptive target for a Bluetooth-only carrier (ms).
    bt_target_ms: f32,
    /// In-flight carrier switch, timed until the stream is audible again.
    switch: Option<SwitchTrace>,
    /// Last sample per channel from the previous delivery; the click
    /// detector diffs against it across packet boundaries.
    last_sample: Option<Vec<f32>>,
    /// Frames of fade-in still owed after a discontinuity (F2).
    ramp_remaining: usize,
    /// Timed out but kept for a fast resume (F8); cleared on the next packet.
    frozen: bool,
    /// Rate limit for the "seq gap" evidence line: a lifetime cap hides the
    /// very stalls the switch measurement is supposed to explain.
    last_gap_log: Instant,
    /// Rate limit for the "audible gap" line.
    last_mute_log: Instant,
    analyzer: SpectrumAnalyzer,
    last_underruns: u32,
    /// Overflow trims seen at the last adaptive check; underruns that follow a
    /// trim are our own doing and must not deepen the buffer.
    last_resets: u32,
    last_target_check: Instant,
    last_target_decay: Instant,
    last_report: Instant,
}

/// One carrier switch measured end to end: how long nothing arrived at all
/// (the link's fault) versus how long we spent re-filling the buffer (our
/// fault). Without the split, a 5 s mute is indistinguishable from a 5 s stall
/// and the fix gets guessed at.
struct SwitchTrace {
    from: &'static str,
    to: &'static str,
    /// Last packet on the old carrier; the mute starts here.
    last_old_packet: Instant,
    /// First packet accepted on the new carrier (the resync/flush point).
    decided_at: Instant,
    /// Warm-up count when the flush was issued.
    starts_before: u32,
    /// Silence we synthesized ourselves (zero-fill across a timeline jump), ms.
    synth_ms: f64,
}

impl Peer {
    fn decoder(&mut self, codec: Codec) -> Result<&mut AudioDecoder> {
        let index = codec as usize;
        if index >= self.decoders.len() {
            return Err(CoreError::Codec("unknown codec".into()));
        }
        if self.decoders[index].is_none() {
            self.decoders[index] = Some(AudioDecoder::new(codec)?);
        }
        Ok(self.decoders[index].as_mut().unwrap())
    }

    fn deliver(
        &mut self,
        payload: &[u8],
        codec: Codec,
        tap: Option<&MixTap>,
        bt: bool,
    ) -> Result<usize> {
        // The decoder returns a slice into its own buffer, so copy it out
        // before touching the ring producer again.
        let mut samples: Vec<f32> = {
            let decoder = self.decoder(codec)?;
            decoder.decode(payload)?.to_vec()
        };
        if let Some(tap) = tap {
            tap.push(self.origin, &samples, MIX_TAP_SAMPLES, bt);
        }
        self.count_clicks(&samples);
        self.analyzer.push(&samples, &self.metrics);
        if self.metrics.packets.load(Ordering::Relaxed) == 0 {
            let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
            let prefix: Vec<String> = payload
                .iter()
                .take(24)
                .map(|byte| format!("{byte:02X}"))
                .collect();
            log::info!(
                "stream {} first packet: {:?} {} B peak {:.4} [{}]",
                self.id,
                codec,
                payload.len(),
                peak,
                prefix.join(" ")
            );
        }
        let frames = samples.len() / CHANNELS;
        if self.ramp_remaining > 0 {
            self.ramp_remaining = apply_fade_in(&mut samples, self.ramp_remaining);
        }
        let (_, rest) = self.producer.push_partial_slice(&samples);
        if !rest.is_empty() {
            self.metrics
                .overflow_packets
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(frames)
    }

    /// Sample-to-sample step detection: a jump beyond [`CLICK_STEP`] is
    /// what the listener hears as a "click". Hard cuts at underruns, gap
    /// zero-fills, flushes and carrier switches all show up here — this is
    /// the acceptance metric for the ramp/xfade work (F2).
    fn count_clicks(&mut self, samples: &[f32]) {
        let channels = samples.len() - samples.len() % CHANNELS;
        if channels == 0 {
            return;
        }
        if let Some(prev) = &self.last_sample {
            let mut clicks = 0u64;
            for ch in 0..prev.len() {
                let mut previous = prev[ch];
                let mut index = ch;
                while index + CHANNELS <= samples.len() {
                    let current = samples[index];
                    if (current - previous).abs() > CLICK_STEP {
                        clicks += 1;
                    }
                    previous = current;
                    index += CHANNELS;
                }
            }
            if clicks > 0 {
                self.metrics.clicks.fetch_add(clicks, Ordering::Relaxed);
            }
        }
        self.last_sample = Some(samples[channels - CHANNELS..channels].to_vec());
    }

    /// Opus packet-loss concealment; returns concealed frames.
    fn conceal(&mut self, packets: u32, frame_frames: u32) -> u32 {
        let mut hidden = 0u32;
        let Some(decoder) = self.decoders[Codec::Opus as usize].as_mut() else {
            return 0;
        };
        {
            let decoder = decoder;
            let producer = &mut self.producer;
            let metrics = &self.metrics;
            for _ in 0..packets {
                match decoder.conceal(frame_frames as usize) {
                    Ok(Some(samples)) => {
                        let frames = (samples.len() / CHANNELS) as u32;
                        let (_, rest) = producer.push_partial_slice(samples);
                        if !rest.is_empty() {
                            metrics.overflow_packets.fetch_add(1, Ordering::Relaxed);
                        }
                        hidden += frames;
                    }
                    Ok(None) => break,
                    Err(error) => {
                        log::warn!("opus PLC failed: {error}");
                        break;
                    }
                }
            }
        }
        hidden
    }

    fn zero_fill(&mut self, frames: u32) {
        let mut remaining = frames.min(frames_for_ms(MAX_GAP_FILL_MS) as u32);
        while remaining > 0 {
            let chunk = remaining.min((self.zero_buf.len() / CHANNELS) as u32) as usize;
            let (_, rest) = self
                .producer
                .push_partial_slice(&self.zero_buf[..chunk * CHANNELS]);
            if !rest.is_empty() {
                self.metrics
                    .overflow_packets
                    .fetch_add(1, Ordering::Relaxed);
                break;
            }
            remaining -= chunk as u32;
        }
    }
}

fn run(
    socket: UdpSocket,
    config: ReceiverConfig,
    mixer: MixerControl,
    registry: Arc<MetricsRegistry>,
    stats: Arc<ReceiverStats>,
    stop: Arc<AtomicBool>,
) {
    crate::platform::boost_thread_priority();
    let mut peers: HashMap<PeerKey, Peer> = HashMap::new();
    let mut buffer = vec![0u8; 65_536];
    // A hub mix arrives over Bluetooth as a reserved stream id. It is a
    // fallback: while Wi-Fi copies of the individual streams are arriving it
    // is dropped, and when it takes over the individual buffers are flushed
    // so the same audio is not played twice.
    let mut last_mix_at = Instant::now() - Duration::from_secs(30);
    let mut mix_stream: Option<u64> = None;
    // Individual (non-mix) origins seen recently, with their last path.
    // The hub mix is a fallback for exactly these streams, so it is only used
    // while at least one of them is missing. Pruning keeps a node that left
    // (or an earlier session's stream) from pinning the mix on forever.
    let mut known_normals: HashMap<u64, (Instant, bool)> = HashMap::new();
    // F7: one origin arriving from two addresses is a cloned node image.
    let mut origin_addrs: HashMap<u64, (SocketAddr, Instant)> = HashMap::new();
    let mut active_mix_mask: u64 = 0;

    while !stop.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buffer) {
            Ok((len, addr)) => {
                stats.datagrams.fetch_add(1, Ordering::Relaxed);
                stats.bytes.fetch_add(len as u64, Ordering::Relaxed);
                let datagram = &buffer[..len];
                // Heartbeats ride on the same socket but are not frames:
                // echo requests straight back, echoes are for the sender.
                if len >= HB_LEN && datagram[..4] == HB_MAGIC {
                    if datagram[12] == HB_REQUEST {
                        let mut echo = [0u8; HB_LEN];
                        echo[..4].copy_from_slice(&HB_MAGIC);
                        echo[4..12].copy_from_slice(&datagram[4..12]);
                        echo[12] = HB_ECHO;
                        let _ = socket.send_to(&echo, addr);
                    }
                    continue;
                }
                let header = match FrameHeader::decode(datagram) {
                    Ok((header, payload)) => (header, payload),
                    Err(error) => {
                        stats.bad_packets.fetch_add(1, Ordering::Relaxed);
                        let prefix: Vec<String> = datagram
                            .iter()
                            .take(8)
                            .map(|byte| format!("{byte:02X}"))
                            .collect();
                        log::debug!(
                            "bad datagram from {addr}: {error} ({} B, [{}])",
                            datagram.len(),
                            prefix.join(" ")
                        );
                        continue;
                    }
                };
                let (header, payload) = header;
                let key = PeerKey::new(header.origin, addr, header.stream_id);
                let is_mix = header.stream_id == crate::protocol::MIX_STREAM_ID;
                // The tunnel injects on loopback, so a loopback source means
                // the Bluetooth carrier — unless the port is an exempted
                // Wi-Fi front (in-memory link simulator in tests).
                let is_bt =
                    addr.ip().is_loopback() && !config.bt_exempt_ports.contains(&addr.port());
                let now = Instant::now();
                if !is_mix && header.origin != 0 {
                    known_normals.insert(header.origin, (now, !is_bt));
                    known_normals.retain(|_, (at, _)| now.duration_since(*at) < KNOWN_NORMAL_TTL);
                    match origin_addrs.get(&header.origin) {
                        Some((prev, at)) if *prev != addr && now.duration_since(*at) < Duration::from_secs(10) => {
                            static CLONE_WARNED: std::sync::atomic::AtomicU64 =
                                std::sync::atomic::AtomicU64::new(0);
                            if CLONE_WARNED.fetch_add(1, Ordering::Relaxed) < 4 {
                                log::warn!(
                                    "udp: origin {:016x} seen from {} and {} — cloned node image (F7)",
                                    header.origin,
                                    prev,
                                    addr
                                );
                            }
                        }
                        _ => {}
                    }
                    origin_addrs.insert(header.origin, (addr, now));
                    origin_addrs.retain(|_, ( _, at)| now.duration_since(*at) < Duration::from_secs(60));
                }
                if is_mix {
                    // A mix carries a bitmask of the origins inside it. It is
                    // only needed while one of them is missing from the
                    // direct Wi-Fi copies.
                    if !mask_needs(header.mix_mask, &known_normals, now) {
                        continue;
                    }
                } else if now.duration_since(last_mix_at) < MIX_ACTIVE && mix_stream.is_some() {
                    // A mix is feeding this node. Individual copies it covers
                    // would be heard twice; copies it does not cover (e.g. a
                    // peer still reachable directly while the hub is offline)
                    // must keep playing.
                    if is_bt {
                        continue;
                    }
                    if !mask_needs(active_mix_mask, &known_normals, now) {
                        if let Some(mix_id) = mix_stream.take() {
                            let _ = mixer.flush_stream(mix_id);
                        }
                    } else if active_mix_mask & (1u64 << (header.origin % 64)) != 0 {
                        continue;
                    }
                }
                let mix_takeover = is_mix
                    && peers
                        .get(&key)
                        .map(|peer| {
                            peer.last_seen.elapsed() > CARRIER_STALL
                                || peer.metrics.packets.load(Ordering::Relaxed) == 0
                        })
                        .unwrap_or(true);
                if mix_takeover {
                    for (other_key, other) in peers.iter() {
                        if *other_key != key {
                            let _ = mixer.flush_stream(other.id);
                        }
                    }
                }

                // Ghost streams: a Bluetooth copy for a stream whose Wi-Fi
                // is still healthy gets dropped below — but it used to build
                // the stream first (a zombie with 0 packets holding a mixer
                // slot and a decoder). Gate BEFORE creating the stream; a
                // first-ever Bluetooth packet (no Wi-Fi state yet, e.g. a
                // phone that never had Wi-Fi) still passes.
                if is_bt && addr.port() != config.bind.port() {
                    if let Some(existing) = peers.get(&key) {
                        if now.duration_since(existing.last_wifi_seen) <= Duration::from_millis(300)
                        {
                            continue;
                        }
                    }
                }

                if !peers.contains_key(&key) {
                    match create_peer(&mixer, &registry, key, addr, &header, &config) {
                        Ok(peer) => {
                            log::info!(
                                "stream {} from {} ({}, origin {:016x}) joined",
                                peer.id,
                                addr,
                                header.codec.as_str(),
                                header.origin
                            );
                            peers.insert(key, peer);
                        }
                        Err(error) => {
                            log::warn!("cannot accept stream from {addr}: {error}");
                            continue;
                        }
                    }
                }

                let Some(peer) = peers.get_mut(&key) else {
                    continue;
                };
                if peer.frozen {
                    // F8 resume: the state survived the timeout, so just ramp
                    // back in instead of rebuilding the stream.
                    peer.frozen = false;
                    peer.ramp_remaining = RAMP_FRAMES;
                    peer.last_seq = None;
                    log::info!("stream {} resumed from freeze", peer.id);
                }
                if !is_bt {
                    peer.last_wifi_seen = now;
                }
                let tap_bt =
                    is_bt && now.duration_since(peer.last_wifi_seen) > Duration::from_millis(300);
                // The Bluetooth copy is a backup, not a second audio source.
                // Both carriers push into the same ring, so whichever arrives
                // first wins the sequence and the other is dropped as a
                // duplicate — letting Bluetooth win splices its 24 kbps Opus
                // into an otherwise raw stream, one 5 ms packet at a time.
                // Ignore it entirely until Wi-Fi has actually stalled.
                let is_local = is_bt && addr.port() == config.bind.port();
                if is_bt && !is_local && !tap_bt {
                    continue;
                }

                // Sequence bookkeeping: duplicate / reordering / loss.
                //
                // Dual-stack delivers the same frame twice (Wi-Fi + a delayed
                // Bluetooth copy). Anything at or behind the newest delivered
                // sequence is a duplicate and is dropped without touching the
                // decoder; a timeline more than ~5 s behind (1000 x 5 ms)
                // resynchronises instead — a restarted sender (fresh random
                // seq) used to be ignored for the remaining ~21 minutes.
                const LATE_DUPLICATE_LIMIT: i32 = -1_000;
                // A carrier switch can land far from the timeline the other
                // carrier has been feeding (a relayed Bluetooth copy may lag
                // the direct Wi-Fi copy by seconds, or jump ahead when Wi-Fi
                // returns). Splicing that as loss would either zero-fill
                // seconds of silence or drop the new carrier until it caught
                // up. Resync instead: flush the queued audio and continue
                // from the newly active carrier.
                const CARRIER_RESYNC_FRAMES: i32 = 100;
                // Only believe a lagging carrier once the active one has
                // actually stopped: a relayed Bluetooth stream can sit
                // hundreds of frames behind while Wi-Fi delivers normally,
                // and accepting those stale bursts would make the timeline
                // jump back and forth.
                let switched = is_bt != peer.last_path_bt || header.codec != peer.last_codec;
                let stalled = peer.last_seen.elapsed() > CARRIER_STALL;
                // What the listener actually loses: the span between two
                // accepted packets. A carrier switch that keeps the sequence
                // timeline continuous measures ~0 ms here, which is exactly the
                // difference between "our switch logic is slow" and "the link
                // was down".
                let mute = peer.last_seen.elapsed();
                if mute > AUDIBLE_MUTE && peer.last_mute_log.elapsed() >= Duration::from_secs(2) {
                    peer.last_mute_log = Instant::now();
                    log::info!(
                        "udp: stream {} audible gap {:.0} ms before this {} packet (previous carrier {})",
                        peer.id,
                        mute.as_secs_f64() * 1000.0,
                        if is_bt { "bt" } else { "wifi" },
                        if peer.last_path_bt { "bt" } else { "wifi" },
                    );
                }
                let mut gap = 0u32;
                if let Some(last) = peer.last_seq {
                    let diff = header.seq.wrapping_sub(last) as i32;
                    if switched && diff.abs() > CARRIER_RESYNC_FRAMES && (diff > 0 || stalled) {
                        log::info!(
                            "udp: stream {} carrier switch {} -> {}, timeline {} {} frames",
                            peer.id,
                            if peer.last_path_bt { "bt" } else { "wifi" },
                            if is_bt { "bt" } else { "wifi" },
                            if diff > 0 { "skips" } else { "rewinds" },
                            diff.unsigned_abs()
                        );
                        mixer.flush_stream(peer.id);
                        peer.last_seq = None;
                        peer.ramp_remaining = RAMP_FRAMES;
                        peer.switch = Some(SwitchTrace {
                            from: if peer.last_path_bt { "bt" } else { "wifi" },
                            to: if is_bt { "bt" } else { "wifi" },
                            last_old_packet: peer.last_seen,
                            decided_at: now,
                            starts_before: peer.metrics.starts.load(Ordering::Relaxed),
                            synth_ms: 0.0,
                        });
                    } else if diff <= 0 {
                        if diff > LATE_DUPLICATE_LIMIT {
                            continue; // duplicate or reordered late packet
                        }
                        for decoder in peer.decoders.iter_mut().flatten() {
                            decoder.reset();
                        }
                        peer.last_seq = None;
                    } else if diff > 1 {
                        gap = (diff - 1).min(20_000) as u32;
                        if gap > 50 && peer.last_gap_log.elapsed() >= Duration::from_secs(2) {
                            peer.last_gap_log = Instant::now();
                            log::warn!(
                                "udp: seq gap {} -> {} ({} missing) from {} origin {:016x} stream {} codec {} {} B",
                                last,
                                header.seq,
                                gap,
                                addr,
                                header.origin,
                                header.stream_id,
                                header.codec.as_str(),
                                payload.len()
                            );
                        }
                    }
                }

                let frame_frames = frames_for_ms(header.frame_ms as f32) as u32;
                if gap > 0 {
                    let lost = gap.saturating_mul(frame_frames);
                    // A timeline jump longer than a blink is what the user hears
                    // as "the mesh went silent", whether or not we also flushed.
                    // Start the same trace so the mute gets attributed.
                    const AUDIBLE_GAP_MS: f64 = 400.0;
                    let synth_ms = crate::format::ms_for_frames(lost as usize) as f64;
                    if synth_ms > AUDIBLE_GAP_MS && peer.switch.is_none() {
                        peer.switch = Some(SwitchTrace {
                            from: if peer.last_path_bt { "bt" } else { "wifi" },
                            to: if is_bt { "bt" } else { "wifi" },
                            last_old_packet: peer.last_seen,
                            decided_at: now,
                            starts_before: peer.metrics.starts.load(Ordering::Relaxed),
                            synth_ms,
                        });
                    }
                    // The listener hears 0 -> signal right after this gap;
                    // ramp the resume instead (F2).
                    peer.ramp_remaining = RAMP_FRAMES;
                    if header.codec == Codec::Opus {
                        let concealed = peer.conceal(gap.min(MAX_PLC_PACKETS), frame_frames);
                        peer.metrics
                            .plc_frames
                            .fetch_add(concealed as u64, Ordering::Relaxed);
                        let remaining = gap.saturating_sub(MAX_PLC_PACKETS);
                        if remaining > 0 {
                            for decoder in peer.decoders.iter_mut().flatten() {
                                decoder.reset();
                            }
                            peer.metrics.lost_frames.fetch_add(
                                remaining as u64 * frame_frames as u64,
                                Ordering::Relaxed,
                            );
                            peer.zero_fill(remaining.saturating_mul(frame_frames));
                        }
                    } else {
                        peer.metrics
                            .lost_frames
                            .fetch_add(lost as u64, Ordering::Relaxed);
                        peer.zero_fill(lost);
                    }
                }

                // Advance the timeline before decoding: a decoder failure must
                // not leave `last_seq` behind, or the next packet re-applies
                // the same gap (zero-fill) on every arrival and the buffer
                // fills with silence.
                peer.last_seq = Some(header.seq);
                match peer.deliver(payload, header.codec, config.mix_tap.as_deref(), tap_bt) {
                    Ok(frames) => {
                        let expected = frame_frames as usize;
                        if frames != expected {
                            static WARNED: std::sync::atomic::AtomicU64 =
                                std::sync::atomic::AtomicU64::new(0);
                            if WARNED.fetch_add(1, Ordering::Relaxed) < 8 {
                                log::warn!(
                                    "udp: stream {} decoded {} frames, expected {} (codec {:?}, payload {} B)",
                                    peer.id,
                                    frames,
                                    expected,
                                    header.codec,
                                    payload.len()
                                );
                            }
                        }
                        peer.last_seen = Instant::now();
                        peer.metrics.active.store(true, Ordering::Relaxed);
                        peer.metrics.packets.fetch_add(1, Ordering::Relaxed);
                        peer.metrics
                            .last_packet_us
                            .store(now_us(), Ordering::Relaxed);
                        // The same logical stream may alternate carriers; show
                        // whichever delivered the newest frame.
                        let path = if addr.ip().is_loopback() {
                            if addr.port() == config.bind.port() {
                                "local"
                            } else if is_bt {
                                "bt"
                            } else {
                                "wifi"
                            }
                        } else {
                            "wifi"
                        };
                        if is_mix {
                            last_mix_at = Instant::now();
                            mix_stream = Some(peer.id);
                            active_mix_mask = header.mix_mask;
                        }
                        peer.metrics.set_source(&addr.to_string(), path);
                        peer.last_path_bt = is_bt;
                        peer.last_codec = header.codec;
                        if peer.last_report.elapsed() >= Duration::from_secs(5) {
                            peer.last_report = Instant::now();
                            let m = &peer.metrics;
                            log::info!(
                                "udp: stream {} {} seq {} pkts {} lost {} resets {} underruns {} buffer {:.0} ms target {:.0} rms {:.1} dB",
                                peer.id,
                                if peer.last_path_bt { "bt" } else { "wifi" },
                                header.seq,
                                m.packets.load(Ordering::Relaxed),
                                m.lost_frames.load(Ordering::Relaxed),
                                m.buffer_resets.load(Ordering::Relaxed),
                                m.underrun_blocks.load(Ordering::Relaxed),
                                crate::format::ms_for_frames(
                                    m.buffer_frames.load(Ordering::Relaxed) as usize
                                ),
                                f32::from_bits(m.target_ms.load(Ordering::Relaxed)),
                                linear_to_db(f32::from_bits(m.rms.load(Ordering::Relaxed)))
                            );
                        }
                        if is_bt {
                            peer.bt_streak = peer.bt_streak.saturating_add(1);
                            if !peer.bt_target_raised && peer.bt_streak >= BT_TARGET_STREAK {
                                let target = target_now(&config).max(BT_STREAM_TARGET_MS);
                                if mixer.set_stream_target_ms(peer.id, target) {
                                    peer.bt_target_raised = true;
                                    peer.bt_target_ms = target;
                                    peer.last_underruns =
                                        peer.metrics.underrun_blocks.load(Ordering::Relaxed);
                                    peer.last_resets =
                                        peer.metrics.buffer_resets.load(Ordering::Relaxed);
                                    peer.last_target_check = Instant::now();
                                    peer.last_target_decay = Instant::now();
                                    log::info!(
                                        "stream {} is Bluetooth-only, buffer target {:.0} ms",
                                        peer.id,
                                        target
                                    );
                                }
                            } else if peer.bt_target_raised {
                                // Closed loop: Bluetooth dropouts show up as
                                // underruns, so deepen the buffer until they
                                // stop, then relax it once stable.
                                if peer.last_target_check.elapsed() >= Duration::from_secs(10) {
                                    let now_underruns =
                                        peer.metrics.underrun_blocks.load(Ordering::Relaxed);
                                    let delta = now_underruns.saturating_sub(peer.last_underruns);
                                    let now_resets =
                                        peer.metrics.buffer_resets.load(Ordering::Relaxed);
                                    let reset_delta = now_resets.saturating_sub(peer.last_resets);
                                    peer.last_underruns = now_underruns;
                                    peer.last_resets = now_resets;
                                    peer.last_target_check = Instant::now();
                                    if delta >= 1 && reset_delta >= 1 {
                                        // Self-inflicted: the overflow trim dropped
                                        // the buffer and the next burst arrived after
                                        // the player had already starved. Deepening
                                        // the target only makes the next trim bigger,
                                        // so the loop stops here.
                                        log::info!(
                                            "stream {} underruns follow {} overflow trim(s), keeping buffer target {:.0} ms",
                                            peer.id,
                                            reset_delta,
                                            peer.bt_target_ms
                                        );
                                    } else if delta >= 1 && peer.bt_target_ms < BT_MAX_TARGET_MS {
                                        let target =
                                            (peer.bt_target_ms + 50.0).min(BT_MAX_TARGET_MS);
                                        if mixer.set_stream_target_ms(peer.id, target) {
                                            peer.bt_target_ms = target;
                                            log::info!(
                                                "stream {} Bluetooth dropouts, buffer target {:.0} ms",
                                                peer.id,
                                                target
                                            );
                                        }
                                        peer.last_target_decay = Instant::now();
                                    } else if delta == 0
                                        && peer.bt_target_ms > BT_STREAM_TARGET_MS
                                        && peer.last_target_decay.elapsed()
                                            >= Duration::from_secs(90)
                                    {
                                        let target =
                                            (peer.bt_target_ms - 50.0).max(BT_STREAM_TARGET_MS);
                                        if mixer.set_stream_target_ms(peer.id, target) {
                                            peer.bt_target_ms = target;
                                        }
                                        peer.last_target_decay = Instant::now();
                                    }
                                }
                            }
                        } else if peer.bt_target_raised {
                            // Wi-Fi is back: a lower target cuts latency again.
                            peer.bt_target_raised = false;
                            peer.bt_streak = 0;
                            let _ = mixer.set_stream_target_ms(peer.id, target_now(&config));
                        } else {
                            peer.bt_streak = 0;
                        }

                        // Step A of the dual-stack plan: report the switch only
                        // once the stream is audible again, so the mute splits
                        // into "nothing arrived" and "we were re-buffering".
                        if let Some(trace) = peer.switch.take() {
                            let started =
                                peer.metrics.starts.load(Ordering::Relaxed) > trace.starts_before;
                            if started {
                                log::info!(
                                    "udp: stream {} switch {} -> {} measured: {:.0} ms with no packet at all, {:.0} ms synthesized as silence, then {:.0} ms re-filling; total mute {:.0} ms (target {:.0} ms)",
                                    peer.id,
                                    trace.from,
                                    trace.to,
                                    trace.decided_at.duration_since(trace.last_old_packet).as_secs_f64() * 1000.0,
                                    trace.synth_ms,
                                    trace.decided_at.elapsed().as_secs_f64() * 1000.0,
                                    trace.last_old_packet.elapsed().as_secs_f64() * 1000.0,
                                    f32::from_bits(peer.metrics.target_ms.load(Ordering::Relaxed)),
                                );
                            } else if trace.decided_at.elapsed() > Duration::from_secs(15) {
                                log::warn!(
                                    "udp: stream {} switch {} -> {} never reached its start level within 15 s",
                                    peer.id,
                                    trace.from,
                                    trace.to
                                );
                            } else {
                                peer.switch = Some(trace);
                            }
                        }
                    }
                    Err(error) => {
                        stats.bad_packets.fetch_add(1, Ordering::Relaxed);
                        log::debug!("decode failed from {addr}: {error}");
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => {
                log::warn!("udp receive error: {error}");
                thread::sleep(Duration::from_millis(10));
            }
        }

        let now = Instant::now();
        peers.retain(|key, peer| {
            // RFCOMM stalls for seconds; retire a Bluetooth-carried stream
            // only after a much longer silence so a stall does not destroy
            // the jitter buffer state (which would cause an audible jump).
            let timeout = if peer.last_path_bt {
                BT_PEER_TIMEOUT
            } else {
                PEER_TIMEOUT
            };
            let silent_for = now.duration_since(peer.last_seen);
            let alive = silent_for < timeout;
            if !alive && !peer.frozen {
                // F8 / MIX1: freeze instead of destroy. Tearing the stream
                // down on every timeout made the receiver rebuild it on the
                // next burst, and the trim<->underrun loop (MIX1) kept the
                // stream churning. Flush to silence and keep the state; the
                // sender's return becomes a fast ramped resume.
                peer.frozen = true;
                peer.metrics.active.store(false, Ordering::Relaxed);
                mixer.flush_stream(peer.id);
                peer.ramp_remaining = RAMP_FRAMES;
                peer.last_seq = None;
                stats.timeouts.fetch_add(1, Ordering::Relaxed);
                log::info!(
                    "stream {} ({key:?}) frozen after {:.1} s silence",
                    peer.id,
                    silent_for.as_secs_f64()
                );
            }
            let expired = peer.frozen && silent_for >= FREEZE_GRACE;
            if expired {
                peer.metrics.active.store(false, Ordering::Relaxed);
                mixer.remove(peer.id);
                registry.remove(peer.id);
                log::info!("stream {} ({key:?}) evicted after freeze grace", peer.id);
            }
            !expired
        });
    }
}

fn create_peer(
    mixer: &MixerControl,
    registry: &Arc<MetricsRegistry>,
    key: PeerKey,
    addr: SocketAddr,
    header: &FrameHeader,
    config: &ReceiverConfig,
) -> Result<Peer> {
    if config.max_streams == 0 {
        return Err(CoreError::InvalidArgument("max_streams is zero".into()));
    }
    if registry.len() >= config.max_streams {
        return Err(CoreError::Backend(format!(
            "stream limit reached ({})",
            config.max_streams
        )));
    }
    let id = stream_key(&key);
    let label = if header.origin == 0 {
        peer_label(&addr, header.stream_id)
    } else {
        format!("{:016x}#{}", header.origin, header.stream_id)
    };
    let base_target = target_now(config);
    let metrics = Arc::new(StreamMetrics::new(
        id,
        header.origin,
        header.stream_id,
        label,
        addr.to_string(),
        header.codec,
        base_target,
    ));
    let bt_carrier = addr.ip().is_loopback() && !config.bt_exempt_ports.contains(&addr.port());
    let target_ms = if bt_carrier {
        base_target.max(BT_STREAM_TARGET_MS)
    } else {
        base_target
    };
    let metrics = {
        store_f32(&metrics.target_ms, target_ms);
        metrics
    };
    let (producer, consumer) = rtrb::RingBuffer::new(PEER_RING_SAMPLES);
    let channel = StreamChannel::new(metrics.clone(), consumer, target_ms);
    if !mixer.add(channel) {
        return Err(CoreError::Backend("mixer command queue is full".into()));
    }
    registry.add(metrics.clone());
    // Slot by codec id: a stream may be discovered on the Bluetooth carrier
    // (Opus) and later receive Wi-Fi copies (raw). Putting the first decoder
    // in slot 0 regardless of its codec made every later raw packet decode
    // with the Opus decoder and fail.
    let mut decoders: [Option<AudioDecoder>; 3] = [None, None, None];
    decoders[header.codec as usize] = Some(AudioDecoder::new(header.codec)?);
    Ok(Peer {
        id,
        origin: header.origin,
        codec: header.codec,
        producer,
        decoders,
        last_seq: None,
        last_seen: Instant::now(),
        last_sample: None,
        ramp_remaining: RAMP_FRAMES,
        frozen: false,
        metrics,
        zero_buf: vec![0.0; frames_for_ms(MAX_GAP_FILL_MS) * CHANNELS],
        bt_streak: if bt_carrier { 1 } else { 0 },
        last_path_bt: bt_carrier,
        last_wifi_seen: if bt_carrier {
            Instant::now() - Duration::from_secs(5)
        } else {
            Instant::now()
        },
        last_codec: header.codec,
        bt_target_raised: bt_carrier,
        bt_target_ms: target_ms,
        switch: None,
        last_gap_log: Instant::now(),
        last_mute_log: Instant::now(),
        analyzer: SpectrumAnalyzer::new(),
        last_underruns: 0,
        last_resets: 0,
        last_target_check: Instant::now(),
        last_target_decay: Instant::now(),
        last_report: Instant::now(),
    })
}

fn stream_key(key: &PeerKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    // Keep the id inside i64 so JSON/Dart can round-trip it exactly.
    hasher.finish() & 0x7FFF_FFFF_FFFF_FFFF
}

/// Sender-side stream identity is independent from the connection address, so
/// two senders behind the same NAT still map to distinct channels.
pub fn peer_label(addr: &SocketAddr, stream_id: u16) -> String {
    format!("{addr}#{stream_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::new_mixer;
    use crate::format::frames_for_ms;

    #[test]
    fn dual_path_duplicate_is_mixed_once() {
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .expect("receiver");

        let first = UdpSocket::bind("127.0.0.1:0").expect("socket a");
        let second = UdpSocket::bind("127.0.0.1:0").expect("socket b");
        let frames = frames_for_ms(5.0);
        let mut datagram = FrameHeader::new(Codec::RawF32, 5, 77, 1, now_us(), 0xABCD_1234)
            .encode()
            .to_vec();
        for _ in 0..frames * CHANNELS {
            datagram.extend_from_slice(&0.1f32.to_le_bytes());
        }

        // The same frame arrives over both carriers: identical origin, stream
        // and sequence, different sockets (Wi-Fi peer vs. local BT tunnel).
        first
            .send_to(&datagram, receiver.local_addr)
            .expect("send wifi");
        second
            .send_to(&datagram, receiver.local_addr)
            .expect("send bt");
        std::thread::sleep(Duration::from_millis(300));

        let snapshots = receiver.registry.snapshots();
        assert_eq!(snapshots.len(), 1, "one logical stream, not two carriers");
        assert_eq!(snapshots[0].packets, 1, "duplicate copy dropped");
        assert!(snapshots[0].id > 0);
        receiver.stop();
        drop(mixer);
    }

    #[test]
    fn bluetooth_carrier_shares_primary_sequence() {
        let pcm = vec![0.1f32; frames_for_ms(5.0) * CHANNELS];
        let mut wifi = FramePacker::new(Codec::RawF32, 5, 0xABC).expect("wifi packer");
        let mut bt =
            FramePacker::with_stream(Codec::Opus, 5, 0xABC, wifi.stream_id(), Some(32_000))
                .expect("bt packer");
        // Let the primary run ahead, as it does while the tunnel is still
        // connecting. The secondary must adopt the primary's sequence rather
        // than counting on its own.
        for _ in 0..37 {
            wifi.build(&pcm).expect("wifi frame");
        }
        for _ in 0..5 {
            let seq = wifi.next_seq();
            wifi.build(&pcm).expect("wifi frame");
            let packet = bt.build_compact_with_seq(&pcm, seq).expect("bt frame");
            let (header, _) = FrameHeader::decode(packet).expect("decode");
            assert_eq!(header.seq, seq);
            assert_eq!(header.stream_id, wifi.stream_id());
            assert_eq!(header.origin, 0xABC);
        }
        assert_eq!(bt.next_seq(), 0, "secondary counter must not advance");
    }

    #[test]
    fn opus_first_stream_accepts_raw_later() {
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .expect("receiver");
        let socket = UdpSocket::bind("127.0.0.1:0").expect("socket");
        let frames = frames_for_ms(5.0);
        let pcm = vec![0.1f32; frames * CHANNELS];

        // First contact over the Bluetooth carrier: Opus.
        let mut encoder =
            AudioEncoder::with_bitrate(Codec::Opus, Some(32_000)).expect("opus encoder");
        let mut opus = Vec::new();
        encoder.encode(&pcm, &mut opus).expect("encode");
        let mut bt = FrameHeader::new(Codec::Opus, 5, 7, 1, now_us(), 0x1234)
            .encode_compact()
            .to_vec();
        bt.extend_from_slice(&opus);
        socket.send_to(&bt, receiver.local_addr).expect("send bt");

        // The next frame arrives over Wi-Fi as raw: the peer must pick a raw
        // decoder, not reuse the Opus one from the first packet.
        let mut raw = FrameHeader::new(Codec::RawF32, 5, 7, 2, now_us(), 0x1234)
            .encode()
            .to_vec();
        for _ in 0..frames * CHANNELS {
            raw.extend_from_slice(&0.1f32.to_le_bytes());
        }
        socket
            .send_to(&raw, receiver.local_addr)
            .expect("send wifi");
        std::thread::sleep(Duration::from_millis(300));

        let snapshots = receiver.registry.snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].packets, 2, "both carriers decoded");
        assert_eq!(snapshots[0].lost_frames, 0);
        receiver.stop();
        drop(mixer);
    }

    #[test]
    fn decode_failure_does_not_repeat_gap() {
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .expect("receiver");
        let socket = UdpSocket::bind("127.0.0.1:0").expect("socket");
        let frames = frames_for_ms(5.0);
        let mut bad = FrameHeader::new(Codec::RawF32, 5, 9, 1, now_us(), 0x5555)
            .encode()
            .to_vec();
        bad.extend_from_slice(&[1, 2, 3]);
        socket.send_to(&bad, receiver.local_addr).expect("send bad");
        let mut good = FrameHeader::new(Codec::RawF32, 5, 9, 2, now_us(), 0x5555)
            .encode()
            .to_vec();
        for _ in 0..frames * CHANNELS {
            good.extend_from_slice(&0.1f32.to_le_bytes());
        }
        socket
            .send_to(&good, receiver.local_addr)
            .expect("send good");
        std::thread::sleep(Duration::from_millis(300));

        let snapshots = receiver.registry.snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].packets, 1);
        assert_eq!(
            snapshots[0].lost_frames, 0,
            "a failed decode must still advance the sequence"
        );
        receiver.stop();
        drop(mixer);
    }

    #[test]
    fn carrier_switch_resyncs_instead_of_zero_filling() {
        fn raw_packet(seq: u32, stream: u16, origin: u64, frames: usize) -> Vec<u8> {
            let mut packet = FrameHeader::new(Codec::RawF32, 5, stream, seq, now_us(), origin)
                .encode()
                .to_vec();
            for _ in 0..frames * CHANNELS {
                packet.extend_from_slice(&0.1f32.to_le_bytes());
            }
            packet
        }
        fn opus_packet(seq: u32, stream: u16, origin: u64, frames: usize) -> Vec<u8> {
            let pcm = vec![0.1f32; frames * CHANNELS];
            let mut encoder =
                AudioEncoder::with_bitrate(Codec::Opus, Some(32_000)).expect("encoder");
            let mut opus = Vec::new();
            encoder.encode(&pcm, &mut opus).expect("encode");
            let mut packet = FrameHeader::new(Codec::Opus, 5, stream, seq, now_us(), origin)
                .encode_compact()
                .to_vec();
            packet.extend_from_slice(&opus);
            packet
        }

        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .expect("receiver");
        let socket = UdpSocket::bind("127.0.0.1:0").expect("socket");
        let frames = frames_for_ms(5.0);

        // Wi-Fi frame 100, then the Bluetooth carrier jumps ahead to 400:
        // the switch must resync, not zero-fill 300 frames.
        socket
            .send_to(&raw_packet(100, 3, 0xCAFE, frames), receiver.local_addr)
            .expect("wifi");
        socket
            .send_to(&opus_packet(400, 3, 0xCAFE, frames), receiver.local_addr)
            .expect("bt ahead");
        std::thread::sleep(Duration::from_millis(250));
        let snapshots = receiver.registry.snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].packets, 2);
        assert_eq!(
            snapshots[0].lost_frames, 0,
            "a carrier jump must not be treated as packet loss"
        );

        // A stale Bluetooth burst while Wi-Fi keeps flowing must be dropped,
        // not resynced: otherwise the timeline jumps back on every burst.
        socket
            .send_to(&raw_packet(900, 5, 0xD00D, frames), receiver.local_addr)
            .expect("wifi live");
        socket
            .send_to(&opus_packet(120, 5, 0xD00D, frames), receiver.local_addr)
            .expect("bt stale");
        std::thread::sleep(Duration::from_millis(150));
        let snapshots = receiver.registry.snapshots();
        let stale = snapshots
            .iter()
            .find(|snapshot| snapshot.label.contains("d00d"))
            .expect("third stream");
        assert_eq!(stale.packets, 1, "stale relay copy dropped");
        assert_eq!(stale.lost_frames, 0);

        // When Wi-Fi actually stops, the lagging Bluetooth carrier is the
        // only path left: accept it and resync after the stall window.
        socket
            .send_to(&raw_packet(900, 4, 0xBEEF, frames), receiver.local_addr)
            .expect("wifi 2");
        std::thread::sleep(Duration::from_millis(250));
        socket
            .send_to(&opus_packet(120, 4, 0xBEEF, frames), receiver.local_addr)
            .expect("bt behind");
        std::thread::sleep(Duration::from_millis(250));
        let snapshots = receiver.registry.snapshots();
        let second = snapshots
            .iter()
            .find(|snapshot| snapshot.label.contains("beef"))
            .expect("second stream");
        assert_eq!(second.packets, 2);
        assert_eq!(second.lost_frames, 0);
        receiver.stop();
        drop(mixer);
    }

    #[test]
    fn mix_tap_keeps_newest_samples_and_excludes_destination() {
        let tap = MixTap::new();
        tap.push(0xAA, &vec![1.0f32; 960], 960, false);
        tap.push(0xBB, &vec![2.0f32; 960], 960, true);
        let snapshot = tap.snapshot(960, 0xAA, Duration::from_secs(1));
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, 0xBB);
        tap.push(0xAA, &vec![3.0f32; 20], 960, false);
        let snapshot = tap.snapshot(960, 0, Duration::from_secs(1));
        let block = snapshot
            .iter()
            .find(|(origin, _, _)| *origin == 0xAA)
            .expect("origin");
        assert_eq!(block.1.len(), 960);
        assert_eq!(block.1[959], 3.0);
        std::thread::sleep(Duration::from_millis(20));
        assert!(tap.snapshot(960, 0, Duration::from_millis(5)).is_empty());
    }

    #[test]
    fn hub_mix_yields_to_live_wifi_and_takes_over_when_it_stops() {
        fn lan_ip() -> Option<std::net::IpAddr> {
            let probe = UdpSocket::bind("0.0.0.0:0").ok()?;
            probe.connect("8.8.8.8:80").ok()?;
            let ip = probe.local_addr().ok()?.ip();
            if ip.is_loopback() {
                None
            } else {
                Some(ip)
            }
        }
        let Some(ip) = lan_ip() else {
            return;
        };
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "0.0.0.0:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .expect("receiver");
        let port = receiver.local_addr.port();
        let wifi = UdpSocket::bind(format!("{ip}:0")).expect("wifi socket");
        let bt = UdpSocket::bind("127.0.0.1:0").expect("bt socket");
        let frames = frames_for_ms(5.0);
        let normal = |seq: u32, stream: u16, origin: u64| {
            let mut packet = FrameHeader::new(Codec::RawF32, 5, stream, seq, now_us(), origin)
                .encode()
                .to_vec();
            for _ in 0..frames * CHANNELS {
                packet.extend_from_slice(&0.1f32.to_le_bytes());
            }
            packet
        };
        let mask =
            (1u64 << (0xAAAu64 % 64)) | (1u64 << (0xBBBu64 % 64)) | (1u64 << (0xCCCu64 % 64));
        let mix_packet = |seq: u32| {
            let mut packet = FrameHeader::new(
                Codec::RawF32,
                5,
                crate::protocol::MIX_STREAM_ID,
                seq,
                now_us(),
                0xBBB,
            )
            .encode_mix(mask)
            .to_vec();
            for _ in 0..frames * CHANNELS {
                packet.extend_from_slice(&0.1f32.to_le_bytes());
            }
            packet
        };

        // Three individual streams (A, hub B, C) over Wi-Fi.
        for (stream, origin) in [(7u16, 0xAAAu64), (9, 0xBBB), (8, 0xCCC)] {
            wifi.send_to(&normal(1, stream, origin), format!("{ip}:{port}"))
                .expect("wifi normal");
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(receiver.registry.snapshots().len(), 3);

        // The mix covers all three and all are fresh: it must be ignored.
        bt.send_to(&mix_packet(1), format!("127.0.0.1:{port}"))
            .expect("mix while wifi");
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            receiver.registry.snapshots().len(),
            3,
            "mix dropped while Wi-Fi is fresh"
        );

        // Wi-Fi gone: the mix takes over.
        std::thread::sleep(Duration::from_millis(600));
        bt.send_to(&mix_packet(2), format!("127.0.0.1:{port}"))
            .expect("mix after wifi");
        std::thread::sleep(Duration::from_millis(120));
        let snapshots = receiver.registry.snapshots();
        assert_eq!(snapshots.len(), 4, "mix accepted after Wi-Fi stops");
        let mix = snapshots
            .iter()
            .find(|snapshot| snapshot.label.contains("#65520"))
            .expect("mix stream");
        assert_eq!(mix.packets, 1);
        assert_eq!(mix.lost_frames, 0);

        // One Wi-Fi stream returns while the mix is active: it is covered by
        // the mix, so its frames must not play twice.
        wifi.send_to(&normal(2, 7, 0xAAA), format!("{ip}:{port}"))
            .expect("wifi A back");
        std::thread::sleep(Duration::from_millis(80));
        let snapshots = receiver.registry.snapshots();
        let a = snapshots
            .iter()
            .find(|snapshot| snapshot.label.contains("aaa"))
            .expect("A stream");
        assert_eq!(a.packets, 1, "normal suppressed while mix covers it");

        // Every covered stream is back: release the mix.
        wifi.send_to(&normal(2, 9, 0xBBB), format!("{ip}:{port}"))
            .expect("wifi B back");
        wifi.send_to(&normal(2, 8, 0xCCC), format!("{ip}:{port}"))
            .expect("wifi C back");
        // The arriving packet that completes the covered set triggers the
        // release; earlier covered copies were dropped, so one more tick each.
        wifi.send_to(&normal(3, 7, 0xAAA), format!("{ip}:{port}"))
            .expect("wifi A tick");
        wifi.send_to(&normal(3, 9, 0xBBB), format!("{ip}:{port}"))
            .expect("wifi B tick");
        std::thread::sleep(Duration::from_millis(150));
        let snapshots = receiver.registry.snapshots();
        for (needle, expected) in [("aaa", 2), ("ccc", 2), ("bbb#", 2)] {
            let stream = snapshots
                .iter()
                .find(|snapshot| {
                    snapshot.label.contains(needle) && !snapshot.label.contains("#65520")
                })
                .expect("normal stream");
            assert_eq!(stream.packets, expected, "{} after release", needle);
        }
        receiver.stop();
        drop(mixer);
    }

    #[test]
    fn different_origins_stay_separate() {
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .expect("receiver");
        let socket = UdpSocket::bind("127.0.0.1:0").expect("socket");
        let frames = frames_for_ms(5.0);
        for (seq, origin) in [(1u32, 0x11u64), (2, 0x22)] {
            let mut datagram = FrameHeader::new(Codec::RawF32, 5, 5, seq, now_us(), origin)
                .encode()
                .to_vec();
            for _ in 0..frames * CHANNELS {
                datagram.extend_from_slice(&0.1f32.to_le_bytes());
            }
            socket
                .send_to(&datagram, receiver.local_addr)
                .expect("send");
        }
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(receiver.registry.snapshots().len(), 2);
        receiver.stop();
        drop(mixer);
    }

    #[test]
    fn heartbeat_echo_round_trip() {
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .unwrap();
        let mut sender = UdpSender::new(receiver.local_addr, Codec::RawF32, 5, 0x1234).unwrap();
        sender.send_heartbeat();
        // The receiver echoes from its bind socket; give it a moment.
        for _ in 0..100 {
            if sender.poll_heartbeat() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let health = sender.path_health();
        assert!(health.samples >= 1, "echo should be measured");
        assert!(health.rtt_p50_us < 50_000, "loopback RTT should be small: {:?}", health);
        assert_eq!(health.loss_pct, 0.0);
        receiver.stop();
        let _ = mixer;
    }

    #[test]
    fn click_detector_counts_hard_steps() {
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let registry = MetricsRegistry::default();
        let header = FrameHeader::new(Codec::RawF32, 5, 7, 1, 0, 0x1234);
        let mut peer = create_peer(
            &control,
            &Arc::new(registry),
            PeerKey::new(0x1234, "127.0.0.1:1".parse().unwrap(), 7),
            "127.0.0.1:1".parse().unwrap(),
            &header,
            &ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
        )
        .unwrap();
        // Smooth sine: no clicks.
        let mut smooth = Vec::new();
        for i in 0..480 {
            let v = 0.25 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin();
            smooth.extend_from_slice(&[v, v]);
        }
        peer.count_clicks(&smooth);
        assert_eq!(peer.metrics.clicks.load(Ordering::Relaxed), 0);
        // Hard cut to -1.0 on one channel: the first transition is a click.
        let mut cut = vec![0.0f32; 480 * 2];
        for i in 0..480 {
            cut[i * 2] = -1.0;
        }
        peer.count_clicks(&cut);
        let clicks = peer.metrics.clicks.load(Ordering::Relaxed);
        assert!(clicks >= 1, "hard cut should count as a click: {clicks}");
        let _ = mixer;
    }

    #[test]
    fn fade_in_ramps_the_head_without_touching_the_tail() {
        // 480 stereo frames: first 240 (RAMP_FRAMES) ramp 1/240..1.0.
        let mut samples: Vec<f32> = vec![1.0; 480 * CHANNELS];
        let left = apply_fade_in(&mut samples, RAMP_FRAMES);
        assert_eq!(left, 0, "3 ms ramp fits in a 10 ms buffer");
        // First sample barely rises, ramp end reaches full scale.
        let first = samples[0];
        assert!(first > 0.0 && first < 0.02, "first sample ~1/240: {first}");
        assert!((samples[(RAMP_FRAMES - 1) * CHANNELS] - 1.0).abs() < 1e-3);
        assert_eq!(samples[RAMP_FRAMES * CHANNELS], 1.0, "tail untouched");
        // Short buffer: ramp spills over, returned remainder > 0.
        let mut short = vec![1.0f32; 100 * CHANNELS];
        let left = apply_fade_in(&mut short, RAMP_FRAMES);
        assert_eq!(left, RAMP_FRAMES - 100);
        assert!((short[(100 - 1) * CHANNELS] - 100.0 / 240.0).abs() < 1e-3);
    }

    #[test]
    fn sender_restart_resyncs_within_five_seconds() {
        // A restarted sender emits a fresh low sequence. Before the limit was
        // tightened, anything behind the newest delivered seq for less than
        // ~21 min was discarded as a late duplicate — the mesh stayed silent
        // for the rest of that window. At ~5 s the resync fires immediately.
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .expect("receiver");
        let origin = 0xABCD_1234;
        let stream = 99u16;
        let socket = UdpSocket::bind("127.0.0.1:0").expect("socket");
        let frames = frames_for_ms(5.0);
        let send_seq = |socket: &UdpSocket, seq: u32| {
            let mut datagram = FrameHeader::new(Codec::RawF32, 5, stream, seq, now_us(), origin)
                .encode()
                .to_vec();
            for _ in 0..frames * CHANNELS {
                datagram.extend_from_slice(&0.1f32.to_le_bytes());
            }
            socket.send_to(&datagram, receiver.local_addr).expect("send");
        };

        // Session 1: run the sequence well past the new limit.
        for seq in 1..=1100u32 {
            send_seq(&socket, seq);
        }
        std::thread::sleep(Duration::from_millis(200));
        // Loopback queues drop a few datagrams under a burst like this, so
        // judge the restart by the delta, not by an absolute total.
        let baseline = receiver.registry.snapshots()[0].packets;

        // Session 2 (restart): the sender starts over at seq 1. A rewind of
        // ~1099 frames is beyond the 1000-packet (5 s) tolerance, so it must
        // resynchronise — and subsequent packets must count as delivered.
        for seq in 1..=10u32 {
            send_seq(&socket, seq);
        }
        std::thread::sleep(Duration::from_millis(200));

        let snapshots = receiver.registry.snapshots();
        assert_eq!(snapshots.len(), 1);
        let after_restart = snapshots[0].packets;
        assert!(
            after_restart >= baseline + 9,
            "restarted sender should be accepted immediately (resync at ~5 s): baseline={baseline} after={after_restart}"
        );
        receiver.stop();
        drop(mixer);
    }

    #[test]
    fn stale_bt_backlog_still_dropped_as_duplicate() {
        // Within the ~5 s window a lagging carrier copy must still be a
        // silent duplicate, not a reset (the receiver must not jump back).
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .expect("receiver");
        let origin = 0x1122_3344;
        let stream = 55u16;
        let socket = UdpSocket::bind("127.0.0.1:0").expect("socket");
        let frames = frames_for_ms(5.0);
        let send_seq = |socket: &UdpSocket, seq: u32| {
            let mut datagram = FrameHeader::new(Codec::RawF32, 5, stream, seq, now_us(), origin)
                .encode()
                .to_vec();
            for _ in 0..frames * CHANNELS {
                datagram.extend_from_slice(&0.1f32.to_le_bytes());
            }
            socket.send_to(&datagram, receiver.local_addr).expect("send");
        };

        for seq in 1..=50u32 {
            send_seq(&socket, seq);
        }
        std::thread::sleep(Duration::from_millis(150));
        let baseline = receiver.registry.snapshots()[0].packets;

        // A copy 40 frames behind (< 5 s): duplicate, not a resync.
        send_seq(&socket, 10);
        std::thread::sleep(Duration::from_millis(150));
        let snapshots = receiver.registry.snapshots();
        assert_eq!(snapshots[0].packets, baseline, "stale copy must be dropped");
        receiver.stop();
        drop(mixer);
    }

    #[test]
    fn timeout_freezes_then_resumes_without_rebuild() {
        // F8/MIX1: a timed-out stream is frozen (kept, silent) instead of
        // destroyed; the sender's return is a fast ramped resume.
        let (mixer, handle) = new_mixer(8);
        let control = MixerControl::new(handle);
        let receiver = spawn(
            ReceiverConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                target_ms: 50.0,
                max_streams: 8,
                live_target_ms: None,
                mix_tap: None,
                bt_exempt_ports: Vec::new(),
            },
            control,
        )
        .expect("receiver");
        let origin = 0xFEED_0001;
        let socket = UdpSocket::bind("127.0.0.1:0").expect("socket");
        let frames = frames_for_ms(5.0);
        let send_seq = |socket: &UdpSocket, seq: u32| {
            let mut datagram = FrameHeader::new(Codec::RawF32, 5, 11, seq, now_us(), origin)
                .encode()
                .to_vec();
            for _ in 0..frames * CHANNELS {
                datagram.extend_from_slice(&0.1f32.to_le_bytes());
            }
            socket.send_to(&datagram, receiver.local_addr).expect("send");
        };

        for seq in 1..=10u32 {
            send_seq(&socket, seq);
        }
        std::thread::sleep(Duration::from_millis(150));
        let baseline = receiver.registry.snapshots()[0].packets;
        let id = receiver.registry.snapshots()[0].id;

        // Cross the timeout: the stream must freeze, not disappear. The
        // loopback peer classifies as the BT carrier, whose timeout is 8 s.
        std::thread::sleep(BT_PEER_TIMEOUT + Duration::from_millis(400));
        let snapshots = receiver.registry.snapshots();
        assert_eq!(snapshots.len(), 1, "frozen stream must stay registered");
        assert!(!snapshots[0].active, "frozen stream is not active");

        // The sender returns: same stream id, no rebuild, audio resumes.
        for seq in 11..=20u32 {
            send_seq(&socket, seq);
        }
        std::thread::sleep(Duration::from_millis(200));
        let snapshots = receiver.registry.snapshots();
        assert_eq!(snapshots.len(), 1, "resume must not create a second stream");
        assert_eq!(snapshots[0].id, id, "same stream identity across freeze");
        assert!(snapshots[0].active, "resumed stream is active");
        assert!(
            snapshots[0].packets >= baseline + 9,
            "resume packets accepted: baseline={baseline} now={}",
            snapshots[0].packets
        );
        receiver.stop();
        drop(mixer);
    }
}
