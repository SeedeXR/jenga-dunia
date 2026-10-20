//! The operation registry: the single API surface behind GUI, CLI and MCP.
//!
//! Every capability is an [`Operation`] with a stable id, a scope, JSON schemas
//! for its request and result (generated from Rust types with `schemars`), and
//! a handler. Adapters call [`Registry::invoke`] with a [`Request`] and get a
//! [`Response`]; they never contain domain logic.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use jd_types::{ErrorCode, JdError, JdResult, SchemaVersion, WorldSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::envelope::{Request, Response, Warning};
use crate::project::Project;

/// Permission scope an operation requires (`docs/architecture/07-agent-interfaces.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// No project access needed.
    None,
    /// Read the open project.
    ProjectRead,
    /// Mutate the open project.
    ProjectWrite,
    /// Create files/directories outside the open project.
    FsWrite,
    /// Write exports.
    Export,
    /// Irreversible actions; require confirmation.
    Destructive,
    /// Network access to geo providers.
    NetworkGeo,
    /// Vision model inference.
    Vision,
}

/// Session state shared by all operations of one adapter instance.
#[derive(Default, Clone)]
pub struct Context {
    project: Arc<Mutex<Option<Project>>>,
    /// Heightfield layers produced by terrain/graph ops, keyed by layer name. Backed by the open
    /// project's on-disk cache (`jd-cache`), so layers outlive the process and the CLI can
    /// evaluate in one invocation and export in the next.
    layers: Arc<Mutex<BTreeMap<String, jd_terrain::Heightfield>>>,
    /// Running jobs (progress + cancel), see `jobs.list` / `jobs.cancel`.
    pub jobs: Arc<crate::jobs::Jobs>,
}

impl Context {
    /// Store a heightfield layer computed for the project at `root` (written through to that
    /// project's cache). Refused when a different project is open by now: a long evaluation must
    /// never land its result in the project the user switched to meanwhile.
    pub fn set_layer_for(
        &self,
        root: &Path,
        name: &str,
        hf: jd_terrain::Heightfield,
    ) -> JdResult<()> {
        if self.project_root().as_deref() != Some(root) {
            return Err(JdError::new(
                ErrorCode::E_INPUT_INVALID,
                "the project changed while the layer was being computed",
            )
            .why("Results are stored in the project they were computed for.")
            .next("Re-run the operation on the project that is open now.")
            .detail("layer", name));
        }
        let limit = self.with_project(|p| Ok(p.doc.settings.cache_limit_bytes))?;
        // Lock order: project (above, briefly) → layers. Cache writes happen under the layers lock
        // so concurrent evaluations never interleave index writes.
        let mut g = self
            .layers
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "layer lock poisoned"))?;
        jd_cache::LayerCache::open(root)
            .with_limit(limit)
            .put(name, &hf)?;
        g.insert(name.to_owned(), hf);
        Ok(())
    }

    /// Run `f` with a stored layer, or fail with `E_INPUT_INVALID`.
    pub fn with_layer<T>(
        &self,
        name: &str,
        f: impl FnOnce(&jd_terrain::Heightfield) -> JdResult<T>,
    ) -> JdResult<T> {
        let root = self.project_root(); // taken before the layer lock: with_project never locks layers
        let mut g = self
            .layers
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "layer lock poisoned"))?;
        if !g.contains_key(name)
            && let Some(root) = root
            && let Some(hf) = jd_cache::LayerCache::open(&root).get(name)?
        {
            g.insert(name.to_owned(), hf);
        }
        match g.get(name) {
            Some(hf) => f(hf),
            None => Err(JdError::new(
                ErrorCode::E_INPUT_INVALID,
                format!("no heightfield layer named `{name}`"),
            )
            .why("Terrain ops read layers produced by terrain.generate or graph.evaluate; they persist in the project's cache/.")
            .next("Run graph.evaluate (or terrain.generate) first; the default layer name is `height`.")
            .detail("layer", name)),
        }
    }

    /// Run `f` with the open project, or fail with `E_INPUT_INVALID` when none is open.
    pub fn with_project<T>(&self, f: impl FnOnce(&mut Project) -> JdResult<T>) -> JdResult<T> {
        let mut guard = self
            .project
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "project lock poisoned"))?;
        match guard.as_mut() {
            Some(p) => f(p),
            None => Err(
                JdError::new(ErrorCode::E_INPUT_INVALID, "no project is open")
                    .why("This operation needs an open project.")
                    .next("Call project.open or project.create first."),
            ),
        }
    }

    /// Directory of the open project, if any (adapters use it to resolve relative paths the
    /// same way the core does).
    pub fn project_root(&self) -> Option<PathBuf> {
        self.project
            .lock()
            .ok()?
            .as_ref()
            .map(|p| p.root().to_path_buf())
    }

    /// Replace the open project (saving is the caller's responsibility). In-memory layers belong
    /// to the previous project and are dropped; the new project's come back from its cache.
    pub fn set_project(&self, p: Option<Project>) -> JdResult<()> {
        let mut guard = self
            .project
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "project lock poisoned"))?;
        *guard = p;
        self.layers
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "layer lock poisoned"))?
            .clear();
        Ok(())
    }
}

type Handler = Arc<dyn Fn(&Context, Value) -> JdResult<(Value, Vec<Warning>)> + Send + Sync>;

/// A registered operation.
#[derive(Clone)]
pub struct Operation {
    /// Stable id, `<subsystem>.<verb>`.
    pub id: &'static str,
    /// One-line description for CLI help and MCP tool listings.
    pub description: &'static str,
    /// Required scope.
    pub scope: Scope,
    /// JSON Schema (2020-12) of `params`.
    pub request_schema: Value,
    /// JSON Schema of `result`.
    pub result_schema: Value,
    handler: Handler,
    /// Returns `status: accepted` with the job id instead of a result.
    detached: bool,
}

/// Machine-readable description of an operation (no handler).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OperationInfo {
    /// Stable id.
    pub id: String,
    /// Description.
    pub description: String,
    /// Required scope.
    pub scope: Scope,
    /// Request schema.
    pub request_schema: Value,
    /// Result schema.
    pub result_schema: Value,
}

/// Result of a detached op: the job to follow.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AcceptedJob {
    /// Job id for `jobs.list` / `jobs.cancel` / `jobs.result`.
    pub job_id: u64,
}

/// The registry.
#[derive(Clone, Default)]
pub struct Registry {
    ops: BTreeMap<&'static str, Operation>,
}

fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

impl Registry {
    /// Register a typed operation. Params are deserialised with `deny_unknown_fields` semantics
    /// enforced by the request type.
    pub fn register<P, R, F>(
        &mut self,
        id: &'static str,
        description: &'static str,
        scope: Scope,
        f: F,
    ) where
        P: for<'de> Deserialize<'de> + schemars::JsonSchema,
        R: Serialize + schemars::JsonSchema,
        F: Fn(&Context, P) -> JdResult<(R, Vec<Warning>)> + Send + Sync + 'static,
    {
        let handler: Handler = Arc::new(move |ctx, params| {
            let p: P = serde_json::from_value(params).map_err(|e| {
                JdError::invalid("params", e.to_string())
                    .why("The request parameters do not match the operation schema.")
                    .next("Compare with `jd schema <op>` and fix the parameters.")
            })?;
            let (r, w) = f(ctx, p)?;
            Ok((serde_json::to_value(r)?, w))
        });
        self.ops.insert(
            id,
            Operation {
                id,
                description,
                scope,
                request_schema: schema_of::<P>(),
                result_schema: schema_of::<R>(),
                handler,
                detached: false,
            },
        );
    }

    /// Register an op that starts a detached job: the handler returns the job id and the caller
    /// receives `status: accepted`, `job_id` and `result: { "job_id": N }`.
    pub fn register_detached<P, F>(
        &mut self,
        id: &'static str,
        description: &'static str,
        scope: Scope,
        f: F,
    ) where
        P: for<'de> Deserialize<'de> + schemars::JsonSchema,
        F: Fn(&Context, P) -> JdResult<u64> + Send + Sync + 'static,
    {
        self.register::<P, AcceptedJob, _>(id, description, scope, move |ctx, p| {
            Ok((AcceptedJob { job_id: f(ctx, p)? }, Vec::new()))
        });
        if let Some(op) = self.ops.get_mut(id) {
            op.detached = true;
        }
    }

    /// All operations, sorted by id.
    pub fn list(&self) -> Vec<OperationInfo> {
        self.ops.values().map(|o| o.info()).collect()
    }

    /// Look up one operation.
    pub fn get(&self, id: &str) -> Option<&Operation> {
        self.ops.get(id)
    }

    /// Invoke an operation with a full envelope. Never panics; every failure is a `Response` with `status: error`.
    pub fn invoke(&self, ctx: &Context, req: Request) -> Response {
        let request_id = req.request_id.clone().unwrap_or_default();
        if req.schema_version != SchemaVersion::CURRENT {
            return Response::error(
                &req.op,
                request_id,
                JdError::new(
                    ErrorCode::E_SCHEMA_VERSION,
                    format!("unsupported API schema_version {}", req.schema_version),
                )
                .next(format!("Use schema_version {}.", SchemaVersion::CURRENT))
                .detail("found", req.schema_version.0)
                .detail("supported", SchemaVersion::CURRENT.0),
            );
        }
        let Some(op) = self.ops.get(req.op.as_str()) else {
            return Response::error(
                &req.op,
                request_id,
                JdError::new(
                    ErrorCode::E_INPUT_INVALID,
                    format!("unknown operation `{}`", req.op),
                )
                .next("List operations with `jd schema --list`.")
                .detail("op", req.op.clone()),
            );
        };
        let span = tracing::info_span!("op", op = op.id, request_id = %request_id);
        let _enter = span.enter();
        let started = std::time::Instant::now();
        let handler = op.handler.clone();
        let detached = op.detached;
        let Request {
            op: op_id, params, ..
        } = req;
        let outcome = handler(ctx, params);
        let duration_ms = started.elapsed().as_millis() as u64;
        match outcome {
            Ok((result, warnings)) => {
                tracing::info!(
                    duration_ms,
                    outcome = if detached { "accepted" } else { "ok" },
                    warnings = warnings.len(),
                    "operation completed"
                );
                let mut r = Response::ok(&op_id, request_id, result, warnings);
                if detached {
                    r.status = crate::envelope::Status::Accepted;
                    r.job_id = r.result.as_ref().and_then(|v| v["job_id"].as_u64());
                }
                r
            }
            Err(e) => {
                tracing::warn!(
                    duration_ms,
                    outcome = "error",
                    code = e.code.as_str(),
                    "operation failed"
                );
                Response::error(&op_id, request_id, e)
            }
        }
    }

    /// The default registry with every built-in operation.
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        builtin::register_all(&mut r);
        r
    }
}

impl Operation {
    /// Description without the handler.
    pub fn info(&self) -> OperationInfo {
        OperationInfo {
            id: self.id.to_owned(),
            description: self.description.to_owned(),
            scope: self.scope,
            request_schema: self.request_schema.clone(),
            result_schema: self.result_schema.clone(),
        }
    }
}

/// Built-in operations.
pub mod builtin {
    use super::*;
    use crate::project::JournalEntry;

    /// `project.create` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ProjectCreate {
        /// Directory to create (conventionally ending in `.jdproj`).
        pub path: PathBuf,
        /// Display name.
        pub name: String,
        /// World settings; defaults when omitted.
        #[serde(default)]
        pub world: Option<WorldSettings>,
    }

    /// `project.open` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ProjectOpen {
        /// Project directory.
        pub path: PathBuf,
    }

    /// Empty params.
    #[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct NoParams {}

    /// `project.set_name` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ProjectSetName {
        /// New display name.
        pub name: String,
    }

    /// `project.set_settings` params: fields left out keep their current value.
    #[derive(Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ProjectSetSettings {
        /// Seconds after the last edit before the desktop autosaves (0 disables).
        #[serde(default)]
        pub autosave_interval_s: Option<u32>,
        /// Layer cache cap in bytes (≥ 16 MiB).
        #[serde(default)]
        pub cache_limit_bytes: Option<u64>,
        /// Preview LOD hint (resolution ÷ 2^lod).
        #[serde(default)]
        pub preview_lod: Option<u8>,
    }

    fn merged_settings(p: &Project, args: &ProjectSetSettings) -> crate::project::ProjectSettings {
        let mut s = p.doc.settings.clone();
        if let Some(v) = args.autosave_interval_s {
            s.autosave_interval_s = v;
        }
        if let Some(v) = args.cache_limit_bytes {
            s.cache_limit_bytes = v;
        }
        if let Some(v) = args.preview_lod {
            s.preview_lod = v;
        }
        s
    }

    /// Summary returned by project ops.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct ProjectSummary {
        /// Project id.
        pub project_id: String,
        /// Name.
        pub name: String,
        /// Directory.
        pub path: String,
        /// World settings.
        pub world: WorldSettings,
        /// Document schema version.
        pub schema_version: SchemaVersion,
        /// Unsaved changes?
        pub dirty: bool,
        /// Journal entries awaiting replay.
        pub pending_journal: usize,
        /// User-tunable settings (`project.json` → `settings`): autosave interval, cache limit, preview LOD.
        pub settings: crate::project::ProjectSettings,
    }

    /// `project.import_asset` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ImportAsset {
        /// Source file on disk.
        pub path: PathBuf,
        /// Name inside `assets/` (default: the source file name).
        #[serde(default)]
        pub name: Option<String>,
    }

    /// `project.import_asset` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct ImportedAsset {
        /// Name to reference from nodes (relative to `assets/`).
        pub name: String,
        /// Bytes copied.
        pub bytes: u64,
        /// GeoTIFF georeferencing (pixel scale, tie point, nodata) when the file carries it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub geo: Option<jd_terrain::GeoTags>,
    }

    /// `jobs.cancel` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct CancelJob {
        /// Job id from `jobs.list`.
        pub id: u64,
    }

    /// `project.recover` params.
    #[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Recover {
        /// Drop journal entries that fail to replay instead of keeping them pending.
        #[serde(default)]
        pub discard_failed: bool,
    }

    /// `project.recover` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct RecoverResult {
        /// Entries replayed in order.
        pub replayed: Vec<JournalEntry>,
        /// Entries that failed to replay (op id + error message).
        pub failed: Vec<(String, String)>,
    }

    /// `schema.get` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct SchemaGet {
        /// Operation id.
        pub op: String,
    }

    fn summary(p: &Project) -> ProjectSummary {
        ProjectSummary {
            project_id: p.doc.id.to_string(),
            name: p.doc.name.clone(),
            path: p.root().display().to_string(),
            world: p.doc.world.clone(),
            schema_version: p.doc.schema_version,
            dirty: p.is_dirty(),
            pending_journal: p.pending_journal.len(),
            settings: p.doc.settings.clone(),
        }
    }

    /// Validate a mutation's parameters without touching the project. Runs **before** the
    /// write-ahead journal so a rejected request never leaves a bogus entry on disk.
    fn validate_mutation(p: &Project, op: &str, params: &Value) -> JdResult<()> {
        match op {
            "project.set_name" => {
                let args: ProjectSetName = serde_json::from_value(params.clone())?;
                Project::validate_name(&args.name)
            }
            "project.set_settings" => {
                let args: ProjectSetSettings = serde_json::from_value(params.clone())?;
                Project::validate_settings(&merged_settings(p, &args))
            }
            _ if op.starts_with("graph.") => super::graph::validate_mutation(p, op, params),
            other => Err(JdError::new(
                ErrorCode::E_INPUT_INVALID,
                format!("`{other}` is not a replayable mutation"),
            )),
        }
    }

    /// Apply a mutating op to the project in memory. Shared by live calls and journal replay so
    /// recovery cannot diverge from normal execution.
    fn apply_mutation(p: &mut Project, op: &str, params: &Value) -> JdResult<()> {
        validate_mutation(p, op, params)?;
        match op {
            "project.set_name" => {
                let args: ProjectSetName = serde_json::from_value(params.clone())?;
                p.set_name(&args.name)
            }
            "project.set_settings" => {
                let args: ProjectSetSettings = serde_json::from_value(params.clone())?;
                let merged = merged_settings(p, &args);
                p.set_settings(merged)
            }
            _ if op.starts_with("graph.") => super::graph::apply_mutation(p, op, params),
            _ => unreachable!("validate_mutation rejects unknown ops"),
        }
    }

    /// Validate → journal → apply. The only path for live mutations.
    pub(crate) fn mutate(p: &mut Project, op: &str, params: Value) -> JdResult<()> {
        validate_mutation(p, op, &params)?;
        p.journal(op, &params)?;
        // Undo history: every user graph edit records the previous graph; `graph.restore` is
        // how undo/redo themselves apply, so it must not be recorded as a new edit.
        let before = (op.starts_with("graph.") && op != "graph.restore").then(|| p.graph.clone());
        apply_mutation(p, op, &params)?;
        if let Some(g) = before {
            p.history.record(g);
        }
        Ok(())
    }

    /// Register every built-in op.
    pub fn register_all(r: &mut Registry) {
        r.register::<ProjectCreate, ProjectSummary, _>(
            "project.create",
            "Create a new project directory and open it",
            Scope::FsWrite,
            |ctx, p| {
                let project = Project::create(&p.path, &p.name, p.world.unwrap_or_default())?;
                let s = summary(&project);
                ctx.set_project(Some(project))?;
                Ok((s, Vec::new()))
            },
        );
        r.register::<ProjectOpen, ProjectSummary, _>(
            "project.open",
            "Open an existing project",
            Scope::ProjectRead,
            |ctx, p| {
                let (project, warnings) = Project::open(&p.path)?;
                let s = summary(&project);
                ctx.set_project(Some(project))?;
                Ok((s, warnings))
            },
        );
        r.register::<NoParams, ProjectSummary, _>(
            "project.info",
            "Describe the open project",
            Scope::ProjectRead,
            |ctx, _| ctx.with_project(|p| Ok((summary(p), Vec::new()))),
        );
        r.register::<NoParams, ProjectSummary, _>(
            "project.save",
            "Write the open project to disk",
            Scope::ProjectWrite,
            |ctx, _| {
                ctx.with_project(|p| {
                    p.save()?;
                    Ok((summary(p), Vec::new()))
                })
            },
        );
        r.register::<NoParams, ProjectSummary, _>(
            "project.close",
            "Close the open project (unsaved changes stay in the journal)",
            Scope::ProjectRead,
            |ctx, _| {
                let s = ctx.with_project(|p| Ok(summary(p)))?;
                ctx.set_project(None)?;
                Ok((s, Vec::new()))
            },
        );
        r.register::<ProjectSetName, ProjectSummary, _>(
            "project.set_name",
            "Rename the open project",
            Scope::ProjectWrite,
            |ctx, args| {
                ctx.with_project(|p| {
                    mutate(
                        p,
                        "project.set_name",
                        serde_json::json!({ "name": args.name }),
                    )?;
                    Ok((summary(p), Vec::new()))
                })
            },
        );
        r.register::<ProjectSetSettings, ProjectSummary, _>(
            "project.set_settings",
            "Change project settings (autosave interval, cache limit, preview LOD); fields left out keep their value — journaled like every mutation",
            Scope::ProjectWrite,
            |ctx, args| {
                ctx.with_project(|p| {
                    mutate(p, "project.set_settings", serde_json::to_value(&args)?)?;
                    Ok((summary(p), Vec::new()))
                })
            },
        );
        r.register::<Recover, RecoverResult, _>(
            "project.recover",
            "Replay unsaved journal entries found when the project was opened, then save (entries that fail stay pending unless discard_failed)",
            Scope::ProjectWrite,
            |ctx, args| {
                ctx.with_project(|p| {
                    let entries = p.take_pending_journal();
                    let mut replayed = Vec::new();
                    let mut failed = Vec::new();
                    let mut failed_entries = Vec::new();
                    for e in entries {
                        match apply_mutation(p, &e.op, &e.params) {
                            Ok(()) => replayed.push(e),
                            Err(err) => {
                                failed.push((e.op.clone(), err.message));
                                failed_entries.push(e);
                            }
                        }
                    }
                    let mut warnings = Vec::new();
                    if failed_entries.is_empty() || args.discard_failed {
                        p.save()?;
                    } else {
                        // Saving would truncate the journal and lose these edits for good; keep them
                        // pending so the user can fix the cause (e.g. install the plugin) and retry.
                        p.pending_journal = failed_entries;
                        warnings.push(Warning::new(
                            "W_RECOVER_INCOMPLETE",
                            "some journal entries could not be replayed; nothing was saved — fix the cause and run project.recover again, or pass discard_failed to drop them",
                        ));
                    }
                    Ok((RecoverResult { replayed, failed }, warnings))
                })
            },
        );
        r.register::<ImportAsset, ImportedAsset, _>(
            "project.import_asset",
            "Copy a file (DEM, heightmap, texture) into the project's assets/ so nodes can reference it by name",
            Scope::FsWrite,
            |ctx, a| {
                let name = a
                    .name
                    .clone()
                    .or_else(|| a.path.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .ok_or_else(|| JdError::invalid("path", "source has no file name"))?;
                // A single normal path component (no separators, prefixes, `.` or `..`).
                let mut comps = Path::new(&name).components();
                // `:` is rejected on every OS so a project stays portable to Windows (drive prefixes).
                if name.contains(':') || !matches!((comps.next(), comps.next()), (Some(std::path::Component::Normal(_)), None)) {
                    return Err(JdError::invalid("name", "must be a plain file name"));
                }
                if !a.path.is_absolute() {
                    return Err(JdError::invalid("path", "source must be an absolute path")
                        .why("Relative paths would resolve against the server's working directory, outside any workspace check."));
                }
                let dest = ctx.with_project(|p| Ok(p.root().join("assets").join(&name)))?;
                if dest.exists() {
                    return Err(JdError::invalid("name", format!("asset `{name}` already exists"))
                        .next("Pick another name; graphs reference assets by name, so existing files are never replaced."));
                }
                std::fs::create_dir_all(dest.parent().unwrap_or(&dest))?;
                let bytes = std::fs::copy(&a.path, &dest)?;
                // Informational: a corrupt TIFF still imports (the input node reports the error later).
                let geo = jd_terrain::geotiff_tags(&dest).ok().flatten();
                Ok((ImportedAsset { name, bytes, geo }, Vec::new()))
            },
        );
        r.register::<NoParams, Vec<crate::jobs::JobInfo>, _>(
            "jobs.list",
            "Running jobs with progress (steps done/total, current node) — poll while an evaluation runs",
            Scope::None,
            |ctx, _| Ok((ctx.jobs.list(), Vec::new())),
        );
        r.register::<CancelJob, bool, _>(
            "jobs.cancel",
            "Request cancellation of a running job; it stops at the next node or kernel iteration with E_CANCELLED",
            Scope::None,
            |ctx, j| Ok((ctx.jobs.cancel(j.id), Vec::new())),
        );
        r.register::<CancelJob, crate::jobs::JobResult, _>(
            "jobs.result",
            "State of a job: running (progress), done (the finished op's envelope, handed over once) or unknown",
            Scope::None,
            |ctx, j| Ok((ctx.jobs.result(j.id), Vec::new())),
        );
        r.register::<NoParams, jd_cache::GcReport, _>(
            "project.cache_gc",
            "Evict the oldest cached layer blobs until the project's cache fits `settings.cache_limit_bytes` (the cache is disposable); reports what is left",
            Scope::ProjectWrite,
            |ctx, _| {
                let (root, limit) = ctx.with_project(|p| Ok((p.root().to_path_buf(), p.doc.settings.cache_limit_bytes)))?;
                let _g = ctx.layers.lock().map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "layer lock poisoned"))?;
                Ok((jd_cache::LayerCache::open(&root).with_limit(limit).gc()?, Vec::new()))
            },
        );
        r.register::<NoParams, crate::diag::EnvInfo, _>(
            "diag.env",
            "Report host environment and detected tools",
            Scope::None,
            |_, _| Ok((crate::diag::env_info(), Vec::new())),
        );
        r.register::<crate::diag::BenchParams, crate::diag::BenchReport, _>(
            "diag.bench",
            "Run a benchmark scene (S1: fBm + ridged + add + thermal×50) with the real kernels; per-stage wall time, peak RSS, content hash",
            Scope::None,
            |_, p| Ok((crate::diag::bench(&p)?, Vec::new())),
        );
        terrain::register_all(r);
        graph::register_all(r);
    }
}

/// Graph operations: journaled mutations of `graph.json` and incremental evaluation that
/// publishes `jd.output.height` nodes as layers (consumed by `terrain.*` ops and the viewport).
pub mod graph {
    use super::builtin::NoParams;
    use super::*;
    use jd_graph::model::PortRef;
    use jd_graph::{Cache, Edge, Evaluator, Graph, Node, NodeKind};
    use jd_types::NodeId;
    use serde_json::Map;

    fn registry() -> &'static jd_graph::Registry {
        static REG: std::sync::OnceLock<jd_graph::Registry> = std::sync::OnceLock::new();
        REG.get_or_init(jd_graph::Registry::with_builtins)
    }

    fn cache() -> &'static Cache {
        // ponytail: process-wide in-memory cache (cap 256 node results); the disk cache comes with jobs.
        static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
        CACHE.get_or_init(|| Cache::with_capacity(256))
    }

    /// `graph.add_node` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct AddNode {
        /// Kind `name@version`, e.g. `jd.noise.fbm@1`.
        pub kind: NodeKind,
        /// Parameters (validated against the kind's schema before journaling).
        #[serde(default)]
        pub params: Map<String, Value>,
        /// Optional client-chosen id (ULID); generated when absent.
        #[serde(default)]
        pub id: Option<NodeId>,
        /// Editor position; when omitted the node lands to the right of the current graph so
        /// scripted additions (CLI, MCP) never stack at the origin.
        #[serde(default)]
        pub position: Option<[f32; 2]>,
    }

    /// `graph.remove_node` / `graph.set_bypass` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct NodeRef {
        /// Node id.
        pub id: NodeId,
        /// For `set_bypass`: the new flag.
        #[serde(default)]
        pub bypass: Option<bool>,
    }

    /// `graph.connect` / `graph.disconnect` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Connect {
        /// Source node.
        pub from: NodeId,
        /// Source output port (default `out`).
        #[serde(default = "out_port")]
        pub from_port: String,
        /// Destination node.
        pub to: NodeId,
        /// Destination input port.
        pub to_port: String,
    }

    fn out_port() -> String {
        "out".into()
    }

    /// `graph.set_param` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct SetParam {
        /// Node id.
        pub id: NodeId,
        /// Parameter key.
        pub key: String,
        /// New value (`null` removes the key, restoring the default).
        pub value: Value,
    }

    /// `graph.append_param` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct AppendParam {
        /// Node id.
        pub id: NodeId,
        /// Array-valued parameter key (created when absent).
        pub key: String,
        /// Element to push.
        pub value: Value,
    }

    /// `graph.evaluate` params.
    #[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Evaluate {
        /// Level of detail (0 = full resolution).
        #[serde(default)]
        pub lod: u8,
    }

    /// Node as reported by `graph.list`.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct NodeView {
        /// Id.
        pub id: NodeId,
        /// Node.
        #[serde(flatten)]
        pub node: Node,
    }

    /// `graph.list` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct GraphView {
        /// Nodes.
        pub nodes: Vec<NodeView>,
        /// Edges.
        pub edges: Vec<Edge>,
        /// Number of edits `graph.undo` can take back.
        pub undo_depth: usize,
        /// Number of undone edits `graph.redo` can re-apply.
        pub redo_depth: usize,
    }

    /// `graph.evaluate` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct EvaluateResult {
        /// Per-node report.
        #[serde(flatten)]
        pub report: jd_graph::EvalReport,
        /// Layers published by output nodes (layer name → content hash).
        pub layers: BTreeMap<String, String>,
    }

    /// `graph.add_node` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct AddedNode {
        /// Id of the new node.
        pub id: NodeId,
    }

    /// `graph.restore` params (how undo/redo are journaled).
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Restore {
        /// The whole graph document to install.
        pub graph: Graph,
    }

    /// `graph.move_node` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct MoveNode {
        /// Node id.
        pub id: NodeId,
        /// New editor position.
        pub position: [f32; 2],
    }

    fn node_mut<'a>(g: &'a mut Graph, id: &NodeId) -> JdResult<&'a mut Node> {
        g.nodes.get_mut(id).ok_or_else(|| {
            JdError::new(ErrorCode::E_INPUT_INVALID, format!("no node {id}"))
                .detail("node", id.to_string())
        })
    }

    fn view(p: &Project) -> GraphView {
        let (undo_depth, redo_depth) = p.history.depths();
        GraphView {
            nodes: p
                .graph
                .nodes
                .iter()
                .map(|(id, n)| NodeView {
                    id: id.clone(),
                    node: n.clone(),
                })
                .collect(),
            edges: p.graph.edges.clone(),
            undo_depth,
            redo_depth,
        }
    }

    /// Validate a graph mutation against the current document **before** it is journaled, so a
    /// rejected request (unknown kind, bad params, missing node/port, type mismatch, cycle) never
    /// leaves an entry on disk. Applies the mutation to a copy and checks the result structurally.
    pub fn validate_mutation(p: &Project, op: &str, params: &Value) -> JdResult<()> {
        // ponytail: graphs are a few hundred nodes at most; cloning per mutation is cheaper than
        // keeping bespoke pre-checks in sync with `apply`.
        if op == "graph.restore" {
            // A snapshot is a state the document already had (undo/redo); only its shape is checked.
            let _: Restore = serde_json::from_value(params.clone())?;
            return Ok(());
        }
        let mut trial = p.graph.clone();
        apply(&mut trial, op, params)?;
        let ev = Evaluator::new(registry(), cache());
        let after = ev.structure_errors(&trial);
        if after.is_empty() {
            return Ok(());
        }
        // A document that is already invalid (missing plugin, hand-edited graph.json) must stay
        // editable: reject only the problems this mutation introduces.
        let key = |e: &JdError| format!("{:?}|{}|{:?}", e.code, e.message, e.details);
        let before: Vec<String> = ev.structure_errors(&p.graph).iter().map(key).collect();
        match after.into_iter().find(|e| !before.contains(&key(e))) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Apply a graph mutation to the in-memory document (live and during journal replay).
    pub fn apply_mutation(p: &mut Project, op: &str, params: &Value) -> JdResult<()> {
        apply(&mut p.graph, op, params)
    }

    fn apply(g: &mut Graph, op: &str, params: &Value) -> JdResult<()> {
        match op {
            "graph.add_node" => {
                let a: AddNode = serde_json::from_value(params.clone())?;
                let id = a.id.unwrap_or_default();
                if g.nodes.contains_key(&id) {
                    return Err(JdError::new(
                        ErrorCode::E_INPUT_INVALID,
                        format!("node {id} already exists"),
                    ));
                }
                g.nodes.insert(
                    id,
                    Node {
                        kind: a.kind,
                        params: a.params,
                        bypass: false,
                        ui: jd_graph::model::UiMeta {
                            position: a.position.unwrap_or_else(|| {
                                let right = g
                                    .nodes
                                    .values()
                                    .map(|n| n.ui.position[0])
                                    .fold(-320.0f32, f32::max);
                                [right + 320.0, 0.0]
                            }),
                            label: None,
                        },
                    },
                );
            }
            "graph.remove_node" => {
                let n: NodeRef = serde_json::from_value(params.clone())?;
                g.nodes.remove(&n.id).ok_or_else(|| {
                    JdError::new(ErrorCode::E_INPUT_INVALID, format!("no node {}", n.id))
                })?;
                g.edges.retain(|e| e.from.node != n.id && e.to.node != n.id);
            }
            "graph.set_bypass" => {
                let n: NodeRef = serde_json::from_value(params.clone())?;
                node_mut(g, &n.id)?.bypass = n.bypass.unwrap_or(true);
            }
            "graph.move_node" => {
                let m: MoveNode = serde_json::from_value(params.clone())?;
                node_mut(g, &m.id)?.ui.position = m.position;
            }
            "graph.restore" => {
                let r: Restore = serde_json::from_value(params.clone())?;
                *g = r.graph;
            }
            "graph.connect" => {
                let c: Connect = serde_json::from_value(params.clone())?;
                // One edge per input port: replacing keeps the graph a function of its inputs.
                // Ports, types and cycles are checked by `validate_structure` on the trial copy.
                g.edges
                    .retain(|e| !(e.to.node == c.to && e.to.port == c.to_port));
                g.edges.push(Edge {
                    from: PortRef {
                        node: c.from,
                        port: c.from_port,
                    },
                    to: PortRef {
                        node: c.to,
                        port: c.to_port,
                    },
                });
            }
            "graph.disconnect" => {
                let c: Connect = serde_json::from_value(params.clone())?;
                let before = g.edges.len();
                g.edges.retain(|e| {
                    !(e.from.node == c.from
                        && e.from.port == c.from_port
                        && e.to.node == c.to
                        && e.to.port == c.to_port)
                });
                if g.edges.len() == before {
                    return Err(JdError::new(ErrorCode::E_INPUT_INVALID, "no such edge"));
                }
            }
            "graph.set_param" => {
                let s: SetParam = serde_json::from_value(params.clone())?;
                let node = node_mut(g, &s.id)?;
                if s.value.is_null() {
                    node.params.remove(&s.key);
                } else {
                    node.params.insert(s.key, s.value);
                }
            }
            "graph.append_param" => {
                let a: AppendParam = serde_json::from_value(params.clone())?;
                let node = node_mut(g, &a.id)?;
                match node
                    .params
                    .entry(a.key.clone())
                    .or_insert_with(|| Value::Array(Vec::new()))
                {
                    Value::Array(items) => items.push(a.value),
                    _ => {
                        return Err(JdError::new(
                            ErrorCode::E_INPUT_INVALID,
                            format!("parameter `{}` is not an array", a.key),
                        )
                        .next("Use graph.set_param to replace a scalar parameter."));
                    }
                }
            }
            other => {
                return Err(JdError::new(
                    ErrorCode::E_INPUT_INVALID,
                    format!("`{other}` is not a graph mutation"),
                ));
            }
        }
        Ok(())
    }

    /// Evaluate the open project's graph under a job control handle and publish its outputs.
    fn evaluate(
        ctx: &Context,
        lod: u8,
        job_id: u64,
        ctl: &jd_graph::JobCtl,
    ) -> JdResult<(EvaluateResult, Vec<Warning>)> {
        let (graph, world, root) = ctx
            .with_project(|p| Ok((p.graph.clone(), p.doc.world.clone(), p.root().to_path_buf())))?;
        // Two outputs publishing the same layer would silently overwrite each other.
        let mut by_layer: BTreeMap<String, &NodeId> = BTreeMap::new();
        for (id, node) in &graph.nodes {
            if node.kind.0.starts_with("jd.output.height@")
                && let Some(other) =
                    by_layer.insert(jd_graph::nodes::output_layer_name(&node.params), id)
            {
                return Err(JdError::new(
                    ErrorCode::E_INPUT_INVALID,
                    format!(
                        "two output nodes publish the layer `{}`",
                        jd_graph::nodes::output_layer_name(&node.params)
                    ),
                )
                .next("Give one of them a different `layer` parameter.")
                .detail("nodes", vec![other.to_string(), id.to_string()]));
            }
        }
        let (outputs, report) = Evaluator::new(registry(), cache())
            .evaluate_job(&graph, &world, lod, Some(&root.join("assets")), ctl)
            .map_err(|err| {
                if err.code == ErrorCode::E_CANCELLED {
                    err.detail("job_id", job_id)
                } else {
                    err
                }
            })?;
        let mut layers = BTreeMap::new();
        for (id, node) in &graph.nodes {
            if node.kind.0.starts_with("jd.output.height@")
                && let Some(v) = outputs.get(&(id.clone(), "out".to_owned()))
            {
                let mut name = jd_graph::nodes::output_layer_name(&node.params);
                if lod > 0 {
                    // Never overwrite the full-resolution layer with a preview.
                    name = format!("{name}#lod{}", lod);
                }
                let hf = v.as_field();
                layers.insert(name.clone(), v.content_hash().to_hex().to_string());
                ctx.set_layer_for(&root, &name, (**hf).clone())?;
            }
        }
        let mut warnings = Vec::new();
        if layers.is_empty() {
            warnings.push(Warning::new(
                "W_NO_OUTPUT",
                "graph has no jd.output.height node; nothing was published as a layer",
            ));
        }
        Ok((EvaluateResult { report, layers }, warnings))
    }

    /// Register graph ops.
    pub fn register_all(r: &mut Registry) {
        r.register::<NoParams, Vec<jd_graph::registry::NodeDefInfo>, _>(
            "graph.kinds",
            "List available node kinds with ports and parameter schemas",
            Scope::None,
            |_, _| Ok((registry().list(), Vec::new())),
        );
        r.register::<NoParams, GraphView, _>(
            "graph.list",
            "Nodes and edges of the open project's graph",
            Scope::ProjectRead,
            |ctx, _| ctx.with_project(|p| Ok((view(p), Vec::new()))),
        );
        r.register::<AddNode, AddedNode, _>("graph.add_node", "Add a node (journaled)", Scope::ProjectWrite, |ctx, a| {
            let id = a.id.clone().unwrap_or_default();
            let params = serde_json::json!({ "kind": a.kind, "params": a.params, "id": id, "position": a.position });
            ctx.with_project(|p| {
                builtin::mutate(p, "graph.add_node", params)?;
                Ok((AddedNode { id }, Vec::new()))
            })
        });
        for (op, desc) in [
            (
                "graph.remove_node",
                "Remove a node and its edges (journaled)",
            ),
            ("graph.set_bypass", "Bypass or un-bypass a node (journaled)"),
        ] {
            r.register::<NodeRef, GraphView, _>(op, desc, Scope::ProjectWrite, move |ctx, n| {
                let params = serde_json::json!({ "id": n.id, "bypass": n.bypass });
                ctx.with_project(|p| {
                    builtin::mutate(p, op, params)?;
                    Ok((view(p), Vec::new()))
                })
            });
        }
        r.register::<MoveNode, GraphView, _>(
            "graph.move_node",
            "Move a node on the editor canvas (journaled)",
            Scope::ProjectWrite,
            |ctx, m| {
                let params = serde_json::json!({ "id": m.id, "position": m.position });
                ctx.with_project(|p| {
                    builtin::mutate(p, "graph.move_node", params)?;
                    Ok((view(p), Vec::new()))
                })
            },
        );
        for (op, desc) in [
            (
                "graph.connect",
                "Connect an output to an input (journaled; rejects bad ports, type mismatches and cycles)",
            ),
            ("graph.disconnect", "Remove an edge (journaled)"),
        ] {
            r.register::<Connect, GraphView, _>(op, desc, Scope::ProjectWrite, move |ctx, c| {
                let params = serde_json::json!({ "from": c.from, "from_port": c.from_port, "to": c.to, "to_port": c.to_port });
                ctx.with_project(|p| {
                    builtin::mutate(p, op, params)?;
                    Ok((view(p), Vec::new()))
                })
            });
        }
        for (op, desc) in [
            (
                "graph.undo",
                "Take back the last graph edit (journaled as graph.restore)",
            ),
            (
                "graph.redo",
                "Re-apply the last undone graph edit (journaled as graph.restore)",
            ),
        ] {
            r.register::<NoParams, GraphView, _>(op, desc, Scope::ProjectWrite, move |ctx, _| {
                ctx.with_project(|p| {
                    let undo = op == "graph.undo";
                    // Peek, apply, then commit the stacks: a restore that fails to journal or
                    // validate leaves history exactly as it was.
                    let target = if undo {
                        p.history.peek_undo()
                    } else {
                        p.history.peek_redo()
                    }
                    .cloned();
                    let Some(graph) = target else {
                        return Err(JdError::new(
                            ErrorCode::E_INPUT_INVALID,
                            format!("nothing to {}", &op[6..]),
                        )
                        .detail("undo_depth", p.history.depths().0)
                        .detail("redo_depth", p.history.depths().1));
                    };
                    let current = p.graph.clone();
                    builtin::mutate(p, "graph.restore", serde_json::json!({ "graph": graph }))?;
                    if undo {
                        p.history.undo(current);
                    } else {
                        p.history.redo(current);
                    }
                    Ok((view(p), Vec::new()))
                })
            });
        }
        r.register::<SetParam, GraphView, _>(
            "graph.set_param",
            "Set (or null to reset) a node parameter (journaled; validated against the kind's schema)",
            Scope::ProjectWrite,
            |ctx, s| {
                let params = serde_json::json!({ "id": s.id, "key": s.key, "value": s.value });
                ctx.with_project(|p| {
                    builtin::mutate(p, "graph.set_param", params)?;
                    Ok((view(p), Vec::new()))
                })
            },
        );
        r.register::<AppendParam, GraphView, _>(
            "graph.append_param",
            "Push one element onto an array parameter (brush strokes): journaled with just the element, so a long sculpt session grows the journal linearly",
            Scope::ProjectWrite,
            |ctx, a| {
                let params = serde_json::json!({ "id": a.id, "key": a.key, "value": a.value });
                ctx.with_project(|p| {
                    builtin::mutate(p, "graph.append_param", params)?;
                    Ok((view(p), Vec::new()))
                })
            },
        );
        r.register::<Evaluate, EvaluateResult, _>(
            "graph.evaluate",
            "Validate and evaluate the graph incrementally; publishes output nodes as layers (`<layer>#lod<N>` when lod > 0)",
            Scope::ProjectWrite,
            |ctx, e| {
                let job = ctx.jobs.start("graph.evaluate");
                evaluate(ctx, e.lod, job.id, &job.ctl)
            },
        );
        r.register_detached::<Evaluate, _>(
            "graph.evaluate_async",
            "Start graph.evaluate on a worker thread and return at once with `status: accepted` + `job_id`; follow it with jobs.list, cancel with jobs.cancel, collect the envelope with jobs.result",
            Scope::ProjectWrite,
            |ctx, e| {
                ctx.with_project(|_| Ok(()))?; // fail now, not on the thread, when nothing is open
                let guard = ctx.jobs.start_owned("graph.evaluate");
                let id = guard.id;
                let ctx2 = ctx.clone();
                std::thread::Builder::new()
                    .name(format!("jd-job-{id}"))
                    .spawn(move || {
                        let envelope = match evaluate(&ctx2, e.lod, id, &guard.ctl) {
                            Ok((r, w)) => serde_json::json!({ "status": "ok", "result": r, "warnings": w }),
                            Err(err) => serde_json::json!({ "status": "error", "error": err }),
                        };
                        ctx2.jobs.finish(id, envelope);
                        drop(guard);
                    })
                    .map_err(|e| JdError::new(ErrorCode::E_INTERNAL, format!("cannot spawn job thread: {e}")))?;
                Ok(id)
            },
        );
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn call(reg: &Registry, ctx: &Context, op: &str, params: Value) -> Response {
            reg.invoke(ctx, Request::new(op, params))
        }

        fn wait_done(reg: &Registry, ctx: &Context, id: u64) -> Value {
            for _ in 0..600 {
                let r = call(reg, ctx, "jobs.result", serde_json::json!({ "id": id }));
                let res = r.result.expect("result");
                if res["state"] == "done" {
                    return res["response"].clone();
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            panic!("job {id} did not finish");
        }

        #[test]
        fn detached_evaluate_is_accepted_then_collected_and_can_be_cancelled() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join("d.jdproj");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [64, 64],
                extent_m: [64.0, 64.0],
                tile_size: 64,
                halo: 4,
                ..Default::default()
            };
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.create",
                    serde_json::json!({ "path": path, "name": "D", "world": world })
                )
                .status,
                crate::envelope::Status::Ok
            );
            // Nothing open → the error comes back synchronously, not from a thread.
            let closed = Context::default();
            assert_eq!(
                call(&reg, &closed, "graph.evaluate_async", serde_json::json!({})).status,
                crate::envelope::Status::Error
            );
            // Accepted + job id; the envelope arrives through jobs.result exactly once.
            let r = call(&reg, &ctx, "graph.evaluate_async", serde_json::json!({}));
            assert_eq!(r.status, crate::envelope::Status::Accepted, "{:?}", r.error);
            let id_u64 = r.job_id.expect("job id");
            let env = wait_done(&reg, &ctx, id_u64);
            assert_eq!(env["status"], "ok", "{env}");
            assert!(env["result"]["layers"]["height"].is_string(), "{env}");
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "jobs.result",
                    serde_json::json!({ "id": id_u64 })
                )
                .result
                .expect("r")["state"],
                "unknown",
                "collected once"
            );
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "terrain.stats",
                    serde_json::json!({ "layer": "height" })
                )
                .status,
                crate::envelope::Status::Ok
            );
            // A long detached job can be cancelled; its envelope carries E_CANCELLED.
            let add = call(
                &reg,
                &ctx,
                "graph.add_node",
                serde_json::json!({ "kind": "jd.erosion.hydraulic@1", "params": { "iterations": 2000 } }),
            );
            let hid = add.result.expect("id")["id"]
                .as_str()
                .expect("str")
                .to_owned();
            let view = call(&reg, &ctx, "graph.list", serde_json::json!({}))
                .result
                .expect("view");
            let noise = view["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["kind"].as_str().unwrap_or("").starts_with("jd.noise"))
                .expect("noise")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            let output = view["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["kind"].as_str().unwrap_or("").starts_with("jd.output"))
                .expect("out")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": noise, "to": hid, "to_port": "in" }),
            );
            call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": hid, "from_port": "out", "to": output, "to_port": "in" }),
            );
            let r = call(&reg, &ctx, "graph.evaluate_async", serde_json::json!({}));
            assert_eq!(r.status, crate::envelope::Status::Accepted, "{:?}", r.error);
            let id2 = r.job_id.expect("id");
            // Visible in jobs.list while running, then cancelled.
            let mut seen = false;
            for _ in 0..200 {
                let listed = call(&reg, &ctx, "jobs.list", serde_json::json!({}))
                    .result
                    .expect("list");
                if listed
                    .as_array()
                    .expect("arr")
                    .iter()
                    .any(|j| j["id"] == id2)
                {
                    seen = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(seen, "detached job never showed in jobs.list");
            assert_eq!(
                call(&reg, &ctx, "jobs.cancel", serde_json::json!({ "id": id2 })).result,
                Some(serde_json::json!(true))
            );
            let env = wait_done(&reg, &ctx, id2);
            assert_eq!(env["status"], "error", "{env}");
            assert_eq!(env["error"]["code"], "E_CANCELLED", "{env}");
        }

        #[test]
        fn append_param_pushes_strokes_and_keeps_the_journal_linear() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join("a.jdproj");
            let ctx = Context::default();
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.create",
                    serde_json::json!({ "path": path, "name": "A" })
                )
                .status,
                crate::envelope::Status::Ok
            );
            let add = call(
                &reg,
                &ctx,
                "graph.add_node",
                serde_json::json!({ "kind": "jd.sculpt.strokes@1" }),
            );
            let id = add.result.expect("id")["id"]
                .as_str()
                .expect("str")
                .to_owned();
            let stroke = |i: usize| serde_json::json!({ "points": [[10.0 * i as f64, 5.0], [10.0 * i as f64 + 3.0, 6.0]], "radius_m": 8.0, "strength_m": 2.0, "mode": "raise", "amount": 0.0 });
            for i in 0..20 {
                let r = call(
                    &reg,
                    &ctx,
                    "graph.append_param",
                    serde_json::json!({ "id": id, "key": "strokes", "value": stroke(i) }),
                );
                assert_eq!(r.status, crate::envelope::Status::Ok, "{:?}", r.error);
            }
            let view = call(&reg, &ctx, "graph.list", serde_json::json!({}))
                .result
                .expect("view");
            let node = view["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["id"] == id)
                .expect("node");
            assert_eq!(
                node["params"]["strokes"].as_array().map(|a| a.len()),
                Some(20)
            );
            // Appending to a scalar is refused before anything is journaled.
            let bad = call(
                &reg,
                &ctx,
                "graph.append_param",
                serde_json::json!({ "id": id, "key": "strokes", "value": 1 }),
            );
            assert_eq!(
                bad.error.map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID),
                "schema rejects a non-stroke element"
            );
            // The journal holds one element per entry: 20 appends are far smaller than 20 full-array set_params.
            let appended = std::fs::metadata(path.join("journal").join("ops.ndjson"))
                .expect("journal")
                .len();
            let ctx2 = Context::default();
            let path2 = dir.path().join("b.jdproj");
            call(
                &reg,
                &ctx2,
                "project.create",
                serde_json::json!({ "path": path2, "name": "B" }),
            );
            let add2 = call(
                &reg,
                &ctx2,
                "graph.add_node",
                serde_json::json!({ "kind": "jd.sculpt.strokes@1" }),
            );
            let id2 = add2.result.expect("id")["id"]
                .as_str()
                .expect("str")
                .to_owned();
            let mut all = Vec::new();
            for i in 0..20 {
                all.push(stroke(i));
                call(
                    &reg,
                    &ctx2,
                    "graph.set_param",
                    serde_json::json!({ "id": id2, "key": "strokes", "value": all }),
                );
            }
            let replaced = std::fs::metadata(path2.join("journal").join("ops.ndjson"))
                .expect("journal")
                .len();
            assert!(
                appended * 3 < replaced,
                "append {appended} B vs set_param {replaced} B"
            );
        }

        #[test]
        fn set_settings_merges_validates_and_journals() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join("s.jdproj");
            let ctx = Context::default();
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.create",
                    serde_json::json!({ "path": path, "name": "S" })
                )
                .status,
                crate::envelope::Status::Ok
            );
            let r = call(
                &reg,
                &ctx,
                "project.set_settings",
                serde_json::json!({ "autosave_interval_s": 1 }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{:?}", r.error);
            let settings = &r.result.expect("summary")["settings"];
            assert_eq!(settings["autosave_interval_s"], 1);
            assert_eq!(
                settings["cache_limit_bytes"], 2_147_483_648u64,
                "untouched fields keep their value"
            );
            let bad = call(
                &reg,
                &ctx,
                "project.set_settings",
                serde_json::json!({ "cache_limit_bytes": 8 }),
            );
            assert_eq!(bad.error.map(|e| e.code), Some(ErrorCode::E_INPUT_INVALID));
            let info = call(&reg, &ctx, "project.info", serde_json::json!({}));
            assert_eq!(
                info.result.expect("info")["settings"]["cache_limit_bytes"],
                2_147_483_648u64,
                "a refused edit changes nothing"
            );
            let journal =
                std::fs::read_to_string(path.join("journal").join("ops.ndjson")).expect("journal");
            let ops: Vec<String> = journal
                .lines()
                .map(|l| {
                    serde_json::from_str::<Value>(l).expect("entry")["op"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned()
                })
                .collect();
            assert_eq!(
                ops,
                vec!["project.set_settings"],
                "the refused edit was never journaled"
            );
        }

        #[test]
        fn build_graph_evaluate_publish_layer_and_recover_mutations() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join("g.jdproj");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [96, 64],
                extent_m: [96.0, 64.0],
                tile_size: 64,
                halo: 4,
                ..Default::default()
            };
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.create",
                    serde_json::json!({ "path": path, "name": "G", "world": world })
                )
                .status,
                crate::envelope::Status::Ok
            );
            let kinds = call(&reg, &ctx, "graph.kinds", serde_json::json!({}));
            assert!(
                kinds
                    .result
                    .as_ref()
                    .and_then(|v| v.as_array())
                    .map(|a| a.len() >= 7)
                    .unwrap_or(false)
            );
            let noise = call(
                &reg,
                &ctx,
                "graph.add_node",
                serde_json::json!({ "kind": "jd.noise.fbm@1", "params": { "seed": 5, "height_range_m": [0.0, 10.0] } }),
            );
            let noise_id = noise.result.expect("added")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            // No position given: placed right of the starter graph (output sits at x = 320).
            let listed = call(&reg, &ctx, "graph.list", serde_json::json!({}));
            let placed = listed.result.expect("list")["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["id"] == noise_id)
                .map(|n| n["ui"]["position"].clone())
                .expect("new node");
            assert_eq!(placed, serde_json::json!([640.0, 0.0]));
            let out = call(
                &reg,
                &ctx,
                "graph.add_node",
                serde_json::json!({ "kind": "jd.output.height@1", "params": { "layer": "height" } }),
            );
            let out_id = out.result.expect("added")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            // Two outputs publishing the same layer (the starter graph already has `height`) are refused.
            let r = call(&reg, &ctx, "graph.evaluate", serde_json::json!({}));
            let e = r.error.expect("duplicate layer");
            assert_eq!(e.code, ErrorCode::E_INPUT_INVALID);
            assert_eq!(
                e.details["nodes"].as_array().map(Vec::len),
                Some(2),
                "{e:?}"
            );
            let r = call(
                &reg,
                &ctx,
                "graph.set_param",
                serde_json::json!({ "id": out_id, "key": "layer", "value": "height2" }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            // Evaluate before connecting: missing input is a stable error.
            let r = call(&reg, &ctx, "graph.evaluate", serde_json::json!({}));
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_GRAPH_MISSING_INPUT)
            );
            let r = call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": noise_id, "to": out_id, "to_port": "in" }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            // A cycle is refused and leaves the document unchanged.
            let r = call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": out_id, "to": noise_id, "to_port": "in" }),
            );
            assert!(r.error.is_some());
            let r = call(
                &reg,
                &ctx,
                "graph.evaluate",
                serde_json::json!({ "lod": 1 }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            let res = r.result.expect("result");
            assert_eq!(res["layers"].as_object().map(|m| m.len()), Some(2));
            assert_eq!(res["nodes"].as_array().map(Vec::len), Some(4));
            // lod > 0 publishes under a suffixed name so the full-resolution layer survives previews.
            assert_eq!(
                res["layers"]
                    .as_object()
                    .and_then(|m| m.keys().next().cloned()),
                Some("height#lod1".to_owned())
            );
            let r = call(
                &reg,
                &ctx,
                "terrain.stats",
                serde_json::json!({ "layer": "height#lod1" }),
            );
            assert_eq!(
                r.result
                    .as_ref()
                    .and_then(|v| v["size_px"].as_array())
                    .map(|a| a.len()),
                Some(2)
            );
            // Unknown kind is rejected before journaling.
            let r = call(
                &reg,
                &ctx,
                "graph.add_node",
                serde_json::json!({ "kind": "jd.nope@1" }),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_NODE_UNKNOWN)
            );
            // Bad params and bad ports are rejected before journaling.
            let r = call(
                &reg,
                &ctx,
                "graph.set_param",
                serde_json::json!({ "id": noise_id, "key": "octaves", "value": "many" }),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID),
                "{r:?}"
            );
            let r = call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": noise_id, "from_port": "nope", "to": out_id, "to_port": "in" }),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID),
                "{r:?}"
            );
            // Moving a node is journaled too.
            let r = call(
                &reg,
                &ctx,
                "graph.move_node",
                serde_json::json!({ "id": noise_id, "position": [10.0, 20.0] }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            // Undo takes the move back, redo re-applies it; both are journaled as graph.restore.
            let r = call(&reg, &ctx, "graph.undo", serde_json::json!({}));
            let v = r.result.expect("undo");
            assert_eq!(
                (v["undo_depth"].as_u64(), v["redo_depth"].as_u64()),
                (Some(4), Some(1))
            );
            let n = v["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["id"] == noise_id)
                .expect("noise");
            assert_eq!(
                n["ui"]["position"],
                serde_json::json!([640.0, 0.0]),
                "position before the move restored"
            );
            let r = call(&reg, &ctx, "graph.redo", serde_json::json!({}));
            let v = r.result.expect("redo");
            assert_eq!(
                (v["undo_depth"].as_u64(), v["redo_depth"].as_u64()),
                (Some(5), Some(0))
            );
            let r = call(&reg, &ctx, "graph.redo", serde_json::json!({}));
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID),
                "nothing left to redo"
            );
            // Crash without saving → reopen → recover replays the seven graph mutations (incl. the two restores).
            ctx.set_project(None).expect("close");
            let ctx2 = Context::default();
            let r = call(
                &reg,
                &ctx2,
                "project.open",
                serde_json::json!({ "path": path }),
            );
            assert_eq!(
                r.result
                    .as_ref()
                    .and_then(|v| v["pending_journal"].as_u64()),
                Some(7)
            );
            // The evaluated layer came back from the project's on-disk cache (jd-cache).
            let r = call(
                &reg,
                &ctx2,
                "terrain.stats",
                serde_json::json!({ "layer": "height#lod1" }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            let r = call(&reg, &ctx2, "project.recover", serde_json::json!({}));
            assert_eq!(
                r.result
                    .as_ref()
                    .and_then(|v| v["replayed"].as_array())
                    .map(Vec::len),
                Some(7)
            );
            assert_eq!(
                r.result
                    .as_ref()
                    .and_then(|v| v["failed"].as_array())
                    .map(Vec::len),
                Some(0)
            );
            let r = call(&reg, &ctx2, "graph.list", serde_json::json!({}));
            let g = r.result.expect("graph");
            assert_eq!(g["nodes"].as_array().map(Vec::len), Some(4));
            assert_eq!(g["edges"].as_array().map(Vec::len), Some(2));
            let moved = g["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["id"] == noise_id)
                .expect("noise");
            assert_eq!(moved["ui"]["position"], serde_json::json!([10.0, 20.0]));
        }

        #[test]
        fn import_asset_requires_absolute_source_and_never_overwrites() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let ctx = Context::default();
            let proj = dir.path().join("i.jdproj");
            call(
                &reg,
                &ctx,
                "project.create",
                serde_json::json!({ "path": proj, "name": "I" }),
            );
            let src = dir.path().join("dem.png");
            std::fs::write(&src, b"not really a png").expect("write");
            let r = call(
                &reg,
                &ctx,
                "project.import_asset",
                serde_json::json!({ "path": "dem.png" }),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID),
                "relative source refused"
            );
            let r = call(
                &reg,
                &ctx,
                "project.import_asset",
                serde_json::json!({ "path": src }),
            );
            assert_eq!(
                r.result.as_ref().and_then(|v| v["name"].as_str()),
                Some("dem.png"),
                "{r:?}"
            );
            let r = call(
                &reg,
                &ctx,
                "project.import_asset",
                serde_json::json!({ "path": src }),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID),
                "existing asset is never replaced"
            );
            assert_eq!(
                std::fs::read(proj.join("assets/dem.png"))
                    .expect("asset")
                    .len(),
                16
            );
            for bad in ["../x.png", "a/b.png", "C:x.png"] {
                let r = call(
                    &reg,
                    &ctx,
                    "project.import_asset",
                    serde_json::json!({ "path": src, "name": bad }),
                );
                assert_eq!(
                    r.error.as_ref().map(|e| e.code),
                    Some(ErrorCode::E_INPUT_INVALID),
                    "{bad}"
                );
            }
        }

        #[test]
        fn a_running_evaluation_is_listed_and_can_be_cancelled() {
            let reg = Arc::new(Registry::with_builtins());
            let dir = tempfile::tempdir().expect("tmp");
            let ctx = Context::default();
            let path = dir.path().join("j.jdproj");
            call(
                &reg,
                &ctx,
                "project.create",
                serde_json::json!({ "path": path, "name": "J", "world": WorldSettings { resolution_px: [96, 96], extent_m: [96.0, 96.0], tile_size: 64, halo: 4, ..Default::default() } }),
            );
            // A long hydraulic run between the starter noise and output.
            let g = call(&reg, &ctx, "graph.list", serde_json::json!({}))
                .result
                .expect("graph");
            let noise = g["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["kind"] == "jd.noise.fbm@1")
                .expect("noise")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            let out = g["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["kind"] == "jd.output.height@1")
                .expect("out")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            let hy = call(&reg, &ctx, "graph.add_node", serde_json::json!({ "kind": "jd.erosion.hydraulic@1", "params": { "iterations": 2000 } })).result.expect("added")["id"].as_str().expect("id").to_owned();
            call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": noise, "to": hy, "to_port": "in" }),
            );
            call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": hy, "to": out, "to_port": "in" }),
            );
            assert!(ctx.jobs.list().is_empty());
            let (reg2, ctx2) = (reg.clone(), ctx.clone());
            let worker = std::thread::spawn(move || {
                call(&reg2, &ctx2, "graph.evaluate", serde_json::json!({}))
            });
            // Wait until the job shows up, then cancel it through the op.
            let mut listed = None;
            for _ in 0..200 {
                let jobs = ctx.jobs.list();
                if let Some(j) = jobs.first() {
                    listed = Some(j.clone());
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let job = listed.expect("job listed while running");
            assert_eq!(job.op, "graph.evaluate");
            assert!(
                job.total >= 3 && job.done <= job.total,
                "progress is reported: {job:?}"
            );
            assert!(!job.current.is_empty(), "current node kind is reported");
            let r = call(
                &reg,
                &ctx,
                "jobs.cancel",
                serde_json::json!({ "id": job.id }),
            );
            assert_eq!(r.result, Some(serde_json::json!(true)));
            let r = worker.join().expect("worker");
            let err = r.error.expect("cancelled");
            assert_eq!(err.code, ErrorCode::E_CANCELLED, "{err:?}");
            assert_eq!(err.details["job_id"], serde_json::json!(job.id));
            assert!(ctx.jobs.list().is_empty(), "finished jobs leave the table");
            // Cancelled ⇒ nothing published: the starter output never produced a `height` layer.
            let r = call(
                &reg,
                &ctx,
                "terrain.stats",
                serde_json::json!({ "layer": "height" }),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID),
                "{r:?}"
            );
            assert_eq!(
                call(&reg, &ctx, "jobs.cancel", serde_json::json!({ "id": 999 })).result,
                Some(serde_json::json!(false))
            );
        }

        #[test]
        fn layer_computed_for_a_closed_project_is_refused() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let ctx = Context::default();
            let a = dir.path().join("a.jdproj");
            let b = dir.path().join("b.jdproj");
            call(
                &reg,
                &ctx,
                "project.create",
                serde_json::json!({ "path": a, "name": "A" }),
            );
            let hf = jd_terrain::Heightfield::new(8, 8, 8, 0).expect("hf");
            // The user switches project while a result for A is still being computed.
            call(
                &reg,
                &ctx,
                "project.create",
                serde_json::json!({ "path": b, "name": "B" }),
            );
            let err = ctx
                .set_layer_for(&a, "height", hf)
                .expect_err("stale project");
            assert_eq!(err.code, ErrorCode::E_INPUT_INVALID);
            assert!(
                !b.join("cache/layers/index.json").exists(),
                "nothing written into B's cache"
            );
        }

        #[test]
        fn invalid_document_stays_editable_and_recover_keeps_failed_entries() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join("b.jdproj");
            let ctx = Context::default();
            call(
                &reg,
                &ctx,
                "project.create",
                serde_json::json!({ "path": path, "name": "B" }),
            );
            ctx.set_project(None).expect("close");
            // Hand-edit graph.json: a node whose kind this build lacks (missing plugin) …
            let gpath = path.join("graph.json");
            let mut g: Value =
                serde_json::from_slice(&std::fs::read(&gpath).expect("read")).expect("json");
            g["nodes"]["01JDGRAPH0000000000BR0KEN0"] = serde_json::json!({ "kind": "jd.plugin.missing@1", "params": {}, "bypass": false, "ui": { "position": [0.0, 0.0] } });
            std::fs::write(&gpath, serde_json::to_vec_pretty(&g).expect("ser")).expect("write");
            // … and a journal entry that can never replay (its target node does not exist).
            let noise = jd_graph::nodes::STARTER_NOISE_ID;
            let bogus = serde_json::json!({ "ts": "2026-10-20T00:00:00Z", "op": "graph.connect", "params": { "from": noise, "from_port": "out", "to": "01JDGRAPH000000000000N0PE0", "to_port": "in" } });
            std::fs::create_dir_all(path.join("journal")).expect("dir");
            std::fs::write(path.join("journal/ops.ndjson"), format!("{bogus}\n")).expect("journal");

            let r = call(
                &reg,
                &ctx,
                "project.open",
                serde_json::json!({ "path": path }),
            );
            assert_eq!(
                r.result
                    .as_ref()
                    .and_then(|v| v["pending_journal"].as_u64()),
                Some(1),
                "{r:?}"
            );
            // Unrelated edits still work on the broken document …
            let r = call(
                &reg,
                &ctx,
                "graph.move_node",
                serde_json::json!({ "id": noise, "position": [5.0, 5.0] }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            // … new problems are still refused …
            let r = call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": noise, "from_port": "nope", "to": jd_graph::nodes::STARTER_OUTPUT_ID, "to_port": "in" }),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID)
            );
            // … and evaluation names the missing kind.
            let r = call(&reg, &ctx, "graph.evaluate", serde_json::json!({}));
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_NODE_UNKNOWN)
            );
            // Removing the broken node is allowed, and undo may bring it back (a prior state of
            // the document is never "introducing" an error) with the stacks staying consistent.
            let r = call(
                &reg,
                &ctx,
                "graph.remove_node",
                serde_json::json!({ "id": "01JDGRAPH0000000000BR0KEN0" }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            let r = call(&reg, &ctx, "graph.undo", serde_json::json!({}));
            let v = r.result.expect("undo of the removal");
            assert_eq!(v["nodes"].as_array().map(Vec::len), Some(3));
            assert_eq!(
                (v["undo_depth"].as_u64(), v["redo_depth"].as_u64()),
                (Some(1), Some(1))
            );
            // Recovery: the bogus entry fails, so nothing is saved and it stays pending.
            let r = call(&reg, &ctx, "project.recover", serde_json::json!({}));
            let res = r.result.as_ref().expect("result");
            // Only entries pending at open are replayed; the live move_node above was already applied.
            assert_eq!(res["replayed"].as_array().map(Vec::len), Some(0));
            assert_eq!(res["failed"].as_array().map(Vec::len), Some(1));
            assert_eq!(
                r.warnings.first().map(|w| w.code.as_str()),
                Some("W_RECOVER_INCOMPLETE")
            );
            let info = call(&reg, &ctx, "project.info", serde_json::json!({}));
            assert_eq!(
                info.result
                    .as_ref()
                    .and_then(|v| v["pending_journal"].as_u64()),
                Some(1)
            );
            assert!(
                std::fs::read_to_string(path.join("journal/ops.ndjson"))
                    .expect("journal")
                    .contains("N0PE0"),
                "journal file untouched"
            );
            // Explicitly discarding the failed entry saves and clears the journal.
            let r = call(
                &reg,
                &ctx,
                "project.recover",
                serde_json::json!({ "discard_failed": true }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            assert!(r.warnings.is_empty());
            let info = call(&reg, &ctx, "project.info", serde_json::json!({}));
            assert_eq!(
                info.result
                    .as_ref()
                    .and_then(|v| v["pending_journal"].as_u64()),
                Some(0)
            );
        }
    }
}

/// Terrain operations (Phase 2 slice 1): generate a noise heightfield for the open project's
/// world and export it. Layers live in the [`Context`] until the graph engine owns them.
pub mod terrain {
    use super::*;
    use jd_terrain::export::{ExportOptions, ExportReport};
    use jd_terrain::{HeightfieldStats, NoiseParams, generate_noise};

    fn default_layer() -> String {
        "height".into()
    }

    /// `terrain.generate` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainGenerate {
        /// Noise parameters (all optional; see schema defaults).
        #[serde(default)]
        pub noise: NoiseParams,
        /// Layer name to store the result under.
        #[serde(default = "default_layer")]
        pub layer: String,
    }

    /// `terrain.stats` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainLayer {
        /// Layer name.
        #[serde(default = "default_layer")]
        pub layer: String,
    }

    /// `terrain.export_heightmap` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainExport {
        /// Layer name.
        #[serde(default = "default_layer")]
        pub layer: String,
        /// Output file; relative paths resolve against the project's `exports/` directory.
        pub path: PathBuf,
        /// Encoding and conventions.
        #[serde(flatten)]
        pub options: ExportOptions,
    }

    /// `terrain.scatter` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainScatter {
        /// 0..1 density mask layer (a biome class mask, wetness, a painted mask…).
        pub mask: String,
        /// Height layer sampled for `z` (null = 0).
        #[serde(default = "default_layer_opt")]
        pub height: Option<String>,
        /// Output `.csv` or `.json`; relative paths resolve against the project's `exports/`.
        pub path: PathBuf,
        /// Spacing, jitter, seed.
        #[serde(flatten)]
        pub scatter: jd_terrain::ScatterParams,
    }

    fn default_layer_opt() -> Option<String> {
        Some(default_layer())
    }

    /// `terrain.scatter` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct ScatterReport {
        /// Written file.
        pub path: PathBuf,
        /// Points written.
        pub count: u64,
        /// Bytes written.
        pub bytes: u64,
        /// Candidates per axis the grid had.
        pub spacing_m: f32,
    }

    /// `terrain.export_mesh` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainExportMesh {
        /// Layer name.
        #[serde(default = "default_layer")]
        pub layer: String,
        /// Output `.glb`; relative paths resolve against the project's `exports/` directory.
        pub path: PathBuf,
        /// Grid step (1 = every sample). Default: the smallest step that keeps the mesh within `max_vertices`.
        #[serde(default)]
        pub step: Option<u32>,
        /// Vertex budget used when `step` is not given (default 1025² = 1 050 625).
        #[serde(default = "default_max_vertices")]
        pub max_vertices: u32,
        /// Embed a baked base-colour texture (hypsometric ramp + slope, one texel per vertex).
        #[serde(default = "yes")]
        pub texture: bool,
    }

    fn yes() -> bool {
        true
    }

    fn default_max_vertices() -> u32 {
        1025 * 1025 // a 1025² grid: 4096- and 4097-px worlds both export at step 4
    }

    /// Result of `terrain.generate` / `terrain.stats`.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct LayerInfo {
        /// Layer name.
        pub layer: String,
        /// Width, height in pixels.
        pub size_px: [u32; 2],
        /// Tile size and halo.
        pub tile_size: u32,
        /// Halo.
        pub halo: u32,
        /// Statistics.
        pub stats: HeightfieldStats,
        /// blake3 content hash (hex).
        pub content_hash: String,
        /// Wall time of the operation in milliseconds.
        pub duration_ms: u64,
    }

    fn info(layer: &str, hf: &jd_terrain::Heightfield, duration_ms: u64) -> LayerInfo {
        LayerInfo {
            layer: layer.to_owned(),
            size_px: [hf.width(), hf.height()],
            tile_size: hf.tile_size(),
            halo: hf.halo(),
            stats: hf.stats(),
            content_hash: hf.content_hash().to_hex().to_string(),
            duration_ms,
        }
    }

    /// `terrain.preview` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainPreview {
        /// Layer name.
        #[serde(default = "default_layer")]
        pub layer: String,
        /// Downsample factor exponent (0 = full, 1 = half, …).
        #[serde(default = "default_preview_lod")]
        pub lod: u8,
    }

    fn default_preview_lod() -> u8 {
        2
    }

    /// `terrain.preview` result: a downsampled u16 heightfield, base64-encoded.
    /// (The desktop app uses the zero-copy `layer_preview` command; this op serves MCP agents
    /// and browser-based tests that cannot receive raw bytes.)
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct PreviewResult {
        /// Width in samples.
        pub width: u32,
        /// Height in samples.
        pub height: u32,
        /// Height of sample value 0.
        pub min: f32,
        /// Height of sample value 65535.
        pub max: f32,
        /// The exact wire format of `jd_terrain::preview::Preview::to_bytes` (16-byte little-endian
        /// header `u32 width, u32 height, f32 min, f32 max` followed by row-major u16 samples),
        /// standard base64 with padding. One decoder serves both transports.
        pub bytes_base64: String,
    }

    /// Register terrain ops.
    pub fn register_all(r: &mut Registry) {
        r.register::<TerrainPreview, PreviewResult, _>(
            "terrain.preview",
            "Downsampled u16 preview of a layer (base64) for agents and viewers",
            Scope::ProjectRead,
            |ctx, p| {
                ctx.with_layer(&p.layer, |hf| {
                    use base64::Engine as _;
                    let pv = jd_terrain::preview::preview_u16(hf, p.lod);
                    Ok((
                        PreviewResult {
                            width: pv.width,
                            height: pv.height,
                            min: pv.min,
                            max: pv.max,
                            bytes_base64: base64::engine::general_purpose::STANDARD
                                .encode(pv.to_bytes()),
                        },
                        Vec::new(),
                    ))
                })
            },
        );
        r.register::<TerrainGenerate, LayerInfo, _>(
            "terrain.generate",
            "Generate a noise heightfield for the open project's world and store it as a layer",
            Scope::ProjectWrite,
            |ctx, p| {
                let (world, root) = ctx
                    .with_project(|proj| Ok((proj.doc.world.clone(), proj.root().to_path_buf())))?;
                let started = std::time::Instant::now();
                let hf = generate_noise(&world, &p.noise)?;
                let out = info(&p.layer, &hf, started.elapsed().as_millis() as u64);
                ctx.set_layer_for(&root, &p.layer, hf)?;
                Ok((out, Vec::new()))
            },
        );
        r.register::<TerrainLayer, LayerInfo, _>(
            "terrain.stats",
            "Statistics and content hash of a layer",
            Scope::ProjectRead,
            |ctx, p| ctx.with_layer(&p.layer, |hf| Ok((info(&p.layer, hf, 0), Vec::new()))),
        );
        r.register::<TerrainExportMesh, jd_terrain::MeshExportReport, _>(
            "terrain.export_mesh",
            "Write a layer as a binary glTF (.glb) grid mesh: metres, +Y up, centred on the world, normals + UVs + PBR material with a baked base-colour texture",
            Scope::Export,
            |ctx, p| {
                let (path, extent) = ctx.with_project(|proj| {
                    let path = if p.path.is_absolute() { p.path.clone() } else { proj.root().join("exports").join(&p.path) };
                    Ok((path, proj.doc.world.extent_m))
                })?;
                let mut warnings = Vec::new();
                let report = ctx.with_layer(&p.layer, |hf| {
                    let step = p.step.unwrap_or_else(|| jd_terrain::auto_step(hf.width(), hf.height(), p.max_vertices));
                    if p.step.is_none() && step > 1 {
                        warnings.push(Warning::new(
                            "W_MESH_DECIMATED",
                            format!("mesh sampled every {step} pixels to stay within {} vertices; pass `step: 1` for the full grid", p.max_vertices),
                        ));
                    }
                    jd_terrain::export_glb(hf, extent, &path, step, p.texture)
                })?;
                Ok((report, warnings))
            },
        );
        r.register::<TerrainScatter, ScatterReport, _>(
            "terrain.scatter",
            "Scatter instance points over the world from a 0..1 density mask layer (stratified jittered grid, deterministic by seed) and write them as CSV or JSON with heights",
            Scope::Export,
            |ctx, p| {
                let (path, extent) = ctx.with_project(|proj| {
                    let path = if p.path.is_absolute() { p.path.clone() } else { proj.root().join("exports").join(&p.path) };
                    Ok((path, proj.doc.world.extent_m))
                })?;
                // Two layers: clone the mask out first (the layer lock is not re-entrant).
                let mask = ctx.with_layer(&p.mask, |hf| Ok(hf.clone()))?;
                let points = match &p.height {
                    Some(h) => ctx.with_layer(h, |hf| jd_terrain::scatter_points(&mask, Some(hf), extent, &p.scatter))?,
                    None => jd_terrain::scatter_points(&mask, None, extent, &p.scatter)?,
                };
                let bytes = jd_terrain::write_points(&path, &points)?;
                Ok((ScatterReport { path, count: points.len() as u64, bytes, spacing_m: p.scatter.spacing_m }, Vec::new()))
            },
        );
        r.register::<TerrainExport, ExportReport, _>(
            "terrain.export_heightmap",
            "Write a layer as EXR (f32), PNG16, RAW r16 or r32 with a JSON sidecar",
            Scope::Export,
            |ctx, p| {
                let path = if p.path.is_absolute() {
                    p.path.clone()
                } else {
                    ctx.with_project(|proj| Ok(proj.root().join("exports").join(&p.path)))?
                };
                let report = ctx.with_layer(&p.layer, |hf| jd_terrain::export_heightmap(hf, &path, &p.options))?;
                let mut warnings = Vec::new();
                if report.quantisation_step_m > 0.0 {
                    warnings.push(
                        Warning::new(
                            "W_EXPORT_LOSS",
                            format!("{:?} quantises heights to {:.4} m steps; range recorded in the sidecar", report.format, report.quantisation_step_m),
                        ),
                    );
                }
                Ok((report, warnings))
            },
        );
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn preview_op_returns_the_wire_format() {
            use base64::Engine as _;
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [64, 48],
                extent_m: [64.0, 48.0],
                tile_size: 64,
                halo: 0,
                ..Default::default()
            };
            reg.invoke(&ctx, Request::new("project.create", serde_json::json!({ "path": dir.path().join("p.jdproj"), "name": "P", "world": world })));
            reg.invoke(
                &ctx,
                Request::new("terrain.generate", serde_json::json!({})),
            );
            let r = reg.invoke(
                &ctx,
                Request::new("terrain.preview", serde_json::json!({ "lod": 1 })),
            );
            let res = r.result.expect("preview");
            assert_eq!(
                (&res["width"], &res["height"]),
                (&serde_json::json!(32), &serde_json::json!(24))
            );
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(res["bytes_base64"].as_str().expect("b64"))
                .expect("decodes");
            let pv = jd_terrain::preview::Preview::from_bytes(&bytes).expect("wire format");
            assert_eq!((pv.width, pv.height), (32, 24));
            assert_eq!(pv.data.len(), 32 * 24);
        }

        #[test]
        fn generate_stats_export_through_the_registry() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let ctx = Context::default();
            let r = reg.invoke(
                &ctx,
                Request::new("terrain.generate", serde_json::json!({})),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID),
                "needs a project"
            );
            let world = WorldSettings {
                resolution_px: [128, 96],
                extent_m: [1024.0, 768.0],
                tile_size: 64,
                halo: 2,
                ..Default::default()
            };
            let r = reg.invoke(&ctx, Request::new("project.create", serde_json::json!({ "path": dir.path().join("t.jdproj"), "name": "T", "world": world })));
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            let r = reg.invoke(
                &ctx,
                Request::new(
                    "terrain.generate",
                    serde_json::json!({ "noise": { "seed": 9, "height_range_m": [0.0, 100.0] } }),
                ),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            let res = r.result.expect("result");
            assert_eq!(res["size_px"], serde_json::json!([128, 96]));
            assert_eq!(res["stats"]["non_finite"], 0);
            let hash = res["content_hash"].as_str().expect("hash").to_owned();
            let r = reg.invoke(&ctx, Request::new("terrain.stats", serde_json::json!({})));
            assert_eq!(
                r.result.as_ref().and_then(|v| v["content_hash"].as_str()),
                Some(hash.as_str())
            );
            let r = reg.invoke(
                &ctx,
                Request::new(
                    "terrain.export_heightmap",
                    serde_json::json!({ "path": "h.png", "format": "png16" }),
                ),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            assert!(r.warnings.iter().any(|w| w.code == "W_EXPORT_LOSS"));
            assert!(dir.path().join("t.jdproj/exports/h.png").exists());
            assert!(
                dir.path()
                    .join("t.jdproj/exports/h.png.jd-height.json")
                    .exists()
            );
            let r = reg.invoke(
                &ctx,
                Request::new(
                    "terrain.export_heightmap",
                    serde_json::json!({ "path": dir.path().join("abs.exr"), "format": "exr" }),
                ),
            );
            assert!(
                r.warnings.is_empty(),
                "exact format has no loss warning: {r:?}"
            );
            assert!(dir.path().join("abs.exr").exists());
            // Mesh export: GLB header, auto decimation warning for a large world.
            let r = reg.invoke(
                &ctx,
                Request::new(
                    "terrain.export_mesh",
                    serde_json::json!({ "path": "t.glb" }),
                ),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            let glb = std::fs::read(dir.path().join("t.jdproj/exports/t.glb")).expect("glb");
            assert_eq!(&glb[..4], b"glTF");
            assert_eq!(
                r.result.as_ref().and_then(|v| v["format"].as_str()),
                Some("glb")
            );
            let r = reg.invoke(
                &ctx,
                Request::new(
                    "terrain.export_heightmap",
                    serde_json::json!({ "layer": "nope", "path": "x.exr", "format": "exr" }),
                ),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoke(reg: &Registry, ctx: &Context, op: &str, params: Value) -> Response {
        reg.invoke(ctx, Request::new(op, params))
    }

    #[test]
    fn every_builtin_has_schemas_and_description() {
        let reg = Registry::with_builtins();
        let list = reg.list();
        assert!(list.len() >= 8);
        for op in &list {
            assert!(!op.description.is_empty(), "{}", op.id);
            assert!(op.request_schema.is_object(), "{}", op.id);
            assert!(op.result_schema.is_object(), "{}", op.id);
            assert!(op.id.contains('.'), "{}", op.id);
        }
    }

    #[test]
    fn unknown_op_and_bad_params_are_stable_errors() {
        let reg = Registry::with_builtins();
        let ctx = Context::default();
        let r = invoke(&reg, &ctx, "nope.op", Value::Null);
        assert_eq!(
            r.error.as_ref().map(|e| e.code),
            Some(ErrorCode::E_INPUT_INVALID)
        );
        let r = invoke(
            &reg,
            &ctx,
            "project.create",
            serde_json::json!({ "bogus": 1 }),
        );
        assert_eq!(
            r.error.as_ref().map(|e| e.code),
            Some(ErrorCode::E_INPUT_INVALID)
        );
        let r = invoke(&reg, &ctx, "project.info", serde_json::json!({}));
        assert_eq!(
            r.error.as_ref().map(|e| e.code),
            Some(ErrorCode::E_INPUT_INVALID)
        );
        let mut req = Request::new("diag.env", serde_json::json!({}));
        req.schema_version = SchemaVersion(2);
        let r = reg.invoke(&ctx, req);
        assert_eq!(
            r.error.as_ref().map(|e| e.code),
            Some(ErrorCode::E_SCHEMA_VERSION)
        );
    }

    #[test]
    fn create_rename_crash_recover_flow() {
        let reg = Registry::with_builtins();
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("w.jdproj");
        let ctx = Context::default();
        let r = invoke(
            &reg,
            &ctx,
            "project.create",
            serde_json::json!({ "path": path, "name": "W" }),
        );
        assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
        let r = invoke(
            &reg,
            &ctx,
            "project.set_name",
            serde_json::json!({ "name": "Renamed" }),
        );
        assert_eq!(
            r.result.as_ref().and_then(|v| v["name"].as_str()),
            Some("Renamed")
        );
        // "Crash": drop the context without saving.
        ctx.set_project(None).expect("close");
        let ctx2 = Context::default();
        let r = invoke(
            &reg,
            &ctx2,
            "project.open",
            serde_json::json!({ "path": path }),
        );
        assert!(
            r.warnings.iter().any(|w| w.code == "W_RECOVERY_AVAILABLE"),
            "{r:?}"
        );
        assert_eq!(
            r.result.as_ref().and_then(|v| v["name"].as_str()),
            Some("W")
        );
        let r = invoke(&reg, &ctx2, "project.recover", serde_json::json!({}));
        assert_eq!(
            r.result
                .as_ref()
                .and_then(|v| v["replayed"].as_array())
                .map(Vec::len),
            Some(1)
        );
        let r = invoke(&reg, &ctx2, "project.info", serde_json::json!({}));
        assert_eq!(
            r.result.as_ref().and_then(|v| v["name"].as_str()),
            Some("Renamed")
        );
        assert_eq!(
            r.result.as_ref().and_then(|v| v["dirty"].as_bool()),
            Some(false)
        );
    }

    #[test]
    fn schemas_snapshot() {
        let reg = Registry::with_builtins();
        let list = reg.list();
        insta::assert_json_snapshot!("builtin-op-schemas", list);
    }

    #[test]
    fn request_params_validate_against_published_schema() {
        let reg = Registry::with_builtins();
        let op = reg.get("project.create").expect("op");
        let validator = jsonschema::validator_for(&op.request_schema).expect("schema compiles");
        assert!(validator.is_valid(&serde_json::json!({ "path": "/tmp/x.jdproj", "name": "X" })));
        assert!(!validator.is_valid(&serde_json::json!({ "name": "X" })));
        assert!(
            !validator.is_valid(&serde_json::json!({ "path": "/tmp/x", "name": "X", "bogus": 1 }))
        );
    }
}
