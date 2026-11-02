//! Self-echo removal for system capture.
//!
//! Loopback capture (WASAPI loopback, Android AudioPlaybackCapture) records
//! everything the endpoint is playing — including the mesh monitor output this
//! node produced. Without correction that monitor audio is re-sent to the
//! other nodes, which then hear themselves: a feedback loop.
//!
//! Three stages, because a single-shot gain subtraction leaked whole echo
//! blocks whenever the lock drifted, and those leaks compounded into runaway
//! feedback and clipping:
//!
//! 1. Delay lock: normalized cross-correlation over the last second finds
//!    where our monitor appears in the capture (coarse 1 ms, then 1-sample
//!    refinement). Re-checked periodically and retried quickly after failures.
//! 2. Per-channel NLMS adaptive filter on the aligned reference absorbs gain
//!    differences, resampler rounding and small jitter, which a scalar gain
//!    could not. Adaptation freezes unless the capture is echo dominated.
//! 3. Fail-safe: once our echo has been *confirmed* in this capture, losing
//!    the lock mutes outgoing audio (and poor cancellation attenuates it)
//!    instead of passing the echo through. Captures that never contained our
//!    echo (no loopback of the monitor, other endpoint) keep passing audio.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::format::{CHANNELS, SAMPLE_RATE};

/// Three seconds of history covers any loopback latency.
const RING_FRAMES: usize = SAMPLE_RATE as usize * 3;

struct Inner {
    data: Vec<f32>,
    head: usize,
    frames: u64,
}

fn monitor() -> &'static Mutex<Inner> {
    static MONITOR: OnceLock<Mutex<Inner>> = OnceLock::new();
    MONITOR.get_or_init(|| {
        Mutex::new(Inner {
            data: vec![0.0; RING_FRAMES * CHANNELS],
            head: 0,
            frames: 0,
        })
    })
}

static REFERENCE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables reference recording while a loopback capture is active.
pub fn set_reference_enabled(enabled: bool) {
    REFERENCE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn reference_enabled() -> bool {
    REFERENCE_ENABLED.load(Ordering::Relaxed)
}

/// Called from the render path with the exact 48 kHz stereo frames that were
/// pulled from the monitor mixer.
pub fn record(samples: &[f32]) {
    if !reference_enabled() {
        return;
    }
    let Ok(mut inner) = monitor().lock() else {
        return;
    };
    for frame in samples.chunks_exact(CHANNELS) {
        let slot = inner.head * CHANNELS;
        inner.data[slot] = frame[0];
        inner.data[slot + 1] = frame[1];
        inner.head = (inner.head + 1) % RING_FRAMES;
        inner.frames += 1;
    }
}

fn copy_frames(inner: &Inner, start_frame: u64, frames: usize, out: &mut Vec<f32>) -> bool {
    let oldest = inner.frames.saturating_sub(RING_FRAMES as u64);
    let end = start_frame + frames as u64;
    if start_frame < oldest || end > inner.frames {
        return false;
    }
    out.clear();
    out.reserve(frames * CHANNELS);
    for index in start_frame..end {
        let slot = (index % RING_FRAMES as u64) as usize * CHANNELS;
        out.push(inner.data[slot]);
        out.push(inner.data[slot + 1]);
    }
    true
}

const LOCK_WINDOW: usize = 4096;
const LOCK_STEP_COARSE: usize = 48; // 1 ms
const MAX_LAG_FRAMES: usize = SAMPLE_RATE as usize; // 1 s
const RELOCK_PERIOD: u64 = SAMPLE_RATE as u64 * 10;
const RELOCK_RETRY: u64 = SAMPLE_RATE as u64 / 2;
const MIN_CAPTURE_RMS: f32 = 1e-3;
const MIN_CORRELATION: f32 = 0.25;
/// An existing lock this good is kept; relocking could jump to a repeated
/// peak in periodic content (music), which then cancels nothing.
const KEEP_LOCK_SCORE: f32 = 0.6;
/// Re-lock search window around the previous offset (±20 ms).
const LOCAL_SEARCH_FRAMES: i64 = 960;
/// On the first lock, prefer the earliest peak within this score of the best:
/// later peaks are repeats of periodic audio, not the direct path.
const PEAK_TOLERANCE: f32 = 0.05;

/// NLMS taps (~5.3 ms) absorbing gain, jitter and resampler rounding.
const FILTER_TAPS: usize = 256;
const MU_START: f32 = 0.35;
const MU_FLOOR: f32 = 0.03;
const MU_DECAY_BLOCKS: f32 = 2000.0;

/// Consecutive poorly-cancelled blocks before the fail-safe mutes output.
const POOR_STREAK_LIMIT: u32 = 40; // ~200 ms at 5 ms blocks
/// Consecutive blocks without echo correlation before re-acquiring the lock.
/// Playback underruns insert silence into the monitor and break alignment;
/// waiting for the periodic re-lock would leak/mute for seconds.
const LOST_STREAK_LIMIT: u32 = 12; // ~60 ms at 5 ms blocks
/// Cancellation ratio (out/in) below which the echo is considered handled.
const ERLE_GOOD: f32 = 0.5;

struct Nlms {
    taps_l: Vec<f32>,
    taps_r: Vec<f32>,
    hist_l: Vec<f32>,
    hist_r: Vec<f32>,
    position: usize,
    updates: f32,
}

impl Nlms {
    fn new() -> Self {
        Self {
            taps_l: vec![0.0; FILTER_TAPS],
            taps_r: vec![0.0; FILTER_TAPS],
            hist_l: vec![0.0; FILTER_TAPS],
            hist_r: vec![0.0; FILTER_TAPS],
            position: 0,
            updates: 0.0,
        }
    }

    fn reset(&mut self) {
        self.taps_l.iter_mut().for_each(|tap| *tap = 0.0);
        self.taps_r.iter_mut().for_each(|tap| *tap = 0.0);
        self.hist_l.iter_mut().for_each(|tap| *tap = 0.0);
        self.hist_r.iter_mut().for_each(|tap| *tap = 0.0);
        self.position = 0;
        self.updates = 0.0;
    }

    fn mu(&self) -> f32 {
        let decay = (1.0 - self.updates / MU_DECAY_BLOCKS).max(0.0);
        MU_FLOOR + (MU_START - MU_FLOOR) * decay
    }

    fn estimate(taps: &[f32], hist: &[f32], position: usize) -> f32 {
        let mut acc = 0.0f32;
        let mut index = position;
        for tap in taps.iter() {
            index = if index == 0 {
                FILTER_TAPS - 1
            } else {
                index - 1
            };
            acc += tap * hist[index];
        }
        acc
    }

    fn push(&mut self, sample_l: f32, sample_r: f32) {
        self.hist_l[self.position] = sample_l;
        self.hist_r[self.position] = sample_r;
        self.position = (self.position + 1) % FILTER_TAPS;
    }

    fn adapt_one(&mut self, desired_l: f32, desired_r: f32, reference_l: f32, reference_r: f32) {
        let mu = self.mu();
        self.push(reference_l, reference_r);
        let estimate_l = Self::estimate(&self.taps_l, &self.hist_l, self.position);
        let estimate_r = Self::estimate(&self.taps_r, &self.hist_r, self.position);
        let error_l = desired_l - estimate_l;
        let error_r = desired_r - estimate_r;

        let power = self
            .hist_l
            .iter()
            .zip(self.hist_r.iter())
            .map(|(l, r)| l * l + r * r)
            .sum::<f32>()
            .max(1e-6);
        let step = mu / power;
        let mut index = self.position;
        for tap in 0..FILTER_TAPS {
            index = if index == 0 {
                FILTER_TAPS - 1
            } else {
                index - 1
            };
            self.taps_l[tap] += step * error_l * self.hist_l[index];
            self.taps_r[tap] += step * error_r * self.hist_r[index];
        }
        self.updates += 1.0;
    }

    /// Advances the filter history without estimating or adapting. Used when
    /// the block carries no echo correlation, so the next active block stays
    /// aligned while skipping the per-sample convolution.
    fn push_block(&mut self, reference: &[f32]) {
        for frame in reference.chunks_exact(CHANNELS) {
            self.hist_l[self.position] = frame[0];
            self.hist_r[self.position] = frame[1];
            self.position = (self.position + 1) % FILTER_TAPS;
        }
    }

    fn filter_block(&mut self, block: &mut [f32], reference: &[f32], adapt: bool) {
        let frames = block.len() / CHANNELS;
        for index in 0..frames {
            let base = index * CHANNELS;
            let reference_l = reference[base];
            let reference_r = reference[base + 1];
            if adapt {
                self.adapt_one(block[base], block[base + 1], reference_l, reference_r);
            } else {
                self.push(reference_l, reference_r);
            }
            let estimate_l = Self::estimate(&self.taps_l, &self.hist_l, self.position);
            let estimate_r = Self::estimate(&self.taps_r, &self.hist_r, self.position);
            block[base] -= estimate_l;
            block[base + 1] -= estimate_r;
        }
    }
}

/// Cancellation state for one capture stream.
pub struct EchoCanceller {
    /// Frames fed through `process` so far.
    captured: u64,
    /// Reference frame = capture frame + offset.
    offset: Option<i64>,
    lag_ms: f32,
    relock_in: u64,
    filter: Nlms,
    /// Our echo has been observed in this capture stream before.
    echo_confirmed: bool,
    /// Consecutive blocks where echo was expected but poorly cancelled.
    poor_streak: u32,
    /// Consecutive blocks without correlation at the locked offset.
    lost_streak: u32,
    /// Last alignment that produced a good lock, used as the anchor when the
    /// current lock is invalidated.
    last_good_offset: Option<i64>,
    scratch_ref: Vec<f32>,
    /// Rolling raw-capture tail used for correlation locking.
    lock_buf: Vec<f32>,
}

impl Default for EchoCanceller {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoCanceller {
    pub fn new() -> Self {
        Self {
            captured: 0,
            offset: None,
            lag_ms: 0.0,
            relock_in: 0,
            filter: Nlms::new(),
            echo_confirmed: false,
            poor_streak: 0,
            lost_streak: 0,
            last_good_offset: None,
            scratch_ref: Vec::new(),
            lock_buf: Vec::new(),
        }
    }

    /// Removes this node's own monitor audio from a captured block in place.
    pub fn process(&mut self, block: &mut [f32]) {
        let frames = block.len() / CHANNELS;
        if frames == 0 {
            return;
        }
        self.captured += frames as u64;
        let start = self.captured - frames as u64;

        // Keep a rolling tail of the raw capture for correlation locking.
        self.lock_buf.extend_from_slice(block);
        let max_len = LOCK_WINDOW * CHANNELS;
        if self.lock_buf.len() > max_len {
            let excess = self.lock_buf.len() - max_len;
            self.lock_buf.drain(0..excess);
        }

        if self.offset.is_none() || self.relock_in == 0 {
            self.relock_in = RELOCK_PERIOD;
            self.try_lock();
            if self.offset.is_none() {
                // Retry quickly: a wrong lock is much worse than a brief mute.
                self.relock_in = RELOCK_RETRY.min(self.relock_in);
            }
        } else {
            self.relock_in = self.relock_in.saturating_sub(frames as u64);
        }

        let Some(offset) = self.offset else {
            if self.echo_confirmed {
                // Fail-safe: our echo is known to be in this capture but the
                // alignment is lost — send silence, never the raw echo.
                block.fill(0.0);
            }
            return;
        };
        let ref_start = start as i64 + offset;
        if ref_start < 0 {
            return;
        }

        let mut reference = std::mem::take(&mut self.scratch_ref);
        let copied = {
            let Ok(inner) = monitor().lock() else {
                self.scratch_ref = reference;
                return;
            };
            copy_frames(&inner, ref_start as u64, frames, &mut reference)
        };
        if !copied {
            self.scratch_ref = reference;
            self.offset = None;
            if self.echo_confirmed {
                block.fill(0.0);
            }
            return;
        }

        let correlation_before = normalized_correlation(block, &reference);

        let near_rms = rms(block);
        let far_rms = rms(&reference);

        // Underruns make the monitor insert silence that is not in the
        // reference, so the locked offset stops fitting. Re-acquire quickly
        // instead of waiting for the 10 s relock (and muting in between).
        // Third-party audio lowers the correlation too, but usually not to
        // near zero, so a strict threshold avoids re-locking on every song.
        if self.echo_confirmed && far_rms > MIN_CAPTURE_RMS && correlation_before < 0.05 {
            self.lost_streak += 1;
            if self.lost_streak >= LOST_STREAK_LIMIT {
                log::debug!("echo: lock lost, re-acquiring");
                self.offset = None;
                self.lost_streak = 0;
                self.poor_streak = 0;
                self.relock_in = 0;
            }
        } else {
            self.lost_streak = 0;
        }

        let echo_dominant = correlation_before >= MIN_CORRELATION && far_rms > 1e-4;

        // Cheap path: nothing of ours in this block (silence, third-party
        // audio under L1, or a muted monitor). Keep the filter timeline fresh
        // but skip adaptation/subtraction — the convolution was the main CPU
        // cost and could starve the audio callback on phones.
        if !echo_dominant && !self.echo_confirmed {
            self.filter.push_block(&reference);
            self.poor_streak = 0;
            self.lost_streak = 0;
            self.scratch_ref = reference;
            return;
        }

        self.filter.filter_block(block, &reference, echo_dominant);

        let output_rms = rms(block);
        let erle = if near_rms > 1e-5 {
            (output_rms / near_rms).min(4.0)
        } else {
            0.0
        };
        let correlation_after = normalized_correlation(block, &reference);

        // Residual gate: the echo is still clearly present after subtraction,
        // so the filter has not converged for this segment — attenuate hard
        // instead of leaking it upstream.
        if far_rms > 1e-4 && correlation_after >= 0.35 && erle > 0.35 {
            for sample in block.iter_mut() {
                *sample *= 0.3;
            }
        }

        if echo_dominant {
            self.echo_confirmed = true;
            if erle < ERLE_GOOD {
                self.poor_streak = 0;
            } else {
                // The echo is clearly present but the filter did not remove
                // it: leak control, mute rather than send the echo upstream.
                self.poor_streak += 1;
                if self.poor_streak == POOR_STREAK_LIMIT {
                    log::warn!(
                        "echo: cancellation stalled (corr {:.2}->{:.2}, erle {:.2}, near {:.4}, far {:.4}, lag {:.1} ms), muting outgoing capture",
                        correlation_before,
                        correlation_after,
                        erle,
                        near_rms,
                        far_rms,
                        self.lag_ms
                    );
                }
                if self.poor_streak >= POOR_STREAK_LIMIT {
                    block.fill(0.0);
                }
            }
        } else {
            // No echo correlation in this block (third-party audio, silence):
            // pass it through — muting would drop audio we must send.
            self.poor_streak = 0;
        }
        self.scratch_ref = reference;
    }

    fn try_lock(&mut self) {
        if self.lock_buf.len() < LOCK_WINDOW * CHANNELS {
            return;
        }
        if rms(&self.lock_buf) < MIN_CAPTURE_RMS {
            return;
        }
        let Ok(inner) = monitor().lock() else {
            return;
        };
        let monitor_now = inner.frames;
        let capture_now = self.captured;
        if monitor_now < LOCK_WINDOW as u64 {
            return;
        }
        // Skip the expensive 1 s lag scan while the local monitor is silent:
        // there is nothing that could have been echoed back.
        if ring_rms(&inner, LOCK_WINDOW) < MIN_CAPTURE_RMS {
            return;
        }

        // A healthy lock is kept. Re-searching the whole 0..1 s window on
        // periodic content (music) can land on a repeated peak and destroy
        // cancellation even though the current alignment is still right.
        if let Some(previous) = self.offset.or(self.last_good_offset) {
            if let Some(score) = self.score_offset(&inner, previous) {
                if score >= KEEP_LOCK_SCORE {
                    self.offset = Some(previous);
                    self.last_good_offset = Some(previous);
                    self.relock_in = RELOCK_PERIOD;
                    return;
                }
            }
            // The delay moved: track it locally first (±20 ms).
            if let Some((score, offset)) = self.search_offsets(
                &inner,
                previous,
                LOCAL_SEARCH_FRAMES,
                LOCK_STEP_COARSE as i64,
            ) {
                if score >= MIN_CORRELATION {
                    self.apply_lock(offset, score, monitor_now, capture_now, &inner);
                    return;
                }
            }
        }

        // First lock (or a large jump): ref_frame = capture_frame + offset;
        // for a candidate lag L, offset = monitor_now - capture_now - L.
        let mut scores: Vec<(i64, f32)> = Vec::new();
        let mut lag = 0usize;
        while lag <= MAX_LAG_FRAMES {
            let offset = (monitor_now as i64) - (capture_now as i64) - (lag as i64);
            if let Some(score) = self.score_offset(&inner, offset) {
                scores.push((offset, score));
            }
            lag += LOCK_STEP_COARSE;
        }
        let best = scores
            .iter()
            .map(|(_, score)| *score)
            .fold(f32::MIN, f32::max);
        if best < MIN_CORRELATION {
            return;
        }
        // Scores are in increasing lag order: the earliest strong peak is the
        // direct path, later ones are repeats of periodic content.
        let threshold = best - PEAK_TOLERANCE;
        let Some(&(offset, score)) = scores.iter().find(|(_, score)| *score >= threshold) else {
            return;
        };
        self.apply_lock(offset, score, monitor_now, capture_now, &inner);
    }

    fn search_offsets(
        &mut self,
        inner: &Inner,
        base: i64,
        range: i64,
        step: i64,
    ) -> Option<(f32, i64)> {
        let mut best: Option<(f32, i64)> = None;
        let mut delta = -range;
        while delta <= range {
            let offset = base + delta;
            if let Some(score) = self.score_offset(inner, offset) {
                if best.map_or(true, |(best_score, _)| score > best_score) {
                    best = Some((score, offset));
                }
            }
            delta += step;
        }
        best
    }

    fn apply_lock(
        &mut self,
        offset: i64,
        score: f32,
        monitor_now: u64,
        capture_now: u64,
        inner: &Inner,
    ) {
        let mut best_offset = offset;
        let mut best_score = score;
        for delta in -(LOCK_STEP_COARSE as i64)..=LOCK_STEP_COARSE as i64 {
            if let Some(fine) = self.score_offset(inner, offset + delta) {
                if fine > best_score {
                    best_score = fine;
                    best_offset = offset + delta;
                }
            }
        }

        self.lag_ms = ((monitor_now as i64 - capture_now as i64 - best_offset) as f32)
            / SAMPLE_RATE as f32
            * 1000.0;
        if self.offset != Some(best_offset) {
            log::info!(
                "echo: locked lag={:.1} ms score={:.2}",
                self.lag_ms,
                best_score
            );
            // A different alignment invalidates what the filter learned.
            self.filter.reset();
            self.poor_streak = 0;
        }
        self.offset = Some(best_offset);
        self.last_good_offset = Some(best_offset);
    }

    fn score_offset(&mut self, inner: &Inner, offset: i64) -> Option<f32> {
        let frames = self.lock_buf.len() / CHANNELS;
        if frames < LOCK_WINDOW {
            return None;
        }
        let start = self.captured - frames as u64;
        let ref_start = start as i64 + offset;
        if ref_start < 0 {
            return None;
        }
        let mut scratch = std::mem::take(&mut self.scratch_ref);
        let copied = copy_frames(inner, ref_start as u64, frames, &mut scratch);
        self.scratch_ref = scratch;
        if !copied {
            return None;
        }
        Some(normalized_correlation(&self.lock_buf, &self.scratch_ref))
    }
}

fn ring_rms(inner: &Inner, frames: usize) -> f32 {
    if inner.frames < frames as u64 {
        return 0.0;
    }
    let start = inner.frames - frames as u64;
    let mut sum = 0.0f64;
    for index in start..inner.frames {
        let slot = (index % RING_FRAMES as u64) as usize * CHANNELS;
        let left = inner.data[slot] as f64;
        let right = inner.data[slot + 1] as f64;
        sum += left * left + right * right;
    }
    (sum / (frames * CHANNELS) as f64).sqrt() as f32
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

fn normalized_correlation(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }
    let mut ab = 0.0f64;
    let mut aa = 0.0f64;
    let mut bb = 0.0f64;
    for index in 0..len {
        ab += (a[index] as f64) * (b[index] as f64);
        aa += (a[index] as f64) * (a[index] as f64);
        bb += (b[index] as f64) * (b[index] as f64);
    }
    let den = (aa * bb).sqrt();
    if den <= 1e-12 {
        0.0
    } else {
        (ab / den) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The monitor ring is process-global; tests must not interleave.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn tone_block(phase: &mut f64, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * CHANNELS);
        for _ in 0..frames {
            let value =
                ((*phase * std::f64::consts::TAU * 440.0 / SAMPLE_RATE as f64).sin() * 0.3) as f32;
            *phase += 1.0;
            out.push(value);
            out.push(value);
        }
        out
    }

    fn noise_block(seed: &mut u64, frames: usize, level: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * CHANNELS);
        for _ in 0..frames {
            for _ in 0..CHANNELS {
                *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let value = ((*seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 2.0 * level;
                out.push(value);
            }
        }
        out
    }

    fn energy(samples: &[f32]) -> f64 {
        samples.iter().map(|s| (*s as f64) * (*s as f64)).sum()
    }

    #[test]
    fn cancels_delayed_self_echo() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_reference_enabled(true);
        let mut canceller = EchoCanceller::new();
        let frames = 480;
        let delay_blocks = 37; // 370 ms of loop latency
        let mut history = std::collections::VecDeque::new();
        let mut phase = 0.0f64;
        let mut seed = 42u64;
        let total = 260;

        let mut baseline_energy = 0.0f64;
        let mut residual_energy = 0.0f64;
        for block_index in 0..total {
            let clean = tone_block(&mut phase, frames);
            // Unrelated app audio in the capture only.
            let noise = noise_block(&mut seed, frames, 0.02);
            record(&clean);
            history.push_back(clean.clone());
            if history.len() <= delay_blocks {
                continue;
            }
            let echoed = history.pop_front().unwrap();
            let mut capture: Vec<f32> = echoed
                .iter()
                .zip(noise.iter())
                .map(|(echo, noise)| echo + noise)
                .collect();
            let before = energy(&capture);
            canceller.process(&mut capture);
            if block_index > total - 40 {
                baseline_energy += before;
                residual_energy += energy(&capture);
            }
        }
        set_reference_enabled(false);
        let reduction_db = 10.0 * (baseline_energy / residual_energy.max(1e-12)).log10();
        assert!(
            reduction_db > 15.0,
            "self echo reduced by {reduction_db:.1} dB only"
        );
    }

    #[test]
    fn adapts_to_gain_and_jitter() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_reference_enabled(true);
        let mut canceller = EchoCanceller::new();
        let frames = 480;
        let total = 360;
        let delay = 37 * frames; // 370 ms of loop latency
        let mut phase = 0.0f64;
        let mut seed = 7u64;
        // Continuous monitor timeline: jitter moves the echo by samples, not
        // by whole blocks, which is what a real resampler/drift does.
        let mut monitor: Vec<f32> = Vec::new();
        let mut baseline_energy = 0.0f64;
        let mut residual_energy = 0.0f64;
        for block_index in 0..total {
            let clean = tone_block(&mut phase, frames);
            record(&clean);
            monitor.extend_from_slice(&clean);
            // ±8 frames, switching every 40 blocks (~0.4 s).
            let jitter = ((block_index / 40) as i64 % 3 - 1) * 8;
            let offset = delay as i64 + jitter;
            if offset <= 0 || monitor.len() / CHANNELS <= offset as usize {
                continue;
            }
            let start = monitor.len() / CHANNELS - offset as usize;
            let echoed = &monitor[start * CHANNELS..(start + frames) * CHANNELS];
            let noise = noise_block(&mut seed, frames, 0.01);
            let mut capture: Vec<f32> = echoed
                .iter()
                .zip(noise.iter())
                .map(|(echo, noise)| echo * 0.7 + noise)
                .collect();
            let before = energy(&capture);
            canceller.process(&mut capture);
            if block_index >= total - 40 {
                baseline_energy += before;
                residual_energy += energy(&capture);
            }
        }
        set_reference_enabled(false);
        let reduction_db = 10.0 * (baseline_energy / residual_energy.max(1e-12)).log10();
        assert!(
            reduction_db > 10.0,
            "echo with gain+jitter reduced by {reduction_db:.1} dB only"
        );
    }

    #[test]
    fn never_touches_capture_without_reference() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_reference_enabled(true);
        let mut canceller = EchoCanceller::new();
        let frames = 480;
        let mut seed = 99u64;
        for _ in 0..120 {
            // Reference recording never happens (monitor silent); unrelated
            // capture content must pass through untouched.
            let mut capture = noise_block(&mut seed, frames, 0.1);
            let before = capture.clone();
            canceller.process(&mut capture);
            assert_eq!(
                capture, before,
                "capture without any reference must pass through unchanged"
            );
        }
        set_reference_enabled(false);
    }

    #[test]
    fn mutes_after_confirmed_echo_loses_lock() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_reference_enabled(true);
        let mut canceller = EchoCanceller::new();
        let frames = 480;
        let mut phase = 0.0f64;
        let mut history = std::collections::VecDeque::new();
        let total = 120;

        // Phase 1: delayed echo confirms and locks.
        for _ in 0..total {
            let clean = tone_block(&mut phase, frames);
            record(&clean);
            history.push_back(clean.clone());
            if history.len() <= 37 {
                continue;
            }
            let echoed = history.pop_front().unwrap();
            let mut capture = echoed.clone();
            canceller.process(&mut capture);
        }
        assert!(canceller.echo_confirmed, "echo should be confirmed");

        // Phase 2: the reference disappears (monitor stopped recording) while
        // echo still arrives. The fail-safe must mute, not pass raw echo.
        set_reference_enabled(false);
        for _ in 0..80 {
            let mut capture = tone_block(&mut phase, frames);
            canceller.process(&mut capture);
            if capture.iter().all(|sample| *sample == 0.0) {
                return;
            }
        }
        panic!("fail-safe never muted after losing the lock with confirmed echo");
    }
}
