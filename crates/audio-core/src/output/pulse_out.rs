//! Playback through PulseAudio (Linux).
//!
//! A dedicated thread renders the 48 kHz stereo mix in 5 ms blocks and blocks
//! in `pa_simple_write`, so the server clock paces the mixer. This also lets
//! the node run against a headless PulseAudio/PipeWire instance and test sinks
//! without touching ALSA devices directly.

use std::f64::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;

use super::adapter::OutputAdapter;
use crate::dsp::Mixer;
use crate::format::{CHANNELS, SAMPLE_RATE};
use crate::{CoreError, Result};

// Small blocks keep the server-side playback buffer — and therefore the
// reference-to-monitor delay the canceller sees — short and stable.
const BLOCK_FRAMES: usize = 120; // 2.5 ms
const BYTES_PER_FRAME: usize = CHANNELS * 4;

pub struct Playback {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Playback {
    pub fn describe(&self) -> String {
        format!(
            "{} @ {} Hz / {} ch",
            self.device_name, self.sample_rate, self.channels
        )
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn open_stream(device: Option<&str>) -> Result<Simple> {
    let spec = Spec {
        format: Format::F32le,
        rate: SAMPLE_RATE,
        channels: CHANNELS as u8,
    };
    // 20 ms keeps the sink from underrunning on a busy NAS; the fill still
    // only oscillates by one 2.5 ms block, inside the canceller's window.
    let target = 20 * SAMPLE_RATE / 1000 * BYTES_PER_FRAME as u32;
    let attr = BufferAttr {
        maxlength: target * 2,
        tlength: target,
        prebuf: target,
        minreq: u32::MAX,
        fragsize: u32::MAX,
    };
    Simple::new(
        None,
        "SirenStudio",
        Direction::Playback,
        device,
        "mesh monitor",
        &spec,
        None,
        Some(&attr),
    )
    .map_err(|e| CoreError::Backend(format!("PulseAudio playback stream: {e}")))
}

fn encode(pcm: &[f32], bytes: &mut Vec<u8>) {
    bytes.clear();
    for sample in pcm {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
}

pub fn list_output_devices() -> Result<Vec<(String, bool)>> {
    // Sink enumeration would need a full introspection context; the CLI
    // accepts an explicit sink name through --device instead.
    Ok(vec![("PulseAudio default sink".to_string(), true)])
}

pub fn start_playback(mixer: Mixer, device_query: Option<&str>) -> Result<Playback> {
    let simple = open_stream(device_query)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let handle = thread::Builder::new()
        .name("siren-pulse-out".into())
        .spawn(move || {
            let mut adapter = OutputAdapter::new(mixer, SAMPLE_RATE, CHANNELS);
            let mut pcm = vec![0.0f32; BLOCK_FRAMES * CHANNELS];
            let mut bytes = Vec::with_capacity(BLOCK_FRAMES * BYTES_PER_FRAME);
            let mut blocks = 0u32;
            while !thread_stop.load(Ordering::Relaxed) {
                adapter.write(&mut pcm);
                encode(&pcm, &mut bytes);
                if let Err(error) = simple.write(&bytes) {
                    log::error!("PulseAudio playback write failed: {error}");
                    break;
                }
                blocks += 1;
                if blocks % 200 == 0 {
                    if let Ok(us) = simple.get_latency() {
                        log::debug!("pulse playback latency {:.1} ms", us.0 as f64 / 1000.0);
                    }
                }
            }
            let _ = simple.drain();
        })
        .map_err(|e| CoreError::Backend(format!("spawn playback thread: {e}")))?;

    Ok(Playback {
        stop,
        handle: Some(handle),
        device_name: device_query
            .map(str::to_string)
            .unwrap_or_else(|| "PulseAudio default sink".into()),
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS as u16,
    })
}

pub fn play_tone(
    freq: f64,
    amplitude: f32,
    duration: Duration,
    device_query: Option<&str>,
) -> Result<Playback> {
    let simple = open_stream(device_query)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let handle = thread::Builder::new()
        .name("siren-pulse-tone".into())
        .spawn(move || {
            let step = TAU * freq / SAMPLE_RATE as f64;
            let mut phase = 0.0f64;
            let mut pcm = vec![0.0f32; BLOCK_FRAMES * CHANNELS];
            let mut bytes = Vec::with_capacity(BLOCK_FRAMES * BYTES_PER_FRAME);
            while !thread_stop.load(Ordering::Relaxed) {
                for frame in pcm.chunks_exact_mut(CHANNELS) {
                    let value = (phase.sin() as f32) * amplitude;
                    frame.fill(value);
                    phase += step;
                    if phase >= TAU {
                        phase -= TAU;
                    }
                }
                encode(&pcm, &mut bytes);
                if let Err(error) = simple.write(&bytes) {
                    log::error!("PulseAudio tone write failed: {error}");
                    break;
                }
            }
            let _ = simple.drain();
        })
        .map_err(|e| CoreError::Backend(format!("spawn tone thread: {e}")))?;

    thread::sleep(duration);
    Ok(Playback {
        stop,
        handle: Some(handle),
        device_name: device_query
            .map(str::to_string)
            .unwrap_or_else(|| "PulseAudio default sink".into()),
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS as u16,
    })
}
