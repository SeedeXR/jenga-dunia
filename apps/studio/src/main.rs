//! Jenga Dunia Studio (ADR-0017): the process that owns the engine — the `jd-core` registry and
//! context — and serves it to the Editor over `jd-ipc`.
//!
//! Phase 1 is `--headless`: bind a private socket, print one ready line on stdout
//! (`{"socket", "token", "session"}`, what a parent needs to connect or launch the Editor with
//! `JD_ENGINE_SOCKET` / `JD_ENGINE_TOKEN`), then serve until stdin closes. The egui window that
//! renders the viewport and launches the Editor itself arrives in Phase 2.

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use jd_core::{Context, Registry};

#[derive(Parser)]
#[command(
    name = "jd-studio",
    version,
    about = "Jenga Dunia Studio: the engine process"
)]
struct Args {
    /// Serve the engine without a window until stdin closes.
    #[arg(long)]
    headless: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    init_logging();
    if !args.headless {
        eprintln!("jd-studio: the Studio window arrives in Phase 2; run with --headless");
        return ExitCode::from(2);
    }
    let server = match jd_ipc::Server::bind(Registry::with_builtins(), Context::default()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(
                code = e.code.as_str(),
                "could not start the engine server: {}",
                e.message
            );
            eprintln!("{}", serde_json::to_string(&e).unwrap_or_default());
            return ExitCode::FAILURE;
        }
    };
    let ready = serde_json::json!({
        "socket": server.socket(),
        "token": server.token(),
        "session": server.session(),
    });
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{ready}").and_then(|()| out.flush()).is_err() {
        tracing::error!("could not write the ready line; nobody can connect");
        return ExitCode::FAILURE;
    }
    drop(out);
    // The parent stops us by closing stdin (or by going away, which closes it too).
    let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
    server.shutdown("Studio stopped");
    ExitCode::SUCCESS
}

/// `tracing` to stderr; `JD_LOG` sets the filter (default `info`).
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("JD_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
