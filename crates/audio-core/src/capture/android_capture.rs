//! Android capture adapter.
//!
//! Microphone: a cpal input stream (Oboe), converted to 48 kHz stereo f32 in
//! software. Requires `ndk_context` to be initialized by the host app and the
//! RECORD_AUDIO runtime permission.
//!
//! System audio: `AudioPlaybackCapture` needs a MediaProjection consent
//! dialog, so the AudioRecord lives in Kotlin; it pushes interleaved 48 kHz
//! stereo f32 buffers through the JNI bridge into a process-global ring.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};

use super::{CaptureKind, CapturePipe, CaptureStats};
use crate::format::{CHANNELS, SAMPLE_RATE};
use crate::{CoreError, Result};

/// Java-side hooks, installed by the FFI crate once the Activity is known.
pub struct JavaBridge {
    /// Ask Kotlin to request RECORD_AUDIO if needed; false = dialog was shown.
    pub ensure_capture_permission: fn() -> bool,
    /// Start MediaProjection consent + AudioPlaybackCapture at 48 kHz stereo.
    pub start_system_capture: fn() -> bool,
    pub stop_system_capture: fn(),
}

static BRIDGE: OnceLock<JavaBridge> = OnceLock::new();

pub fn install_java_bridge(bridge: JavaBridge) {
    let _ = BRIDGE.set(bridge);
}

fn bridge() -> Result<&'static JavaBridge> {
    BRIDGE.get().ok_or_else(|| {
        CoreError::Backend("capture bridge not initialized (JNI context missing)".into())
    })
}

pub fn start(kind: CaptureKind) -> Result<CapturePipe> {
    let bridge = bridge()?;
    if !(bridge.ensure_capture_permission)() {
        return Err(CoreError::Backend(
            "需要录音权限：请在系统对话框中允许后重试".into(),
        ));
    }
    match kind {
        CaptureKind::Microphone => start_microphone(),
        CaptureKind::Loopback => start_system_capture(bridge),
    }
}

// ---------------------------------------------------------------------------
// System audio (Kotlin AudioPlaybackCapture → JNI push)
// ---------------------------------------------------------------------------

struct SystemSink {
    producer: Option<rtrb::Producer<f32>>,
    stats: Option<Arc<CaptureStats>>,
    canceller: Option<crate::dsp::echo::EchoCanceller>,
}

static SYSTEM_SINK: OnceLock<Mutex<SystemSink>> = OnceLock::new();

fn system_sink() -> &'static Mutex<SystemSink> {
    SYSTEM_SINK.get_or_init(|| {
        Mutex::new(SystemSink {
            producer: None,
            stats: None,
            canceller: None,
        })
    })
}

/// Called from the JNI thread for every captured block (48 kHz stereo f32).
pub fn push_system_audio(samples: &[f32]) -> usize {
    let mut sink = match system_sink().lock() {
        Ok(sink) => sink,
        Err(_) => return 0,
    };
    let stats = sink.stats.clone();
    let Some(stats) = stats else {
        return 0;
    };
    let mut filtered = samples.to_vec();
    if let Some(canceller) = sink.canceller.as_mut() {
        canceller.process(&mut filtered);
    }
    let Some(producer) = sink.producer.as_mut() else {
        return 0;
    };
    let (_, rest) = producer.push_partial_slice(&filtered);
    if !rest.is_empty() {
        stats
            .dropped_frames
            .fetch_add((rest.len() / CHANNELS) as u64, Ordering::Relaxed);
    }
    stats
        .frames
        .fetch_add((samples.len() / CHANNELS) as u64, Ordering::Relaxed);
    samples.len()
}

fn start_system_capture(bridge: &'static JavaBridge) -> Result<CapturePipe> {
    let (producer, consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS * 2);
    let stats = Arc::new(CaptureStats::default());
    let stop = Arc::new(AtomicBool::new(false));

    // L1: AndroidManifest declares allowAudioPlaybackCapture="false", so our
    // own playback never enters this capture. The canceller is redundant and
    // its periodic lock scans compete with the AAudio callback (underruns), so
    // it is off for system capture by default. Set SIREN_ANDROID_ECHO=1 for
    // ROMs that ignore the app-wide capture policy.
    let echo_enabled = std::env::var("SIREN_ANDROID_ECHO")
        .map(|value| value == "1")
        .unwrap_or(false);
    {
        let mut sink = system_sink().lock().unwrap();
        sink.producer = Some(producer);
        sink.stats = Some(stats.clone());
        sink.canceller = echo_enabled.then(crate::dsp::echo::EchoCanceller::new);
    }
    if echo_enabled {
        crate::dsp::echo::set_reference_enabled(true);
    }

    if !(bridge.start_system_capture)() {
        let mut sink = system_sink().lock().unwrap();
        sink.producer = None;
        sink.stats = None;
        sink.canceller = None;
        crate::dsp::echo::set_reference_enabled(false);
        return Err(CoreError::Backend(
            "无法启动系统内录（MediaProjection）".into(),
        ));
    }

    let thread_stop = stop.clone();
    let handle = thread::Builder::new()
        .name("siren-android-syscap".into())
        .spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(50));
            }
            (bridge.stop_system_capture)();
            crate::dsp::echo::set_reference_enabled(false);
            if let Ok(mut sink) = system_sink().lock() {
                sink.producer = None;
                sink.stats = None;
                sink.canceller = None;
            }
        })
        .map_err(|e| CoreError::Backend(format!("spawn syscapture watcher: {e}")))?;

    log::info!("capture opened: system audio (Android AudioPlaybackCapture)");
    Ok(CapturePipe::from_parts(
        consumer,
        stats,
        stop,
        handle,
        "system audio (AudioPlaybackCapture) @ 48000 Hz, 2 ch, f32".into(),
    ))
}

// ---------------------------------------------------------------------------
// Microphone (cpal/Oboe input)
// ---------------------------------------------------------------------------

fn start_microphone() -> Result<CapturePipe> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<std::result::Result<String, String>>(1);
    let (producer, consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS * 2);
    let stats = Arc::new(CaptureStats::default());
    let stop = Arc::new(AtomicBool::new(false));

    let thread_stats = stats.clone();
    let thread_stop = stop.clone();
    let handle = thread::Builder::new()
        .name("siren-android-mic".into())
        .spawn(move || {
            if let Err(message) = run_microphone(producer, &thread_stats, &thread_stop, &ready_tx) {
                log::error!("mic capture stopped: {message}");
            }
        })
        .map_err(|e| CoreError::Backend(format!("spawn mic capture thread: {e}")))?;

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
            let _ = handle.join();
            Err(CoreError::Backend(
                "timed out while opening the microphone".into(),
            ))
        }
    }
}

fn run_microphone(
    producer: rtrb::Producer<f32>,
    stats: &Arc<CaptureStats>,
    stop: &Arc<AtomicBool>,
    ready_tx: &mpsc::SyncSender<std::result::Result<String, String>>,
) -> std::result::Result<(), String> {
    let result = open_mic_stream(producer, stats, stop, ready_tx);
    if let Err(error) = &result {
        let _ = ready_tx.send(Err(error.to_string()));
    }
    result.map_err(|e| e.to_string())
}

fn open_mic_stream(
    producer: rtrb::Producer<f32>,
    stats: &Arc<CaptureStats>,
    stop: &Arc<AtomicBool>,
    ready_tx: &mpsc::SyncSender<std::result::Result<String, String>>,
) -> Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| CoreError::Backend("no microphone available".into()))?;
    let supported = device
        .default_input_config()
        .map_err(|e| CoreError::Backend(format!("microphone default config: {e}")))?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.config();
    let device_channels = config.channels as usize;
    let device_rate = config.sample_rate;

    let callback_stats = stats.clone();
    let error_cb = |error: cpal::Error| log::error!("mic stream error: {error}");

    let stream = match sample_format {
        SampleFormat::F32 => build_input::<f32>(
            &device,
            config,
            device_channels,
            device_rate,
            producer,
            callback_stats,
            error_cb,
        ),
        SampleFormat::I16 => build_input::<i16>(
            &device,
            config,
            device_channels,
            device_rate,
            producer,
            callback_stats,
            error_cb,
        ),
        SampleFormat::U16 => build_input::<u16>(
            &device,
            config,
            device_channels,
            device_rate,
            producer,
            callback_stats,
            error_cb,
        ),
        other => Err(CoreError::Backend(format!(
            "unsupported microphone sample format {other:?}"
        ))),
    }?;

    stream
        .play()
        .map_err(|e| CoreError::Backend(format!("start microphone: {e}")))?;

    let description = format!(
        "microphone ({}) @ {} Hz, {} ch",
        device.to_string(),
        device_rate,
        device_channels
    );
    log::info!("capture opened: {description}");
    let _ = ready_tx.send(Ok(description));

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(20));
    }
    drop(stream);
    Ok(())
}

fn build_input<T>(
    device: &cpal::Device,
    config: StreamConfig,
    device_channels: usize,
    device_rate: u32,
    producer: rtrb::Producer<f32>,
    stats: Arc<CaptureStats>,
    error_cb: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream>
where
    T: SizedSample + Sample + FromSample<f32> + 'static,
    f32: FromSample<T>,
{
    let mut adapter = MicAdapter::new(device_channels, device_rate, producer, stats);
    device
        .build_input_stream(
            config,
            move |data: &[T], _info| adapter.push(data),
            error_cb,
            None,
        )
        .map_err(|e| CoreError::Backend(format!("build microphone stream: {e}")))
}

/// Converts the device's interleaved input into 48 kHz stereo f32 and pushes
/// whole frames into the ring.
struct MicAdapter {
    device_channels: usize,
    /// Input frames per output frame.
    ratio: f64,
    pos: f64,
    producer: rtrb::Producer<f32>,
    stats: Arc<CaptureStats>,
    scratch: Vec<f32>,
    frame: Vec<f32>,
}

impl MicAdapter {
    fn new(
        device_channels: usize,
        device_rate: u32,
        producer: rtrb::Producer<f32>,
        stats: Arc<CaptureStats>,
    ) -> Self {
        Self {
            device_channels: device_channels.max(1),
            ratio: device_rate.max(1) as f64 / SAMPLE_RATE as f64,
            pos: 0.0,
            producer,
            stats,
            scratch: Vec::with_capacity(4096),
            frame: Vec::with_capacity(2),
        }
    }

    fn push<T>(&mut self, data: &[T])
    where
        T: Sample + FromSample<f32>,
        f32: FromSample<T>,
    {
        let frames = data.len() / self.device_channels;
        for index in 0..frames {
            let base = index * self.device_channels;
            let left = f32::from_sample(data[base]);
            let right = if self.device_channels > 1 {
                f32::from_sample(data[base + 1])
            } else {
                left
            };
            self.scratch.push(left);
            self.scratch.push(right);
        }

        // Linear resample with a persistent fractional position.
        while self.pos + 1.0 < (self.scratch.len() / CHANNELS) as f64 {
            let index = self.pos.floor() as usize;
            let frac = (self.pos - index as f64) as f32;
            let l0 = self.scratch[index * CHANNELS];
            let r0 = self.scratch[index * CHANNELS + 1];
            let l1 = self.scratch[(index + 1) * CHANNELS];
            let r1 = self.scratch[(index + 1) * CHANNELS + 1];
            self.frame.clear();
            self.frame.push(l0 + (l1 - l0) * frac);
            self.frame.push(r0 + (r1 - r0) * frac);
            let (_, rest) = self.producer.push_partial_slice(&self.frame);
            if !rest.is_empty() {
                self.stats.dropped_frames.fetch_add(1, Ordering::Relaxed);
            }
            self.pos += self.ratio;
        }

        let consumed = self.pos.floor() as usize;
        if consumed >= 1 {
            self.scratch.drain(0..consumed * CHANNELS);
            self.pos -= consumed as f64;
        }
    }
}
