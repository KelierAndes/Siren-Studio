//! Deterministic network-fault scenarios driving two real mesh nodes over
//! in-memory lossy links (F19 / PLAN P0-1).
//!
//! Two `HybridNode`s run on loopback (Bluetooth disabled, mDNS off) with
//! pinned static peers pointing at [`audio_core::net::spawn_link`] proxies
//! instead of at each other. The proxies forward every datagram to the real
//! node port under a programmable profile (delay / jitter / loss / hard
//! partition), swapped at runtime, so one node pair serves all scenarios and
//! every number is reproducible without privileges or a real network.

use std::time::Duration;

use audio_core::engine::{SendCodec, SendSource};
use audio_core::hybrid::{HybridConfig, HybridNode, HybridOptions};
use audio_core::mesh::MeshConfig;
use audio_core::net::{spawn_link, LinkHandle, LinkProfile};
use audio_core::protocol::MIX_STREAM_ID;

/// 5 ms raw frames: the receiver counts `lost_frames` in *sample frames*
/// (gap packets × 240), while `packets` counts datagrams.
const SAMPLE_FRAMES_PER_PACKET: f64 = 240.0;

/// A node pair plus the handles gating both directions of their link.
struct TestMesh {
    sender: HybridNode,
    receiver: HybridNode,
    sender_uid: u64,
    /// sender -> receiver direction.
    forward: LinkHandle,
    /// receiver -> sender direction.
    reverse: LinkHandle,
}

/// Grab a free loopback port. There is a small bind-then-release race, but
/// for tests it is negligible; a stolen port fails loudly at start.
fn reserve_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .expect("reserve port")
        .local_addr()
        .expect("port")
        .port()
}

/// Start the pair. Both nodes send the default 440 Hz tone; neither touches
/// an audio device (`output: false` -> `CenterMode::Silent`).
fn build_pair() -> TestMesh {
    build_pair_full(SendCodec::Raw, None)
}

/// `password` sets the group password on both nodes (F6). Pass different
/// strings to build a mismatched pair.
fn build_pair_full(codec: SendCodec, password: Option<&str>) -> TestMesh {
    build_pair_inner(codec, password)
}

/// `codec` selects the Wi-Fi payload codec; S16 must deliver loss-free at
/// roughly half the raw bandwidth (988 B frames fit one MTU).
fn build_pair_with_codec(codec: SendCodec) -> TestMesh {
    build_pair_inner(codec, None)
}

fn build_pair_inner(codec: SendCodec, password: Option<&str>) -> TestMesh {
    let (rx_port, tx_port, fwd_port, rev_port) = (
        reserve_port(),
        reserve_port(),
        reserve_port(),
        reserve_port(),
    );

    let node = |name: &str, port: u16, peers: Vec<String>| {
        let mut mesh_config = MeshConfig::default();
        mesh_config.bind = "127.0.0.1".into();
        mesh_config.port = port;
        mesh_config.name = name.into();
        mesh_config.discovery = false;
        mesh_config.advertise = false;
        mesh_config.output = false;
        mesh_config.source = SendSource::Tone;
        mesh_config.codec = codec;
        mesh_config.frame_ms = 5;
        // node ids must be >=16 hex chars to be accepted (otherwise both
        // nodes fall back to the same machine-persisted id and collide).
        mesh_config.node_id = Some(match name {
            "sim-rx" => "aaaaaaaaaaaa0001".into(),
            _ => "aaaaaaaaaaaa0002".into(),
        });
        mesh_config.static_peers = peers;
        // Our Wi-Fi fronts are loopback ports; without this the receiver
        // would classify their packets as the Bluetooth carrier.
        mesh_config.bt_exempt_ports = vec![fwd_port, rev_port];
        mesh_config.password = password.map(str::to_string);
        HybridNode::start(
            mesh_config,
            HybridOptions {
                config: HybridConfig {
                    bt_enabled: false,
                    ..HybridConfig::default()
                },
                // Keep test state out of the user's real config directory.
                state_path: Some(std::env::temp_dir().join(format!("siren-link-sim-{name}"))),
                ..HybridOptions::default()
            },
        )
        .expect("start node")
    };

    // Links must exist before the nodes: the nodes' static peers point at
    // the link fronts.
    let forward = spawn_link(
        &format!("127.0.0.1:{fwd_port}"),
        format!("127.0.0.1:{rx_port}").parse().expect("addr"),
        LinkProfile::default(),
    )
    .expect("forward link")
    .1;
    let reverse = spawn_link(
        &format!("127.0.0.1:{rev_port}"),
        format!("127.0.0.1:{tx_port}").parse().expect("addr"),
        LinkProfile::default(),
    )
    .expect("reverse link")
    .1;

    let receiver = node("sim-rx", rx_port, vec![format!("127.0.0.1:{rev_port}")]);
    let sender = node("sim-tx", tx_port, vec![format!("127.0.0.1:{fwd_port}")]);
    let sender_uid = sender.mesh().uid();
    TestMesh {
        sender,
        receiver,
        sender_uid,
        forward,
        reverse,
    }
}

impl Drop for TestMesh {
    fn drop(&mut self) {
        self.sender.stop();
        self.receiver.stop();
    }
}

/// The receiver's snapshot of the sender's audio stream, if present.
fn rx_stream(mesh: &TestMesh) -> Option<audio_core::metrics::StreamSnapshot> {
    mesh.receiver
        .snapshot()
        .center
        .streams
        .into_iter()
        .find(|s| s.origin == mesh.sender_uid && s.stream_id != MIX_STREAM_ID)
}

/// Cumulative `lost_frames` (sample frames) converted to lost datagrams.
fn lost_packets(snapshot: &audio_core::metrics::StreamSnapshot) -> f64 {
    snapshot.lost_frames as f64 / SAMPLE_FRAMES_PER_PACKET
}

/// Loss ratio over a window, given before/after snapshots.
fn window_loss_ratio(before: &audio_core::metrics::StreamSnapshot, after: &audio_core::metrics::StreamSnapshot) -> f64 {
    let lost = lost_packets(after) - lost_packets(before);
    let delivered = (after.packets - before.packets) as f64;
    lost / (delivered + lost).max(1.0)
}

fn wait_secs(secs: u64) {
    std::thread::sleep(Duration::from_secs(secs));
}

/// Scenario 1 — healthy link: the stream comes up with negligible loss.
#[test]
fn stable_link_delivers_audio() {
    let mesh = build_pair();
    wait_secs(2);

    let stream = rx_stream(&mesh).expect("sender stream visible at receiver");
    assert!(
        stream.packets >= 300,
        "expected ~400 packets in 2 s (5 ms frames), got {}",
        stream.packets
    );
    let lost = lost_packets(&stream);
    let loss_ratio = lost / (stream.packets as f64 + lost).max(1.0);
    assert!(
        loss_ratio < 0.02,
        "stable link should not lose audio: lost={lost:.1} packets={} ratio={loss_ratio:.3}",
        stream.packets
    );
    assert!(stream.active, "stream should be active on a healthy link");
}

/// Scenario 2 — sustained 30% loss: losses show up in the counter, roughly
/// matching the profile.
#[test]
fn loss_profile_shows_in_counters() {
    let mesh = build_pair();
    wait_secs(1); // come up healthy first

    let before = rx_stream(&mesh).expect("stream up before fault");
    mesh.forward.set_profile(LinkProfile {
        loss_percent: 30.0,
        ..LinkProfile::default()
    });
    wait_secs(3); // 600 frames

    let after = rx_stream(&mesh).expect("stream still present under loss");
    let ratio = window_loss_ratio(&before, &after);
    assert!(
        (0.15..0.45).contains(&ratio),
        "30% loss should land near 30%: lost={:.1} packets delta={} ratio={ratio:.3}",
        lost_packets(&after) - lost_packets(&before),
        after.packets - before.packets
    );
}

/// Scenario 3 — 250 ms one-way latency with small jitter: the jitter buffer
/// absorbs it and the stream stays active with no meaningful loss. (Jitter
/// stays within the 30 ms target buffer; larger spreads reorder packets and
/// are covered by the next scenario.)
#[test]
fn high_latency_is_absorbed_not_lost() {
    let mesh = build_pair();
    wait_secs(1);

    mesh.forward.set_profile(LinkProfile {
        base_delay: Duration::from_millis(250),
        jitter: Duration::from_millis(5),
        ..LinkProfile::default()
    });
    wait_secs(3);

    let stream = rx_stream(&mesh).expect("stream survives high latency");
    assert!(
        stream.active,
        "250 ms latency must not tear the stream down (timeout is 3 s)"
    );
    let lost = lost_packets(&stream);
    let loss_ratio = lost / (stream.packets as f64 + lost).max(1.0);
    assert!(
        loss_ratio < 0.05,
        "latency alone should not lose audio: lost={lost:.1} ratio={loss_ratio:.3}"
    );
}

/// Scenario 3b — jitter wider than the buffer reorders datagrams: the
/// receiver has no reorder window, so a late packet counts as a gap (lost +
/// zero-fill) and the out-of-order original as a duplicate. This test pins
/// that current behaviour as the baseline for any future reorder tolerance.
#[test]
fn reorder_beyond_buffer_counts_as_loss() {
    let mesh = build_pair();
    wait_secs(1);

    mesh.forward.set_profile(LinkProfile {
        base_delay: Duration::from_millis(250),
        jitter: Duration::from_millis(60),
        ..LinkProfile::default()
    });
    wait_secs(3);

    let stream = rx_stream(&mesh).expect("stream survives heavy jitter");
    let lost = lost_packets(&stream);
    let loss_ratio = lost / (stream.packets as f64 + lost).max(1.0);
    assert!(
        loss_ratio > 0.20,
        "jitter beyond the buffer depth should show up as loss: lost={lost:.1} ratio={loss_ratio:.3}"
    );
}

/// Scenario 5 — Gilbert-style burst loss: drops cluster (a drop drags the
/// next burst-1 packets with it). The link's `max_drop_run` proves the
/// clustering; the receiver sees far worse audio than the same average loss
/// spread evenly would cause.
#[test]
fn burst_loss_clusters_drops() {
    let mesh = build_pair();
    wait_secs(1);

    mesh.forward.set_profile(LinkProfile {
        loss_percent: 20.0,
        burst_len: 5,
        ..LinkProfile::default()
    });
    wait_secs(3);

    let stats = mesh.forward.stats();
    assert!(
        stats.max_drop_run >= 5,
        "burst loss should produce runs of >=5 consecutive drops: max_run={} drops={}",
        stats.max_drop_run,
        stats.dropped_loss
    );

    let after = rx_stream(&mesh).expect("stream still present under burst loss");
    let lost = lost_packets(&after);
    let loss_ratio = lost / (after.packets as f64 + lost).max(1.0);
    assert!(
        loss_ratio > 0.30,
        "20% trigger with bursts of 5 should land well above 20% average: lost={lost:.1} ratio={loss_ratio:.3}"
    );
}

/// Scenario 6 — bandwidth cap below the raw codec's appetite: raw f32
/// 5 ms frames need ~3.1 Mbps (1948 B × 200/s); capping to 1800 kbps must
/// drop the excess on the link and show up as receiver loss.
#[test]
fn bandwidth_cap_drops_excess() {
    let mesh = build_pair();
    wait_secs(1);

    mesh.forward.set_profile(LinkProfile {
        bandwidth_kbps: Some(1800.0),
        ..LinkProfile::default()
    });
    wait_secs(3);

    let stats = mesh.forward.stats();
    assert!(
        stats.dropped_bandwidth > 0,
        "1800 kbps cap should drop part of the ~3100 kbps raw stream"
    );

    let after = rx_stream(&mesh).expect("stream still present under bandwidth cap");
    let lost = lost_packets(&after);
    let loss_ratio = lost / (after.packets as f64 + lost).max(1.0);
    assert!(
        (0.2..0.65).contains(&loss_ratio),
        "bandwidth cap should lose roughly the excess fraction: lost={lost:.1} ratio={loss_ratio:.3}"
    );
}
/// Scenario 4 — hard partition, then recovery: packets stop during the cut
/// and the whole missing span lands in `lost_frames` when the first packet
/// after restore arrives (the receiver only sees the gap then).
#[test]
fn partition_and_recovery() {
    let mesh = build_pair();
    wait_secs(1);

    let healthy = rx_stream(&mesh).expect("stream up before partition");
    mesh.forward.cut();
    wait_secs(2);

    // No packets arrive during a full cut, so the loss counter can only move
    // once audio resumes; here we assert delivery actually stopped.
    let cut = rx_stream(&mesh).expect("stream entry persists across a 2 s partition");
    let packets_during_cut = cut.packets.saturating_sub(healthy.packets);
    assert!(
        packets_during_cut <= 5,
        "partition should stop delivery: packets_during_cut={packets_during_cut}"
    );

    mesh.forward.restore();
    // After restore: audio resumes within ~1.5 s and the receiver accounts
    // the whole cut as lost frames (gap accounting, udp.rs).
    wait_secs(2);
    let recovered = rx_stream(&mesh).expect("stream present after recovery");
    let resumed = recovered.packets - cut.packets;
    let lost_for_cut = lost_packets(&recovered) - lost_packets(&cut);
    assert!(
        resumed >= 200,
        "audio should resume after the link is restored: delta={resumed}"
    );
    assert!(
        lost_for_cut >= 300.0,
        "the receiver should account the cut span as loss: lost_for_cut={lost_for_cut:.1}"
    );
}

/// Scenario 7 — s16 payload (F1): same 5 ms timeline at half the wire size;
/// the stream is loss-free and the sender's bandwidth matches ~988 B frames
/// (988 x 200 pps x 8 bit = ~1580 kbps) instead of raw's ~3100 kbps.
#[test]
fn s16_payload_halves_bandwidth_and_delivers() {
    let mesh = build_pair_with_codec(SendCodec::S16);
    wait_secs(2);

    let kbps = mesh.sender.snapshot().sender.kbps;
    assert!(
        (1200.0..2000.0).contains(&kbps),
        "s16 should send ~1580 kbps, got {kbps}"
    );

    let stream = rx_stream(&mesh).expect("s16 stream visible at receiver");
    assert!(
        stream.packets >= 300,
        "expected ~400 packets in 2 s, got {}",
        stream.packets
    );
    let lost = lost_packets(&stream);
    let loss_ratio = lost / (stream.packets as f64 + lost).max(1.0);
    assert!(
        loss_ratio < 0.02 && stream.codec == "s16",
        "s16 stream should be loss-free: codec={} lost={lost:.1} ratio={loss_ratio:.3}",
        stream.codec
    );
    assert!(stream.active);
}

/// F6 acceptance: matching passwords deliver audio.
#[test]
fn group_password_gates_the_mesh() {
    let mesh = build_pair_full(SendCodec::Raw, Some("group-pass"));
    wait_secs(2);

    let stream = rx_stream(&mesh).expect("same password delivers audio");
    assert!(stream.packets >= 300, "packets={} with matching password", stream.packets);
}

#[test]
fn wrong_password_delivers_nothing() {
    // Two nodes, different passwords: the receiver must reject every
    // sealed datagram — no stream ever appears.
    let (rx_port, tx_port, fwd_port, rev_port) = (
        reserve_port(),
        reserve_port(),
        reserve_port(),
        reserve_port(),
    );
    let forward = spawn_link(
        &format!("127.0.0.1:{fwd_port}"),
        format!("127.0.0.1:{rx_port}").parse().expect("addr"),
        Default::default(),
    )
    .expect("forward link")
    .1;
    let _reverse = spawn_link(
        &format!("127.0.0.1:{rev_port}"),
        format!("127.0.0.1:{tx_port}").parse().expect("addr"),
        Default::default(),
    )
    .expect("reverse link")
    .1;
    let node = |name: &str, id: &str, port: u16, peers: Vec<String>, password: &str| {
        let mut mesh_config = MeshConfig::default();
        mesh_config.bind = "127.0.0.1".into();
        mesh_config.port = port;
        mesh_config.name = name.into();
        mesh_config.discovery = false;
        mesh_config.advertise = false;
        mesh_config.output = false;
        mesh_config.source = SendSource::Tone;
        mesh_config.codec = SendCodec::Raw;
        mesh_config.frame_ms = 5;
        mesh_config.node_id = Some(id.to_string());
        mesh_config.static_peers = peers;
        mesh_config.bt_exempt_ports = vec![fwd_port, rev_port];
        mesh_config.password = Some(password.to_string());
        HybridNode::start(
            mesh_config,
            HybridOptions {
                config: HybridConfig {
                    bt_enabled: false,
                    ..HybridConfig::default()
                },
                state_path: Some(std::env::temp_dir().join(format!("siren-link-sim-{name}"))),
                ..HybridOptions::default()
            },
        )
        .expect("start node")
    };
    let receiver = node(
        "sim-rx2",
        "aaaaaaaaaaaa0001",
        rx_port,
        vec![format!("127.0.0.1:{rev_port}")],
        "password-rx",
    );
    let sender = node(
        "sim-tx2",
        "aaaaaaaaaaaa0002",
        tx_port,
        vec![format!("127.0.0.1:{fwd_port}")],
        "password-tx",
    );
    wait_secs(2);
    let stream = receiver
        .snapshot()
        .center
        .streams
        .into_iter()
        .find(|s| s.origin == sender.mesh().uid());
    assert!(
        stream.is_none() || stream.unwrap().packets == 0,
        "a wrong password must deliver no audio"
    );
    receiver.stop();
    sender.stop();
}
