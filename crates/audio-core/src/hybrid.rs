//! Automatic dual-stack mesh node: Wi-Fi and Bluetooth at the same time.
//!
//! There is no user-visible transport choice.  The node always joins the
//! Wi-Fi mesh (mDNS + UDP) and, when a Bluetooth adapter is available, also
//! builds a Bluetooth overlay:
//!
//! * every node advertises its Bluetooth address and origin bit over mDNS;
//! * the BT-capable node with the smallest id becomes the hub and listens on a
//!   fixed RFCOMM channel, everyone else dials it;
//! * the hub relays frames between its links, so a two-hop overlay works
//!   without Wi-Fi;
//! * the mesh sender writes every frame to both carriers at once (full rate
//!   over Wi-Fi, low-rate Opus over Bluetooth).  The receiver de-duplicates by
//!   `(origin, stream_id, seq)`, so whichever copy arrives first wins and a
//!   carrier hand-over never doubles the audio.
//!
//! Election runs with hysteresis: a leaf keeps its hub while it is reachable,
//! and a hub only hands over after a smaller node has been visible twice, so
//! a half-converged topology cannot bounce links around.  When there is no
//! Wi-Fi at all (no mDNS), a leaf dials the hub it remembered from the last
//! successful session, which is what lets a phone join an offline mesh.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::bt::{Tunnel, TunnelConfig};
use crate::mesh::{BtStatus, MeshConfig, MeshEngine, MeshSnapshot, TopologyPeer};
use crate::rfcomm::{BtListener, BtTransport, NativeBt};
use crate::{CoreError, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HybridConfig {
    /// Try to build the Bluetooth overlay in addition to Wi-Fi.
    pub bt_enabled: bool,
    /// RFCOMM channel both sides agree on (no SDP lookup).
    pub bt_channel: u8,
}

impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            bt_enabled: true,
            bt_channel: 16,
        }
    }
}

/// Host-provided Bluetooth backend (Android hands this to the Kotlin layer).
pub struct HybridOptions {
    pub config: HybridConfig,
    pub transport: Option<Arc<dyn BtTransport>>,
    /// Where to remember the last successful hub (offline join). Android has
    /// no usable default location from Rust and passes `filesDir` here.
    pub state_path: Option<PathBuf>,
}

impl Default for HybridOptions {
    fn default() -> Self {
        Self {
            config: HybridConfig::default(),
            transport: None,
            state_path: None,
        }
    }
}

struct HybridShared {
    listener: Mutex<Option<Arc<dyn BtListener>>>,
    connecting: AtomicBool,
    /// Pacing for dial attempts: a failing RFCOMM dial must not storm the
    /// hub (and the local Bluetooth stack) once per second.
    last_dial: Mutex<Option<Instant>>,
    /// Consecutive dial failures since the last success; a hub that keeps
    /// refusing connections is dropped and the election re-runs.
    dial_failures: AtomicU32,
    /// One classic-Bluetooth inquiry at a time.
    scanning: AtomicBool,
    /// Hub found by an inquiry probe (also persisted to hub.json).
    discovered: Mutex<Option<RememberedHub>>,
    /// Bluetooth address of the hub we currently hold a link to. Adopting an
    /// mDNS hub whose address matches must NOT tear the live link down.
    linked: Mutex<Option<String>>,
    /// E3: consecutive failed offline dial rounds (drives the backoff).
    offline_failures: AtomicU32,
    /// True only while this node is the hub and listening. A leaf must not
    /// accept extra links: relaying from a leaf would put two audio streams
    /// on one narrow RFCOMM link and starve both.
    is_hub: AtomicBool,
}

pub struct HybridNode {
    mesh: Arc<MeshEngine>,
    tunnel: Arc<Tunnel>,
    shared: Arc<HybridShared>,
    stop: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl HybridNode {
    pub fn start(mut mesh_config: MeshConfig, options: HybridOptions) -> Result<Self> {
        let transport: Arc<dyn BtTransport> = options
            .transport
            .clone()
            .unwrap_or_else(|| Arc::new(NativeBt));
        let enabled = options.config.bt_enabled && transport.available();
        let local_address = if enabled {
            transport.local_address()
        } else {
            None
        };
        if enabled {
            mesh_config.bt = local_address.clone();
            mesh_config.bt_channel = Some(options.config.bt_channel);
        } else {
            mesh_config.bt = None;
        }
        let state_path = options.state_path.clone().or_else(default_state_path);

        let mesh = Arc::new(MeshEngine::start(mesh_config)?);
        let tunnel = Arc::new(Tunnel::start(TunnelConfig {
            mesh_bind: loopback_bind(&mesh.bind_addr())?,
            node_bit: mesh.bit(),
            node_uid: mesh.uid(),
            ttl: 5,
            links: Vec::new(),
        })?);
        if enabled {
            mesh.set_tunnel(Some(tunnel.local_addr()));
        }
        mesh.set_bt_status(BtStatus {
            enabled,
            role: if enabled {
                "waiting".into()
            } else {
                "off".into()
            },
            state: if enabled {
                "waiting".into()
            } else {
                "off".into()
            },
            local_addr: local_address.clone(),
            channel: options.config.bt_channel,
            ..BtStatus::default()
        });

        let shared = Arc::new(HybridShared {
            listener: Mutex::new(None),
            connecting: AtomicBool::new(false),
            last_dial: Mutex::new(None),
            dial_failures: AtomicU32::new(0),
            scanning: AtomicBool::new(false),
            discovered: Mutex::new(None),
            linked: Mutex::new(None),
            offline_failures: AtomicU32::new(0),
            is_hub: AtomicBool::new(false),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let join = if enabled {
            let mesh = mesh.clone();
            let tunnel = tunnel.clone();
            let shared = shared.clone();
            let stop = stop.clone();
            let transport = transport.clone();
            let channel = options.config.bt_channel;
            let local = local_address.clone();
            Some(
                thread::Builder::new()
                    .name("siren-hybrid".into())
                    .spawn(move || {
                        supervise(
                            mesh, tunnel, transport, channel, local, state_path, shared, stop,
                        )
                    })
                    .map_err(|e| CoreError::Backend(format!("spawn hybrid: {e}")))?,
            )
        } else {
            None
        };

        Ok(Self {
            mesh,
            tunnel,
            shared,
            stop,
            join: Mutex::new(join),
        })
    }

    pub fn mesh(&self) -> &MeshEngine {
        &self.mesh
    }

    pub fn mesh_arc(&self) -> Arc<MeshEngine> {
        self.mesh.clone()
    }

    pub fn snapshot(&self) -> MeshSnapshot {
        self.mesh.snapshot()
    }

    pub fn stop(&self) {
        if !self.stop.swap(true, Ordering::Relaxed) {
            crate::engine::stop_trace("hybrid");
        }
        if let Some(listener) = self.shared.listener.lock().unwrap().take() {
            listener.close();
        }
        self.tunnel.stop();
        self.mesh.stop();
    }
}

impl Drop for HybridNode {
    fn drop(&mut self) {
        log::info!("hybrid: HybridNode dropped");
        self.stop();
        if let Some(join) = self.join.lock().unwrap().take() {
            // The supervisor can be inside a blocking RFCOMM connect, so it is
            // reaped in the background instead of holding up whoever dropped
            // the node (the FFI release runs on the UI's poll thread).
            let _ = thread::Builder::new()
                .name("siren-hybrid-reaper".into())
                .spawn(move || {
                    let _ = join.join();
                });
        }
    }
}

#[derive(Clone)]
struct Candidate {
    id: String,
    uid: u64,
    name: String,
    bt: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct RememberedHub {
    id: String,
    name: String,
    bt: String,
    channel: u8,
    at: u64,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct HubMemory {
    hubs: Vec<RememberedHub>,
    /// Whether this node was acting as the Bluetooth hub when it last ran.
    /// Two desktops without mDNS would both self-elect as hub and never
    /// connect; this makes them split into listener/dialer deterministically.
    was_hub: bool,
}

fn load_state(path: &Option<PathBuf>) -> HubMemory {
    let Some(path) = path else {
        return HubMemory::default();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return HubMemory::default();
    };
    serde_json::from_str::<HubMemory>(&text).unwrap_or_default()
}

fn save_state(path: &Option<PathBuf>, memory: &HubMemory) {
    let Some(path) = path else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(memory) {
        let _ = std::fs::write(path, json);
    }
}

fn load_memory(path: &Option<PathBuf>) -> Vec<RememberedHub> {
    load_state(path).hubs
}

fn remember_hub(path: &Option<PathBuf>, hub: &RememberedHub) {
    let mut memory = load_state(path);
    memory.hubs.retain(|entry| entry.bt != hub.bt);
    memory.hubs.insert(0, hub.clone());
    memory.hubs.truncate(3);
    save_state(path, &memory);
}

fn set_was_hub(path: &Option<PathBuf>, value: bool) {
    let mut memory = load_state(path);
    if memory.was_hub != value {
        memory.was_hub = value;
        save_state(path, &memory);
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Default place to remember hubs; Android always passes an explicit path.
fn default_state_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|base| PathBuf::from(base).join("SirenStudio").join("hub.json"))
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
            return Some(PathBuf::from(runtime).join("siren-hub.json"));
        }
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".cache/siren/hub.json"))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// How many consecutive one-second observations a decision needs. This keeps
/// a half-converged mDNS view from bouncing the overlay (and the audio).
const HUB_HANDOVER_TICKS: u32 = 2;
const HUB_LOST_TICKS: u32 = 3;
const REMEMBERED_DIAL_SECS: u64 = 5;
const DIAL_PACING: Duration = Duration::from_secs(3);
/// How often a hub refreshes its discoverable window and a hub-less leaf
/// runs a Bluetooth inquiry.
const DISCOVERABLE_REFRESH_SECS: u64 = 120;
const SCAN_INTERVAL_SECS: u64 = 20;

fn supervise(
    mesh: Arc<MeshEngine>,
    tunnel: Arc<Tunnel>,
    transport: Arc<dyn BtTransport>,
    channel: u8,
    local: Option<String>,
    state_path: Option<PathBuf>,
    shared: Arc<HybridShared>,
    stop: Arc<AtomicBool>,
) {
    let self_id = mesh.node_id().to_string();
    let self_name = mesh.name().to_string();
    // Exactly one side of a pair may dial: the smaller uid listens, the larger
    // dials. When both ends dial right after a teardown the pair gets two
    // RFCOMM links, and the duplicate handling then keeps replacing the healthy
    // one — that is the ~35 s link churn measured between PC and NAS.
    let self_uid = mesh.uid();
    let state_path = state_path.clone();
    let mut acting_hub = false;
    let mut current_hub: Option<Candidate> = None;
    let mut smaller_streak: u32 = 0;
    let mut missing_streak: u32 = 0;
    let mut last_error: Option<String> = None;
    let initial_state = load_state(&state_path);
    let mut remembered = initial_state.hubs;
    let mut was_hub = initial_state.was_hub;
    let mut remembered_index: usize = 0;
    let mut last_remembered_attempt: Option<Instant> = None;
    let mut offline_leaf = false;
    let mut remembered_hub: Option<RememberedHub> = None;
    let mut last_discoverable = Instant::now() - Duration::from_secs(DISCOVERABLE_REFRESH_SECS);
    let mut last_rx_bytes: u64 = 0;
    let mut last_rx_at = Instant::now();
    let mut rx_window_start = Instant::now();
    let mut rx_window_bytes: u64 = 0;
    let mut tx_window_bytes: u64 = 0;
    let mut last_hub_redial: Option<Instant> = None;
    let mut last_scan: Option<Instant> = None;
    let mut last_quality_log = Instant::now();
    let mut last_quality_tx = 0u64;
    let mut last_quality_rx = 0u64;
    // TTL blacklist: a device that refused RFCOMM channel 16 is skipped for a
    // while so repeated inquiries walk further down the list instead of
    // retrying the same accessories forever.
    let bad_candidates: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));

    // Remembered hubs this node is allowed to dial: only one with a smaller uid,
    // i.e. one that is the hub by election. Dialing a larger-uid peer would make
    // both ends connect to each other at the same time.
    let dial_targets = |remembered: &[RememberedHub]| -> Vec<RememberedHub> {
        remembered
            .iter()
            .filter(|entry| {
                let uid = uid_of(&entry.id);
                uid != 0 && uid < self_uid
            })
            .cloned()
            .collect()
    };
    // Same rule while mDNS is alive, but judged on the peer's *current* uid:
    // node ids are random per process, so hub.json can remember an identity that
    // no longer belongs to that address — dialing on the stale uid made the hub
    // call the very leaf that was calling it.
    let live_dial_targets =
        |remembered: &[RememberedHub], topology: &[TopologyPeer]| -> Vec<RememberedHub> {
            remembered
                .iter()
                .filter(|entry| {
                    topology
                        .iter()
                        .find(|peer| peer.bt.as_deref() == Some(entry.bt.as_str()))
                        .map(|peer| peer.uid != 0 && peer.uid < self_uid)
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        };

    while !stop.load(Ordering::Relaxed) {
        let topology = mesh.topology();
        let (elected, candidates) = elect(&self_id, &self_name, local.as_deref(), &topology);
        let elected_self = elected.as_ref().map(|c| c.id == self_id).unwrap_or(false);
        // A remembered hub is only worth dialing if it outranks us; otherwise
        // we are the hub and it must come to us. While mDNS is alive the ranking
        // uses the peer's current uid, because node ids are random per process
        // and hub.json can remember an identity that no longer matches.
        let dialable = !dial_targets(&remembered).is_empty();
        let live_dialable = !live_dial_targets(&remembered, &topology).is_empty();

        // ---- decide role / hub with hysteresis -------------------------
        // A live leaf link pins the current role: mDNS churn (a desktop
        // momentarily missing from the topology) must never drag a working
        // link to a different, non-listening node.
        let pinned =
            !acting_hub && tunnel.link_count() > 0 && shared.linked.lock().unwrap().is_some();
        if pinned {
            // keep current_hub/offline state exactly as it is
        } else if acting_hub {
            if elected_self {
                smaller_streak = 0;
            } else if elected.is_some() {
                smaller_streak += 1;
                if smaller_streak >= HUB_HANDOVER_TICKS {
                    log::info!(
                        "hybrid: handing hub over to {} ({})",
                        elected.as_ref().unwrap().name,
                        elected.as_ref().unwrap().bt
                    );
                    if let Some(listener) = shared.listener.lock().unwrap().take() {
                        listener.close();
                    }
                    tunnel.clear_links();
                    acting_hub = false;
                    current_hub = elected.clone();
                    smaller_streak = 0;
                }
            }
        } else {
            let connected = tunnel.link_count() > 0;
            if let Some(current) = current_hub.clone() {
                let present = candidates.iter().any(|c| c.id == current.id);
                if connected && !present {
                    missing_streak += 1;
                    if missing_streak >= HUB_LOST_TICKS {
                        log::info!("hybrid: hub {} no longer visible", current.name);
                        missing_streak = 0;
                        current_hub = None;
                    }
                } else {
                    missing_streak = 0;
                }
            }
            if current_hub.is_none() {
                match elected {
                    Some(hub) if hub.id != self_id => {
                        let already_linked = tunnel.link_count() > 0
                            && shared.linked.lock().unwrap().as_deref() == Some(hub.bt.as_str());
                        if !already_linked {
                            log::info!("hybrid: leaf link to hub {} ({})", hub.name, hub.bt);
                            tunnel.clear_links();
                        }
                        current_hub = Some(hub);
                    }
                    Some(_) => {
                        // We are the only BT-capable node in view. Without
                        // mDNS nobody can discover anyone: a node that was a
                        // leaf last time dials its remembered hub first, and
                        // only listens once that keeps failing. A node that
                        // was a hub keeps listening.
                        let solo = candidates.len() <= 1;
                        let dial_failed = shared.dial_failures.load(Ordering::Relaxed) >= 3;
                        // Stay an offline leaf once connected: checking
                        // `link_count == 0` here made a connected leaf flip to
                        // hub on the next tick and tear its own link down.
                        if solo && !was_hub && dialable && !dial_failed {
                            offline_leaf = true;
                        } else {
                            if dial_failed {
                                shared.dial_failures.store(0, Ordering::Relaxed);
                            }
                            tunnel.clear_links();
                            acting_hub = true;
                            offline_leaf = false;
                        }
                    }
                    None => {
                        // No mDNS topology (Wi-Fi gone): fall back to the hub
                        // remembered from the last successful session — but only
                        // when that hub outranks us. If it does not, we are the
                        // hub and must listen; dialing anyway puts two RFCOMM
                        // links on the same pair.
                        if dialable {
                            if !offline_leaf {
                                tunnel.clear_links();
                                offline_leaf = true;
                            }
                        } else {
                            tunnel.clear_links();
                            acting_hub = true;
                            offline_leaf = false;
                        }
                    }
                }
            }
        }

        // ---- act --------------------------------------------------------
        let mut connected: bool = false;
        // A hub that was promoted only because its known hub was unreachable
        // must keep trying to merge back; otherwise two hubs split the net.
        if acting_hub && tunnel.link_count() > 0 {
            let linked = shared.linked.lock().unwrap().clone();
            if let (Some(address), Some(entry)) = (linked, remembered.first()) {
                if address == entry.bt {
                    log::info!(
                        "hybrid: rejoined known hub {}, stepping down from hub",
                        entry.name
                    );
                    if let Some(listener) = shared.listener.lock().unwrap().take() {
                        listener.close();
                    }
                    acting_hub = false;
                    current_hub = Some(Candidate {
                        uid: uid_of(&entry.id),
                        id: entry.id.clone(),
                        name: entry.name.clone(),
                        bt: entry.bt.clone(),
                    });
                }
            }
        }
        shared.is_hub.store(acting_hub, Ordering::Relaxed);
        // A leaf keeps exactly one uplink. Doing this outside the current_hub
        // branch also covers the offline/mDNS-less state, where reconnects
        // used to accumulate duplicate links on the hub.
        if !acting_hub && tunnel.link_count() > 1 {
            tunnel.keep_latest_link();
        }
        if tunnel.link_count() == 0 {
            *shared.linked.lock().unwrap() = None;
        }
        if acting_hub {
            if !was_hub {
                was_hub = true;
                set_was_hub(&state_path, true);
            }
            if live_dialable
                && tunnel.link_count() == 0
                && !shared.connecting.load(Ordering::Relaxed)
                && last_hub_redial
                    .map(|at| at.elapsed() >= Duration::from_secs(45))
                    .unwrap_or(true)
            {
                last_hub_redial = Some(Instant::now());
                let entry = live_dial_targets(&remembered, &topology)
                    .into_iter()
                    .next()
                    .expect("live_dialable was checked above");
                log::info!(
                    "hybrid: hub retries known hub {} ({})",
                    entry.name,
                    entry.bt
                );
                shared.connecting.store(true, Ordering::Relaxed);
                let transport = transport.clone();
                let tunnel = tunnel.clone();
                let connected = shared.clone();
                let dial_stop = stop.clone();
                thread::Builder::new()
                    .name("siren-bt-redial".into())
                    .spawn(move || {
                        match dial_with_backoff(
                            &transport,
                            &tunnel,
                            &entry.bt,
                            entry.channel,
                            &dial_stop,
                        ) {
                            Ok(_) => {
                                connected.dial_failures.store(0, Ordering::Relaxed);
                                *connected.linked.lock().unwrap() = Some(entry.bt.clone());
                            }
                            Err(error) => {
                                log::debug!("hybrid: hub redial {}: {error}", entry.bt);
                            }
                        }
                        connected.connecting.store(false, Ordering::Relaxed);
                    })
                    .ok();
            }
            if last_discoverable.elapsed() >= Duration::from_secs(DISCOVERABLE_REFRESH_SECS) {
                if transport.set_discoverable(180) {
                    log::info!("hybrid: hub discoverable for classic inquiry");
                }
                last_discoverable = Instant::now();
            }
            if shared.listener.lock().unwrap().is_none() {
                match transport.listen(channel) {
                    Ok(listener) => {
                        let listener: Arc<dyn BtListener> = Arc::from(listener);
                        *shared.listener.lock().unwrap() = Some(listener.clone());
                        spawn_listener(listener, tunnel.clone(), shared.clone(), stop.clone());
                        log::info!("hybrid: hub on RFCOMM channel {channel}");
                        last_error = None;
                    }
                    Err(error) => {
                        log::warn!("hybrid: cannot listen on channel {channel}: {error}");
                        last_error = Some(error.to_string());
                    }
                }
            }
        } else if let Some(hub) = current_hub.clone() {
            if tunnel.link_count() > 0 && was_hub {
                was_hub = false;
                set_was_hub(&state_path, false);
            }
            if tunnel.link_count() > 1 {
                // Extra links belong to an earlier hub role; keep the newest
                // one so a leaf never relays. Clearing every link here made
                // the leaf drop its working connection and redial in a loop.
                tunnel.keep_latest_link();
            }
            let dial_ready = {
                let mut last = shared.last_dial.lock().unwrap();
                let ready = last.map(|at| at.elapsed() >= DIAL_PACING).unwrap_or(true);
                if ready {
                    *last = Some(Instant::now());
                }
                ready
            };
            if shared.dial_failures.load(Ordering::Relaxed) >= 3 {
                log::info!("hybrid: hub {} refuses connections, re-electing", hub.name);
                shared.dial_failures.store(0, Ordering::Relaxed);
                current_hub = None;
            }
            if tunnel.link_count() == 0 && dial_ready && !shared.connecting.load(Ordering::Relaxed)
            {
                shared.connecting.store(true, Ordering::Relaxed);
                let transport = transport.clone();
                let tunnel = tunnel.clone();
                let connected = shared.clone();
                let address = hub.bt.clone();
                let memory = RememberedHub {
                    id: hub.id.clone(),
                    name: hub.name.clone(),
                    bt: hub.bt.clone(),
                    channel,
                    at: unix_secs(),
                };
                let memory_path = state_path.clone();
                let dial_stop = stop.clone();
                thread::Builder::new()
                    .name("siren-bt-dial".into())
                    .spawn(move || {
                        let result =
                            dial_with_backoff(&transport, &tunnel, &address, channel, &dial_stop);
                        match &result {
                            Ok(_) => {
                                connected.dial_failures.store(0, Ordering::Relaxed);
                                *connected.linked.lock().unwrap() = Some(address.clone());
                                remember_hub(&memory_path, &memory);
                            }
                            Err(error) => {
                                connected.dial_failures.fetch_add(1, Ordering::Relaxed);
                                log::debug!("hybrid: dial {address}/{channel}: {error}");
                            }
                        }
                        connected.connecting.store(false, Ordering::Relaxed);
                    })
                    .ok();
            }
            connected = tunnel.link_count() > 0;
        } else if offline_leaf {
            // Fully offline leaf (Android without Wi-Fi): dial the remembered
            // hub, rotating through the ones we have seen before. A live link
            // is left alone; only role/topology changes clear it.
            if tunnel.link_count() > 0 && was_hub {
                was_hub = false;
                set_was_hub(&state_path, false);
            }
            if tunnel.link_count() > 0 {
                if let Some(found) = shared.discovered.lock().unwrap().clone() {
                    remembered.retain(|entry| entry.bt != found.bt);
                    remembered.insert(0, found.clone());
                    remembered_hub = Some(found);
                } else if !remembered.is_empty() {
                    let index = (remembered_index + remembered.len() - 1) % remembered.len();
                    remembered_hub = Some(remembered[index].clone());
                }
            }
            let dial_ready = {
                let mut last = shared.last_dial.lock().unwrap();
                let ready = last.map(|at| at.elapsed() >= DIAL_PACING).unwrap_or(true);
                if ready {
                    *last = Some(Instant::now());
                }
                ready
            };
            let scan_due = tunnel.link_count() == 0
                && last_scan
                    .map(|at| at.elapsed() >= Duration::from_secs(SCAN_INTERVAL_SECS))
                    .unwrap_or(true);
            // Scan only when there is nothing to dial: probing random
            // accessories takes seconds each and must never preempt retrying
            // the remembered hub (that turned a 3 s reconnect into 30 s).
            if scan_due && !shared.scanning.load(Ordering::Relaxed) && remembered.is_empty() {
                last_scan = Some(Instant::now());
                shared.scanning.store(true, Ordering::Relaxed);
                let transport = transport.clone();
                let tunnel = tunnel.clone();
                let shared_scan = shared.clone();
                let state_path = state_path.clone();
                let bad = bad_candidates.clone();
                let own = local.clone().unwrap_or_default();
                thread::Builder::new()
                    .name("siren-bt-scan".into())
                    .spawn(move || {
                        let devices = transport.scan(6);
                        log::info!("hybrid: inquiry found {} device(s)", devices.len());
                        // Only a few devices in range answer RFCOMM channel 16;
                        // probing is the identification, so keep it bounded.
                        let mut probes = 0u32;
                        for device in devices {
                            if probes >= 6 {
                                break;
                            }
                            if device.address.eq_ignore_ascii_case(&own) || tunnel.link_count() > 0
                            {
                                continue;
                            }
                            {
                                let mut bad = bad.lock().unwrap();
                                if bad
                                    .get(&device.address)
                                    .map(|at: &Instant| at.elapsed().as_secs() < 600)
                                    .unwrap_or(false)
                                {
                                    continue;
                                }
                                bad.remove(&device.address);
                            }
                            probes += 1;
                            log::info!("hybrid: probing {} ({})", device.address, device.name);
                            match transport.connect(&device.address, channel) {
                                Ok(link) => {
                                    let Ok(id) = tunnel.add_link(Box::new(link)) else {
                                        continue;
                                    };
                                    let before = tunnel.stats().rx_bytes;
                                    std::thread::sleep(Duration::from_millis(2500));
                                    let alive = tunnel.stats().rx_bytes > before;
                                    if alive {
                                        *shared_scan.linked.lock().unwrap() =
                                            Some(device.address.clone());
                                        log::info!(
                                            "hybrid: discovered hub over Bluetooth {} ({})",
                                            device.name,
                                            device.address
                                        );
                                        let memory = RememberedHub {
                                            id: String::new(),
                                            name: if device.name.is_empty() {
                                                "Bluetooth hub".into()
                                            } else {
                                                device.name.clone()
                                            },
                                            bt: device.address.clone(),
                                            channel,
                                            at: unix_secs(),
                                        };
                                        remember_hub(&state_path, &memory);
                                        *shared_scan.discovered.lock().unwrap() = Some(memory);
                                        break;
                                    }
                                    tunnel.remove_link(id);
                                    bad.lock()
                                        .unwrap()
                                        .insert(device.address.clone(), Instant::now());
                                    log::debug!(
                                        "hybrid: {} answered RFCOMM but is not a Siren hub",
                                        device.address
                                    );
                                }
                                Err(error) => {
                                    bad.lock()
                                        .unwrap()
                                        .insert(device.address.clone(), Instant::now());
                                    log::info!(
                                        "hybrid: candidate {} rejected: {error}",
                                        device.address
                                    );
                                }
                            }
                        }
                        shared_scan.scanning.store(false, Ordering::Relaxed);
                    })
                    .ok();
            }
            let failures = shared.offline_failures.load(Ordering::Relaxed);
            if dialable
                && tunnel.link_count() == 0
                && dial_ready
                && !shared.connecting.load(Ordering::Relaxed)
                && last_remembered_attempt
                    .map(|at| at.elapsed() >= offline_backoff(failures))
                    .unwrap_or(true)
            {
                let targets = dial_targets(&remembered);
                let entry = targets[remembered_index % targets.len()].clone();
                remembered_index = (remembered_index + 1) % targets.len();
                last_remembered_attempt = Some(Instant::now());
                log::info!(
                    "hybrid: offline, dialing remembered hub {} ({})",
                    entry.name,
                    entry.bt
                );
                shared.connecting.store(true, Ordering::Relaxed);
                let transport = transport.clone();
                let tunnel = tunnel.clone();
                let connected = shared.clone();
                let memory_path = state_path.clone();
                let dial_stop = stop.clone();
                thread::Builder::new()
                    .name("siren-bt-dial".into())
                    .spawn(move || {
                        match dial_once(&transport, &tunnel, &entry.bt, entry.channel, &dial_stop) {
                            Ok(_) => {
                                connected.dial_failures.store(0, Ordering::Relaxed);
                                connected.offline_failures.store(0, Ordering::Relaxed);
                                *connected.linked.lock().unwrap() = Some(entry.bt.clone());
                                remember_hub(&memory_path, &entry);
                            }
                            Err(error) => {
                                connected.dial_failures.fetch_add(1, Ordering::Relaxed);
                                let failures =
                                    connected.offline_failures.fetch_add(1, Ordering::Relaxed) + 1;
                                log::info!(
                                    "hybrid: remembered dial {} failed ({failures} in a row, next in {:?}): {error}",
                                    entry.bt,
                                    offline_backoff(failures)
                                );
                            }
                        }
                        connected.connecting.store(false, Ordering::Relaxed);
                    })
                    .ok();
            }
        }

        // ---- link watchdog ----------------------------------------------
        // A half-open RFCOMM link keeps `link_count` at 1 forever: writes
        // succeed into buffers, reads block, and the leaf never re-dials.
        // If no byte arrives for six seconds while a link exists, reset it.
        let stats = tunnel.stats();
        // The RF diagnosis needs the gap *shape*, not just byte counts: sniff
        // parking, coexistence retransmission and piconet sharing all look like
        // "fewer bytes" in a counter.
        if stats.links > 0 && last_quality_log.elapsed() >= Duration::from_secs(10) {
            // A leaf's Bluetooth downlink is idle by design while its Wi-Fi is
            // healthy, so the receive-side percentiles only mean something when
            // traffic is actually flowing. Report the send side and the write
            // drops next to them, and say how much was received.
            let seconds = last_quality_log.elapsed().as_secs_f64().max(1.0);
            let tx_kb_s = stats.tx_bytes.saturating_sub(last_quality_tx) as f64 / 1024.0 / seconds;
            let rx_kb_s = stats.rx_bytes.saturating_sub(last_quality_rx) as f64 / 1024.0 / seconds;
            last_quality_tx = stats.tx_bytes;
            last_quality_rx = stats.rx_bytes;
            last_quality_log = Instant::now();
            let gaps = if stats.gap_samples == 0 {
                "no downlink packets (idle by design while Wi-Fi works)".to_string()
            } else {
                format!(
                    "gap p50 {} p90 {} p99 {} max {} ms, stalls >500 ms: {} ({} samples)",
                    stats.gap_p50_ms,
                    stats.gap_p90_ms,
                    stats.gap_p99_ms,
                    stats.gap_max_ms,
                    stats.gap_stalls_500ms,
                    stats.gap_samples,
                )
            };
            log::info!(
                "bt: link quality: {} link(s), tx {:.1} KB/s, rx {:.1} KB/s, write drops {}, {}",
                stats.links,
                tx_kb_s,
                rx_kb_s,
                stats.dropped,
                gaps,
            );
        }
        if stats.links > 0 {
            last_rx_bytes = stats.rx_bytes;
            last_rx_at = Instant::now();
            // A dying RFCOMM link can trickle its stale buffer for tens of
            // seconds, so "some bytes arrived" is not proof of life. Audio
            // should flow at tens of KB/s; anything far below is dead.
            if rx_window_start.elapsed() >= Duration::from_secs(6) {
                let window = stats.rx_bytes.saturating_sub(rx_window_bytes);
                let tx_window = stats.tx_bytes.saturating_sub(tx_window_bytes);
                rx_window_start = Instant::now();
                rx_window_bytes = stats.rx_bytes;
                tx_window_bytes = stats.tx_bytes;
                // A hub deliberately sends nothing over Bluetooth while the
                // leaf's Wi-Fi is healthy (N-1 downlink is zero). Only treat
                // the link as dead when neither direction moved.
                if window < 4096 && tx_window < 4096 {
                    log::warn!(
                        "hybrid: Bluetooth link stalled ({} B in / {} B out in 6s), resetting links",
                        window,
                        tx_window
                    );
                    tunnel.clear_links();
                    *shared.linked.lock().unwrap() = None;
                    shared.connecting.store(false, Ordering::Relaxed);
                }
            }
        } else {
            last_rx_bytes = stats.rx_bytes;
            last_rx_at = Instant::now();
            rx_window_start = Instant::now();
            rx_window_bytes = stats.rx_bytes;
        }

        // ---- publish status ---------------------------------------------
        let role = if acting_hub {
            "hub"
        } else if current_hub.is_some() || (offline_leaf && tunnel.link_count() > 0) {
            "leaf"
        } else {
            "waiting"
        };
        let hub_for_status = if acting_hub {
            None
        } else {
            current_hub.clone().or_else(|| {
                remembered_hub.clone().map(|hub| Candidate {
                    uid: uid_of(&hub.id),
                    id: hub.id,
                    name: hub.name,
                    bt: hub.bt,
                })
            })
        };
        let state = if acting_hub {
            "listening"
        } else if tunnel.link_count() > 0 {
            "connected"
        } else if shared.connecting.load(Ordering::Relaxed) {
            "connecting"
        } else {
            "waiting"
        };
        let status = BtStatus {
            enabled: true,
            role: role.into(),
            state: state.into(),
            hub_id: if acting_hub {
                Some(self_id.clone())
            } else {
                hub_for_status.as_ref().map(|c| c.id.clone())
            },
            hub_name: if acting_hub {
                Some(self_name.clone())
            } else {
                hub_for_status.as_ref().map(|c| c.name.clone())
            },
            hub_addr: if acting_hub {
                local.clone()
            } else {
                hub_for_status.as_ref().map(|c| c.bt.clone())
            },
            local_addr: local.clone(),
            channel,
            links: stats.links,
            tx_bytes: stats.tx_bytes,
            rx_bytes: stats.rx_bytes,
            dropped: stats.dropped,
            gap_samples: stats.gap_samples,
            gap_p99_ms: stats.gap_p99_ms,
            gap_max_ms: stats.gap_max_ms,
            gap_stalls_500ms: stats.gap_stalls_500ms,
            rx_kb_per_s: stats.gap_kb_per_s,
            last_error: last_error.clone(),
        };
        mesh.set_bt_status(status);
        // Flip the tunnel into N-1 mode together with the role it publishes,
        // so the mix sender and the relay switch in the same tick.
        tunnel.set_n1(acting_hub);
        thread::sleep(Duration::from_millis(1000));
    }
    if let Some(listener) = shared.listener.lock().unwrap().take() {
        listener.close();
    }
}

/// Dial a hub with a short escalating backoff. An RFCOMM link that just dropped
/// normally comes back within a few hundred milliseconds, and waiting for the
/// supervisor's next paced attempt would leave the mesh silent for seconds after
/// every stall. The whole burst counts as one failure, so re-election timing is
/// unchanged: a hub that is really gone is still given up on after three bursts.
/// One connect attempt, no inner retries (E3/A2): the offline redial loop
/// owns the pacing. The old inner 0/200/600/1500 ms retries made every
/// round fire four RFCOMM dials — 470 dials in 12 min against a dead hub.
fn dial_once(
    transport: &Arc<dyn BtTransport>,
    tunnel: &Arc<Tunnel>,
    address: &str,
    channel: u8,
    stop: &AtomicBool,
) -> Result<()> {
    if stop.load(Ordering::Relaxed) || tunnel.link_count() > 0 {
        return Err(CoreError::Backend("dial cancelled".into()));
    }
    transport
        .connect(address, channel)
        .and_then(|link| tunnel.add_link(Box::new(link)))
        .map(|_| ())
}

/// Exponential redial pacing for the offline leaf (E3): 1/2/4/8/16/30 s,
/// and once 5 rounds have failed in a row fall back to a 5 min probe so a
/// hub that is simply switched off costs one dial per 5 minutes.
fn offline_backoff(failures: u32) -> Duration {
    if failures >= 5 {
        return Duration::from_secs(300);
    }
    Duration::from_secs(1u64 << failures.min(5))
}

fn dial_with_backoff(
    transport: &Arc<dyn BtTransport>,
    tunnel: &Arc<Tunnel>,
    address: &str,
    channel: u8,
    stop: &AtomicBool,
) -> Result<()> {
    const BACKOFF_MS: [u64; 4] = [0, 200, 600, 1_500];
    let mut result = Err(CoreError::Backend("dial cancelled".into()));
    for delay_ms in BACKOFF_MS {
        // A link may have arrived from the other side while we were waiting;
        // dialing anyway would hand the peer a duplicate to sort out.
        if stop.load(Ordering::Relaxed) || tunnel.link_count() > 0 {
            return result;
        }
        if delay_ms > 0 {
            thread::sleep(Duration::from_millis(delay_ms));
        }
        result = transport
            .connect(address, channel)
            .and_then(|link| tunnel.add_link(Box::new(link)))
            .map(|_| ());
        if result.is_ok() {
            return Ok(());
        }
        log::debug!(
            "hybrid: dial {address} failed in {:.0} ms backoff",
            delay_ms
        );
    }
    result
}

fn spawn_listener(
    listener: Arc<dyn BtListener>,
    tunnel: Arc<Tunnel>,
    shared: Arc<HybridShared>,
    stop: Arc<AtomicBool>,
) {
    thread::Builder::new()
        .name("siren-bt-accept".into())
        .spawn(move || {
            let mut accept_errors = 0u32;
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok(link) => {
                        accept_errors = 0;
                        if !shared.is_hub.load(Ordering::Relaxed) {
                            // Role changed while the connection was pending:
                            // this node is a leaf now and must not relay.
                            log::info!("hybrid: rejecting link, node is a leaf");
                            drop(link);
                            continue;
                        }
                        log::info!("hybrid: hub accepted a link");
                        if let Err(error) = tunnel.add_link(Box::new(link)) {
                            log::warn!("hybrid: cannot attach link: {error}");
                        }
                    }
                    Err(_) => {
                        accept_errors += 1;
                        if accept_errors >= 30 {
                            // The listening socket died under us (adapter
                            // reset, BlueZ restart): let the supervisor
                            // re-create the listener.
                            log::warn!("hybrid: listener failing repeatedly, re-creating");
                            listener.close();
                            break;
                        }
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        })
        .ok();
}

/// Hub = the Bluetooth-capable node with the smallest id. Returns the hub and
/// the full candidate list (for diagnostics).
fn elect(
    self_id: &str,
    self_name: &str,
    local: Option<&str>,
    topology: &[TopologyPeer],
) -> (Option<Candidate>, Vec<Candidate>) {
    let mut candidates: Vec<Candidate> = Vec::new();
    if let Some(address) = local {
        candidates.push(Candidate {
            id: self_id.to_string(),
            uid: uid_of(self_id),
            name: self_name.to_string(),
            bt: address.to_string(),
        });
    }
    for peer in topology {
        let Some(address) = peer.bt.as_ref() else {
            continue;
        };
        if peer.id == self_id {
            continue;
        }
        candidates.push(Candidate {
            id: peer.id.clone(),
            uid: uid_of(&peer.id),
            name: peer.name.clone(),
            bt: address.clone(),
        });
    }
    candidates.sort_by_key(|candidate| candidate.uid);
    (candidates.first().cloned(), candidates)
}

fn uid_of(id: &str) -> u64 {
    crate::mesh::node_uid(id)
}

/// The tunnel injects on loopback; an unspecified bind address must be
/// rewritten to 127.0.0.1 for Windows.
fn loopback_bind(bind: &str) -> Result<std::net::SocketAddr> {
    let mut addr: std::net::SocketAddr = bind
        .parse()
        .map_err(|error| CoreError::Backend(format!("mesh bind '{bind}': {error}")))?;
    if addr.ip().is_unspecified() {
        addr.set_ip(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    }
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str, bt: Option<&str>) -> TopologyPeer {
        TopologyPeer {
            name: format!("node-{id}"),
            id: id.into(),
            uid: crate::mesh::node_uid(id),
            endpoint: "127.0.0.1:47900".into(),
            bit: Some(1),
            bt: bt.map(str::to_string),
            bt_channel: Some(16),
        }
    }

    #[test]
    fn smallest_id_with_a_radio_becomes_hub() {
        let topology = vec![peer("00000000000000ff", Some("AA:AA:AA:AA:AA:02"))];
        let (hub, _) = elect(
            "00000000000000aa",
            "self",
            Some("AA:AA:AA:AA:AA:01"),
            &topology,
        );
        assert_eq!(hub.expect("hub").id, "00000000000000aa");
    }

    #[test]
    fn node_without_an_address_is_a_leaf() {
        let topology = vec![peer("00000000000000ff", Some("AA:AA:AA:AA:AA:02"))];
        let (hub, _) = elect("0000000000000001", "android", None, &topology);
        let hub = hub.expect("remote hub");
        assert_eq!(hub.id, "00000000000000ff");
        assert_eq!(hub.bt, "AA:AA:AA:AA:AA:02");
    }

    #[test]
    fn peers_without_bluetooth_are_ignored() {
        let topology = vec![peer("00000000000000ff", None)];
        let (hub, candidates) = elect("00000000000000aa", "self", None, &topology);
        assert!(hub.is_none());
        assert!(candidates.is_empty());
    }

    #[test]
    fn remembers_and_rotates_hubs() {
        let dir = std::env::temp_dir().join(format!("siren-test-{}", std::process::id()));
        let path = dir.join("hub.json");
        let _ = std::fs::remove_file(&path);
        let state = Some(path.clone());
        let hub = RememberedHub {
            id: "aa".into(),
            name: "hub".into(),
            bt: "AA:AA:AA:AA:AA:01".into(),
            channel: 16,
            at: 1,
        };
        remember_hub(&state, &hub);
        remember_hub(
            &state,
            &RememberedHub {
                id: "bb".into(),
                name: "newer".into(),
                bt: "AA:AA:AA:AA:AA:02".into(),
                channel: 16,
                at: 2,
            },
        );
        let loaded = load_memory(&state);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].bt, "AA:AA:AA:AA:AA:02");
        // Re-remembering the same address moves it to the front, no duplicates.
        remember_hub(&state, &hub);
        let loaded = load_memory(&state);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].bt, "AA:AA:AA:AA:AA:01");
        let _ = std::fs::remove_file(&path);
    }
}
