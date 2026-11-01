//! Command/event types shared by every front end.
//!
//! Windows/Android will map these onto `flutter_rust_bridge` structs; the
//! Linux daemon serializes the very same types as JSON over a WebSocket. The
//! shapes are intentionally plain-data only.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineCommand {
    SetGain { stream: u64, gain_db: f32 },
    SetMute { stream: u64, muted: bool },
    SetTargetBufferMs { stream: Option<u64>, ms: f32 },
    SetMasterGainDb { db: f32 },
    RemoveStream { stream: u64 },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    StreamAdded {
        stream: u64,
        label: String,
        addr: String,
        codec: String,
    },
    StreamRemoved {
        stream: u64,
    },
    Stats {
        master: crate::metrics::MasterSnapshot,
        streams: Vec<crate::metrics::StreamSnapshot>,
    },
    Error {
        message: String,
    },
}
