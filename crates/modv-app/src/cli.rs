//! Command-line interface for the M0 spike binary.
//!
//! Parsed with clap's builder API: the `derive` feature of clap is not
//! enabled in this workspace and `Cargo.toml` is frozen for this milestone.
//! The `--socket`/`--state` flags are accepted for forward compatibility
//! with M1 (WS-14) and are unused here. `--emit-fixtures` writes the M2 parity
//! fixtures and exits before any GPU/window work (see [`crate::fixtures`]).

use std::path::PathBuf;

use clap::{Arg, ArgAction, Command, value_parser};

/// Parsed command-line options for `modv-engine`.
#[derive(Clone, Debug)]
pub struct Cli {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Borderless fullscreen on the primary monitor.
    pub fullscreen: bool,
    /// Stop after this many frames. Defaults to "until window closed" in
    /// windowed mode, and 600 in headless mode.
    pub frames: Option<u64>,
    /// No window; render offscreen and report. Implies `--bench`.
    pub headless: bool,
    /// Print the JSON frame report to stdout on exit.
    pub bench: bool,
    /// Windowed mode: stop after this many wall seconds. `0` disables the
    /// limit (run until the window is closed).
    pub bench_seconds: f64,
    /// Headless render, save the final frame as PNG, then exit.
    pub screenshot: Option<PathBuf>,
    /// Frequency of the synthetic sine audio fixture in Hz. `0` feeds silence.
    pub audio_fixture: f64,
    /// Print adapter info as JSON and exit.
    pub print_adapter: bool,
    /// Dump the adapter info JSON to stderr at startup (a one-line summary is
    /// always logged to stderr regardless of this flag).
    pub log_adapter: bool,
    /// Reserved for M1 (WS-14): control socket path. Accepted but unused here.
    pub socket: Option<PathBuf>,
    /// Reserved for M1: state snapshot path. Accepted but unused here.
    pub state: Option<PathBuf>,
    /// Write the deterministic store fixtures (`deltas.json` +
    /// `snapshot.json`) into this directory and exit. Runs before any GPU,
    /// window or socket setup — see [`crate::fixtures`].
    pub emit_fixtures: Option<PathBuf>,
}

impl Cli {
    /// The number of frames to render in headless mode (default: 600).
    pub fn headless_frames(&self) -> u64 {
        self.frames.unwrap_or(600)
    }

    /// The wall-clock deadline for windowed mode, or `None` when disabled.
    pub fn bench_deadline(&self) -> Option<std::time::Duration> {
        if self.bench_seconds > 0.0 {
            Some(std::time::Duration::from_secs_f64(self.bench_seconds))
        } else {
            None
        }
    }
}

/// Parse the process arguments.
pub fn parse() -> Cli {
    let matches = Command::new("modv-engine")
        .version(env!("CARGO_PKG_VERSION"))
        .about("modV engine — Rust hybrid spike (docs/rust-hybrid.md, M0)")
        .arg(
            Arg::new("width")
                .long("width")
                .value_name("N")
                .help("Output width")
                .default_value("1280")
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("height")
                .long("height")
                .value_name("N")
                .help("Output height")
                .default_value("720")
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("fullscreen")
                .long("fullscreen")
                .help("Borderless fullscreen on the primary monitor")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("frames")
                .long("frames")
                .value_name("N")
                .help("Stop after N frames (windowed: default until window closed; headless: default 600)")
                .value_parser(value_parser!(u64)),
        )
        .arg(
            Arg::new("headless")
                .long("headless")
                .help("No window; render offscreen and report (implies --bench)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("bench")
                .long("bench")
                .help("Print the JSON FrameReport to stdout on exit")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("bench-seconds")
                .long("bench-seconds")
                .value_name("S")
                .help("Windowed: stop after S wall seconds (0 = run until window closed)")
                .default_value("10")
                .value_parser(value_parser!(f64)),
        )
        .arg(
            Arg::new("screenshot")
                .long("screenshot")
                .value_name("PATH")
                .help("Headless render, save the final frame as PNG, exit"),
        )
        .arg(
            Arg::new("audio-fixture")
                .long("audio-fixture")
                .value_name("HZ")
                .help("Feed a sine wave at HZ frequency (0 = silence)")
                .default_value("110")
                .value_parser(value_parser!(f64)),
        )
        .arg(
            Arg::new("print-adapter")
                .long("print-adapter")
                .help("Print adapter info as JSON and exit")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("log-adapter")
                .long("log-adapter")
                .help("Dump the adapter info JSON to stderr at startup (a one-line summary is always logged)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("socket")
                .long("socket")
                .value_name("PATH")
                .help("Reserved for M1 (WS-14): control socket path"),
        )
        .arg(
            Arg::new("state")
                .long("state")
                .value_name("PATH")
                .help("Reserved for M1: state snapshot path"),
        )
        .arg(
            Arg::new("emit-fixtures")
                .long("emit-fixtures")
                .value_name("DIR")
                .help("Write the deterministic store fixtures (deltas.json, snapshot.json) into DIR and exit")
                .value_parser(value_parser!(PathBuf)),
        )
        .get_matches();

    let socket = matches.get_one::<String>("socket").map(PathBuf::from);
    let mut bench_seconds = *matches
        .get_one::<f64>("bench-seconds")
        .expect("clap default for bench-seconds");
    // Supervisor mode (spawned by Electron with --socket): the engine runs
    // until engine/shutdown, never on the M0 benchmark clock. An explicit
    // --bench-seconds is still honoured.
    if socket.is_some()
        && matches.value_source("bench-seconds") == Some(clap::parser::ValueSource::DefaultValue)
    {
        bench_seconds = 0.0;
    }

    Cli {
        width: *matches
            .get_one::<u32>("width")
            .expect("clap default for width"),
        height: *matches
            .get_one::<u32>("height")
            .expect("clap default for height"),
        fullscreen: matches.get_flag("fullscreen"),
        frames: matches.get_one::<u64>("frames").copied(),
        headless: matches.get_flag("headless"),
        bench: matches.get_flag("bench"),
        bench_seconds,
        screenshot: matches.get_one::<String>("screenshot").map(PathBuf::from),
        audio_fixture: *matches
            .get_one::<f64>("audio-fixture")
            .expect("clap default for audio-fixture"),
        print_adapter: matches.get_flag("print-adapter"),
        log_adapter: matches.get_flag("log-adapter"),
        socket,
        state: matches.get_one::<String>("state").map(PathBuf::from),
        emit_fixtures: matches.get_one::<PathBuf>("emit-fixtures").cloned(),
    }
}
