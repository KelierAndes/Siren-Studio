use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use audio_core::bt::{Tunnel, TunnelConfig};
use audio_core::engine::{default_node_name, SendCodec, SendSource};
use audio_core::mesh::{MeshConfig, MeshEngine};

use crate::util;

#[derive(clap::Args, Debug)]
pub struct BtArgs {
    #[arg(long)]
    pub name: Option<String>,
    /// Origin bit (0..63); every node in the chain needs a distinct bit.
    #[arg(long, default_value_t = 0)]
    pub bit: u8,
    /// Local mesh UDP port (bound on 127.0.0.1).
    #[arg(long, default_value_t = 48000)]
    pub port: u16,
    /// TCP port that accepts one Bluetooth-link bridge (repeatable, hub side).
    #[arg(long = "listen-link")]
    pub listen_links: Vec<u16>,
    /// TCP bridge address to connect to (`ip:port`, repeatable, leaf side).
    #[arg(long = "link")]
    pub links: Vec<String>,
    /// Local source: tone, loopback or mic.
    #[arg(long, default_value = "tone")]
    pub source: String,
    #[arg(long, default_value_t = 440.0)]
    pub freq: f64,
    #[arg(long, default_value_t = 0.25)]
    pub amplitude: f32,
    #[arg(long)]
    pub no_play: bool,
    #[arg(long, default_value_t = 5)]
    pub ttl: u8,
    /// Jitter-buffer target per stream (Bluetooth needs more than LAN).
    #[arg(long, default_value_t = 150.0)]
    pub target_ms: f32,
    /// Opus bitrate for Bluetooth links (RFCOMM cannot carry the LAN default).
    #[arg(long, default_value_t = 64_000)]
    pub opus_bitrate: i32,
    #[arg(long)]
    pub duration: Option<f64>,
    #[arg(long, default_value_t = 1.0)]
    pub stats_interval: f64,
}

pub fn run(args: BtArgs) -> Result<()> {
    audio_core::platform::begin_high_res_timer();
    if args.bit >= 64 {
        return Err(anyhow!("--bit must be between 0 and 63"));
    }
    if args.port == 0 {
        return Err(anyhow!("--port must not be zero"));
    }
    if !(5.0..=500.0).contains(&args.target_ms) {
        return Err(anyhow!("--target-ms must be between 5 and 500"));
    }
    if !(6_000..=510_000).contains(&args.opus_bitrate) {
        return Err(anyhow!("--opus-bitrate must be between 6000 and 510000"));
    }
    std::env::set_var("SIREN_OPUS_BITRATE", args.opus_bitrate.to_string());
    if args.listen_links.is_empty() && args.links.is_empty() {
        return Err(anyhow!(
            "no Bluetooth links: pass --listen-link (hub) or --link (leaf)"
        ));
    }
    let source = match args.source.to_ascii_lowercase().as_str() {
        "tone" | "sine" => SendSource::Tone,
        "loopback" | "system" => SendSource::Loopback,
        "mic" | "microphone" => SendSource::Microphone,
        other => return Err(anyhow!("unknown source '{other}' (tone, loopback, mic)")),
    };

    let mut streams = Vec::new();
    for port in &args.listen_links {
        let listener = TcpListener::bind(("127.0.0.1", *port))
            .with_context(|| format!("bind Bluetooth bridge port {port}"))?;
        listener.set_nonblocking(true)?;
        println!("bt: waiting for a bridge on 127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    stream.set_nodelay(true).ok();
                    println!("bt: bridge connected on port {port}");
                    streams.push(stream);
                    break;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(anyhow!("timed out waiting for bridge on port {port}"));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    for address in &args.links {
        println!("bt: connecting bridge {address}");
        let deadline = Instant::now() + Duration::from_secs(25);
        let stream = loop {
            match TcpStream::connect(address) {
                Ok(stream) => break stream,
                Err(error) => {
                    if Instant::now() >= deadline {
                        return Err(error)
                            .with_context(|| format!("connect Bluetooth bridge {address}"));
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        };
        stream.set_nodelay(true).ok();
        streams.push(stream);
    }

    let mesh_bind: std::net::SocketAddr = format!("127.0.0.1:{}", args.port).parse()?;
    let tunnel = Tunnel::start(TunnelConfig {
        mesh_bind,
        node_bit: args.bit,
        node_uid: 0,
        ttl: args.ttl,
        links: streams.into_iter().map(|stream| Box::new(stream) as Box<dyn audio_core::bt::Link>).collect(),
    })
    .context("start Bluetooth tunnel")?;

    let name = args.name.clone().unwrap_or_else(default_node_name);
    let config = MeshConfig {
        bind: "127.0.0.1".into(),
        port: args.port,
        name: name.clone(),
        advertise: false,
        discovery: false,
        output: !args.no_play,
        target_ms: args.target_ms,
        source,
        codec: SendCodec::Opus,
        frame_ms: 10,
        freq: args.freq,
        amplitude: args.amplitude,
        static_peers: vec![tunnel.local_addr().to_string()],
        ..MeshConfig::default()
    };
    println!(
        "bt: node \"{name}\" bit={} mesh=127.0.0.1:{} tunnel={} links={}",
        args.bit,
        args.port,
        tunnel.local_addr(),
        args.listen_links.len() + args.links.len()
    );
    let engine = MeshEngine::start(config).context("start Bluetooth mesh node")?;
    tunnel.set_node_uid(engine.uid());

    let stop = util::stop_flag();
    let started = Instant::now();
    let deadline = args
        .duration
        .map(|seconds| started + Duration::from_secs_f64(seconds));
    let interval = Duration::from_secs_f64(args.stats_interval.max(0.2));
    let mut next_report = started + interval;
    loop {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        if Instant::now() < next_report {
            continue;
        }
        next_report += interval;
        let snapshot = engine.snapshot();
        let streams = snapshot
            .center
            .streams
            .iter()
            .map(|stream| format!("{} {:.1} dB", stream.addr, stream.rms_db))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "[{:5.1}s] bit {} | streams {} [{}] | master {:.1} dB",
            snapshot.seconds,
            args.bit,
            snapshot.center.streams.len(),
            streams,
            snapshot.center.master.rms_db
        );
        if let Some(error) = &snapshot.error {
            eprintln!("bt error: {error}");
        }
    }

    engine.stop();
    drop(tunnel);
    println!("bt: stopped");
    Ok(())
}
