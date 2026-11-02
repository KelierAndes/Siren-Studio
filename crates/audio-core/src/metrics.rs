//! Lock-free runtime meters shared between the audio thread, the network
//! thread and status consumers (CLI / FFI / WebSocket).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::protocol::Codec;

const SILENCE_DB: f32 = -120.0;

pub fn linear_to_db(linear: f32) -> f32 {
    if !linear.is_finite() || linear <= 1e-6 {
        SILENCE_DB
    } else {
        (20.0 * linear.log10()).max(SILENCE_DB)
    }
}

pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn store_f32(atom: &AtomicU32, value: f32) {
    atom.store(value.to_bits(), Ordering::Relaxed);
}

pub fn load_f32(atom: &AtomicU32) -> f32 {
    f32::from_bits(atom.load(Ordering::Relaxed))
}

pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Per-remote-stream meters. Written by the mixer (levels, buffer) and the
/// receiver (packet counters); read by status consumers.
pub struct StreamMetrics {
    pub id: u64,
    pub origin: u64,
    pub stream_id: u16,
    pub label: String,
    /// Address the most recent copy arrived from (Wi-Fi peer or local tunnel).
    pub addr: Mutex<String>,
    /// Carrier of the most recent copy: `wifi`, `bt` or `local`.
    pub path: Mutex<String>,
    /// 32 log-spaced band levels (0..1) for the UI EQ display.
    pub spectrum: Mutex<Vec<f32>>,
    pub codec: Codec,
    pub gain_db: AtomicU32,
    pub muted: AtomicBool,
    pub rms: AtomicU32,
    pub peak: AtomicU32,
    pub peak_hold: AtomicU32,
    pub buffer_frames: AtomicU32,
    pub target_ms: AtomicU32,
    pub ratio: AtomicU32,
    pub underrun_blocks: AtomicU32,
    pub underrun_frames: AtomicU32,
    pub buffer_resets: AtomicU32,
    /// How often this stream reached its start water level (warm-ups after a
    /// join or a carrier-switch flush). The receiver pairs it with the switch
    /// timestamps to measure how long a carrier switch mutes the stream.
    pub starts: AtomicU32,
    pub packets: AtomicU64,
    pub lost_frames: AtomicU64,
    pub plc_frames: AtomicU64,
    pub overflow_packets: AtomicU64,
    pub last_packet_us: AtomicU64,
    pub active: AtomicBool,
}

impl StreamMetrics {
    pub fn new(
        id: u64,
        origin: u64,
        stream_id: u16,
        label: String,
        addr: String,
        codec: Codec,
        target_ms: f32,
    ) -> Self {
        Self {
            id,
            origin,
            stream_id,
            label,
            addr: Mutex::new(addr),
            path: Mutex::new("wifi".into()),
            spectrum: Mutex::new(Vec::new()),
            codec,
            gain_db: AtomicU32::new(0f32.to_bits()),
            muted: AtomicBool::new(false),
            rms: AtomicU32::new(0f32.to_bits()),
            peak: AtomicU32::new(0f32.to_bits()),
            peak_hold: AtomicU32::new(0f32.to_bits()),
            buffer_frames: AtomicU32::new(0),
            target_ms: AtomicU32::new(target_ms.to_bits()),
            ratio: AtomicU32::new(1f32.to_bits()),
            underrun_blocks: AtomicU32::new(0),
            underrun_frames: AtomicU32::new(0),
            buffer_resets: AtomicU32::new(0),
            starts: AtomicU32::new(0),
            packets: AtomicU64::new(0),
            lost_frames: AtomicU64::new(0),
            plc_frames: AtomicU64::new(0),
            overflow_packets: AtomicU64::new(0),
            last_packet_us: AtomicU64::new(0),
            active: AtomicBool::new(false),
        }
    }

    pub fn gain(&self) -> f32 {
        load_f32(&self.gain_db)
    }

    pub fn set_gain_db(&self, db: f32) {
        store_f32(&self.gain_db, db.clamp(-60.0, 24.0));
    }

    /// Remember which carrier delivered the latest copy of this stream.
    pub fn set_source(&self, addr: &str, path: &str) {
        if let Ok(mut current) = self.addr.lock() {
            if *current != addr {
                current.clear();
                current.push_str(addr);
            }
        }
        if let Ok(mut current) = self.path.lock() {
            if *current != path {
                current.clear();
                current.push_str(path);
            }
        }
    }

    pub fn set_spectrum(&self, bands: &[f32]) {
        if let Ok(mut current) = self.spectrum.lock() {
            current.clear();
            current.extend_from_slice(bands);
        }
    }

    pub fn snapshot(&self) -> StreamSnapshot {
        let now = now_us();
        let last = self.last_packet_us.load(Ordering::Relaxed);
        let rms = load_f32(&self.rms);
        let peak = load_f32(&self.peak);
        StreamSnapshot {
            id: self.id,
            origin: self.origin,
            stream_id: self.stream_id,
            label: self.label.clone(),
            addr: self.addr.lock().map(|addr| addr.clone()).unwrap_or_default(),
            path: self.path.lock().map(|path| path.clone()).unwrap_or_default(),
            spectrum: self
                .spectrum
                .lock()
                .map(|bands| bands.iter().map(|v| *v as f64).collect())
                .unwrap_or_default(),
            codec: self.codec.as_str().to_string(),
            active: self.active.load(Ordering::Relaxed),
            gain_db: sanitize(self.gain()),
            muted: self.muted.load(Ordering::Relaxed),
            rms_db: sanitize(linear_to_db(rms)),
            peak_db: sanitize(linear_to_db(peak)),
            peak_hold_db: sanitize(linear_to_db(load_f32(&self.peak_hold))),
            buffer_ms: sanitize(crate::format::ms_for_frames(
                self.buffer_frames.load(Ordering::Relaxed) as usize,
            )),
            target_ms: sanitize(load_f32(&self.target_ms)),
            ratio: {
                let value = load_f32(&self.ratio) as f64;
                if value.is_finite() {
                    (value * 1_000_000.0).round() / 1_000_000.0
                } else {
                    1.0
                }
            },
            underruns: self.underrun_blocks.load(Ordering::Relaxed),
            underrun_frames: self.underrun_frames.load(Ordering::Relaxed),
            buffer_resets: self.buffer_resets.load(Ordering::Relaxed),
            packets: self.packets.load(Ordering::Relaxed),
            lost_frames: self.lost_frames.load(Ordering::Relaxed),
            plc_frames: self.plc_frames.load(Ordering::Relaxed),
            overflow_packets: self.overflow_packets.load(Ordering::Relaxed),
            age_ms: if last == 0 {
                0
            } else {
                now.saturating_sub(last) / 1000
            },
        }
    }
}

fn sanitize<T: Into<f64>>(value: T) -> f64 {
    let value: f64 = value.into();
    if value.is_finite() {
        // Round to keep JSON readable.
        (value * 1000.0).round() / 1000.0
    } else {
        SILENCE_DB as f64
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamSnapshot {
    pub id: u64,
    /// Sender uid (Bluetooth/v2 identity); UIs map it to the node's name.
    /// Hex string so browsers do not lose precision on a u64.
    #[serde(serialize_with = "hex64")]
    pub origin: u64,
    pub stream_id: u16,
    pub label: String,
    pub addr: String,
    /// `wifi`, `bt` or `local` (loopback test link).
    pub path: String,
    /// 32 log-spaced band levels (0..1), empty until enough audio arrives.
    pub spectrum: Vec<f64>,
    pub codec: String,
    pub active: bool,
    pub gain_db: f64,
    pub muted: bool,
    pub rms_db: f64,
    pub peak_db: f64,
    pub peak_hold_db: f64,
    pub buffer_ms: f64,
    pub target_ms: f64,
    pub ratio: f64,
    pub underruns: u32,
    pub underrun_frames: u32,
    pub buffer_resets: u32,
    pub packets: u64,
    pub lost_frames: u64,
    pub plc_frames: u64,
    pub overflow_packets: u64,
    pub age_ms: u64,
}

/// Serialize a u64 as a 16-char hex string (JS numbers lose precision).
pub fn hex64<S: serde::Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format!("{value:016x}"))
}

/// Master bus meters.
pub struct MixerMetrics {
    pub rms: AtomicU32,
    pub peak: AtomicU32,
    pub peak_hold: AtomicU32,
    pub frames_rendered: AtomicU64,
    pub clipped_samples: AtomicU64,
    pub active_streams: AtomicU32,
}

impl MixerMetrics {
    pub fn new() -> Self {
        Self {
            rms: AtomicU32::new(0f32.to_bits()),
            peak: AtomicU32::new(0f32.to_bits()),
            peak_hold: AtomicU32::new(0f32.to_bits()),
            frames_rendered: AtomicU64::new(0),
            clipped_samples: AtomicU64::new(0),
            active_streams: AtomicU32::new(0),
        }
    }

    pub fn snapshot(&self) -> MasterSnapshot {
        MasterSnapshot {
            rms_db: sanitize(linear_to_db(load_f32(&self.rms))),
            peak_db: sanitize(linear_to_db(load_f32(&self.peak))),
            peak_hold_db: sanitize(linear_to_db(load_f32(&self.peak_hold))),
            frames_rendered: self.frames_rendered.load(Ordering::Relaxed),
            clipped_samples: self.clipped_samples.load(Ordering::Relaxed),
            active_streams: self.active_streams.load(Ordering::Relaxed),
        }
    }
}

impl Default for MixerMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MasterSnapshot {
    pub rms_db: f64,
    pub peak_db: f64,
    pub peak_hold_db: f64,
    pub frames_rendered: u64,
    pub clipped_samples: u64,
    pub active_streams: u32,
}

/// Shared list of active streams, used by status consumers.
#[derive(Default)]
pub struct MetricsRegistry {
    streams: Mutex<Vec<Arc<StreamMetrics>>>,
}

impl MetricsRegistry {
    pub fn add(&self, metrics: Arc<StreamMetrics>) {
        let mut streams = self.streams.lock().unwrap();
        streams.retain(|s| s.id != metrics.id);
        streams.push(metrics);
    }

    pub fn remove(&self, id: u64) {
        self.streams.lock().unwrap().retain(|s| s.id != id);
    }

    pub fn snapshots(&self) -> Vec<StreamSnapshot> {
        let streams = self.streams.lock().unwrap();
        let mut out: Vec<_> = streams.iter().map(|s| s.snapshot()).collect();
        out.sort_by_key(|s| s.id);
        out
    }

    pub fn len(&self) -> usize {
        self.streams.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn with_stream<R>(&self, id: u64, f: impl FnOnce(&StreamMetrics) -> R) -> Option<R> {
        let streams = self.streams.lock().unwrap();
        streams
            .iter()
            .find(|stream| stream.id == id)
            .map(|stream| f(stream))
    }

    /// Returns false if the stream is not present (already timed out).
    pub fn set_gain_db(&self, id: u64, db: f32) -> bool {
        self.with_stream(id, |stream| stream.set_gain_db(db))
            .is_some()
    }

    pub fn set_muted(&self, id: u64, muted: bool) -> bool {
        self.with_stream(id, |stream| stream.muted.store(muted, Ordering::Relaxed))
            .is_some()
    }
}
