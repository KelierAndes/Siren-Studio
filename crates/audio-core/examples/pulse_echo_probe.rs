//! Diagnostic: measures the PulseAudio monitor path and the echo canceller
//! without any networking. Plays deterministic noise into the default sink,
//! records its monitor, and prints per-block input/output RMS.
//!
//! Linux only; run with `XDG_RUNTIME_DIR` pointing at a live PulseAudio or
//! PipeWire server.

#[cfg(target_os = "linux")]
fn main() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use libpulse_binding::def::BufferAttr;
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    use libpulse_simple_binding::Simple;

    use audio_core::dsp::echo::{self, EchoCanceller};

    const RATE: u32 = 48_000;
    const CH: usize = 2;
    const PLAY_FRAMES: usize = 120; // 2.5 ms
    const REC_FRAMES: usize = 240; // 5 ms

    fn spec() -> Spec {
        Spec { format: Format::F32le, rate: RATE, channels: CH as u8 }
    }

    fn bytes_for(frames: usize) -> Vec<u8> {
        vec![0u8; frames * CH * 4]
    }

    fn encode(pcm: &[f32], bytes: &mut [u8]) {
        for (index, sample) in pcm.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&sample.to_le_bytes());
        }
    }

    fn decode(bytes: &[u8], pcm: &mut [f32]) {
        for (index, sample) in pcm.iter_mut().enumerate() {
            *sample = f32::from_le_bytes([
                bytes[index * 4],
                bytes[index * 4 + 1],
                bytes[index * 4 + 2],
                bytes[index * 4 + 3],
            ]);
        }
    }

    echo::set_reference_enabled(true);
    let stop = Arc::new(AtomicBool::new(false));

    let play_stop = stop.clone();
    let player = thread::spawn(move || {
        let target = 5 * RATE / 1000 * (CH as u32 * 4);
        let attr = BufferAttr {
            maxlength: target * 4,
            tlength: target,
            prebuf: target,
            minreq: u32::MAX,
            fragsize: u32::MAX,
        };
        let simple = Simple::new(
            None,
            "siren-probe",
            Direction::Playback,
            None,
            "probe tone",
            &spec(),
            None,
            Some(&attr),
        )
        .expect("playback stream");
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut pcm = vec![0.0f32; PLAY_FRAMES * CH];
        let mut bytes = bytes_for(PLAY_FRAMES);
        while !play_stop.load(Ordering::Relaxed) {
            for sample in pcm.iter_mut() {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                *sample = ((seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 0.4;
            }
            encode(&pcm, &mut bytes);
            echo::record(&pcm);
            if simple.write(&bytes).is_err() {
                break;
            }
        }
        let _ = simple.drain();
    });

    thread::sleep(Duration::from_millis(300));
    let attr = BufferAttr {
        maxlength: (RATE as usize * CH * 4 / 4) as u32,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: (REC_FRAMES * CH * 4) as u32,
    };
    let record = Simple::new(
        None,
        "siren-probe",
        Direction::Record,
        Some("@DEFAULT_MONITOR@"),
        "probe capture",
        &spec(),
        None,
        Some(&attr),
    )
    .expect("record stream");

    let mut canceller = EchoCanceller::new();
    let mut bytes = bytes_for(REC_FRAMES);
    let mut pcm = vec![0.0f32; REC_FRAMES * CH];
    let mut sum_in = 0.0f64;
    let mut sum_out = 0.0f64;
    let blocks = 600; // 3 s
    for index in 0..blocks {
        if record.read(&mut bytes).is_err() {
            break;
        }
        decode(&bytes, &mut pcm);
        let before: f64 = pcm.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        canceller.process(&mut pcm);
        let after: f64 = pcm.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        if index >= blocks - 100 {
            sum_in += before;
            sum_out += after;
        }
        if index % 100 == 0 {
            let in_db = 10.0 * (before / pcm.len() as f64).max(1e-12).log10();
            let out_db = 10.0 * (after / pcm.len() as f64).max(1e-12).log10();
            println!("block {index:4}: in {in_db:6.1} dB  out {out_db:6.1} dB");
        }
    }
    let reduction = 10.0 * (sum_in / sum_out.max(1e-12)).log10();
    println!("cancellation: {reduction:.1} dB");
    stop.store(true, Ordering::Relaxed);
    let _ = player.join();
    echo::set_reference_enabled(false);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("pulse_echo_probe is Linux-only");
}
