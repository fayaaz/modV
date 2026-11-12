//! Command-line interface for the M0 spike binary.
//!
//! Parsed with clap's builder API: the `derive` feature of clap is not
//! enabled in this workspace and `Cargo.toml` is frozen for this milestone.
//! The `--socket`/`--state` flags are accepted for forward compatibility
//! with M1 (WS-14) and are unused here. `--emit-fixtures` writes the M2 parity
//! fixtures and exits before any GPU/window work (see [`crate::fixtures`]).

use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, Command, error::ErrorKind, value_parser};

/// Which side owns audio capture and analysis (WS-9).
///
/// The default (`renderer`) keeps the current transport: the renderer's
/// WebAudio pump analyses the audio and commits the payload with
/// `meyda/SET_BANDS` over the control socket. Every other source hands capture
/// to the engine, which then analyses with
/// [`modv_media::audio::engine::AudioFeed`] and ignores the renderer's bands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioSourceSelection {
    /// The renderer's WebAudio pump owns capture and analysis (default).
    #[default]
    Renderer,
    /// The engine captures the default sink's monitor through PipeWire (Linux).
    Pipewire,
    /// The engine captures an input device through cpal.
    Input,
}

impl AudioSourceSelection {
    /// Whether the engine owns capture (and the renderer's pump must stand
    /// down). `Store::set_bands` ignores renderer frames while this is set.
    pub fn engine_owned(self) -> bool {
        !matches!(self, Self::Renderer)
    }

    /// The canonical `--audio-source` spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Renderer => "renderer",
            Self::Pipewire => "pipewire",
            Self::Input => "input",
        }
    }
}

/// A `--audio-source`/`--audio-device` validation failure.
///
/// Rendered by [`parse`] through clap, so it is reported before any GPU,
/// window or socket work (exit code 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioSourceError {
    /// The value is not `renderer`, `pipewire`, `input` or `input:<device>`
    /// (`renderer` takes no device suffix).
    UnknownSource(String),
    /// `input:` was given without a device.
    MissingDevice,
}

impl std::fmt::Display for AudioSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSource(value) => write!(
                f,
                "unknown audio source {value:?} (accepted: renderer, pipewire, input, \
                 input:<device>; the MODV_AUDIO_SOURCE environment variable sets the default)"
            ),
            Self::MissingDevice => write!(
                f,
                "audio source \"input:\" needs a device name or id; run \
                 --list-audio-devices to see the available inputs \
                 (e.g. --audio-source input:alsa_input.pci-0000_00_1f.3.analog-stereo), \
                 or omit the suffix to use the default input device"
            ),
        }
    }
}

impl std::error::Error for AudioSourceError {}

/// Parse one `--audio-source` value.
///
/// Accepted spellings: `renderer`, `pipewire`, `input` (host default input) and
/// `input:<device>` (a device name or id, see [`crate::audio::resolve_input_device`]).
/// The device in the suffix wins over `--audio-device`; `<device>` is trimmed,
/// and an empty suffix is [`AudioSourceError::MissingDevice`].
pub fn parse_audio_source(
    value: &str,
) -> Result<(AudioSourceSelection, Option<String>), AudioSourceError> {
    let (head, device) = match value.split_once(':') {
        Some((head, device)) => (head.trim(), Some(device.trim())),
        None => (value.trim(), None),
    };

    let selection = match head {
        "renderer" => AudioSourceSelection::Renderer,
        "pipewire" => AudioSourceSelection::Pipewire,
        "input" => AudioSourceSelection::Input,
        _ => return Err(AudioSourceError::UnknownSource(value.trim().to_string())),
    };

    let device = match device {
        Some("") => return Err(AudioSourceError::MissingDevice),
        Some(device) => Some(device.to_string()),
        None => None,
    };

    if selection == AudioSourceSelection::Renderer && device.is_some() {
        // `renderer` has no capture of its own, so a device would be silently
        // ignored; treat it as an invalid value instead.
        return Err(AudioSourceError::UnknownSource(value.trim().to_string()));
    }

    Ok((selection, device))
}

/// Resolve the configured source: `--audio-source` first, then
/// `$MODV_AUDIO_SOURCE`, then the `renderer` default.
///
/// An empty environment value is treated as unset, so
/// `MODV_AUDIO_SOURCE= engine` behaves like the default.
pub fn resolve_audio_source(
    explicit: Option<&str>,
    environment: Option<&str>,
) -> Result<(AudioSourceSelection, Option<String>), AudioSourceError> {
    match explicit.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => parse_audio_source(value),
        None => match environment.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => parse_audio_source(value),
            None => Ok((AudioSourceSelection::Renderer, None)),
        },
    }
}

/// Resolve the device for the selected source: the `input:<device>` suffix
/// first, then `--audio-device`, then `$MODV_AUDIO_DEVICE`.
///
/// Empty values are treated as absent, so `input:` alone is the only
/// missing-device error (the default input device needs no name).
pub fn resolve_audio_device(
    inline: Option<String>,
    explicit: Option<&str>,
    environment: Option<&str>,
) -> Option<String> {
    if inline.is_some() {
        return inline;
    }

    explicit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| environment.map(str::trim).filter(|value| !value.is_empty()))
        .map(str::to_string)
}

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
    /// Directory holding the vendored ISF corpus (`*.fs` + `*.vs`), resolved
    /// from `--isf-dir`, then `$MODV_ISF_DIR`, then a development default.
    /// See [`resolve_isf_dir`].
    pub isf_dir: Option<PathBuf>,
    /// Which side owns audio capture/analysis (WS-9):
    /// `--audio-source renderer|pipewire|input|input:<device>`, with
    /// `$MODV_AUDIO_SOURCE` as the fallback. See [`AudioSourceSelection`].
    pub audio_source: AudioSourceSelection,
    /// Input device name or id for `--audio-source input`, or the target sink
    /// node for `--audio-source pipewire` (`--audio-device`). A device in the
    /// `input:<device>` suffix wins over this.
    pub audio_device: Option<String>,
    /// Print the host's audio input devices (`--list-audio-devices`) and exit.
    pub list_audio_devices: bool,
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
        .arg(
            Arg::new("isf-dir")
                .long("isf-dir")
                .value_name("PATH")
                .help("Directory with the ISF corpus (*.fs + optional *.vs); defaults to $MODV_ISF_DIR or a dev path")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("audio-source")
                .long("audio-source")
                .value_name("SOURCE")
                .help("Audio capture owner: renderer (default), pipewire, input or input:<device>; env MODV_AUDIO_SOURCE"),
        )
        .arg(
            Arg::new("audio-device")
                .long("audio-device")
                .value_name("NAME_OR_ID")
                .help("Input device for --audio-source input (or a target sink node for pipewire); env MODV_AUDIO_DEVICE"),
        )
        .arg(
            Arg::new("list-audio-devices")
                .long("list-audio-devices")
                .help("Print the host's audio input devices (name, id, channels, sample rates) and exit")
                .action(ArgAction::SetTrue),
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

    // WS-9: `--audio-source` wins over `$MODV_AUDIO_SOURCE`; `input:<device>`
    // wins over `--audio-device` (which itself falls back to
    // `$MODV_AUDIO_DEVICE`). Validation failures exit through clap with a
    // clear message before any GPU/window work.
    let environment_source = std::env::var("MODV_AUDIO_SOURCE").ok();
    let (audio_source, inline_device) = match resolve_audio_source(
        matches
            .get_one::<String>("audio-source")
            .map(String::as_str),
        environment_source.as_deref(),
    ) {
        Ok(resolved) => resolved,
        Err(error) => exit_with_audio_error(error),
    };
    let environment_device = std::env::var("MODV_AUDIO_DEVICE").ok();
    let audio_device = resolve_audio_device(
        inline_device,
        matches
            .get_one::<String>("audio-device")
            .map(String::as_str),
        environment_device.as_deref(),
    );

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
        isf_dir: resolve_isf_dir(matches.get_one::<PathBuf>("isf-dir").cloned()),
        audio_source,
        audio_device,
        list_audio_devices: matches.get_flag("list-audio-devices"),
    }
}

/// Report an audio-source/device validation failure through clap (exit code 2).
///
/// The missing-device case appends the host's input-device table so the user
/// can copy a device name straight out of the message.
fn exit_with_audio_error(error: AudioSourceError) -> ! {
    let mut message = error.to_string();
    if error == AudioSourceError::MissingDevice {
        message.push_str("\n\n");
        message.push_str(&crate::audio::describe_devices(
            &modv_media::audio::capture::input_devices(),
        ));
    }
    message.push('\n');
    clap::Error::raw(ErrorKind::InvalidValue, message).exit()
}

/// Resolve the ISF corpus directory the engine should scan.
///
/// Order (first match wins):
///
/// 1. `--isf-dir <PATH>` (the Electron supervisor always passes this);
/// 2. `$MODV_ISF_DIR`;
/// 3. a development default relative to the executable. The canonical Docker
///    build writes `crates/target/release/modv-engine`, so the repository root
///    is three levels above the binary directory and the corpus is at
///    `src/renderer/src/application/renderers/isf/modules/isf`. A Cargo
///    `target/` at the repository root and the current working directory are
///    tried as well. A missing directory is not an error: the engine starts
///    without ISF modules (the scan warns once).
pub fn resolve_isf_dir(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if explicit.is_some() {
        return explicit;
    }
    if let Some(path) = std::env::var_os("MODV_ISF_DIR") {
        return Some(PathBuf::from(path));
    }
    development_isf_dir()
}

/// The development-only corpus lookup described on [`resolve_isf_dir`].
fn development_isf_dir() -> Option<PathBuf> {
    let relative = Path::new("src/renderer/src/application/renderers/isf/modules/isf");
    let mut candidates = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(binary_dir) = executable.parent() {
            // `crates/target/release` → repository root.
            candidates.push(binary_dir.join("..").join("..").join("..").join(relative));
            // `<repo>/target/release` (workspace target at the repository root).
            candidates.push(binary_dir.join("..").join("..").join(relative));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(relative));
    }
    candidates.into_iter().find(|candidate| candidate.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_source_accepts_every_documented_spelling() {
        assert_eq!(
            parse_audio_source("renderer"),
            Ok((AudioSourceSelection::Renderer, None))
        );
        assert_eq!(
            parse_audio_source("pipewire"),
            Ok((AudioSourceSelection::Pipewire, None))
        );
        assert_eq!(
            parse_audio_source("pipewire:alsa_output.hdmi"),
            Ok((
                AudioSourceSelection::Pipewire,
                Some("alsa_output.hdmi".to_string())
            )),
            "pipewire takes an optional target sink node"
        );
        assert_eq!(
            parse_audio_source("input"),
            Ok((AudioSourceSelection::Input, None))
        );
        assert_eq!(
            parse_audio_source(" input: alsa_input.pci "),
            Ok((
                AudioSourceSelection::Input,
                Some("alsa_input.pci".to_string())
            ))
        );
    }

    #[test]
    fn renderer_rejects_a_device_suffix() {
        assert_eq!(
            parse_audio_source("renderer:alsa:null"),
            Err(AudioSourceError::UnknownSource(
                "renderer:alsa:null".to_string()
            )),
            "the renderer pump has no capture of its own to target"
        );
    }

    #[test]
    fn audio_source_rejects_an_unknown_source_with_the_accepted_values() {
        let error = parse_audio_source("microphone").expect_err("unknown source");

        assert_eq!(
            error,
            AudioSourceError::UnknownSource("microphone".to_string())
        );
        let message = error.to_string();
        assert!(message.contains("microphone"), "{message}");
        assert!(message.contains("renderer, pipewire, input"), "{message}");
        assert!(message.contains("MODV_AUDIO_SOURCE"), "{message}");
    }

    #[test]
    fn audio_source_with_an_empty_device_asks_for_one_and_lists_devices() {
        let error = parse_audio_source("input:").expect_err("empty device");

        assert_eq!(error, AudioSourceError::MissingDevice);
        let message = error.to_string();
        assert!(message.contains("--list-audio-devices"), "{message}");
        assert!(message.contains("default input device"), "{message}");

        // The device table is appended when clap reports the error.
        let table = crate::audio::describe_devices(&[]);
        assert!(table.contains("no audio input devices"), "{table}");
    }

    #[test]
    fn resolution_prefers_the_flag_then_the_environment_then_the_default() {
        assert_eq!(
            resolve_audio_source(Some("pipewire"), Some("input")),
            Ok((AudioSourceSelection::Pipewire, None))
        );
        assert_eq!(
            resolve_audio_source(None, Some("input:mic")),
            Ok((AudioSourceSelection::Input, Some("mic".to_string())))
        );
        assert_eq!(
            resolve_audio_source(None, None),
            Ok((AudioSourceSelection::Renderer, None))
        );
        // Empty values read as unset.
        assert_eq!(
            resolve_audio_source(Some("  "), Some("")),
            Ok((AudioSourceSelection::Renderer, None))
        );
    }

    #[test]
    fn engine_owned_is_only_false_for_the_renderer_pump() {
        assert!(!AudioSourceSelection::Renderer.engine_owned());
        assert!(AudioSourceSelection::Pipewire.engine_owned());
        assert!(AudioSourceSelection::Input.engine_owned());
        assert_eq!(
            AudioSourceSelection::default(),
            AudioSourceSelection::Renderer
        );
    }

    #[test]
    fn audio_device_prefers_the_inline_suffix_then_flag_then_environment() {
        assert_eq!(
            resolve_audio_device(Some("suffix".into()), Some("flag"), Some("env")),
            Some("suffix".to_string())
        );
        assert_eq!(
            resolve_audio_device(None, Some(" flag "), Some("env")),
            Some("flag".to_string())
        );
        assert_eq!(
            resolve_audio_device(None, None, Some("env")),
            Some("env".to_string())
        );
        assert_eq!(
            resolve_audio_device(None, Some("  "), Some("")),
            None,
            "blank values are absent"
        );
    }
}
