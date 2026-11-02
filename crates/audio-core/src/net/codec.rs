//! Raw PCM and Opus payload codecs.
//!
//! Opus is driven through `opus-head-sys` (vendored libopus 1.6). The thin
//! wrappers below own the C state and keep all `unsafe` in one place.

use opus_head_sys as ffi;

use crate::format::{bytes_to_pcm, pcm_to_bytes, CHANNELS, SAMPLE_RATE};
use crate::protocol::Codec;
use crate::{CoreError, Result};

const OPUS_BITRATE: i32 = 192_000;

fn configured_bitrate() -> i32 {
    std::env::var("SIREN_OPUS_BITRATE")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| (6_000..=510_000).contains(value))
        .unwrap_or(OPUS_BITRATE)
}
const OPUS_COMPLEXITY: i32 = 5;
const MAX_OPUS_PACKET: usize = 1500;
const MAX_DECODE_FRAMES: usize = SAMPLE_RATE as usize * 120 / 1000;

pub struct AudioEncoder {
    codec: Codec,
    opus: Option<OpusEncoder>,
}

impl AudioEncoder {
    pub fn new(codec: Codec) -> Result<Self> {
        Self::with_bitrate(codec, None)
    }

    /// `bitrate` overrides `SIREN_OPUS_BITRATE`; the Bluetooth carrier uses a
    /// low rate so a narrow RFCOMM link can carry every frame.
    pub fn with_bitrate(codec: Codec, bitrate: Option<i32>) -> Result<Self> {
        let opus = match codec {
            Codec::RawF32 => None,
            Codec::Opus => Some(OpusEncoder::new(bitrate)?),
        };
        Ok(Self { codec, opus })
    }

    /// Encode one interleaved stereo frame into `out`.
    pub fn encode(&mut self, pcm: &[f32], out: &mut Vec<u8>) -> Result<usize> {
        match self.codec {
            Codec::RawF32 => {
                pcm_to_bytes(pcm, out);
                Ok(out.len())
            }
            Codec::Opus => {
                out.resize(MAX_OPUS_PACKET, 0);
                let written = self.opus.as_mut().unwrap().encode(pcm, out)?;
                out.truncate(written);
                Ok(written)
            }
        }
    }
}

pub struct AudioDecoder {
    codec: Codec,
    opus: Option<OpusDecoder>,
    buf: Vec<f32>,
    decoded_any: bool,
}

impl AudioDecoder {
    pub fn new(codec: Codec) -> Result<Self> {
        let opus = match codec {
            Codec::RawF32 => None,
            Codec::Opus => Some(OpusDecoder::new()?),
        };
        Ok(Self {
            codec,
            opus,
            buf: vec![0.0; MAX_DECODE_FRAMES * CHANNELS],
            decoded_any: false,
        })
    }

    pub fn decode(&mut self, payload: &[u8]) -> Result<&[f32]> {
        match self.codec {
            Codec::RawF32 => {
                if payload.len() % (CHANNELS * 4) != 0 {
                    return Err(CoreError::Codec(format!(
                        "raw payload is not a whole stereo frame: {} bytes",
                        payload.len()
                    )));
                }
                bytes_to_pcm(payload, &mut self.buf);
                Ok(&self.buf)
            }
            Codec::Opus => {
                let max_frames = self.buf.len() / CHANNELS;
                let frames =
                    self.opus
                        .as_mut()
                        .unwrap()
                        .decode(Some(payload), &mut self.buf, max_frames)?;
                self.decoded_any = true;
                Ok(&self.buf[..frames * CHANNELS])
            }
        }
    }

    /// Generate one concealment frame (Opus PLC). Returns `None` if no packet
    /// has been decoded yet.
    pub fn conceal(&mut self, frame_frames: usize) -> Result<Option<&[f32]>> {
        if self.codec != Codec::Opus || !self.decoded_any {
            return Ok(None);
        }
        let frames = self
            .opus
            .as_mut()
            .unwrap()
            .decode(None, &mut self.buf, frame_frames)?;
        Ok(Some(&self.buf[..frames * CHANNELS]))
    }

    pub fn reset(&mut self) {
        if let Some(decoder) = self.opus.as_mut() {
            decoder.reset();
        }
        self.decoded_any = false;
    }

    pub fn has_decoded(&self) -> bool {
        self.decoded_any
    }
}

struct OpusEncoder {
    ptr: *mut ffi::OpusEncoder,
}

// The encoder is only ever used from the thread that owns it; moving that
// ownership is fine.
unsafe impl Send for OpusEncoder {}

impl OpusEncoder {
    fn new(bitrate_override: Option<i32>) -> Result<Self> {
        let mut error = 0i32;
        let ptr = unsafe {
            ffi::opus_encoder_create(
                SAMPLE_RATE as i32,
                CHANNELS as i32,
                ffi::OPUS_APPLICATION_RESTRICTED_LOWDELAY as i32,
                &mut error,
            )
        };
        if ptr.is_null() || error != ffi::OPUS_OK as i32 {
            return Err(CoreError::Codec(format!(
                "opus encoder init failed ({error})"
            )));
        }
        let encoder = Self { ptr };
        unsafe {
            ffi::opus_encoder_ctl(
                ptr,
                ffi::OPUS_SET_BITRATE_REQUEST as i32,
                bitrate_override.unwrap_or_else(configured_bitrate),
            );
            ffi::opus_encoder_ctl(
                ptr,
                ffi::OPUS_SET_COMPLEXITY_REQUEST as i32,
                OPUS_COMPLEXITY,
            );
            ffi::opus_encoder_ctl(
                ptr,
                ffi::OPUS_SET_SIGNAL_REQUEST as i32,
                ffi::OPUS_SIGNAL_MUSIC as i32,
            );
        }
        Ok(encoder)
    }

    fn encode(&mut self, pcm: &[f32], out: &mut [u8]) -> Result<usize> {
        let frames = pcm.len() / CHANNELS;
        let written = unsafe {
            ffi::opus_encode_float(
                self.ptr,
                pcm.as_ptr(),
                frames as i32,
                out.as_mut_ptr(),
                out.len() as i32,
            )
        };
        if written < 0 {
            return Err(CoreError::Codec(format!("opus encode failed ({written})")));
        }
        Ok(written as usize)
    }
}

impl Drop for OpusEncoder {
    fn drop(&mut self) {
        unsafe { ffi::opus_encoder_destroy(self.ptr) };
    }
}

struct OpusDecoder {
    ptr: *mut ffi::OpusDecoder,
}

unsafe impl Send for OpusDecoder {}

impl OpusDecoder {
    fn new() -> Result<Self> {
        let mut error = 0i32;
        let ptr =
            unsafe { ffi::opus_decoder_create(SAMPLE_RATE as i32, CHANNELS as i32, &mut error) };
        if ptr.is_null() || error != ffi::OPUS_OK as i32 {
            return Err(CoreError::Codec(format!(
                "opus decoder init failed ({error})"
            )));
        }
        Ok(Self { ptr })
    }

    /// Decode a packet, or run packet-loss concealment when `payload` is None.
    fn decode(
        &mut self,
        payload: Option<&[u8]>,
        out: &mut [f32],
        max_frames: usize,
    ) -> Result<usize> {
        let (data, len) = match payload {
            Some(bytes) => (bytes.as_ptr(), bytes.len() as i32),
            None => (std::ptr::null(), 0),
        };
        let frame_capacity = max_frames.min(out.len() / CHANNELS) as i32;
        let frames = unsafe {
            ffi::opus_decode_float(self.ptr, data, len, out.as_mut_ptr(), frame_capacity, 0)
        };
        if frames < 0 {
            return Err(CoreError::Codec(format!("opus decode failed ({frames})")));
        }
        Ok(frames as usize)
    }

    fn reset(&mut self) {
        unsafe {
            ffi::opus_decoder_ctl(self.ptr, ffi::OPUS_RESET_STATE as i32);
        }
    }
}

impl Drop for OpusDecoder {
    fn drop(&mut self) {
        unsafe { ffi::opus_decoder_destroy(self.ptr) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::ToneSource;
    use crate::format::{frames_for_ms, CHANNELS};

    #[test]
    fn raw_roundtrip() {
        let mut pcm = vec![0.0f32; frames_for_ms(5.0) * CHANNELS];
        ToneSource::new(440.0, 0.25).fill(&mut pcm);
        let mut encoder = AudioEncoder::new(Codec::RawF32).unwrap();
        let mut decoder = AudioDecoder::new(Codec::RawF32).unwrap();
        let mut payload = Vec::new();
        encoder.encode(&pcm, &mut payload).unwrap();
        let decoded = decoder.decode(&payload).unwrap();
        assert_eq!(decoded.len(), pcm.len());
        assert!(decoded
            .iter()
            .zip(pcm.iter())
            .all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn opus_roundtrip_reconstructs_a_tone() {
        let frames = frames_for_ms(10.0);
        let mut pcm = vec![0.0f32; frames * CHANNELS];
        ToneSource::new(440.0, 0.25).fill(&mut pcm);
        let mut encoder = AudioEncoder::new(Codec::Opus).unwrap();
        let mut decoder = AudioDecoder::new(Codec::Opus).unwrap();
        let mut payload = Vec::new();
        encoder.encode(&pcm, &mut payload).unwrap();
        assert!(
            payload.len() < 800,
            "unexpected opus size {}",
            payload.len()
        );

        let decoded = decoder.decode(&payload).unwrap();
        assert_eq!(decoded.len(), pcm.len());
        let energy: f32 = decoded.iter().map(|s| s * s).sum();
        assert!(energy > 1.0, "decoded signal is silent");
        let plc = decoder.conceal(frames).unwrap().unwrap();
        assert_eq!(plc.len(), pcm.len());
    }
}

#[cfg(test)]
mod bitrate_tests {
    use super::*;

    fn tone_rms(bitrate: i32) -> f32 {
        let frame = crate::format::frames_for_ms(5.0) * CHANNELS;
        let pcm: Vec<f32> = (0..frame)
            .map(|index| {
                let sample = index / CHANNELS;
                let phase = 2.0 * std::f32::consts::PI * 440.0 * sample as f32 / SAMPLE_RATE as f32;
                phase.sin() * 0.25
            })
            .collect();
        let mut encoder = AudioEncoder::with_bitrate(Codec::Opus, Some(bitrate)).expect("encoder");
        let mut decoder = AudioDecoder::new(Codec::Opus).expect("decoder");
        let mut payload = Vec::new();
        encoder.encode(&pcm, &mut payload).expect("encode");
        let decoded = decoder.decode(&payload).expect("decode");
        let power: f32 = decoded.iter().map(|s| s * s).sum::<f32>() / decoded.len() as f32;
        power.sqrt()
    }

    #[test]
    fn opus_32k_and_64k_encode_real_audio() {
        let rms32 = tone_rms(32_000);
        let rms64 = tone_rms(64_000);
        println!("rms 32k={rms32:.4} 64k={rms64:.4}");
        assert!(rms32 > 0.05, "32 kbps decoded near-silence: {rms32}");
        assert!(rms64 > 0.05, "64 kbps decoded near-silence: {rms64}");
    }
}
