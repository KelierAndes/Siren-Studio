use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use audio_core::engine::{default_node_name, SendCodec, SendSource};
use audio_core::mesh::{MeshConfig, MeshEngine};
use serde::Serialize;

use crate::util;

#[derive(clap::Args, Debug)]
pub struct MeshArgs {
    /// UDP bind address.
    #[arg(long, default_value = "0.0.0.0")]
    pub bind: String,
    #[arg(long, default_value_t = 47900)]
    pub port: u16,
    #[arg(long)]
    pub name: Option<String>,
    /// Local audio source: tone, loopback (system audio) or mic.
    #[arg(long, default_value = "tone")]
    pub source: String,
    /// Mix and play received audio locally.
    #[arg(long)]
    pub no_play: bool,
    /// Jitter-buffer target per stream, in milliseconds.
    #[arg(long, default_value_t = 30.0)]
    pub target_ms: f32,
    #[arg(long, default_value = "raw")]
    pub codec: String,
    #[arg(long, default_value_t = 5)]
    pub frame_ms: u16,
    #[arg(long, default_value_t = 440.0)]
    pub freq: f64,
    #[arg(long, default_value_t = 0.25)]
    pub amplitude: f32,
    /// Do not advertise this node over mDNS.
    #[arg(long)]
    pub no_discover: bool,
    /// Linux: isolate capture from playback with a PipeWire virtual sink
    /// (`pipewire`), or `off` for the monitor + DSP path.
    #[arg(long, value_name = "MODE")]
    pub isolate: Option<String>,
    /// Sink for local playback (default: current default sink).
    #[arg(long)]
    pub output_sink: Option<String>,
    /// Virtual sink that other applications play into (isolation mode).
    #[arg(long)]
    pub capture_sink: Option<String>,
    /// Pin peers without discovery (`ip:port`, repeatable).
    #[arg(long = "peer")]
    pub peers: Vec<String>,
    /// Seconds between mDNS scans.
    #[arg(long, default_value_t = 2)]
    pub discovery_secs: u64,
    /// F6: group password — AEAD-sealed Wi-Fi datagrams + signed election.
    #[arg(long)]
    pub password: Option<String>,
    /// Stop automatically after N seconds.
    #[arg(long)]
    pub duration: Option<f64>,
    /// Emit one snapshot JSON object per interval on stdout.
    #[arg(long)]
    pub json: bool,
    /// Append snapshot JSON lines to this file.
    #[arg(long)]
    pub stats_json: Option<String>,
    /// Record the mixed audio to this WAV file instead of playing it locally.
    #[arg(long)]
    pub record: Option<String>,
    #[arg(long, default_value_t = 1.0)]
    pub stats_interval: f64,
}

#[derive(Serialize)]
struct Summary<'a> {
    event: &'static str,
    name: &'a str,
    node_id: &'a str,
    seconds: f64,
    peers: usize,
    frames_sent: u64,
    kbps: f64,
    capture_frames: u64,
    capture_dropped_frames: u64,
}

pub fn run(args: MeshArgs) -> Result<()> {
    audio_core::platform::begin_high_res_timer();
    if args.target_ms < 5.0 || args.target_ms > 500.0 {
        return Err(anyhow!("--target-ms must be between 5 and 500"));
    }
    let source = match args.source.to_ascii_lowercase().as_str() {
        "tone" | "sine" => SendSource::Tone,
        "loopback" | "system" => SendSource::Loopback,
        "mic" | "microphone" => SendSource::Microphone,
        other => {
            return Err(anyhow!(
                "unknown source '{other}' (use tone, loopback or mic)"
            ))
        }
    };
    let codec = match args.codec.to_ascii_lowercase().as_str() {
        "raw" | "pcm" | "f32" => SendCodec::Raw,
        "opus" => SendCodec::Opus,
        "s16" => SendCodec::S16,
        other => return Err(anyhow!("unknown codec '{other}' (use raw or opus)")),
    };

    let isolation = match crate::isolate::setup(
        args.isolate.as_deref(),
        args.output_sink.as_deref(),
        args.capture_sink.as_deref(),
    ) {
        Ok(isolation) => isolation,
        Err(error) => {
            eprintln!(
                "warning: isolation unavailable: {error}; \
                 falling back to monitor capture + echo cancellation"
            );
            None
        }
    };
    let output_device = isolation
        .as_ref()
        .map(|isolation| isolation.playback_sink().to_string())
        .or_else(|| args.output_sink.clone());
    if let Some(isolation) = &isolation {
        println!(
            "isolation: capture {} → playback {}",
            isolation.capture_sink(),
            isolation.playback_sink()
        );
    }

    let name = args.name.clone().unwrap_or_else(default_node_name);
    let config = MeshConfig {
        output_device,
        bind: args.bind.clone(),
        port: args.port,
        name: name.clone(),
        advertise: !args.no_discover,
        output: !args.no_play,
        target_ms: args.target_ms,
        source,
        codec,
        frame_ms: args.frame_ms,
        freq: args.freq,
        amplitude: args.amplitude,
        discovery_secs: args.discovery_secs.max(1),
        static_peers: args.peers.clone(),
        password: args.password.clone(),
        record: args.record.clone(),
        ..MeshConfig::default()
    };

    println!(
        "siren mesh \"{name}\" on {}:{} — source {:?}, output {}, buffer {:.0} ms{}",
        args.bind,
        args.port,
        source,
        if args.no_play { "off" } else { "on" },
        args.target_ms,
        args.record
            .as_ref()
            .map(|path| format!(", recording to {path}"))
            .unwrap_or_default()
    );
    let engine = MeshEngine::start(config).context("start mesh node")?;
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
        if let Some(path) = &args.stats_json {
            util::append_json_line(path, &snapshot)?;
        }
        if args.json {
            println!("{}", serde_json::to_string(&snapshot)?);
        } else {
            let levels = snapshot
                .center
                .streams
                .iter()
                .map(|stream| format!("{:.0} dB", stream.rms_db))
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "[{:5.1}s] peers {} | rx {} [{}] | tx {:.0} kbps @ {:.0} dBFS | capture {} f, dropped {} | out {}",
                snapshot.seconds,
                snapshot.peers.len(),
                snapshot.center.streams.len(),
                levels,
                snapshot.sender.kbps,
                snapshot.sender.tx_rms_db,
                snapshot.sender.capture_frames,
                snapshot.sender.capture_dropped_frames,
                snapshot.center.master.rms_db,
            );
        }
        if let Some(error) = &snapshot.error {
            eprintln!("mesh error: {error}");
        }
    }

    let snapshot = engine.snapshot();
    engine.wait_until_stopped();
    let summary = Summary {
        event: "summary",
        name: &snapshot.name,
        node_id: &snapshot.node_id,
        seconds: snapshot.seconds,
        peers: snapshot.peers.len(),
        frames_sent: snapshot.sender.frames,
        kbps: snapshot.sender.kbps,
        capture_frames: snapshot.sender.capture_frames,
        capture_dropped_frames: snapshot.sender.capture_dropped_frames,
    };
    if args.json {
        println!("{}", serde_json::to_string(&summary)?);
    } else {
        println!(
            "summary: {} peer(s), sent {} frames ({:.0} kbps), capture {} frames over {:.1} s",
            summary.peers,
            summary.frames_sent,
            summary.kbps,
            summary.capture_frames,
            summary.seconds
        );
    }
    Ok(())
}
