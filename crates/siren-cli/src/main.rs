mod bt;
mod center;
mod isolate;
mod mesh;
mod misc;
mod rfcomm;
mod send;
mod util;
mod webui;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "siren",
    version,
    about = "SirenStudio — LAN audio routing and mixing",
    long_about = "Route system audio from many devices to one center node and mix it in real time.\n\
                  Nodes speak 48 kHz stereo over a custom low-latency UDP protocol (raw f32 or Opus)."
)]
struct Cli {
    /// Log level: error, warn, info, debug, trace.
    #[arg(long, global = true, default_value = "info")]
    log: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a center: receive every sender, mix, play and/or record.
    Center(center::CenterArgs),
    /// Send this device's audio (system loopback, microphone or a test tone).
    Send(send::SendArgs),
    /// Run a headless decentralized mesh node (every node sends and mixes).
    Mesh(mesh::MeshArgs),
    /// Serve the browser control panel for a headless mesh node.
    Web(webui::WebArgs),
    /// Run a Bluetooth-only multi-hop node (RFCOMM tunnel, TTL/origin routing).
    Bt(bt::BtArgs),
    /// Bridge an RFCOMM link to a local TCP byte stream for `siren bt`.
    Rfcomm(rfcomm::RfcommArgs),
    /// Classic Bluetooth inquiry: list discoverable devices nearby.
    BtScan {
        #[arg(long, default_value_t = 6)]
        seconds: u8,
        /// Make this adapter discoverable first (hub-side check).
        #[arg(long)]
        discoverable: bool,
    },
    /// List playback devices visible to the audio backend.
    Devices,
    /// Browse the LAN for Siren nodes.
    Discover {
        #[arg(long, default_value_t = 4.0)]
        seconds: f64,
        #[arg(long)]
        json: bool,
    },
    /// Analyze a recorded WAV: RMS, peak, silence blocks and tone levels.
    Analyze {
        file: String,
        /// Frequency to measure; repeatable.
        #[arg(long = "freq")]
        freqs: Vec<f64>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 0.5)]
        skip_start: f64,
        #[arg(long, default_value_t = 0.25)]
        skip_end: f64,
        #[arg(long, default_value_t = -45.0, allow_hyphen_values = true)]
        presence_db: f64,
    },
    /// Play a test tone through the default output (used to test loopback).
    Playtone {
        #[arg(long, default_value_t = 1000.0)]
        freq: f64,
        #[arg(long, default_value_t = 0.2)]
        amplitude: f32,
        #[arg(long, default_value_t = 3.0)]
        duration: f64,
        #[arg(long)]
        device: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    let filter = format!("siren={level},audio_core={level},warn", level = cli.log);
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(filter))
        .format_timestamp_millis()
        .init();

    let result = match cli.command {
        Command::Center(args) => center::run(args),
        Command::Send(args) => send::run(args),
        Command::Mesh(args) => mesh::run(args),
        Command::Web(args) => webui::run(args),
        Command::Bt(args) => bt::run(args),
        Command::Rfcomm(args) => rfcomm::run(args),
        Command::BtScan {
            seconds,
            discoverable,
        } => {
            use audio_core::rfcomm::{BtTransport, NativeBt};
            let transport = NativeBt;
            if discoverable {
                println!("discoverable: {}", transport.set_discoverable(180));
            }
            let devices = transport.scan(seconds);
            if devices.is_empty() {
                println!("(no discoverable Bluetooth devices found)");
            }
            for device in devices {
                println!("{}  {}", device.address, device.name);
            }
            Ok(())
        }
        Command::Devices => misc::devices(),
        Command::Discover { seconds, json } => misc::discover(seconds, json),
        Command::Analyze {
            file,
            freqs,
            json,
            skip_start,
            skip_end,
            presence_db,
        } => misc::analyze(&file, &freqs, json, skip_start, skip_end, presence_db),
        Command::Playtone {
            freq,
            amplitude,
            duration,
            device,
        } => misc::playtone(freq, amplitude, duration, device),
    };

    if let Err(error) = result {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
