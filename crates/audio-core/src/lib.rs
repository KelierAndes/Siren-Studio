//! SirenStudio audio routing core.
//!
//! The crate is split into platform-independent audio/DSP/network building
//! blocks plus thin platform adapters:
//!
//! - [`format`] fixed 48 kHz / stereo / f32 transport format
//! - [`protocol`] custom UDP frame header
//! - [`dsp`] lock-free stream channels, drift correction and the mixer
//! - [`net`] UDP transport, Opus codec and mDNS discovery
//! - [`capture`] tone generator plus WASAPI loopback/microphone capture
//! - [`output`] playback and WAV recording
//! - [`analyze`] offline verification helpers

pub mod analyze;
pub mod bt;
pub mod capture;
pub mod dsp;
pub mod engine;
pub mod format;
pub mod hybrid;
pub mod mesh;
pub mod metrics;
pub mod net;
pub mod output;
pub mod platform;
pub mod protocol;
pub mod rfcomm;
pub mod types;

pub use format::{CHANNELS, SAMPLE_RATE};

/// Errors raised by the audio core.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] protocol::ProtocolError),
    #[error("audio backend error: {0}")]
    Backend(String),
    #[error("audio codec error: {0}")]
    Codec(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("service discovery error: {0}")]
    Discovery(String),
    #[error("wav error: {0}")]
    Wav(#[from] hound::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
