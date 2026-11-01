//! Real-time WAV recorder used as a headless output sink (verification,
//! capture, and later the Linux daemon's "record mix" feature).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::dsp::Mixer;
use crate::format::{frames_for_ms, CHANNELS, SAMPLE_RATE};
use crate::{CoreError, Result};

#[derive(Clone, Debug, serde::Serialize)]
pub struct RecordSummary {
    pub path: String,
    pub frames: u64,
    pub seconds: f64,
}

pub fn record_to_wav(
    mut mixer: Mixer,
    path: &Path,
    stop: Arc<AtomicBool>,
) -> Result<RecordSummary> {
    crate::platform::boost_thread_priority();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let spec = hound::WavSpec {
        channels: CHANNELS as u16,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| CoreError::Wav(e))?;
    log::info!("recording mix to {}", path.display());

    let block_frames = frames_for_ms(5.0);
    let block_time = Duration::from_micros((block_frames as u64 * 1_000_000) / SAMPLE_RATE as u64);
    let mut buffer = vec![0.0f32; block_frames * CHANNELS];
    let mut frames_written = 0u64;
    let start = Instant::now();
    let mut deadline = start;

    while !stop.load(Ordering::Relaxed) {
        // If the OS stalled us, do not render a burst of blocks to "catch up":
        // that drains the jitter buffers faster than the senders refill them.
        let now = Instant::now();
        if now.saturating_duration_since(deadline) > block_time {
            deadline = now;
        }
        deadline += block_time;
        mixer.render(&mut buffer);
        for sample in &buffer {
            writer
                .write_sample(*sample)
                .map_err(|e| CoreError::Wav(e))?;
        }
        frames_written += block_frames as u64;

        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
    }

    writer.finalize().map_err(|e| CoreError::Wav(e))?;
    let seconds = frames_written as f64 / SAMPLE_RATE as f64;
    Ok(RecordSummary {
        path: path.to_string_lossy().to_string(),
        frames: frames_written,
        seconds,
    })
}
