//! Shared mixer→device rendering adapter: pulls from the mixer on demand and
//! linearly resamples/maps 48 kHz stereo into whatever the endpoint accepts.

use crate::dsp::Mixer;
use crate::format::{CHANNELS, SAMPLE_RATE};
use cpal::{FromSample, Sample};

pub(crate) struct OutputAdapter {
    mixer: Mixer,
    ratio: f64,
    out_channels: usize,
    pending: Vec<f32>,
    pos: f64,
    scratch: Vec<f32>,
}

impl OutputAdapter {
    pub(crate) fn new(mixer: Mixer, out_rate: u32, out_channels: usize) -> Self {
        Self {
            mixer,
            ratio: SAMPLE_RATE as f64 / out_rate.max(1) as f64,
            out_channels: out_channels.max(1),
            pending: Vec::with_capacity(16_384),
            pos: 0.0,
            scratch: vec![0.0; 512 * CHANNELS],
        }
    }

    /// Re-target after the endpoint's actual rate/channels are known.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(crate) fn retune(&mut self, rate: u32, channels: usize) {
        self.ratio = SAMPLE_RATE as f64 / rate.max(1) as f64;
        self.out_channels = channels.max(1);
        self.pending.clear();
        self.pos = 0.0;
    }

    fn ensure(&mut self, frames: usize) {
        let need = (self.pos + frames as f64 * self.ratio).ceil() as usize + 2;
        while self.pending.len() / CHANNELS < need {
            let available = self.pending.len() / CHANNELS;
            let want = (need - available).min(self.scratch.len() / CHANNELS);
            let slice = &mut self.scratch[..want * CHANNELS];
            self.mixer.render(slice);
            crate::dsp::echo::record(slice);
            self.pending.extend_from_slice(slice);
        }
    }

    pub(crate) fn write<T>(&mut self, data: &mut [T])
    where
        T: Sample + FromSample<f32>,
    {
        let out_frames = data.len() / self.out_channels;
        if out_frames == 0 {
            return;
        }
        self.ensure(out_frames);

        for i in 0..out_frames {
            let idx = self.pos.floor() as usize;
            let frac = (self.pos - idx as f64) as f32;
            let left = lerp(
                self.pending[idx * CHANNELS],
                self.pending[(idx + 1) * CHANNELS],
                frac,
            );
            let right = lerp(
                self.pending[idx * CHANNELS + 1],
                self.pending[(idx + 1) * CHANNELS + 1],
                frac,
            );
            let frame = &mut data[i * self.out_channels..(i + 1) * self.out_channels];
            if self.out_channels == 1 {
                frame[0] = T::from_sample((left + right) * 0.5);
            } else {
                frame[0] = T::from_sample(left);
                frame[1] = T::from_sample(right);
                for sample in frame[2..].iter_mut() {
                    *sample = T::from_sample(0.0f32);
                }
            }
            self.pos += self.ratio;
        }

        let consumed = (self.pos.floor() as usize).saturating_sub(1);
        let available = self.pending.len() / CHANNELS;
        if consumed >= available {
            self.pending.clear();
            self.pos = 0.0;
        } else if consumed > 0 {
            self.pending.copy_within(consumed * CHANNELS.., 0);
            let keep = (available - consumed) * CHANNELS;
            self.pending.truncate(keep);
            self.pos -= consumed as f64;
        }
    }
}

#[inline]
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
