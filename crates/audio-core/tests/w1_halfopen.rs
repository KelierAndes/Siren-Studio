//! W1 acceptance: a half-open RFCOMM link must be judged dead by the leaf.
//!
//! The real incident (E3): the hub's Bluetooth side stops answering but the
//! TCP-style socket stays open — the leaf's writes keep "succeeding" into
//! buffers (7.8 KB/s measured), the 6 s byte-window watchdog sees outbound
//! traffic and stays quiet, and the leaf holds a dead link forever.
//!
//! The fix under test is the tunnel keepalive (`bt::Liveness`): after 2 s of
//! inbound silence the leaf probes with `SKP1`; a probe left unanswered for
//! 3 s resets the link and the redial loop takes over. The hub answers pings
//! unconditionally, so only the *leaf* needs the keepalive flag — which a
//! statically pinned topology grants through the `?proto=4` pin hint.
//!
//! The sim reproduces the half-open exactly: a TCP proxy sits between leaf
//! and hub. Leaf→hub always forwards (writes keep succeeding); hub→leaf
//! forwards until the test flips the switch, then reads-and-discards — the
//! hub's replies die in the proxy while its TCP stack keeps ACKing, which is
//! byte-for-byte what a blocked RFCOMM peer looks like.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use audio_core::hybrid::{HybridConfig, HybridNode, HybridOptions};
use audio_core::mesh::MeshConfig;
use audio_core::engine::SendSource;
use audio_core::Result;
use audio_core::net::{spawn_link, LinkHandle};
use audio_core::rfcomm::{BtListener, BtTransport};

const HUB_ID: &str = "aaaaaaaaaaaa0001";
const LEAF_ID: &str = "aaaaaaaaaaaa0002";

/// Simulated Bluetooth adapter over loopback TCP.
struct SimBt {
    listener: TcpListener,
    /// Where `connect` actually dials: normally the address given, but the
    /// leaf dials the *proxy*, which is what makes it the half-open victim.
    redirect: Option<u16>,
}

impl SimBt {
    fn new() -> std::io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind("127.0.0.1:0")?,
            redirect: None,
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
        let port: u16 = match (&self.redirect, address.strip_prefix("sim-")) {
            (Some(proxy), Some(_)) => *proxy,
            _ => address
                .strip_prefix("sim-")
                .and_then(|p| p.parse().ok())
                .ok_or_else(|| {
                    audio_core::CoreError::Backend(format!("bad sim address {address}"))
                })?,
        };
        TcpStream::connect(("127.0.0.1", port))
            .map_err(|e| audio_core::CoreError::Backend(e.to_string()))
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

/// One accepted leaf connection: pump leaf→hub forever; pump hub→leaf until
/// `cut_downlink` flips, then swallow the bytes (TCP stays healthy — the
/// half-open).
fn serve_halfopen(
    leaf_stream: TcpStream,
    hub_stream: TcpStream,
    cut_downlink: Arc<AtomicBool>,
) {
    let mut leaf_rx = leaf_stream.try_clone().expect("clone leaf rx");
    let mut hub_tx = hub_stream.try_clone().expect("clone hub tx");
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut leaf_rx, &mut hub_tx);
    });

    let mut hub_rx = hub_stream;
    let mut leaf_tx = leaf_stream;
    let mut buf = [0u8; 4096];
    loop {
        match hub_rx.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if !cut_downlink.load(Ordering::Relaxed) {
                    if leaf_tx.write_all(&buf[..n]).is_err() {
                        return;
                    }
                }
                // else: read and drop — the proxy's socket stays open and
                // ACKs, so the hub sees a healthy peer that never answers.
            }
        }
    }
}

fn run_proxy(proxy: TcpListener, hub_addr: std::net::SocketAddr, cut_downlink: Arc<AtomicBool>) {
    for stream in proxy.incoming() {
        let Ok(leaf_stream) = stream else { break };
        let Ok(hub_stream) = TcpStream::connect(hub_addr) else {
            break;
        };
        let cut = cut_downlink.clone();
        std::thread::spawn(move || serve_halfopen(leaf_stream, hub_stream, cut));
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
    #[allow(dead_code)] // held so the link threads stay alive
    forward: LinkHandle,
    #[allow(dead_code)]
    reverse: LinkHandle,
}

fn build_pair(proxy_port: u16, hub_bt: &SimBt) -> TestMesh {
    let (hub_port, leaf_port, fwd_port, rev_port) = (
        reserve_port(),
        reserve_port(),
        reserve_port(),
        reserve_port(),
    );
    let leaf_bt = SimBt {
        redirect: Some(proxy_port),
        ..SimBt::new().expect("sim bt leaf")
    };

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
            mesh_config.bt_exempt_ports = vec![fwd_port, rev_port];
            HybridNode::start(
                mesh_config,
                HybridOptions {
                    config: HybridConfig::default(),
                    transport: Some(std::sync::Arc::new(SimBt {
                        listener: bt.listener.try_clone().expect("clone listener"),
                        redirect: bt.redirect,
                    })),
                    state_path: Some(std::env::temp_dir().join(format!("siren-w1-{name}"))),
                    ..HybridOptions::default()
                },
            )
            .expect("start node")
        };

    // The leaf's pin carries `proto=4` (BT_KEEPALIVE_PROTO): standing in for
    // the mDNS TXT record that flips the keepalive on for this hub.
    let hub = node(
        "w1-hub",
        HUB_ID,
        hub_port,
        vec![format!("127.0.0.1:{fwd_port}?id={LEAF_ID}&name=leaf")],
        &hub_bt,
    );
    let leaf = node(
        "w1-leaf",
        LEAF_ID,
        leaf_port,
        vec![format!(
            "127.0.0.1:{rev_port}?id={HUB_ID}&bt={}&proto=4&name=hub",
            hub_bt.address()
        )],
        &leaf_bt,
    );
    TestMesh {
        hub,
        leaf,
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
fn leaf_judges_half_open_link_dead_and_redials() {
    // One shared hub adapter: the node listens on it AND the proxy dials it.
    let hub_bt = SimBt::new().expect("hub bt");
    let hub_listener_addr = hub_bt.listener.local_addr().expect("hub bt addr");
    let proxy = TcpListener::bind("127.0.0.1:0").expect("proxy bind");
    let proxy_port = proxy.local_addr().unwrap().port();
    let cut_downlink = Arc::new(AtomicBool::new(false));
    {
        let cut = cut_downlink.clone();
        std::thread::spawn(move || run_proxy(proxy, hub_listener_addr, cut));
    }

    let mesh = build_pair(proxy_port, &hub_bt);

    // Link up and audio flowing. Note: ka_rt stays 0 while healthy — the
    // dual-stack BT downlink is never silent for 2 s, so no probe is sent.
    // The keepalive only wakes up when the downlink goes quiet, which is
    // exactly the half-open case below.
    wait_for(
        || mesh.leaf.snapshot().bt.links > 0,
        Duration::from_secs(20),
        "leaf to dial the hub through the proxy",
    );
    wait_for(
        || mesh.hub.snapshot().bt.links > 0,
        Duration::from_secs(10),
        "hub to accept the leaf link",
    );

    // Half-open the downlink: the hub's replies die in the proxy, but the
    // hub's TCP keeps ACKing the leaf's writes — the byte-window watchdog
    // must NOT be what saves us here (the leaf's BT uplink keeps the
    // outbound window above 4 KB, which is the W1 blind spot this test
    // exercises).
    cut_downlink.store(true, Ordering::Relaxed);

    // KEEPALIVE_PING_AFTER(2 s) + KEEPALIVE_DEAD_AFTER(3 s) + loop cadence.
    wait_for(
        || mesh.leaf.snapshot().bt.ka_dead > 0,
        Duration::from_secs(15),
        "the leaf to judge the unanswered probe a dead link",
    );

    // The dead link is torn down and the redial loop takes over: within a
    // short window the leaf either holds 0 links (dialing) or a fresh one.
    let saw_gap = (0..50).any(|_| {
        let links = mesh.leaf.snapshot().bt.links;
        if links == 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
        false
    });
    assert!(
        saw_gap,
        "leaf must tear down the half-open link before redialing"
    );

    // Redial succeeds through the same proxy (a new half-open forms, but the
    // point is the recovery loop ran and re-established a link).
    wait_for(
        || mesh.leaf.snapshot().bt.links > 0,
        Duration::from_secs(20),
        "the redial loop to re-establish a link",
    );
    // And the cycle is self-sustaining: the fresh link goes half-open too,
    // and the leaf kills it again without any external help.
    let ka_dead_before = mesh.leaf.snapshot().bt.ka_dead;
    wait_for(
        || mesh.leaf.snapshot().bt.ka_dead > ka_dead_before,
        Duration::from_secs(20),
        "the keepalive to catch the second half-open link as well",
    );
}
