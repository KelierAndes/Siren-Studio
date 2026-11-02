//! Embedded web control panel for the headless Linux mesh node.
//!
//! One process serves the page, the JSON API and the mesh engine. The browser
//! drives the same actions as the desktop/mobile UI: join, leave, source,
//! buffer target, master gain and per-stream gain/mute. Status is polled as
//! JSON, so the page needs no build step and no external assets.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use audio_core::engine::{default_node_name, SendCodec, SendSource};
use audio_core::hybrid::{HybridConfig, HybridNode, HybridOptions};
use audio_core::mesh::{MeshConfig, MeshSnapshot};

use crate::util;

#[derive(clap::Args, Debug)]
pub struct WebArgs {
    /// Address for the control panel, e.g. `0.0.0.0:8080`.
    #[arg(long, default_value = "0.0.0.0:8080")]
    pub listen: String,
    /// Join the mesh immediately with the initial settings.
    #[arg(long)]
    pub autostart: bool,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long, default_value = "loopback")]
    pub source: String,
    #[arg(long)]
    pub no_play: bool,
    #[arg(long, default_value_t = 30.0)]
    pub target_ms: f32,
    #[arg(long, default_value_t = 47900)]
    pub port: u16,
    #[arg(long, default_value = "raw")]
    pub codec: String,
    #[arg(long, default_value_t = 5)]
    pub frame_ms: u16,
    #[arg(long, default_value_t = 440.0)]
    pub freq: f64,
    /// Linux: isolate capture from playback with a PipeWire virtual sink
    /// (`pipewire`), or `off` for the monitor + DSP path.
    #[arg(long, value_name = "MODE")]
    pub isolate: Option<String>,
    /// Sink for local playback (default: current default sink).
    #[arg(long)]
    pub output_sink: Option<String>,
    /// Virtual sink that other applications play into (isolation mode).
    #[arg(long)]
    pub capture_sink: Option<String>,
    /// Pin peers without discovery (`ip:port`, repeatable).
    #[arg(long = "peer")]
    pub peers: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub name: String,
    /// `tone`, `loopback` or `mic`.
    pub source: String,
    pub output: bool,
    pub target_ms: f32,
    pub port: u16,
    pub codec: String,
    pub frame_ms: u16,
    pub freq: f64,
    /// Output sink for local playback (isolation mode sets it explicitly).
    pub output_sink: Option<String>,
    pub peers: Vec<String>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            name: default_node_name(),
            source: "loopback".into(),
            output: true,
            target_ms: 30.0,
            port: 47900,
            codec: "raw".into(),
            frame_ms: 5,
            freq: 440.0,
            output_sink: None,
            peers: Vec::new(),
        }
    }
}

impl UiConfig {
    fn source_value(&self) -> Option<SendSource> {
        match self.source.as_str() {
            "tone" => Some(SendSource::Tone),
            "loopback" => Some(SendSource::Loopback),
            "mic" => Some(SendSource::Microphone),
            _ => None,
        }
    }

    fn codec_value(&self) -> Option<SendCodec> {
        match self.codec.as_str() {
            "raw" => Some(SendCodec::Raw),
            "opus" => Some(SendCodec::Opus),
            _ => None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("节点名称不能为空".into());
        }
        if self.source_value().is_none() {
            return Err("未知音源".into());
        }
        if self.codec_value().is_none() {
            return Err("未知编码".into());
        }
        if !(5.0..=500.0).contains(&self.target_ms) {
            return Err("缓冲目标需在 5–500 ms".into());
        }
        if self.port == 0 {
            return Err("端口无效".into());
        }
        if !(1..=60).contains(&self.frame_ms) {
            return Err("帧长需在 1–60 ms".into());
        }
        if self.codec == "opus" && !matches!(self.frame_ms, 5 | 10 | 20 | 40 | 60) {
            return Err("Opus 帧长仅支持 5/10/20/40/60 ms".into());
        }
        Ok(())
    }

    fn to_mesh_config(&self) -> MeshConfig {
        MeshConfig {
            output_device: self
                .output_sink
                .as_ref()
                .map(|sink| sink.trim().to_string())
                .filter(|sink| !sink.is_empty()),
            bind: "0.0.0.0".into(),
            port: self.port,
            name: self.name.trim().to_string(),
            advertise: true,
            output: self.output,
            target_ms: self.target_ms,
            source: self.source_value().unwrap_or(SendSource::Loopback),
            codec: self.codec_value().unwrap_or(SendCodec::Raw),
            frame_ms: self.frame_ms,
            freq: self.freq,
            amplitude: 0.25,
            discovery_secs: 2,
            static_peers: self.peers.clone(),
            ..MeshConfig::default()
        }
    }

    fn to_hybrid_config(&self) -> HybridConfig {
        // The dual stack is fully software-managed: Wi-Fi is always on and
        // Bluetooth joins automatically. The RFCOMM channel is fixed (16) and
        // only reachable through SIREN_BT_CHANNEL for interop debugging.
        let channel = std::env::var("SIREN_BT_CHANNEL")
            .ok()
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|channel| (1..=30).contains(channel))
            .unwrap_or(16);
        HybridConfig {
            bt_enabled: true,
            bt_channel: channel,
        }
    }
}

struct Inner {
    node: Option<HybridNode>,
    starting: bool,
    error: Option<String>,
    config: UiConfig,
}

fn start_engine(config: &UiConfig) -> Result<HybridNode, String> {
    HybridNode::start(
        config.to_mesh_config(),
        HybridOptions {
            config: config.to_hybrid_config(),
            transport: None,
            state_path: None,
        },
    )
    .map_err(|error| error.to_string())
}

pub struct WebState {
    inner: Mutex<Inner>,
    /// Playback sink every engine start must use (isolation mode).
    output_sink: Option<String>,
    isolation: Option<IsolationView>,
}

#[derive(Clone, Serialize)]
struct IsolationView {
    capture_sink: String,
    playback_sink: String,
}

#[derive(Serialize)]
struct Status {
    running: bool,
    starting: bool,
    error: Option<String>,
    config: UiConfig,
    snapshot: Option<serde_json::Value>,
    isolation: Option<IsolationView>,
    version: &'static str,
}

/// Stream ids are u64; JavaScript numbers lose precision above 2^53, which
/// made per-stream gain/mute requests target a non-existent id. Serialize
/// them as strings for the browser.
fn snapshot_json(snapshot: MeshSnapshot) -> serde_json::Value {
    let mut value = serde_json::to_value(snapshot).unwrap_or(serde_json::Value::Null);
    if let Some(streams) = value
        .pointer_mut("/center/streams")
        .and_then(|streams| streams.as_array_mut())
    {
        for stream in streams {
            if let Some(id) = stream.get("id").cloned() {
                let text = match id {
                    serde_json::Value::Number(number) => number.to_string(),
                    other => other.to_string(),
                };
                stream["id"] = serde_json::Value::String(text);
            }
        }
    }
    value
}

fn ok() -> Response {
    Json(json!({ "ok": true })).into_response()
}

fn fail(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "ok": false, "error": message.into() })),
    )
        .into_response()
}

async fn index() -> Html<&'static str> {
    Html(include_str!("webui/index.html"))
}

async fn status(State(state): State<Arc<WebState>>) -> Json<Status> {
    let inner = state.inner.lock().unwrap();
    Json(Status {
        running: inner.node.is_some(),
        starting: inner.starting,
        error: inner.error.clone(),
        config: inner.config.clone(),
        snapshot: inner
            .node
            .as_ref()
            .map(|node| snapshot_json(node.snapshot())),
        isolation: state.isolation.clone(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn start(State(state): State<Arc<WebState>>, Json(config): Json<UiConfig>) -> Response {
    let mut config = config;
    if config.output_sink.is_none() {
        config.output_sink = state.output_sink.clone();
    }
    if let Err(message) = config.validate() {
        return fail(StatusCode::BAD_REQUEST, message);
    }
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.node.is_some() || inner.starting {
            return fail(StatusCode::CONFLICT, "已在组网或正在加入");
        }
        inner.starting = true;
        inner.error = None;
        inner.config = config.clone();
    }

    let start_config = config.clone();
    let result = tokio::task::spawn_blocking(move || start_engine(&start_config)).await;

    let mut inner = state.inner.lock().unwrap();
    inner.starting = false;
    match result {
        Ok(Ok(node)) => {
            inner.node = Some(node);
            ok()
        }
        Ok(Err(error)) => {
            inner.error = Some(error.clone());
            fail(StatusCode::BAD_REQUEST, error)
        }
        Err(error) => {
            inner.error = Some(error.to_string());
            fail(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
        }
    }
}

async fn leave(State(state): State<Arc<WebState>>) -> Response {
    log::info!("webui: leave requested");
    let node = {
        let mut inner = state.inner.lock().unwrap();
        if inner.starting {
            return fail(StatusCode::CONFLICT, "正在加入组网");
        }
        inner.node.take()
    };
    if let Some(node) = node {
        node.stop();
    }
    ok()
}

#[derive(Deserialize)]
struct StreamPatch {
    gain_db: Option<f32>,
    muted: Option<bool>,
}

async fn patch_stream(
    State(state): State<Arc<WebState>>,
    Path(id): Path<u64>,
    Json(patch): Json<StreamPatch>,
) -> Response {
    let inner = state.inner.lock().unwrap();
    let Some(node) = inner.node.as_ref() else {
        return fail(StatusCode::CONFLICT, "未组网");
    };
    let engine = node.mesh();
    let mut found = true;
    if let Some(gain_db) = patch.gain_db {
        found &= engine.center().set_gain_db(id, gain_db);
    }
    if let Some(muted) = patch.muted {
        found &= engine.center().set_muted(id, muted);
    }
    if found {
        ok()
    } else {
        fail(StatusCode::NOT_FOUND, "流不存在")
    }
}

#[derive(Deserialize)]
struct ControlPatch {
    master_gain_db: Option<f32>,
    target_ms: Option<f32>,
}

async fn patch_controls(
    State(state): State<Arc<WebState>>,
    Json(patch): Json<ControlPatch>,
) -> Response {
    let inner = state.inner.lock().unwrap();
    let Some(node) = inner.node.as_ref() else {
        return fail(StatusCode::CONFLICT, "未组网");
    };
    let engine = node.mesh();
    if let Some(db) = patch.master_gain_db {
        if let Err(error) = engine.center().set_master_gain_db(db) {
            return fail(StatusCode::BAD_REQUEST, error.to_string());
        }
    }
    if let Some(ms) = patch.target_ms {
        if let Err(error) = engine.center().set_target_ms(ms) {
            return fail(StatusCode::BAD_REQUEST, error.to_string());
        }
    }
    ok()
}

pub fn run(args: WebArgs) -> anyhow::Result<()> {
    let initial = UiConfig {
        name: args.name.clone().unwrap_or_else(default_node_name),
        source: args.source.clone(),
        output: !args.no_play,
        target_ms: args.target_ms,
        port: args.port,
        codec: args.codec.clone(),
        frame_ms: args.frame_ms,
        freq: args.freq,
        output_sink: args.output_sink.clone(),
        peers: args.peers.clone(),
    };
    initial.validate().map_err(anyhow::Error::msg)?;

    let isolation = match crate::isolate::setup(
        args.isolate.as_deref(),
        args.output_sink.as_deref(),
        args.capture_sink.as_deref(),
    ) {
        Ok(isolation) => isolation,
        Err(error) => {
            eprintln!(
                "warning: isolation unavailable: {error}; falling back to monitor capture + echo cancellation"
            );
            None
        }
    };
    let output_sink = isolation
        .as_ref()
        .map(|isolation| isolation.playback_sink().to_string())
        .or_else(|| args.output_sink.clone());
    let initial = UiConfig {
        output_sink: output_sink.clone(),
        ..initial
    };
    if let Some(isolation) = &isolation {
        println!(
            "isolation: capture {} → playback {}",
            isolation.capture_sink(),
            isolation.playback_sink()
        );
    }

    let isolation_view = isolation.as_ref().map(|isolation| IsolationView {
        capture_sink: isolation.capture_sink().to_string(),
        playback_sink: isolation.playback_sink().to_string(),
    });
    let state = Arc::new(WebState {
        isolation: isolation_view,
        output_sink,
        inner: Mutex::new(Inner {
            node: None,
            starting: false,
            error: None,
            config: initial.clone(),
        }),
    });

    let stop = util::stop_flag();

    if args.autostart {
        println!("joining mesh with the initial settings…");
        match start_engine(&initial) {
            Ok(node) => state.inner.lock().unwrap().node = Some(node),
            Err(error) => {
                eprintln!("warning: initial mesh start failed: {error}");
                state.inner.lock().unwrap().error = Some(error.to_string());
            }
        }
    }

    // The panel reports "joined" as long as a node is registered, so a node
    // whose sender loop went away would leave the hub silent and marked as
    // healthy until someone restarted it. Re-join with the same settings.
    {
        let heal = state.clone();
        let heal_stop = stop.clone();
        let _ = std::thread::Builder::new()
            .name("siren-web-heal".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(2));
                if heal_stop.load(Ordering::Relaxed) {
                    return;
                }
                let dead = {
                    let inner = heal.inner.lock().unwrap();
                    inner
                        .node
                        .as_ref()
                        .map(|node| !node.snapshot().running)
                        .unwrap_or(false)
                };
                if !dead {
                    continue;
                }
                log::warn!("webui: mesh node stopped unexpectedly, re-joining");
                let dead_node = {
                    let mut inner = heal.inner.lock().unwrap();
                    inner.starting = true;
                    inner.node.take()
                };
                drop(dead_node);
                let config = heal.inner.lock().unwrap().config.clone();
                match start_engine(&config) {
                    Ok(node) => {
                        let mut inner = heal.inner.lock().unwrap();
                        inner.node = Some(node);
                        inner.starting = false;
                    }
                    Err(error) => {
                        let mut inner = heal.inner.lock().unwrap();
                        inner.starting = false;
                        inner.error = Some(error.clone());
                        log::warn!("webui: re-join failed: {error}");
                    }
                }
            });
    }

    let app = Router::new()
        .route("/", get(index))
        .route("/api/status", get(status))
        .route("/api/start", post(start))
        .route("/api/stop", post(leave))
        .route("/api/controls", post(patch_controls))
        .route("/api/streams/{id}", post(patch_stream))
        .with_state(state.clone());

    let listen = args.listen.clone();
    let server_stop = stop.clone();
    let server = std::thread::Builder::new()
        .name("siren-webui".into())
        .spawn(move || -> anyhow::Result<()> {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?;
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind(&listen).await?;
                println!("webui: listening on http://{listen}");
                axum::serve(listener, app)
                    .with_graceful_shutdown(async move {
                        while !server_stop.load(Ordering::Relaxed) {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    })
                    .await?;
                Ok::<(), anyhow::Error>(())
            })
        })
        .map_err(|e| anyhow::anyhow!("spawn webui thread: {e}"))?;

    util::wait(&stop, None);
    println!("shutting down…");
    let node = {
        let mut inner = state.inner.lock().unwrap();
        inner.node.take()
    };
    if let Some(node) = node {
        node.stop();
    }
    drop(isolation);
    let _ = server.join();
    Ok(())
}
