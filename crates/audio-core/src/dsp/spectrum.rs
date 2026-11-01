//! Lightweight per-stream spectrum for the UI's EQ display.
//!
//! 32 log-spaced bands between 20 Hz and 16 kHz, fed from the decoded stereo
//! audio. A 1024-point Hann-windowed FFT runs every fourth packet (~50 Hz
//! update), which is cheap enough for the receiver thread and smooth enough
//! for a 12 Hz UI poll.

use crate::metrics::StreamMetrics;

pub const SPECTRUM_BANDS: usize = 32;
const FFT_SIZE: usize = 1024;
const SAMPLE_RATE: f32 = 48_000.0;
const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 16_000.0;

pub struct SpectrumAnalyzer {
    window: Vec<f32>,
    band_bins: Vec<(usize, usize)>,
    ring: Vec<f32>,
    write: usize,
    filled: usize,
    packets: u32,
    smooth: [f32; SPECTRUM_BANDS],
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        let window = (0..FFT_SIZE)
            .map(|i| {
                let x = i as f32 / (FFT_SIZE as f32 - 1.0);
                0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
            })
            .collect();
        let mut band_bins = Vec::with_capacity(SPECTRUM_BANDS);
        for i in 0..SPECTRUM_BANDS {
            let t0 = i as f32 / SPECTRUM_BANDS as f32;
            let t1 = (i + 1) as f32 / SPECTRUM_BANDS as f32;
            let f0 = MIN_HZ * (MAX_HZ / MIN_HZ).powf(t0);
            let f1 = MIN_HZ * (MAX_HZ / MIN_HZ).powf(t1);
            let b0 = ((f0 * FFT_SIZE as f32 / SAMPLE_RATE).floor() as usize).max(1);
            let b1 = ((f1 * FFT_SIZE as f32 / SAMPLE_RATE).ceil() as usize)
                .min(FFT_SIZE / 2);
            band_bins.push((b0, b1.max(b0)));
        }
        Self {
            window,
            band_bins,
            ring: vec![0.0; FFT_SIZE],
            write: 0,
            filled: 0,
            packets: 0,
            smooth: [0.0; SPECTRUM_BANDS],
        }
    }

    /// Feed one decoded interleaved stereo block.
    pub fn push(&mut self, samples: &[f32], metrics: &StreamMetrics) {
        for frame in samples.chunks_exact(2) {
            self.ring[self.write] = (frame[0] + frame[1]) * 0.5;
            self.write = (self.write + 1) % FFT_SIZE;
            self.filled = (self.filled + 1).min(FFT_SIZE);
        }
        self.packets = self.packets.wrapping_add(1);
        if self.filled == FFT_SIZE && self.packets % 4 == 0 {
            self.analyze();
            metrics.set_spectrum(&self.smooth);
        }
    }

    pub fn bands(&self) -> [f32; SPECTRUM_BANDS] {
        self.smooth
    }

    fn analyze(&mut self) {
        let mut re = vec![0.0f32; FFT_SIZE];
        let mut im = vec![0.0f32; FFT_SIZE];
        for (i, (r, w)) in re.iter_mut().zip(self.window.iter()).enumerate() {
            let index = (self.write + i) % FFT_SIZE;
            *r = self.ring[index] * w;
        }
        fft(&mut re, &mut im);
        for (band, (b0, b1)) in self.band_bins.iter().enumerate() {
            let mut power = 0.0f32;
            for bin in *b0..=*b1 {
                power += re[bin] * re[bin] + im[bin] * im[bin];
            }
            // Hann window halves the peak; 4/N restores the amplitude scale.
            let magnitude = power.sqrt() * 4.0 / FFT_SIZE as f32;
            let db = 20.0 * magnitude.max(1e-7).log10();
            let value = ((db + 72.0) / 62.0).clamp(0.0, 1.0);
            self.smooth[band] = self.smooth[band] * 0.55 + value * 0.45;
        }
    }
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2usize;
    while len <= n {
        let angle = -std::f32::consts::TAU / len as f32;
        let (wr, wi) = (angle.cos(), angle.sin());
        let mut start = 0usize;
        while start < n {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let vr = re[b] * cr - im[b] * ci;
                let vi = re[b] * ci + im[b] * cr;
                re[b] = re[a] - vr;
                im[b] = im[a] - vi;
                re[a] += vr;
                im[a] += vi;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
            start += len;
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_lands_in_its_log_band() {
        let mut analyzer = SpectrumAnalyzer::new();
        let metrics = StreamMetrics::new(
            1,
            0,
            0,
            "test".into(),
            "127.0.0.1:1".into(),
            crate::protocol::Codec::RawF32,
            100.0,
        );
        let tone_hz = 1000.0f32;
        for packet in 0..40 {
            let mut samples = Vec::with_capacity(480);
            for i in 0..240 {
                let t = (packet * 240 + i) as f32 / SAMPLE_RATE;
                let v = (std::f32::consts::TAU * tone_hz * t).sin() * 0.25;
                samples.push(v);
                samples.push(v);
            }
            analyzer.push(&samples, &metrics);
        }
        let bands = analyzer.bands();
        let peak = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        // 1 kHz sits near the middle of a 20 Hz..16 kHz log axis.
        assert!((14..=22).contains(&peak), "peak band {peak}");
        assert!(bands[peak] > 0.6, "peak {}", bands[peak]);
    }
}
