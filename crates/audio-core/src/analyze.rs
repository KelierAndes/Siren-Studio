//! Offline WAV analysis used to prove end-to-end behaviour in CI/scripts.
//!
//! Goertzel evaluates the energy at the exact test frequencies (cheap and
//! precise for known tones), and 5 ms block RMS detects dropouts.

use serde::Serialize;

use crate::metrics::linear_to_db;
use crate::{CoreError, Result};

#[derive(Clone, Debug, Serialize)]
pub struct ToneLevel {
    pub freq_hz: f64,
    /// Frequency of the strongest nearby bin; drift correction may shift the
    /// tone by a few hundredths of a percent.
    pub detected_hz: f64,
    pub amplitude: f64,
    pub db: f64,
    pub present: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnalyzeResult {
    pub path: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_s: f64,
    pub window_start_s: f64,
    pub window_end_s: f64,
    pub rms_db: f64,
    pub peak_db: f64,
    pub dc_offset: f64,
    pub clipped_samples: u64,
    pub silent_blocks: u64,
    pub total_blocks: u64,
    pub tones: Vec<ToneLevel>,
}

pub fn analyze_wav(
    path: &str,
    freqs: &[f64],
    skip_start_s: f64,
    skip_end_s: f64,
    presence_db: f64,
) -> Result<AnalyzeResult> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let sample_rate = spec.sample_rate;

    let mut interleaved: Vec<f32> = Vec::new();
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                interleaved.push(sample?);
            }
        }
        hound::SampleFormat::Int => {
            if spec.bits_per_sample <= 16 {
                let scale = 1.0 / 32_768.0;
                for sample in reader.samples::<i16>() {
                    interleaved.push(sample? as f32 * scale);
                }
            } else {
                let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
                for sample in reader.samples::<i32>() {
                    interleaved.push(sample? as f32 * scale);
                }
            }
        }
    }

    if channels == 0 {
        return Err(CoreError::InvalidArgument("wav has zero channels".into()));
    }
    let total_frames = interleaved.len() / channels;
    let duration_s = total_frames as f64 / sample_rate as f64;

    let start_frame = ((skip_start_s.max(0.0) * sample_rate as f64) as usize).min(total_frames);
    let end_skip = ((skip_end_s.max(0.0) * sample_rate as f64) as usize).min(total_frames);
    let end_frame = total_frames.saturating_sub(end_skip).max(start_frame);

    let region = &interleaved[start_frame * channels..end_frame * channels];
    let mut mono = Vec::with_capacity(region.len() / channels);
    let mut peak = 0.0f32;
    let mut sumsq = 0.0f64;
    let mut dc = 0.0f64;
    let mut clipped = 0u64;
    for frame in region.chunks_exact(channels) {
        let mut sum = 0.0f32;
        for &sample in frame {
            peak = peak.max(sample.abs());
            sumsq += (sample as f64) * (sample as f64);
            dc += sample as f64;
            if sample.abs() >= 0.999 {
                clipped += 1;
            }
            sum += sample;
        }
        mono.push(sum / channels as f32);
    }
    let sample_count = region.len().max(1) as f64;

    let block_frames = (sample_rate as usize * 5) / 1000;
    let mut silent_blocks = 0u64;
    let mut total_blocks = 0u64;
    if block_frames > 0 {
        for block in mono.chunks(block_frames) {
            if block.len() < block_frames / 2 {
                continue;
            }
            total_blocks += 1;
            let rms = (block.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
                / block.len() as f64)
                .sqrt();
            if rms < 1e-4 {
                silent_blocks += 1;
            }
        }
    }

    let tones = freqs
        .iter()
        .map(|&freq| {
            // Search a ±2 Hz window so drift correction (which is allowed to
            // shift pitch by 0.05 %) does not read as "tone missing".
            let mut best = (freq, goertzel_amplitude(&mono, freq, sample_rate as f64));
            let mut offset = -2.0;
            while offset <= 2.0 {
                let candidate = goertzel_amplitude(&mono, freq + offset, sample_rate as f64);
                if candidate > best.1 {
                    best = (freq + offset, candidate);
                }
                offset += 0.25;
            }
            let (detected_hz, amplitude) = best;
            let db = linear_to_db(amplitude as f32) as f64;
            ToneLevel {
                freq_hz: freq,
                detected_hz,
                amplitude,
                db,
                present: db >= presence_db,
            }
        })
        .collect();

    Ok(AnalyzeResult {
        path: path.to_string(),
        sample_rate,
        channels: spec.channels,
        duration_s,
        window_start_s: start_frame as f64 / sample_rate as f64,
        window_end_s: end_frame as f64 / sample_rate as f64,
        rms_db: linear_to_db((sumsq / sample_count).sqrt() as f32) as f64,
        peak_db: linear_to_db(peak) as f64,
        dc_offset: dc / sample_count,
        clipped_samples: clipped,
        silent_blocks,
        total_blocks,
        tones,
    })
}

fn goertzel_amplitude(samples: &[f32], freq: f64, sample_rate: f64) -> f64 {
    let n = samples.len();
    if n < 4 || freq <= 0.0 || freq >= sample_rate * 0.5 {
        return 0.0;
    }
    let omega = 2.0 * std::f64::consts::PI * freq / sample_rate;
    let coeff = 2.0 * omega.cos();
    let mut s1 = 0.0f64;
    let mut s2 = 0.0f64;
    let mut window_sum = 0.0f64;
    for (i, sample) in samples.iter().enumerate() {
        let window = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos();
        let s0 = *sample as f64 * window + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
        window_sum += window;
    }
    let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
    2.0 * power.sqrt() / window_sum.max(1e-9)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::ToneSource;
    use crate::format::{CHANNELS, SAMPLE_RATE};

    #[test]
    fn detects_mixed_tones() {
        let path = std::env::temp_dir().join("siren-analyze-test.wav");
        let spec = hound::WavSpec {
            channels: CHANNELS as u16,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        let mut a = ToneSource::new(440.0, 0.2);
        let mut b = ToneSource::new(660.0, 0.2);
        let frames = SAMPLE_RATE as usize;
        let mut a_pcm = vec![0.0f32; frames * CHANNELS];
        let mut b_pcm = vec![0.0f32; frames * CHANNELS];
        a.fill(&mut a_pcm);
        b.fill(&mut b_pcm);
        for frame in 0..frames {
            let left = (a_pcm[frame * CHANNELS] + b_pcm[frame * CHANNELS]) * 0.5;
            let right = (a_pcm[frame * CHANNELS + 1] + b_pcm[frame * CHANNELS + 1]) * 0.5;
            writer.write_sample(left).unwrap();
            writer.write_sample(right).unwrap();
        }
        writer.finalize().unwrap();

        let result = analyze_wav(
            path.to_str().unwrap(),
            &[440.0, 660.0, 1000.0],
            0.05,
            0.05,
            -40.0,
        )
        .unwrap();
        assert!(result.tones[0].present);
        assert!(result.tones[1].present);
        assert!(!result.tones[2].present);
        assert!(result.tones[0].db > -30.0 && result.tones[0].db < 0.0);
        assert_eq!(result.silent_blocks, 0);
        let _ = std::fs::remove_file(path);
    }
}
