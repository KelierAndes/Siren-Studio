//! System audio capture.
//!
//! Windows uses WASAPI: a render endpoint opened in loopback mode for system
//! audio, or a capture endpoint for microphones. The capture thread pushes
//! interleaved f32 frames into a lock-free ring; the sender's pacing loop
//! drains it at exactly 48 kHz and zero-fills gaps so the clock never stalls.

pub mod tone;

#[cfg(windows)]
mod process_loopback;
#[cfg(windows)]
mod wasapi_capture;
#[cfg(target_os = "android")]
pub mod android_capture;
#[cfg(target_os = "linux")]
mod pulse_capture;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::format::CHANNELS;
use crate::Result;

pub use tone::ToneSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    /// Everything the system is currently playing (WASAPI loopback).
    Loopback,
    /// Default recording device.
    Microphone,
}

#[derive(Default)]
pub struct CaptureStats {
    pub frames: AtomicU64,
    pub dropped_frames: AtomicU64,
    pub silent_frames: AtomicU64,
    pub read_errors: AtomicU64,
}

pub struct CapturePipe {
    rx: rtrb::Consumer<f32>,
    stats: Arc<CaptureStats>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    pub description: String,
}

impl CapturePipe {
    pub fn start(kind: CaptureKind) -> Result<Self> {
        #[cfg(windows)]
        {
            wasapi_capture::start(kind)
        }
        #[cfg(target_os = "android")]
        {
            android_capture::start(kind)
        }
        #[cfg(target_os = "linux")]
        {
            pulse_capture::start(kind)
        }
        #[cfg(not(any(windows, target_os = "android", target_os = "linux")))]
        {
            let _ = kind;
            Err(crate::CoreError::Backend(
                "system audio capture is implemented for Windows (WASAPI), Android \
                 (AudioPlaybackCapture) and Linux (PulseAudio)"
                    .into(),
            ))
        }
    }

    pub fn from_parts(
        rx: rtrb::Consumer<f32>,
        stats: Arc<CaptureStats>,
        stop: Arc<AtomicBool>,
        handle: JoinHandle<()>,
        description: String,
    ) -> Self {
        Self {
            rx,
            stats,
            stop,
            handle: Some(handle),
            description,
        }
    }

    pub fn stats(&self) -> Arc<CaptureStats> {
        self.stats.clone()
    }

    /// Copy up to `out.len()` interleaved samples into `out`, zero-filling the
    /// remainder. Returns the number of frames actually read.
    pub fn read_frame(&mut self, out: &mut [f32]) -> usize {
        let filled = {
            let (filled, rest) = self.rx.pop_partial_slice(out);
            let count = filled.len();
            rest.fill(0.0);
            count
        };
        filled / CHANNELS
    }

    pub fn buffered_frames(&self) -> usize {
        self.rx.slots() / CHANNELS
    }

    /// Signals the capture thread to stop. Deliberately does not join: the
    /// Android/Oboe teardown can block for a long time, and the capture thread
    /// owns everything it needs, so it finishes on its own.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take();
    }
}

impl Drop for CapturePipe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take();
    }
}
