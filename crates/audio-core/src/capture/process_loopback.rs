//! WASAPI process-loopback capture with this process excluded.
//!
//! Windows 10 build 20348+ / Windows 11 can activate a virtual capture
//! endpoint that mirrors every render stream *except* a target process tree.
//! Excluding ourselves keeps the mesh monitor output out of the capture at the
//! audio-engine level, so nothing this node plays can be sent back to the
//! other nodes — no DSP needed, no residual leakage.
//!
//! The virtual endpoint is not event driven: callers poll [`next`].

use wasapi::{
    AudioCaptureClient, AudioClient, BufferInfo, Direction, SampleType, StreamMode, WaveFormat,
};

use crate::format::{CHANNELS, SAMPLE_RATE};
use crate::{CoreError, Result};

const BYTES_PER_SAMPLE: usize = 4;

/// Loopback stream that omits the target process tree.
pub struct ProcessLoopback {
    client: AudioClient,
    capture: AudioCaptureClient,
    bytes: Vec<u8>,
}

impl ProcessLoopback {
    /// Opens the virtual endpoint, excluding `exclude_pid` and its children.
    pub fn open(exclude_pid: u32) -> Result<Self> {
        let mut client = AudioClient::new_application_loopback_client(exclude_pid, false)
            .map_err(|e| CoreError::Backend(format!("process loopback activation: {e}")))?;
        let format = WaveFormat::new(
            32,
            32,
            &SampleType::Float,
            SAMPLE_RATE as usize,
            CHANNELS,
            None,
        );
        let mode = StreamMode::PollingShared {
            autoconvert: false,
            buffer_duration_hns: 200_000,
        };
        client
            .initialize_client(&format, &Direction::Capture, &mode)
            .map_err(|e| CoreError::Backend(format!("process loopback format: {e}")))?;
        let capture = client
            .get_audiocaptureclient()
            .map_err(|e| CoreError::Backend(format!("process loopback capture client: {e}")))?;
        client
            .start_stream()
            .map_err(|e| CoreError::Backend(format!("process loopback start: {e}")))?;
        Ok(Self {
            client,
            capture,
            bytes: Vec::new(),
        })
    }

    /// Pulls the next available packet into `out`. `false` means no packet.
    pub fn next(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        let frames = match self.capture.get_next_packet_size() {
            Ok(Some(frames)) => frames as usize,
            Ok(None) => return Ok(false),
            Err(e) => {
                return Err(CoreError::Backend(format!(
                    "process loopback packet size: {e}"
                )))
            }
        };
        self.bytes.resize(frames * CHANNELS * BYTES_PER_SAMPLE, 0);
        let (read, info): (u32, BufferInfo) = self
            .capture
            .read_from_device(&mut self.bytes)
            .map_err(|e| CoreError::Backend(format!("process loopback read: {e}")))?;
        let read = read as usize;
        out.clear();
        if info.flags.silent {
            out.resize(read * CHANNELS, 0.0);
        } else {
            out.extend(
                self.bytes[..read * CHANNELS * BYTES_PER_SAMPLE]
                    .chunks_exact(BYTES_PER_SAMPLE)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            );
        }
        Ok(true)
    }

    pub fn stop(&self) {
        let _ = self.client.stop_stream();
    }
}
