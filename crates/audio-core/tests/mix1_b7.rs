//! MIX1 (#64 / former B7): one hub, two *pure-Bluetooth* leaves.
//!
//! The leaves pin the hub through Wi-Fi fronts that are cut for the whole
//! test, so the hub never receives a single Wi-Fi packet from either of
//! them — every leaf sample arrives through the simulated RFCOMM tunnel and
//! is classified as the BT carrier. That is the exact condition under which
//! the hub must relay audio as one N−1 mix stream (0xFFF0, Opus, v4 compact
//! header with an origin mask) per leaf link instead of forwarding the
//! individual streams.
//!
//! Assertions:
//! * both leaves dial and hold a BT link;
//! * the mix stream arrives at *both* leaves (audio, not silence);
//! * `mix_count == 2` on each leaf: the newest header's mask folds in the
//!   hub's own tone plus the *other* leaf (N−1 — a leaf never receives its
//!   own audio back);
//! * no direct (Wi-Fi) stream from the hub exists on the leaves;
//! * the receive water level stays bounded — the mix path must neither
//!   starve (underrun storm) nor balloon past the trim ceiling.

use std::io;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use audio_core::hybrid::{HybridConfig, HybridNode, HybridOptions};
use audio_core::mesh::MeshConfig;
use audio_core::engine::SendSource;
use audio_core::Result;
use audio_core::net::{spawn_link, LinkHandle};
use audio_core::rfcomm::{BtListener, BtTransport};

/// `…0001 < …0002 < …0003`: deterministic election, node A is the hub.
const HUB_ID: &str = "aaaaaaaaaaaa0001";
const LEAF_A_ID: &str = "aaaaaaaaaaaa0002";
const LEAF_B_ID: &str = "aaaaaaaaaaaa0003";

/// Simulated Bluetooth adapter over loopback TCP (same trick as
/// `bt_overlay.rs`).
struct SimBt {
    listener: TcpListener,
}

impl SimBt {
    fn new() -> io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind("127.0.0.1:0")?,
        })
    }

    fn address(&self) -> String {
        format!("sim-{}", self.listener.local_addr().unwrap().port())
    }
}

impl BtTransport for SimBt {
    fn local_address(&self) -> Option<String> {
        Some(self.address())
    }

    fn connect(&self, address: &str, _channel: u8) -> Result<TcpStream> {
        let port: u16 = address
            .strip_prefix("sim-")
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| audio_core::CoreError::Backend(format!("bad sim address {address}")))?;
        TcpStream::connect(("127.0.0.1", port)).map_err(|e| audio_core::CoreError::Backend(e.to_string()))
    }

    fn listen(&self, _channel: u8) -> Result<Box<dyn BtListener>> {
        Ok(Box::new(SimListener {
            listener: self.listener.try_clone()?,
        }))
    }
}

struct SimListener {
    listener: TcpListener,
}

impl BtListener for SimListener {
    fn accept(&self) -> Result<TcpStream> {
        self.listener
            .accept()
            .map(|(stream, _)| stream)
            .map_err(|e| audio_core::CoreError::Backend(e.to_string()))
    }

    fn close(&self) {
        if let Ok(addr) = self.listener.local_addr() {
            let _ = TcpStream::connect(addr);
        }
    }
}

fn reserve_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .expect("reserve port")
        .local_addr()
        .expect("port")
        .port()
}

struct TestMesh {
    hub: HybridNode,
    leaf_a: HybridNode,
    leaf_b: HybridNode,
    hub_uid: u64,
    /// leaf A -> hub Wi-Fi front, cut for the whole test. Held (not read)
    /// so the link threads stay alive for the duration of the test.
    #[allow(dead_code)]
    front_a: LinkHandle,
    /// leaf B -> hub Wi-Fi front, cut for the whole test.
    #[allow(dead_code)]
    front_b: LinkHandle,
}

fn build_trio() -> TestMesh {
    let (hub_port, leaf_a_port, leaf_b_port, front_a_port, front_b_port) = (
        reserve_port(),
        reserve_port(),
        reserve_port(),
        reserve_port(),
        reserve_port(),
    );
    let hub_bt = SimBt::new().expect("sim bt hub");
    let leaf_a_bt = SimBt::new().expect("sim bt leaf a");
    let leaf_b_bt = SimBt::new().expect("sim bt leaf b");

    // The leaves' only Wi-Fi route to the hub, dead on arrival: their static
    // peer points here, every datagram is dropped, so the hub classifies
    // both leaves as Bluetooth-only.
    let front_a = spawn_link(
        &format!("127.0.0.1:{front_a_port}"),
        format!("127.0.0.1:{hub_port}").parse().expect("addr"),
        Default::default(),
    )
    .expect("front a link")
    .1;
    let front_b = spawn_link(
        &format!("127.0.0.1:{front_b_port}"),
        format!("127.0.0.1:{hub_port}").parse().expect("addr"),
        Default::default(),
    )
    .expect("front b link")
    .1;
    front_a.cut();
    front_b.cut();

    let node = |name: &str, id: &str, port: u16, peers: Vec<String>, bt: &SimBt, exempt: Vec<u16>| {
            let mut mesh_config = MeshConfig::default();
            mesh_config.bind = "127.0.0.1".into();
            mesh_config.port = port;
            mesh_config.name = name.into();
            mesh_config.discovery = false;
            mesh_config.advertise = false;
            mesh_config.output = false;
            mesh_config.source = SendSource::Tone;
            mesh_config.frame_ms = 5;
            mesh_config.node_id = Some(id.to_string());
            mesh_config.static_peers = peers;
            mesh_config.bt_exempt_ports = exempt;
            HybridNode::start(
                mesh_config,
                HybridOptions {
                    config: HybridConfig::default(),
                    transport: Some(std::sync::Arc::new(SimBt {
                        listener: bt.listener.try_clone().expect("clone listener"),
                    })),
                    state_path: Some(std::env::temp_dir().join(format!("siren-mix1-{name}"))),
                    ..HybridOptions::default()
                },
            )
            .expect("start node")
        };

    // The hub has no static peers at all: with a local adapter and no
    // BT-hinted candidates it self-elects as the solo hub and listens. The
    // leaves pin the (dead) Wi-Fi front carrying the hub's id + bt hint —
    // the dial input.
    let hub = node("mix1-hub", HUB_ID, hub_port, vec![], &hub_bt, vec![front_a_port, front_b_port]);
    let leaf_a = node(
        "mix1-leaf-a",
        LEAF_A_ID,
        leaf_a_port,
        vec![format!(
            "127.0.0.1:{front_a_port}?id={HUB_ID}&bt={}&name=hub",
            hub_bt.address()
        )],
        &leaf_a_bt,
        vec![],
    );
    let leaf_b = node(
        "mix1-leaf-b",
        LEAF_B_ID,
        leaf_b_port,
        vec![format!(
            "127.0.0.1:{front_b_port}?id={HUB_ID}&bt={}&name=hub",
            hub_bt.address()
        )],
        &leaf_b_bt,
        vec![],
    );
    let hub_uid = hub.mesh().uid();
    TestMesh {
        hub,
        leaf_a,
        leaf_b,
        hub_uid,
        front_a,
        front_b,
    }
}

impl Drop for TestMesh {
    fn drop(&mut self) {
        self.hub.stop();
        self.leaf_a.stop();
        self.leaf_b.stop();
    }
}

fn mix_stream(mesh: &TestMesh, leaf: &HybridNode) -> Option<audio_core::metrics::StreamSnapshot> {
    use audio_core::protocol::MIX_STREAM_ID;
    leaf.snapshot()
        .center
        .streams
        .into_iter()
        .find(|s| s.origin == mesh.hub_uid && s.stream_id == MIX_STREAM_ID)
}

fn direct_stream(mesh: &TestMesh, leaf: &HybridNode) -> Option<audio_core::metrics::StreamSnapshot> {
    use audio_core::protocol::MIX_STREAM_ID;
    leaf.snapshot()
        .center
        .streams
        .into_iter()
        .find(|s| s.origin == mesh.hub_uid && s.stream_id != MIX_STREAM_ID)
}

fn wait_for(pred: impl Fn() -> bool, timeout: Duration, what: &str) {
    let start = Instant::now();
    while !pred() {
        assert!(
            start.elapsed() < timeout,
            "timed out after {timeout:?} waiting for: {what}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn n1_mix_reaches_two_pure_bt_leaves() {
    let mesh = build_trio();

    // Both leaves dial the hub over the simulated RFCOMM transport.
    wait_for(
        || mesh.leaf_a.snapshot().bt.links > 0 && mesh.leaf_b.snapshot().bt.links > 0,
        Duration::from_secs(20),
        "both leaves to dial the hub over the simulated transport",
    );
    assert_eq!(mesh.hub.snapshot().bt.role, "hub");
    assert_eq!(
        mesh.hub.snapshot().bt.links, 2,
        "hub should hold both leaf links"
    );

    // The N−1 mix must arrive at *both* leaves.
    wait_for(
        || {
            mix_stream(&mesh, &mesh.leaf_a).map(|s| s.packets > 100).unwrap_or(false)
                && mix_stream(&mesh, &mesh.leaf_b).map(|s| s.packets > 100).unwrap_or(false)
        },
        Duration::from_secs(15),
        "the 0xFFF0 mix to reach both leaves",
    );

    // mix_count == 2: hub's own tone + the other leaf, never the
    // destination's own audio (N−1).
    wait_for(
        || {
            [(&mesh.leaf_a, &mesh.leaf_b), (&mesh.leaf_b, &mesh.leaf_a)].iter().all(
                |(dest, other)| {
                    mix_stream(&mesh, dest)
                        .map(|s| {
                            s.mix_count == 2
                                && s.rms_db > -60.0
                                && s.path == "bt"
                        })
                        .unwrap_or(false)
                        && other.snapshot().center.streams.len() >= 1
                },
            )
        },
        Duration::from_secs(15),
        "both leaves to see a 2-source non-silent BT mix",
    );

    // No individual Wi-Fi stream from the hub may exist: the leaves are
    // BT-only, so relaying per-stream would defeat the N−1 design.
    for leaf_name in ["leaf_a", "leaf_b"] {
        let leaf = match leaf_name {
            "leaf_a" => &mesh.leaf_a,
            _ => &mesh.leaf_b,
        };
        assert!(
            direct_stream(&mesh, leaf).is_none(),
            "{leaf_name} must not receive a direct (Wi-Fi) stream from the hub"
        );
    }

    // Water level: the mix receive buffer must stay between audible and the
    // trim ceiling (2×target+10 ms, target 100 ms → 210 ms; allow margin for
    // the sampler racing a trim). Underruns should stay at startup noise.
    let mut max_buffer = 0.0f64;
    let mut underruns = 0u32;
    for _ in 0..30 {
        for leaf in [&mesh.leaf_a, &mesh.leaf_b] {
            if let Some(s) = mix_stream(&mesh, leaf) {
                max_buffer = max_buffer.max(s.buffer_ms);
                underruns = underruns.max(s.underruns);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        max_buffer > 1.0,
        "mix buffer should hold real audio, max {max_buffer} ms"
    );
    assert!(
        max_buffer <= 300.0,
        "mix water level ballooned to {max_buffer} ms (trim path not bounding it)"
    );
    assert!(
        underruns <= 4,
        "mix starved {underruns} times on a lossless simulated link"
    );
}
