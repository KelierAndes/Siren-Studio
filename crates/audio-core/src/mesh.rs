//! Decentralized mesh node: receives and mixes every peer's audio while
//! sending local audio to all discovered peers — no explicit roles, no
//! manual addressing.
//!
//! Composition: a [`CenterEngine`] provides UDP receive + mixer + optional
//! local playback; a sender thread pushes the local capture to every peer;
//! a discovery thread keeps the peer set in sync over mDNS (TXT `id` filters
//! self-advertisement).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Bluetooth carrier pacing. With compact headers (24 B per packet instead of
/// 44 B) a full-rate stream costs ~70 kbps, so the overlay always transmits:
/// a 1/10 duty starved the fallback whenever Wi-Fi was present but not
/// delivering (buffer underruns, choppy audio).
const BT_DUTY_CYCLE: u64 = 1;

/// The Bluetooth overlay runs at a low rate: in a three-node star each link
/// carries two streams (own + relayed), and a piconet shared by two slaves
/// has roughly 600-900 kbps of usable airtime. 64 kbps payload + 44 B header
/// per 5 ms frame (≈70 kbps overhead) per stream overflows it.
const BT_OPUS_BITRATE: i32 = 24_000;

use crate::capture::{CaptureKind, CapturePipe, CaptureStats, ToneSource};
use crate::format::{frames_for_ms, CHANNELS};
use crate::engine::{CenterConfig, CenterEngine, CenterMode, SendCodec, SendSource};
use crate::format::SAMPLE_RATE;
use crate::metrics::now_us;
use crate::net::discovery;
use crate::net::udp::{FramePacker, UdpSender};
use crate::{CoreError, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MeshConfig {
    pub bind: String,
    pub port: u16,
    pub name: String,
    pub advertise: bool,
    /// Mix and play received audio locally.
    pub output: bool,
    /// Output device/sink name for local playback (default: system default).
    pub output_device: Option<String>,
    pub target_ms: f32,
    pub max_streams: usize,
    pub source: SendSource,
    pub codec: SendCodec,
    pub frame_ms: u16,
    pub freq: f64,
    pub amplitude: f32,
    /// Run mDNS discovery. Turn off for Bluetooth-only chains with static links.
    pub discovery: bool,
    /// Re-scan interval in seconds.
    pub discovery_secs: u64,
    /// Endpoints pinned without discovery (`ip:port`).
    pub static_peers: Vec<String>,
    /// Bluetooth origin bit (0..63). `None` derives one from the node id.
    pub bit: Option<u8>,
    /// Local Bluetooth adapter address advertised to peers for auto-role.
    pub bt: Option<String>,
    /// RFCOMM channel advertised to peers.
    pub bt_channel: Option<u8>,
}

impl Default for MeshConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0".into(),
            port: 47900,
            name: crate::engine::default_node_name(),
            advertise: true,
            output: true,
            output_device: None,
            target_ms: 30.0,
            max_streams: 32,
            source: SendSource::Tone,
            codec: SendCodec::Raw,
            frame_ms: 5,
            freq: 440.0,
            amplitude: 0.25,
            discovery: true,
            discovery_secs: 5,
            static_peers: Vec::new(),
            bit: None,
            bt: None,
            bt_channel: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MeshPeer {
    pub name: String,
    pub id: String,
    /// Numeric sender id carried in every audio frame (hex string for JS).
    #[serde(serialize_with = "crate::metrics::hex64")]
    pub uid: u64,
    pub endpoint: String,
    pub bit: Option<u8>,
    pub bt: Option<String>,
}

/// One node seen over mDNS. Used both for the Wi-Fi send list and for the
/// Bluetooth hub election.
#[derive(Clone, Debug, Serialize)]
pub struct TopologyPeer {
    pub name: String,
    pub id: String,
    #[serde(serialize_with = "crate::metrics::hex64")]
    pub uid: u64,
    pub endpoint: String,
    pub bit: Option<u8>,
    pub bt: Option<String>,
    pub bt_channel: Option<u8>,
}

/// State of the Bluetooth carrier. Wi-Fi is always on; this only describes
/// the best-effort second path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BtStatus {
    pub enabled: bool,
    /// `hub`, `leaf` or `off`.
    pub role: String,
    /// `listening`, `connected`, `connecting`, `waiting` or `error`.
    pub state: String,
    pub hub_id: Option<String>,
    pub hub_name: Option<String>,
    pub hub_addr: Option<String>,
    pub local_addr: Option<String>,
    pub channel: u8,
    pub links: usize,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub dropped: u64,
    pub last_error: Option<String>,
}

impl Default for BtStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            role: "off".into(),
            state: "off".into(),
            hub_id: None,
            hub_name: None,
            hub_addr: None,
            local_addr: None,
            channel: 16,
            links: 0,
            tx_bytes: 0,
            rx_bytes: 0,
            dropped: 0,
            last_error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MeshSenderStats {
    pub frames: u64,
    pub packets: u64,
    pub bytes: u64,
    pub kbps: f64,
    pub capture_frames: u64,
    pub capture_dropped_frames: u64,
    pub capture_silent_frames: u64,
    pub capture_read_errors: u64,
    pub capture_buffer_frames: usize,
    /// Level of the frames actually handed to the network, after echo
    /// cancellation — the number to watch for loop leakage.
    pub tx_rms_db: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MeshSnapshot {
    pub running: bool,
    pub stopping: bool,
    pub name: String,
    pub node_id: String,
    pub bit: u8,
    pub seconds: f64,
    pub bind: String,
    pub output: bool,
    pub peers: Vec<MeshPeer>,
    pub topology: Vec<TopologyPeer>,
    pub bt: BtStatus,
    pub sender: MeshSenderStats,
    pub center: crate::engine::CenterSnapshot,
    pub error: Option<String>,
}

struct MeshShared {
    stop: Arc<AtomicBool>,
    running: AtomicBool,
    started: Mutex<Instant>,
    name: String,
    node_id: String,
    uid: u64,
    bit: u8,
    output: bool,
    error: Mutex<Option<String>>,
    frames: AtomicU64,
    packets: AtomicU64,
    bytes: AtomicU64,
    capture: Mutex<Option<Arc<CaptureStats>>>,
    capture_buffer: AtomicUsize,
    tx_rms_db: AtomicU32,
    peers: Mutex<Vec<MeshPeer>>,
    topology: Arc<Mutex<Vec<TopologyPeer>>>,
    tunnel: Mutex<Option<SocketAddr>>,
    bt: Mutex<BtStatus>,
    /// Latest decoded audio per origin; the hub builds N-1 mixes from it.
    mix_tap: Arc<crate::net::udp::MixTap>,
    /// Set when this node currently acts as the Bluetooth hub.
    bt_hub: AtomicBool,
}

pub struct MeshEngine {
    center: CenterEngine,
    shared: Arc<MeshShared>,
    join: Mutex<Option<JoinHandle<()>>>,
}

fn random_node_id() -> String {
    let seed = now_us()
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ ((std::process::id() as u64) << 32);
    format!("{seed:016x}")
}

/// Stable numeric identity for a node id string. Older builds produced
/// 128-bit ids; the low 64 bits are enough and keep hub ordering stable.
pub fn node_uid(node_id: &str) -> u64 {
    let tail = if node_id.len() > 16 {
        &node_id[node_id.len() - 16..]
    } else {
        node_id
    };
    u64::from_str_radix(tail, 16).unwrap_or_else(|_| {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        node_id.hash(&mut hasher);
        hasher.finish()
    })
}

impl MeshEngine {
    pub fn start(config: MeshConfig) -> Result<Self> {
        if !(1..=60).contains(&config.frame_ms) {
            return Err(CoreError::InvalidArgument(
                "frame duration must be between 1 and 60 ms".into(),
            ));
        }
        if config.codec == SendCodec::Opus && !matches!(config.frame_ms, 5 | 10 | 20 | 40 | 60) {
            return Err(CoreError::InvalidArgument(
                "Opus supports 5/10/20/40/60 ms frames".into(),
            ));
        }
        // Deterministic runs (tests, multi-device rigs) can pin the identity;
        // otherwise a fresh random id is used. Hub election orders by uid, so
        // pinning keeps the same device hub across restarts.
        let node_id = std::env::var("SIREN_NODE_ID")
            .ok()
            .filter(|id| id.len() >= 16 && id.chars().all(|c| c.is_ascii_hexdigit()))
            .unwrap_or_else(random_node_id);
        let uid = node_uid(&node_id);
        let bit = config.bit.unwrap_or((uid % 64) as u8);
        let mix_tap = Arc::new(crate::net::udp::MixTap::new());
        let shared = Arc::new(MeshShared {
            stop: Arc::new(AtomicBool::new(false)),
            running: AtomicBool::new(false),
            started: Mutex::new(Instant::now()),
            name: config.name.clone(),
            node_id: node_id.clone(),
            uid,
            bit,
            output: config.output,
            error: Mutex::new(None),
            frames: AtomicU64::new(0),
            packets: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            capture: Mutex::new(None),
            capture_buffer: AtomicUsize::new(0),
            tx_rms_db: AtomicU32::new(0),
            peers: Mutex::new(Vec::new()),
            topology: Arc::new(Mutex::new(Vec::new())),
            tunnel: Mutex::new(None),
            bt: Mutex::new(BtStatus {
                channel: config.bt_channel.unwrap_or(16),
                ..BtStatus::default()
            }),
            mix_tap: mix_tap.clone(),
            bt_hub: AtomicBool::new(false),
        });

        let center = CenterEngine::start(CenterConfig {
            bind: config.bind.clone(),
            port: config.port,
            target_ms: config.target_ms,
            max_streams: config.max_streams,
            mode: if config.output {
                CenterMode::Playback
            } else {
                CenterMode::Silent
            },
            device: config.output_device.clone(),
            record_path: None,
            advertise: config.advertise,
            name: config.name.clone(),
            role: "mesh".into(),
            node_id: node_id.clone(),
            bit: Some(bit),
            bt: config.bt.clone(),
            bt_channel: config.bt_channel,
            mix_tap: Some(mix_tap),
        })?;

        let bind = center.bind_addr();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_shared = shared.clone();
        let join = std::thread::Builder::new()
            .name("siren-mesh-send".into())
            .spawn(move || {
                run_mesh_sender(config, node_id, bind, thread_shared, ready_tx);
            })
            .map_err(|e| CoreError::Backend(format!("spawn mesh sender: {e}")))?;

        match ready_rx.recv_timeout(Duration::from_secs(25)) {
            Ok(Ok(())) => Ok(Self {
                center,
                shared,
                join: Mutex::new(Some(join)),
            }),
            Ok(Err(message)) => {
                shared.stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err(CoreError::Backend(message))
            }
            Err(_) => {
                shared.stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err(CoreError::Backend(
                    "mesh node did not start within 25 seconds".into(),
                ))
            }
        }
    }

    /// Receive/mix side: gain, mute and buffer controls live on the center.
    pub fn center(&self) -> &CenterEngine {
        &self.center
    }

    pub fn snapshot(&self) -> MeshSnapshot {
        let running = self.shared.running.load(Ordering::Relaxed);
        let seconds = self.shared.started.lock().unwrap().elapsed().as_secs_f64();
        let bytes = self.shared.bytes.load(Ordering::Relaxed);
        let (capture_frames, dropped, silent, errors) = self
            .shared
            .capture
            .lock()
            .unwrap()
            .as_ref()
            .map(|stats| {
                (
                    stats.frames.load(Ordering::Relaxed),
                    stats.dropped_frames.load(Ordering::Relaxed),
                    stats.silent_frames.load(Ordering::Relaxed),
                    stats.read_errors.load(Ordering::Relaxed),
                )
            })
            .unwrap_or((0, 0, 0, 0));
        MeshSnapshot {
            running,
            stopping: running && self.shared.stop.load(Ordering::Relaxed),
            name: self.shared.name.clone(),
            node_id: self.shared.node_id.clone(),
            bit: self.shared.bit,
            seconds,
            bind: self.center.bind_addr(),
            output: self.shared.output,
            peers: self.shared.peers.lock().unwrap().clone(),
            topology: self.shared.topology.lock().unwrap().clone(),
            bt: self.shared.bt.lock().unwrap().clone(),
            sender: MeshSenderStats {
                frames: self.shared.frames.load(Ordering::Relaxed),
                packets: self.shared.packets.load(Ordering::Relaxed),
                bytes,
                kbps: if seconds > 0.1 {
                    bytes as f64 * 8.0 / seconds / 1000.0
                } else {
                    0.0
                },
                capture_frames,
                capture_dropped_frames: dropped,
                capture_silent_frames: silent,
                capture_read_errors: errors,
                capture_buffer_frames: self.shared.capture_buffer.load(Ordering::Relaxed),
                tx_rms_db: f32::from_bits(self.shared.tx_rms_db.load(Ordering::Relaxed)) as f64,
            },
            center: self.center.snapshot(),
            error: self.shared.error.lock().unwrap().clone(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    /// Stable node identity carried in every audio frame.
    pub fn uid(&self) -> u64 {
        self.shared.uid
    }

    pub fn node_id(&self) -> &str {
        &self.shared.node_id
    }

    pub fn name(&self) -> &str {
        &self.shared.name
    }

    pub fn bind_addr(&self) -> String {
        self.center.bind_addr()
    }

    pub fn bit(&self) -> u8 {
        self.shared.bit
    }

    pub fn topology(&self) -> Vec<TopologyPeer> {
        self.shared.topology.lock().unwrap().clone()
    }

    /// Point the sender at a Bluetooth tunnel, or `None` to disable the second
    /// carrier. Safe to call while audio is running.
    pub fn set_tunnel(&self, addr: Option<SocketAddr>) {
        *self.shared.tunnel.lock().unwrap() = addr;
    }

    pub fn set_bt_status(&self, status: BtStatus) {
        self.shared
            .bt_hub
            .store(status.role == "hub", Ordering::Relaxed);
        *self.shared.bt.lock().unwrap() = status;
    }

    pub fn stop(&self) {
        if !self.shared.stop.swap(true, Ordering::Relaxed) {
            crate::engine::stop_trace("mesh");
        }
        self.center.stop();
    }
}

impl Drop for MeshEngine {
    fn drop(&mut self) {
        log::info!("mesh: MeshEngine dropped");
        self.stop();
        if let Some(join) = self.join.lock().unwrap().take() {
            let _ = std::thread::Builder::new()
                .name("siren-mesh-reaper".into())
                .spawn(move || {
                    let _ = join.join();
                });
        }
    }
}

#[derive(Clone)]
struct SendPeer {
    name: String,
    id: String,
    addr: SocketAddr,
    bit: Option<u8>,
    bt: Option<String>,
    /// Configured with `--peer`, so it is never expected to show up in mDNS.
    pinned: bool,
    /// Last time this node was seen in a scan (or pinned). Unicast keeps
    /// flowing long after a scan misses it: a lost multicast answer must not
    /// split a mesh that is still reachable by IP.
    last_seen: Instant,
}

/// How long a node may stay absent from mDNS scans before we stop sending to
/// its last known address.
const PEER_KEEPALIVE_TOLERANCE: Duration = Duration::from_secs(60);
/// A peer younger than this is fresh enough to show in the UI peer list; older
/// entries keep being sent to but are no longer advertised as online.
const PEER_UI_FRESHNESS: Duration = Duration::from_secs(6);

/// Peer set shared between the discovery and sender threads.
type PeerMap = Arc<Mutex<HashMap<String, SendPeer>>>;

fn run_mesh_sender(
    config: MeshConfig,
    node_id: String,
    bind: String,
    shared: Arc<MeshShared>,
    ready_tx: mpsc::SyncSender<std::result::Result<(), String>>,
) {
    crate::platform::boost_thread_priority();
    crate::platform::begin_high_res_timer();
    let stop = &shared.stop;

    let codec = config.codec.to_codec();

    // Discovery thread keeps this map fresh; static peers are pinned.
    let peers: PeerMap = Arc::new(Mutex::new(HashMap::new()));
    for pinned in &config.static_peers {
        match resolve_ipv4(pinned) {
            Ok(addr) => {
                peers.lock().unwrap().insert(
                    addr.to_string(),
                    SendPeer {
                        name: "pinned".into(),
                        id: String::new(),
                        addr,
                        bit: None,
                        bt: None,
                        pinned: true,
                        last_seen: Instant::now(),
                    },
                );
            }
            Err(error) => {
                let message = error.to_string();
                *shared.error.lock().unwrap() = Some(message.clone());
                let _ = ready_tx.send(Err(message));
                return;
            }
        }
    }

    if config.discovery {
        let peers = peers.clone();
        let stop_flag = shared.stop.clone();
        let topology = shared.topology.clone();
        let _ = std::thread::Builder::new()
            .name("siren-mesh-discover".into())
            .spawn(move || {
                run_discovery_loop(
                    node_id,
                    bind,
                    peers,
                    topology,
                    stop_flag,
                    config.discovery_secs,
                );
            });
    }

    let mut tone = ToneSource::new(config.freq, config.amplitude);
    let mut pipe: Option<CapturePipe> = match config.source {
        SendSource::Tone => None,
        SendSource::Loopback | SendSource::Microphone => {
            let kind = if config.source == SendSource::Loopback {
                CaptureKind::Loopback
            } else {
                CaptureKind::Microphone
            };
            match CapturePipe::start(kind) {
                Ok(pipe) => {
                    *shared.capture.lock().unwrap() = Some(pipe.stats());
                    Some(pipe)
                }
                Err(error) => {
                    let message = error.to_string();
                    *shared.error.lock().unwrap() = Some(message.clone());
                    let _ = ready_tx.send(Err(message));
                    return;
                }
            }
        }
    };

    *shared.started.lock().unwrap() = Instant::now();
    shared.running.store(true, Ordering::Relaxed);
    if ready_tx.send(Ok(())).is_err() {
        stop.store(true, Ordering::Relaxed);
    }

    let frame_frames = frames_for_ms(config.frame_ms as f32);
    let frame_duration =
        Duration::from_micros((frame_frames as u64 * 1_000_000) / SAMPLE_RATE as u64);
    let mut pcm = vec![0.0f32; frame_frames * CHANNELS];
    let mut next_tick = Instant::now();
    let mut senders: HashMap<String, UdpSender> = HashMap::new();
    let mut tunnel_sender: Option<(SocketAddr, UdpSender)> = None;
    let mut total_packets: u64 = 0;
    let origin = shared.uid;
    let mut tx_report: u64 = 0;
    let mut packer = match FramePacker::new(codec, config.frame_ms, origin) {
        Ok(packer) => packer,
        Err(error) => {
            let message = error.to_string();
            *shared.error.lock().unwrap() = Some(message.clone());
            let _ = ready_tx.send(Err(message));
            return;
        }
    };
    // The Bluetooth carrier encodes the same timeline at a low Opus bitrate:
    // a narrow RFCOMM link carries every frame, so the fallback path stays
    // gapless even though Wi-Fi uses the full-rate codec.
    let mut bt_packer = match FramePacker::with_stream(
        crate::protocol::Codec::Opus,
        config.frame_ms,
        origin,
        packer.stream_id(),
        Some(BT_OPUS_BITRATE),
    ) {
        Ok(packer) => packer,
        Err(error) => {
            let message = error.to_string();
            *shared.error.lock().unwrap() = Some(message.clone());
            let _ = ready_tx.send(Err(message));
            return;
        }
    };
    let mut bt_tick: u64 = 0;
    // N-1 forwarding: the hub builds one mix per Bluetooth destination that
    // contains every source except that destination's own audio.
    let mix_ms = if config.frame_ms <= 5 { 10 } else { config.frame_ms };
    let mix_samples = frames_for_ms(mix_ms as f32) * CHANNELS;
    let mut mix_local: Vec<f32> = Vec::with_capacity(mix_samples * 2);
    let mut mix_encoders: HashMap<u64, FramePacker> = HashMap::new();
    let mut n1_logged = false;

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if next_tick > now {
            std::thread::sleep(next_tick - now);
        } else if now.duration_since(next_tick) > Duration::from_millis(150) {
            next_tick = now;
        }

        match pipe.as_mut() {
            Some(pipe) => {
                pipe.read_frame(&mut pcm);
                shared
                    .capture_buffer
                    .store(pipe.buffered_frames(), Ordering::Relaxed);
            }
            None => tone.fill(&mut pcm),
        }

        let power: f32 = pcm.iter().map(|sample| sample * sample).sum();
        let rms = (power / pcm.len() as f32).sqrt();
        shared
            .tx_rms_db
            .store((20.0 * rms.max(1e-6).log10()).to_bits(), Ordering::Relaxed);

        // Sync local senders with the current peer set.
        let current = peers.lock().unwrap().clone();
        senders.retain(|endpoint, _| current.contains_key(endpoint));
        for (endpoint, peer) in &current {
            if !senders.contains_key(endpoint) {
                match UdpSender::new(peer.addr, codec, config.frame_ms, origin) {
                    Ok(sender) => {
                        log::info!("mesh: sending to {} ({endpoint})", peer.name);
                        senders.insert(endpoint.clone(), sender);
                    }
                    Err(error) => {
                        log::warn!("mesh: cannot open sender for {endpoint}: {error}");
                    }
                }
            }
        }
        // The Bluetooth tunnel is a second, always-parallel carrier: the same
        // frame goes out over both paths and the receiver keeps whichever copy
        // arrives first.
        let tunnel_addr = *shared.tunnel.lock().unwrap();
        let stale = match (&tunnel_sender, tunnel_addr) {
            (Some((addr, _)), Some(current)) => *addr != current,
            (Some(_), None) => true,
            (None, Some(_)) => true,
            (None, None) => false,
        };
        if stale {
            tunnel_sender = match tunnel_addr {
                Some(addr) => match UdpSender::new(addr, codec, config.frame_ms, origin) {
                    Ok(sender) => {
                        log::info!("mesh: Bluetooth tunnel carrier on {addr}");
                        Some((addr, sender))
                    }
                    Err(error) => {
                        log::warn!("mesh: cannot open Bluetooth carrier: {error}");
                        None
                    }
                },
                None => None,
            };
        }

        let peer_snapshot: Vec<MeshPeer> = current
            .iter()
            .filter(|(_, peer)| {
                peer.pinned || peer.last_seen.elapsed() < PEER_UI_FRESHNESS
            })
            .map(|(_, peer)| MeshPeer {
                name: peer.name.clone(),
                id: peer.id.clone(),
                uid: node_uid(&peer.id),
                endpoint: peer.addr.to_string(),
                bit: peer.bit,
                bt: peer.bt.clone(),
            })
            .collect();
        *shared.peers.lock().unwrap() = peer_snapshot;

        // Encode once per tick per carrier: Wi-Fi peers share the primary
        // (full-rate) datagram, the Bluetooth tunnel gets its own low-rate
        // Opus datagram with the same origin/stream/seq, which is what lets
        // the receiver dedupe and switch carriers seamlessly.
        let tick_seq = packer.next_seq();
        let packet = match packer.build(&pcm) {
            Ok(packet) => packet.to_vec(),
            Err(error) => {
                log::warn!("mesh: encode failed: {error}");
                Vec::new()
            }
        };
        if !packet.is_empty() {
            for sender in senders.values_mut() {
                match sender.send_packet(&packet) {
                    Ok(_) => {
                        total_packets += 1;
                    }
                    Err(error) => {
                        log::warn!("mesh: send to {} failed: {error}", sender.target());
                    }
                }
            }
        }
        if let Some((_, sender)) = tunnel_sender.as_mut() {
            if shared.bt_hub.load(Ordering::Relaxed) {
                // Hub: one N-1 mix per destination link instead of forwarding
                // every stream separately. 10 ms Opus frames halve the header
                // overhead on top of halving the number of streams.
                mix_local.extend_from_slice(&pcm);
                if mix_local.len() >= mix_samples {
                    let local = &mix_local[mix_local.len() - mix_samples..];
                    let destinations = shared.mix_tap.snapshot(
                        mix_samples,
                        origin,
                        Duration::from_millis(500),
                    );
                    if !destinations.is_empty() && !n1_logged {
                        n1_logged = true;
                        log::info!(
                            "bt: N-1 relay active, 1 mixed stream per link ({} destination(s))",
                            destinations.len()
                        );
                    }
                    for (dest, _, dest_bt) in &destinations {
                        // Only a destination that already fell back to
                        // Bluetooth needs the mix; while its Wi-Fi is healthy
                        // it receives the individual streams directly.
                        if !*dest_bt {
                            continue;
                        }
                        let mut mix = local.to_vec();
                        let mut sources = 1u32;
                        let mut mask = 1u64 << (origin % 64);
                        for (other, block, _) in &destinations {
                            if other == dest {
                                continue;
                            }
                            sources += 1;
                            mask |= 1u64 << (other % 64);
                            let take = block.len().min(mix_samples);
                            let offset = mix_samples - take;
                            for (index, sample) in block[block.len() - take..].iter().enumerate() {
                                mix[offset + index] += *sample;
                            }
                        }
                        let gain = if sources > 1 { 0.7 } else { 1.0 };
                        for sample in mix.iter_mut() {
                            *sample *= gain;
                        }
                        if !mix_encoders.contains_key(dest) {
                            match FramePacker::with_stream(
                                crate::protocol::Codec::Opus,
                                mix_ms,
                                origin,
                                crate::protocol::MIX_STREAM_ID,
                                Some(BT_OPUS_BITRATE),
                            ) {
                                Ok(encoder) => {
                                    mix_encoders.insert(*dest, encoder);
                                }
                                Err(error) => {
                                    log::warn!("bt: mix encoder failed: {error}");
                                    continue;
                                }
                            }
                        }
                        let encoder = mix_encoders.get_mut(dest).unwrap();
                        // 20 B compact header: 8 B saved on every 10 ms mix.
                        let Ok(frame) = encoder.build_mix(&mix, mask) else {
                            continue;
                        };
                        // Targeted local envelope: the tunnel routes it to the
                        // link whose leaf origin matches `dest`.
                        let mut envelope = Vec::with_capacity(11 + frame.len());
                        envelope.push(0xB3);
                        envelope.extend_from_slice(&(frame.len() as u16).to_le_bytes());
                        envelope.extend_from_slice(&dest.to_le_bytes());
                        envelope.extend_from_slice(frame);
                        if sender.send_packet(&envelope).is_ok() {
                            total_packets += 1;
                        }
                    }
                    mix_local.clear();
                }
            } else {
                bt_tick = bt_tick.wrapping_add(1);
                if bt_tick % BT_DUTY_CYCLE == 0 {
                    if let Ok(bt_packet) = bt_packer.build_compact_with_seq(&pcm, tick_seq) {
                        let bt_packet = bt_packet.to_vec();
                        if let Some((_, sender)) = tunnel_sender.as_mut() {
                            match sender.send_packet(&bt_packet) {
                                Ok(_) => {
                                    total_packets += 1;
                                }
                                Err(error) => {
                                    log::debug!("mesh: Bluetooth send failed: {error}");
                                }
                            }
                        }
                    }
                }
            }
        }
        shared
            .frames
            .fetch_add(frame_frames as u64, Ordering::Relaxed);
        shared.packets.store(total_packets, Ordering::Relaxed);
        let mut bytes: u64 = senders.values().map(|s| s.bytes_sent()).sum();
        if let Some((_, sender)) = &tunnel_sender {
            bytes += sender.bytes_sent();
        }
        shared.bytes.store(bytes, Ordering::Relaxed);

        tx_report = tx_report.wrapping_add(1);
        if tx_report % 1000 == 0 {
            log::info!(
                "mesh: tx level {:.1} dB ({} Wi-Fi link(s), {} packet(s))",
                20.0 * rms.max(1e-6).log10(),
                senders.len(),
                total_packets
            );
        }

        next_tick += frame_duration;
    }

    if let Some(pipe) = pipe.take() {
        pipe.stop();
    }
    shared.capture_buffer.store(0, Ordering::Relaxed);
    shared.running.store(false, Ordering::Relaxed);
}

fn run_discovery_loop(
    node_id: String,
    bind: String,
    peers: PeerMap,
    topology: Arc<Mutex<Vec<TopologyPeer>>>,
    stop: Arc<AtomicBool>,
    _interval_secs: u64,
) {
    let own_port = bind.rsplit(':').next().unwrap_or("").to_string();
    while !stop.load(Ordering::Relaxed) {
        match discovery::discover(Duration::from_secs(2)) {
            Ok(found) => {
                let mut map = peers.lock().unwrap();
                let mut seen_topology = Vec::new();
                for peer in &found {
                    if peer.role != "mesh" {
                        continue;
                    }
                    // Filter self: TXT id, or same port with no id (defensive).
                    if peer.id == node_id
                        || (peer.id.is_empty() && peer.port.to_string() == own_port)
                    {
                        continue;
                    }
                    // A node with several network interfaces resolves once per
                    // address; dedupe by the node's unique id (falling back to
                    // name:port) so only one stream per node is sent and mixed.
                    let key = peer_key(peer);
                    let endpoint = format!("{}:{}", peer.addr, peer.port);
                    if !map.contains_key(&key) {
                        log::info!(
                            "mesh: discovered {endpoint} ({}) bit {:?} bt {:?}",
                            peer.name,
                            peer.bit,
                            peer.bt
                        );
                    }
                    if let Ok(addr) = resolve_ipv4(&endpoint) {
                        map.insert(
                            key,
                            SendPeer {
                                name: peer.name.clone(),
                                id: peer.id.clone(),
                                addr,
                                bit: peer.bit,
                                bt: peer.bt.clone(),
                                pinned: false,
                                last_seen: Instant::now(),
                            },
                        );
                    }
                    seen_topology.push(TopologyPeer {
                        name: peer.name.clone(),
                        id: peer.id.clone(),
                        uid: node_uid(&peer.id),
                        endpoint,
                        bit: peer.bit,
                        bt: peer.bt.clone(),
                        bt_channel: peer.bt_channel,
                    });
                }
                seen_topology.sort_by(|a, b| a.id.cmp(&b.id));
                *topology.lock().unwrap() = seen_topology;
                // Keep sending to a node long after its last mDNS answer: a
                // dropped multicast (AP isolation, IGMP snooping, a phone in
                // power save) is not the peer leaving, and unicast still works.
                let gone: Vec<String> = map
                    .iter()
                    .filter(|(_, peer)| {
                        !peer.pinned && peer.last_seen.elapsed() > PEER_KEEPALIVE_TOLERANCE
                    })
                    .map(|(key, _)| key.clone())
                    .collect();
                for key in gone {
                    log::info!("mesh: peer {key} left");
                    map.remove(&key);
                }
            }
            Err(error) => {
                log::warn!("mesh: discovery failed: {error}");
            }
        }
        std::thread::sleep(Duration::from_millis(800));
    }
}

fn peer_key(peer: &discovery::DiscoveredPeer) -> String {
    if peer.id.is_empty() {
        format!("{}:{}", peer.name, peer.port)
    } else {
        peer.id.clone()
    }
}

fn resolve_ipv4(endpoint: &str) -> Result<SocketAddr> {
    use std::net::ToSocketAddrs;
    endpoint
        .to_socket_addrs()
        .map_err(|e| CoreError::Discovery(format!("cannot resolve '{endpoint}': {e}")))?
        .find(|addr| addr.is_ipv4())
        .ok_or_else(|| CoreError::Discovery(format!("no ipv4 address for '{endpoint}'")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn mesh_nodes_exchange_audio_without_manual_discovery() {
        // Node B: silent receiver. Node A pins B statically (discovery itself
        // needs live mDNS, exercised on real devices); B must hear A's tone.
        let node_b = MeshEngine::start(MeshConfig {
            port: 0,
            output: false,
            advertise: false,
            ..Default::default()
        })
        .expect("start node b");
        let b_addr = node_b.center.bind_addr();

        let node_a = MeshEngine::start(MeshConfig {
            port: 0,
            output: false,
            advertise: false,
            static_peers: vec![b_addr],
            ..Default::default()
        })
        .expect("start node a");

        let mut heard = None;
        for _ in 0..50 {
            thread::sleep(Duration::from_millis(100));
            let snapshot = node_b.snapshot();
            if !snapshot.center.streams.is_empty() && snapshot.center.master.peak_hold_db > -40.0 {
                heard = Some(snapshot);
                break;
            }
        }
        let snapshot = heard.expect("node b never received node a's tone");
        assert_eq!(snapshot.center.streams.len(), 1);
        assert!(snapshot.center.receiver.datagrams > 0);
        assert_eq!(node_a.snapshot().sender.frames > 0, true);
        assert_eq!(node_a.snapshot().peers.len(), 1);

        node_a.stop();
        node_b.stop();
        for _ in 0..60 {
            if !node_a.is_running() && !node_b.is_running() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(!node_a.is_running());
        assert!(!node_b.is_running());
    }
}
