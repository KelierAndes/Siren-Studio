//! PulseAudio capture (Linux).
//!
//! System capture records the default sink's monitor, i.e. everything the
//! machine is playing. That mix also contains this node's mesh monitor output,
//! so — unlike the Windows process-loopback path — the DSP canceller runs on
//! the captured blocks to remove the node's own playback before it is sent
//! back to the other nodes.
//!
//! Microphone capture uses the server's default source. PulseAudio (or the
//! PipeWire pulse layer) converts both to 48 kHz stereo f32.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;

use super::{CaptureKind, CapturePipe, CaptureStats};
use crate::format::{CHANNELS, SAMPLE_RATE};
use crate::{CoreError, Result};

const FRAGMENT_FRAMES: usize = 240; // 5 ms
const BYTES_PER_FRAME: usize = CHANNELS * 4;
const MAX_CONSECUTIVE_ERRORS: u32 = 100;

pub fn start(kind: CaptureKind) -> Result<CapturePipe> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<std::result::Result<String, String>>(1);
    let (mut producer, consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS * 2);
    let stats = Arc::new(CaptureStats::default());
    let stop = Arc::new(AtomicBool::new(false));

    let thread_stats = stats.clone();
    let thread_stop = stop.clone();
    let handle = thread::Builder::new()
        .name("siren-pulse-capture".into())
        .spawn(move || {
            let result = run(kind, &mut producer, &thread_stats, &thread_stop, ready_tx);
            if let Err(message) = result {
                log::error!("capture stopped: {message}");
            }
        })
        .map_err(|e| CoreError::Backend(format!("failed to spawn capture thread: {e}")))?;

    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(description)) => Ok(CapturePipe::from_parts(
            consumer,
            stats,
            stop,
            handle,
            description,
        )),
        Ok(Err(message)) => {
            let _ = handle.join();
            Err(CoreError::Backend(message))
        }
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            Err(CoreError::Backend(
                "timed out while opening the PulseAudio capture stream".into(),
            ))
        }
    }
}

fn run(
    kind: CaptureKind,
    producer: &mut rtrb::Producer<f32>,
    stats: &Arc<CaptureStats>,
    stop: &Arc<AtomicBool>,
    ready_tx: mpsc::SyncSender<std::result::Result<String, String>>,
) -> std::result::Result<(), String> {
    match stream(kind, producer, stats, stop, &ready_tx) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = ready_tx.send(Err(error.clone()));
            Err(error)
        }
    }
}

fn stream(
    kind: CaptureKind,
    producer: &mut rtrb::Producer<f32>,
    stats: &Arc<CaptureStats>,
    stop: &Arc<AtomicBool>,
    ready_tx: &mpsc::SyncSender<std::result::Result<String, String>>,
) -> std::result::Result<(), String> {
    // Isolation mode points the capture at a dedicated virtual sink and turns
    // the canceller off: our playback never enters that monitor, so the DSP
    // would only burn CPU (and it must not record a reference at all).
    let source_override = std::env::var("SIREN_CAPTURE_SOURCE").ok();
    let cancel_echo = std::env::var("SIREN_ECHO_CANCEL")
        .map(|value| value != "0")
        .unwrap_or(true);
    let (device, label, cancel_echo) = match kind {
        CaptureKind::Loopback => (
            source_override.as_deref().or(Some("@DEFAULT_MONITOR@")),
            if cancel_echo {
                "system audio (PulseAudio monitor)"
            } else {
                "system audio (PulseAudio monitor, isolated)"
            },
            cancel_echo,
        ),
        CaptureKind::Microphone => (None, "microphone (PulseAudio source)", false),
    };

    let spec = Spec {
        format: Format::F32le,
        rate: SAMPLE_RATE,
        channels: CHANNELS as u8,
    };
    let attr = BufferAttr {
        // Bound the record queue: a long pause (e.g. the lock scan) must not
        // leave the capture permanently behind the playback timeline.
        maxlength: (SAMPLE_RATE as usize * CHANNELS * 4 / 4) as u32, // 250 ms
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: (FRAGMENT_FRAMES * BYTES_PER_FRAME) as u32,
    };
    let simple = Simple::new(
        None,
        "SirenStudio",
        Direction::Record,
        device,
        "mesh capture",
        &spec,
        None,
        Some(&attr),
    )
    .map_err(|e| {
        if cancel_echo {
            format!(
                "PulseAudio monitor capture failed ({e}); \
                 system audio needs PulseAudio >= 15 or PipeWire"
            )
        } else {
            format!("PulseAudio record stream failed: {e}")
        }
    })?;

    let description = format!(
        "{label} @ {SAMPLE_RATE} Hz, {CHANNELS} ch, f32, {:.0} ms fragments",
        FRAGMENT_FRAMES as f32 * 1000.0 / SAMPLE_RATE as f32
    );
    log::info!("capture opened: {description}");
    let _ = ready_tx.send(Ok(description.clone()));

    if cancel_echo {
        crate::dsp::echo::set_reference_enabled(true);
    }
    let mut canceller = crate::dsp::echo::EchoCanceller::new();

    let mut bytes = vec![0u8; FRAGMENT_FRAMES * BYTES_PER_FRAME];
    let mut pcm = Vec::with_capacity(FRAGMENT_FRAMES * CHANNELS);
    let mut errors = 0u32;
    while !stop.load(Ordering::Relaxed) {
        if let Err(error) = simple.read(&mut bytes) {
            errors += 1;
            stats.read_errors.fetch_add(1, Ordering::Relaxed);
            log::warn!("PulseAudio capture read failed: {error}");
            if errors >= MAX_CONSECUTIVE_ERRORS {
                break;
            }
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        errors = 0;

        pcm.clear();
        pcm.extend(
            bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
        if cancel_echo {
            canceller.process(&mut pcm);
        }
        if pcm.iter().all(|sample| *sample == 0.0) {
            stats
                .silent_frames
                .fetch_add(FRAGMENT_FRAMES as u64, Ordering::Relaxed);
        }
        push(producer, &pcm, stats);
        stats
            .frames
            .fetch_add(FRAGMENT_FRAMES as u64, Ordering::Relaxed);
    }

    if cancel_echo {
        crate::dsp::echo::set_reference_enabled(false);
    }
    Ok(())
}

fn push(producer: &mut rtrb::Producer<f32>, samples: &[f32], stats: &Arc<CaptureStats>) {
    let (_, rest) = producer.push_partial_slice(samples);
    if !rest.is_empty() {
        stats
            .dropped_frames
            .fetch_add((rest.len() / CHANNELS) as u64, Ordering::Relaxed);
    }
}
