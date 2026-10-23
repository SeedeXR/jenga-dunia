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
    layers: Arc<Mutex<BTreeMap<String, Arc<jd_terrain::Heightfield>>>>,
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
        self.set_layer_arc(root, name, Arc::new(hf))
    }

    /// [`Self::set_layer_for`] for a field another holder (the evaluator's outputs) already
    /// shares: no whole-field clone — a 16k² layer is 1 GiB.
    pub fn set_layer_arc(
        &self,
        root: &Path,
        name: &str,
        hf: Arc<jd_terrain::Heightfield>,
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

    /// A published layer as a shared field (loaded from the cache under `root` on first use).
    /// Takes only the layer lock, so it may be called while the project lock is held.
    pub fn layer_arc(
        &self,
        root: &Path,
        name: &str,
    ) -> JdResult<Option<Arc<jd_terrain::Heightfield>>> {
        let mut g = self
            .layers
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "layer lock poisoned"))?;
        if !g.contains_key(name)
            && let Some(hf) = jd_cache::LayerCache::open(root).get(name)?
        {
            g.insert(name.to_owned(), Arc::new(hf));
        }
        Ok(g.get(name).cloned())
    }

    /// The layer loader handed to the graph evaluator for `jd.input.layer@1`.
    pub fn layer_loader(&self, root: &Path) -> jd_graph::registry::LayerLoader {
        let (ctx, root) = (self.clone(), root.to_path_buf());
        Arc::new(move |name: &str| ctx.layer_arc(&root, name))
    }

    /// The u16 preview of a layer: from memory when it is loaded, otherwise streamed straight
    /// from its cache blob (the same bytes; a 16k² layer is never loaded just to be previewed).
    pub fn preview_layer(&self, name: &str, lod: u8) -> JdResult<jd_terrain::preview::Preview> {
        let root = self.project_root();
        let loaded = self
            .layers
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "layer lock poisoned"))?
            .get(name)
            .cloned();
        if let Some(hf) = loaded {
            return Ok(jd_terrain::preview::preview_u16(&hf, lod));
        }
        if let Some(root) = root
            && let Some(pv) = jd_cache::LayerCache::open(&root).preview(name, lod)?
        {
            return Ok(pv);
        }
        self.with_layer(name, |hf| Ok(jd_terrain::preview::preview_u16(hf, lod)))
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
            g.insert(name.to_owned(), Arc::new(hf));
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

    /// `diag.bundle` params.
    #[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct DiagBundle {
        /// Output directory; relative paths resolve against the project's `exports/`. Default `exports/diagnostics-<UTC stamp>`.
        #[serde(default)]
        pub dir: Option<PathBuf>,
        /// How many journal entries to include from the end.
        #[serde(default = "journal_lines")]
        pub journal_lines: u32,
    }

    fn journal_lines() -> u32 {
        200
    }

    /// `diag.bundle` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct DiagBundleReport {
        /// Where the bundle went.
        pub dir: PathBuf,
        /// Files written.
        pub files: Vec<String>,
        /// Total bytes.
        pub bytes: u64,
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
        /// Shared wind of the world: every wind-driven node without its own `wind_*` setting follows it.
        #[serde(default)]
        pub wind: Option<jd_types::Wind>,
        /// World-space origin of pixel (0, 0) in metres, f64 — layers stay in local metres, so
        /// moving the origin re-bases the world without touching a sample (GeoTIFF tie points,
        /// the Unreal profile and georeferenced exports follow it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub origin: Option<[f64; 2]>,
        /// Coordinate reference system of the origin, e.g. `EPSG:32637`; an empty string clears it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub crs: Option<String>,
        /// Memory budget for evaluations, bytes (≥ 256 MiB; default 4 GiB).
        #[serde(default)]
        pub memory_budget_bytes: Option<u64>,
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
        if let Some(v) = args.memory_budget_bytes {
            s.memory_budget_bytes = v;
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
            "project.add_attribution" => {
                let a: crate::project::Attribution = serde_json::from_value(params.clone())?;
                if a.source.trim().is_empty() || a.text.trim().is_empty() {
                    return Err(JdError::invalid(
                        "params",
                        "`source` and `text` are required",
                    ));
                }
                Ok(())
            }
            "project.set_settings" => {
                let args: ProjectSetSettings = serde_json::from_value(params.clone())?;
                if let Some(w) = args.wind {
                    let world = WorldSettings {
                        wind: w,
                        ..p.doc.world.clone()
                    };
                    world
                        .validate()
                        .map_err(|e| JdError::invalid("params.wind", e))?;
                }
                if let Some(o) = args.origin
                    && !o.iter().all(|v| v.is_finite())
                {
                    return Err(JdError::invalid("params.origin", "must be finite metres"));
                }
                if let Some(c) = &args.crs
                    && !c.is_empty()
                {
                    if c.trim() != c || c.len() > 64 {
                        return Err(JdError::invalid(
                            "params.crs",
                            "an identifier such as `EPSG:32637` (≤ 64 chars, no surrounding spaces), or empty to clear",
                        ));
                    }
                    // The build must be able to project it, or imports and exports fail later.
                    jd_geo::proj_string(c)
                        .map_err(|e| JdError::invalid("params.crs", e.message))?;
                }
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
            "project.add_attribution" => {
                let a: crate::project::Attribution = serde_json::from_value(params.clone())?;
                if !p
                    .doc
                    .attribution
                    .iter()
                    .any(|x| x.source == a.source && x.text == a.text)
                {
                    p.doc.attribution.push(a);
                }
                Ok(())
            }
            "project.set_settings" => {
                let args: ProjectSetSettings = serde_json::from_value(params.clone())?;
                let merged = merged_settings(p, &args);
                p.set_settings(merged)?;
                if let Some(w) = args.wind {
                    p.doc.world.wind = w;
                }
                if let Some(o) = args.origin {
                    p.doc.world.origin = o;
                }
                if let Some(c) = args.crs {
                    p.doc.world.crs = (!c.is_empty()).then_some(c);
                }
                Ok(())
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
        p.revision += 1;
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
            "Change project settings (autosave interval, cache limit, preview LOD, memory budget, the world's shared wind, the world origin and CRS); fields left out keep their value — journaled like every mutation",
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
        r.register::<DiagBundle, DiagBundleReport, _>(
            "diag.bundle",
            "Write a diagnostic bundle for a bug report: host environment, project.json / graph.json / scene.json, the tail of the journal, versions — no heightfield data",
            Scope::Export,
            |ctx, p| {
                let (root, name) = ctx.with_project(|pr| Ok((pr.root().to_path_buf(), pr.doc.name.clone())))?;
                let stamp = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ").to_string();
                let dir = match &p.dir {
                    Some(d) if d.is_absolute() => d.clone(),
                    Some(d) => root.join("exports").join(d),
                    None => root.join("exports").join(format!("diagnostics-{stamp}")),
                };
                std::fs::create_dir_all(&dir)?;
                let mut files = Vec::new();
                let mut put = |name: &str, bytes: &[u8]| -> JdResult<()> {
                    std::fs::write(dir.join(name), bytes)?;
                    files.push(name.to_owned());
                    Ok(())
                };
                put("env.json", &serde_json::to_vec_pretty(&crate::diag::env_info())?)?;
                put(
                    "versions.json",
                    &serde_json::to_vec_pretty(&serde_json::json!({
                        "jd": env!("CARGO_PKG_VERSION"), "schema_version": SchemaVersion::CURRENT, "project": name,
                        "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "written": stamp,
                    }))?,
                )?;
                for f in ["project.json", "graph.json", "scene.json"] {
                    if let Ok(bytes) = std::fs::read(root.join(f)) {
                        put(f, &bytes)?;
                    }
                }
                if let Ok(journal) = std::fs::read_to_string(root.join("journal").join("ops.ndjson")) {
                    let lines: Vec<&str> = journal.lines().collect();
                    let tail = &lines[lines.len().saturating_sub(p.journal_lines as usize)..];
                    put("journal-tail.ndjson", (tail.join("\n") + "\n").as_bytes())?;
                }
                let bytes = files.iter().map(|f| std::fs::metadata(dir.join(f)).map(|m| m.len()).unwrap_or(0)).sum();
                Ok((DiagBundleReport { dir, files, bytes }, Vec::new()))
            },
        );
        r.register::<crate::diag::BenchParams, crate::diag::BenchReport, _>(
            "diag.bench",
            "Run a benchmark scene (S1: fBm + ridged + add + thermal×50) with the real kernels; per-stage wall time, peak RSS, content hash",
            Scope::None,
            |_, p| Ok((crate::diag::bench(&p)?, Vec::new())),
        );
        terrain::register_all(r);
        graph::register_all(r);
        crate::geo::register(r);
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
        /// Evaluate only the output nodes publishing these layers (and what feeds them); the
        /// gate pattern runs the coarse chain at a high lod and the fine chain at lod 0.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub outputs: Option<Vec<String>>,
        /// Detached jobs only: give up after this many seconds (`E_TIMEOUT` in the result).
        #[serde(default)]
        pub timeout_s: Option<f32>,
        /// Detached jobs only: queue priority `low` | `normal` (default) | `high`; a higher
        /// priority starts before older queued jobs, never pre-empts a running one.
        #[serde(default)]
        pub priority: Option<String>,
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
        /// Timeline position 0..1 at which keyed parameters resolve (`graph.set_time`).
        #[serde(default)]
        pub time: f32,
    }

    /// `graph.set_time` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct SetTime {
        /// Timeline position, 0..1.
        pub time: f32,
    }

    /// `timeline.export` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TimelineExport {
        /// The output node's layer to write, one file per frame.
        #[serde(default = "height_layer")]
        pub layer: String,
        /// Number of frames (2..=10000), spread evenly from `from` to `to` inclusive.
        pub frames: u32,
        /// First frame's timeline position (0..1).
        #[serde(default)]
        pub from: f32,
        /// Last frame's timeline position (0..1).
        #[serde(default = "one")]
        pub to: f32,
        /// Output directory; relative paths resolve against the project's `exports/`. Files are `<layer>_<frame>.<ext>`.
        pub dir: PathBuf,
        /// Level of detail (0 = full resolution).
        #[serde(default)]
        pub lod: u8,
        /// Encoding and conventions; without `range_m` the sequence shares the min/max over all frames so quantised frames do not flicker.
        #[serde(flatten)]
        pub options: jd_terrain::ExportOptions,
    }

    fn height_layer() -> String {
        "height".to_owned()
    }
    fn one() -> f32 {
        1.0
    }

    /// One exported frame.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct TimelineFrame {
        /// Timeline position.
        pub time: f32,
        /// File written.
        pub file: String,
        /// Height statistics of the frame, metres.
        pub min: f32,
        /// Maximum.
        pub max: f32,
        /// Mean.
        pub mean: f32,
    }

    /// `timeline.export` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct TimelineExportReport {
        /// Directory the frames went to.
        pub dir: PathBuf,
        /// The frames in order.
        pub frames: Vec<TimelineFrame>,
        /// Height range the quantised frames share, metres.
        pub range_m: [f32; 2],
        /// Wall time, milliseconds.
        pub duration_ms: u64,
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
            time: p.graph.time,
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
            "graph.set_time" => {
                let s: SetTime = serde_json::from_value(params.clone())?;
                if !(0.0..=1.0).contains(&s.time) {
                    return Err(JdError::invalid("time", "must be within 0..=1"));
                }
                g.time = s.time;
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
        only: Option<Vec<String>>,
        job_id: u64,
        ctl: &jd_graph::JobCtl,
    ) -> JdResult<(EvaluateResult, Vec<Warning>)> {
        let (graph, world, root, budget, revision) = ctx.with_project(|p| {
            Ok((
                p.graph.clone(),
                p.doc.world.clone(),
                p.root().to_path_buf(),
                p.doc.settings.memory_budget_bytes,
                p.revision,
            ))
        })?;
        // Pre-flight: refuse evaluations the memory budget cannot hold instead of thrashing.
        let estimate = Evaluator::new(registry(), cache())
            .only_outputs(only.clone())
            .estimate_bytes(&graph, &world, lod)?;
        if estimate > budget {
            return Err(JdError::new(
                ErrorCode::E_RESOURCE_MEMORY,
                format!("evaluation needs an estimated {} MiB, over the project's {} MiB budget", estimate >> 20, budget >> 20),
            )
            .why("Every node's outputs stay alive until consumed, plus the heaviest kernel's scratch.")
            .next("Evaluate at a higher lod, reduce the world resolution, split the graph, or raise settings.memory_budget_bytes.")
            .detail("estimated_bytes", estimate)
            .detail("budget_bytes", budget));
        }
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
            .with_layers(ctx.layer_loader(&root))
            .only_outputs(only.clone())
            .evaluate_job(&graph, &world, lod, Some(&root.join("assets")), ctl)
            .map_err(|err| {
                if err.code == ErrorCode::E_CANCELLED {
                    err.detail("job_id", job_id)
                } else {
                    err
                }
            })?;
        let mut layers = BTreeMap::new();
        // Two evaluations of one project may overlap (an agent and the desktop's live preview, or
        // Generate right behind the starter preview); each snapshots the graph at its start, so
        // the one that started on an older graph must not publish over the newer result.
        let stale = ctx
            .with_project(|p| Ok(p.revision != revision))
            .unwrap_or(true);
        if stale {
            return Ok((
                EvaluateResult { report, layers },
                vec![Warning::new(
                    "W_STALE_GRAPH",
                    "the graph changed while this evaluation ran: its layers were not published — evaluate again",
                )],
            ));
        }
        for (id, node) in &graph.nodes {
            if node.kind.0.starts_with("jd.output.height@")
                && let Some(v) = outputs.get(&(id.clone(), "out".to_owned()))
            {
                let mut name = jd_graph::nodes::output_layer_name(&node.params);
                if lod > 0 {
                    // Never overwrite the full-resolution layer with a preview.
                    name = format!("{name}#lod{}", lod);
                }
                layers.insert(name.clone(), v.content_hash().to_hex().to_string());
                ctx.set_layer_arc(&root, &name, Arc::clone(v.as_field()))?;
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
        r.register::<SetTime, GraphView, _>(
            "graph.set_time",
            "Move the timeline (0..1, journaled): parameters written as `{\"keys\": [[t, v], …]}` interpolate here — numbers linearly, anything else stepwise — so erosion steps, sea level or wind can be scrubbed and keyframed",
            Scope::ProjectWrite,
            |ctx, s| {
                let params = serde_json::json!({ "time": s.time });
                ctx.with_project(|p| {
                    builtin::mutate(p, "graph.set_time", params)?;
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
                evaluate(ctx, e.lod, e.outputs.clone(), job.id, &job.ctl)
            },
        );
        r.register_detached::<Evaluate, _>(
            "graph.evaluate_async",
            "Start graph.evaluate on a worker thread and return at once with `status: accepted` + `job_id`; follow it with jobs.list, cancel with jobs.cancel, collect the envelope with jobs.result",
            Scope::ProjectWrite,
            |ctx, e| {
                ctx.with_project(|_| Ok(()))?; // fail now, not on the thread, when nothing is open
                if let Some(t) = e.timeout_s
                    && !(t > 0.0 && t.is_finite())
                {
                    return Err(JdError::invalid("timeout_s", "must be a positive number of seconds"));
                }
                let priority = match e.priority.as_deref() {
                    None => 1,
                    Some(p) => crate::jobs::PRIORITIES
                        .iter()
                        .position(|k| *k == p)
                        .ok_or_else(|| JdError::invalid("priority", "must be one of low, normal, high"))?
                        as u8,
                };
                let guard = ctx.jobs.start_owned("graph.evaluate", priority);
                let id = guard.id;
                let ticket = ctx.jobs.ticket(priority); // queue position, taken before the thread exists
                let ctx2 = ctx.clone();
                std::thread::Builder::new()
                    .name(format!("jd-job-{id}"))
                    .spawn(move || {
                        // Queue: wait for a worker slot (half the cores by default); a cancel while
                        // queued ends the job without running it.
                        if !ctx2.jobs.acquire_slot(id, ticket, &guard.ctl.cancel) {
                            let err = JdError::new(ErrorCode::E_CANCELLED, "cancelled while queued").detail("job_id", id);
                            ctx2.jobs.finish(id, serde_json::json!({ "status": "error", "error": err }));
                            drop(guard);
                            return;
                        }
                        // Timeout: a watchdog raises the cancel flag at the deadline; the evaluator
                        // then stops at its next check and the envelope says E_TIMEOUT.
                        let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        if let Some(t) = e.timeout_s {
                            let (cancel, timed_out, done) = (guard.ctl.cancel.clone(), timed_out.clone(), done.clone());
                            let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f32(t);
                            std::thread::spawn(move || {
                                while std::time::Instant::now() < deadline {
                                    if done.load(std::sync::atomic::Ordering::Relaxed) {
                                        return;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                }
                                if !done.load(std::sync::atomic::Ordering::Relaxed) {
                                    timed_out.store(true, std::sync::atomic::Ordering::Relaxed);
                                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                }
                            });
                        }
                        let outcome = evaluate(&ctx2, e.lod, e.outputs.clone(), id, &guard.ctl);
                        done.store(true, std::sync::atomic::Ordering::Relaxed);
                        ctx2.jobs.release_slot();
                        let envelope = match outcome {
                            Ok((r, w)) => serde_json::json!({ "status": "ok", "result": r, "warnings": w }),
                            Err(err) if err.code == ErrorCode::E_CANCELLED && timed_out.load(std::sync::atomic::Ordering::Relaxed) => {
                                let t = JdError::new(ErrorCode::E_TIMEOUT, format!("evaluation exceeded timeout_s = {}", e.timeout_s.unwrap_or(0.0)))
                                    .next("Raise timeout_s, lower the lod, or reduce iterations.")
                                    .detail("job_id", id);
                                serde_json::json!({ "status": "error", "error": t })
                            }
                            Err(err) => serde_json::json!({ "status": "error", "error": err }),
                        };
                        ctx2.jobs.finish(id, envelope);
                        drop(guard);
                    })
                    .map_err(|e| JdError::new(ErrorCode::E_INTERNAL, format!("cannot spawn job thread: {e}")))?;
                Ok(id)
            },
        );
        r.register::<TimelineExport, TimelineExportReport, _>(
            "timeline.export",
            "Evaluate the graph at evenly spaced timeline positions and write one heightmap per frame (keyframed parameters animate; the project's own time is untouched)",
            Scope::Export,
            |ctx, p| {
                if !(2..=10_000).contains(&p.frames) {
                    return Err(JdError::invalid("frames", "must be within 2..=10000"));
                }
                for (name, t) in [("from", p.from), ("to", p.to)] {
                    if !(0.0..=1.0).contains(&t) {
                        return Err(JdError::invalid(name, "must be within 0..=1"));
                    }
                }
                let started = std::time::Instant::now();
                let (mut graph, world, root, budget) = ctx.with_project(|pr| Ok((pr.graph.clone(), pr.doc.world.clone(), pr.root().to_path_buf(), pr.doc.settings.memory_budget_bytes)))?;
                let out = graph
                    .nodes
                    .iter()
                    .find(|(_, n)| n.kind.0.starts_with("jd.output.height@") && jd_graph::nodes::output_layer_name(&n.params) == p.layer)
                    .map(|(id, _)| id.clone())
                    .ok_or_else(|| JdError::invalid("layer", format!("no jd.output.height node publishes `{}`", p.layer)))?;
                let dir = if p.dir.is_absolute() { p.dir.clone() } else { root.join("exports").join(&p.dir) };
                std::fs::create_dir_all(&dir)?;
                let job = ctx.jobs.start("timeline.export");
                // Every frame is kept until the shared range is known: frames × field must fit the budget.
                let mut fields: Vec<(f32, std::sync::Arc<jd_terrain::Heightfield>)> = Vec::with_capacity(p.frames as usize);
                for k in 0..p.frames {
                    graph.time = p.from + (p.to - p.from) * k as f32 / (p.frames - 1) as f32;
                    let (outputs, _) = Evaluator::new(registry(), cache()).evaluate_job(&graph, &world, p.lod, Some(&root.join("assets")), &job.ctl)?;
                    let hf = outputs.get(&(out.clone(), "out".to_owned())).map(|v| v.as_field().clone()).ok_or_else(|| JdError::new(ErrorCode::E_INTERNAL, "output node produced nothing"))?;
                    if (hf.width() as u64 * hf.height() as u64 * 4).saturating_mul(p.frames as u64) > budget {
                        return Err(JdError::new(ErrorCode::E_RESOURCE_MEMORY, "the frame sequence does not fit the project's memory budget")
                            .next("Export fewer frames per call, a higher lod, or raise settings.memory_budget_bytes."));
                    }
                    fields.push((graph.time, hf));
                }
                let stats = |hf: &jd_terrain::Heightfield| {
                    let v = hf.to_vec();
                    let (mut lo, mut hi, mut sum) = (f32::MAX, f32::MIN, 0.0f64);
                    for x in &v {
                        lo = lo.min(*x);
                        hi = hi.max(*x);
                        sum += f64::from(*x);
                    }
                    (lo, hi, (sum / v.len().max(1) as f64) as f32)
                };
                let per: Vec<(f32, f32, f32)> = fields.iter().map(|(_, hf)| stats(hf)).collect();
                let range_m = p.options.range_m.unwrap_or_else(|| {
                    let lo = per.iter().map(|s| s.0).fold(f32::MAX, f32::min);
                    let hi = per.iter().map(|s| s.1).fold(f32::MIN, f32::max);
                    [lo, if hi > lo { hi } else { lo + 1.0 }]
                });
                let opts = jd_terrain::ExportOptions { range_m: Some(range_m), ..p.options.clone() };
                let geo = jd_terrain::GeoRef { origin: world.origin, spacing: world.spacing_m(), crs: world.crs.clone() };
                let ext = match p.options.format {
                    jd_terrain::HeightmapFormat::Exr => "exr",
                    jd_terrain::HeightmapFormat::Png16 => "png",
                    jd_terrain::HeightmapFormat::RawR16 => "r16",
                    jd_terrain::HeightmapFormat::RawR32 => "r32",
                    jd_terrain::HeightmapFormat::Tiff32 => "tif",
                };
                let width = (p.frames - 1).to_string().len().max(4);
                let mut frames = Vec::with_capacity(fields.len());
                for (k, ((time, hf), (min, max, mean))) in fields.iter().zip(&per).enumerate() {
                    let file = format!("{}_{k:0width$}.{ext}", p.layer);
                    jd_terrain::export_heightmap_geo(hf, &dir.join(&file), &opts, Some(&geo))?;
                    frames.push(TimelineFrame { time: *time, file, min: *min, max: *max, mean: *mean });
                }
                Ok((TimelineExportReport { dir, frames, range_m, duration_ms: started.elapsed().as_millis() as u64 }, Vec::new()))
            },
        );
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn diag_bundle_collects_documents_env_and_journal_tail() {
            use crate::envelope::Status;
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [32, 32],
                extent_m: [32.0, 32.0],
                tile_size: 64,
                halo: 0,
                ..Default::default()
            };
            reg.invoke(&ctx, Request::new("project.create", serde_json::json!({ "path": dir.path().join("d.jdproj"), "name": "D", "world": world })));
            for _ in 0..3 {
                call(
                    &reg,
                    &ctx,
                    "project.set_settings",
                    serde_json::json!({ "preview_lod": 1 }),
                );
            }
            let r = call(
                &reg,
                &ctx,
                "diag.bundle",
                serde_json::json!({ "dir": "diag", "journal_lines": 2 }),
            );
            assert_eq!(r.status, Status::Ok, "{:?}", r.error);
            let rep: super::super::builtin::DiagBundleReport =
                serde_json::from_value(r.result.expect("report")).expect("shape");
            assert_eq!(
                rep.files,
                [
                    "env.json",
                    "versions.json",
                    "project.json",
                    "graph.json",
                    "scene.json",
                    "journal-tail.ndjson"
                ]
            );
            assert!(rep.files.iter().all(|f| rep.dir.join(f).is_file()) && rep.bytes > 0);
            assert_eq!(
                std::fs::read_to_string(rep.dir.join("journal-tail.ndjson"))
                    .expect("tail")
                    .lines()
                    .count(),
                2
            );
            assert!(
                rep.dir
                    .starts_with(dir.path().join("d.jdproj").join("exports"))
            );
        }

        #[test]
        fn the_world_wind_drives_wind_nodes_unless_they_set_their_own() {
            use crate::envelope::Status;
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [64, 64],
                extent_m: [640.0, 640.0],
                tile_size: 64,
                halo: 0,
                ..Default::default()
            };
            reg.invoke(&ctx, Request::new("project.create", serde_json::json!({ "path": dir.path().join("w.jdproj"), "name": "W", "world": world })));
            let nodes = call(&reg, &ctx, "graph.list", serde_json::json!({}))
                .result
                .expect("list")["nodes"]
                .clone();
            let id_of = |kind: &str| {
                nodes
                    .as_array()
                    .expect("nodes")
                    .iter()
                    .find(|n| n["kind"] == kind)
                    .expect(kind)["id"]
                    .clone()
            };
            let (noise, height) = (id_of("jd.noise.fbm@1"), id_of("jd.output.height@1"));
            let dunes = call(&reg, &ctx, "graph.add_node", serde_json::json!({ "kind": "jd.erosion.aeolian@1", "params": { "iterations": 20 } })).result.expect("add")["id"].clone();
            call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": noise, "from_port": "out", "to": dunes, "to_port": "in" }),
            );
            call(
                &reg,
                &ctx,
                "graph.connect",
                serde_json::json!({ "from": dunes, "from_port": "out", "to": height, "to_port": "in" }),
            );
            let hash = |reg: &Registry, ctx: &Context| {
                call(reg, ctx, "graph.evaluate", serde_json::json!({}))
                    .result
                    .expect("eval")["layers"]["height"]
                    .clone()
            };
            let westerly = hash(&reg, &ctx);
            let r = call(
                &reg,
                &ctx,
                "project.set_settings",
                serde_json::json!({ "wind": { "azimuth_deg": 90.0, "speed_mps": 8.0 } }),
            );
            assert_eq!(r.status, Status::Ok, "{:?}", r.error);
            assert_eq!(
                r.result.expect("summary")["world"]["wind"]["azimuth_deg"],
                90.0
            );
            let easterly = hash(&reg, &ctx);
            assert_ne!(
                westerly, easterly,
                "the shared wind turned: the dunes differ"
            );
            // The node's own setting wins over the world's.
            call(
                &reg,
                &ctx,
                "graph.set_param",
                serde_json::json!({ "id": dunes, "key": "wind_azimuth_deg", "value": 270.0 }),
            );
            assert_eq!(hash(&reg, &ctx), westerly);
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.set_settings",
                    serde_json::json!({ "wind": { "azimuth_deg": 90.0, "speed_mps": -1.0 } })
                )
                .status,
                Status::Error
            );
        }

        #[test]
        fn origin_and_crs_rebase_the_world_without_touching_the_layers() {
            use crate::envelope::Status;
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [64, 64],
                extent_m: [2560.0, 2560.0],
                tile_size: 64,
                halo: 0,
                ..Default::default()
            };
            reg.invoke(&ctx, Request::new("project.create", serde_json::json!({ "path": dir.path().join("g.jdproj"), "name": "G", "world": world })));
            let before = call(&reg, &ctx, "graph.evaluate", serde_json::json!({}))
                .result
                .expect("eval")["layers"]["height"]
                .clone();
            let r = call(
                &reg,
                &ctx,
                "project.set_settings",
                serde_json::json!({ "origin": [500_000.0, 4_000_000.0], "crs": "EPSG:32637" }),
            );
            assert_eq!(r.status, Status::Ok, "{:?}", r.error);
            let world = &r.result.expect("summary")["world"];
            assert_eq!(world["origin"], serde_json::json!([500_000.0, 4_000_000.0]));
            assert_eq!(world["crs"], "EPSG:32637");
            // Generators sample world space: the window moved over the infinite noise field, so
            // the layer is a different (seamlessly adjacent) piece of it — not a shifted copy.
            let after = call(&reg, &ctx, "graph.evaluate", serde_json::json!({}))
                .result
                .expect("eval")["layers"]["height"]
                .clone();
            assert_ne!(before, after, "noise follows the world origin");
            // The georeferenced TIFF carries the origin as its tie point (north-west corner).
            let r = call(
                &reg,
                &ctx,
                "terrain.export_heightmap",
                serde_json::json!({ "layer": "height", "path": "geo.tif", "format": "tiff32", "georeference": true }),
            );
            assert_eq!(r.status, Status::Ok, "{:?}", r.error);
            let tif = std::fs::read(dir.path().join("g.jdproj/exports/geo.tif")).expect("tif");
            let tie = 4_000_000.0f64 + 2560.0; // origin y + extent = the north edge
            assert!(
                tif.windows(8).any(|w| w == 500_000.0f64.to_le_bytes())
                    && tif.windows(8).any(|w| w == tie.to_le_bytes()),
                "tie point x and north edge present in the TIFF tags"
            );
            // Persisted through save → reopen; an empty crs clears it; a padded one is refused.
            call(&reg, &ctx, "project.save", serde_json::json!({}));
            let r = call(
                &reg,
                &ctx,
                "project.set_settings",
                serde_json::json!({ "crs": " EPSG:1 " }),
            );
            assert_eq!(r.status, Status::Error);
            let r = call(
                &reg,
                &ctx,
                "project.set_settings",
                serde_json::json!({ "crs": "" }),
            );
            assert_eq!(r.status, Status::Ok);
            assert!(r.result.expect("summary")["world"]["crs"].is_null());
        }

        #[test]
        fn timeline_export_writes_one_frame_per_time_with_a_shared_range() {
            use crate::envelope::Status;
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
            reg.invoke(&ctx, Request::new("project.create", serde_json::json!({ "path": dir.path().join("t.jdproj"), "name": "T", "world": world })));
            let noise = call(&reg, &ctx, "graph.list", serde_json::json!({}))
                .result
                .expect("list")["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["kind"] == "jd.noise.fbm@1")
                .expect("noise")["id"]
                .clone();
            // The terrain doubles in height over the timeline.
            let r = call(
                &reg,
                &ctx,
                "graph.set_param",
                serde_json::json!({ "id": noise, "key": "height_range_m", "value": { "keys": [[0.0, [0.0, 100.0]], [1.0, [0.0, 200.0]]] } }),
            );
            assert_eq!(r.status, Status::Ok, "{:?}", r.error);
            let r = call(
                &reg,
                &ctx,
                "timeline.export",
                serde_json::json!({ "frames": 3, "dir": "seq", "format": "png16" }),
            );
            assert_eq!(r.status, Status::Ok, "{:?}", r.error);
            let rep: TimelineExportReport =
                serde_json::from_value(r.result.expect("report")).expect("shape");
            assert_eq!(rep.frames.len(), 3);
            assert_eq!(
                rep.frames
                    .iter()
                    .map(|f| f.file.as_str())
                    .collect::<Vec<_>>(),
                ["height_0000.png", "height_0001.png", "height_0002.png"]
            );
            assert!(
                rep.frames.iter().all(|f| rep.dir.join(&f.file).exists()),
                "every frame on disk"
            );
            assert!(
                (rep.frames[0].time, rep.frames[1].time, rep.frames[2].time) == (0.0, 0.5, 1.0)
            );
            assert!(
                rep.frames[2].max > rep.frames[0].max * 1.5,
                "keyed range animates: {:?}",
                rep.frames
            );
            assert_eq!(
                rep.range_m[1], rep.frames[2].max,
                "the quantised frames share the sequence's range"
            );
            // The project's own time is untouched and nothing was journaled.
            let g = call(&reg, &ctx, "graph.list", serde_json::json!({}))
                .result
                .expect("list");
            assert_eq!(g["time"], 0.0);
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "timeline.export",
                    serde_json::json!({ "frames": 1, "dir": "seq", "format": "png16" })
                )
                .status,
                Status::Error
            );
        }

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
        fn detached_jobs_queue_on_one_slot_and_honour_timeouts() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join("q.jdproj");
            let ctx = Context::default();
            ctx.jobs.set_max_detached(1);
            let world = WorldSettings {
                resolution_px: [256, 256],
                extent_m: [256.0, 256.0],
                tile_size: 256,
                halo: 4,
                ..Default::default()
            };
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.create",
                    serde_json::json!({ "path": path, "name": "Q", "world": world })
                )
                .status,
                crate::envelope::Status::Ok
            );
            // noise → hydraulic(2000) → output: long enough to observe queueing.
            let view = call(&reg, &ctx, "graph.list", serde_json::json!({}))
                .result
                .expect("view");
            let noise = view["nodes"]
                .as_array()
                .expect("n")
                .iter()
                .find(|n| n["kind"].as_str().unwrap_or("").starts_with("jd.noise"))
                .expect("noise")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            let output = view["nodes"]
                .as_array()
                .expect("n")
                .iter()
                .find(|n| n["kind"].as_str().unwrap_or("").starts_with("jd.output"))
                .expect("out")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            let hid = call(&reg, &ctx, "graph.add_node", serde_json::json!({ "kind": "jd.erosion.hydraulic@1", "params": { "iterations": 2000 } })).result.expect("id")["id"].as_str().expect("s").to_owned();
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
            let a = call(&reg, &ctx, "graph.evaluate_async", serde_json::json!({}))
                .job_id
                .expect("a");
            let b = call(&reg, &ctx, "graph.evaluate_async", serde_json::json!({}))
                .job_id
                .expect("b");
            // Give both threads a moment: one runs, the other waits for the single slot.
            let mut states = (String::new(), String::new());
            for _ in 0..200 {
                let listed = call(&reg, &ctx, "jobs.list", serde_json::json!({}))
                    .result
                    .expect("list");
                let arr = listed.as_array().expect("arr");
                let st = |id: u64| {
                    arr.iter()
                        .find(|j| j["id"] == id)
                        .and_then(|j| j["state"].as_str())
                        .unwrap_or("")
                        .to_owned()
                };
                states = (st(a), st(b));
                if states.0 == "running" && states.1 == "queued" {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(states, ("running".to_owned(), "queued".to_owned()));
            // Priorities: a `high` job submitted after `b` takes the slot first when `a` ends;
            // `low` goes last. Running jobs are never pre-empted.
            let bad = call(
                &reg,
                &ctx,
                "graph.evaluate_async",
                serde_json::json!({ "priority": "urgent" }),
            );
            assert_eq!(bad.error.map(|e| e.code), Some(ErrorCode::E_INPUT_INVALID));
            let c = call(
                &reg,
                &ctx,
                "graph.evaluate_async",
                serde_json::json!({ "priority": "high" }),
            )
            .job_id
            .expect("c");
            let d = call(
                &reg,
                &ctx,
                "graph.evaluate_async",
                serde_json::json!({ "priority": "low" }),
            )
            .job_id
            .expect("d");
            let states_of = |ids: &[u64]| -> Vec<(String, String)> {
                let listed = call(&reg, &ctx, "jobs.list", serde_json::json!({}))
                    .result
                    .expect("list");
                let arr = listed.as_array().expect("arr");
                ids.iter()
                    .map(|id| {
                        let j = arr.iter().find(|j| j["id"] == *id);
                        let f = |k: &str| j.and_then(|j| j[k].as_str()).unwrap_or("").to_owned();
                        (f("state"), f("priority"))
                    })
                    .collect()
            };
            assert_eq!(states_of(&[a, c])[0].0, "running");
            assert_eq!(
                states_of(&[b, c, d]),
                vec![
                    ("queued".to_owned(), "normal".to_owned()),
                    ("queued".to_owned(), "high".to_owned()),
                    ("queued".to_owned(), "low".to_owned())
                ]
            );
            call(&reg, &ctx, "jobs.cancel", serde_json::json!({ "id": a }));
            assert_eq!(wait_done(&reg, &ctx, a)["error"]["code"], "E_CANCELLED");
            let mut st = Vec::new();
            for _ in 0..200 {
                st = states_of(&[b, c, d]);
                if st[1].0 == "running" {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(
                st.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(),
                ["queued", "running", "queued"],
                "high overtakes the queue: {st:?}"
            );
            // Cancelling a queued job ends it without running; cancelling the running one frees the slot.
            assert_eq!(
                call(&reg, &ctx, "jobs.cancel", serde_json::json!({ "id": b })).result,
                Some(serde_json::json!(true))
            );
            let env_b = wait_done(&reg, &ctx, b);
            assert_eq!(env_b["error"]["code"], "E_CANCELLED", "{env_b}");
            assert_eq!(env_b["error"]["message"], "cancelled while queued");
            call(&reg, &ctx, "jobs.cancel", serde_json::json!({ "id": d }));
            assert_eq!(
                wait_done(&reg, &ctx, d)["error"]["message"],
                "cancelled while queued"
            );
            call(&reg, &ctx, "jobs.cancel", serde_json::json!({ "id": c }));
            assert_eq!(wait_done(&reg, &ctx, c)["error"]["code"], "E_CANCELLED");
            // Timeout: the watchdog cancels and the envelope says E_TIMEOUT.
            let bad = call(
                &reg,
                &ctx,
                "graph.evaluate_async",
                serde_json::json!({ "timeout_s": -1.0 }),
            );
            assert_eq!(bad.error.map(|e| e.code), Some(ErrorCode::E_INPUT_INVALID));
            let t = call(
                &reg,
                &ctx,
                "graph.evaluate_async",
                serde_json::json!({ "timeout_s": 0.05 }),
            )
            .job_id
            .expect("t");
            let env_t = wait_done(&reg, &ctx, t);
            assert_eq!(env_t["error"]["code"], "E_TIMEOUT", "{env_t}");
            assert_eq!(env_t["error"]["details"]["job_id"], t);
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
        fn memory_budget_refuses_oversized_evaluations_with_an_estimate() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join("m.jdproj");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [512, 512],
                extent_m: [512.0, 512.0],
                tile_size: 512,
                halo: 4,
                ..Default::default()
            };
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.create",
                    serde_json::json!({ "path": path, "name": "M", "world": world })
                )
                .status,
                crate::envelope::Status::Ok
            );
            // 512² noise → output: ~2 MiB of outputs; a 256 MiB budget (the minimum) is plenty.
            assert_eq!(
                call(&reg, &ctx, "graph.evaluate", serde_json::json!({})).status,
                crate::envelope::Status::Ok
            );
            // Shrink the budget to the minimum and add a hydraulic node (40 B/cell scratch) on a
            // world large enough to exceed it: 4096² → outputs 4 × 64 MiB + scratch 640 MiB.
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.set_settings",
                    serde_json::json!({ "memory_budget_bytes": 256 * 1024 * 1024 })
                )
                .status,
                crate::envelope::Status::Ok
            );
            let too_small = call(
                &reg,
                &ctx,
                "project.set_settings",
                serde_json::json!({ "memory_budget_bytes": 1024 }),
            );
            assert_eq!(
                too_small.error.map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID)
            );
            let big = WorldSettings {
                resolution_px: [4096, 4096],
                extent_m: [4096.0, 4096.0],
                tile_size: 512,
                halo: 4,
                ..Default::default()
            };
            let path2 = dir.path().join("big.jdproj");
            let ctx2 = Context::default();
            assert_eq!(
                call(
                    &reg,
                    &ctx2,
                    "project.create",
                    serde_json::json!({ "path": path2, "name": "Big", "world": big })
                )
                .status,
                crate::envelope::Status::Ok
            );
            call(
                &reg,
                &ctx2,
                "project.set_settings",
                serde_json::json!({ "memory_budget_bytes": 256 * 1024 * 1024 }),
            );
            let view = call(&reg, &ctx2, "graph.list", serde_json::json!({}))
                .result
                .expect("view");
            let noise = view["nodes"]
                .as_array()
                .expect("n")
                .iter()
                .find(|n| n["kind"].as_str().unwrap_or("").starts_with("jd.noise"))
                .expect("noise")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            let output = view["nodes"]
                .as_array()
                .expect("n")
                .iter()
                .find(|n| n["kind"].as_str().unwrap_or("").starts_with("jd.output"))
                .expect("out")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            let hid = call(&reg, &ctx2, "graph.add_node", serde_json::json!({ "kind": "jd.erosion.hydraulic@1", "params": { "iterations": 1 } })).result.expect("id")["id"].as_str().expect("s").to_owned();
            call(
                &reg,
                &ctx2,
                "graph.connect",
                serde_json::json!({ "from": noise, "to": hid, "to_port": "in" }),
            );
            call(
                &reg,
                &ctx2,
                "graph.connect",
                serde_json::json!({ "from": hid, "from_port": "out", "to": output, "to_port": "in" }),
            );
            let r = call(&reg, &ctx2, "graph.evaluate", serde_json::json!({}));
            let err = r.error.expect("refused");
            assert_eq!(err.code, ErrorCode::E_RESOURCE_MEMORY);
            let est = err.details["estimated_bytes"].as_u64().expect("estimate");
            assert!(est > 256 * 1024 * 1024, "estimate {est}");
            assert_eq!(err.details["budget_bytes"], 256 * 1024 * 1024);
            // A coarser lod fits: 4096² / 16 at lod 4.
            assert_eq!(
                call(
                    &reg,
                    &ctx2,
                    "graph.evaluate",
                    serde_json::json!({ "lod": 4 })
                )
                .status,
                crate::envelope::Status::Ok,
                "lod 4 fits the budget"
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
        fn an_evaluation_that_outlives_a_graph_edit_publishes_nothing() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join("s.jdproj");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [96, 96],
                extent_m: [960.0, 960.0],
                tile_size: 64,
                halo: 4,
                ..Default::default()
            };
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.create",
                    serde_json::json!({ "path": path, "name": "S", "world": world })
                )
                .status,
                crate::envelope::Status::Ok
            );
            let noise = jd_graph::nodes::STARTER_NOISE_ID.to_string();
            let out = jd_graph::nodes::STARTER_OUTPUT_ID.to_string();
            // A slow node between noise and the output: thousands of hydraulic steps.
            let hyd = call(
                &reg,
                &ctx,
                "graph.add_node",
                serde_json::json!({ "kind": "jd.erosion.hydraulic@1", "params": { "iterations": 2000 } }),
            )
            .result
            .expect("hydraulic")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "graph.disconnect",
                    serde_json::json!({ "from": noise, "to": out, "to_port": "in" })
                )
                .status,
                crate::envelope::Status::Ok
            );
            for (from, to) in [(noise.as_str(), hyd.as_str()), (hyd.as_str(), out.as_str())] {
                assert_eq!(
                    call(
                        &reg,
                        &ctx,
                        "graph.connect",
                        serde_json::json!({ "from": from, "to": to, "to_port": "in" })
                    )
                    .status,
                    crate::envelope::Status::Ok
                );
            }
            // Start the long evaluation and wait until it has snapshotted the graph and is inside
            // the hydraulic node (jobs.list: 1 of 3 steps done), then edit the graph under it.
            let started = call(&reg, &ctx, "graph.evaluate_async", serde_json::json!({}));
            let id = started.job_id.expect("job id");
            let mut running = false;
            for _ in 0..600 {
                let jobs = call(&reg, &ctx, "jobs.list", serde_json::json!({}))
                    .result
                    .expect("jobs");
                if jobs.as_array().is_some_and(|j| {
                    j.iter()
                        .any(|x| x["id"] == id && x["done"].as_u64() >= Some(1))
                }) {
                    running = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(running, "the job never reported progress");
            let r = call(
                &reg,
                &ctx,
                "graph.set_param",
                serde_json::json!({ "id": noise, "key": "seed", "value": 9 }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            let done = wait_done(&reg, &ctx, id); // the finished op's envelope
            let env = &done;
            assert_eq!(env["status"], "ok", "{done:?}");
            assert!(
                env["warnings"]
                    .as_array()
                    .is_some_and(|w| w.iter().any(|x| x["code"] == "W_STALE_GRAPH")),
                "stale warning expected: {done:?}"
            );
            assert!(
                env["result"]["layers"]
                    .as_object()
                    .is_some_and(|l| l.is_empty()),
                "no layers published: {done:?}"
            );
            // A fresh evaluation on the edited graph publishes as usual.
            let r = call(
                &reg,
                &ctx,
                "graph.evaluate",
                serde_json::json!({ "lod": 2 }),
            );
            assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            assert!(
                r.result.expect("result")["layers"]
                    .as_object()
                    .is_some_and(|l| !l.is_empty())
            );
        }

        #[test]
        fn timeline_keys_resolve_at_the_journaled_time_and_validate_at_every_key() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join("t.jdproj");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [64, 64],
                extent_m: [640.0, 640.0],
                tile_size: 64,
                halo: 4,
                ..Default::default()
            };
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "project.create",
                    serde_json::json!({ "path": path, "name": "T", "world": world })
                )
                .status,
                crate::envelope::Status::Ok
            );
            // Thermal erosion whose step count is keyed 1 → 200 over the timeline, spliced in front of the output.
            let noise = jd_graph::nodes::STARTER_NOISE_ID.to_string();
            let out = jd_graph::nodes::STARTER_OUTPUT_ID.to_string();
            let th = call(
                &reg,
                &ctx,
                "graph.add_node",
                serde_json::json!({ "kind": "jd.erosion.thermal@1", "params": { "iterations": { "keys": [[0.0, 1], [1.0, 200]] }, "talus_deg": 30.0 } }),
            )
            .result
            .expect("thermal")["id"]
                .as_str()
                .expect("id")
                .to_owned();
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "graph.disconnect",
                    serde_json::json!({ "from": noise, "to": out, "to_port": "in" })
                )
                .status,
                crate::envelope::Status::Ok
            );
            for (from, to) in [(noise.as_str(), th.as_str()), (th.as_str(), out.as_str())] {
                let r = call(
                    &reg,
                    &ctx,
                    "graph.connect",
                    serde_json::json!({ "from": from, "to": to, "to_port": "in" }),
                );
                assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
            }
            let at = |t: f64| {
                let r = call(
                    &reg,
                    &ctx,
                    "graph.set_time",
                    serde_json::json!({ "time": t }),
                );
                assert_eq!(r.result.expect("view")["time"].as_f64(), Some(t));
                let r = call(&reg, &ctx, "graph.evaluate", serde_json::json!({}));
                assert_eq!(r.status, crate::envelope::Status::Ok, "{r:?}");
                let st = call(
                    &reg,
                    &ctx,
                    "terrain.stats",
                    serde_json::json!({ "layer": "height" }),
                );
                st.result.expect("stats")["content_hash"]
                    .as_str()
                    .expect("hash")
                    .to_owned()
            };
            let (h0, h_mid, h1) = (at(0.0), at(0.5), at(1.0));
            assert_ne!(h0, h1, "1 step and 200 steps differ");
            assert_ne!(h0, h_mid);
            assert_ne!(h_mid, h1);
            assert_eq!(
                at(0.5),
                h_mid,
                "the same time evaluates to the same hash (cached)"
            );
            // A keyframe outside the schema is refused before journaling, even though time 0 is fine.
            let r = call(
                &reg,
                &ctx,
                "graph.set_param",
                serde_json::json!({ "id": th, "key": "iterations", "value": { "keys": [[0.0, 10], [1.0, "many"]] } }),
            );
            assert_eq!(
                r.error.as_ref().map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID),
                "{r:?}"
            );
            assert_eq!(
                call(
                    &reg,
                    &ctx,
                    "graph.set_time",
                    serde_json::json!({ "time": 1.5 })
                )
                .error
                .map(|e| e.code),
                Some(ErrorCode::E_INPUT_INVALID)
            );
            // Time is journaled: undo takes the last successful move (1.0 → 0.5) back to 1.0; the
            // refused edits above were never journaled.
            let r = call(&reg, &ctx, "graph.undo", serde_json::json!({}));
            assert_eq!(r.result.expect("undo")["time"].as_f64(), Some(1.0));
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

    /// `terrain.export_profile` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainExportProfile {
        /// Target engine.
        pub profile: jd_terrain::EngineProfile,
        /// Output directory; relative paths resolve against the project's `exports/`.
        pub dir: PathBuf,
        /// Height layer.
        #[serde(default = "default_layer")]
        pub layer: String,
        /// 0..1 mask layers written as weightmaps (`weight_<name>.png`).
        #[serde(default)]
        pub masks: Vec<String>,
        /// Height range to encode (default: the layer's min/max).
        #[serde(default)]
        pub range_m: Option<[f32; 2]>,
        /// The material stack to translate: masked layers become weightmaps, `materials.json` carries the look, and the report's `loss` lists what the engine cannot take.
        #[serde(default)]
        pub materials: Vec<MaterialSpec>,
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
        /// Output `.csv`, `.json` or `.glb` (proxy mesh instanced with `EXT_mesh_gpu_instancing`);
        /// relative paths resolve against the project's `exports/`. Omit it to get the points
        /// back in the result instead (viewport preview; at most 200 000).
        #[serde(default)]
        pub path: Option<PathBuf>,
        /// Instance a generated rock (seeded from the scatter seed) instead of the 1 m cone proxy in `.glb` output.
        #[serde(default)]
        pub rock: Option<jd_terrain::RockParams>,
        /// Keep clear of an earlier scatter: no point within `radius_m` of any instance in that CSV (ecosystem layers that exclude each other).
        #[serde(default)]
        pub avoid: Option<ScatterAvoid>,
        /// Use the project's timeline position as the scatter `time` when `time` is not given (the desktop's live preview: scrub the timeline and the population grows).
        #[serde(default)]
        pub follow_time: bool,
        /// Write `path` in an engine's own conventions instead of the plain formats: `unreal` → DataTable CSV in centimetres (X east, Y south, Z up, Yaw, Scale), `unity` → JSON in the Terrain's local metres (x east, y up, z north, nx/nz normalised).
        #[serde(default)]
        pub engine: Option<jd_terrain::EngineProfile>,
        /// Spacing, jitter, seed.
        #[serde(flatten)]
        pub scatter: jd_terrain::ScatterParams,
    }

    /// An earlier scatter layer to keep clear of.
    #[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ScatterAvoid {
        /// Its `.csv` (`u,v,z,r`); relative paths resolve against the project's `exports/`.
        pub csv: PathBuf,
        /// Exclusion radius around each of its instances, metres (> 0).
        pub radius_m: f32,
    }

    fn default_layer_opt() -> Option<String> {
        Some(default_layer())
    }

    /// `terrain.scatter` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct ScatterReport {
        /// Written file (null when the points were returned inline).
        pub path: Option<PathBuf>,
        /// Points placed.
        pub count: u64,
        /// Points dropped for `avoid` (within its radius of an earlier layer).
        #[serde(default)]
        pub excluded: u64,
        /// Bytes written (0 inline).
        pub bytes: u64,
        /// Grid spacing used.
        pub spacing_m: f32,
        /// The points (`u` east, `v` north from the south edge, `z`, `r`) when no `path` was given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub points: Option<Vec<jd_terrain::ScatterPoint>>,
    }

    /// `terrain.export_mesh` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainExportMesh {
        /// Layer name.
        #[serde(default = "default_layer")]
        pub layer: String,
        /// Output `.glb`, `.fbx`, `.obj`, `.stl`, `.ply`, `.usda`, `.usdc` or `.usdz`; relative paths resolve against the project's `exports/` directory.
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
        /// Decimate the sampled grid to at most this many triangles (quadric edge collapse):
        /// flat ground collapses, ridges keep their detail. Not for `.fbx`.
        #[serde(default)]
        pub target_triangles: Option<u32>,
        /// Material layers baked into the GLB's albedo and roughness textures (see `terrain.textures`).
        #[serde(default)]
        pub materials: Vec<MaterialSpec>,
        /// Also write this many coarser levels beside the file (`<stem>_lod1.<ext>`, …), each with
        /// a quarter of the previous level's triangles; the report lists them under `lods`. 1..=8, not for `.fbx`.
        #[serde(default)]
        pub lods: Option<u32>,
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

    /// `vision.depth` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct VisionDepth {
        /// Photograph to read (PNG or JPEG; absolute, or relative to the project's `assets/`).
        pub image: PathBuf,
        /// ONNX model (absolute, or relative to the project root): MoGe-2 (`model.onnx` from
        /// `Ruicheng/moge-2-vit{s,b,l}-normal-onnx`, MIT) or Depth Anything 3 metric
        /// (`Heliosoph/da3metric-large-onnx`, Apache-2.0) — told apart by the graph's outputs.
        pub model: PathBuf,
        /// Output heightmap, OpenEXR f32 (absolute, or relative to the project's `exports/`): the
        /// relief 0..1, near = high, row 0 = the top of the photograph, masked pixels 0 — or,
        /// with `metric`, the depth per pixel in metres (masked pixels 0).
        #[serde(default = "default_vision_path")]
        pub path: PathBuf,
        /// The camera's horizontal field of view in degrees, when known: MoGe-2 then fits the
        /// z shift only (its own estimate is the default), DA3 needs it for metres.
        #[serde(default)]
        #[schemars(range(min = 1.0, max = 179.0))]
        pub fov_x_deg: Option<f32>,
        /// Write the depth in metres instead of the 0..1 relief.
        #[serde(default)]
        pub metric: bool,
        /// DA3 only: the `da3-base` ONNX export (`Heliosoph/da3-base-onnx`, Apache-2.0; absolute,
        /// or relative to the project root) whose camera head estimates the field of view when
        /// `fov_x_deg` is not given — the metric export has no camera head of its own.
        #[serde(default)]
        pub camera_model: Option<PathBuf>,
        /// Long side the photograph is resized to before inference (518..=4096).
        #[serde(default = "default_vision_px")]
        #[schemars(range(min = 518, max = 4096))]
        pub max_px: u32,
        /// Token budget of the model (1200..=3600): more is finer and slower.
        #[serde(default = "default_vision_tokens")]
        #[schemars(range(min = 1200, max = 3600))]
        pub num_tokens: i64,
    }
    fn default_vision_path() -> PathBuf {
        PathBuf::from("photo-depth.exr")
    }
    fn default_vision_px() -> u32 {
        1024
    }
    fn default_vision_tokens() -> i64 {
        1800
    }

    /// `vision.depth` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct VisionDepthResult {
        /// The EXR written.
        pub path: PathBuf,
        /// Field size in pixels.
        pub px: [u32; 2],
        /// Share of pixels the model masked out (sky, nothing), 0..1.
        pub masked_share: f32,
        /// `moge2` or `da3-metric`.
        pub model: String,
        /// Whether `depth_m` (and a `metric` EXR) is in metres: always for MoGe-2, for DA3 only
        /// with `fov_x_deg`.
        pub metric: bool,
        /// Horizontal field of view in degrees (recovered by MoGe-2, estimated by DA3's camera
        /// head, or the one given).
        pub fov_x_deg: Option<f32>,
        /// `recovered`, `given`, `camera` or `none`.
        pub fov_source: String,
        /// Depth over the trusted pixels, `[min, max]`: metres when `metric`, else the model's
        /// canonical units.
        pub depth_m: [f32; 2],
        /// MoGe-2's metric scale (metres per unit of shifted z; 1 for DA3).
        pub metric_scale: f32,
        /// Milliseconds loading the model (0 when it was already resident).
        pub load_ms: u64,
        /// Milliseconds in the session.
        pub infer_ms: u64,
    }

    /// `sky.bake` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct SkyBake {
        /// Sun and atmosphere.
        #[serde(default, flatten)]
        pub sky: jd_terrain::SkyParams,
        /// Width of the equirectangular map in pixels (even, 16..=8192); the height is half.
        #[serde(default = "default_sky_width")]
        pub width: u32,
        /// Output `.exr`; relative paths resolve against the project's `exports/`. A `.png` preview is written beside it.
        #[serde(default = "default_sky_path")]
        pub path: PathBuf,
    }

    fn default_sky_width() -> u32 {
        2048
    }
    fn default_sky_path() -> PathBuf {
        PathBuf::from("sky.exr")
    }

    /// `sky.preview` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct SkyPreview {
        /// Sun and atmosphere.
        #[serde(default, flatten)]
        pub sky: jd_terrain::SkyParams,
        /// Width of the equirectangular preview in pixels (even, 16..=2048); the height is half.
        #[serde(default = "default_sky_preview_width")]
        pub width: u32,
    }

    fn default_sky_preview_width() -> u32 {
        512
    }

    /// `sky.preview` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct SkyPreviewResult {
        /// Preview size in pixels.
        pub width: u32,
        /// Preview height in pixels.
        pub height: u32,
        /// Tonemapped sRGB PNG (equirectangular, row 0 = zenith, column 0 = north), standard base64.
        pub png_base64: String,
    }

    /// `sky.bake` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct SkyBakeResult {
        /// The HDRI written (OpenEXR, RGB f32, equirectangular, row 0 = zenith, column 0 = north).
        pub path: PathBuf,
        /// Tonemapped preview written beside it.
        pub preview: PathBuf,
        /// Map size in pixels.
        pub px: [u32; 2],
        /// Linear radiance straight up and just above the horizon (north), after exposure.
        pub zenith_rgb: [f32; 3],
        /// Linear radiance just above the horizon (north), after exposure.
        pub horizon_rgb: [f32; 3],
    }

    /// `terrain.horizon` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainHorizon {
        /// How much larger than the world the horizon field is (1.5..=16), centred on the world.
        #[serde(default = "default_horizon_scale")]
        pub scale: f32,
        /// Resolution of the horizon field in pixels per side (16..=1024).
        #[serde(default = "default_horizon_px")]
        pub px: u32,
    }

    fn default_horizon_scale() -> f32 {
        3.0
    }
    fn default_horizon_px() -> u32 {
        256
    }

    /// `terrain.rocks` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainRocks {
        /// How many rocks to write (1..=64): `rock_0.glb` … seeded `seed`, `seed + 1`, ….
        #[serde(default = "default_rock_count")]
        pub count: u32,
        /// Shape: radius, detail, roughness, flatten, base seed.
        #[serde(default, flatten)]
        pub rock: jd_terrain::RockParams,
        /// Output directory; relative paths resolve against the project's `exports/`.
        #[serde(default = "default_rocks_dir")]
        pub dir: PathBuf,
    }

    fn default_rock_count() -> u32 {
        4
    }
    fn default_rocks_dir() -> PathBuf {
        PathBuf::from("rocks")
    }

    /// One written rock.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct RockFile {
        /// Written `.glb`.
        pub path: PathBuf,
        /// Seed of this rock.
        pub seed: u64,
        /// Triangles.
        pub triangles: u32,
        /// Bounding box size x / y / z, metres.
        pub size_m: [f32; 3],
    }

    /// `ocean.bake` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct OceanBake {
        /// Height layer the sea floods.
        #[serde(default = "default_layer")]
        pub layer: String,
        /// Sea level, metres.
        #[serde(default)]
        pub level_m: f32,
        /// Water clarity and tint (the sea-level node's `visibility_m` / `tint`).
        #[serde(default, flatten)]
        pub look: jd_terrain::WaterLook,
        /// Wind sea (the sea-level node's `wind_speed_mps` / `fetch_km` / `wind_azimuth_deg`).
        #[serde(default, flatten)]
        pub wind: jd_terrain::WaveParams,
        /// Largest side of the depth / colour / foam maps, pixels.
        #[serde(default = "default_max_px")]
        pub max_px: u32,
        /// Side of the tileable wave normal map, pixels.
        #[serde(default = "default_wave_px")]
        pub wave_px: u32,
        /// World size the wave tile covers, metres; omit for a quarter of the world's width.
        #[serde(default)]
        pub wave_tile_m: Option<f32>,
        /// Optional 0..1 shore layer (the coastal node's `shore` output): added to the foam as wash along exposed, eroding shores.
        #[serde(default)]
        pub shore: Option<String>,
        /// Output directory; relative paths resolve against the project's `exports/`.
        #[serde(default = "default_water_dir")]
        pub dir: PathBuf,
    }

    fn default_wave_px() -> u32 {
        512
    }
    fn default_water_dir() -> PathBuf {
        PathBuf::from("water")
    }

    /// `ocean.bake` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct OceanBakeResult {
        /// Directory the four PNGs were written to.
        pub dir: PathBuf,
        /// `water_depth.png` (16-bit grey, depth / max_depth_m), `water_colour.png` (sRGB), `water_foam.png` (8-bit grey), `wave_normal.png` (tangent space, tileable), `wave_displacement.png` (16-bit RGB: east / up / north offsets at t = 0, `v / displacement_scale_m · 0.5 + 0.5`, tileable).
        pub files: Vec<String>,
        /// Depth / colour / foam map size, pixels.
        pub px: [u32; 2],
        /// Deepest water, metres (the depth map's white point).
        pub max_depth_m: f32,
        /// Wave tile side in metres and pixels.
        pub wave_tile_m: f32,
        /// Wave tile side, pixels.
        pub wave_px: u32,
        /// The wind sea that was baked.
        pub waves: jd_terrain::WaveSpectrum,
        /// The components as snapped to the tile (wavelength and direction may differ slightly from `waves`).
        pub tile_components: Vec<jd_terrain::WaveComponent>,
        /// Metres per full channel swing of `wave_displacement.png` (the largest offset the sea reaches).
        pub displacement_scale_m: f32,
    }

    /// `terrain.textures` params.
    #[derive(Debug, Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TerrainTextures {
        /// Layer name.
        #[serde(default = "default_layer")]
        pub layer: String,
        /// Largest texture side in pixels (the field is sampled every ⌈size / max_px⌉ cells).
        #[serde(default = "default_max_px")]
        pub max_px: u32,
        /// Material layers blended over the hypsometric base in order, each by its 0..1 mask layer (biome class masks, snow, wetness, painted) — the material graph's layer stack.
        #[serde(default)]
        pub materials: Vec<MaterialSpec>,
    }

    /// One material layer of the stack.
    #[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct MaterialSpec {
        /// 0..1 mask layer that weights this material (omit for everywhere).
        #[serde(default)]
        pub mask: Option<String>,
        /// Base colour, sRGB 0..1.
        pub albedo: [f32; 3],
        /// Roughness 0..1.
        #[serde(default = "default_roughness")]
        pub roughness: f32,
        /// Tileable detail (grain repeated every `tile_m` metres): baked into the macro maps and
        /// the GLB by world position, returned as tiles for the viewport, written as tiles for
        /// the engines.
        #[serde(default)]
        pub detail: Option<DetailSpec>,
    }

    /// A layer's tileable detail.
    #[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct DetailSpec {
        /// Metres per repeat.
        #[serde(default = "default_tile_m")]
        #[schemars(range(min = 0.01, max = 100000.0))]
        pub tile_m: f32,
        /// Normal strength 0..1.
        #[serde(default = "default_bump")]
        #[schemars(range(min = 0.0, max = 1.0))]
        pub bump: f32,
        /// Albedo variation 0..1.
        #[serde(default = "default_contrast")]
        #[schemars(range(min = 0.0, max = 1.0))]
        pub contrast: f32,
        /// Noise seed.
        #[serde(default)]
        pub seed: u64,
        /// Tile side in pixels (16..=2048).
        #[serde(default = "default_detail_px")]
        #[schemars(range(min = 16, max = 2048))]
        pub px: u32,
    }
    fn default_tile_m() -> f32 {
        8.0
    }
    fn default_bump() -> f32 {
        0.5
    }
    fn default_contrast() -> f32 {
        0.3
    }
    fn default_detail_px() -> u32 {
        512
    }
    fn detail_of(d: &Option<DetailSpec>) -> Option<jd_terrain::detail::Detail> {
        d.as_ref().map(|d| jd_terrain::detail::Detail {
            tile_m: d.tile_m,
            bump: d.bump,
            contrast: d.contrast,
            seed: d.seed,
            px: d.px,
        })
    }

    /// One detail tile of a `terrain.textures` result.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct DetailOut {
        /// Index of the layer in `materials`.
        pub layer: usize,
        /// Its mask layer name, if any.
        pub mask: Option<String>,
        /// Metres per repeat.
        pub tile_m: f32,
        /// Tile side, pixels.
        pub px: u32,
        /// The colour tile (sRGB PNG, base64).
        pub albedo_png_base64: String,
        /// The tangent-space normal tile (PNG, base64).
        pub normal_png_base64: String,
    }

    fn default_roughness() -> f32 {
        0.9
    }

    /// Resolve the stack's mask layers into fields (cloned out: the layer lock is not re-entrant).
    fn material_layers(
        ctx: &Context,
        specs: &[MaterialSpec],
    ) -> JdResult<Vec<jd_terrain::MaterialLayer>> {
        specs
            .iter()
            .map(|m| {
                let mask = m
                    .mask
                    .as_ref()
                    .map(|name| ctx.with_layer(name, |hf| Ok(hf.clone())))
                    .transpose()?;
                Ok(jd_terrain::MaterialLayer {
                    mask,
                    albedo: m.albedo,
                    roughness: m.roughness,
                    detail: detail_of(&m.detail),
                })
            })
            .collect()
    }

    fn default_max_px() -> u32 {
        1024
    }

    /// `terrain.textures` result: PNGs, standard base64.
    #[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct TexturesResult {
        /// Texture width in pixels.
        pub width: u32,
        /// Texture height in pixels.
        pub height: u32,
        /// Base colour (hypsometric ramp blended to rock on slopes), row 0 = south.
        pub albedo_png_base64: String,
        /// Tangent-space normal map (+X east, +Y north, +Z out).
        pub normal_png_base64: String,
        /// Roughness in G (0.92 flat → 0.62 steep rock, 0.6 above the snow line), metallic 0 in B.
        pub roughness_png_base64: String,
        /// Detail tiles of the layers that have one.
        #[serde(default)]
        pub details: Vec<DetailOut>,
        /// The first four detail layers' masks packed into an RGBA PNG at the map size (base64),
        /// for blending the tiles in a viewport; empty without details.
        #[serde(default)]
        pub detail_weights_png_base64: String,
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
        r.register::<jd_terrain::WaveParams, jd_terrain::WaveSpectrum, _>(
            "ocean.waves",
            "Wind sea for a wind speed and fetch (JONSWAP growth capped at the fully developed Pierson–Moskowitz sea): significant height, peak period and wavelength, breaking depth, and the Gerstner components the viewport animates",
            Scope::None,
            |_, p| Ok((jd_terrain::wave_spectrum(&p)?, Vec::new())),
        );
        r.register::<TerrainHorizon, PreviewResult, _>(
            "terrain.horizon",
            "The world's noise continued beyond its border: the graph's noise node evaluated over `scale` × the extent (same seed and origin, so it matches at the edge) at a coarse `px`, as a u16 preview — a horizon skirt for viewers; erosion and the rest of the graph are not continued",
            Scope::ProjectRead,
            |ctx, p| {
                if !(1.5..=16.0).contains(&p.scale) {
                    return Err(JdError::invalid("scale", "must be within 1.5..=16"));
                }
                if !(16..=1024).contains(&p.px) {
                    return Err(JdError::invalid("px", "must be within 16..=1024"));
                }
                let (world, noise) = ctx.with_project(|proj| {
                    let node = proj.graph.nodes.values().find(|n| n.kind.0.starts_with("jd.noise."));
                    let noise: jd_terrain::NoiseParams = match node {
                        Some(n) => serde_json::from_value(Value::Object(jd_graph::model::resolve_keys(&n.params, proj.graph.time)))?,
                        None => return Err(JdError::invalid("graph", "no noise node to continue").next("Add a jd.noise.* node, or generate from the panel.")),
                    };
                    Ok((proj.doc.world.clone(), noise))
                })?;
                let extent = [world.extent_m[0] * f64::from(p.scale), world.extent_m[1] * f64::from(p.scale)];
                let far = WorldSettings {
                    extent_m: extent,
                    resolution_px: [p.px, p.px],
                    tile_size: p.px.max(64).next_power_of_two().min(512),
                    halo: 4,
                    origin: [world.origin[0] - (extent[0] - world.extent_m[0]) / 2.0, world.origin[1] - (extent[1] - world.extent_m[1]) / 2.0],
                    ..world.clone()
                };
                let hf = generate_noise(&far, &noise)?;
                use base64::Engine as _;
                let pv = jd_terrain::preview::preview_u16(&hf, 0);
                Ok((
                    PreviewResult { width: pv.width, height: pv.height, min: pv.min, max: pv.max, bytes_base64: base64::engine::general_purpose::STANDARD.encode(pv.to_bytes()) },
                    Vec::new(),
                ))
            },
        );
        r.register::<TerrainRocks, Vec<RockFile>, _>(
            "terrain.rocks",
            "Generate seeded, faceted boulders as .glb meshes (icosphere squashed and displaced by value noise, flat base, flat shading) — scatter proxies and props",
            Scope::Export,
            |ctx, p| {
                if !(1..=64).contains(&p.count) {
                    return Err(JdError::invalid("count", "must be within 1..=64"));
                }
                let dir = if p.dir.is_absolute() { p.dir.clone() } else { ctx.with_project(|proj| Ok(proj.root().join("exports").join(&p.dir)))? };
                std::fs::create_dir_all(&dir)?;
                let mut files = Vec::with_capacity(p.count as usize);
                for i in 0..p.count {
                    let seed = p.rock.seed.wrapping_add(u64::from(i));
                    let mesh = jd_terrain::rock_mesh(&jd_terrain::RockParams { seed, ..p.rock.clone() })?;
                    let path = dir.join(format!("rock_{i}.glb"));
                    let span = |k: usize| {
                        let (lo, hi) = mesh.positions.as_chunks::<3>().0.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v[k]), hi.max(v[k])));
                        hi - lo
                    };
                    let size_m = [span(0), span(1), span(2)];
                    let r = jd_terrain::mesh::write_glb_mesh(&mesh, [f64::from(size_m[0]), f64::from(size_m[2])], &path, 1, None)?;
                    files.push(RockFile { path, seed, triangles: r.triangles, size_m });
                }
                Ok((files, Vec::new()))
            },
        );
        r.register::<SkyPreview, SkyPreviewResult, _>(
            "sky.preview",
            "The physical sky (Preetham) as a small tonemapped equirectangular PNG for viewports — same model as `sky.bake`, no project needed",
            Scope::None,
            |_, p| {
                if !(16..=2048).contains(&p.width) || !p.width.is_multiple_of(2) {
                    return Err(JdError::invalid("width", "must be even and within 16..=2048"));
                }
                use base64::Engine as _;
                let png = jd_terrain::sky_preview_png(&p.sky, p.width)?;
                Ok((SkyPreviewResult { width: p.width, height: p.width / 2, png_base64: base64::engine::general_purpose::STANDARD.encode(png) }, Vec::new()))
            },
        );
        r.register::<SkyBake, SkyBakeResult, _>(
            "sky.bake",
            "Bake a physical sky (Preetham daylight model: sun azimuth/elevation, turbidity) as an equirectangular HDRI — OpenEXR RGB f32 for engines and renderers, with a tonemapped PNG preview beside it",
            Scope::Export,
            |ctx, p| {
                let path = ctx.with_project(|proj| Ok(if p.path.is_absolute() { p.path.clone() } else { proj.root().join("exports").join(&p.path) }))?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let (w, h, zenith_rgb, horizon_rgb) = jd_terrain::bake_sky_hdri(&p.sky, p.width, &path)?;
                let preview = path.with_extension("png");
                std::fs::write(&preview, jd_terrain::sky_preview_png(&p.sky, p.width.min(1024))?)?;
                Ok((SkyBakeResult { path, preview, px: [w, h], zenith_rgb, horizon_rgb }, Vec::new()))
            },
        );
        #[cfg(feature = "vision")]
        r.register::<VisionDepth, VisionDepthResult, _>(
            "vision.depth",
            "Monocular depth from a photograph (MoGe-2 or Depth Anything 3 metric through ONNX Runtime, CPU) written as an OpenEXR relief 0..1 — or metres with `metric` — import it with `jd.input.heightmap@1` to sculpt from a picture",
            Scope::Export,
            |ctx, p| {
                let (image, model, path) = ctx.with_project(|proj| {
                    let image = if p.image.is_absolute() { p.image.clone() } else { proj.root().join("assets").join(&p.image) };
                    let model = if p.model.is_absolute() { p.model.clone() } else { proj.root().join(&p.model) };
                    let path = if p.path.is_absolute() { p.path.clone() } else { proj.root().join("exports").join(&p.path) };
                    Ok((image, model, path))
                })?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let camera = p.camera_model.as_ref().map(|c| if c.is_absolute() { Ok(c.clone()) } else { ctx.with_project(|proj| Ok(proj.root().join(c))) }).transpose()?;
                let d = jd_vision::depth_from_image(&image, &model, p.max_px, p.num_tokens, p.fov_x_deg, camera.as_deref())?;
                if p.metric && !d.metric {
                    return Err(JdError::invalid("metric", "DA3's depth is canonical without the camera's field of view").next("Pass `fov_x_deg` or `camera_model` (the da3-base export), or use MoGe-2, which recovers it."));
                }
                let field = if p.metric { &d.depth_m } else { &d.height01 };
                let mut hf = jd_terrain::Heightfield::new(d.width, d.height, 64, 0)?;
                hf.fill_with(|x, y| {
                    let v = field[(y * d.width + x) as usize];
                    if v.is_finite() { v } else { 0.0 }
                });
                let depth_m = d.depth_m.iter().filter(|v| v.is_finite()).fold([f32::INFINITY, f32::NEG_INFINITY], |[lo, hi], &v| [lo.min(v), hi.max(v)]);
                jd_terrain::export_heightmap(
                    &hf,
                    &path,
                    &jd_terrain::ExportOptions { format: jd_terrain::HeightmapFormat::Exr, flip_y: Some(false), range_m: None, georeference: false },
                )?;
                Ok((
                    VisionDepthResult { path, px: [d.width, d.height], masked_share: d.masked_share, model: d.model.as_str().into(), metric: d.metric, fov_x_deg: d.fov_x_deg, fov_source: d.fov_source.into(), depth_m, metric_scale: d.metric_scale, load_ms: d.load_ms, infer_ms: d.infer_ms },
                    Vec::new(),
                ))
            },
        );
        r.register::<OceanBake, OceanBakeResult, _>(
            "ocean.bake",
            "Write the water textures of a sea at `level_m`: depth (16-bit), the water's colour through its clarity and tint (sRGB), the shoreline + surf foam, and a tileable Gerstner wave normal map for the wind sea",
            Scope::Export,
            |ctx, p| {
                if !(16..=8192).contains(&p.max_px) {
                    return Err(JdError::invalid("max_px", "must be within 16..=8192"));
                }
                let (dir, extent) = ctx.with_project(|proj| {
                    let dir = if p.dir.is_absolute() { p.dir.clone() } else { proj.root().join("exports").join(&p.dir) };
                    Ok((dir, proj.doc.world.extent_m))
                })?;
                let waves = jd_terrain::wave_spectrum(&p.wind)?;
                // Two layers: clone the shore mask out first (the layer lock is not re-entrant).
                let shore = p.shore.as_ref().map(|s| ctx.with_layer(s, |hf| Ok(hf.clone()))).transpose()?;
                let maps = ctx.with_layer(&p.layer, |hf| jd_terrain::bake_water_with(hf, p.level_m, &p.look, waves.breaking_depth_m, p.max_px, shore.as_ref()))?;
                let tile_m = p.wave_tile_m.unwrap_or((extent[0] / 4.0) as f32);
                let (normal, tile_components) = jd_terrain::bake_wave_normal(&waves.components, tile_m, p.wave_px)?;
                let (displacement, displacement_scale_m) = jd_terrain::bake_wave_displacement(&tile_components, tile_m, p.wave_px)?;
                std::fs::create_dir_all(&dir)?;
                let files = ["water_depth.png", "water_colour.png", "water_foam.png", "wave_normal.png", "wave_displacement.png"];
                for (name, bytes) in files.iter().zip([&maps.depth, &maps.colour, &maps.foam, &normal, &displacement]) {
                    std::fs::write(dir.join(name), bytes)?;
                }
                Ok((
                    OceanBakeResult {
                        dir,
                        files: files.iter().map(|f| f.to_string()).collect(),
                        px: maps.px,
                        max_depth_m: maps.max_depth_m,
                        wave_tile_m: tile_m,
                        wave_px: p.wave_px,
                        waves,
                        tile_components,
                        displacement_scale_m,
                    },
                    Vec::new(),
                ))
            },
        );
        r.register::<TerrainTextures, TexturesResult, _>(
            "terrain.textures",
            "Bake the material maps of a layer — albedo, tangent-space normal, roughness — as PNGs (base64) for viewers and engines",
            Scope::ProjectRead,
            |ctx, p| {
                if !(16..=8192).contains(&p.max_px) {
                    return Err(JdError::invalid("max_px", "must be within 16..=8192"));
                }
                let extent = ctx.with_project(|proj| Ok(proj.doc.world.extent_m))?;
                let layers = material_layers(ctx, &p.materials)?;
                ctx.with_layer(&p.layer, |hf| {
                    use base64::Engine as _;
                    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
                    let maps = jd_terrain::bake_maps_with(hf, extent, p.max_px, &layers)?;
                    Ok((
                        TexturesResult {
                            width: maps.px[0],
                            height: maps.px[1],
                            albedo_png_base64: b64(&maps.albedo),
                            normal_png_base64: b64(&maps.normal),
                            roughness_png_base64: b64(&maps.roughness),
                            details: maps.details.iter().map(|(k, t)| DetailOut { layer: *k, mask: p.materials[*k].mask.clone(), tile_m: t.tile_m, px: t.px, albedo_png_base64: b64(&t.albedo), normal_png_base64: b64(&t.normal) }).collect(),
                            detail_weights_png_base64: maps.detail_weights.as_deref().map(b64).unwrap_or_default(),
                        },
                        Vec::new(),
                    ))
                })
            },
        );
        r.register::<TerrainPreview, PreviewResult, _>(
            "terrain.preview",
            "Downsampled u16 preview of a layer (base64) for agents and viewers",
            Scope::ProjectRead,
            |ctx, p| {
                use base64::Engine as _;
                let pv = ctx.preview_layer(&p.layer, p.lod)?;
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
            "Write a layer as a grid mesh — .glb (normals, UVs, PBR material with baked base-colour texture), .fbx (binary 7.4 quads, Phong material with the baked texture embedded), .obj, binary .stl, binary .ply with vertex colours, .usda, or .usdz (needs usdzip) — metres, +Y up, centred on the world",
            Scope::Export,
            |ctx, p| {
                let (path, extent) = ctx.with_project(|proj| {
                    let path = if p.path.is_absolute() { p.path.clone() } else { proj.root().join("exports").join(&p.path) };
                    Ok((path, proj.doc.world.extent_m))
                })?;
                let mut warnings = Vec::new();
                let materials = material_layers(ctx, &p.materials)?;
                if !materials.is_empty() && !matches!(path.extension().and_then(|e| e.to_str()), None | Some("glb")) {
                    warnings.push(Warning::new("W_EXPORT_LOSS", "`materials` are baked into GLB textures only; this format carries the base albedo (FBX) or no texture — export .glb or paint the layers in the engine from `terrain.export_profile`"));
                }
                let report = ctx.with_layer(&p.layer, |hf| {
                    let step = p.step.unwrap_or_else(|| jd_terrain::auto_step(hf.width(), hf.height(), p.max_vertices));
                    if p.step.is_none() && step > 1 {
                        warnings.push(Warning::new(
                            "W_MESH_DECIMATED",
                            format!("mesh sampled every {step} pixels to stay within {} vertices; pass `step: 1` for the full grid", p.max_vertices),
                        ));
                    }
                    match p.lods {
                        Some(n) if n > 0 => jd_terrain::export_mesh_lods_with(hf, extent, &path, step, p.texture, p.target_triangles, n, &materials),
                        _ => jd_terrain::export_mesh_with(hf, extent, &path, step, p.texture, p.target_triangles, &materials),
                    }
                })?;
                ctx.with_project(|proj| crate::geo::write_attribution_txt(proj, path.parent().unwrap_or(proj.root())).map(|_| ()))?;
                Ok((report, warnings))
            },
        );
        r.register::<TerrainExportProfile, jd_terrain::ProfileReport, _>(
            "terrain.export_profile",
            "Write a Unity or Unreal terrain import set: heightmap in the engine's encoding and row order, one 8-bit weightmap per mask layer, and a README with the computed Terrain Height / Z scale",
            Scope::Export,
            |ctx, p| {
                let dir = if p.dir.is_absolute() { p.dir.clone() } else { ctx.with_project(|proj| Ok(proj.root().join("exports").join(&p.dir)))? };
                let height = ctx.with_layer(&p.layer, |hf| Ok(hf.clone()))?;
                let mut masks = Vec::new();
                for name in &p.masks {
                    masks.push((name.clone(), ctx.with_layer(name, |hf| Ok(hf.clone()))?));
                }
                let refs: Vec<(String, &jd_terrain::Heightfield)> = masks.iter().map(|(n, hf)| (n.clone(), hf)).collect();
                let mat_fields: Vec<Option<jd_terrain::Heightfield>> = p.materials.iter().map(|m| m.mask.as_ref().map(|n| ctx.with_layer(n, |hf| Ok(hf.clone()))).transpose()).collect::<JdResult<_>>()?;
                let stack: Vec<jd_terrain::ProfileMaterial<'_>> = p
                    .materials
                    .iter()
                    .zip(&mat_fields)
                    .enumerate()
                    .map(|(i, (m, f))| jd_terrain::ProfileMaterial { name: m.mask.clone().unwrap_or_else(|| format!("material_{i}")), mask: f.as_ref(), albedo: m.albedo, roughness: m.roughness, detail: detail_of(&m.detail) })
                    .collect();
                let geo = ctx.with_project(|proj| {
                    let w = &proj.doc.world;
                    Ok(jd_terrain::GeoRef { origin: w.origin, spacing: w.spacing_m(), crs: w.crs.clone() })
                })?;
                let report = jd_terrain::export_profile_with(p.profile, &height, &refs, &stack, &dir, p.range_m, Some(&geo))?;
                let warnings = report.notes.iter().map(|n| Warning::new("W_ENGINE_SIZE", n.clone())).collect();
                ctx.with_project(|proj| crate::geo::write_attribution_txt(proj, &dir).map(|_| ()))?;
                Ok((report, warnings))
            },
        );
        r.register::<TerrainScatter, ScatterReport, _>(
            "terrain.scatter",
            "Scatter instance points over the world from a 0..1 density mask layer (stratified jittered grid, deterministic by seed): write CSV / JSON / GLB instances to `path`, or return the points when `path` is omitted",
            Scope::Export,
            |ctx, p| {
                let (path, extent, avoid_csv) = ctx.with_project(|proj| {
                    let under = |f: &PathBuf| if f.is_absolute() { f.clone() } else { proj.root().join("exports").join(f) };
                    Ok((p.path.as_ref().map(under), proj.doc.world.extent_m, p.avoid.as_ref().map(|a| under(&a.csv))))
                })?;
                // Two layers: clone the mask out first (the layer lock is not re-entrant).
                let mask = ctx.with_layer(&p.mask, |hf| Ok(hf.clone()))?;
                let mut scatter = p.scatter.clone();
                if scatter.time.is_none() && p.follow_time {
                    scatter.time = Some(ctx.with_project(|pr| Ok(pr.graph.time))?);
                }
                let points = match &p.height {
                    Some(h) => ctx.with_layer(h, |hf| jd_terrain::scatter_points(&mask, Some(hf), extent, &scatter))?,
                    None => jd_terrain::scatter_points(&mask, None, extent, &scatter)?,
                };
                let placed = points.len() as u64;
                let points = match (&p.avoid, avoid_csv) {
                    (Some(a), Some(csv)) => jd_terrain::exclude_near(points, &jd_terrain::read_points_csv(&csv)?, a.radius_m)?,
                    _ => points,
                };
                let count = points.len() as u64;
                let excluded = placed - count;
                let (bytes, points) = match &path {
                    Some(f) if let Some(engine) = p.engine => (jd_terrain::write_points_engine(f, &points, extent, engine)?, None),
                    Some(f) => {
                        let rock = p.rock.as_ref().map(|r| jd_terrain::rock_mesh(&jd_terrain::RockParams { seed: r.seed ^ p.scatter.seed, ..r.clone() })).transpose()?;
                        (jd_terrain::write_points_with(f, &points, extent, rock.as_ref())?, None)
                    }
                    None if count > 200_000 => {
                        return Err(JdError::invalid("spacing_m", format!("{count} points is too many to return inline (max 200 000)"))
                            .next("Give a `path` to write a file, or use a larger spacing."));
                    }
                    None => (0, Some(points)),
                };
                let warnings = (p.engine.is_some() && p.rock.is_some())
                    .then(|| Warning::new("W_EXPORT_LOSS", "`rock` proxies are not written in an engine format: the file holds instances only"))
                    .into_iter()
                    .collect();
                ctx.with_project(|proj| crate::geo::write_attribution_txt(proj, path.as_deref().and_then(Path::parent).unwrap_or(proj.root())).map(|_| ()))?;
                Ok((ScatterReport { path, count, excluded, bytes, spacing_m: p.scatter.spacing_m, points }, warnings))
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
                let geo = ctx.with_project(|proj| {
                    let w = &proj.doc.world;
                    Ok(jd_terrain::GeoRef { origin: w.origin, spacing: w.spacing_m(), crs: w.crs.clone() })
                })?;
                let report = ctx.with_layer(&p.layer, |hf| jd_terrain::export_heightmap_geo(hf, &path, &p.options, Some(&geo)))?;
                let mut warnings = Vec::new();
                if report.quantisation_step_m > 0.0 {
                    warnings.push(
                        Warning::new(
                            "W_EXPORT_LOSS",
                            format!("{:?} quantises heights to {:.4} m steps; range recorded in the sidecar", report.format, report.quantisation_step_m),
                        ),
                    );
                }
                ctx.with_project(|proj| crate::geo::write_attribution_txt(proj, path.parent().unwrap_or(proj.root())).map(|_| ()))?;
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
            // Material maps: capped at max_px, three PNGs of the same size.
            let r = reg.invoke(
                &ctx,
                Request::new("terrain.textures", serde_json::json!({ "max_px": 32 })),
            );
            let res = r.result.expect("textures");
            assert_eq!(
                (&res["width"], &res["height"]),
                (&serde_json::json!(32), &serde_json::json!(24))
            );
            for key in [
                "albedo_png_base64",
                "normal_png_base64",
                "roughness_png_base64",
            ] {
                let png = base64::engine::general_purpose::STANDARD
                    .decode(res[key].as_str().expect("b64"))
                    .expect("decodes");
                assert_eq!(&png[1..4], b"PNG", "{key}");
            }
            let bad = reg.invoke(
                &ctx,
                Request::new("terrain.textures", serde_json::json!({ "max_px": 4 })),
            );
            assert_eq!(bad.error.map(|e| e.code), Some(ErrorCode::E_INPUT_INVALID));
        }

        #[test]
        fn scatter_returns_points_inline_without_a_path() {
            let reg = Registry::with_builtins();
            let dir = tempfile::tempdir().expect("tmp");
            let ctx = Context::default();
            let world = WorldSettings {
                resolution_px: [64, 64],
                extent_m: [640.0, 640.0],
                tile_size: 64,
                halo: 0,
                ..Default::default()
            };
            reg.invoke(&ctx, Request::new("project.create", serde_json::json!({ "path": dir.path().join("s.jdproj"), "name": "S", "world": world })));
            reg.invoke(
                &ctx,
                Request::new(
                    "terrain.generate",
                    serde_json::json!({ "noise": { "height_range_m": [1.0, 2.0] } }),
                ),
            );
            // Heights ≥ 1 clamp to density 1: every 40 m cell keeps its candidate.
            let r = reg.invoke(
                &ctx,
                Request::new(
                    "terrain.scatter",
                    serde_json::json!({ "mask": "height", "spacing_m": 40.0 }),
                ),
            );
            let res = r.result.expect("inline points");
            assert_eq!(
                (res["count"].as_u64(), res["bytes"].as_u64()),
                (Some(256), Some(0))
            );
            assert!(res["path"].is_null());
            let pts = res["points"].as_array().expect("points");
            assert_eq!(pts.len(), 256);
            assert!(
                pts.iter()
                    .all(|q| q["z"].as_f64().is_some_and(|z| (1.0..=2.0).contains(&z)))
            );
            // With a path the points go to the file and are not echoed.
            let r = reg.invoke(
                &ctx,
                Request::new(
                    "terrain.scatter",
                    serde_json::json!({ "mask": "height", "spacing_m": 40.0, "path": "pts.json" }),
                ),
            );
            let res = r.result.expect("file");
            assert!(res["points"].is_null() && res["bytes"].as_u64() > Some(0));
            assert!(dir.path().join("s.jdproj/exports/pts.json").exists());
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
