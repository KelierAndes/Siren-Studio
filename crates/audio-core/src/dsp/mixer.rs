//! Multi-track float mixer driven from the output callback (or a recording
//! clock). Channel add/remove/settings travel through a lock-free SPSC queue;
//! per-channel gain and mute are atomics so a UI thread never blocks audio.

use std::sync::atomic::Ordering;
use std::time::Instant;
use std::sync::{Arc, Mutex};

use rtrb::{Consumer, Producer};

use crate::dsp::resampler::DriftResampler;
use crate::format::{ms_for_frames, CHANNELS, SAMPLE_RATE};
use crate::metrics::{db_to_linear, store_f32, MixerMetrics, StreamMetrics};

pub enum MixerCommand {
    Add(Box<StreamChannel>),
    Remove(u64),
    SetTargetMs(f32),
    SetStreamTargetMs(u64, f32),
    Flush(u64),
    SetMasterGainDb(f32),
}

/// One remote stream: its lock-free ring consumer plus drift correction state.
pub struct StreamChannel {
    pub metrics: Arc<StreamMetrics>,
    rx: Consumer<f32>,
    resampler: DriftResampler,
    /// Whether the last render was past the start water level; the false→true
    /// edge is the moment this stream becomes audible again.
    started_last: bool,
    /// Last overflow trim, for the I1 interval field.
    last_trim: Option<Instant>,
}

impl StreamChannel {
    pub fn new(metrics: Arc<StreamMetrics>, rx: Consumer<f32>, target_ms: f32) -> Self {
        Self {
            metrics,
            rx,
            resampler: DriftResampler::new(target_ms),
            started_last: false,
            last_trim: None,
        }
    }

    pub fn id(&self) -> u64 {
        self.metrics.id
    }

    fn flush(&mut self) {
        while self.rx.pop().is_ok() {}
        self.resampler.reset();
        self.started_last = false;
        self.metrics.buffer_frames.store(0, Ordering::Relaxed);
    }

    fn render(&mut self, out: &mut [f32]) {
        let gain_db = self.metrics.gain();
        let muted = self.metrics.muted.load(Ordering::Relaxed);
        let gain = if muted || gain_db <= -60.0 {
            0.0
        } else {
            db_to_linear(gain_db)
        };

        let report = self
            .resampler
            .render(&mut self.rx, out.len() / CHANNELS, gain, out);

        store_f32(&self.metrics.rms, report.rms);
        store_f32(&self.metrics.peak, report.peak);
        self.metrics
            .peak_hold
            .fetch_max(report.peak.to_bits(), Ordering::Relaxed);
        self.metrics
            .buffer_frames
            .store(report.fill_frames, Ordering::Relaxed);
        store_f32(&self.metrics.ratio, report.ratio);
        if report.underran {
            self.metrics.underrun_blocks.fetch_add(1, Ordering::Relaxed);
            self.metrics
                .underrun_frames
                .fetch_add(report.missing_frames, Ordering::Relaxed);
        }
        if report.buffer_reset {
            self.metrics.buffer_resets.fetch_add(1, Ordering::Relaxed);
            let since = self
                .last_trim
                .map(|at| at.elapsed().as_secs_f64() * 1000.0)
                .unwrap_or(f64::NAN);
            self.last_trim = Some(Instant::now());
            log::info!(
                "trace: trim stream={} fill_before_ms={:.1} fill_after_ms={:.1} target_ms={:.0} ms_since_last_trim={:.0} ratio={:.5} path={} dropped_ms={:.1}",
                self.id(),
                ms_for_frames(report.pre_trim_frames as usize),
                ms_for_frames(report.fill_frames as usize),
                f32::from_bits(self.metrics.target_ms.load(Ordering::Relaxed)),
                since,
                report.ratio,
                self.metrics
                    .path
                    .lock()
                    .map(|p| p.clone())
                    .unwrap_or_else(|_| "?".into()),
                ms_for_frames(report.pre_trim_frames as usize)
                    - ms_for_frames(report.fill_frames as usize),
            );
        }
        if report.started && !self.started_last {
            self.metrics.starts.fetch_add(1, Ordering::Relaxed);
        }
        self.started_last = report.started;
    }
}

/// Feed-forward soft-knee limiter with 1 ms lookahead (F4). The old bare
/// `tanh` compressed the whole bus from ~half scale up, dulling normal
/// multi-source mixes; this stays linear below the knee and only pulls gain
/// down on detected overs, acting 1 ms early so the attack is click-free.
struct Limiter {
    /// Pre-gain frames, `LOOKAHEAD_FRAMES` behind the envelope head.
    delay: Vec<f32>,
    pos: usize,
    /// |x| per delayed frame, for the running max over the lookahead window.
    env_ring: Vec<f32>,
    gain: f32,
    primed: usize,
}

const LOOKAHEAD_FRAMES: usize = 48; // 1 ms
/// Gain starts pulling below this envelope (soft knee begins).
const KNEE: f32 = 0.9;
/// Full release from a limited gain back to unity, in frames.
const RELEASE_FRAMES: f32 = SAMPLE_RATE as f32 * 0.05; // 50 ms

impl Limiter {
    fn new() -> Self {
        Self {
            delay: vec![0.0; LOOKAHEAD_FRAMES * CHANNELS],
            pos: 0,
            env_ring: vec![0.0; LOOKAHEAD_FRAMES],
            gain: 1.0,
            primed: 0,
        }
    }

    /// Soft-knee curve: unity below the knee, quadratically easing into the
    /// hard ratio `KNEE/env` at 2x the knee.
    fn knee_gain(env: f32) -> f32 {
        if env <= KNEE {
            1.0
        } else {
            let x = ((env - KNEE) / KNEE).min(1.0);
            let hard = KNEE / env;
            1.0 - (1.0 - hard) * x * x
        }
    }

    /// Apply to one block of interleaved stereo (pre-gain samples already
    /// multiplied by master gain). Replaces the samples in place.
    fn process(&mut self, out: &mut [f32], clipped: &mut u64) {
        let frames = out.len() / CHANNELS;
        let release_step = 1.0 / RELEASE_FRAMES;
        // A fully silent block means nothing is playing: mute must cut
        // immediately, so flush the lookahead tail instead of emitting it.
        if out.iter().all(|s| s.abs() < 1e-9) {
            self.delay.fill(0.0);
            self.env_ring.fill(0.0);
            self.primed = 0;
            return;
        }
        for frame in 0..frames {
            let slice = &out[frame * CHANNELS..(frame + 1) * CHANNELS];
            let mut absmax = 0.0f32;
            let mut over = false;
            for sample in slice {
                let value = *sample;
                absmax = absmax.max(value.abs());
                if value.abs() > 1.0 {
                    over = true;
                }
            }
            if over {
                *clipped += CHANNELS as u64;
            }
            // Delay the pre-gain signal and track its envelope.
            self.delay[self.pos * CHANNELS..(self.pos + 1) * CHANNELS].copy_from_slice(slice);
            self.env_ring[self.pos] = absmax;
            self.pos = (self.pos + 1) % LOOKAHEAD_FRAMES;
            self.primed = (self.primed + 1).min(LOOKAHEAD_FRAMES);
            if self.primed < LOOKAHEAD_FRAMES {
                // First 1 ms: the lookahead ring is priming, so there is no
                // delayed frame to emit — output silence rather than an
                // unprocessed frame.
                for sample in out[frame * CHANNELS..(frame + 1) * CHANNELS].iter_mut() {
                    *sample = 0.0;
                }
                continue;
            }

            // Envelope for the frame leaving the delay = max over the whole
            // lookahead window.
            let env = self.env_ring.iter().copied().fold(0.0f32, f32::max);
            let desired = Self::knee_gain(env);
            if desired < self.gain {
                self.gain = desired; // instant attack: the lookahead bought us time
            } else {
                self.gain = (self.gain + release_step).min(desired);
            }

            let out_frame = &mut out[frame * CHANNELS..(frame + 1) * CHANNELS];
            let delayed = &self.delay[self.pos * CHANNELS..(self.pos + 1) * CHANNELS];
            for (sample, input) in out_frame.iter_mut().zip(delayed) {
                *sample = (*input * self.gain).clamp(-1.0, 1.0);
            }
        }
    }
}

pub struct Mixer {
    slots: Vec<Option<Box<StreamChannel>>>,
    cmd_rx: Consumer<MixerCommand>,
    master_gain: f32,
    limiter: Limiter,
    /// Last target set by the UI. Streams that join later must use it too,
    /// otherwise a target chosen before any peer connected silently reverts
    /// to the startup value and network jitter becomes underruns.
    target_override: Option<f32>,
    /// Whether the previous rendered block came out as exact digital zero; the
    /// true→false edge is one interruption, however many blocks it lasts.
    silent_last: bool,
    /// Start of the current all-zero run, for the I3 log line.
    silent_since: Option<Instant>,
    metrics: Arc<MixerMetrics>,
}

pub struct MixerHandle {
    tx: Producer<MixerCommand>,
    metrics: Arc<MixerMetrics>,
}

/// Returns the audio-thread mixer and the control handle used by the network
/// receiver (or a future FFI/WebSocket front end).
pub fn new_mixer(max_streams: usize) -> (Mixer, MixerHandle) {
    let (tx, cmd_rx) = rtrb::RingBuffer::new(128);
    let metrics = Arc::new(MixerMetrics::new());
    let mixer = Mixer {
        slots: (0..max_streams).map(|_| None).collect(),
        cmd_rx,
        master_gain: 1.0,
        limiter: Limiter::new(),
        target_override: None,
        silent_last: false,
        silent_since: None,
        metrics: metrics.clone(),
    };
    (mixer, MixerHandle { tx, metrics })
}

impl MixerHandle {
    pub fn add(&mut self, channel: StreamChannel) -> bool {
        self.tx.push(MixerCommand::Add(Box::new(channel))).is_ok()
    }

    pub fn remove(&mut self, id: u64) -> bool {
        self.tx.push(MixerCommand::Remove(id)).is_ok()
    }

    pub fn set_target_ms(&mut self, ms: f32) -> bool {
        self.tx.push(MixerCommand::SetTargetMs(ms)).is_ok()
    }

    pub fn set_stream_target_ms(&mut self, id: u64, ms: f32) -> bool {
        self.tx
            .push(MixerCommand::SetStreamTargetMs(id, ms))
            .is_ok()
    }

    /// Discard a stream's queued audio (timeline resync after a carrier
    /// switch).
    pub fn flush_stream(&mut self, id: u64) -> bool {
        self.tx.push(MixerCommand::Flush(id)).is_ok()
    }

    pub fn set_master_gain_db(&mut self, db: f32) -> bool {
        self.tx.push(MixerCommand::SetMasterGainDb(db)).is_ok()
    }

    pub fn metrics(&self) -> Arc<MixerMetrics> {
        self.metrics.clone()
    }
}

/// Cloneable control handle shared by the network receiver and a UI. The audio
/// callback never touches this mutex; it only guards the SPSC command producer.
#[derive(Clone)]
pub struct MixerControl {
    inner: Arc<Mutex<MixerHandle>>,
}

impl MixerControl {
    pub fn new(handle: MixerHandle) -> Self {
        Self {
            inner: Arc::new(Mutex::new(handle)),
        }
    }

    pub fn add(&self, channel: StreamChannel) -> bool {
        self.inner.lock().unwrap().add(channel)
    }

    pub fn remove(&self, id: u64) -> bool {
        self.inner.lock().unwrap().remove(id)
    }

    pub fn set_target_ms(&self, ms: f32) -> bool {
        self.inner.lock().unwrap().set_target_ms(ms)
    }

    /// Per-stream buffer target (e.g. raise for a Bluetooth-only carrier).
    pub fn set_stream_target_ms(&self, id: u64, ms: f32) -> bool {
        self.inner.lock().unwrap().set_stream_target_ms(id, ms)
    }

    pub fn flush_stream(&self, id: u64) -> bool {
        self.inner.lock().unwrap().flush_stream(id)
    }

    pub fn set_master_gain_db(&self, db: f32) -> bool {
        self.inner.lock().unwrap().set_master_gain_db(db)
    }

    pub fn metrics(&self) -> Arc<MixerMetrics> {
        self.inner.lock().unwrap().metrics()
    }
}

impl Mixer {
    /// Fill `out` (interleaved stereo, 48 kHz) with the mix of all live
    /// streams. Must be called from a single audio thread.
    pub fn render(&mut self, out: &mut [f32]) {
        while let Ok(command) = self.cmd_rx.pop() {
            match command {
                MixerCommand::Add(mut channel) => {
                    if let Some(ms) = self.target_override {
                        channel.resampler.set_target_ms(ms);
                        store_f32(&channel.metrics.target_ms, ms);
                    }
                    if let Some(slot) = self.slots.iter_mut().find(|slot| slot.is_none()) {
                        *slot = Some(channel);
                    } else {
                        log::warn!("mixer is full, dropping stream {}", channel.id());
                    }
                }
                MixerCommand::Remove(id) => {
                    if let Some(slot) = self
                        .slots
                        .iter_mut()
                        .find(|slot| slot.as_ref().is_some_and(|c| c.id() == id))
                    {
                        *slot = None;
                    }
                }
                MixerCommand::SetTargetMs(ms) => {
                    self.target_override = Some(ms);
                    for slot in self.slots.iter_mut().flatten() {
                        slot.resampler.set_target_ms(ms);
                        store_f32(&slot.metrics.target_ms, ms);
                    }
                }
                MixerCommand::SetStreamTargetMs(id, ms) => {
                    if let Some(slot) = self
                        .slots
                        .iter_mut()
                        .find(|slot| slot.as_ref().is_some_and(|c| c.id() == id))
                    {
                        if let Some(channel) = slot {
                            channel.resampler.set_target_ms(ms);
                            store_f32(&channel.metrics.target_ms, ms);
                        }
                    }
                }
                MixerCommand::Flush(id) => {
                    if let Some(channel) = self
                        .slots
                        .iter_mut()
                        .flatten()
                        .find(|channel| channel.id() == id)
                    {
                        channel.flush();
                    }
                }
                MixerCommand::SetMasterGainDb(db) => {
                    self.master_gain = db_to_linear(db);
                }
            }
        }

        out.fill(0.0);
        let mut active = 0u32;
        for slot in self.slots.iter_mut() {
            if let Some(channel) = slot {
                channel.render(out);
                active += 1;
            }
        }

        for sample in out.iter_mut() {
            *sample *= self.master_gain;
        }
        let mut peak = 0.0f32;
        let mut sumsq = 0.0f64;
        let mut clipped = 0u64;
        self.limiter.process(out, &mut clipped);
        for sample in out.iter() {
            peak = peak.max(sample.abs());
            sumsq += (*sample as f64) * (*sample as f64);
        }

        let frames = (out.len() / CHANNELS) as u64;
        if peak == 0.0 {
            self.metrics.silent_frames.fetch_add(frames, Ordering::Relaxed);
            if !self.silent_last {
                self.metrics
                    .silent_gaps
                    .fetch_add(1, Ordering::Relaxed);
                self.silent_since = Some(Instant::now());
                log::debug!(
                    "mixer: digital silence starts with {active} stream(s) attached"
                );
            }
        } else if self.silent_last {
            // I3: one zero run at the mixer output, as long as it lasted.
            let run_ms = self
                .silent_since
                .map(|at| at.elapsed().as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
            if run_ms >= 5.0 {
                log::info!(
                    "trace: zerorun point=mix run_ms={:.1} streams={active}",
                    run_ms
                );
            }
            self.silent_since = None;
        }
        self.silent_last = peak == 0.0;

        let samples = out.len().max(1) as f64;
        store_f32(&self.metrics.rms, (sumsq / samples).sqrt() as f32);
        store_f32(&self.metrics.peak, peak);
        self.metrics
            .peak_hold
            .fetch_max(peak.to_bits(), Ordering::Relaxed);
        self.metrics
            .frames_rendered
            .fetch_add(frames, Ordering::Relaxed);
        self.metrics
            .clipped_samples
            .fetch_add(clipped, Ordering::Relaxed);
        self.metrics.active_streams.store(active, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{frames_for_ms, SAMPLE_RATE};
    use crate::metrics::load_f32;
    use crate::protocol::Codec;

    struct Tone {
        phase: f64,
        step: f64,
    }

    impl Tone {
        fn new(freq: f64) -> Self {
            Self {
                phase: 0.0,
                step: 2.0 * std::f64::consts::PI * freq / SAMPLE_RATE as f64,
            }
        }

        fn fill(&mut self, pcm: &mut [f32]) {
            for frame in pcm.chunks_exact_mut(CHANNELS) {
                let value = (self.phase.sin() as f32) * 0.25;
                frame[0] = value;
                frame[1] = value;
                self.phase = (self.phase + self.step) % (2.0 * std::f64::consts::PI);
            }
        }
    }

    #[test]
    fn target_set_before_a_stream_joins_applies_to_it() {
        let (mut mixer, mut handle) = new_mixer(4);
        assert!(handle.set_target_ms(80.0));

        let (mut producer, consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS);
        let metrics = Arc::new(StreamMetrics::new(
            1,
            0,
            0,
            "late".into(),
            "127.0.0.1:1".into(),
            Codec::RawF32,
            30.0,
        ));
        let mut prefill = vec![0.0f32; frames_for_ms(30.0) * CHANNELS];
        Tone::new(440.0).fill(&mut prefill);
        let _ = producer.push_partial_slice(&prefill);
        assert!(handle.add(StreamChannel::new(metrics.clone(), consumer, 30.0)));

        let mut out = vec![0.0f32; 240 * CHANNELS];
        mixer.render(&mut out);
        assert!(
            (load_f32(&metrics.target_ms) - 80.0).abs() < 0.01,
            "late stream kept the startup target: {}",
            load_f32(&metrics.target_ms)
        );
    }

    #[test]
    fn counts_digital_silence_runs_that_no_other_metric_sees() {
        // 挂上流但环里没数据：输出是纯零。这正是实录里"既不记欠载也不记丢帧"
        // 的中断，必须有一个指标能数到它。
        let (mut mixer, mut handle) = new_mixer(4);
        let (_producer, consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS);
        let metrics = Arc::new(StreamMetrics::new(
            7,
            0,
            0,
            "starved".into(),
            "127.0.0.1:1".into(),
            Codec::RawF32,
            30.0,
        ));
        assert!(handle.add(StreamChannel::new(metrics.clone(), consumer, 30.0)));

        let mut out = vec![0.0f32; frames_for_ms(50.0) * CHANNELS];
        for _ in 0..10 {
            mixer.render(&mut out);
        }
        let master = mixer.metrics.snapshot();
        assert_eq!(master.silent_gaps, 1, "one continuous run of silence");
        assert_eq!(master.silent_ms, 500, "ten 50 ms blocks of zeros");

        // 持续供音频之后，静音计数必须停下来：修剪不再产生数字静音。
        let (mut p2, c2) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS);
        let live = Arc::new(StreamMetrics::new(
            8,
            0,
            0,
            "live".into(),
            "127.0.0.1:1".into(),
            Codec::RawF32,
            30.0,
        ));
        assert!(handle.add(StreamChannel::new(live.clone(), c2, 30.0)));
        let mut tone = Tone::new(440.0);
        for _ in 0..10 {
            let mut block = vec![0.0f32; frames_for_ms(50.0) * CHANNELS];
            tone.fill(&mut block);
            let _ = p2.push_partial_slice(&block);
            mixer.render(&mut out);
        }
        let after = mixer.metrics.snapshot();
        assert_eq!(
            after.silent_gaps, 1,
            "a fed stream must not fall back into digital silence"
        );
        assert_eq!(after.silent_ms, 500, "silent time stopped growing");
    }

    #[test]
    fn mixes_two_tones_through_the_soft_limiter() {
        let (mut mixer, mut handle) = new_mixer(8);
        let mut producers = Vec::new();
        let mut tones = Vec::new();
        let mut feeds = Vec::new();

        for (freq, id) in [(440.0, 1u64), (660.0, 2u64)] {
            let (mut producer, consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS);
            let metrics = Arc::new(StreamMetrics::new(
                id,
                0,
                0,
                format!("tone-{id}"),
                "127.0.0.1:1".into(),
                Codec::RawF32,
                30.0,
            ));
            let mut prefill = vec![0.0f32; frames_for_ms(30.0) * CHANNELS];
            Tone::new(freq).fill(&mut prefill);
            let _ = producer.push_partial_slice(&prefill);

            assert!(handle.add(StreamChannel::new(metrics, consumer, 30.0)));
            producers.push(producer);
            tones.push(Tone::new(freq));
            feeds.push(vec![0.0f32; 240 * CHANNELS]);
        }

        let mut out = vec![0.0f32; 240 * CHANNELS];
        let mut peak: f32 = 0.0;
        let mut rms_sum = 0.0f64;
        let mut blocks = 0;
        for _ in 0..200 {
            for i in 0..producers.len() {
                tones[i].fill(&mut feeds[i]);
                let _ = producers[i].push_partial_slice(&feeds[i]);
            }
            mixer.render(&mut out);
            for sample in &out {
                peak = peak.max(sample.abs());
                rms_sum += (*sample as f64) * (*sample as f64);
            }
            blocks += 1;
        }

        let master = handle.metrics().snapshot();
        assert_eq!(master.frames_rendered, blocks * 240);
        assert_eq!(master.active_streams, 2);
        assert!(
            peak <= 1.0,
            "soft limiter must keep peak <= 1.0, got {peak}"
        );
        let rms = (rms_sum / (blocks * 240 * CHANNELS as u64) as f64).sqrt();
        assert!(rms > 0.1, "mix should be audible, rms {rms}");
    }

    #[test]
    fn high_gain_is_limited_and_mute_still_drains() {
        let (mut mixer, mut handle) = new_mixer(8);
        let (mut producer, consumer) = rtrb::RingBuffer::new(SAMPLE_RATE as usize * CHANNELS);
        let metrics = Arc::new(StreamMetrics::new(
            7,
            0,
            0,
            "hot".into(),
            "127.0.0.1:2".into(),
            Codec::RawF32,
            20.0,
        ));
        metrics.set_gain_db(18.0);
        let mut prefill = vec![0.0f32; frames_for_ms(20.0) * CHANNELS];
        Tone::new(1000.0).fill(&mut prefill);
        let _ = producer.push_partial_slice(&prefill);
        assert!(handle.add(StreamChannel::new(metrics.clone(), consumer, 20.0)));

        let mut tone = Tone::new(1000.0);
        let mut feed = vec![0.0f32; 240 * CHANNELS];
        let mut out = vec![0.0f32; 240 * CHANNELS];
        let mut peak = 0.0f32;
        for _ in 0..100 {
            tone.fill(&mut feed);
            let _ = producer.push_partial_slice(&feed);
            mixer.render(&mut out);
            for sample in &out {
                peak = peak.max(sample.abs());
            }
        }
        assert!(peak > 0.9 && peak <= 1.0, "limiter engaged, peak {peak}");
        assert!(handle.metrics().snapshot().clipped_samples > 0);

        // Muted channels keep consuming so their rings do not overflow.
        metrics.muted.store(true, Ordering::Relaxed);
        tone.fill(&mut feed);
        let _ = producer.push_partial_slice(&feed);
        mixer.render(&mut out);
        assert!(out.iter().all(|s| s.abs() < 1e-6));
        assert!(load_f32(&metrics.rms) < 1e-6);
        assert!(producer.slots() > 0, "muted stream must still be drained");
    }

    #[test]
    fn limiter_stays_linear_below_the_knee() {
        // F4: a -6 dBFS peak must pass through untouched — the old tanh
        // compressed it to tanh(0.5)=0.462 (-5.7 dBFS heard as -6.7).
        let (mut mixer, mut handle) = new_mixer(4);
        let (mut producer, consumer) = rtrb::RingBuffer::new(96_000);
        let metrics = Arc::new(StreamMetrics::new(
            1,
            0x1234,
            7,
            "test".into(),
            "127.0.0.1:1".into(),
            Codec::RawF32,
            20.0,
        ));
        metrics.set_gain_db(6.0); // 0.25 tone -> 0.5 peak, below the 0.9 knee
        assert!(handle.add(StreamChannel::new(metrics.clone(), consumer, 20.0)));

        let mut tone = Tone::new(1000.0);
        let mut feed = vec![0.0f32; 240 * CHANNELS];
        let mut out = vec![0.0f32; 240 * CHANNELS];
        let mut peak = 0.0f32;
        for _ in 0..50 {
            tone.fill(&mut feed);
            let _ = producer.push_partial_slice(&feed);
            mixer.render(&mut out);
            if handle.metrics().snapshot().frames_rendered > 2 * 240 {
                for sample in &out {
                    peak = peak.max(sample.abs());
                }
            }
        }
        assert!(
            peak > 0.47 && peak <= 0.51,
            "below-knee signal must stay linear (tanh would give ~0.462): {peak}"
        );
    }
}
