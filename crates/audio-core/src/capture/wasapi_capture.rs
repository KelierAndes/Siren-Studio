//! WASAPI capture adapter (Windows only).
//!
//! System capture first tries the process-loopback endpoint, which mirrors
//! every render stream except this process tree — our own monitor output can
//! never be echoed back into the mesh. On systems without that API the render
//! endpoint is opened with `Direction::Capture` (classic loopback) and the
//! DSP canceller subtracts the monitor reference afterwards.
//!
//! A capture endpoint gives us the default microphone. Both paths hand out
//! 48 kHz / stereo / f32; the classic loopback path asks WASAPI for automatic
//! format conversion and falls back to the device mix format plus software
//! conversion if the driver refuses.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

use super::{CaptureKind, CapturePipe, CaptureStats};
use crate::format::{CHANNELS, SAMPLE_RATE};
use crate::{CoreError, Result};

pub fn start(kind: CaptureKind) -> Result<CapturePipe> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<std::result::Result<String, String>>(1);
    let (mut producer, consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS * 2);
    let stats = Arc::new(CaptureStats::default());
    let stop = Arc::new(AtomicBool::new(false));

    let thread_stats = stats.clone();
    let thread_stop = stop.clone();
    let handle = thread::Builder::new()
        .name("siren-wasapi-capture".into())
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
            // The thread may be stuck inside the audio service; joining it
            // would hang the caller as well.
            stop.store(true, Ordering::Relaxed);
            Err(CoreError::Backend(
                "timed out while opening the WASAPI capture stream".into(),
            ))
        }
    }
}

struct CaptureFormat {
    channels: usize,
    bits: usize,
    is_float: bool,
    block_align: usize,
}

impl CaptureFormat {
    fn from_wave_format(format: &WaveFormat) -> Result<Self> {
        let channels = format.get_nchannels() as usize;
        let bits = format.get_bitspersample() as usize;
        let is_float = matches!(format.get_subformat(), Ok(SampleType::Float));
        if channels == 0 || !matches!(bits, 16 | 24 | 32) {
            return Err(CoreError::Backend(format!(
                "unsupported capture format: {channels}ch/{bits}bit"
            )));
        }
        Ok(Self {
            channels,
            bits,
            is_float,
            block_align: channels * bits / 8,
        })
    }

    fn convert(&self, bytes: &[u8], out: &mut Vec<f32>) {
        out.clear();
        let frame_bytes = self.block_align;
        if frame_bytes == 0 {
            return;
        }
        for frame in bytes.chunks_exact(frame_bytes) {
            let (left, right) = if self.channels == 1 {
                let value = self.decode(frame, 0);
                (value, value)
            } else {
                (self.decode(frame, 0), self.decode(frame, 1))
            };
            out.push(left);
            out.push(right);
        }
    }

    fn decode(&self, frame: &[u8], channel: usize) -> f32 {
        let bytes_per_sample = self.bits / 8;
        let start = channel * bytes_per_sample;
        let sample = &frame[start..start + bytes_per_sample];
        match (self.is_float, self.bits) {
            (true, 32) => f32::from_le_bytes(sample.try_into().unwrap()),
            (false, 16) => i16::from_le_bytes(sample.try_into().unwrap()) as f32 / 32768.0,
            (false, 24) => {
                let value = i32::from_le_bytes([0, sample[0], sample[1], sample[2]]);
                (value >> 8) as f32 / 8_388_608.0
            }
            (false, 32) => i32::from_le_bytes(sample.try_into().unwrap()) as f32 / 2_147_483_648.0,
            _ => 0.0,
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
    match open_and_stream(kind, producer, stats, stop, &ready_tx) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Ignored when the successful readiness message was already sent.
            let _ = ready_tx.send(Err(error.to_string()));
            Err(error.to_string())
        }
    }
}

fn open_and_stream(
    kind: CaptureKind,
    producer: &mut rtrb::Producer<f32>,
    stats: &Arc<CaptureStats>,
    stop: &Arc<AtomicBool>,
    ready_tx: &mpsc::SyncSender<std::result::Result<String, String>>,
) -> Result<()> {
    crate::platform::boost_thread_priority();
    let _ = initialize_mta();

    if kind == CaptureKind::Loopback {
        match stream_process_loopback(producer, stats, stop, ready_tx) {
            Ok(()) => return Ok(()),
            Err(error) => log::warn!(
                "process loopback unavailable ({error}); \
                 falling back to endpoint loopback with echo cancellation"
            ),
        }
    }

    let enumerator = DeviceEnumerator::new()
        .map_err(|e| CoreError::Backend(format!("WASAPI enumerator: {e}")))?;
    let device = match kind {
        CaptureKind::Loopback => enumerator
            .get_default_device(&Direction::Render)
            .map_err(|e| CoreError::Backend(format!("no default playback device: {e}")))?,
        CaptureKind::Microphone => enumerator
            .get_default_device(&Direction::Capture)
            .map_err(|e| CoreError::Backend(format!("no default recording device: {e}")))?,
    };
    let label = if kind == CaptureKind::Loopback {
        "system audio (WASAPI loopback)"
    } else {
        "microphone (WASAPI capture)"
    };

    let mut audio_client = device
        .get_iaudioclient()
        .map_err(|e| CoreError::Backend(format!("IAudioClient: {e}")))?;
    let (default_period, _min_period) = audio_client
        .get_device_period()
        .map_err(|e| CoreError::Backend(format!("device period: {e}")))?;

    let desired = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        SAMPLE_RATE as usize,
        CHANNELS,
        None,
    );
    let auto_mode = StreamMode::PollingShared {
        autoconvert: true,
        buffer_duration_hns: default_period.max(10_000),
    };

    // Both capture kinds initialize a Capture stream. For a render endpoint
    // that is what enables WASAPI loopback; the endpoint stays a Render device.
    let format = match audio_client.initialize_client(&desired, &Direction::Capture, &auto_mode) {
        Ok(()) => desired,
        Err(auto_error) => {
            log::warn!("48 kHz autoconvert failed ({auto_error}), trying the device mix format");
            audio_client = device
                .get_iaudioclient()
                .map_err(|e| CoreError::Backend(format!("IAudioClient: {e}")))?;
            let mix = audio_client
                .get_mixformat()
                .map_err(|e| CoreError::Backend(format!("mix format: {e}")))?;
            if mix.get_samplespersec() != SAMPLE_RATE {
                return Err(CoreError::Backend(format!(
                    "device runs at {} Hz and refused format conversion; \
                     set the Windows output device to 48 kHz",
                    mix.get_samplespersec()
                )));
            }
            let no_auto = StreamMode::PollingShared {
                autoconvert: false,
                buffer_duration_hns: default_period.max(10_000),
            };
            audio_client
                .initialize_client(&mix, &Direction::Capture, &no_auto)
                .map_err(|e| CoreError::Backend(format!("mix-format capture init: {e}")))?;
            mix
        }
    };

    let converter = CaptureFormat::from_wave_format(&format)?;
    let capture = audio_client
        .get_audiocaptureclient()
        .map_err(|e| CoreError::Backend(format!("IAudioCaptureClient: {e}")))?;
    audio_client
        .start_stream()
        .map_err(|e| CoreError::Backend(format!("start capture stream: {e}")))?;

    let description = format!(
        "{label} @ {} Hz, {} ch, {}",
        format.get_samplespersec(),
        format.get_nchannels(),
        if converter.is_float { "f32" } else { "pcm" }
    );
    log::info!("capture opened: {description}");
    let _ = ready_tx.send(Ok(description));

    // Loopback records our own monitor output too; track the exact rendered
    // reference so the canceller can subtract it back out.
    let cancel_echo = kind == CaptureKind::Loopback;
    if cancel_echo {
        crate::dsp::echo::set_reference_enabled(true);
    }
    let mut canceller = crate::dsp::echo::EchoCanceller::new();

    let mut bytes = Vec::new();
    let mut converted = Vec::with_capacity(16_384);
    while !stop.load(Ordering::Relaxed) {
        match capture.get_next_packet_size() {
            Ok(Some(frames)) => {
                let frames = frames as usize;
                bytes.resize(frames * converter.block_align, 0);
                match capture.read_from_device(&mut bytes) {
                    Ok((read, info)) => {
                        let read = read as usize;
                        if info.flags.silent {
                            let zeros = vec![0.0f32; read * CHANNELS];
                            push(producer, &zeros, stats);
                            stats
                                .silent_frames
                                .fetch_add(read as u64, Ordering::Relaxed);
                        } else {
                            converter
                                .convert(&bytes[..read * converter.block_align], &mut converted);
                            if cancel_echo {
                                canceller.process(&mut converted);
                            }
                            push(producer, &converted, stats);
                        }
                        stats.frames.fetch_add(read as u64, Ordering::Relaxed);
                    }
                    Err(error) => {
                        stats.read_errors.fetch_add(1, Ordering::Relaxed);
                        log::warn!("capture read failed: {error}");
                        thread::sleep(Duration::from_millis(5));
                    }
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(error) => {
                stats.read_errors.fetch_add(1, Ordering::Relaxed);
                log::warn!("capture packet query failed: {error}");
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    let _ = audio_client.stop_stream();
    if cancel_echo {
        crate::dsp::echo::set_reference_enabled(false);
    }
    Ok(())
}

/// Streams system audio with this process excluded. Returns `Err` only if the
/// virtual endpoint cannot be opened; read failures after that are tolerated.
fn stream_process_loopback(
    producer: &mut rtrb::Producer<f32>,
    stats: &Arc<CaptureStats>,
    stop: &Arc<AtomicBool>,
    ready_tx: &mpsc::SyncSender<std::result::Result<String, String>>,
) -> Result<()> {
    let mut loopback = super::process_loopback::ProcessLoopback::open(std::process::id())?;
    let description = format!(
        "system audio (WASAPI process loopback, this app excluded) @ {SAMPLE_RATE} Hz, \
         {CHANNELS} ch, f32"
    );
    log::info!("capture opened: {description}");
    let _ = ready_tx.send(Ok(description));

    let mut block = Vec::with_capacity(16_384);
    while !stop.load(Ordering::Relaxed) {
        match loopback.next(&mut block) {
            Ok(true) => {
                let frames = (block.len() / CHANNELS) as u64;
                push(producer, &block, stats);
                stats.frames.fetch_add(frames, Ordering::Relaxed);
            }
            Ok(false) => thread::sleep(Duration::from_millis(2)),
            Err(error) => {
                stats.read_errors.fetch_add(1, Ordering::Relaxed);
                log::warn!("process loopback read failed: {error}");
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
    loopback.stop();
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
