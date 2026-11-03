//! Programmable lossy UDP link for deterministic mesh tests (F19).
//!
//! A [`Link`] sits between two real UDP endpoints: a node sends to the
//! link's front address, and a forwarder thread delivers every datagram to
//! the real target after the configured delay and jitter — or drops it per
//! the current profile. Everything runs in-process on loopback, so the
//! scenarios in `tests/link_sim.rs` need no privileges, no extra hardware
//! and no real network; `cargo test` runs them anywhere.
//!
//! Profiles are swapped at runtime through the [`LinkHandle`], which is how
//! the tests stage faults (loss, latency, partition) without restarting the
//! nodes. The RNG is seeded per link, so a scenario is reproducible run to
//! run as long as the packet order is.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Impairments applied to every datagram crossing the link.
#[derive(Clone, Debug)]
pub struct LinkProfile {
    /// Fixed one-way delay added to every packet.
    pub base_delay: Duration,
    /// Uniform extra delay (0..jitter) drawn per packet.
    pub jitter: Duration,
    /// Independent loss probability, in percent (0..100).
    pub loss_percent: f64,
    /// Gilbert-style burst loss: after a drop, the next `burst_len - 1`
    /// packets are dropped unconditionally (0 = bursts off). Losses cluster,
    /// which is what a congested RFCOMM link looks like and what hurts the
    /// receiver far more than the same average loss spread out.
    pub burst_len: u32,
    /// Sustained throughput ceiling in kbps; datagrams exceeding the token
    /// bucket are dropped (`dropped_bandwidth`). `None` = unlimited.
    pub bandwidth_kbps: Option<f64>,
}

impl Default for LinkProfile {
    fn default() -> Self {
        Self {
            base_delay: Duration::ZERO,
            jitter: Duration::ZERO,
            loss_percent: 0.0,
            burst_len: 0,
            bandwidth_kbps: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LinkStats {
    pub forwarded: u64,
    pub dropped_loss: u64,
    pub dropped_cut: u64,
    pub dropped_bandwidth: u64,
    /// Longest run of consecutive drops seen so far (burst diagnostics).
    pub max_drop_run: u64,
}

struct LinkShared {
    profile: Mutex<LinkProfile>,
    /// Hard partition: drop everything while set (`LinkHandle::cut`).
    cut: AtomicBool,
    forwarded: AtomicU64,
    dropped_loss: AtomicU64,
    dropped_cut: AtomicU64,
    dropped_bandwidth: AtomicU64,
    max_drop_run: AtomicU64,
}

/// Runtime control over one direction of a link.
#[derive(Clone)]
pub struct LinkHandle {
    shared: Arc<LinkShared>,
}

impl LinkHandle {
    pub fn set_profile(&self, profile: LinkProfile) {
        *self.shared.profile.lock().unwrap() = profile;
    }

    /// Simulate a link failure: every datagram is dropped until
    /// [`LinkHandle::restore`].
    pub fn cut(&self) {
        self.shared.cut.store(true, Ordering::Relaxed);
    }

    pub fn restore(&self) {
        self.shared.cut.store(false, Ordering::Relaxed);
    }

    pub fn stats(&self) -> LinkStats {
        let shared = &self.shared;
        LinkStats {
            forwarded: shared.forwarded.load(Ordering::Relaxed),
            dropped_loss: shared.dropped_loss.load(Ordering::Relaxed),
            dropped_cut: shared.dropped_cut.load(Ordering::Relaxed),
            dropped_bandwidth: shared.dropped_bandwidth.load(Ordering::Relaxed),
            max_drop_run: shared.max_drop_run.load(Ordering::Relaxed),
        }
    }
}

/// xorshift64*: small, dependency-free and deterministic per seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct Pending {
    due: Instant,
    data: Vec<u8>,
}

/// Start a forwarder thread: bind `front` (e.g. `127.0.0.1:0`) and relay
/// every datagram to `target` under `profile`. Returns the resolved front
/// address (what nodes put into `static_peers`) and the runtime handle.
pub fn spawn_link(
    front: &str,
    target: SocketAddr,
    profile: LinkProfile,
) -> io::Result<(SocketAddr, LinkHandle)> {
    let socket = UdpSocket::bind(front)?;
    socket.set_read_timeout(Some(Duration::from_millis(2)))?;
    let addr = socket.local_addr()?;

    let shared = Arc::new(LinkShared {
        profile: Mutex::new(profile),
        cut: AtomicBool::new(false),
        forwarded: AtomicU64::new(0),
        dropped_loss: AtomicU64::new(0),
        dropped_cut: AtomicU64::new(0),
        dropped_bandwidth: AtomicU64::new(0),
        max_drop_run: AtomicU64::new(0),
    });
    let handle = LinkHandle {
        shared: shared.clone(),
    };

    let seed = (addr.port() as u64) ^ 0x9E37_79B9_7F4A_7C15;
    let poll = Duration::from_millis(2);
    std::thread::Builder::new()
        .name("siren-link-sim".into())
        .spawn(move || forward_loop(socket, target, shared, seed, poll))?;
    Ok((addr, handle))
}

fn forward_loop(
    socket: UdpSocket,
    target: SocketAddr,
    shared: Arc<LinkShared>,
    seed: u64,
    poll: Duration,
) {
    let mut rng = Rng::new(seed);
    // Min-heap of scheduled deliveries, keyed by due time.
    let mut queue: BinaryHeap<Reverse<(Instant, u64, Vec<u8>)>> = BinaryHeap::new();
    let mut ticket: u64 = 0;
    let mut buffer = vec![0u8; 65535];
    // Gilbert burst state: packets left to drop in the current burst.
    let mut burst_remaining: u32 = 0;
    let mut drop_run: u64 = 0;
    // Token bucket for the bandwidth cap.
    let mut tokens: f64 = 0.0;
    let mut last_refill = Instant::now();
    loop {
        // Deliver everything that came due.
        let now = Instant::now();
        while let Some(Reverse((due, _, _data))) = queue.peek() {
            if *due > now {
                break;
            }
            let Reverse((_, _, data)) = queue.pop().unwrap();
            if socket.send_to(&data, target).is_ok() {
                shared.forwarded.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Wait for the next packet, but wake up when the head of the queue
        // is due (whichever comes first).
        let wait = queue
            .peek()
            .map(|Reverse((due, _, _))| due.saturating_duration_since(Instant::now()))
            .unwrap_or(poll)
            .min(poll);
        socket.set_read_timeout(Some(wait.max(Duration::from_micros(1)))).ok();
        match socket.recv_from(&mut buffer) {
            Ok((len, _from)) => {
                if shared.cut.load(Ordering::Relaxed) {
                    shared.dropped_cut.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let profile = shared.profile.lock().unwrap().clone();

                // Token bucket: refill by elapsed time, drop what it cannot
                // carry. The bucket holds ~250 ms of the configured rate so
                // short bursts still pass.
                if let Some(kbps) = profile.bandwidth_kbps {
                    let now = Instant::now();
                    tokens += now.duration_since(last_refill).as_secs_f64()
                        * kbps
                        * 1000.0
                        / 8.0;
                    last_refill = now;
                    let capacity = kbps * 1000.0 / 8.0 * 0.25;
                    tokens = tokens.min(capacity);
                    if (len as f64) > tokens {
                        shared.dropped_bandwidth.fetch_add(1, Ordering::Relaxed);
                        drop_run += 1;
                        shared
                            .max_drop_run
                            .fetch_max(drop_run, Ordering::Relaxed);
                        continue;
                    }
                    tokens -= len as f64;
                }

                let burst = profile.burst_len;
                let dropped = if burst_remaining > 0 {
                    burst_remaining -= 1;
                    true
                } else if profile.loss_percent > 0.0
                    && rng.next_f64() * 100.0 < profile.loss_percent
                {
                    if burst > 1 {
                        burst_remaining = burst - 1;
                    }
                    true
                } else {
                    false
                };
                if dropped {
                    shared.dropped_loss.fetch_add(1, Ordering::Relaxed);
                    drop_run += 1;
                    shared.max_drop_run.fetch_max(drop_run, Ordering::Relaxed);
                    continue;
                }
                drop_run = 0;
                let jitter = if profile.jitter.is_zero() {
                    Duration::ZERO
                } else {
                    Duration::from_secs_f64(profile.jitter.as_secs_f64() * rng.next_f64())
                };
                ticket += 1;
                queue.push(Reverse((
                    Instant::now() + profile.base_delay + jitter,
                    ticket,
                    buffer[..len].to_vec(),
                )));
            }
            Err(_) => {}
        }
    }
}
