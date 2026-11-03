//! In-memory Bluetooth overlay: two real `HybridNode`s elect hub/leaf and
//! dual-stack over a simulated RFCOMM transport (PLAN P0-1 milestone 2).
//!
//! `SimBt` implements [`audio_core::rfcomm::BtTransport`] with plain
//! loopback TCP: the adapter "address" is `sim-<port>`, `connect` dials it,
//! `listen` serves it. Wi-Fi runs through the same lossy links as
//! `link_sim.rs`, which lets this test cut Wi-Fi and watch audio survive on
//! the Bluetooth carrier — the core dual-stack guarantee, deterministically.

use std::io;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use audio_core::hybrid::{HybridConfig, HybridNode, HybridOptions};
use audio_core::mesh::MeshConfig;
use audio_core::engine::SendSource;
use audio_core::Result;
use audio_core::net::{spawn_link, LinkHandle};
use audio_core::rfcomm::{BtListener, BtTransport};

/// node ids are the low 16 hex chars of the uid, so these give a
/// deterministic election: …0001 < …0002, node A is the hub.
const HUB_ID: &str = "aaaaaaaaaaaa0001";
const LEAF_ID: &str = "aaaaaaaaaaaa0002";

/// Simulated Bluetooth adapter over loopback TCP.
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

    /// `TcpListener` clones cannot be closed directly; dial ourselves once to
    /// unblock a waiting `accept`, mirroring what closing a real listener
    /// does to a blocked peer.
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
    leaf: HybridNode,
    hub_uid: u64,
    /// hub -> leaf Wi-Fi direction.
    forward: LinkHandle,
    /// leaf -> hub Wi-Fi direction.
    reverse: LinkHandle,
}

fn build_pair() -> TestMesh {
    let (hub_port, leaf_port, fwd_port, rev_port) = (
        reserve_port(),
        reserve_port(),
        reserve_port(),
        reserve_port(),
    );
    let hub_bt = SimBt::new().expect("sim bt hub");
    let leaf_bt = SimBt::new().expect("sim bt leaf");

    let forward = spawn_link(
        &format!("127.0.0.1:{fwd_port}"),
        format!("127.0.0.1:{leaf_port}").parse().expect("addr"),
        Default::default(),
    )
    .expect("forward link")
    .1;
    let reverse = spawn_link(
        &format!("127.0.0.1:{rev_port}"),
        format!("127.0.0.1:{hub_port}").parse().expect("addr"),
        Default::default(),
    )
    .expect("reverse link")
    .1;

    let node = |name: &str, id: &str, port: u16, peers: Vec<String>, bt: &SimBt| {
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
            // Both Wi-Fi fronts are loopback ports; exempt both on every
            // node (each node receives from one of them) so the receiver
            // does not classify link-front traffic as the BT carrier.
            mesh_config.bt_exempt_ports = vec![fwd_port, rev_port];
            HybridNode::start(
                mesh_config,
                HybridOptions {
                    config: HybridConfig::default(),
                    transport: Some(std::sync::Arc::new(SimBt {
                        listener: bt.listener.try_clone().expect("clone listener"),
                    })),
                    state_path: Some(std::env::temp_dir().join(format!("siren-bt-sim-{name}"))),
                    ..HybridOptions::default()
                },
            )
            .expect("start node")
        };

    // Leaf pins the hub through the *reverse* link front (so `cut()` gates
    // its Wi-Fi), carrying the hub's id + bt hint — the election input. The
    // hub pins the leaf through the forward link front without a `bt`, so
    // only the hub is a listen candidate.
    let hub = node(
        "sim-hub",
        HUB_ID,
        hub_port,
        vec![format!("127.0.0.1:{fwd_port}?id={LEAF_ID}&name=leaf")],
        &hub_bt,
    );
    let leaf = node(
        "sim-leaf",
        LEAF_ID,
        leaf_port,
        vec![format!(
            "127.0.0.1:{rev_port}?id={HUB_ID}&bt={}&name=hub",
            hub_bt.address()
        )],
        &leaf_bt,
    );
    let hub_uid = hub.mesh().uid();
    TestMesh {
        hub,
        leaf,
        hub_uid,
        forward,
        reverse,
    }
}

impl Drop for TestMesh {
    fn drop(&mut self) {
        self.hub.stop();
        self.leaf.stop();
    }
}

/// Cumulative packets of the hub's N-1 mix stream (0xFFF0) at the leaf.
fn mix_stream_packets(mesh: &TestMesh) -> u64 {
    use audio_core::protocol::MIX_STREAM_ID;
    mesh.leaf
        .snapshot()
        .center
        .streams
        .into_iter()
        .find(|s| s.origin == mesh.hub_uid && s.stream_id == MIX_STREAM_ID)
        .map(|s| s.packets)
        .unwrap_or(0)
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

fn stream_from(mesh: &TestMesh) -> Option<audio_core::metrics::StreamSnapshot> {
    use audio_core::protocol::MIX_STREAM_ID;
    mesh.leaf
        .snapshot()
        .center
        .streams
        .into_iter()
        .find(|s| s.origin == mesh.hub_uid && s.stream_id != MIX_STREAM_ID)
}

/// Election + dial: the smaller uid listens, the leaf dials over the
/// simulated RFCOMM transport, and both report the link in their snapshot.
#[test]
fn bt_overlay_connects_leaf_to_hub() {
    let mesh = build_pair();

    wait_for(
        || mesh.leaf.snapshot().bt.links > 0,
        Duration::from_secs(20),
        "leaf to dial the hub over the simulated transport",
    );
    assert_eq!(mesh.hub.snapshot().bt.role, "hub");
    assert_eq!(mesh.hub.snapshot().bt.state, "listening");
    assert!(mesh.hub.snapshot().bt.links > 0, "hub should hold the link");
    assert_eq!(mesh.leaf.snapshot().bt.role, "leaf");

    // Dual stack: audio flows Wi-Fi + BT simultaneously, dedup on the leaf.
    wait_for(
        || stream_from(&mesh).map(|s| s.packets > 200).unwrap_or(false),
        Duration::from_secs(5),
        "hub audio to reach the leaf",
    );
}

/// The core dual-stack guarantee: cut Wi-Fi and the audio keeps flowing on
/// the Bluetooth carrier (Opus, compact header); restore Wi-Fi and both
/// carriers come back without tearing the stream down.
#[test]
fn audio_survives_wifi_cut_on_bt_carrier() {
    let mesh = build_pair();

    wait_for(
        || mesh.leaf.snapshot().bt.links > 0,
        Duration::from_secs(20),
        "BT link up",
    );
    wait_for(
        || stream_from(&mesh).map(|s| s.packets > 200).unwrap_or(false),
        Duration::from_secs(5),
        "audio flowing",
    );
    let before = stream_from(&mesh).expect("stream present");

    // Cut Wi-Fi both ways; the simulated RFCOMM link stays up.
    mesh.forward.cut();
    mesh.reverse.cut();
    std::thread::sleep(Duration::from_secs(3));

    // Per the N-1 design the hub stops relaying individual streams once the
    // leaf is Bluetooth-only: the audio continues through the *mix* stream
    // (0xFFF0, Opus, compact header) instead of the direct stream.
    let during = stream_from(&mesh).expect("direct stream entry persists during cut");
    assert!(
        during.packets - before.packets <= 40,
        "direct Wi-Fi stream should stall while cut: delta={}",
        during.packets - before.packets
    );
    wait_for(
        || mix_stream_packets(&mesh) > 100,
        Duration::from_secs(5),
        "N-1 mix to carry hub audio over BT while Wi-Fi is cut",
    );

    mesh.forward.restore();
    mesh.reverse.restore();
    std::thread::sleep(Duration::from_secs(2));
    let after = stream_from(&mesh).expect("stream present after restore");
    assert!(
        after.packets - during.packets >= 200,
        "direct stream should resume after Wi-Fi returns: delta={}",
        after.packets - during.packets
    );
}
