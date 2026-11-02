//! Stateful engine facade used by the Flutter app (via `flutter_rust_bridge`)
//! and, later, by the Linux WebSocket daemon.
//!
//! The CLI remains a thin scripted consumer; this module owns long-running
//! center/sender lifetimes and exposes serializable snapshots. All heavy work
//! stays on the same threads as the CLI: UDP receiver, capture, output callback
//! and recorder.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::capture::{CaptureKind, CapturePipe, CaptureStats, ToneSource};
use crate::dsp::{new_mixer, Mixer, MixerControl};
use crate::format::{frames_for_ms, CHANNELS, SAMPLE_RATE};
use crate::metrics::{
    load_f32, store_f32, MasterSnapshot, MetricsRegistry, MixerMetrics, StreamSnapshot,
};
use crate::net::discovery::{self, Advertiser};
use crate::net::udp::{
    self, ReceiverConfig, ReceiverHandle, ReceiverStats, ReceiverStatsSnapshot, UdpSender,
};
use crate::output::{record_to_wav, start_playback, Playback, RecordSummary};
use crate::protocol::Codec;
use crate::{CoreError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CenterMode {
    Playback,
    Record,
    Silent,
    /// Diagnostic mode: no UDP receiver and no audio sink. Used to isolate
    /// process-level crashes from the audio/network threads.
    Idle,
    /// Diagnostic mode: UDP receiver only, no audio sink rendering.
    Monitor,
    /// Diagnostic mode: audio sink rendering only, no UDP receiver.
    Sink,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CenterConfig {
    /// Optional tap shared with a mesh hub that forwards N-1 mixes.
    #[serde(skip)]
    pub mix_tap: Option<Arc<crate::net::udp::MixTap>>,
    pub bind: String,
    pub port: u16,
    pub target_ms: f32,
    pub max_streams: usize,
    pub mode: CenterMode,
    pub device: Option<String>,
    pub record_path: Option<String>,
    pub advertise: bool,
    pub name: String,
    /// mDNS service role ("center", or "mesh" for decentralized nodes).
    pub role: String,
    /// Stable per-node identifier used to filter self-discovery.
    pub node_id: String,
    /// Bluetooth origin bit advertised over mDNS.
    pub bit: Option<u8>,
    /// Local Bluetooth adapter address advertised over mDNS.
    pub bt: Option<String>,
    /// RFCOMM channel advertised over mDNS.
    pub bt_channel: Option<u8>,
}

impl Default for CenterConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0".into(),
            port: 47900,
            target_ms: 30.0,
            max_streams: 32,
            mode: CenterMode::Playback,
            device: None,
            record_path: None,
            advertise: true,
            name: default_node_name(),
            role: "center".into(),
            node_id: String::new(),
            bit: None,
            bt: None,
            bt_channel: None,
            mix_tap: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CenterInfo {
    pub bind: String,
    pub mode: CenterMode,
    pub device: String,
    pub record_path: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CenterSnapshot {
    pub running: bool,
    pub stopping: bool,
    pub info: CenterInfo,
    pub target_ms: f32,
    pub master_gain_db: f32,
    pub seconds: f64,
    pub master: MasterSnapshot,
    pub streams: Vec<StreamSnapshot>,
    pub receiver: ReceiverStatsSnapshot,
    pub record_frames: u64,
    pub record_seconds: f64,
    pub error: Option<String>,
}

enum CenterControl {
    SetTargetMs(f32),
    SetMasterGainDb(f32),
}

struct CenterShared {
    stop: Arc<AtomicBool>,
    running: AtomicBool,
    registry: Arc<MetricsRegistry>,
    master: Arc<MixerMetrics>,
    receiver_stats: Arc<ReceiverStats>,
    control: MixerControl,
    ctrl_tx: Mutex<Option<Sender<CenterControl>>>,
    info: Mutex<CenterInfo>,
    target_ms: Arc<AtomicU32>,
    master_gain_db: AtomicU32,
    record: Mutex<Option<RecordSummary>>,
    error: Mutex<Option<String>>,
    started: Instant,
}

enum Sink {
    Playback(Playback),
    Record(JoinHandle<Result<RecordSummary>>),
    Silent(Mixer),
    Idle,
}

pub struct CenterEngine {
    shared: Arc<CenterShared>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl CenterEngine {
    pub fn start(config: CenterConfig) -> Result<Self> {
        crate::platform::begin_high_res_timer();
        if !(5.0..=500.0).contains(&config.target_ms) {
            return Err(CoreError::InvalidArgument(
                "target buffer must be between 5 and 500 ms".into(),
            ));
        }
        if config.mode == CenterMode::Record
            && config
                .record_path
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .is_none()
        {
            return Err(CoreError::InvalidArgument(
                "record mode needs a record path".into(),
            ));
        }
        let bind_ip: IpAddr = config
            .bind
            .parse()
            .map_err(|_| CoreError::InvalidArgument(format!("invalid bind '{}'", config.bind)))?;
        let bind = SocketAddr::new(bind_ip, config.port);
        let max_streams = config.max_streams.max(1);
        log::info!("center: starting on {bind}, mode {:?}", config.mode);

        let (mixer, mixer_handle) = new_mixer(max_streams);
        let master = mixer_handle.metrics();
        let control = MixerControl::new(mixer_handle);
        let target_ms = Arc::new(AtomicU32::new(config.target_ms.to_bits()));
        let receiver = if matches!(config.mode, CenterMode::Idle | CenterMode::Sink) {
            log::info!("center: idle mode, no receiver or sink");
            None
        } else {
            let receiver = udp::spawn(
                ReceiverConfig {
                    bind,
                    target_ms: config.target_ms,
                    max_streams,
                    live_target_ms: Some(target_ms.clone()),
                    mix_tap: config.mix_tap.clone(),
                },
                control.clone(),
            )?;
            log::info!("center: receiver bound on {}", receiver.local_addr);
            Some(receiver)
        };

        let advertiser: Option<Advertiser> =
            if config.advertise && !matches!(config.mode, CenterMode::Idle | CenterMode::Sink) {
                let port = receiver
                    .as_ref()
                    .map(|receiver| receiver.local_addr.port())
                    .unwrap_or(0);
                match discovery::advertise_full(
                    &config.role,
                    &config.name,
                    port,
                    "raw,opus",
                    &config.node_id,
                    config.bit,
                    config.bt.as_deref(),
                    config.bt_channel,
                ) {
                    Ok(advertiser) => Some(advertiser),
                    Err(error) => {
                        log::warn!("mDNS advertising failed: {error}");
                        None
                    }
                }
            } else {
                None
            };

        let info = CenterInfo {
            bind: receiver
                .as_ref()
                .map(|receiver| receiver.local_addr.to_string())
                .unwrap_or_else(|| "idle".into()),
            mode: config.mode,
            device: String::new(),
            record_path: config.record_path.clone(),
            name: config.name.clone(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(CenterShared {
            stop: stop.clone(),
            running: AtomicBool::new(false),
            registry: receiver
                .as_ref()
                .map(|receiver| receiver.registry.clone())
                .unwrap_or_default(),
            master,
            receiver_stats: receiver
                .as_ref()
                .map(|receiver| receiver.stats.clone())
                .unwrap_or_default(),
            control,
            ctrl_tx: Mutex::new(None),
            info: Mutex::new(info),
            target_ms,
            master_gain_db: AtomicU32::new(0f32.to_bits()),
            record: Mutex::new(None),
            error: Mutex::new(None),
            started: Instant::now(),
        });
        let (ctrl_tx, ctrl_rx) = mpsc::channel();
        *shared.ctrl_tx.lock().unwrap() = Some(ctrl_tx);

        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_shared = shared.clone();
        let thread_stop = stop.clone();
        let mode = config.mode;
        let device_query = config.device.clone();
        let record_path = config.record_path.clone();
        log::info!("center: spawning sink thread");
        let join = thread::Builder::new()
            .name("siren-center".into())
            .spawn(move || {
                run_center(
                    mixer,
                    receiver,
                    advertiser,
                    thread_shared,
                    ctrl_rx,
                    ready_tx,
                    thread_stop,
                    mode,
                    device_query,
                    record_path,
                );
            })
            .map_err(|e| CoreError::Backend(format!("spawn center: {e}")))?;

        log::info!("center: waiting for sink readiness");
        match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => {
                log::info!("center: ready");
                Ok(Self {
                    shared,
                    join: Mutex::new(Some(join)),
                })
            }
            Ok(Err(message)) => {
                log::error!("center: sink failed: {message}");
                stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err(CoreError::Backend(message))
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err(CoreError::Backend(
                    "center did not start within 15 seconds".into(),
                ))
            }
        }
    }

    pub fn snapshot(&self) -> CenterSnapshot {
        let info = self.shared.info.lock().unwrap().clone();
        let (record_frames, record_seconds) = self
            .shared
            .record
            .lock()
            .unwrap()
            .as_ref()
            .map(|record| (record.frames, record.seconds))
            .unwrap_or((0, 0.0));
        let running = self.shared.running.load(Ordering::Relaxed);
        CenterSnapshot {
            running,
            stopping: running && self.shared.stop.load(Ordering::Relaxed),
            info,
            target_ms: load_f32(&self.shared.target_ms),
            master_gain_db: load_f32(&self.shared.master_gain_db),
            seconds: self.shared.started.elapsed().as_secs_f64(),
            master: self.shared.master.snapshot(),
            streams: self.shared.registry.snapshots(),
            receiver: self.shared.receiver_stats.snapshot(),
            record_frames,
            record_seconds,
            error: self.shared.error.lock().unwrap().clone(),
        }
    }

    pub fn bind_addr(&self) -> String {
        self.shared.info.lock().unwrap().bind.clone()
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        // Teardown has to be attributable: a center that stops without the UI
        // asking for it silently drops every stream in the mesh.
        if !self.shared.stop.swap(true, Ordering::Relaxed) {
            stop_trace("center");
        }
    }

    pub fn set_gain_db(&self, stream: u64, gain_db: f32) -> bool {
        self.shared.registry.set_gain_db(stream, gain_db)
    }

    pub fn set_muted(&self, stream: u64, muted: bool) -> bool {
        self.shared.registry.set_muted(stream, muted)
    }

    pub fn set_target_ms(&self, ms: f32) -> Result<()> {
        if !(5.0..=500.0).contains(&ms) {
            return Err(CoreError::InvalidArgument(
                "target buffer must be between 5 and 500 ms".into(),
            ));
        }
        store_f32(&self.shared.target_ms, ms);
        self.send_control(CenterControl::SetTargetMs(ms))
    }

    pub fn set_master_gain_db(&self, db: f32) -> Result<()> {
        let db = db.clamp(-60.0, 24.0);
        store_f32(&self.shared.master_gain_db, db);
        self.send_control(CenterControl::SetMasterGainDb(db))
    }

    fn send_control(&self, command: CenterControl) -> Result<()> {
        let tx = self.shared.ctrl_tx.lock().unwrap();
        match tx.as_ref() {
            Some(tx) => tx
                .send(command)
                .map_err(|_| CoreError::Backend("center is stopping".into())),
            None => Err(CoreError::Backend("center control is not available".into())),
        }
    }
}

/// Names the thread that asked an engine to stop. Release builds ship without
/// symbols, so the full backtrace stays behind `SIREN_STOP_TRACE=1`.
pub(crate) fn stop_trace(component: &str) {
    let thread = std::thread::current()
        .name()
        .unwrap_or("<unnamed thread>")
        .to_string();
    if std::env::var_os("SIREN_STOP_TRACE").is_some() {
        log::info!(
            "{component}: stop requested by {thread}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
    } else {
        log::info!("{component}: stop requested by {thread}");
    }
}

impl Drop for CenterEngine {
    fn drop(&mut self) {
        self.stop();
        if let Some(join) = self.join.lock().unwrap().take() {
            let _ = thread::Builder::new()
                .name("siren-center-reaper".into())
                .spawn(move || {
                    let _ = join.join();
                });
        }
    }
}

fn run_center(
    mixer: Mixer,
    receiver: Option<ReceiverHandle>,
    advertiser: Option<Advertiser>,
    shared: Arc<CenterShared>,
    ctrl_rx: Receiver<CenterControl>,
    ready_tx: mpsc::SyncSender<std::result::Result<(), String>>,
    stop: Arc<AtomicBool>,
    mode: CenterMode,
    device_query: Option<String>,
    record_path: Option<String>,
) {
    crate::platform::boost_thread_priority();
    log::info!("center sink: thread started");

    let mut sink = match mode {
        CenterMode::Idle | CenterMode::Monitor => {
            log::info!("center sink: {mode:?}");
            shared.info.lock().unwrap().device = "monitor".into();
            shared.running.store(true, Ordering::Relaxed);
            if ready_tx.send(Ok(())).is_err() {
                stop.store(true, Ordering::Relaxed);
            }
            drop(mixer);
            Sink::Idle
        }
        CenterMode::Playback => {
            log::info!("center sink: opening playback device {:?}", device_query);
            match start_playback(mixer, device_query.as_deref()) {
                Ok(playback) => {
                    log::info!("center sink: playback opened: {}", playback.describe());
                    shared.info.lock().unwrap().device = playback.describe();
                    shared.running.store(true, Ordering::Relaxed);
                    if ready_tx.send(Ok(())).is_err() {
                        stop.store(true, Ordering::Relaxed);
                    }
                    Sink::Playback(playback)
                }
                Err(error) => {
                    log::error!("center sink: playback failed: {error}");
                    let _ = ready_tx.send(Err(error.to_string()));
                    if let Some(receiver) = receiver {
                        receiver.stop();
                    }
                    return;
                }
            }
        }
        CenterMode::Record => {
            let Some(path) = record_path else {
                let _ = ready_tx.send(Err("record mode needs a record path".into()));
                if let Some(receiver) = receiver {
                    receiver.stop();
                }
                return;
            };
            let path = PathBuf::from(path);
            let record_stop = stop.clone();
            match thread::Builder::new()
                .name("siren-recorder".into())
                .spawn(move || record_to_wav(mixer, &path, record_stop))
            {
                Ok(handle) => {
                    shared.info.lock().unwrap().device = "WAV recorder".into();
                    shared.running.store(true, Ordering::Relaxed);
                    if ready_tx.send(Ok(())).is_err() {
                        stop.store(true, Ordering::Relaxed);
                    }
                    Sink::Record(handle)
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("spawn recorder: {error}")));
                    if let Some(receiver) = receiver {
                        receiver.stop();
                    }
                    return;
                }
            }
        }
        CenterMode::Silent | CenterMode::Sink => {
            shared.info.lock().unwrap().device = "silent clock".into();
            shared.running.store(true, Ordering::Relaxed);
            if ready_tx.send(Ok(())).is_err() {
                stop.store(true, Ordering::Relaxed);
            }
            Sink::Silent(mixer)
        }
    };

    let mut buffer = vec![0.0f32; frames_for_ms(5.0) * CHANNELS];
    let mut deadline = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        while let Ok(command) = ctrl_rx.try_recv() {
            match command {
                CenterControl::SetTargetMs(ms) => {
                    if !shared.control.set_target_ms(ms) {
                        log::warn!("mixer command queue full while setting target");
                    }
                }
                CenterControl::SetMasterGainDb(db) => {
                    if !shared.control.set_master_gain_db(db) {
                        log::warn!("mixer command queue full while setting master gain");
                    }
                }
            }
        }
        match &mut sink {
            Sink::Silent(mixer) => {
                deadline += Duration::from_millis(5);
                mixer.render(&mut buffer);
                let now = Instant::now();
                if deadline > now {
                    thread::sleep(deadline - now);
                } else if now.duration_since(deadline) > Duration::from_millis(100) {
                    deadline = now;
                }
            }
            _ => thread::sleep(Duration::from_millis(10)),
        }
    }

    match sink {
        Sink::Playback(playback) => drop(playback),
        Sink::Idle => {}
        Sink::Record(handle) => match handle.join() {
            Ok(Ok(summary)) => *shared.record.lock().unwrap() = Some(summary),
            Ok(Err(error)) => *shared.error.lock().unwrap() = Some(error.to_string()),
            Err(_) => *shared.error.lock().unwrap() = Some("recorder thread panicked".into()),
        },
        Sink::Silent(_) => {}
    }
    if let Some(receiver) = receiver {
        receiver.stop();
    }
    drop(advertiser);
    log::info!("center sink: loop exited, mode {mode:?}");
    shared.running.store(false, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendSource {
    Tone,
    Loopback,
    Microphone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendCodec {
    Raw,
    Opus,
}

impl SendCodec {
    fn as_str(self) -> &'static str {
        match self {
            SendCodec::Raw => Codec::RawF32.as_str(),
            SendCodec::Opus => Codec::Opus.as_str(),
        }
    }

    pub(crate) fn to_codec(self) -> Codec {
        match self {
            SendCodec::Raw => Codec::RawF32,
            SendCodec::Opus => Codec::Opus,
        }
    }
}

impl SendSource {
    fn as_str(self) -> &'static str {
        match self {
            SendSource::Tone => "tone",
            SendSource::Loopback => "loopback",
            SendSource::Microphone => "microphone",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SenderConfig {
    pub to: String,
    pub source: SendSource,
    pub codec: SendCodec,
    pub frame_ms: u16,
    pub freq: f64,
    pub amplitude: f32,
    pub duration: Option<f64>,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            to: "auto".into(),
            source: SendSource::Tone,
            codec: SendCodec::Raw,
            frame_ms: 5,
            freq: 440.0,
            amplitude: 0.25,
            duration: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SenderInfo {
    pub target: String,
    pub source: String,
    pub codec: String,
    pub frame_ms: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct SenderSnapshot {
    pub running: bool,
    pub stopping: bool,
    pub info: SenderInfo,
    pub seconds: f64,
    pub frames: u64,
    pub packets: u64,
    pub bytes: u64,
    pub kbps: f64,
    pub capture_frames: u64,
    pub capture_dropped_frames: u64,
    pub capture_silent_frames: u64,
    pub capture_read_errors: u64,
    pub capture_buffer_frames: usize,
    pub error: Option<String>,
}

struct SenderShared {
    stop: AtomicBool,
    running: AtomicBool,
    started: Mutex<Instant>,
    info: Mutex<SenderInfo>,
    error: Mutex<Option<String>>,
    frames: AtomicU64,
    packets: AtomicU64,
    bytes: AtomicU64,
    capture: Mutex<Option<Arc<CaptureStats>>>,
    capture_buffer: AtomicUsize,
}

pub struct SenderEngine {
    shared: Arc<SenderShared>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl SenderEngine {
    pub fn start(config: SenderConfig) -> Result<Self> {
        crate::platform::begin_high_res_timer();
        if !(1..=60).contains(&config.frame_ms) {
            return Err(CoreError::InvalidArgument(
                "frame duration must be between 1 and 60 ms".into(),
            ));
        }
        if config.codec == SendCodec::Opus && !matches!(config.frame_ms, 5 | 10 | 20 | 40 | 60) {
            return Err(CoreError::InvalidArgument(
                "Opus supports 5/10/20/40/60 ms frames".into(),
            ));
        }
        if config.source == SendSource::Tone
            && (!config.freq.is_finite() || config.freq <= 0.0 || config.freq >= 20000.0)
        {
            return Err(CoreError::InvalidArgument(
                "tone frequency must be between 0 and 20 kHz".into(),
            ));
        }

        let shared = Arc::new(SenderShared {
            stop: AtomicBool::new(false),
            running: AtomicBool::new(false),
            started: Mutex::new(Instant::now()),
            info: Mutex::new(SenderInfo {
                target: String::new(),
                source: config.source.as_str().into(),
                codec: config.codec.as_str().into(),
                frame_ms: config.frame_ms,
            }),
            error: Mutex::new(None),
            frames: AtomicU64::new(0),
            packets: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            capture: Mutex::new(None),
            capture_buffer: AtomicUsize::new(0),
        });
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_shared = shared.clone();
        let join = thread::Builder::new()
            .name("siren-sender".into())
            .spawn(move || run_sender(config, thread_shared, ready_tx))
            .map_err(|e| CoreError::Backend(format!("spawn sender: {e}")))?;

        match ready_rx.recv_timeout(Duration::from_secs(25)) {
            Ok(Ok(())) => Ok(Self {
                shared,
                join: Mutex::new(Some(join)),
            }),
            Ok(Err(message)) => {
                shared.stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err(CoreError::Backend(message))
            }
            Err(_) => {
                shared.stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                Err(CoreError::Backend(
                    "sender did not start within 25 seconds".into(),
                ))
            }
        }
    }

    pub fn snapshot(&self) -> SenderSnapshot {
        let running = self.shared.running.load(Ordering::Relaxed);
        let seconds = self.shared.started.lock().unwrap().elapsed().as_secs_f64();
        let bytes = self.shared.bytes.load(Ordering::Relaxed);
        let (capture_frames, dropped, silent, errors) = self
            .shared
            .capture
            .lock()
            .unwrap()
            .as_ref()
            .map(|stats| {
                (
                    stats.frames.load(Ordering::Relaxed),
                    stats.dropped_frames.load(Ordering::Relaxed),
                    stats.silent_frames.load(Ordering::Relaxed),
                    stats.read_errors.load(Ordering::Relaxed),
                )
            })
            .unwrap_or((0, 0, 0, 0));
        SenderSnapshot {
            running,
            stopping: running && self.shared.stop.load(Ordering::Relaxed),
            info: self.shared.info.lock().unwrap().clone(),
            seconds,
            frames: self.shared.frames.load(Ordering::Relaxed),
            packets: self.shared.packets.load(Ordering::Relaxed),
            bytes,
            kbps: if seconds > 0.1 {
                bytes as f64 * 8.0 / seconds / 1000.0
            } else {
                0.0
            },
            capture_frames,
            capture_dropped_frames: dropped,
            capture_silent_frames: silent,
            capture_read_errors: errors,
            capture_buffer_frames: self.shared.capture_buffer.load(Ordering::Relaxed),
            error: self.shared.error.lock().unwrap().clone(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for SenderEngine {
    fn drop(&mut self) {
        self.stop();
        if let Some(join) = self.join.lock().unwrap().take() {
            let _ = thread::Builder::new()
                .name("siren-sender-reaper".into())
                .spawn(move || {
                    let _ = join.join();
                });
        }
    }
}

fn run_sender(
    config: SenderConfig,
    shared: Arc<SenderShared>,
    ready_tx: mpsc::SyncSender<std::result::Result<(), String>>,
) {
    crate::platform::boost_thread_priority();
    crate::platform::begin_high_res_timer();
    let stop = &shared.stop;

    let codec = config.codec.to_codec();
    let target = match discovery::resolve_target(&config.to, Duration::from_secs(5)) {
        Ok(target) => target,
        Err(error) => {
            let message = error.to_string();
            *shared.error.lock().unwrap() = Some(message.clone());
            let _ = ready_tx.send(Err(message));
            return;
        }
    };
    let origin = (crate::metrics::now_us() ^ ((std::process::id() as u64) << 32)).max(1);
    let mut sender = match UdpSender::new(target, codec, config.frame_ms, origin) {
        Ok(sender) => sender,
        Err(error) => {
            let message = error.to_string();
            *shared.error.lock().unwrap() = Some(message.clone());
            let _ = ready_tx.send(Err(message));
            return;
        }
    };

    let mut tone = ToneSource::new(config.freq, config.amplitude);
    let mut pipe: Option<CapturePipe> = match config.source {
        SendSource::Tone => None,
        SendSource::Loopback | SendSource::Microphone => {
            let kind = if config.source == SendSource::Loopback {
                CaptureKind::Loopback
            } else {
                CaptureKind::Microphone
            };
            match CapturePipe::start(kind) {
                Ok(pipe) => {
                    *shared.capture.lock().unwrap() = Some(pipe.stats());
                    Some(pipe)
                }
                Err(error) => {
                    let message = error.to_string();
                    *shared.error.lock().unwrap() = Some(message.clone());
                    let _ = ready_tx.send(Err(message));
                    return;
                }
            }
        }
    };

    let source_description = pipe
        .as_ref()
        .map(|pipe| pipe.description.clone())
        .unwrap_or_else(|| format!("{:.1} Hz tone", config.freq));
    *shared.info.lock().unwrap() = SenderInfo {
        target: target.to_string(),
        source: source_description,
        codec: config.codec.as_str().into(),
        frame_ms: config.frame_ms,
    };
    *shared.started.lock().unwrap() = Instant::now();
    shared.running.store(true, Ordering::Relaxed);
    if ready_tx.send(Ok(())).is_err() {
        stop.store(true, Ordering::Relaxed);
    }

    let frame_frames = frames_for_ms(config.frame_ms as f32);
    let frame_duration =
        Duration::from_micros((frame_frames as u64 * 1_000_000) / SAMPLE_RATE as u64);
    let started = Instant::now();
    let deadline = config
        .duration
        .map(|seconds| started + Duration::from_secs_f64(seconds.max(0.0)));
    let mut pcm = vec![0.0f32; frame_frames * CHANNELS];
    let mut next_tick = started;
    let mut diag_frames = 0u32;
    let mut send_failures = 0u32;

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if deadline.is_some_and(|deadline| now >= deadline) {
            break;
        }
        if next_tick > now {
            thread::sleep(next_tick - now);
        } else if now.duration_since(next_tick) > Duration::from_millis(150) {
            next_tick = now;
        }

        match pipe.as_mut() {
            Some(pipe) => {
                pipe.read_frame(&mut pcm);
                shared
                    .capture_buffer
                    .store(pipe.buffered_frames(), Ordering::Relaxed);
            }
            None => tone.fill(&mut pcm),
        }

        match sender.send(&pcm) {
            Ok(_) => {
                send_failures = 0;
                shared
                    .frames
                    .fetch_add(frame_frames as u64, Ordering::Relaxed);
                shared
                    .packets
                    .store(sender.packets_sent(), Ordering::Relaxed);
                shared.bytes.store(sender.bytes_sent(), Ordering::Relaxed);
            }
            Err(error) => {
                // Transient EAGAIN happens while ARP resolves or the NIC
                // re-associates; only give up after a full second of failures.
                send_failures += 1;
                if send_failures == 1 {
                    log::warn!("sender: send failed: {error}");
                }
                if send_failures >= 200 {
                    *shared.error.lock().unwrap() = Some(error.to_string());
                    break;
                }
            }
        }
        diag_frames += 1;
        if diag_frames % 500 == 0 {
            log::info!(
                "sender: tick frames={} packets={}",
                shared.frames.load(Ordering::Relaxed),
                shared.packets.load(Ordering::Relaxed),
            );
        }
        next_tick += frame_duration;
    }

    log::info!("sender: loop exited");
    if let Some(pipe) = pipe.take() {
        pipe.stop();
    }
    shared.capture_buffer.store(0, Ordering::Relaxed);
    shared.running.store(false, Ordering::Relaxed);
}

pub fn default_node_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "siren-node".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_center_receives_a_tone_sender() {
        let center = CenterEngine::start(CenterConfig {
            bind: "127.0.0.1".into(),
            port: 0,
            mode: CenterMode::Silent,
            advertise: false,
            ..Default::default()
        })
        .expect("start center");
        let port = center
            .bind_addr()
            .rsplit(':')
            .next()
            .expect("port")
            .to_string();

        let sender = SenderEngine::start(SenderConfig {
            to: format!("127.0.0.1:{port}"),
            source: SendSource::Tone,
            codec: SendCodec::Raw,
            duration: Some(3.0),
            ..Default::default()
        })
        .expect("start sender");

        let mut heard = None;
        for _ in 0..30 {
            thread::sleep(Duration::from_millis(100));
            let snapshot = center.snapshot();
            if !snapshot.streams.is_empty() && snapshot.master.peak_hold_db > -40.0 {
                heard = Some(snapshot);
                break;
            }
        }
        let snapshot = heard.expect("center never received the tone");
        assert_eq!(snapshot.streams.len(), 1);
        assert_eq!(snapshot.streams[0].codec, "raw");
        assert!(snapshot.receiver.datagrams > 0);

        let stream_id = snapshot.streams[0].id;
        assert!(center.set_muted(stream_id, true));
        assert!(center.set_gain_db(stream_id, -6.0));
        assert!(center.set_target_ms(40.0).is_ok());

        sender.stop();
        center.stop();
        for _ in 0..40 {
            if !center.is_running() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(!center.is_running());
    }
}
