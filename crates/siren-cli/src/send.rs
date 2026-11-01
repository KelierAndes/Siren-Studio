use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use audio_core::capture::{CaptureKind, CapturePipe, CaptureStats, ToneSource};
use audio_core::format::{frames_for_ms, CHANNELS, SAMPLE_RATE};
use audio_core::net::discovery;
use audio_core::net::udp::UdpSender;
use audio_core::protocol::Codec;
use serde::Serialize;

use crate::util;

#[derive(clap::Args, Debug)]
pub struct SendArgs {
    /// Center address (`ip:port`) or `auto` to find one over mDNS.
    #[arg(long, default_value = "auto")]
    pub to: String,
    /// Audio source: tone, loopback (system audio) or mic.
    #[arg(long, default_value = "tone")]
    pub source: String,
    #[arg(long, default_value_t = 440.0)]
    pub freq: f64,
    #[arg(long, default_value_t = 0.25)]
    pub amplitude: f32,
    /// Payload codec: raw (f32 PCM) or opus.
    #[arg(long, default_value = "raw")]
    pub codec: String,
    /// Network frame duration in milliseconds (Opus: 5/10/20/40/60, raw: 1..60).
    #[arg(long, default_value_t = 5)]
    pub frame_ms: u16,
    #[arg(long)]
    pub duration: Option<f64>,
    /// Print the final summary as JSON on stdout.
    #[arg(long)]
    pub json: bool,
    /// Also write the final JSON summary to this file.
    #[arg(long)]
    pub summary_json: Option<String>,
}

pub fn run(args: SendArgs) -> Result<()> {
    audio_core::platform::begin_high_res_timer();
    audio_core::platform::boost_thread_priority();
    let codec = match args.codec.to_ascii_lowercase().as_str() {
        "raw" | "pcm" | "f32" => Codec::RawF32,
        "opus" => Codec::Opus,
        other => return Err(anyhow!("unknown codec '{other}' (use raw or opus)")),
    };
    let frame_ms = args.frame_ms;
    if !(1..=60).contains(&frame_ms) {
        return Err(anyhow!("--frame-ms must be between 1 and 60"));
    }
    if codec == Codec::Opus && !matches!(frame_ms, 5 | 10 | 20 | 40 | 60) {
        return Err(anyhow!(
            "Opus supports 5/10/20/40/60 ms frames (got {frame_ms} ms)"
        ));
    }
    let frames = frames_for_ms(frame_ms as f32);
    if frames == 0 {
        return Err(anyhow!("frame duration is too small"));
    }

    let target = resolve_target(&args.to)?;
    eprintln!(
        "sending {} ms {} frames to {target}",
        frame_ms,
        codec.as_str()
    );

    let origin = (audio_core::metrics::now_us() ^ ((std::process::id() as u64) << 32)).max(1);
    let mut sender = UdpSender::new(target, codec, frame_ms, origin)?;
    eprintln!("local UDP port {}", sender.local_addr()?.port());

    let mut tone = ToneSource::new(args.freq, args.amplitude);
    let mut pipe: Option<CapturePipe> = match args.source.to_ascii_lowercase().as_str() {
        "tone" | "sine" => None,
        "loopback" | "system" => {
            let pipe =
                CapturePipe::start(CaptureKind::Loopback).context("cannot capture system audio")?;
            eprintln!("source: {}", pipe.description);
            Some(pipe)
        }
        "mic" | "microphone" => {
            let pipe =
                CapturePipe::start(CaptureKind::Microphone).context("cannot open microphone")?;
            eprintln!("source: {}", pipe.description);
            Some(pipe)
        }
        other => return Err(anyhow!("unknown source '{other}' (tone, loopback, mic)")),
    };
    if pipe.is_none() {
        eprintln!(
            "source: {} Hz sine @ {:.2} amplitude",
            args.freq, args.amplitude
        );
    }

    let stop = util::stop_flag();
    let frame_duration = Duration::from_micros((frames as u64 * 1_000_000) / SAMPLE_RATE as u64);
    let started = Instant::now();
    let deadline = args
        .duration
        .map(|seconds| started + Duration::from_secs_f64(seconds));
    let mut pcm = vec![0.0f32; frames * CHANNELS];
    let mut next_tick = started;
    let mut next_report = started + Duration::from_secs(1);
    let mut frames_sent = 0u64;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let now = Instant::now();
        if deadline.is_some_and(|deadline| now >= deadline) {
            break;
        }
        if next_tick > now {
            thread::sleep(next_tick - now);
        } else if now.duration_since(next_tick) > Duration::from_millis(150) {
            next_tick = now;
        }

        match pipe.as_mut() {
            Some(pipe) => {
                pipe.read_frame(&mut pcm);
            }
            None => tone.fill(&mut pcm),
        }
        sender.send(&pcm).context("send audio frame")?;
        frames_sent += frames as u64;
        next_tick += frame_duration;

        let now = Instant::now();
        if now >= next_report {
            let elapsed = started.elapsed().as_secs_f64().max(1e-9);
            eprintln!(
                "[{:5.1}s] {} frames, {:.0} kbps, {:.1} ms/frame budget{}",
                elapsed,
                frames_sent,
                sender.bytes_sent() as f64 * 8.0 / elapsed / 1000.0,
                frame_ms,
                pipe.as_ref()
                    .map(|pipe| format!(
                        ", capture buffer {} f, dropped {}",
                        pipe.buffered_frames(),
                        pipe.stats().dropped_frames.load(Ordering::Relaxed)
                    ))
                    .unwrap_or_default()
            );
            next_report += Duration::from_secs(1);
        }
    }

    stop.store(true, Ordering::Relaxed);
    let capture_stats = pipe.as_ref().map(|pipe| pipe.stats());
    let capture_description = pipe.as_ref().map(|pipe| pipe.description.clone());
    if let Some(pipe) = pipe.take() {
        pipe.stop();
    }

    let elapsed = started.elapsed().as_secs_f64();
    let capture = capture_stats.map(|stats| capture_summary(stats, capture_description));
    let summary = SendSummary {
        event: "summary",
        target: target.to_string(),
        source: args.source.clone(),
        codec: codec.as_str().to_string(),
        frame_ms,
        seconds: round3(elapsed),
        frames: frames_sent,
        packets: sender.packets_sent(),
        bytes: sender.bytes_sent(),
        kbps: round3(sender.bytes_sent() as f64 * 8.0 / elapsed.max(1e-9) / 1000.0),
        capture,
    };

    if !args.json {
        println!(
            "summary: sent {} frame(s), {:.1} kbps over {:.1} s to {}",
            summary.frames, summary.kbps, summary.seconds, summary.target
        );
        if let Some(capture) = &summary.capture {
            println!(
                "  capture: {} frames, {} dropped, {} silent, {} read error(s)",
                capture.frames, capture.dropped_frames, capture.silent_frames, capture.read_errors
            );
        }
    }
    let json = serde_json::to_string(&summary)?;
    if args.json {
        println!("{json}");
    }
    if let Some(path) = &args.summary_json {
        std::fs::write(path, format!("{json}\n")).context("write summary JSON")?;
    }
    Ok(())
}

fn resolve_target(to: &str) -> Result<SocketAddr> {
    if to.eq_ignore_ascii_case("auto") {
        eprintln!("searching for a center over mDNS (up to 5 s)…");
        let center = discovery::find_center(Duration::from_secs(5))?
            .ok_or_else(|| anyhow!("no center found; start `siren center` or pass --to ip:port"))?;
        eprintln!(
            "found center \"{}\" at {}:{}",
            center.name, center.addr, center.port
        );
        return format!("{}:{}", center.addr, center.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow!("cannot resolve the discovered center address"));
    }
    let addresses: Vec<_> = to
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve '{to}'"))?
        .collect();
    addresses
        .iter()
        .find(|addr| addr.is_ipv4())
        .or_else(|| addresses.first())
        .copied()
        .ok_or_else(|| anyhow!("no address for '{to}' (expected ip:port)"))
}

fn capture_summary(stats: Arc<CaptureStats>, description: Option<String>) -> CaptureSummary {
    CaptureSummary {
        description: description.unwrap_or_default(),
        frames: stats.frames.load(Ordering::Relaxed),
        dropped_frames: stats.dropped_frames.load(Ordering::Relaxed),
        silent_frames: stats.silent_frames.load(Ordering::Relaxed),
        read_errors: stats.read_errors.load(Ordering::Relaxed),
    }
}

#[derive(Serialize)]
struct CaptureSummary {
    description: String,
    frames: u64,
    dropped_frames: u64,
    silent_frames: u64,
    read_errors: u64,
}

#[derive(Serialize)]
struct SendSummary {
    event: &'static str,
    target: String,
    source: String,
    codec: String,
    frame_ms: u16,
    seconds: f64,
    frames: u64,
    packets: u64,
    bytes: u64,
    kbps: f64,
    capture: Option<CaptureSummary>,
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
