//! Linux capture isolation via PipeWire's PulseAudio layer.
//!
//! A virtual sink becomes the default device, so every other application
//! plays into it; the node captures that sink's monitor and routes it to the
//! real playback sink with a loopback. The node's own monitor output goes
//! straight to the playback sink, so it never enters the captured monitor.
//!
//! The net effect is structural self-echo removal (no DSP): capture is the
//! other applications only. Everything is created through `pactl`, so it
//! works with both PipeWire's pulse layer and a plain PulseAudio server
//! (the latter is rejected because it cannot isolate by node).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

const LOCK_NAME: &str = "siren-isolate.pid";
const DEFAULT_CAPTURE_SINK: &str = "siren_capture";

/// Owns the virtual sink, its loopback and the temporary default-sink change.
pub struct Isolation {
    capture_sink: String,
    playback_sink: String,
    /// Set only when this process created the loopback / sink.
    loopback_module: Option<String>,
    sink_module: Option<String>,
    lock_path: PathBuf,
}

impl Isolation {
    pub fn capture_sink(&self) -> &str {
        &self.capture_sink
    }

    pub fn playback_sink(&self) -> &str {
        &self.playback_sink
    }
}

impl Drop for Isolation {
    fn drop(&mut self) {
        if let Some(module) = &self.loopback_module {
            let _ = pactl(&["unload-module", module]);
        }
        let _ = pactl(&["set-default-sink", &self.playback_sink]);
        if let Some(module) = &self.sink_module {
            let _ = pactl(&["unload-module", module]);
        }
        let _ = std::fs::remove_file(&self.lock_path);
        log::info!(
            "isolation: removed virtual sink {}, default sink restored to {}",
            self.capture_sink,
            self.playback_sink
        );
    }
}

/// Returns `Ok(None)` for `off`, or an active isolation that must stay alive
/// for as long as the node runs.
pub fn setup(
    mode: Option<&str>,
    playback_sink: Option<&str>,
    capture_sink: Option<&str>,
) -> Result<Option<Isolation>> {
    let mode = mode.unwrap_or("off").to_ascii_lowercase();
    if matches!(mode.as_str(), "off" | "none" | "no") {
        return Ok(None);
    }
    if mode != "pipewire" {
        bail!("unknown isolation mode '{mode}' (use pipewire or off)");
    }
    if !cfg!(target_os = "linux") {
        bail!("pipewire isolation is Linux-only");
    }

    let info = pactl(&["info"]).context("PulseAudio/PipeWire is not reachable (pactl info)")?;
    if !info.to_ascii_lowercase().contains("pipewire") {
        bail!("isolated capture needs PipeWire's pulse layer (server is not PipeWire)");
    }
    let default_sink = info
        .lines()
        .find_map(|line| line.strip_prefix("Default Sink: "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let playback_sink = playback_sink
        .map(str::to_string)
        .or_else(|| default_sink.clone())
        .ok_or_else(|| anyhow!("no default sink to play into; pass --output-sink"))?;
    let capture_sink = capture_sink
        .unwrap_or(DEFAULT_CAPTURE_SINK)
        .trim()
        .to_string();
    if capture_sink.is_empty() || capture_sink == playback_sink {
        bail!("capture and playback sinks must be different");
    }

    let lock_path = lock_path()?;
    acquire_lock(&lock_path)?;

    let mut sink_module = None;
    let mut loopback_module = None;
    let result = (|| -> Result<Isolation> {
        let sinks = pactl(&["list", "short", "sinks"])?;
        let sink_exists = sinks
            .lines()
            .any(|line| line.split('\t').nth(1) == Some(capture_sink.as_str()));
        if !sink_exists {
            let id = pactl(&[
                "load-module",
                "module-null-sink",
                &format!("sink_name={capture_sink}"),
                "rate=48000",
                "channels=2",
                "sink_properties=device.description=SirenCapture",
            ])?;
            sink_module = Some(id.trim().to_string());
        }

        let source = format!("{capture_sink}.monitor");
        let modules = pactl(&["list", "short", "modules"])?;
        let existing_loopback = modules.lines().find_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let id = parts.next()?;
            let name = parts.next()?;
            let args = parts.next().unwrap_or("");
            (name == "module-loopback"
                && args.contains(&format!("source={source}"))
                && args.contains(&format!("sink={playback_sink}")))
            .then(|| id.to_string())
        });
        if existing_loopback.is_none() {
            let id = pactl(&[
                "load-module",
                "module-loopback",
                &format!("source={source}"),
                &format!("sink={playback_sink}"),
                "latency_msec=20",
            ])?;
            loopback_module = Some(id.trim().to_string());
        }

        pactl(&["set-default-sink", &capture_sink])?;

        // Capture reads the explicit monitor; playback goes to the playback
        // sink. The DSP must stay out of the way (its reference would be
        // silent anyway, but skipping it saves a core).
        std::env::set_var("SIREN_CAPTURE_SOURCE", &source);
        std::env::set_var("SIREN_ECHO_CANCEL", "0");
        std::env::set_var("PULSE_SINK", &playback_sink);

        log::info!(
            "isolation: capture {} (monitor) -> playback {}, other apps are captured only",
            capture_sink,
            playback_sink
        );
        Ok(Isolation {
            capture_sink: capture_sink.clone(),
            playback_sink: playback_sink.clone(),
            loopback_module: loopback_module.clone(),
            sink_module: sink_module.clone(),
            lock_path: lock_path.clone(),
        })
    })();

    match result {
        Ok(isolation) => Ok(Some(isolation)),
        Err(error) => {
            if let Some(module) = &loopback_module {
                let _ = pactl(&["unload-module", module]);
            }
            if let Some(module) = &sink_module {
                let _ = pactl(&["unload-module", module]);
            }
            let _ = std::fs::remove_file(&lock_path);
            Err(error)
        }
    }
}

fn lock_path() -> Result<PathBuf> {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    Ok(PathBuf::from(dir).join(LOCK_NAME))
}

fn acquire_lock(path: &Path) -> Result<()> {
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(pid) = text.trim().parse::<u32>() {
            if Path::new(&format!("/proc/{pid}")).exists() {
                bail!("another isolated Siren node is running (pid {pid})");
            }
        }
        let _ = std::fs::remove_file(path);
    }
    std::fs::write(path, std::process::id().to_string())
        .with_context(|| format!("write isolation lock {}", path.display()))?;
    Ok(())
}

fn pactl(args: &[&str]) -> Result<String> {
    let run = |program: &str| Command::new(program).args(args).output();
    let output = run("pactl")
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                run("/usr/bin/pactl")
            } else {
                Err(error)
            }
        })
        .map_err(|error| anyhow!("run pactl: {error}"))?;
    if !output.status.success() {
        bail!(
            "pactl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
