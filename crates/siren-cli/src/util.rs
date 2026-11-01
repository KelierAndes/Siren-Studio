use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;

pub fn stop_flag() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let handler = stop.clone();
    let _ = ctrlc::set_handler(move || {
        handler.store(true, Ordering::Relaxed);
        eprintln!("\nstopping…");
    });
    stop
}

pub fn wait(stop: &AtomicBool, duration: Option<f64>) {
    let deadline = duration.map(|seconds| Instant::now() + Duration::from_secs_f64(seconds));
    while !stop.load(Ordering::Relaxed) {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn default_node_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "siren-node".into())
}

pub fn append_json_line(path: &str, value: &impl serde::Serialize) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
