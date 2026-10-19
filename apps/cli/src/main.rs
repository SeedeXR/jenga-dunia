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
    /// Terrain generation.
    Terrain {
        #[command(subcommand)]
        what: TerrainCommand,
    },
    /// Export project data.
    Export {
        #[command(subcommand)]
        what: ExportCommand,
    },
    /// Diagnostics.
    Diag {
        #[command(subcommand)]
        what: DiagCommand,
    },
}

#[derive(Subcommand)]
enum TerrainCommand {
    /// Generate a noise heightfield for the project world (optionally export it in one go).
    Generate {
        /// Project directory.
        #[arg(long)]
        project: PathBuf,
        /// Noise seed.
        #[arg(long, default_value_t = 0)]
        seed: i32,
        /// Lattice: open_simplex2 | open_simplex2s | perlin | value | value_cubic | cellular.
        #[arg(long, default_value = "open_simplex2")]
        kind: String,
        /// Fractal: none | fbm | ridged | ping_pong.
        #[arg(long, default_value = "fbm")]
        fractal: String,
        /// Feature size in metres (wavelength of the first octave).
        #[arg(long, default_value_t = 1024.0)]
        feature_size: f32,
        /// Octaves (1–16).
        #[arg(long, default_value_t = 6)]
        octaves: u8,
        /// Minimum height in metres.
        #[arg(long, default_value_t = 0.0)]
        min: f32,
        /// Maximum height in metres.
        #[arg(long, default_value_t = 500.0)]
        max: f32,
        /// Layer name.
        #[arg(long, default_value = "height")]
        layer: String,
        /// Also export to this file (format from extension: .exr .png .r16 .r32).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ExportCommand {
    /// Export a heightfield layer. Needs a generated layer in this invocation; prefer `jd terrain generate --out`.
    Heightmap {
        /// Project directory.
        #[arg(long)]
        project: PathBuf,
        /// Output file (relative paths go under the project's exports/).
        path: PathBuf,
        /// Format: exr | png16 | raw_r16 | raw_r32 (default: from extension).
        #[arg(long)]
        format: Option<String>,
        /// Layer name.
        #[arg(long, default_value = "height")]
        layer: String,
    },
}

#[derive(Subcommand)]
enum DiagCommand {
    /// Host environment, memory, detected tools.
    Env,
}

fn format_from_ext(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "exr" => Some("exr"),
        "png" => Some("png16"),
        "r16" | "raw" => Some("raw_r16"),
        "r32" => Some("raw_r32"),
        _ => None,
    }
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
        Command::Terrain {
            what:
                TerrainCommand::Generate {
                    project,
                    seed,
                    kind,
                    fractal,
                    feature_size,
                    octaves,
                    min,
                    max,
                    layer,
                    out,
                },
        } => {
            let mut outs = vec![invoke(
                &registry,
                &ctx,
                "project.open",
                json!({ "path": project }),
            )];
            if outs[0].status != Status::Ok {
                return finish(outs, json);
            }
            let noise = json!({ "seed": seed, "kind": kind, "fractal": fractal, "feature_size_m": feature_size, "octaves": octaves, "height_range_m": [min, max] });
            outs.push(invoke(
                &registry,
                &ctx,
                "terrain.generate",
                json!({ "noise": noise, "layer": layer }),
            ));
            if let Some(out) = out
                && outs.last().map(|r| r.status == Status::Ok).unwrap_or(false)
            {
                let Some(format) = format_from_ext(&out) else {
                    eprintln!(
                        "error: cannot infer a heightmap format from `{}` (use .exr .png .r16 .r32)",
                        out.display()
                    );
                    return ExitCode::from(2);
                };
                outs.push(invoke(
                    &registry,
                    &ctx,
                    "terrain.export_heightmap",
                    json!({ "layer": layer, "path": out, "format": format }),
                ));
            }
            outs
        }
        Command::Export {
            what:
                ExportCommand::Heightmap {
                    project,
                    path,
                    format,
                    layer,
                },
        } => {
            let mut outs = vec![invoke(
                &registry,
                &ctx,
                "project.open",
                json!({ "path": project }),
            )];
            if outs[0].status != Status::Ok {
                return finish(outs, json);
            }
            let Some(format) = format.as_deref().or_else(|| format_from_ext(&path)) else {
                eprintln!(
                    "error: cannot infer a heightmap format from `{}`; pass --format",
                    path.display()
                );
                return ExitCode::from(2);
            };
            outs.push(invoke(
                &registry,
                &ctx,
                "terrain.export_heightmap",
                json!({ "layer": layer, "path": path, "format": format }),
            ));
            outs
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
