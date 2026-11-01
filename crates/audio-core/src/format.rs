//! Transport-wide audio format constants.
//!
//! Every node on the mesh speaks the same format: 48 kHz, stereo, 32-bit
//! float PCM. Senders resample at the capture boundary, the center resamples
//! per stream for clock drift, and the output adapter resamples to whatever
//! the physical device wants.

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
pub const SAMPLE_BYTES: usize = 4;

/// Sample frames in `ms` milliseconds, rounded to the nearest frame.
pub fn frames_for_ms(ms: f32) -> usize {
    (ms * SAMPLE_RATE as f32 / 1000.0).round() as usize
}

/// Milliseconds covered by `frames` sample frames.
pub fn ms_for_frames(frames: usize) -> f32 {
    frames as f32 * 1000.0 / SAMPLE_RATE as f32
}

pub fn samples_for_frames(frames: usize) -> usize {
    frames * CHANNELS
}

/// Convert interleaved f32 PCM to little-endian bytes.
pub fn pcm_to_bytes(pcm: &[f32], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(pcm.len() * SAMPLE_BYTES);
    for sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
}

/// Convert little-endian f32 bytes to interleaved PCM, appending to `out`.
pub fn bytes_to_pcm(bytes: &[u8], out: &mut Vec<f32>) {
    out.clear();
    out.reserve(bytes.len() / SAMPLE_BYTES);
    for chunk in bytes.chunks_exact(SAMPLE_BYTES) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_math() {
        assert_eq!(frames_for_ms(5.0), 240);
        assert_eq!(frames_for_ms(2.5), 120);
        assert_eq!(frames_for_ms(10.0), 480);
        assert!((ms_for_frames(240) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn pcm_roundtrip() {
        let pcm = vec![0.0, 0.25, -0.5, 1.0, -1.0, 0.125];
        let mut bytes = Vec::new();
        pcm_to_bytes(&pcm, &mut bytes);
        let mut decoded = Vec::new();
        bytes_to_pcm(&bytes, &mut decoded);
        assert_eq!(pcm, decoded);
    }
}
