//! Per-stream clock-drift correction.
//!
//! Every remote sender has its own crystal. A sender running at 48 001 Hz
//! slowly fills the center's ring buffer while a 47 999 Hz sender drains it.
//! A PI controller watches the fill level and nudges a fractional resampling
//! ratio (bounded to ±0.2 %, inaudible on music) so the buffer parks at the
//! configured target. Interpolation is 4-point cubic Hermite.

use rtrb::Consumer;

use crate::format::{frames_for_ms, ms_for_frames, CHANNELS, SAMPLE_RATE};

const MAX_BUFFER_FRAMES: usize = 8192;
const START_FRACTION: f64 = 1.0;
/// Smoothing time constant for the fill-level estimate. Packet arrival jitter
/// must not turn into audible pitch wobble.
const FILL_SMOOTHING_S: f64 = 0.3;
/// If the buffered audio exceeds this multiple of the target (e.g. after a
/// long OS stall), drop the excess instead of waiting minutes for the ±0.05 %
/// drift correction to drain it.
const MAX_FILL_RATIO: f64 = 2.0;
const FILL_HEADROOM_MS: f64 = 10.0;

/// Buffer level above which the mixer drops the oldest audio to pull latency
/// back to target. The hot path and the UIs share this so the ceiling a panel
/// shows is the ceiling that will actually fire.
pub fn fill_ceiling_frames(target_frames: f64) -> f64 {
    target_frames * MAX_FILL_RATIO + frames_for_ms(FILL_HEADROOM_MS as f32) as f64
}

pub fn fill_ceiling_ms(target_ms: f32) -> f32 {
    ms_for_frames(fill_ceiling_frames(frames_for_ms(target_ms) as f64) as usize)
}

/// The integral term leaks so a startup overshoot cannot keep the buffer
/// draining for a minute; the steady-state error stays far below 1 ms.
const INTEGRAL_LEAK_PER_S: f64 = 0.2;

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderReport {
    /// Peak of this stream's contribution after gain, before the master bus.
    pub peak: f32,
    /// RMS of this stream's contribution after gain.
    pub rms: f32,
    pub fill_frames: u32,
    pub ratio: f32,
    pub underran: bool,
    pub missing_frames: u32,
    pub started: bool,
    /// The buffer was far over target and was trimmed back.
    pub buffer_reset: bool,
    /// Frames available *before* that trim, so the log can show how much was
    /// thrown away (I1). Zero when no trim happened.
    pub pre_trim_frames: u32,
}

pub struct DriftResampler {
    target_frames: f64,
    kp: f64,
    ki: f64,
    max_correction: f64,
    integral: f64,
    ratio: f64,
    fill_ema: f64,
    pos: f64,
    buf: Vec<f32>,
    len_frames: usize,
    started: bool,
    /// Frames of output fade-in still owed after a trim (MIX1/F8).
    ramp_left: usize,
    /// E1: last trim instant (rate limiting) and the previous fill reading
    /// (flood detection: defer trimming while the buffer still grows).
    last_trim: Option<std::time::Instant>,
    last_available: usize,
}

impl DriftResampler {
    pub fn new(target_ms: f32) -> Self {
        Self {
            target_frames: frames_for_ms(target_ms).max(1) as f64,
            kp: 0.02,
            ki: 0.005,
            // 0.2 % = ~3.4 cents, still inaudible on music, but large enough to
            // follow software-paced senders and real crystal mismatch.
            max_correction: 0.002,
            integral: 0.0,
            ratio: 1.0,
            fill_ema: frames_for_ms(target_ms).max(1) as f64,
            pos: 1.0,
            buf: vec![0.0; MAX_BUFFER_FRAMES * CHANNELS],
            len_frames: 0,
            started: false,
            ramp_left: 0,
            last_trim: None,
            last_available: 0,
        }
    }

    /// Drop every buffered sample and restart from silence. Used when the
    /// receiver switches carriers and the two timelines cannot be spliced
    /// (a relayed Bluetooth copy can lag the Wi-Fi copy by seconds).
    pub fn reset(&mut self) {
        self.integral = 0.0;
        self.ratio = 1.0;
        self.fill_ema = self.target_frames;
        self.pos = 1.0;
        self.len_frames = 0;
        self.started = false;
    }

    pub fn set_target_ms(&mut self, ms: f32) {
        self.target_frames = frames_for_ms(ms).max(1) as f64;
    }

    pub fn target_ms(&self) -> f32 {
        ms_for_frames(self.target_frames as usize)
    }

    pub fn ratio(&self) -> f32 {
        self.ratio as f32
    }

    pub fn is_started(&self) -> bool {
        self.started
    }

    /// Mix this stream into `out` (already zeroed by the mixer).
    pub fn render(
        &mut self,
        rx: &mut Consumer<f32>,
        out_frames: usize,
        gain: f32,
        out: &mut [f32],
    ) -> RenderReport {
        let out_frames = out_frames.min(out.len() / CHANNELS);
        if out_frames == 0 {
            return RenderReport::default();
        }

        let available = self.len_frames + rx.slots() / CHANNELS;
        let reset_threshold = fill_ceiling_frames(self.target_frames);
        let mut trimmed = false;
        let mut pre_trim_frames = 0u32;
        // E1/A4: rate-limit the shave — one burst on the phone produced three
        // trims in 80 ms with a single 751.6 ms drop. At most 60 ms per trim,
        // at least 200 ms apart, and defer while the backlog is still
        // growing. An extreme backlog (4x the ceiling) still drops straight
        // to target: protecting the ring beats latency smoothing.
        let still_flooding = available > self.last_available + 40;
        self.last_available = available;
        let emergency = available as f64 > reset_threshold * 4.0;
        let cooling = self
            .last_trim
            .map(|at| at.elapsed() < std::time::Duration::from_millis(200))
            .unwrap_or(false);
        let trim_due = emergency || (!cooling && !still_flooding);
        if self.started && available as f64 > reset_threshold && trim_due {
            // Drop the oldest audio that is above the trim ceiling. This only
            // happens after a long stall; the alternative is minutes of added
            // latency.
            //
            // MIX1 (#64): the shave must stop *at the ceiling*, not at the
            // target. Shaving 60 ms every 240 ms walked a 573 ms backlog down
            // toward 250 ms, and the measured ~458 ms Bluetooth stall then
            // drained the rest into digital silence: one 3-minute leaf window
            // shed 13.56 s of audio to trims and printed 15.27 s of `mix`
            // zero-runs. The level between target and ceiling is the cushion
            // that survives an arrival stall, not waste.
            //
            // The trim must keep rendering: the old path returned here, and
            // because the mixer zeroes the output buffer first, every trim
            // cost a whole block of digital silence — measured as 15–195 ms
            // holes that no underrun, loss or overflow counter reported.
            let floor = if emergency {
                self.target_frames as usize
            } else {
                reset_threshold as usize
            };
            let excess_raw = available.saturating_sub(floor);
            let excess = if emergency {
                excess_raw
            } else {
                excess_raw.min(frames_for_ms(60.0))
            };
            let head = self.pos.floor() as usize;
            let frac = self.pos - head as f64;
            let unplayed = self.len_frames.saturating_sub(head + 1);
            let drop_local = excess.min(unplayed);
            if drop_local > 0 {
                let from = (head + 1 + drop_local) * CHANNELS;
                let to = self.len_frames * CHANNELS;
                if from < to {
                    self.buf.copy_within(from..to, (head + 1) * CHANNELS);
                }
                self.len_frames -= drop_local;
                self.pos = head as f64 + 1.0 + frac;
            }
            let mut to_drop = excess - drop_local;
            while to_drop > 0 {
                let want = to_drop.min(1024) * CHANNELS;
                let got = {
                    let (popped, _) = rx.pop_partial_slice(&mut self.buf[..want]);
                    popped.len() / CHANNELS
                };
                if got == 0 {
                    break;
                }
                to_drop -= got;
            }
            // MIX1 (#64): stay started with the PI state reset and fade the
            // seam in, so the trim neither re-warms nor clicks.
            self.integral = 0.0;
            self.fill_ema = self.target_frames;
            self.ramp_left = crate::format::RAMP_FRAMES;
            self.last_trim = Some(std::time::Instant::now());
            trimmed = true;
            pre_trim_frames = available as u32;
        }
        // 修剪块要报告修剪之后的水位，否则 UI 会显示一个再也不会出现的缓冲深度。
        let fill_report = if trimmed {
            (self.len_frames + rx.slots() / CHANNELS) as u32
        } else {
            available as u32
        };
        if !self.started {
            if available as f64 >= self.target_frames * START_FRACTION && available >= 4 {
                self.started = true;
                self.pos = 1.0;
                self.integral = 0.0;
                self.ratio = 1.0;
                self.fill_ema = available as f64;
            } else {
                return RenderReport {
                    fill_frames: available as u32,
                    ratio: 1.0,
                    started: false,
                    ..Default::default()
                };
            }
        }

        // PI controller around the smoothed fill level. A positive error means
        // the buffer is too full, so consume slightly faster (ratio > 1).
        let dt = out_frames as f64 / SAMPLE_RATE as f64;
        let alpha = dt / (FILL_SMOOTHING_S + dt);
        self.fill_ema += alpha * (available as f64 - self.fill_ema);
        let error = ((self.fill_ema - self.target_frames) / self.target_frames).clamp(-4.0, 4.0);
        self.integral = (self.integral * (1.0 - INTEGRAL_LEAK_PER_S * dt) + self.ki * error * dt)
            .clamp(-self.max_correction * 0.75, self.max_correction * 0.75);
        let correction =
            (self.kp * error + self.integral).clamp(-self.max_correction, self.max_correction);
        self.ratio = 1.0 + correction;

        let need = (self.pos + out_frames as f64 * self.ratio).ceil() as usize + 3;
        self.fill_to(rx, need);

        if self.len_frames < 4 {
            self.started = false;
            self.pos = 1.0;
            self.integral = 0.0;
            self.fill_ema = self.target_frames;
            return RenderReport {
                fill_frames: (self.len_frames + rx.slots() / CHANNELS) as u32,
                ratio: self.ratio as f32,
                underran: true,
                missing_frames: out_frames as u32,
                started: false,
                buffer_reset: trimmed,
                ..Default::default()
            };
        }

        let mut peak = 0.0f32;
        let mut sumsq = 0.0f64;
        let mut missing_frames = 0u32;

        for i in 0..out_frames {
            let index = self.pos.floor() as usize;
            if index >= 1 && index + 2 < self.len_frames {
                let frac = (self.pos - index as f64) as f32;
                for ch in 0..CHANNELS {
                    let base = index * CHANNELS + ch;
                    let y0 = self.buf[base - CHANNELS];
                    let y1 = self.buf[base];
                    let y2 = self.buf[base + CHANNELS];
                    let y3 = self.buf[base + 2 * CHANNELS];
                    let mut sample = hermite(y0, y1, y2, y3, frac) * gain;
                    if self.ramp_left > 0 {
                        // MIX1/F8: fade the stream back in after a trim.
                        let done = (crate::format::RAMP_FRAMES - self.ramp_left) as f32;
                        sample *= ((done + i as f32 + 1.0)
                            / crate::format::RAMP_FRAMES as f32)
                            .min(1.0);
                    }
                    peak = peak.max(sample.abs());
                    sumsq += (sample as f64) * (sample as f64);
                    out[i * CHANNELS + ch] += sample;
                }
                if self.ramp_left > 0 {
                    self.ramp_left = self.ramp_left.saturating_sub(1);
                }
                self.pos += self.ratio;
            } else {
                missing_frames += 1;
                self.pos += self.ratio;
            }
        }

        let consumed = (self.pos.floor() as i64 - 1).max(0) as usize;
        if consumed >= self.len_frames {
            self.len_frames = 0;
            self.pos = 1.0;
        } else if consumed > 0 {
            let from = consumed * CHANNELS;
            let to = self.len_frames * CHANNELS;
            self.buf.copy_within(from..to, 0);
            self.len_frames -= consumed;
            self.pos -= consumed as f64;
        }

        if self.len_frames == 0 {
            self.started = false;
            self.integral = 0.0;
            self.fill_ema = self.target_frames;
        }

        let samples = (out_frames * CHANNELS) as f64;
        RenderReport {
            peak,
            rms: (sumsq / samples).sqrt() as f32,
            // Report the buffer health seen at the start of this block, before
            // the device callback consumed its share. This is the number the
            // `--target-ms` setting refers to.
            fill_frames: fill_report,
            ratio: self.ratio as f32,
            underran: missing_frames > 0,
            missing_frames,
            started: self.started,
            buffer_reset: trimmed,
            pre_trim_frames,
        }
    }

    fn fill_to(&mut self, rx: &mut Consumer<f32>, need_frames: usize) {
        if self.len_frames >= need_frames {
            return;
        }
        let want = (need_frames - self.len_frames).min(MAX_BUFFER_FRAMES - self.len_frames);
        if want == 0 {
            return;
        }
        let start = self.len_frames * CHANNELS;
        let end = start + want * CHANNELS;
        let got = {
            let (filled, _rest) = rx.pop_partial_slice(&mut self.buf[start..end]);
            filled.len() / CHANNELS
        };
        self.len_frames += got;
    }
}

/// Catmull-Rom style cubic interpolation; `t` in `[0, 1)` between y1 and y2.
#[inline]
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c0 = y1;
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + c0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceiling_is_two_targets_plus_headroom() {
        // Guards the ms/frames composition: feeding ms into the frames helper
        // used to report 11.25 ms for a 30 ms target, below the target itself.
        assert!((fill_ceiling_ms(30.0) - 70.0).abs() < 0.05);
        assert!((fill_ceiling_ms(400.0) - 810.0).abs() < 0.05);
        assert!(fill_ceiling_ms(30.0) > 30.0);
    }

    struct Tone {
        phase: f64,
        step: f64,
        amplitude: f32,
    }

    impl Tone {
        fn new(freq: f64) -> Self {
            Self {
                phase: 0.0,
                step: 2.0 * std::f64::consts::PI * freq / SAMPLE_RATE as f64,
                amplitude: 0.25,
            }
        }

        fn fill(&mut self, pcm: &mut [f32]) {
            for frame in pcm.chunks_exact_mut(CHANNELS) {
                let value = (self.phase.sin() as f32) * self.amplitude;
                frame[0] = value;
                frame[1] = value;
                self.phase = (self.phase + self.step) % (2.0 * std::f64::consts::PI);
            }
        }
    }

    fn run(rate_num: u64, rate_den: u64, seconds: u32) -> (f32, u32, u32, u32, f64) {
        let (mut producer, mut consumer) =
            rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS * 2);
        let target_frames = frames_for_ms(30.0);
        let mut tone = Tone::new(440.0);
        let mut feed = vec![0.0f32; 241 * CHANNELS];

        for _ in 0..6 {
            tone.fill(&mut feed[..240 * CHANNELS]);
            let _ = producer.push_partial_slice(&feed[..240 * CHANNELS]);
        }

        let mut resampler = DriftResampler::new(30.0);
        let mut out = vec![0.0f32; 240 * CHANNELS];
        let mut extra = 0f64;
        let mut underruns = 0u32;
        let mut last_fill = 0u32;
        let mut last_ratio = 1.0f32;
        let mut crossings = 0u32;
        let mut last_sign = false;
        let blocks = seconds as u64 * 200;

        for _ in 0..blocks {
            extra += (rate_num as f64 / rate_den as f64 - 1.0) * 240.0;
            let mut frames = 240usize;
            while extra >= 1.0 {
                frames += 1;
                extra -= 1.0;
            }
            while extra <= -1.0 {
                frames -= 1;
                extra += 1.0;
            }
            tone.fill(&mut feed[..frames * CHANNELS]);
            let _ = producer.push_partial_slice(&feed[..frames * CHANNELS]);

            out.fill(0.0);
            let report = resampler.render(&mut consumer, 240, 1.0, &mut out);
            if report.underran {
                underruns += 1;
            }
            last_fill = report.fill_frames;
            last_ratio = report.ratio;

            for frame in out.chunks_exact(CHANNELS) {
                let sign = frame[0] >= 0.0;
                if sign && !last_sign {
                    crossings += 1;
                }
                last_sign = sign;
            }
        }

        let freq = crossings as f64 / seconds as f64;
        (last_ratio, underruns, last_fill, target_frames as u32, freq)
    }

    #[test]
    fn parks_buffer_and_tracks_fast_clock() {
        // 1 extra input frame per second = ~20.8 ppm fast sender.
        let (ratio, underruns, fill, target, freq) = run(48_001, 48_000, 120);
        assert_eq!(underruns, 0, "unexpected underruns");
        assert!(ratio > 1.000005, "ratio should speed up, got {ratio}");
        assert!(
            ratio < 1.0005,
            "ratio correction should stay tiny, got {ratio}"
        );
        assert!(
            (fill as i64 - target as i64).abs() < 480,
            "fill {fill} should stay near target {target}"
        );
        assert!((freq - 440.0).abs() < 3.0, "pitch drifted: {freq} Hz");
    }

    #[test]
    fn tracks_slow_clock() {
        let (ratio, underruns, fill, target, _freq) = run(47_999, 48_000, 120);
        assert_eq!(underruns, 0);
        assert!(ratio < 0.999995, "ratio should slow down, got {ratio}");
        assert!(ratio > 0.9995);
        assert!((fill as i64 - target as i64).abs() < 480);
    }

    #[test]
    fn trims_are_rate_limited_and_stay_started() {
        // E1/A4: one trim shaves at most 60 ms, trims are >= 200 ms apart, and
        // the stream keeps rendering (no silent block, no re-warm). The backlog
        // stays under 4x the ceiling so this exercises the rate-limited path,
        // not the emergency ring-protection path.
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS);
        let mut resampler = DriftResampler::new(30.0);
        let mut out = vec![0.0f32; 240 * CHANNELS];
        let prefill = vec![0.1f32; frames_for_ms(30.0) * CHANNELS];
        producer.push_entire_slice(&prefill).unwrap();
        let report = resampler.render(&mut consumer, 240, 1.0, &mut out);
        assert!(report.started && !report.buffer_reset);

        let flood = vec![0.1f32; frames_for_ms(150.0) * CHANNELS];
        producer.push_entire_slice(&flood).unwrap();
        // E1/A4: the block that *receives* the burst does not shave — the
        // backlog is still growing, so waiting costs nothing and a shave there
        // would only cut audio the sender has not finished delivering.
        let report = resampler.render(&mut consumer, 240, 1.0, &mut out);
        assert!(
            !report.buffer_reset,
            "trim defers while the backlog is still growing"
        );

        let report = resampler.render(&mut consumer, 240, 1.0, &mut out);
        assert!(report.buffer_reset, "first trim fires above the ceiling");
        assert!(report.started, "trim must not re-warm the stream");
        assert!(
            report.peak > 0.05,
            "the trimmed block must still carry audio"
        );
        assert!(
            ms_for_frames(report.fill_frames as usize) <= 180.0 - 59.0,
            "one trim shaves at most 60 ms: fill {}",
            ms_for_frames(report.fill_frames as usize)
        );

        // Cooldown: the next block must not trim again.
        let report = resampler.render(&mut consumer, 240, 1.0, &mut out);
        assert!(!report.buffer_reset, "200 ms cooldown must hold");

        // MIX1 (#64): repeated trims stop at the ceiling. The level between the
        // ceiling and the target is the cushion that survives the next arrival
        // stall; walking it down to the target is what produced 13.56 s of trims
        // followed by 15.27 s of mix silence in one 3-minute Bluetooth window.
        let ceiling_ms = fill_ceiling_ms(30.0);
        let ceiling = frames_for_ms(ceiling_ms);
        let mut shaves = 0;
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(210));
            let report = resampler.render(&mut consumer, 240, 1.0, &mut out);
            if !report.buffer_reset {
                continue;
            }
            shaves += 1;
            assert!(
                report.fill_frames as f64 + 2.0 * 240.0 >= ceiling as f64,
                "trim went below the ceiling {ceiling_ms:.0} ms: fill {}",
                ms_for_frames(report.fill_frames as usize)
            );
            assert!(report.started && report.peak > 0.05, "audio keeps flowing");
        }
        assert!(shaves >= 1, "the backlog was still above the ceiling");
    }

    #[test]
    fn extreme_backlog_drops_to_target_immediately() {
        // Emergency: 4x the ceiling means the ring is about to overflow;
        // latency smoothing loses to ring protection there.
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS);
        let mut resampler = DriftResampler::new(30.0);
        let mut out = vec![0.0f32; 240 * CHANNELS];
        let prefill = vec![0.1f32; frames_for_ms(30.0) * CHANNELS];
        producer.push_entire_slice(&prefill).unwrap();
        resampler.render(&mut consumer, 240, 1.0, &mut out);
        let flood = vec![0.1f32; frames_for_ms(3000.0) * CHANNELS];
        let _ = producer.push_partial_slice(&flood);
        let report = resampler.render(&mut consumer, 240, 1.0, &mut out);
        assert!(report.buffer_reset);
        assert!(
            report.fill_frames <= frames_for_ms(45.0) as u32,
            "emergency should drop straight to target: {}",
            report.fill_frames
        );
    }

    #[test]
    fn does_not_start_before_prebuffer() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(48_000 * CHANNELS);
        let mut resampler = DriftResampler::new(30.0);
        let mut out = vec![0.0f32; 240 * CHANNELS];
        let feed = vec![0.1f32; 60 * CHANNELS];
        let _ = producer.push_partial_slice(&feed);
        let report = resampler.render(&mut consumer, 240, 1.0, &mut out);
        assert!(!report.started);
        assert!(out.iter().all(|s| *s == 0.0));
        // Fill past 80 % of the target and it starts.
        let bulk = vec![0.1f32; 1_440 * CHANNELS];
        let _ = producer.push_partial_slice(&bulk);
        let report = resampler.render(&mut consumer, 240, 1.0, &mut out);
        assert!(report.started);
        assert!(report.peak > 0.05);
    }
}
