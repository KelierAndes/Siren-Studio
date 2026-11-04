use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use audio_core::dsp::{new_mixer, Mixer, MixerControl};
use audio_core::format::{frames_for_ms, CHANNELS};
use audio_core::metrics::{MasterSnapshot, MetricsRegistry, MixerMetrics, StreamSnapshot};
use audio_core::net::discovery::Advertiser;
use audio_core::net::udp::{ReceiverConfig, ReceiverStatsSnapshot};
use audio_core::output::{record_to_wav, start_playback, Playback, RecordSummary};
use serde::Serialize;

use crate::util;

#[derive(clap::Args, Debug)]
pub struct CenterArgs {
    /// UDP bind address.
    #[arg(long, default_value = "0.0.0.0")]
    pub bind: String,
    #[arg(long, default_value_t = 47900)]
    pub port: u16,
    /// Jitter-buffer target per stream, in milliseconds.
    #[arg(long, default_value_t = 30.0)]
    pub target_ms: f32,
    /// F6: group password (AEAD-sealed datagrams).
    #[arg(long)]
    pub password: Option<String>,
    #[arg(long, default_value_t = 32)]
    pub max_streams: usize,
    /// Record the mix to a 32-bit float WAV instead of playing it.
    #[arg(long)]
    pub record: Option<String>,
    /// Playback device name substring (default: system default).
    #[arg(long)]
    pub device: Option<String>,
    /// Mix without opening an output device (clock is software-timed).
    #[arg(long)]
    pub no_play: bool,
    /// Stop automatically after N seconds.
    #[arg(long)]
    pub duration: Option<f64>,
    /// Name advertised over mDNS.
    #[arg(long)]
    pub name: Option<String>,
    /// Do not advertise this center over mDNS.
    #[arg(long)]
    pub no_discover: bool,
    /// Append JSON stats (one object per line + a final summary) to this file.
    #[arg(long)]
    pub stats_json: Option<String>,
    #[arg(long, default_value_t = 1.0)]
    pub stats_interval: f64,
}

enum Sink {
    Record(JoinHandle<Result<RecordSummary, audio_core::CoreError>>),
    Silent(JoinHandle<()>),
    Playback(Playback),
}

pub fn run(args: CenterArgs) -> Result<()> {
    audio_core::platform::begin_high_res_timer();
    let bind_ip: IpAddr = args
        .bind
        .parse()
        .with_context(|| format!("invalid bind address '{}'", args.bind))?;
    let bind = SocketAddr::new(bind_ip, args.port);
    if args.target_ms < 5.0 || args.target_ms > 500.0 {
        return Err(anyhow!("--target-ms must be between 5 and 500"));
    }

    let (mixer, mixer_handle) = new_mixer(args.max_streams);
    let master = mixer_handle.metrics();
    let mixer_control = MixerControl::new(mixer_handle);
    let receiver = audio_core::net::udp::spawn(
        ReceiverConfig {
            bind,
            target_ms: args.target_ms,
            max_streams: args.max_streams,
            live_target_ms: None,
            mix_tap: None,
            bt_exempt_ports: Vec::new(),
            key: None,
            gossip: None,
        },
        mixer_control,
    )?;
    let local_addr = receiver.local_addr;
    let registry = receiver.registry.clone();
    let receiver_stats = receiver.stats.clone();

    let name = args.name.clone().unwrap_or_else(util::default_node_name);
    let _advertiser: Option<Advertiser> = if args.no_discover {
        None
    } else {
        match audio_core::net::discovery::advertise("center", &name, local_addr.port(), "raw,opus", "")
        {
            Ok(advertiser) => {
                println!("discovery: advertising as \"{name} (center)\"");
                Some(advertiser)
            }
            Err(error) => {
                eprintln!("warning: mDNS advertising failed: {error}");
                None
            }
        }
    };

    println!(
        "siren center \"{name}\" listening on {local_addr} (UDP), target buffer {:.0} ms",
        args.target_ms
    );

    let stop = util::stop_flag();
    let sink = start_sink(&args, mixer, &stop)?;
    let stats_stop = stop.clone();
    let stats_thread = spawn_stats(
        registry.clone(),
        master.clone(),
        stats_stop,
        args.stats_interval.max(0.2),
        args.stats_json.clone(),
    );
    let started = Instant::now();

    util::wait(&stop, args.duration);
    stop.store(true, Ordering::Relaxed);

    let record_summary = stop_sink(sink)?;
    let _ = stats_thread.join();

    let streams = registry.snapshots();
    let receiver_snapshot = receiver_stats.snapshot();
    receiver.stop();

    let elapsed = started.elapsed().as_secs_f64();
    let summary = CenterSummary {
        event: "summary",
        bind: local_addr.to_string(),
        name,
        seconds: round3(elapsed),
        master: master.snapshot(),
        streams,
        receiver: receiver_snapshot,
        record: record_summary,
    };

    println!();
    println!(
        "summary: {:.1} s, {} stream(s), master peak-hold {} dBFS, {} clipped sample(s)",
        summary.seconds,
        summary.streams.len(),
        summary.master.peak_hold_db,
        summary.master.clipped_samples
    );
    for stream in &summary.streams {
        println!(
            "  {} [{}] peak-hold {} dBFS, buffer {} ms, ratio {:.5}, lost {} frame(s), underruns {}, resets {}",
            stream.label,
            stream.codec,
            stream.peak_hold_db,
            stream.buffer_ms,
            stream.ratio,
            stream.lost_frames,
            stream.underruns,
            stream.buffer_resets
        );
    }
    if let Some(record) = &summary.record {
        println!(
            "  recorded {} ({:.1} s) to {}",
            record.frames, record.seconds, record.path
        );
    }
    if let Some(path) = &args.stats_json {
        util::append_json_line(path, &summary)?;
    }
    Ok(())
}

fn start_sink(args: &CenterArgs, mixer: Mixer, stop: &Arc<AtomicBool>) -> Result<Sink> {
    if let Some(path) = &args.record {
        let path = path.clone();
        let stop = stop.clone();
        println!("mode: recording the mix to {path}");
        let handle = thread::Builder::new()
            .name("siren-recorder".into())
            .spawn(move || record_to_wav(mixer, Path::new(&path), stop))
            .context("spawn recorder")?;
        return Ok(Sink::Record(handle));
    }

    if args.no_play {
        println!("mode: silent (no output device)");
        let stop = stop.clone();
        let handle = thread::Builder::new()
            .name("siren-silent-clock".into())
            .spawn(move || silent_clock(mixer, stop))
            .context("spawn silent clock")?;
        return Ok(Sink::Silent(handle));
    }

    let playback = start_playback(mixer, args.device.as_deref())?;
    println!("mode: playback → {}", playback.describe());
    Ok(Sink::Playback(playback))
}

fn stop_sink(sink: Sink) -> Result<Option<RecordSummary>> {
    match sink {
        Sink::Record(handle) => {
            let result = handle
                .join()
                .map_err(|_| anyhow!("recorder thread panicked"))?;
            Ok(Some(result.context("recording failed")?))
        }
        Sink::Silent(handle) => {
            let _ = handle.join();
            Ok(None)
        }
        Sink::Playback(playback) => {
            drop(playback);
            Ok(None)
        }
    }
}

fn silent_clock(mut mixer: Mixer, stop: Arc<AtomicBool>) {
    let frames = frames_for_ms(5.0);
    let mut buffer = vec![0.0f32; frames * CHANNELS];
    let step = Duration::from_secs_f64(0.005);
    let mut deadline = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        deadline += step;
        mixer.render(&mut buffer);
        let now = Instant::now();
        if deadline > now {
            thread::sleep(deadline - now);
        }
    }
}

#[derive(Serialize)]
struct StatsLine<'a> {
    ts_ms: u128,
    elapsed_s: f64,
    master: &'a MasterSnapshot,
    streams: &'a [StreamSnapshot],
}

fn spawn_stats(
    registry: Arc<MetricsRegistry>,
    master: Arc<MixerMetrics>,
    stop: Arc<AtomicBool>,
    interval: f64,
    json_path: Option<String>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("siren-stats".into())
        .spawn(move || {
            let mut file = json_path.as_ref().and_then(|path| {
                if let Some(parent) = Path::new(path)
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    Ok(file) => Some(std::io::BufWriter::new(file)),
                    Err(error) => {
                        eprintln!("warning: cannot open stats JSON {path}: {error}");
                        None
                    }
                }
            });
            let started = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                let mut slept = 0.0;
                while slept < interval && !stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(50));
                    slept += 0.05;
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let streams = registry.snapshots();
                let master_snapshot = master.snapshot();
                let elapsed = started.elapsed().as_secs_f64();
                println!(
                    "[{:6.1}s] mix {:7.1} dBFS  peak {:7.1} dBFS  {} stream(s)",
                    elapsed,
                    master_snapshot.rms_db,
                    master_snapshot.peak_db,
                    streams.len()
                );
                for stream in &streams {
                    println!(
                        "    {:<22} {:<4} {:7.1} dB  buf {:5.1}/{:4.1} ms  ratio {:.5}  lost {:>6}  under {:>4}  reset {:>3}",
                        stream.label,
                        stream.codec,
                        stream.rms_db,
                        stream.buffer_ms,
                        stream.target_ms,
                        stream.ratio,
                        stream.lost_frames,
                        stream.underruns,
                        stream.buffer_resets
                    );
                }
                if let Some(file) = file.as_mut() {
                    use std::io::Write;
                    let line = StatsLine {
                        ts_ms: util::now_ms(),
                        elapsed_s: round3(elapsed),
                        master: &master_snapshot,
                        streams: &streams,
                    };
                    if serde_json::to_writer(&mut *file, &line).is_ok() {
                        let _ = file.write_all(b"\n");
                        let _ = file.flush();
                    }
                }
            }
        })
        .expect("spawn stats thread")
}

#[derive(Serialize)]
struct CenterSummary {
    event: &'static str,
    bind: String,
    name: String,
    seconds: f64,
    master: MasterSnapshot,
    streams: Vec<StreamSnapshot>,
    receiver: ReceiverStatsSnapshot,
    record: Option<RecordSummary>,
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
