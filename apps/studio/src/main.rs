//! `jd-studio`: Jenga Dunia Studio (ADR-0017).
//!
//! By default it opens the Studio window (3-D viewport and panels) and launches the Editor (the
//! node graph, `jd-app`) against its engine. `--headless` serves the engine without a window:
//! it prints one ready line on stdout (`{"socket", "token", "session"}`, what a parent needs to
//! connect, or to launch the Editor with `JD_ENGINE_SOCKET` / `JD_ENGINE_TOKEN`) and serves
//! until stdin closes.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use eframe::egui_wgpu;
use jd_core::{Context, Registry};

#[derive(Parser)]
#[command(
    name = "jd-studio",
    version,
    about = "Jenga Dunia Studio: the engine, the 3-D viewport and the Editor's host"
)]
struct Args {
    /// Serve the engine without a window until stdin closes.
    #[arg(long)]
    headless: bool,
    /// The Editor binary (default: `JD_EDITOR`, else `jd-app` next to `jd-studio`).
    #[arg(long, conflicts_with = "no_editor")]
    editor: Option<PathBuf>,
    /// Do not launch the Editor.
    #[arg(long)]
    no_editor: bool,
    /// A project (`.jdproj`) to open.
    project: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    init_logging();
    let server = match jd_ipc::Server::bind(Registry::with_builtins(), Context::default()) {
        Ok(s) => Arc::new(s),
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
    if args.headless {
        return headless(&server);
    }
    let editor = if args.no_editor {
        None
    } else {
        match jd_studio::editor::locate(args.editor) {
            Ok(p) => Some(p),
            Err(msg) => {
                // Studio still works without the Editor; the panel offers to open it later.
                tracing::warn!("{msg}");
                None
            }
        }
    };
    let options = jd_studio::Options {
        editor,
        project: args.project,
    };
    let result = eframe::run_native(
        "Jenga Dunia Studio",
        native_options(),
        Box::new(move |cc| {
            // One GPU for the window and the terrain kernels (erosion output can feed the view).
            if let Some(rs) = &cc.wgpu_render_state {
                let info = rs.adapter.get_info();
                let name = format!("{} ({:?})", info.name, info.backend);
                if jd_gpu::Gpu::install(rs.device.clone(), rs.queue.clone(), name.clone()) {
                    tracing::info!(adapter = %name, "terrain kernels share the window's GPU");
                }
            }
            Ok(Box::new(jd_studio::StudioApp::new(cc, server, options)))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "the Studio window failed");
            ExitCode::FAILURE
        }
    }
}

fn native_options() -> eframe::NativeOptions {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.power_preference = egui_wgpu::wgpu::PowerPreference::HighPerformance;
    // The adapter's own limits: the erosion kernels bind storage buffers over the 128 MiB default.
    setup.device_descriptor = Arc::new(|adapter| egui_wgpu::wgpu::DeviceDescriptor {
        label: Some("jd-studio"),
        required_limits: adapter.limits(),
        ..Default::default()
    });
    let mut options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Jenga Dunia Studio")
            .with_app_id("jenga-dunia-studio")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 520.0]),
        // The window opens at its default size each time, as the desktop's does.
        persist_window: false,
        ..Default::default()
    };
    options.wgpu_options.wgpu_setup = egui_wgpu::WgpuSetup::CreateNew(setup);
    options
}

fn headless(server: &jd_ipc::Server) -> ExitCode {
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
