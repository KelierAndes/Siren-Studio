//! Bluetooth (or any byte-stream) transport for the mesh.
//!
//! The existing UDP/Opus pipeline stays untouched: a node sends its local
//! stream to a local tunnel socket, the tunnel frames each mesh datagram with
//! a small routing envelope and writes it to every byte-stream link
//! (RFCOMM / LE L2CAP / a local TCP bridge in tests). Received envelopes are
//! injected back into the local mesh and forwarded to the other links.
//!
//! Routing envelope, compact form (4 bytes, little endian) emitted by this
//! version, and the legacy 16-byte form still accepted on read:
//!
//! | 0 | magic `0xB2` (compact) / `0xB1` (legacy) |
//! | 1..3 | payload length |
//! | 3 | TTL (high nibble) + hop count (low nibble) |
//!
//! The legacy form also carried an origin bitmask; compact packets identify
//! their source through the audio frame's `origin` id instead, which is
//! unique per node and survives forwarding.
//!
//! Multi-hop rules: drop if the origin mask contains this node's bit (a
//! forwarded copy of our own audio came back), drop at TTL 0, deduplicate by
//! `(origin, stream_id, sequence)`, then inject locally and forward to every
//! link except the one it arrived on.
//!
//! Links can be added and removed while the tunnel runs: a hub starts with no
//! links, accepts RFCOMM peers whenever they show up, and each accepted socket
//! becomes a link. The tunnel keeps running with zero links so the mesh sender
//! can keep writing into it; packets are simply dropped until a link exists.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{CoreError, Result};

const MAGIC: u8 = 0xB1;
const VERSION: u8 = 1;
const HEADER_LEN: usize = 16;
const MAGIC_COMPACT: u8 = 0xB2;
/// A local envelope addressed to one destination leaf (N-1 mix).
const MAGIC_TARGETED: u8 = 0xB3;
const HEADER_LEN_COMPACT: usize = 4;
const MAX_PAYLOAD: usize = 65_535;
const DEDUPE_LIMIT: usize = 8192;
/// Per-link write queue: 32 x 10 ms frames = 320 ms of buffering before the
/// carrier starts dropping instead of accumulating latency.
const WRITE_QUEUE_LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub origin: u64,
    pub ttl: u8,
    pub hop: u8,
    pub payload: Vec<u8>,
}

impl Envelope {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.push(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&(self.payload.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.origin.to_le_bytes());
        out.push(self.ttl);
        out.push(self.hop);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&self.payload);
        out
    }

    /// 4-byte envelope used for every outgoing packet.
    pub fn encode_compact(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN_COMPACT + self.payload.len());
        out.push(MAGIC_COMPACT);
        out.extend_from_slice(&(self.payload.len() as u16).to_le_bytes());
        let ttl = self.ttl.min(15);
        let hop = self.hop.min(15);
        out.push((ttl << 4) | hop);
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<(Self, usize)> {
        if bytes.len() >= HEADER_LEN_COMPACT && bytes[0] == MAGIC_COMPACT {
            let len = u16::from_le_bytes([bytes[1], bytes[2]]) as usize;
            if bytes.len() < HEADER_LEN_COMPACT + len {
                return None;
            }
            let ttl = bytes[3] >> 4;
            let hop = bytes[3] & 0x0F;
            return Some((
                Self {
                    origin: 0,
                    ttl,
                    hop,
                    payload: bytes[HEADER_LEN_COMPACT..HEADER_LEN_COMPACT + len].to_vec(),
                },
                HEADER_LEN_COMPACT + len,
            ));
        }
        if bytes.len() < HEADER_LEN || bytes[0] != MAGIC || bytes[1] != VERSION {
            return None;
        }
        let len = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
        if bytes.len() < HEADER_LEN + len {
            return None;
        }
        let origin = u64::from_le_bytes(bytes[4..12].try_into().ok()?);
        Some((
            Self {
                origin,
                ttl: bytes[12],
                hop: bytes[13],
                payload: bytes[HEADER_LEN..HEADER_LEN + len].to_vec(),
            },
            HEADER_LEN + len,
        ))
    }
}

/// A full-duplex byte stream the tunnel can read and write independently.
///
/// `try_clone_link` must return another handle on the *same* connection so the
/// reader and writer can block in separate threads; `shutdown_link` unblocks
/// both.
pub trait Link: Send {
    fn read_link(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    fn write_link(&mut self, buf: &[u8]) -> std::io::Result<usize>;
    fn try_clone_link(&self) -> std::io::Result<Box<dyn Link>>;
    fn shutdown_link(&self);
    fn set_nodelay(&self) {}
    fn description(&self) -> String {
        "link".into()
    }
}

impl Link for TcpStream {
    fn read_link(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.read(buf)
    }

    fn write_link(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.write(buf)
    }

    fn try_clone_link(&self) -> std::io::Result<Box<dyn Link>> {
        Ok(Box::new(self.try_clone()?))
    }

    fn shutdown_link(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }

    fn set_nodelay(&self) {
        let _ = TcpStream::set_nodelay(self, true);
    }

    fn description(&self) -> String {
        self.peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "tcp".into())
    }
}

pub struct TunnelConfig {
    /// Local UDP port the mesh sends its own stream to (`static_peers`).
    pub mesh_bind: SocketAddr,
    /// Bit (0..=63) that identifies this node in the legacy origin mask.
    pub node_bit: u8,
    /// Stable node id embedded in every audio frame; used to drop our own
    /// audio that came back through a loop.
    pub node_uid: u64,
    pub ttl: u8,
    /// Connected byte-stream links (RFCOMM, L2CAP CoC or a local TCP bridge).
    pub links: Vec<Box<dyn Link>>,
}

#[derive(Default)]
struct TunnelStats {
    tx_bytes: AtomicU64,
    rx_bytes: AtomicU64,
    tx_packets: AtomicU64,
    rx_packets: AtomicU64,
    dropped: AtomicU64,
    last_rx_us: AtomicU64,
    links: AtomicUsize,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct TunnelStatsSnapshot {
    pub links: usize,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub dropped: u64,
    pub last_rx_ms: u64,
}

pub struct Tunnel {
    local_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    uid: Arc<AtomicU64>,
    stats: Arc<TunnelStats>,
    links: Arc<Mutex<HashMap<usize, Box<dyn Link>>>>,
    next_link: AtomicUsize,
    commands: Mutex<mpsc::Sender<Command>>,
    events: mpsc::Sender<Event>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    /// Hub mode: only N-1 mixes go down the links, nothing is relayed.
    n1: Arc<AtomicBool>,
}

impl Tunnel {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Set after the mesh node exists (the CLI starts the tunnel first).
    pub fn set_node_uid(&self, uid: u64) {
        self.uid.store(uid, Ordering::Relaxed);
    }

    /// Hub mode: forward only the N-1 mixes the mesh sender builds, stop
    /// relaying individual streams and stop broadcasting the local stream.
    pub fn set_n1(&self, on: bool) {
        if self.n1.swap(on, Ordering::Relaxed) != on {
            log::info!("bt: N-1 forwarding {}", if on { "on" } else { "off" });
        }
    }

    pub fn link_count(&self) -> usize {
        self.stats.links.load(Ordering::Relaxed)
    }

    pub fn stats(&self) -> TunnelStatsSnapshot {
        let last = self.stats.last_rx_us.load(Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        TunnelStatsSnapshot {
            links: self.stats.links.load(Ordering::Relaxed),
            tx_bytes: self.stats.tx_bytes.load(Ordering::Relaxed),
            rx_bytes: self.stats.rx_bytes.load(Ordering::Relaxed),
            tx_packets: self.stats.tx_packets.load(Ordering::Relaxed),
            rx_packets: self.stats.rx_packets.load(Ordering::Relaxed),
            dropped: self.stats.dropped.load(Ordering::Relaxed),
            last_rx_ms: if last == 0 { 0 } else { now.saturating_sub(last) },
        }
    }

    /// Keep only the newest link (a leaf never relays; extra links are
    /// reconnect leftovers and clearing all of them would force a redial).
    pub fn keep_latest_link(&self) {
        let mut links = self.links.lock().unwrap();
        if links.len() <= 1 {
            return;
        }
        let keep = *links.keys().max().unwrap();
        let extra: Vec<usize> = links
            .keys()
            .copied()
            .filter(|id| *id != keep)
            .collect();
        for id in &extra {
            if let Some(link) = links.remove(id) {
                link.shutdown_link();
            }
        }
        drop(links);
        let commands = self.commands.lock().unwrap();
        for id in extra {
            let _ = commands.send(Command::CloseLink { id });
        }
    }

    /// Close every link but keep listening on the local UDP port.
    pub fn clear_links(&self) {
        let mut links = self.links.lock().unwrap();
        for (_, link) in links.drain() {
            link.shutdown_link();
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.clear_links();
    }

    pub fn start(config: TunnelConfig) -> Result<Self> {
        if config.node_bit >= 64 {
            return Err(CoreError::InvalidArgument(
                "node bit must be between 0 and 63".into(),
            ));
        }
        let local = UdpSocket::bind("127.0.0.1:0")?;
        let local_addr = local.local_addr()?;
        let inject = UdpSocket::bind("127.0.0.1:0")?;
        inject.set_write_timeout(Some(Duration::from_millis(200)))?;
        local.set_read_timeout(Some(Duration::from_millis(100)))?;

        let stop = Arc::new(AtomicBool::new(false));
        let n1 = Arc::new(AtomicBool::new(false));
        let uid = Arc::new(AtomicU64::new(config.node_uid));
        let stats = Arc::new(TunnelStats::default());
        let links: Arc<Mutex<HashMap<usize, Box<dyn Link>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, event_rx) = mpsc::channel::<Event>();
        let (command_tx, command_rx) = mpsc::channel::<Command>();
        let mut threads = Vec::new();

        // The tunnel starts empty: links are added as RFCOMM peers appear.
        let mut next_link = 0usize;
        for link in config.links {
            attach_link(
                link,
                next_link,
                &event_tx,
                &command_tx,
                &links,
                &stats,
                &mut threads,
            )?;
            next_link += 1;
        }

        let udp_reader = local.try_clone()?;
        let udp_stop = stop.clone();
        let events = event_tx.clone();
        threads.push(
            thread::Builder::new()
                .name("siren-bt-udp".into())
                .spawn(move || {
                    let mut buffer = vec![0u8; 65_536];
                    while !udp_stop.load(Ordering::Relaxed) {
                        match udp_reader.recv_from(&mut buffer) {
                            Ok((len, _)) => {
                                let Some((dest, payload)) =
                                    parse_local_datagram(&buffer[..len])
                                else {
                                    continue;
                                };
                                let event = Event::Local {
                                    dest,
                                    payload: payload.to_vec(),
                                };
                                if events.send(event).is_err() {
                                    break;
                                }
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    ErrorKind::WouldBlock | ErrorKind::TimedOut
                                ) => {}
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|e| CoreError::Backend(format!("spawn bt udp: {e}")))?,
        );

        let origin_bit = 1u64 << config.node_bit;
        let ttl = config.ttl.max(1);
        let self_uid = uid.clone();
        let mesh_bind = config.mesh_bind;
        let relay_stop = stop.clone();
        let relay_stats = stats.clone();
        let relay_links = links.clone();
        let relay_n1 = n1.clone();
        threads.push(
            thread::Builder::new()
                .name("siren-bt-relay".into())
                .spawn(move || {
                    let mut seen = Seen::new();
                    let mut writers: HashMap<usize, LinkWriter> = HashMap::new();
                    let mut link_origins: HashMap<usize, (u64, Instant)> = HashMap::new();
                    let mut attached_at: HashMap<usize, Instant> = HashMap::new();
                    let mut window: u64 = 0;
                    let mut last_sweep = Instant::now();
                    while !relay_stop.load(Ordering::Relaxed) {
                        match command_rx.try_recv() {
                            Ok(Command::AddLink { id, writer }) => {
                                writers.insert(id, LinkWriter::new(writer));
                                attached_at.insert(id, Instant::now());
                                relay_stats.links.store(writers.len(), Ordering::Relaxed);
                            }
                            Ok(Command::CloseLink { id }) => {
                                writers.remove(&id);
                                attached_at.remove(&id);
                                relay_stats.links.store(writers.len(), Ordering::Relaxed);
                            }
                            Err(mpsc::TryRecvError::Empty) => {}
                            Err(mpsc::TryRecvError::Disconnected) => break,
                        }
                        let event = match event_rx.recv_timeout(Duration::from_millis(20)) {
                            Ok(event) => event,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        };
                        match event {
                            Event::Local { dest, payload } => {
                                // In hub mode the mesh sender replaces both
                                // the broadcast local stream and the relay with
                                // one targeted mix per link.
                                if relay_n1.load(Ordering::Relaxed) && dest.is_none() {
                                    continue;
                                }
                                let targets: Vec<usize> = match dest {
                                    Some(origin) => writers
                                        .keys()
                                        .filter(|id| {
                                            link_origins
                                                .get(id)
                                                .map(|(seen, _)| *seen == origin)
                                                .unwrap_or(false)
                                        })
                                        .copied()
                                        .collect(),
                                    None => writers.keys().copied().collect(),
                                };
                                if targets.is_empty() {
                                    continue;
                                }
                                let bytes = Envelope {
                                    origin: origin_bit,
                                    ttl,
                                    hop: 0,
                                    payload,
                                }
                                .encode_compact();
                                relay_stats
                                    .tx_packets
                                    .fetch_add(targets.len() as u64, Ordering::Relaxed);
                                relay_stats.tx_bytes.fetch_add(
                                    (bytes.len() * targets.len()) as u64,
                                    Ordering::Relaxed,
                                );
                                for id in targets {
                                    let Some(writer) = writers.get_mut(&id) else {
                                        continue;
                                    };
                                    writer.offer(bytes.clone());
                                    if let Some(skip) = writer.maybe_throttle(window, id) {
                                        if skip > 0 {
                                            log::info!("bt: link {id} congested, sending 1/{}", skip + 1);
                                        } else {
                                            log::info!("bt: link {id} recovered to full rate");
                                        }
                                    }
                                    if writer.drops > 0 {
                                        relay_stats.dropped.fetch_add(1, Ordering::Relaxed);
                                        writer.drops = 0;
                                    }
                                }
                            }
                            Event::LinkClosed(id) => {
                                writers.remove(&id);
                                link_origins.remove(&id);
                                if let Some(link) = relay_links.lock().unwrap().remove(&id) {
                                    link.shutdown_link();
                                }
                                relay_stats.links.store(writers.len(), Ordering::Relaxed);
                            }
                            Event::Link(source, mut envelope) => {
                                let own = self_uid.load(Ordering::Relaxed);
                                let payload_origin =
                                    crate::protocol::frame_identity(&envelope.payload)
                                        .map(|(origin, _, _)| origin)
                                        .unwrap_or(0);
                                let looped = envelope.origin & origin_bit != 0
                                    || (own != 0 && payload_origin == own);
                                if looped || envelope.ttl == 0 {
                                    relay_stats.dropped.fetch_add(1, Ordering::Relaxed);
                                    continue;
                                }
                                if !seen.insert(&envelope) {
                                    relay_stats.dropped.fetch_add(1, Ordering::Relaxed);
                                    continue;
                                }
                                relay_stats.rx_packets.fetch_add(1, Ordering::Relaxed);
                                relay_stats
                                    .rx_bytes
                                    .fetch_add(envelope.payload.len() as u64, Ordering::Relaxed);
                                relay_stats.last_rx_us.store(now_us(), Ordering::Relaxed);
                                if payload_origin != 0 {
                                    if relay_n1.load(Ordering::Relaxed) {
                                        // A reconnect arrives as a new link; the
                                        // previous one is stale even if its RFCOMM
                                        // session lingers. Keeping the "fresher"
                                        // link instead was tried and made things
                                        // worse: a half-open link keeps looking
                                        // alive here while the peer has given up on
                                        // it, so the newcomer got rejected, the peer
                                        // re-dialled immediately, and the pair
                                        // churned ~8 links/min with constant
                                        // carrier switches. Telling a duplicate
                                        // from a reconnect needs the link to say
                                        // who it is, not a timer.
                                        let old = link_origins
                                            .iter()
                                            .find(|(id, (origin, _))| {
                                                **id != source
                                                    && *origin == payload_origin
                                            })
                                            .map(|(id, _)| *id);
                                        if let Some(old) = old {
                                            writers.remove(&old);
                                            link_origins.remove(&old);
                                            attached_at.remove(&old);
                                            if let Some(link) =
                                                relay_links.lock().unwrap().remove(&old)
                                            {
                                                link.shutdown_link();
                                            }
                                            relay_stats
                                                .links
                                                .store(writers.len(), Ordering::Relaxed);
                                            log::info!(
                                                "bt: duplicate link {old} for origin {payload_origin:016x} closed"
                                            );
                                        }
                                    }
                                    link_origins.insert(source, (payload_origin, Instant::now()));
                                }
                                let _ = inject.send_to(&envelope.payload, mesh_bind);
                                // Hub mode: the N-1 mix already carries this
                                // stream to the other links.
                                if relay_n1.load(Ordering::Relaxed) {
                                    continue;
                                }
                                envelope.ttl -= 1;
                                envelope.hop = envelope.hop.saturating_add(1);
                                let bytes = envelope.encode_compact();
                                for (index, writer) in writers.iter_mut() {
                                    if *index != source {
                                        writer.offer(bytes.clone());
                                        if let Some(skip) = writer.maybe_throttle(window, *index) {
                                            if skip > 0 {
                                                log::info!(
                                                    "bt: link {index} congested, sending 1/{}",
                                                    skip + 1
                                                );
                                            } else {
                                                log::info!(
                                                    "bt: link {index} recovered to full rate"
                                                );
                                            }
                                        }
                                        if writer.drops > 0 {
                                            relay_stats
                                                .dropped
                                                .fetch_add(1, Ordering::Relaxed);
                                            writer.drops = 0;
                                        }
                                    }
                                }
                            }
                        }
                        window = window.wrapping_add(1);
                        // A leaf that reconnected can leave a half-open ACL
                        // behind. Never close a live link (that makes the leaf
                        // redial in a loop); only sweep links that have been
                        // silent for a while.
                        if last_sweep.elapsed() >= Duration::from_secs(5) {
                            last_sweep = Instant::now();
                            let stale: Vec<usize> = link_origins
                                .iter()
                                .filter(|(_, (_, at))| {
                                    at.elapsed() >= Duration::from_secs(10)
                                })
                                .map(|(id, _)| *id)
                                .collect();
                            // Links that never delivered a single frame are
                            // leftover half-open ACLs; they have no origin
                            // timestamp to age out, so close them by age.
                            let orphans: Vec<usize> = attached_at
                                .iter()
                                .filter(|(id, at)| {
                                    at.elapsed() >= Duration::from_secs(30)
                                        && !link_origins.contains_key(id)
                                })
                                .map(|(id, _)| *id)
                                .collect();
                            for id in orphans {
                                writers.remove(&id);
                                attached_at.remove(&id);
                                if let Some(link) = relay_links.lock().unwrap().remove(&id) {
                                    link.shutdown_link();
                                }
                                log::info!("bt: link {id} never carried audio, closed");
                            }
                            for id in stale {
                                writers.remove(&id);
                                attached_at.remove(&id);
                                link_origins.remove(&id);
                                if let Some(link) = relay_links.lock().unwrap().remove(&id) {
                                    link.shutdown_link();
                                }
                                log::info!("bt: link {id} silent for 10s, closed");
                            }
                            if !link_origins.is_empty() {
                                relay_stats
                                    .links
                                    .store(writers.len(), Ordering::Relaxed);
                            }
                        }
                    }
                })
                .map_err(|e| CoreError::Backend(format!("spawn bt relay: {e}")))?,
        );

        Ok(Self {
            local_addr,
            stop,
            uid,
            stats,
            links,
            next_link: AtomicUsize::new(1),
            commands: Mutex::new(command_tx),
            events: event_tx,
            threads: Mutex::new(threads),
            n1,
        })
    }

    /// Add a link while the tunnel runs. The connection is used from a
    /// dedicated reader thread and a dedicated writer thread.
    pub fn add_link(&self, link: Box<dyn Link>) -> Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(CoreError::Backend("tunnel is stopped".into()));
        }
        let id = self.next_link.fetch_add(1, Ordering::Relaxed);
        link.set_nodelay();
        log::info!("bt: link {id} opened ({})", link.description());
        let mut threads = self.threads.lock().unwrap();
        attach_link(
            link,
            id,
            &self.events,
            &self.commands.lock().unwrap(),
            &self.links,
            &self.stats,
            &mut threads,
        )?;
        Ok(id)
    }

    /// Close one link (used when a scanned candidate turns out not to be a
    /// Siren hub). Other links keep running.
    pub fn remove_link(&self, id: usize) {
        if let Some(link) = self.links.lock().unwrap().remove(&id) {
            link.shutdown_link();
        }
        let _ = self.commands.lock().unwrap().send(Command::CloseLink { id });
    }
}

/// Decode a locally produced datagram. A targeted envelope carries the
/// destination leaf origin (N-1 mix); everything else is a broadcast.
fn parse_local_datagram(datagram: &[u8]) -> Option<(Option<u64>, &[u8])> {
    if datagram.len() >= 11 && datagram[0] == MAGIC_TARGETED {
        let len = u16::from_le_bytes([datagram[1], datagram[2]]) as usize;
        if 11 + len > datagram.len() {
            return None;
        }
        let dest = u64::from_le_bytes(datagram[3..11].try_into().ok()?);
        Some((Some(dest), &datagram[11..11 + len]))
    } else {
        Some((None, datagram))
    }
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Wire one link into the relay: reader thread feeds events, writer thread
/// drains commands. `id` must be unique per tunnel.
fn attach_link(
    link: Box<dyn Link>,
    id: usize,
    events: &mpsc::Sender<Event>,
    commands: &mpsc::Sender<Command>,
    links: &Arc<Mutex<HashMap<usize, Box<dyn Link>>>>,
    _stats: &Arc<TunnelStats>,
    threads: &mut Vec<JoinHandle<()>>,
) -> Result<()> {
    let mut reader = link
        .try_clone_link()
        .map_err(|e| CoreError::Backend(format!("clone link: {e}")))?;
    let shutdown = link
        .try_clone_link()
        .map_err(|e| CoreError::Backend(format!("clone link: {e}")))?;
    links.lock().unwrap().insert(id, shutdown);

    let events = events.clone();
    threads.push(
        thread::Builder::new()
            .name(format!("siren-bt-rx-{id}"))
            .spawn(move || {
                let mut head = [0u8; HEADER_LEN];
                loop {
                    // The first byte selects the envelope version.
                    if read_exact(reader.as_mut(), &mut head[..1]).is_err() {
                        break;
                    }
                    let (len, origin, ttl, hop) = match head[0] {
                        MAGIC => {
                            if read_exact(reader.as_mut(), &mut head[1..HEADER_LEN]).is_err() {
                                break;
                            }
                            let len = u16::from_le_bytes([head[2], head[3]]) as usize;
                            let origin =
                                u64::from_le_bytes(head[4..12].try_into().unwrap_or([0; 8]));
                            (len, origin, head[12], head[13])
                        }
                        MAGIC_COMPACT => {
                            if read_exact(reader.as_mut(), &mut head[1..HEADER_LEN_COMPACT])
                                .is_err()
                            {
                                break;
                            }
                            let len = u16::from_le_bytes([head[1], head[2]]) as usize;
                            (len, 0, head[3] >> 4, head[3] & 0x0F)
                        }
                        _ => break,
                    };
                    if len > MAX_PAYLOAD {
                        break;
                    }
                    let mut payload = vec![0u8; len];
                    if read_exact(reader.as_mut(), &mut payload).is_err() {
                        break;
                    }
                    let envelope = Envelope {
                        origin,
                        ttl,
                        hop,
                        payload,
                    };
                    if events.send(Event::Link(id, envelope)).is_err() {
                        break;
                    }
                }
                log::info!("bt: link {id} reader ended ({})", reader.description());
                let _ = events.send(Event::LinkClosed(id));
            })
            .map_err(|e| CoreError::Backend(format!("spawn bt reader: {e}")))?,
    );

    // Bounded queue: a stalled Bluetooth link must not buffer the whole
    // stream in memory. Wi-Fi carries the same frames in parallel, so dropping
    // here only means the far end keeps fewer Bluetooth fallback copies.
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(WRITE_QUEUE_LIMIT);
    commands
        .send(Command::AddLink { id, writer: tx })
        .map_err(|_| CoreError::Backend("tunnel relay stopped".into()))?;
    let mut writer = link;
    threads.push(
        thread::Builder::new()
            .name(format!("siren-bt-tx-{id}"))
            .spawn(move || {
                for bytes in rx {
                    if write_all(writer.as_mut(), &bytes).is_err() {
                        log::info!("bt: link {id} writer failed ({})", writer.description());
                        break;
                    }
                }
                writer.shutdown_link();
            })
            .map_err(|e| CoreError::Backend(format!("spawn bt writer: {e}")))?,
    );
    Ok(())
}

fn read_exact(link: &mut dyn Link, mut buffer: &mut [u8]) -> std::io::Result<()> {
    while !buffer.is_empty() {
        match link.read_link(buffer) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "link closed",
                ))
            }
            Ok(read) => buffer = &mut buffer[read..],
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn write_all(link: &mut dyn Link, mut buffer: &[u8]) -> std::io::Result<()> {
    while !buffer.is_empty() {
        match link.write_link(buffer) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::WriteZero,
                    "link closed",
                ))
            }
            Ok(written) => buffer = &buffer[written..],
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop();
        // Relays exit on their own; join the short-lived ones only. Reader
        // threads may be blocked in RFCOMM and are closed by `stop`.
        if let Ok(mut threads) = self.threads.lock() {
            for handle in threads.drain(..) {
                let _ = handle.join();
            }
        }
    }
}

enum Event {
    /// Locally produced audio. `dest` selects the destination leaf's origin
    /// (N-1 mix); `None` broadcasts to every link.
    Local { dest: Option<u64>, payload: Vec<u8> },
    Link(usize, Envelope),
    LinkClosed(usize),
}

enum Command {
    AddLink {
        id: usize,
        writer: mpsc::SyncSender<Vec<u8>>,
    },
    CloseLink {
        id: usize,
    },
}

/// One outgoing link with congestion-aware throttling. A narrow (or RF-poor)
/// link that keeps dropping must not keep consuming the shared Bluetooth
/// airtime: every congested window halves its offered rate so healthy links
/// in the same piconet keep their throughput.
struct LinkWriter {
    tx: mpsc::SyncSender<Vec<u8>>,
    drops: u64,
    window_drops: u64,
    skip: u8,
    counter: u64,
}

impl LinkWriter {
    fn new(tx: mpsc::SyncSender<Vec<u8>>) -> Self {
        Self {
            tx,
            drops: 0,
            window_drops: 0,
            skip: 0,
            counter: 0,
        }
    }

    /// Offer one packet; returns false when intentionally skipped for pacing.
    fn offer(&mut self, bytes: Vec<u8>) -> bool {
        self.counter = self.counter.wrapping_add(1);
        if self.skip > 0 && self.counter % (self.skip as u64 + 1) != 0 {
            return false;
        }
        match self.tx.try_send(bytes) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(_)) => {
                self.drops += 1;
                self.window_drops += 1;
                false
            }
            Err(mpsc::TrySendError::Disconnected(_)) => false,
        }
    }

    /// Called once per relayed packet; re-evaluates the throttle every ~200
    /// packets (about one second at 5 ms frames / 200 fps).
    fn maybe_throttle(&mut self, window: u64, _id: usize) -> Option<u8> {
        if window % 200 != 0 {
            return None;
        }
        let previous = self.skip;
        if self.window_drops > 40 {
            self.skip = (self.skip + 1).min(4);
        } else if self.window_drops == 0 {
            self.skip = self.skip.saturating_sub(1);
        }
        self.window_drops = 0;
        (self.skip != previous).then_some(self.skip)
    }
}

struct Seen {
    order: VecDeque<(u64, u16, u32)>,
    set: HashSet<(u64, u16, u32)>,
}

impl Seen {
    fn new() -> Self {
        Self {
            order: VecDeque::new(),
            set: HashSet::new(),
        }
    }

    fn insert(&mut self, envelope: &Envelope) -> bool {
        let payload = &envelope.payload;
        let Some((origin, stream_id, sequence)) = crate::protocol::frame_identity(payload) else {
            return true;
        };
        let key = (if origin != 0 { origin } else { envelope.origin }, stream_id, sequence);
        if !self.set.insert(key) {
            return false;
        }
        self.order.push_back(key);
        if self.order.len() > DEDUPE_LIMIT {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targeted_local_envelope_is_parsed() {
        let payload = b"frame";
        let mut packet = vec![0xB3];
        packet.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        packet.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        packet.extend_from_slice(payload);
        let (dest, body) = parse_local_datagram(&packet).expect("targeted");
        assert_eq!(dest, Some(0x1122_3344_5566_7788));
        assert_eq!(body, payload);

        let (dest, body) = parse_local_datagram(b"plain").expect("broadcast");
        assert_eq!(dest, None);
        assert_eq!(body, b"plain");

        let mut truncated = vec![0xB3];
        truncated.extend_from_slice(&20u16.to_le_bytes());
        truncated.extend_from_slice(&7u64.to_le_bytes());
        truncated.extend_from_slice(b"xy");
        assert!(parse_local_datagram(&truncated).is_none());
    }

    #[test]
    fn envelope_roundtrip() {
        let envelope = Envelope {
            origin: 0b1010,
            ttl: 4,
            hop: 1,
            payload: vec![1, 2, 3, 4],
        };
        let bytes = envelope.encode();
        let (decoded, used) = Envelope::decode(&bytes).expect("decode");
        assert_eq!(used, bytes.len());
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn compact_envelope_roundtrip() {
        let envelope = Envelope {
            origin: 0,
            ttl: 5,
            hop: 2,
            payload: vec![9, 8, 7],
        };
        let bytes = envelope.encode_compact();
        assert_eq!(bytes.len(), HEADER_LEN_COMPACT + 3);
        let (decoded, used) = Envelope::decode(&bytes).expect("decode compact");
        assert_eq!(used, bytes.len());
        assert_eq!(decoded.ttl, 5);
        assert_eq!(decoded.hop, 2);
        assert_eq!(decoded.payload, vec![9, 8, 7]);
    }

    #[test]
    fn dedupe_same_stream_sequence() {
        let mut seen = Seen::new();
        let payload = {
            let mut p = vec![b'S', b'R', 1, 0, 0, 0];
            p.extend_from_slice(&7u16.to_le_bytes());
            p.extend_from_slice(&42u32.to_le_bytes());
            p
        };
        let envelope = Envelope {
            origin: 2,
            ttl: 5,
            hop: 0,
            payload,
        };
        assert!(seen.insert(&envelope));
        assert!(!seen.insert(&envelope));
        let mut other = envelope.clone();
        other.payload[8] = 43;
        assert!(seen.insert(&other));
    }
}
#[cfg(test)]
mod tunnel_tests {
    use super::*;
    use crate::protocol::{Codec, FrameHeader};
    use std::net::UdpSocket;
    use std::time::{Duration, Instant};

    #[test]
    fn compact_envelope_carries_a_decodable_frame() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let addr = listener.local_addr().expect("addr");
        let client = TcpStream::connect(addr).expect("client");
        let (server, _) = listener.accept().expect("accept");
        server.set_nodelay(true).ok();
        let tunnel = Tunnel::start(TunnelConfig {
            mesh_bind: "127.0.0.1:1".parse().unwrap(),
            node_bit: 1,
            node_uid: 0x1234,
            ttl: 5,
            links: vec![Box::new(server)],
        })
        .expect("tunnel");

        let mut frame = FrameHeader::new(Codec::Opus, 5, 9, 1, 0, 0x1234)
            .encode_compact()
            .to_vec();
        frame.extend_from_slice(&[7u8; 40]);
        let sender = UdpSocket::bind("127.0.0.1:0").expect("udp");
        sender
            .send_to(&frame, tunnel.local_addr())
            .expect("send local");
        std::thread::sleep(Duration::from_millis(300));

        let mut client = client;
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let mut magic = [0u8; 1];
        client.read_exact(&mut magic).expect("magic");
        assert_eq!(magic[0], MAGIC_COMPACT, "writer must emit a compact envelope");
        let mut head = [0u8; HEADER_LEN_COMPACT - 1];
        client.read_exact(&mut head).expect("head");
        let len = u16::from_le_bytes([head[0], head[1]]) as usize;
        assert_eq!(len, frame.len());
        let mut payload = vec![0u8; len];
        client.read_exact(&mut payload).expect("payload");
        assert_eq!(&payload[0..2], b"SR", "payload must be the audio frame");
        let (decoded, _) = FrameHeader::decode(&payload).expect("decode frame");
        assert_eq!(decoded.seq, 1);
        assert_eq!(decoded.origin, 0x1234);
        tunnel.stop();
    }
}
