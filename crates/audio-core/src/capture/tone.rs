//! Deterministic sine source used by tests and by `siren send --source tone`.

use crate::format::{CHANNELS, SAMPLE_RATE};

pub struct ToneSource {
    phase: f64,
    step: f64,
    amplitude: f32,
}

impl ToneSource {
    pub fn new(freq: f64, amplitude: f32) -> Self {
        Self {
            phase: 0.0,
            step: 2.0 * std::f64::consts::PI * freq / SAMPLE_RATE as f64,
            amplitude: amplitude.clamp(0.0, 1.0),
        }
    }

    pub fn fill(&mut self, pcm: &mut [f32]) {
        for frame in pcm.chunks_exact_mut(CHANNELS) {
            let value = (self.phase.sin() as f32) * self.amplitude;
            frame[0] = value;
            frame[1] = value;
            self.phase += self.step;
            if self.phase >= 2.0 * std::f64::consts::PI {
                self.phase -= 2.0 * std::f64::consts::PI;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::frames_for_ms;

    #[test]
    fn tone_has_expected_frequency() {
        let mut source = ToneSource::new(1000.0, 0.5);
        let mut pcm = vec![0.0f32; frames_for_ms(1000.0) * CHANNELS];
        source.fill(&mut pcm);
        let crossings = pcm
            .chunks_exact(CHANNELS)
            .fold((0u32, false), |(count, last_positive), frame| {
                let positive = frame[0] >= 0.0;
                (count + u32::from(positive && !last_positive), positive)
            })
            .0;
        assert!(
            (crossings as i32 - 1000).abs() <= 2,
            "got {crossings} cycles"
        );
    }
}
