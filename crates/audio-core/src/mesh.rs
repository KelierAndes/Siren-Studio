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
use crate::engine::{CenterConfig, CenterEngine, CenterMode, SendCodec, SendSource};
use crate::format::SAMPLE_RATE;
use crate::format::{frames_for_ms, CHANNELS};
use crate::metrics::now_us;
use crate::net::discovery;
use crate::net::udp::{FramePacker, UdpSender};
use crate::protocol::Codec;
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
    /// Loopback ports that carry Wi-Fi (in-memory link simulators); passed
    /// through to the receiver's carrier classification.
    #[serde(skip)]
    pub bt_exempt_ports: Vec<u16>,
    /// F6: group password — PSK for AEAD-sealed Wi-Fi datagrams and the
    /// election signature. None = degraded, unencrypted mode (warned).
    #[serde(skip)]
    pub password: Option<String>,
    /// Stable node identity. `None` falls back to `SIREN_NODE_ID`, then to the
    /// id persisted for this machine, then to a fresh random id.
    pub node_id: Option<String>,
    /// Write the mixed output to a WAV file instead of playing it. Used to
    /// inspect a real 3-minute session frame by frame.
    pub record: Option<String>,
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
            node_id: None,
            record: None,
            bt_exempt_ports: Vec::new(),
            password: None,
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
    /// Heartbeat path health to this peer (None until samples exist).
    pub rtt_ms: Option<f64>,
    pub rtt_p99_ms: Option<f64>,
    pub loss_pct: Option<f64>,
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
    /// F16: the peer's advertised mesh protocol version (mDNS TXT `proto`).
    /// Drives capability gates such as the Bluetooth tunnel keepalive.
    #[serde(default)]
    pub proto: Option<u8>,
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
    /// Arrival-gap shape of the Bluetooth links over the last window: the
    /// difference between sniff parking, 2.4 GHz coexistence and piconet
    /// sharing lives here, not in the byte counters.
    pub gap_samples: u32,
    pub gap_p99_ms: u32,
    pub gap_max_ms: u32,
    pub gap_stalls_500ms: u32,
    pub rx_kb_per_s: f64,
    /// Tunnel keepalive: completed round trips, and links closed because a probe
    /// was never answered. This is the only Bluetooth liveness proof that a
    /// half-open link cannot fake with buffered writes.
    #[serde(default)]
    pub ka_rt: u64,
    #[serde(default)]
    pub ka_dead: u64,
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
            gap_samples: 0,
            gap_p99_ms: 0,
            gap_max_ms: 0,
            gap_stalls_500ms: 0,
            rx_kb_per_s: 0.0,
            ka_rt: 0,
            ka_dead: 0,
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
    /// E2: how long the capture has been silent enough to suppress the
    /// audio uplink (keeps flowing via heartbeats only).
    pub silent_ms: u64,
    /// E7/A11: unix us when the first audible capture frame was queued.
    pub first_frame_epoch_us: u64,
    /// F5: the codec actually being sent right now (changes under
    /// adaptive degradation).
    pub codec: String,
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
    /// Per-endpoint heartbeat health measured by the sender thread.
    health: Mutex<HashMap<String, crate::net::udp::PathHealth>>,
    /// F7: warned about a duplicate uid this session.
    clone_warned: Mutex<bool>,
    /// E2: total time the capture was silent enough to suppress uplink (ms).
    silent_tx_ms: AtomicU64,
    /// F15: learned node table (from gossip datagrams).
    gossip: Arc<crate::net::udp::GossipTable>,
    /// F5: current sending codec (adaptive switches update it).
    sender_codec: Mutex<Option<&'static str>>,
    /// E7/A11: unix us of the first audible captured frame — absolute
    /// end-to-end latency becomes measurable against the receiver's clock.
    first_frame_epoch_us: AtomicU64,
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
    let seed = now_us().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((std::process::id() as u64) << 32);
    format!("{seed:016x}")
}

/// Where this machine keeps its node identity. Deliberately not next to
/// `hub.json` on Linux: that one lives in `$XDG_RUNTIME_DIR` and vanishes on
/// reboot, while the identity has to outlive both.
/// F7: a cloned machine image produces two nodes with the same identity.
/// The running engine cannot change its uid safely, but persisting a fresh
/// id means the next start stops colliding. Called once per session when the
/// receiver sees our own uid coming from the network.
fn regenerate_node_id() {
    let Some(path) = node_id_path() else { return };
    let id = random_node_id();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, &id);
    log::warn!("mesh: duplicate uid detected on the network; regenerated node id for the next start");
}

fn node_id_path() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("LOCALAPPDATA").ok().map(|base| {
            std::path::PathBuf::from(base)
                .join("SirenStudio")
                .join("node.id")
        })
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("HOME")
            .ok()
            .map(|home| std::path::PathBuf::from(home).join(".cache/siren/node.id"))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

fn persisted_node_id() -> String {
    persisted_node_id_in(node_id_path())
}

/// Read this node's identity from `path`, creating and storing one if absent.
/// Public because Android picks its own storage directory in the FFI layer.
pub fn persisted_node_id_in(path: Option<std::path::PathBuf>) -> String {
    if let Some(path) = &path {
        if let Ok(text) = std::fs::read_to_string(path) {
            let id = text.trim().to_string();
            if id.len() >= 16 && id.chars().all(|c| c.is_ascii_hexdigit()) {
                return id;
            }
        }
    }
    let id = random_node_id();
    if let Some(path) = &path {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, &id);
    }
    id
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
        // Identity order: explicit config (tests, multi-node rigs) > SIREN_NODE_ID
        // > the id persisted for this machine > a fresh random id. A random id
        // per process made every restart re-run the hub election, so the mesh
        // picked a different hub and tore every Bluetooth link down.
        let node_id = [config.node_id.clone(), std::env::var("SIREN_NODE_ID").ok()]
            .into_iter()
            .flatten()
            .find(|id| id.len() >= 16 && id.chars().all(|c| c.is_ascii_hexdigit()))
            .unwrap_or_else(persisted_node_id);
        let uid = node_uid(&node_id);
        let bit = config.bit.unwrap_or((uid % 64) as u8);
        let mix_tap = Arc::new(crate::net::udp::MixTap::new());
        let gossip = Arc::new(crate::net::udp::GossipTable::new());
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
            health: Mutex::new(HashMap::new()),
            clone_warned: Mutex::new(false),
            silent_tx_ms: AtomicU64::new(0),
            first_frame_epoch_us: AtomicU64::new(0),
            gossip: gossip.clone(),
            sender_codec: Mutex::new(Some(config.codec.to_codec().as_str())),
            topology: Arc::new(Mutex::new(Vec::new())),
            tunnel: Mutex::new(None),
            bt: Mutex::new(BtStatus {
                channel: config.bt_channel.unwrap_or(16),
                ..BtStatus::default()
            }),
            mix_tap: mix_tap.clone(),
            bt_hub: AtomicBool::new(false),
        });

        // Static peers carrying identity or Bluetooth hints seed the
        // topology, so hub election works without mDNS (offline rigs,
        // static meshes, in-memory tests).
        let pinned_topology: Vec<TopologyPeer> = config
            .static_peers
            .iter()
            .filter_map(|spec| match parse_pinned_peer(spec) {
                Ok(info) => (info.id.is_some() || info.bt.is_some()).then_some(info),
                Err(error) => {
                    log::warn!("mesh: ignoring bad static peer '{spec}': {error}");
                    None
                }
            })
            .map(|info| {
                let id = info.id.unwrap_or_default();
                TopologyPeer {
                    name: info.name.unwrap_or_else(|| "pinned".into()),
                    uid: node_uid(&id),
                    id,
                    endpoint: info.addr.to_string(),
                    bit: info.bit,
                    bt: info.bt,
                    bt_channel: None,
                    // A pinned row carries no mDNS TXT, so the peer's version
                    // is unknown: capability gates stay closed.
                    proto: None,
                }
            })
            .collect();
        if !pinned_topology.is_empty() {
            *shared.topology.lock().unwrap() = pinned_topology;
        }

        let center = CenterEngine::start(CenterConfig {
            bind: config.bind.clone(),
            port: config.port,
            target_ms: config.target_ms,
            max_streams: config.max_streams,
            mode: if config.record.is_some() {
                CenterMode::Record
            } else if config.output {
                CenterMode::Playback
            } else {
                CenterMode::Silent
            },
            device: config.output_device.clone(),
            record_path: config.record.clone(),
            advertise: config.advertise,
            name: config.name.clone(),
            role: "mesh".into(),
            node_id: node_id.clone(),
            bit: Some(bit),
            bt: config.bt.clone(),
            bt_channel: config.bt_channel,
            mix_tap: Some(mix_tap),
            bt_exempt_ports: config.bt_exempt_ports.clone(),
            password: config.password.clone(),
            gossip: Some(gossip.clone()),
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
                silent_ms: self.shared.silent_tx_ms.load(Ordering::Relaxed),
                first_frame_epoch_us: self.shared.first_frame_epoch_us.load(Ordering::Relaxed),
                codec: self
                    .shared
                    .sender_codec
                    .lock()
                    .unwrap()
                    .unwrap_or("raw")
                    .to_string(),
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

    /// Stop and block until the sender thread has exited. A headless run that
    /// returns before this leaves the recorder mid-file: the WAV header would
    /// keep its placeholder sizes and the recording is unreadable.
    pub fn wait_until_stopped(&self) {
        self.stop();
        if let Some(join) = self.join.lock().unwrap().take() {
            let _ = join.join();
        }
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
const PEER_KEEPALIVE_TOLERANCE: Duration = Duration::from_secs(30);
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
        match parse_pinned_peer(pinned) {
            Ok(info) => {
                peers.lock().unwrap().insert(
                    info.addr.to_string(),
                    SendPeer {
                        name: info.name.clone().unwrap_or_else(|| "pinned".into()),
                        id: info.id.clone().unwrap_or_default(),
                        addr: info.addr,
                        bit: info.bit,
                        bt: info.bt,
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
        // F6: when a group password is set, election credentials (bit/bt)
        // from peers whose signature does not verify are stripped — a
        // passwordless node cannot take over the hub election.
        let election_key = config
            .password
            .as_ref()
            .map(|pw| crate::net::crypto::MeshKey::from_password(pw));
        let discover_node_id = node_id.clone();
        let discover_bind = bind.clone();
        let _ = std::thread::Builder::new()
            .name("siren-mesh-discover".into())
            .spawn(move || {
                run_discovery_loop(
                    discover_node_id,
                    discover_bind,
                    peers,
                    topology,
                    stop_flag,
                    config.discovery_secs,
                    election_key,
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
    // F5/E6: per-endpoint adaptive codec state. `0` = configured codec;
    // while running Opus the counter tracks consecutive healthy ticks.
    let mut adaptive: HashMap<String, u32> = HashMap::new();
    // F5 anti-flap: each degrade->restore cycle on one endpoint doubles the
    // healthy time required before the next restore (30 s, 1 min, 2, 4, 8,
    // 16 min cap). A genuinely improved link is still re-probed, just less
    // and less often; a capped link settles on Opus instead of flapping.
    let mut restore_attempts: HashMap<String, u32> = HashMap::new();
    const ADAPT_RTT_US: u64 = 400_000;
    const ADAPT_HEALTHY_US: u64 = 200_000;
    // The adaptive block runs once per captured frame, so a healthy window
    // must be scaled by the frame period: a fixed tick count timed 0.3 s at
    // 5 ms frames while the comment claimed 30 s.
    let adapt_back_ticks = (30_000 / u32::from(config.frame_ms.max(1))).max(1);
    const ADAPT_OPUS_BITRATE: i32 = 48_000;
    let mut senders: HashMap<String, UdpSender> = HashMap::new();
    let mut tunnel_sender: Option<(SocketAddr, UdpSender)> = None;
    let mut total_packets: u64 = 0;
    let origin = shared.uid;
    let mut hb_counter: u32 = 0;
    if config.password.is_none() {
        // F6 degraded mode: no group password, traffic is plaintext.
        log::warn!("mesh: NO group password set — running in unencrypted degraded mode (F6)");
    }
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
    let mix_ms = if config.frame_ms <= 5 {
        10
    } else {
        config.frame_ms
    };
    let mix_samples = frames_for_ms(mix_ms as f32) * CHANNELS;
    let mut mix_local: Vec<f32> = Vec::with_capacity(mix_samples * 2);
    let mut mix_encoders: HashMap<u64, FramePacker> = HashMap::new();
    let mut n1_logged = false;
    let mut cap_zero_since: Option<Instant> = None;
    let mut cap_zero_last = false;

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
                // I3 at the capture point: a run of all-zero blocks means the
                // source itself went silent, before any network or mixer is
                // involved. Only the sender that captures can tell this apart
                // from a mixer hole.
                let silent = pcm.iter().all(|sample| *sample == 0.0);
                if silent {
                    if !cap_zero_last {
                        cap_zero_since = Some(Instant::now());
                    }
                } else if cap_zero_last {
                    if let Some(at) = cap_zero_since {
                        let run_ms = at.elapsed().as_secs_f64() * 1000.0;
                        if run_ms >= 5.0 {
                            log::info!("trace: zerorun point=capture run_ms={run_ms:.1}");
                        }
                    }
                    cap_zero_since = None;
                }
                cap_zero_last = silent;
                if !silent {
                    // E7: mark the first audible frame so the absolute
                    // capture->playback latency can be measured (A11).
                    let _ = shared.first_frame_epoch_us.compare_exchange(
                        0,
                        crate::metrics::now_us(),
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    );
                }
            }
            None => tone.fill(&mut pcm),
        }
        // E2/A5: a source that captures pure digital silence (app not
        // playing yet) used to stream 8 s of full-water-level zeros — every
        // health metric green, nothing audible. After 200 ms of continuous
        // all-zero capture, stop sending audio frames (heartbeats keep the
        // path measured); resume on the first non-zero frame.
        let capture_silent_200ms = cap_zero_since
            .map(|at| at.elapsed() >= Duration::from_millis(200))
            .unwrap_or(false);
        if capture_silent_200ms {
            shared
                .silent_tx_ms
                .fetch_add(config.frame_ms as u64, Ordering::Relaxed);
        }

        let power: f32 = pcm.iter().map(|sample| sample * sample).sum();
        let rms = (power / pcm.len() as f32).sqrt();
        shared
            .tx_rms_db
            .store((20.0 * rms.max(1e-6).log10()).to_bits(), Ordering::Relaxed);

        // E4/A6: a peer whose IP changed (phone roaming between the AP's
        // addresses, or a restarted node) kept receiving our audio at a dead
        // endpoint for ~20 s. The audio we RECEIVE carries the peer's current
        // IP — prefer it over a stale discovery entry (port stays from
        // discovery: the incoming port is an ephemeral sender port).
        //
        // F5: hearing the peer's audio is ALSO proof of liveness — under
        // congestion mDNS answers get flaky and the 30 s TTL would expire a
        // peer whose uplink still flows, dropping the sender right when the
        // adaptive downgrade needs a subject. Refresh last_seen from audio.
        {
            let mut peers_now = peers.lock().unwrap();
            for peer in peers_now.values_mut() {
                let uid = node_uid(&peer.id);
                if uid == 0 {
                    continue;
                }
                // The IP may only be replaced by a REAL Wi-Fi source: audio
                // delivered through the Bluetooth tunnel arrives from the
                // tunnel's loopback inject socket, and rewriting the peer to
                // 127.0.0.1 pointed our downlink at ourselves (T-D finding).
                // Liveness still counts for any carrier.
                if let Some((addr, at)) = shared.mix_tap.last_addr(uid, Duration::from_secs(3)) {
                    if !addr.ip().is_loopback() && addr.ip() != peer.addr.ip() {
                        log::info!(
                            "mesh: peer {} moved {} -> {} (from received audio)",
                            peer.name,
                            peer.addr.ip(),
                            addr.ip()
                        );
                        peer.addr.set_ip(addr.ip());
                    }
                    peer.last_seen = at;
                }
            }
        }

        // Sync local senders with the current peer set.
        let current = peers.lock().unwrap().clone();
        senders.retain(|endpoint, _| current.contains_key(endpoint));
        let mesh_key: Option<Arc<crate::net::crypto::MeshKey>> = config
            .password
            .as_ref()
            .map(|pw| Arc::new(crate::net::crypto::MeshKey::from_password(pw)));
        for (endpoint, peer) in &current {
            let existing = senders.get_mut(endpoint);
            if let Some(sender) = existing {
                // E4 只改 peer.addr 还不够：senders 以 node id 为键，键不变就
                // 永远不重建发送端，包继续发往旧地址（实测 NAS 的 topology 里
                // PC 已换成 .214，PC 却始终收不到 Linux 的一路流）。
                //
                // 但也不能每次抖动都跟着改绑：多网卡机器（NAS 的 enp1s0 是默认路由，
                // wlp3s0 只是次要出口）的 mDNS 会轮流给两个 A 记录，peer.addr 就来回翻，
                // 跟着翻等于每几秒重连一次 socket（3 分钟健康基线里量到 39 次）。
                // 只有当前目标确实听不到心跳应答时，才换发送地址。
                let health = sender.path_health();
                let unanswered =
                    health.sent >= 4 && (health.samples == 0 || health.loss_pct >= 40.0);
                if unanswered {
                    match sender.retarget(peer.addr) {
                        Ok(true) => log::info!(
                            "mesh: {endpoint} retargeted to {} (downlink unanswered, hb loss {:.0} %)",
                            peer.addr,
                            health.loss_pct
                        ),
                        Ok(false) => {}
                        Err(error) => log::warn!("mesh: retarget {endpoint} failed: {error}"),
                    }
                }
            } else {
                match UdpSender::new(peer.addr, codec, config.frame_ms, origin) {
                    Ok(mut sender) => {
                        if let Some(key) = &mesh_key {
                            sender.set_key(key.clone());
                        }
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

        let health = shared.health.lock().unwrap().clone();
        let peer_snapshot = build_peer_snapshot(&current, &health);
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
        if !packet.is_empty() && !capture_silent_200ms {
            let mut drop_endpoints: Vec<String> = Vec::new();
            for (endpoint, sender) in senders.iter_mut() {
                match sender.send_packet(&packet) {
                    Ok(_) => {
                        total_packets += 1;
                    }
                    Err(error) => {
                        log::warn!("mesh: send to {} failed: {error}", sender.target());
                        // E4: three consecutive failures usually mean the
                        // endpoint is dead (refused / unreachable). Drop the
                        // peer so discovery re-finds its new address; pinned
                        // peers have nothing to re-find and stay.
                        if sender.failures() >= 3 {
                            let pinned = current
                                .get(endpoint)
                                .map(|peer| peer.pinned)
                                .unwrap_or(false);
                            if !pinned {
                                drop_endpoints.push(endpoint.clone());
                            }
                        }
                    }
                }
            }
            if !drop_endpoints.is_empty() {
                let mut peers_now = peers.lock().unwrap();
                for endpoint in &drop_endpoints {
                    if peers_now.remove(endpoint).is_some() {
                        log::info!("mesh: dropped stale endpoint {endpoint} for rediscovery");
                    }
                }
            }
        }
        // F7: if the receiver decoded audio carrying OUR uid, a cloned
        // image is on the network. Persist a regenerated id for the next
        // start (the running engine cannot change its uid safely).
        if hb_counter % (crate::net::udp::HB_TICKS as u32 * 10) == 0 {
            let clones = shared.mix_tap.snapshot(mix_samples, 0, Duration::from_secs(5));
            if clones.iter().any(|(other, _, _, _)| *other == origin)
                && !*shared.clone_warned.lock().unwrap()
            {
                *shared.clone_warned.lock().unwrap() = true;
                log::warn!(
                    "mesh: our own uid {:016x} arrived from the network — cloned node image (F7)",
                    origin
                );
                regenerate_node_id();
            }
        }
        // Heartbeat (F10): request every 500 ms, drain echoes every tick,
        // and publish the rolling path health for the snapshot/UI.
        hb_counter += 1;
        // F15: every 5 s, advertise our node table to every peer and promote
        // freshly learned nodes into senders (mDNS-free mesh healing).
        let gossip_due = hb_counter % (crate::net::udp::HB_TICKS as u32 * 10) == 0;
        if gossip_due {
            let known: Vec<(String, SocketAddr)> = current
                .values()
                .filter(|peer| !peer.id.is_empty() && peer.id != node_id)
                .map(|peer| (peer.id.clone(), peer.addr))
                .collect();
            let self_addr: SocketAddr = bind
                .parse()
                .unwrap_or_else(|_| "127.0.0.1:47900".parse().unwrap());
            let datagram = crate::net::udp::build_gossip(&node_id, self_addr, &known);
            for sender in senders.values_mut() {
                let _ = sender.send_packet(&datagram);
            }
            for (id, addr) in shared.gossip.fresh(Duration::from_secs(30)) {
                if id == node_id {
                    continue;
                }
                let already = current.values().any(|peer| peer.id == id);
                if !already {
                    log::info!("mesh: gossip learned node {id} at {addr}");
                    peers.lock().unwrap().insert(
                        addr.to_string(),
                        SendPeer {
                            name: "gossip".into(),
                            id: id.clone(),
                            addr,
                            bit: None,
                            bt: None,
                            pinned: false,
                            last_seen: Instant::now(),
                        },
                    );
                }
            }
        }
        let refresh_routes = hb_counter % (crate::net::udp::HB_TICKS as u32 * 60) == 0;
        for (endpoint, sender) in senders.iter_mut() {
            sender.poll_heartbeat();
            if refresh_routes {
                // F13：connected UDP 会把 connect 时选好的路由缓存下来。手机
                // Wi-Fi 关掉再打开之后，发送方向仍留在旧接口（实测卡在蓝牙），
                // 每 30 s 重绑一次让它回到当前默认路由。
                sender.refresh_route();
            }
            if hb_counter % (crate::net::udp::HB_TICKS as u32) == 0 {
                sender.send_heartbeat();
            }
            // F5: degrade to Opus when the path stays slow OR is so
            // saturated that heartbeat echoes stop coming back (a full
            // queue means zero RTT samples but ~100 % heartbeat loss —
            // rtt alone is blind exactly when it matters). The switch
            // keeps the stream id and sequence, so the receiver hears a
            // carrier-style change.
            let health = sender.path_health();
            let saturated = health.samples > 0 && health.loss_pct >= 40.0;
            let no_echo = health.samples == 0 && health.sent >= 4;
            let slow = health.samples >= 4 && health.rtt_p99_us > ADAPT_RTT_US;
            let entry = adaptive.entry(endpoint.clone()).or_insert(0);
            let degraded = saturated || no_echo || slow;
            if degraded && *entry == 0 {
                *entry = 1;
            } else if degraded && *entry >= 1 {
                *entry += 1;
                if *entry >= 2 && sender.codec() != Codec::Opus {
                    match sender.set_codec(Codec::Opus, Some(ADAPT_OPUS_BITRATE)) {
                        Ok(()) => {
                            log::info!(
                                "mesh: {endpoint} degraded (rtt p99 {:.0} ms, hb loss {:.0} %) — switching to Opus {}",
                                health.rtt_p99_us as f64 / 1000.0,
                                health.loss_pct,
                                ADAPT_OPUS_BITRATE
                            );
                            *shared.sender_codec.lock().unwrap() = Some(Codec::Opus.as_str());
                            *restore_attempts.entry(endpoint.clone()).or_insert(0) += 1;
                        }
                        Err(error) => log::warn!("mesh: adaptive switch failed: {error}"),
                    }
                }
            } else if !degraded && *entry > 0 {
                let healthy = health.rtt_p99_us <= ADAPT_HEALTHY_US && health.loss_pct <= 5.0;
                if healthy {
                    *entry += 1;
                    let attempts = restore_attempts.get(endpoint).copied().unwrap_or(0);
                    let backoff = adapt_back_ticks << attempts.min(5);
                    if *entry - 1 >= backoff && sender.codec() != codec {
                        match sender.set_codec(codec, None) {
                            Ok(()) => {
                                log::info!(
                                    "mesh: {endpoint} recovered — restoring {}",
                                    codec.as_str()
                                );
                                *shared.sender_codec.lock().unwrap() = Some(codec.as_str());
                            }
                            Err(error) => log::warn!("mesh: adaptive restore failed: {error}"),
                        }
                        adaptive.insert(endpoint.clone(), 0);
                    }
                } else {
                    *entry = 1;
                }
            } else if !degraded {
                *entry = 0;
            }
            if hb_counter % (crate::net::udp::HB_TICKS as u32 * 40) == 0 {
                let (sent, lost, samples) = sender.heartbeat_counts();
                log::debug!(
                    "hb → {endpoint}: sent {sent}, unanswered {lost}, samples {samples}"
                );
            }
            shared
                .health
                .lock()
                .unwrap()
                .insert(endpoint.clone(), health);
        }
        if let Some((_, sender)) = tunnel_sender.as_mut() {
            if shared.bt_hub.load(Ordering::Relaxed) {
                // Hub: one N-1 mix per destination link instead of forwarding
                // every stream separately. 10 ms Opus frames halve the header
                // overhead on top of halving the number of streams.
                mix_local.extend_from_slice(&pcm);
                if mix_local.len() >= mix_samples {
                    let local = &mix_local[mix_local.len() - mix_samples..];
                    let destinations =
                        shared
                            .mix_tap
                            .snapshot(mix_samples, origin, Duration::from_millis(500));
                    if !destinations.is_empty() && !n1_logged {
                        n1_logged = true;
                        log::info!(
                            "bt: N-1 relay active, 1 mixed stream per link ({} destination(s))",
                            destinations.len()
                        );
                    }
                    for (dest, _, dest_bt, _) in &destinations {
                        // Only a destination that already fell back to
                        // Bluetooth needs the mix; while its Wi-Fi is healthy
                        // it receives the individual streams directly.
                        if !*dest_bt {
                            continue;
                        }
                        let mut mix = local.to_vec();
                        let mut sources = 1u32;
                        let mut mask = 1u64 << (origin % 64);
                        for (other, block, _, _) in &destinations {
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
    election_key: Option<crate::net::crypto::MeshKey>,
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
                    // F16: a peer speaking a NEWER mesh protocol than us may
                    // emit frames we cannot parse — skip it (older peers and
                    // proto-less legacy peers are accepted).
                    if let Some(proto) = peer.proto {
                        if proto > crate::protocol::MESH_PROTO_VERSION {
                            static PROTO_SKIP_LOGGED: std::sync::atomic::AtomicU64 =
                                std::sync::atomic::AtomicU64::new(0);
                            if PROTO_SKIP_LOGGED.fetch_add(1, Ordering::Relaxed) < 4 {
                                log::warn!(
                                    "mesh: skipping {} ({}) — protocol {proto} > ours {}",
                                    peer.name,
                                    peer.id,
                                    crate::protocol::MESH_PROTO_VERSION
                                );
                            }
                            continue;
                        }
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
                    // F6: with a group key, election credentials without a
                    // valid signature are stripped — the peer can still send
                    // (its audio is AEAD-gated anyway) but cannot win the
                    // hub election or advertise a dial target.
                    let (bit, bt, bt_channel) = match &election_key {
                        Some(key) => {
                            let valid = peer.sig.as_deref().is_some_and(|sig| {
                                key.verify_election(&peer.id, peer.bit, peer.bt.as_deref(), peer.bt_channel, sig)
                            });
                            if valid {
                                (peer.bit, peer.bt.clone(), peer.bt_channel)
                            } else {
                                if peer.bt.is_some() || peer.bit.is_some() {
                                    log::warn!(
                                        "mesh: peer {} ({}) has unsigned/invalid election credentials — ignored",
                                        peer.name,
                                        peer.id
                                    );
                                }
                                (None, None, None)
                            }
                        }
                        None => (peer.bit, peer.bt.clone(), peer.bt_channel),
                    };
                    if let Ok(addr) = resolve_ipv4(&endpoint) {
                        map.insert(
                            key,
                            SendPeer {
                                name: peer.name.clone(),
                                id: peer.id.clone(),
                                addr,
                                bit,
                                bt: bt.clone(),
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
                        bit,
                        bt,
                        bt_channel,
                        proto: peer.proto,
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

/// Online nodes for the UI snapshot, with their heartbeat path health.
///
/// The `peers` map, `senders` and the heartbeat `health` map all key on
/// `peer_key`, so the lookup has to use that key too; the socket address is a
/// different string and looking it up silently reports null RTT for every peer.
fn build_peer_snapshot(
    current: &HashMap<String, SendPeer>,
    health: &HashMap<String, crate::net::udp::PathHealth>,
) -> Vec<MeshPeer> {
    // 同一台物理机器可能占两个键：mDNS 发现的条目以 node id 为键，gossip 学到的
    // 以地址为键（name 恒为 "gossip"），同名实例还会被 mDNS 自动加 "(2)" 后缀。
    // 不去重就会出现"顶栏写 3 台、卡片只有 1 张"这种自相矛盾的读数。
    let mut fresh: HashMap<String, (&String, &SendPeer)> = HashMap::new();
    for (key, peer) in current {
        if !(peer.pinned || peer.last_seen.elapsed() < PEER_UI_FRESHNESS) {
            continue;
        }
        let group = if peer.id.is_empty() {
            key.clone()
        } else {
            peer.id.clone()
        };
        match fresh.get(&group) {
            Some((_, best)) if best.last_seen >= peer.last_seen => {}
            _ => {
                fresh.insert(group, (key, peer));
            }
        }
    }
    let known_ips: Vec<std::net::IpAddr> = fresh
        .values()
        .filter(|(_, peer)| !peer.id.is_empty())
        .map(|(_, peer)| peer.addr.ip())
        .collect();

    let mut rows: Vec<MeshPeer> = Vec::new();
    for (key, peer) in fresh.values() {
        // 没有 id 的 gossip 行如果与某个已收录条目同 IP，就是同一台机器的重复计数。
        if peer.id.is_empty() && known_ips.contains(&peer.addr.ip()) {
            continue;
        }
        let measured = health.get(key.as_str()).filter(|h| h.samples > 0);
        rows.push(MeshPeer {
            name: peer.name.clone(),
            id: peer.id.clone(),
            uid: node_uid(&peer.id),
            endpoint: peer.addr.to_string(),
            bit: peer.bit,
            bt: peer.bt.clone(),
            rtt_ms: measured.map(|h| h.rtt_p50_us as f64 / 1000.0),
            rtt_p99_ms: measured.map(|h| h.rtt_p99_us as f64 / 1000.0),
            loss_pct: measured.map(|h| h.loss_pct),
        });
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name).then(a.endpoint.cmp(&b.endpoint)));
    rows
}

fn resolve_ipv4(endpoint: &str) -> Result<SocketAddr> {
    use std::net::ToSocketAddrs;
    endpoint
        .to_socket_addrs()
        .map_err(|e| CoreError::Discovery(format!("cannot resolve '{endpoint}': {e}")))?
        .find(|addr| addr.is_ipv4())
        .ok_or_else(|| CoreError::Discovery(format!("no ipv4 address for '{endpoint}'")))
}

/// A pinned peer parsed out of `MeshConfig::static_peers`.
#[derive(Clone, Debug)]
pub struct PinnedPeer {
    pub addr: SocketAddr,
    pub bt: Option<String>,
    pub bit: Option<u8>,
    pub name: Option<String>,
    pub id: Option<String>,
}

/// Parse a static peer spec: `ip:port`, optionally followed by `?`-params
/// that stand in for the mDNS TXT records real discovery would deliver —
/// `bt=<address>` and `id=<s>` make the peer take part in the Bluetooth hub
/// election, `bit=<n>` and `name=<s>` enrich the topology. Unknown params
/// are rejected so a typo cannot silently disable the overlay.
pub fn parse_pinned_peer(spec: &str) -> Result<PinnedPeer> {
    let (endpoint, params) = spec.split_once('?').unwrap_or((spec, ""));
    let mut peer = PinnedPeer {
        addr: resolve_ipv4(endpoint)?,
        bt: None,
        bit: None,
        name: None,
        id: None,
    };
    for pair in params.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            CoreError::InvalidArgument(format!(
                "pinned peer param must be key=value: '{pair}' in '{spec}'"
            ))
        })?;
        match key {
            "bt" => peer.bt = Some(value.to_string()),
            "bit" => {
                peer.bit = Some(value.parse().map_err(|_| {
                    CoreError::InvalidArgument(format!("bad bit '{value}' in '{spec}'"))
                })?)
            }
            "name" => peer.name = Some(value.to_string()),
            "id" => peer.id = Some(value.to_string()),
            other => {
                return Err(CoreError::InvalidArgument(format!(
                    "unknown pinned peer param '{other}' in '{spec}'"
                )))
            }
        }
    }
    Ok(peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn snapshot_reports_heartbeat_health_under_the_same_key_as_the_sender_list() {
        // The peers map is keyed by node id, not by "ip:port". A snapshot that
        // looked health up by address reported null RTT for every peer while the
        // heartbeats themselves were answering fine.
        let addr: SocketAddr = "192.168.2.124:47900".parse().unwrap();
        let peers = HashMap::from([(
            "72a176cebfe88db4".to_string(),
            SendPeer {
                name: "Phone".into(),
                id: "72a176cebfe88db4".into(),
                addr,
                bit: None,
                bt: None,
                pinned: true,
                last_seen: Instant::now(),
            },
        )]);
        let health = HashMap::from([(
            "72a176cebfe88db4".to_string(),
            crate::net::udp::PathHealth {
                rtt_p50_us: 8_000,
                rtt_p99_us: 21_000,
                loss_pct: 2.5,
                samples: 40,
                sent: 0,
        },
        )]);

        let snapshot = build_peer_snapshot(&peers, &health);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].endpoint, addr.to_string());
        assert_eq!(snapshot[0].rtt_ms, Some(8.0));
        assert_eq!(snapshot[0].rtt_p99_ms, Some(21.0));
        assert_eq!(snapshot[0].loss_pct, Some(2.5));

        // No samples yet means "unknown", not "0 ms".
        let silent = HashMap::from([(
            "72a176cebfe88db4".to_string(),
            crate::net::udp::PathHealth::default(),
        )]);
        let snapshot = build_peer_snapshot(&peers, &silent);
        assert_eq!(snapshot[0].rtt_ms, None);
    }

    #[test]
    fn peer_snapshot_counts_one_node_once_across_gossip_and_discovery() {
        // 顶栏写"3 台"而卡片只有 1 张的根源：mDNS 条目以 node id 为键，gossip
        // 学到的同一条以地址为键（name 恒为 "gossip"），两者都进快照就是重复计数。
        let addr: SocketAddr = "192.168.2.107:47900".parse().unwrap();
        let now = Instant::now();
        let peers = HashMap::from([
            (
                "687ad46a475a5f7c".to_string(),
                SendPeer {
                    name: "SIREN-PC (mesh)".into(),
                    id: "687ad46a475a5f7c".into(),
                    addr,
                    bit: Some(60),
                    bt: Some("E0:8F:4C:16:9F:58".into()),
                    pinned: false,
                    last_seen: now,
                },
            ),
            (
                addr.to_string(),
                SendPeer {
                    name: "gossip".into(),
                    id: String::new(),
                    addr,
                    bit: None,
                    bt: None,
                    pinned: false,
                    last_seen: now,
                },
            ),
            (
                "72a176cebfe88db4".to_string(),
                SendPeer {
                    name: "Siren Node (mesh)".into(),
                    id: "72a176cebfe88db4".into(),
                    addr: "192.168.2.248:47900".parse().unwrap(),
                    bit: Some(52),
                    bt: None,
                    pinned: false,
                    last_seen: now,
                },
            ),
        ]);
        let snapshot = build_peer_snapshot(&peers, &HashMap::new());
        assert_eq!(
            snapshot.len(),
            2,
            "同一台机器的 gossip 行不该再算一台：{:?}",
            snapshot.iter().map(|p| &p.name).collect::<Vec<_>>()
        );
        assert!(snapshot.iter().all(|p| p.name != "gossip"));
    }

    #[test]
    fn mesh_nodes_exchange_audio_without_manual_discovery() {
        // Node B: silent receiver. Node A pins B statically (discovery itself
        // needs live mDNS, exercised on real devices); B must hear A's tone.
        // Both engines run in one process, so they must not share the machine
        // identity file — pin distinct ids.
        let node_b = MeshEngine::start(MeshConfig {
            port: 0,
            output: false,
            advertise: false,
            node_id: Some("1111111111111111".into()),
            ..Default::default()
        })
        .expect("start node b");
        let b_addr = node_b.center.bind_addr();

        let node_a = MeshEngine::start(MeshConfig {
            port: 0,
            output: false,
            advertise: false,
            static_peers: vec![b_addr],
            node_id: Some("2222222222222222".into()),
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


    fn test_mesh_snapshot() -> MeshSnapshot {
        // A stopped engine still yields the full snapshot shape.
        let mut config = MeshConfig::default();
        config.bind = "127.0.0.1".into();
        config.port = 0;
        config.node_id = Some("aaaaaaaaaaaa0001".into());
        config.discovery = false;
        config.advertise = false;
        config.output = false;
        let engine = MeshEngine::start(config).expect("engine for snapshot");
        let mut snapshot = engine.snapshot();
        // Fabricate one stream entry so the per-stream contract is exercised.
        snapshot.center.streams.push(crate::metrics::StreamSnapshot {
            id: 1,
            origin: 0xAAAA_0000_0000_0002,
            stream_id: 7,
            label: "test".into(),
            addr: "127.0.0.1:1".into(),
            path: "wifi".into(),
            mix_count: 0,
            spectrum: vec![0.0; 32],
            codec: "s16".into(),
            active: true,
            gain_db: 0.0,
            muted: false,
            rms_db: -20.0,
            peak_db: -12.0,
            peak_hold_db: -12.0,
            buffer_ms: 30.0,
            target_ms: 30.0,
            buffer_max_ms: 70.0,
            ratio: 1.0,
            underruns: 0,
            underrun_frames: 0,
            buffer_resets: 0,
            packets: 10,
            lost_frames: 0,
            plc_frames: 0,
            overflow_packets: 0,
            clicks: 0,
            age_ms: 1,
        });
        snapshot
    }

    #[test]
    fn snapshot_json_contract_is_stable() {
        // F20: the WebUI/FFI JSON contract, pinned by assertions. Numeric
        // u64 identity fields must serialize as 16-char hex (JS precision);
        // UI-visible fields added over time must keep their names. Breaking
        // any of these breaks the panels silently.
        let mut node = test_mesh_snapshot();
        node.center.streams.retain(|_| true);
        let json = serde_json::to_string(&node).expect("snapshot serializes");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        let stream = &value["center"]["streams"][0];
        assert_eq!(
            stream["origin"].as_str().map(|s| s.len()),
            Some(16),
            "origin must be a 16-char hex string"
        );
        for field in [
            "packets",
            "lost_frames",
            "plc_frames",
            "overflow_packets",
            "clicks",
            "underruns",
            "buffer_resets",
            "buffer_ms",
            "buffer_max_ms",
            "age_ms",
            "codec",
            "path",
            "mix_count",
        ] {
            assert!(stream.get(field).is_some(), "stream.{field} missing from JSON contract");
        }
        for field in ["silent_ms", "first_frame_epoch_us", "kbps", "tx_rms_db"] {
            assert!(
                value["sender"].get(field).is_some(),
                "sender.{field} missing from JSON contract"
            );
        }
        for field in ["links", "state", "role", "tx_bytes", "rx_bytes", "gap_p99_ms"] {
            assert!(value["bt"].get(field).is_some(), "bt.{field} missing from JSON contract");
        }
        assert!(value["peers"].is_array(), "peers must be an array (rtt_ms/loss_pct live per peer)");
        for field in ["running", "peers", "topology", "bt", "sender", "center", "error"] {
            assert!(value.get(field).is_some(), "snapshot.{field} missing");
        }
    }
}
