use std::time::Duration;

use anyhow::{Context, Result};
use audio_core::analyze::analyze_wav;
use audio_core::net::discovery;
use audio_core::output::{list_output_devices, play_tone};

pub fn devices() -> Result<()> {
    let devices = list_output_devices().context("cannot enumerate output devices")?;
    if devices.is_empty() {
        println!("no output devices found");
        return Ok(());
    }
    println!("output devices:");
    for (name, is_default) in devices {
        println!("  {} {name}", if is_default { "*" } else { " " });
    }
    println!("\nCapture always uses the Windows default playback device (loopback)");
    println!("or the default recording device (microphone).");
    Ok(())
}

pub fn discover(seconds: f64, json: bool) -> Result<()> {
    let peers = discovery::discover(Duration::from_secs_f64(seconds.max(0.5)))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&peers)?);
        return Ok(());
    }
    if peers.is_empty() {
        println!(
            "no Siren nodes found ({:.1} s) — check the firewall and that a center is running",
            seconds
        );
        return Ok(());
    }
    println!("found {} node(s):", peers.len());
    for peer in peers {
        println!(
            "  {:<8} {:<28} {:>16}:{:<5} codec {}",
            peer.role, peer.name, peer.addr, peer.port, peer.codec
        );
    }
    Ok(())
}

pub fn analyze(
    file: &str,
    freqs: &[f64],
    json: bool,
    skip_start: f64,
    skip_end: f64,
    presence_db: f64,
) -> Result<()> {
    let result = analyze_wav(file, freqs, skip_start, skip_end, presence_db)
        .with_context(|| format!("cannot analyze {file}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    println!(
        "{}: {:.2} s, {} Hz, {} ch",
        result.path, result.duration_s, result.sample_rate, result.channels
    );
    println!(
        "window {:.2}–{:.2} s: rms {:.1} dBFS, peak {:.1} dBFS, dc {:+.6}, clipped {}",
        result.window_start_s,
        result.window_end_s,
        result.rms_db,
        result.peak_db,
        result.dc_offset,
        result.clipped_samples
    );
    println!(
        "silence: {} of {} blocks (5 ms)",
        result.silent_blocks, result.total_blocks
    );
    for tone in &result.tones {
        println!(
            "  {:8.1} Hz (found at {:7.2} Hz): {:7.1} dBFS  amplitude {:.4}  {}",
            tone.freq_hz,
            tone.detected_hz,
            tone.db,
            tone.amplitude,
            if tone.present { "present" } else { "absent" }
        );
    }
    Ok(())
}

pub fn playtone(freq: f64, amplitude: f32, duration: f64, device: Option<String>) -> Result<()> {
    audio_core::platform::begin_high_res_timer();
    println!(
        "playing {:.1} Hz sine at amplitude {:.2} for {:.1} s",
        freq, amplitude, duration
    );
    let playback = play_tone(
        freq,
        amplitude,
        Duration::from_secs_f64(duration.max(0.1)),
        device.as_deref(),
    )?;
    println!("output: {}", playback.describe());
    Ok(())
}
