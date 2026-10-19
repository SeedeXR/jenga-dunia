//! `jd` — Jenga Dunia command-line interface.
//!
//! A thin adapter: every subcommand builds a [`jd_core::Request`] and prints the
//! [`jd_core::Response`]. `--json` prints the raw envelope (machine-readable);
//! otherwise a short human summary. Exit codes follow `docs/spec/error-codes.md`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use jd_core::{Context, Registry, Request, Response, Status};
use jd_types::WorldSettings;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "jd", version, about = "Jenga Dunia — procedural world generation", long_about = None)]
struct Cli {
    /// Print the raw JSON response envelope on stdout.
    #[arg(long, global = true)]
    json: bool,
    /// Log level filter (e.g. `info`, `jd_core=debug`).
    #[arg(long, global = true, default_value = "warn", env = "JD_LOG")]
    log: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new project directory.
    New {
        /// Directory to create (e.g. `MyWorld.jdproj`).
        path: PathBuf,
        /// Display name (defaults to the directory stem).
        #[arg(long)]
        name: Option<String>,
        /// World extent in metres (square).
        #[arg(long, default_value_t = 4096.0)]
        extent: f64,
        /// Heightfield resolution in pixels (square).
        #[arg(long, default_value_t = 4096)]
        resolution: u32,
        /// Master seed.
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    /// Open a project and print its summary (replays journal with --recover).
    Open {
        /// Project directory.
        path: PathBuf,
        /// Replay unsaved journal entries and save.
        #[arg(long)]
        recover: bool,
    },
    /// Invoke any operation by id with JSON params (opens `--project` first when given).
    Op {
        /// Operation id, e.g. `project.set_name`.
        id: String,
        /// JSON object of parameters.
        #[arg(long, default_value = "{}")]
        params: String,
        /// Project to open before invoking.
        #[arg(long)]
        project: Option<PathBuf>,
        /// Save the project after the operation.
        #[arg(long)]
        save: bool,
    },
    /// Print JSON schemas of operations.
    Schema {
        /// Operation id; omit with --list.
        op: Option<String>,
        /// List all operations.
        #[arg(long)]
        list: bool,
    },
    /// Diagnostics.
    Diag {
        #[command(subcommand)]
        what: DiagCommand,
    },
}

#[derive(Subcommand)]
enum DiagCommand {
    /// Host environment, memory, detected tools.
    Env,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&cli.log))
        .with_writer(std::io::stderr)
        .init();
    let registry = Registry::with_builtins();
    let ctx = Context::default();
    let json = cli.json;

    let responses: Vec<Response> = match cli.command {
        Command::New {
            path,
            name,
            extent,
            resolution,
            seed,
        } => {
            // `jd new Hills` → `Hills.jdproj`; an explicit extension is kept as given.
            let path = if path.extension().is_none() {
                path.with_extension(jd_core::project::PROJECT_EXT)
            } else {
                path
            };
            let name = name.unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "World".into())
            });
            let world = WorldSettings {
                extent_m: [extent, extent],
                resolution_px: [resolution, resolution],
                seed,
                ..Default::default()
            };
            vec![invoke(
                &registry,
                &ctx,
                "project.create",
                json!({ "path": path, "name": name, "world": world }),
            )]
        }
        Command::Open { path, recover } => {
            let mut out = vec![invoke(
                &registry,
                &ctx,
                "project.open",
                json!({ "path": path }),
            )];
            if recover && out[0].status == Status::Ok {
                out.push(invoke(&registry, &ctx, "project.recover", json!({})));
            }
            out
        }
        Command::Op {
            id,
            params,
            project,
            save,
        } => {
            let mut out = Vec::new();
            if let Some(p) = project {
                out.push(invoke(
                    &registry,
                    &ctx,
                    "project.open",
                    json!({ "path": p }),
                ));
                if out[0].status != Status::Ok {
                    return finish(out, json);
                }
            }
            let params: Value = match serde_json::from_str(&params) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: --params is not valid JSON: {e}");
                    return ExitCode::from(2);
                }
            };
            out.push(invoke(&registry, &ctx, &id, params));
            if save && out.last().map(|r| r.status == Status::Ok).unwrap_or(false) {
                out.push(invoke(&registry, &ctx, "project.save", json!({})));
            }
            out
        }
        Command::Schema { op, list } => {
            let payload = if list || op.is_none() {
                Value::Array(registry.list().into_iter().map(|o| json!({ "id": o.id, "description": o.description, "scope": o.scope })).collect())
            } else {
                let id = op.unwrap_or_default();
                match registry.get(&id) {
                    Some(o) => serde_json::to_value(o.info()).unwrap_or(Value::Null),
                    None => {
                        eprintln!("error: unknown operation `{id}` (see `jd schema --list`)");
                        return ExitCode::from(2);
                    }
                }
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).unwrap_or_default()
            );
            return ExitCode::SUCCESS;
        }
        Command::Diag {
            what: DiagCommand::Env,
        } => vec![invoke(&registry, &ctx, "diag.env", json!({}))],
    };
    finish(responses, json)
}

fn invoke(registry: &Registry, ctx: &Context, op: &str, params: Value) -> Response {
    let mut req = Request::new(op, params);
    req.client = Some(jd_core::envelope::Client {
        kind: "cli".into(),
        name: "jd".into(),
        scopes: vec!["*".into()],
    });
    registry.invoke(ctx, req)
}

fn finish(responses: Vec<Response>, json: bool) -> ExitCode {
    let mut exit = 0;
    for r in &responses {
        if json {
            println!("{}", serde_json::to_string(r).unwrap_or_default());
        } else {
            print_human(r);
        }
        if let Some(e) = &r.error {
            exit = e.code.exit_code();
        }
    }
    ExitCode::from(exit as u8)
}

fn print_human(r: &Response) {
    match r.status {
        Status::Ok => {
            println!("ok  {}", r.op);
            if let Some(v) = &r.result {
                println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
            }
        }
        Status::Accepted => println!(
            "accepted  {}  job {}",
            r.op,
            r.job_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default()
        ),
        Status::Error => {
            if let Some(e) = &r.error {
                eprintln!("error  {}  {}: {}", r.op, e.code.as_str(), e.message);
                if !e.why.is_empty() {
                    eprintln!("  why:  {}", e.why);
                }
                if !e.next.is_empty() {
                    eprintln!("  next: {}", e.next);
                }
            }
        }
    }
    for w in &r.warnings {
        eprintln!("warning  {}: {}", w.code, w.message);
    }
}
