//! modv-engine binary — the engine process. See docs/rust-hybrid.md.
//!
//! M0 spike harness plus the M1 supervision channel and the M2 store +
//! compositor: windowed rendering of the engine state, control socket,
//! snapshot/restore and the shared-memory preview ring.

mod cli;
mod compositor;
mod headless;
mod ids;
mod report;
mod store_ops;
mod supervisor;
mod window;

use std::process::ExitCode;

use modv_render::{Engine, EngineConfig};

fn main() -> ExitCode {
    let cli = cli::parse();
    if cli.socket.is_some() && cli.headless {
        eprintln!("engine: note — the control socket is only polled in windowed mode");
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

    match window::run(cli, engine, adapter) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
