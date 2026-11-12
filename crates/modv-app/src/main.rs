//! modv-engine binary — M0 spike harness. See docs/rust-hybrid.md (M0).

mod cli;
mod headless;
mod report;
mod window;

use std::process::ExitCode;

use modv_render::{Engine, EngineConfig};

fn main() -> ExitCode {
    let cli = cli::parse();
    if let Some(socket) = &cli.socket {
        eprintln!(
            "engine socket flag accepted but unused in the M0 spike: {}",
            socket.display()
        );
    }
    if let Some(state) = &cli.state {
        eprintln!(
            "engine state flag accepted but unused in the M0 spike: {}",
            state.display()
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

    match window::run(cli, engine, adapter) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
