//! Playback through the platform's default output device.
//!
//! The mixer always runs at 48 kHz stereo. `OutputAdapter` renders on demand
//! and linearly resamples/maps to whatever the device accepts, so a 44.1 kHz
//! or mono endpoint still works. (The monitoring path is not the hi-fi path;
//! the recording sink writes the untouched 48 kHz mix.)

use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};

use super::adapter::OutputAdapter;
use crate::dsp::Mixer;
use crate::{CoreError, Result};

pub struct Playback {
    _stream: cpal::Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Playback {
    pub fn describe(&self) -> String {
        format!(
            "{} @ {} Hz / {} ch",
            self.device_name, self.sample_rate, self.channels
        )
    }
}

pub fn list_output_devices() -> Result<Vec<(String, bool)>> {
    let host = cpal::default_host();
    let default = host.default_output_device().map(|d| d.to_string());
    let mut devices = Vec::new();
    for device in host
        .output_devices()
        .map_err(|e| CoreError::Backend(format!("enumerate output devices: {e}")))?
    {
        let name = device.to_string();
        let is_default = default.as_deref() == Some(name.as_str());
        devices.push((name, is_default));
    }
    Ok(devices)
}

fn pick_device(query: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(query) = query.filter(|q| !q.is_empty()) {
        let needle = query.to_lowercase();
        for device in host
            .output_devices()
            .map_err(|e| CoreError::Backend(format!("enumerate output devices: {e}")))?
        {
            if device.to_string().to_lowercase().contains(&needle) {
                return Ok(device);
            }
        }
        return Err(CoreError::Backend(format!(
            "no output device matches '{query}'"
        )));
    }
    host.default_output_device()
        .ok_or_else(|| CoreError::Backend("no default output device".into()))
}

pub fn start_playback(mixer: Mixer, device_query: Option<&str>) -> Result<Playback> {
    let device = pick_device(device_query)?;
    let device_name = device.to_string();
    let supported = device
        .default_output_config()
        .map_err(|e| CoreError::Backend(format!("default output config: {e}")))?;
    let sample_format = supported.sample_format();
    let config = supported.config();
    let sample_rate = config.sample_rate;
    let channels = config.channels;

    let stream = match sample_format {
        SampleFormat::F32 => build::<f32>(&device, config, mixer),
        SampleFormat::I16 => build::<i16>(&device, config, mixer),
        SampleFormat::U16 => build::<u16>(&device, config, mixer),
        other => Err(CoreError::Backend(format!(
            "unsupported output sample format {other:?}"
        ))),
    }?;
    stream
        .play()
        .map_err(|e| CoreError::Backend(format!("start playback: {e}")))?;

    Ok(Playback {
        _stream: stream,
        device_name,
        sample_rate,
        channels,
    })
}

fn build<T>(device: &cpal::Device, config: StreamConfig, mixer: Mixer) -> Result<cpal::Stream>
where
    T: SizedSample + Sample + FromSample<f32> + 'static,
{
    let out_channels = config.channels as usize;
    let out_rate = config.sample_rate;
    let mut adapter = OutputAdapter::new(mixer, out_rate, out_channels);
    let error_callback = |error: cpal::Error| log::error!("output stream error: {error}");
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _info| adapter.write(data),
            error_callback,
            None,
        )
        .map_err(|e| CoreError::Backend(format!("build output stream: {e}")))
}

/// Standalone sine generator used to test loopback capture end to end.
pub fn play_tone(
    freq: f64,
    amplitude: f32,
    duration: Duration,
    device_query: Option<&str>,
) -> Result<Playback> {
    let device = pick_device(device_query)?;
    let device_name = device.to_string();
    let supported = device
        .default_output_config()
        .map_err(|e| CoreError::Backend(format!("default output config: {e}")))?;
    let sample_format = supported.sample_format();
    let config = supported.config();
    let sample_rate = config.sample_rate;
    let channels = config.channels as usize;

    let stream = match sample_format {
        SampleFormat::F32 => {
            build_tone::<f32>(&device, config, freq, amplitude, sample_rate, channels)
        }
        SampleFormat::I16 => {
            build_tone::<i16>(&device, config, freq, amplitude, sample_rate, channels)
        }
        SampleFormat::U16 => {
            build_tone::<u16>(&device, config, freq, amplitude, sample_rate, channels)
        }
        other => Err(CoreError::Backend(format!(
            "unsupported output sample format {other:?}"
        ))),
    }?;
    stream
        .play()
        .map_err(|e| CoreError::Backend(format!("start playback: {e}")))?;
    std::thread::sleep(duration);
    Ok(Playback {
        _stream: stream,
        device_name,
        sample_rate,
        channels: channels as u16,
    })
}

fn build_tone<T>(
    device: &cpal::Device,
    config: StreamConfig,
    freq: f64,
    amplitude: f32,
    sample_rate: u32,
    channels: usize,
) -> Result<cpal::Stream>
where
    T: SizedSample + Sample + FromSample<f32> + 'static,
{
    let step = 2.0 * std::f64::consts::PI * freq / sample_rate as f64;
    let mut phase = 0f64;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _info| {
                for frame in data.chunks_mut(channels) {
                    let value = (phase.sin() as f32) * amplitude;
                    phase += step;
                    if phase >= 2.0 * std::f64::consts::PI {
                        phase -= 2.0 * std::f64::consts::PI;
                    }
                    for sample in frame.iter_mut() {
                        *sample = T::from_sample(value);
                    }
                }
            },
            |error| log::error!("tone stream error: {error}"),
            None,
        )
        .map_err(|e| CoreError::Backend(format!("build tone stream: {e}")))
}
