//! The operation registry: the single API surface behind GUI, CLI and MCP.
//!
//! Every capability is an [`Operation`] with a stable id, a scope, JSON schemas
//! for its request and result (generated from Rust types with `schemars`), and
//! a handler. Adapters call [`Registry::invoke`] with a [`Request`] and get a
//! [`Response`]; they never contain domain logic.

use std::collections::BTreeMap;
use std::path::PathBuf;
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
    /// In-memory heightfield layers produced by terrain ops, keyed by layer name.
    /// ponytail: replaced by the graph engine's content-addressed cache in Phase 2.
    layers: Arc<Mutex<BTreeMap<String, jd_terrain::Heightfield>>>,
}

impl Context {
    /// Store a heightfield layer.
    pub fn set_layer(&self, name: &str, hf: jd_terrain::Heightfield) -> JdResult<()> {
        let mut g = self
            .layers
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "layer lock poisoned"))?;
        g.insert(name.to_owned(), hf);
        Ok(())
    }

    /// Run `f` with a stored layer, or fail with `E_INPUT_INVALID`.
    pub fn with_layer<T>(
        &self,
        name: &str,
        f: impl FnOnce(&jd_terrain::Heightfield) -> JdResult<T>,
    ) -> JdResult<T> {
        let g = self
            .layers
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "layer lock poisoned"))?;
        match g.get(name) {
            Some(hf) => f(hf),
            None => Err(JdError::new(
                ErrorCode::E_INPUT_INVALID,
                format!("no heightfield layer named `{name}`"),
            )
            .why("Terrain ops read from layers produced earlier in this session.")
            .next("Run terrain.generate first (default layer name: `height`).")
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

    /// Replace the open project (saving is the caller's responsibility).
    pub fn set_project(&self, p: Option<Project>) -> JdResult<()> {
        let mut guard = self
            .project
            .lock()
            .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "project lock poisoned"))?;
        *guard = p;
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
            },
        );
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
        let Request {
            op: op_id, params, ..
        } = req;
        let outcome = handler(ctx, params);
        let duration_ms = started.elapsed().as_millis() as u64;
        match outcome {
            Ok((result, warnings)) => {
                tracing::info!(
                    duration_ms,
                    outcome = "ok",
                    warnings = warnings.len(),
                    "operation completed"
                );
                Response::ok(&op_id, request_id, result, warnings)
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
        }
    }

    /// Validate a mutation's parameters without touching the project. Runs **before** the
    /// write-ahead journal so a rejected request never leaves a bogus entry on disk.
    fn validate_mutation(op: &str, params: &Value) -> JdResult<()> {
        match op {
            "project.set_name" => {
                let args: ProjectSetName = serde_json::from_value(params.clone())?;
                Project::validate_name(&args.name)
            }
            other => Err(JdError::new(
                ErrorCode::E_INPUT_INVALID,
                format!("`{other}` is not a replayable mutation"),
            )),
        }
    }

    /// Apply a mutating op to the project in memory. Shared by live calls and journal replay so
    /// recovery cannot diverge from normal execution.
    fn apply_mutation(p: &mut Project, op: &str, params: &Value) -> JdResult<()> {
        validate_mutation(op, params)?;
        match op {
            "project.set_name" => {
                let args: ProjectSetName = serde_json::from_value(params.clone())?;
                p.set_name(&args.name)
            }
            _ => unreachable!("validate_mutation rejects unknown ops"),
        }
    }

    /// Validate → journal → apply. The only path for live mutations.
    fn mutate(p: &mut Project, op: &str, params: Value) -> JdResult<()> {
        validate_mutation(op, &params)?;
        p.journal(op, &params)?;
        apply_mutation(p, op, &params)
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
        r.register::<NoParams, RecoverResult, _>(
            "project.recover",
            "Replay unsaved journal entries found when the project was opened, then save",
            Scope::ProjectWrite,
            |ctx, _| {
                ctx.with_project(|p| {
                    let entries = p.take_pending_journal();
                    let mut replayed = Vec::new();
                    let mut failed = Vec::new();
                    for e in entries {
                        match apply_mutation(p, &e.op, &e.params) {
                            Ok(()) => replayed.push(e),
                            Err(err) => failed.push((e.op.clone(), err.message)),
                        }
                    }
                    p.save()?;
                    Ok((RecoverResult { replayed, failed }, Vec::new()))
                })
            },
        );
        r.register::<NoParams, crate::diag::EnvInfo, _>(
            "diag.env",
            "Report host environment and detected tools",
            Scope::None,
            |_, _| Ok((crate::diag::env_info(), Vec::new())),
        );
        terrain::register_all(r);
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

    /// Register terrain ops.
    pub fn register_all(r: &mut Registry) {
        r.register::<TerrainGenerate, LayerInfo, _>(
            "terrain.generate",
            "Generate a noise heightfield for the open project's world and store it as a layer",
            Scope::ProjectWrite,
            |ctx, p| {
                let world = ctx.with_project(|proj| Ok(proj.doc.world.clone()))?;
                let started = std::time::Instant::now();
                let hf = generate_noise(&world, &p.noise)?;
                let out = info(&p.layer, &hf, started.elapsed().as_millis() as u64);
                ctx.set_layer(&p.layer, hf)?;
                Ok((out, Vec::new()))
            },
        );
        r.register::<TerrainLayer, LayerInfo, _>(
            "terrain.stats",
            "Statistics and content hash of a layer",
            Scope::ProjectRead,
            |ctx, p| ctx.with_layer(&p.layer, |hf| Ok((info(&p.layer, hf, 0), Vec::new()))),
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
