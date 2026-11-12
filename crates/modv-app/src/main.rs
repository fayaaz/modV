//! modv-engine binary — the engine process. See docs/rust-hybrid.md.
//!
//! M0 spike harness plus the M1 supervision channel and the M2 store +
//! compositor: windowed rendering of the engine state, control socket,
//! snapshot/restore and the shared-memory preview ring.

mod audio;
mod cli;
mod compositor;
mod fixtures;
mod headless;
mod ids;
mod input_links;
mod module_preview;
mod module_registry;
mod performance;
mod report;
mod scene_ops;
mod scene_preview;
mod scene_transition;
mod store_ops;
mod supervisor;
mod window;

use std::process::ExitCode;

use modv_render::{Engine, EngineConfig};

fn main() -> ExitCode {
    die_with_parent();

    let cli = cli::parse();

    // WS-9: device enumeration is a pure host query — answer it before any GPU,
    // window or socket work so `--list-audio-devices` works headless and in CI.
    if cli.list_audio_devices {
        audio::print_devices();
        return ExitCode::SUCCESS;
    }

    // Fixture emission is a pure store session: no GPU device, no window and no
    // control socket are touched, so it runs before `Engine::new`.
    if let Some(dir) = &cli.emit_fixtures {
        return match fixtures::emit(dir) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: could not emit fixtures: {err}");
                ExitCode::FAILURE
            }
        };
    }

    if cli.socket.is_some() && cli.headless {
        eprintln!("engine: note — the control socket is only polled in windowed mode");
    }
    if cli.audio_source.engine_owned() && (cli.headless || cli.screenshot.is_some()) {
        eprintln!(
            "engine: note — --audio-source {} is only wired in windowed mode; headless renders against --audio-fixture",
            cli.audio_source.as_str()
        );
    }

    let engine = match Engine::new(&EngineConfig::default()) {
        Ok(engine) => engine,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };
    let adapter = report::AdapterSummary::from_engine(&engine);
    eprintln!(
        "adapter: {} ({}) vendor={:#06x} driver={}",
        adapter.name, adapter.backend, adapter.vendor, adapter.driver
    );
    if cli.log_adapter {
        match serde_json::to_string_pretty(&adapter.to_json()) {
            Ok(text) => eprintln!("adapter info (json): {text}"),
            Err(err) => eprintln!("error: failed to serialise adapter info: {err}"),
        }
    }
    if cli.print_adapter {
        match serde_json::to_string_pretty(&adapter.to_json()) {
            Ok(text) => println!("{text}"),
            Err(err) => {
                eprintln!("error: failed to serialise adapter info: {err}");
                return ExitCode::FAILURE;
            }
        }
        return ExitCode::SUCCESS;
    }

    if cli.headless || cli.screenshot.is_some() {
        return match headless::run(cli, engine, adapter) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(problem) = x11_backend_problem() {
        eprintln!("engine: ERROR {problem}");
        return ExitCode::FAILURE;
    }

    match window::run(cli, engine, adapter) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Preflight for the X11 fallback: winit loads `libxkbcommon-x11` at runtime
/// (via xkbcommon-dl) and panics with an unhelpful backtrace when it is
/// missing. Detect the situation up front and fail with an actionable line.
///
/// Only applies when the session is X11: the Wayland path needs
/// `libxkbcommon` only, which is a hard dependency of the windowing stack.
#[cfg(target_os = "linux")]
fn x11_backend_problem() -> Option<String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_none() {
        return None;
    }

    for name in ["libxkbcommon-x11.so.0", "libxkbcommon-x11.so"] {
        let Ok(candidate) = std::ffi::CString::new(name) else {
            continue;
        };
        // SAFETY: dlopen() with a valid NUL-terminated name; the handle from
        // dlopen is closed immediately and never dereferenced.
        let handle =
            unsafe { libc::dlopen(candidate.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
        if !handle.is_null() {
            // SAFETY: `handle` came from dlopen above.
            unsafe { libc::dlclose(handle) };
            return None;
        }
    }

    Some(
        "the X11 fallback requires libxkbcommon-x11 (e.g. the libxkbcommon-x11-0 \
         package); install it or run the engine under Wayland"
            .to_string(),
    )
}

#[cfg(not(target_os = "linux"))]
fn x11_backend_problem() -> Option<String> {
    None
}

/// Ask the kernel to SIGTERM this process when its parent dies.
///
/// The Electron supervisor normally sends `engine/shutdown` on quit, but if
/// the main process is SIGKILLed or the container is torn down abruptly the
/// engine would otherwise be orphaned (it owns a native window and GPU
/// device). `PR_SET_PDEATHSIG` ties its lifetime to the spawner; the periodic
/// snapshot is the state-restore safety net in that path.
#[cfg(target_os = "linux")]
fn die_with_parent() {
    // SAFETY: prctl(PR_SET_PDEATHSIG, SIGTERM) only sets a per-process signal
    // disposition; it has no memory-safety implications and cannot fail in a
    // way that would leave this process in an invalid state.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
}

#[cfg(not(target_os = "linux"))]
fn die_with_parent() {}
