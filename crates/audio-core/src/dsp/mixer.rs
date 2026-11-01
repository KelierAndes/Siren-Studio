//! Multi-track float mixer driven from the output callback (or a recording
//! clock). Channel add/remove/settings travel through a lock-free SPSC queue;
//! per-channel gain and mute are atomics so a UI thread never blocks audio.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use rtrb::{Consumer, Producer};

use crate::dsp::resampler::DriftResampler;
use crate::format::CHANNELS;
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
}

impl StreamChannel {
    pub fn new(metrics: Arc<StreamMetrics>, rx: Consumer<f32>, target_ms: f32) -> Self {
        Self {
            metrics,
            rx,
            resampler: DriftResampler::new(target_ms),
        }
    }

    pub fn id(&self) -> u64 {
        self.metrics.id
    }

    fn flush(&mut self) {
        while self.rx.pop().is_ok() {}
        self.resampler.reset();
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
        }
    }
}

pub struct Mixer {
    slots: Vec<Option<Box<StreamChannel>>>,
    cmd_rx: Consumer<MixerCommand>,
    master_gain: f32,
    /// Last target set by the UI. Streams that join later must use it too,
    /// otherwise a target chosen before any peer connected silently reverts
    /// to the startup value and network jitter becomes underruns.
    target_override: Option<f32>,
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
        target_override: None,
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

        let mut peak = 0.0f32;
        let mut sumsq = 0.0f64;
        let mut clipped = 0u64;
        for sample in out.iter_mut() {
            let pre = *sample * self.master_gain;
            if pre.abs() > 1.0 {
                clipped += 1;
            }
            // Soft limiter: tanh keeps overs above 1.0 from hard-clipping.
            let post = pre.tanh();
            *sample = post;
            peak = peak.max(post.abs());
            sumsq += (post as f64) * (post as f64);
        }

        let samples = out.len().max(1) as f64;
        store_f32(&self.metrics.rms, (sumsq / samples).sqrt() as f32);
        store_f32(&self.metrics.peak, peak);
        self.metrics
            .peak_hold
            .fetch_max(peak.to_bits(), Ordering::Relaxed);
        self.metrics
            .frames_rendered
            .fetch_add((out.len() / CHANNELS) as u64, Ordering::Relaxed);
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
}
