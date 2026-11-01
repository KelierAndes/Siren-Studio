pub mod adapter;
pub mod cpal_out;
#[cfg(target_os = "linux")]
pub mod pulse_out;
pub mod wav;

#[cfg(not(target_os = "linux"))]
pub use cpal_out::{list_output_devices, play_tone, start_playback, Playback};
#[cfg(target_os = "linux")]
pub use pulse_out::{list_output_devices, play_tone, start_playback, Playback};
pub use wav::{record_to_wav, RecordSummary};
