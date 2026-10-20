//! The `.jdproj` project directory (`docs/spec/project-format.md`, ADR-0011).
//!
//! Layout: `project.json`, `graph.json`, `scene.json`, `assets/manifest.json`,
//! `journal/ops.ndjson` (write-ahead log of mutating ops), `cache/`
//! (disposable), `.lock`. Writes are atomic (temp file + rename). Unknown JSON
//! keys are preserved and reported as warnings.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use jd_types::{ErrorCode, JdError, JdResult, ProjectId, SchemaVersion, Units, WorldSettings};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::envelope::Warning;

/// Extension the project directory carries.
pub const PROJECT_EXT: &str = "jdproj";
const PROJECT_FILE: &str = "project.json";
const GRAPH_FILE: &str = "graph.json";
const SCENE_FILE: &str = "scene.json";
const ASSETS_MANIFEST: &str = "assets/manifest.json";
const JOURNAL_FILE: &str = "journal/ops.ndjson";
const LOCK_FILE: &str = ".lock";

/// Relative paths of the sub-documents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProjectFiles {
    /// Graph document.
    pub graph: String,
    /// Scene document.
    pub scene: String,
    /// Asset manifest.
    pub assets: String,
}

impl Default for ProjectFiles {
    fn default() -> Self {
        Self {
            graph: GRAPH_FILE.into(),
            scene: SCENE_FILE.into(),
            assets: ASSETS_MANIFEST.into(),
        }
    }
}

/// User-tunable project settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProjectSettings {
    /// Autosave interval in seconds (0 disables).
    pub autosave_interval_s: u32,
    /// Cache size cap in bytes.
    pub cache_limit_bytes: u64,
    /// Preview LOD (resolution divided by 2^lod).
    pub preview_lod: u8,
    /// Evaluations whose pre-flight memory estimate exceeds this are refused (`E_RESOURCE_MEMORY`).
    /// Default 4 GiB — the brief's ceiling (ADR-0008).
    #[serde(default = "default_memory_budget")]
    pub memory_budget_bytes: u64,
}

fn default_memory_budget() -> u64 {
    4 * 1024 * 1024 * 1024
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            autosave_interval_s: 120,
            cache_limit_bytes: 2 * 1024 * 1024 * 1024,
            preview_lod: 2,
            memory_budget_bytes: default_memory_budget(),
        }
    }
}

/// A data-source attribution obligation carried into exports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Attribution {
    /// Source name (e.g. "Copernicus DEM GLO-30").
    pub source: String,
    /// Required notice text.
    pub text: String,
    /// Licence or terms URL.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

/// `project.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProjectDocument {
    /// Document schema version.
    pub schema_version: SchemaVersion,
    /// Version of the app that last wrote the document.
    pub app_version: String,
    /// Stable project id.
    pub id: ProjectId,
    /// Display name.
    pub name: String,
    /// RFC 3339 creation time.
    pub created: String,
    /// RFC 3339 last-modified time.
    pub modified: String,
    /// Scene units.
    pub units: Units,
    /// World settings.
    pub world: WorldSettings,
    /// Sub-document paths.
    pub files: ProjectFiles,
    /// Settings.
    pub settings: ProjectSettings,
    /// Attribution obligations.
    #[serde(default)]
    pub attribution: Vec<Attribution>,
    /// Namespaced plugin data.
    #[serde(default)]
    pub extensions: Map<String, Value>,
    /// Unknown keys from newer/older writers, preserved verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// `graph.json` — the procedural graph (`jd-graph` owns the type).
pub type GraphDocument = jd_graph::Graph;

/// `scene.json` — Phase 1 header only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SceneDocument {
    /// Document schema version.
    pub schema_version: SchemaVersion,
    /// Scene units (canonical frame declared explicitly).
    pub units: Units,
    /// Preserved unknown keys.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for SceneDocument {
    fn default() -> Self {
        Self {
            schema_version: SchemaVersion::CURRENT,
            units: Units::default(),
            extra: BTreeMap::new(),
        }
    }
}

/// One write-ahead journal entry: a mutating op that was applied in memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct JournalEntry {
    /// RFC 3339 timestamp.
    pub ts: String,
    /// Operation id.
    pub op: String,
    /// Parameters as received.
    pub params: Value,
}

/// An open project: documents in memory plus the directory they live in.
#[derive(Debug)]
pub struct Project {
    root: PathBuf,
    /// `project.json`.
    pub doc: ProjectDocument,
    /// `graph.json`.
    pub graph: GraphDocument,
    /// `scene.json`.
    pub scene: SceneDocument,
    /// Journal entries found on open that were never saved (crash recovery).
    pub pending_journal: Vec<JournalEntry>,
    /// Undo/redo stacks of graph snapshots (session-only; the journal is the durable record).
    pub history: History,
    /// Bumped by every journaled mutation (session-only): an evaluation that started on an older
    /// revision publishes nothing, so a slow run can never overwrite a newer graph's layers.
    pub revision: u64,
    dirty: bool,
    lock_nonce: String,
}

/// Undo/redo history of the graph document. Every journaled `graph.*` mutation pushes the
/// previous graph; undo/redo are themselves journaled as `graph.restore`, so recovery replays
/// exactly what the user saw.
/// ponytail: whole-graph snapshots, capped at 100 entries and 64 MiB of serialised graph (a
/// stroke-heavy graph — 500 strokes × 50 points — serialises to 2 MiB, so ~30 of those; ordinary
/// graphs are kilobytes and keep all 100). Diff-based history with an on-disk archive is the
/// upgrade if a session ever needs deeper undo than that.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<(GraphDocument, usize)>,
    redo: Vec<(GraphDocument, usize)>,
}

impl History {
    const CAP: usize = 100;
    const BYTES_CAP: usize = 64 << 20;
    fn size_of(g: &GraphDocument) -> usize {
        serde_json::to_vec(g).map_or(0, |v| v.len())
    }
    /// Record the graph as it was before a user mutation (clears the redo stack).
    pub fn record(&mut self, before: GraphDocument) {
        let size = Self::size_of(&before);
        self.undo.push((before, size));
        self.redo.clear();
        self.trim_to(Self::CAP, Self::BYTES_CAP);
    }
    /// Drop the oldest snapshots past the entry and byte caps (the newest always stays).
    fn trim_to(&mut self, cap: usize, bytes_cap: usize) {
        let mut bytes = self.undo_bytes();
        while self.undo.len() > 1 && (self.undo.len() > cap || bytes > bytes_cap) {
            bytes -= self.undo.remove(0).1;
        }
    }
    /// Pop the snapshot to restore for undo, remembering `current` for redo.
    pub fn undo(&mut self, current: GraphDocument) -> Option<GraphDocument> {
        let (g, _) = self.undo.pop()?;
        let size = Self::size_of(&current);
        self.redo.push((current, size));
        Some(g)
    }
    /// Pop the snapshot to restore for redo, remembering `current` for undo.
    pub fn redo(&mut self, current: GraphDocument) -> Option<GraphDocument> {
        let (g, _) = self.redo.pop()?;
        let size = Self::size_of(&current);
        self.undo.push((current, size));
        Some(g)
    }
    /// The snapshot `undo` would restore, without changing the stacks.
    pub fn peek_undo(&self) -> Option<&GraphDocument> {
        self.undo.last().map(|(g, _)| g)
    }
    /// The snapshot `redo` would restore, without changing the stacks.
    pub fn peek_redo(&self) -> Option<&GraphDocument> {
        self.redo.last().map(|(g, _)| g)
    }
    /// (undo depth, redo depth).
    pub fn depths(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    /// Bytes of serialised graph the undo stack holds.
    pub fn undo_bytes(&self) -> usize {
        self.undo.iter().map(|(_, n)| n).sum()
    }
}

fn now_rfc3339() -> String {
    jiff::Timestamp::now().to_string()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> JdResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn to_pretty(value: &impl Serialize) -> JdResult<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, what: &str) -> JdResult<T> {
    let bytes = fs::read(path).map_err(|e| {
        JdError::new(
            ErrorCode::E_PROJECT_CORRUPT,
            format!("cannot read {what}: {e}"),
        )
        .why("The project directory is missing a required document.")
        .next("Restore the file from a backup or recreate the project.")
        .detail("path", path.display().to_string())
    })?;
    serde_json::from_slice(&bytes).map_err(|e| {
        JdError::new(
            ErrorCode::E_PROJECT_CORRUPT,
            format!("{what} is not valid: {e}"),
        )
        .why("A project document failed to parse.")
        .next("If a journal exists, open with recovery; otherwise restore from backup.")
        .detail("path", path.display().to_string())
    })
}

/// Sub-document paths must stay inside the project directory: relative, no `..`, no root.
fn check_relative(what: &str, rel: &str) -> JdResult<()> {
    use std::path::Component;
    let p = Path::new(rel);
    let escapes = p.is_absolute()
        || p.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        });
    if escapes || rel.is_empty() {
        return Err(JdError::new(ErrorCode::E_PROJECT_CORRUPT, format!("{what} must be a relative path inside the project"))
            .why("A sub-document path that escapes the project directory could read or overwrite unrelated files.")
            .next(format!("Set `{what}` in project.json to a relative path such as `graph.json`."))
            .detail("path", rel));
    }
    Ok(())
}

fn check_version(found: SchemaVersion, what: &str) -> JdResult<()> {
    if found.is_supported() {
        Ok(())
    } else {
        Err(JdError::new(ErrorCode::E_SCHEMA_VERSION, format!("{what} schema_version {found} is not supported"))
            .why(format!("This build reads schema versions 1..={}.", SchemaVersion::CURRENT))
            .next("Open the project with a newer Jenga Dunia, or export it from the version that wrote it.")
            .detail("found", found.0)
            .detail("supported", SchemaVersion::CURRENT.0))
    }
}

/// Warn about preserved unknown keys so forward-compatible documents are visible.
fn unknown_keys_warning(what: &str, extra: &BTreeMap<String, Value>) -> Option<Warning> {
    if extra.is_empty() {
        None
    } else {
        let keys: Vec<&str> = extra.keys().map(String::as_str).collect();
        Some(
            Warning::new(
                "W_UNKNOWN_FIELDS",
                format!(
                    "{what} has {} unknown field(s), preserved verbatim",
                    keys.len()
                ),
            )
            .with_details(serde_json::json!({ "document": what, "keys": keys })),
        )
    }
}

impl Warning {
    fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}

impl Project {
    /// Create a new project directory at `path` (must not exist or be empty).
    pub fn create(path: &Path, name: &str, world: WorldSettings) -> JdResult<Self> {
        world
            .validate()
            .map_err(|m| JdError::invalid("params.world", m))?;
        if name.trim().is_empty() {
            return Err(JdError::invalid(
                "params.name",
                "project name must not be empty",
            ));
        }
        if path.is_file() {
            return Err(JdError::invalid(
                "params.path",
                "target path is an existing file, not a directory",
            )
            .next("Choose a directory path (conventionally ending in `.jdproj`)."));
        }
        if path.exists() && fs::read_dir(path)?.next().is_some() {
            return Err(JdError::new(
                ErrorCode::E_INPUT_INVALID,
                "target directory exists and is not empty",
            )
            .why("Creating a project would overwrite unrelated files.")
            .next("Choose an empty or new directory.")
            .detail("path", path.display().to_string()));
        }
        fs::create_dir_all(path)?;
        for sub in ["assets", "strokes", "cache", "exports", "journal"] {
            fs::create_dir_all(path.join(sub))?;
        }
        let now = now_rfc3339();
        let doc = ProjectDocument {
            schema_version: SchemaVersion::CURRENT,
            app_version: crate::APP_VERSION.to_owned(),
            id: ProjectId::new(),
            name: name.to_owned(),
            created: now.clone(),
            modified: now,
            units: Units::default(),
            world,
            files: ProjectFiles::default(),
            settings: ProjectSettings::default(),
            attribution: Vec::new(),
            extensions: Map::new(),
            extra: BTreeMap::new(),
        };
        let mut p = Self {
            root: path.to_path_buf(),
            doc,
            graph: jd_graph::nodes::starter(),
            scene: SceneDocument::default(),
            pending_journal: Vec::new(),
            history: History::default(),
            revision: 0,
            dirty: true,
            lock_nonce: String::new(),
        };
        write_atomic(
            &path.join(ASSETS_MANIFEST),
            b"{\n  \"schema_version\": 1,\n  \"assets\": []\n}\n",
        )?;
        write_atomic(&path.join("cache/.gitignore"), b"*\n")?;
        p.acquire_lock()?;
        p.save()?;
        Ok(p)
    }

    /// Open an existing project. Returns warnings (unknown fields, pending journal, stale lock).
    pub fn open(path: &Path) -> JdResult<(Self, Vec<Warning>)> {
        let mut warnings = Vec::new();
        let doc: ProjectDocument = read_json(&path.join(PROJECT_FILE), "project.json")?;
        check_version(doc.schema_version, "project.json")?;
        doc.world.validate().map_err(|m| {
            JdError::new(
                ErrorCode::E_PROJECT_CORRUPT,
                format!("world settings invalid: {m}"),
            )
            .next("Fix `world` in project.json.")
        })?;
        for (what, rel) in [
            ("files.graph", &doc.files.graph),
            ("files.scene", &doc.files.scene),
            ("files.assets", &doc.files.assets),
        ] {
            check_relative(what, rel)?;
        }
        let graph: GraphDocument = read_json(&path.join(&doc.files.graph), "graph.json")?;
        check_version(graph.schema_version, "graph.json")?;
        let scene: SceneDocument = read_json(&path.join(&doc.files.scene), "scene.json")?;
        check_version(scene.schema_version, "scene.json")?;
        warnings.extend(unknown_keys_warning("project.json", &doc.extra));
        warnings.extend(unknown_keys_warning("graph.json", &graph.extra));
        warnings.extend(unknown_keys_warning("scene.json", &scene.extra));

        let pending_journal = read_journal(&path.join(JOURNAL_FILE))?;
        if !pending_journal.is_empty() {
            warnings.push(
                Warning::new(
                    "W_RECOVERY_AVAILABLE",
                    format!("{} unsaved operation(s) found in the journal; run project.recover to replay them", pending_journal.len()),
                )
                .with_details(serde_json::json!({ "count": pending_journal.len() })),
            );
        }
        let mut p = Self {
            root: path.to_path_buf(),
            doc,
            graph,
            scene,
            pending_journal,
            history: History::default(),
            revision: 0,
            dirty: false,
            lock_nonce: String::new(),
        };
        if let Some(w) = p.acquire_lock()? {
            warnings.push(w);
        }
        Ok((p, warnings))
    }

    /// Directory of this project.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// True when in-memory documents differ from disk.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Record a mutating op in the write-ahead journal **before** applying it in memory.
    pub fn journal(&mut self, op: &str, params: &Value) -> JdResult<()> {
        let entry = JournalEntry {
            ts: now_rfc3339(),
            op: op.to_owned(),
            params: params.clone(),
        };
        let path = self.root.join(JOURNAL_FILE);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        serde_json::to_writer(&mut f, &entry)?;
        f.write_all(b"\n")?;
        f.sync_data()?;
        self.dirty = true;
        Ok(())
    }

    /// Validate a project name (shared by create, set_name and pre-journal validation).
    pub fn validate_name(name: &str) -> JdResult<()> {
        if name.trim().is_empty() {
            return Err(JdError::invalid(
                "params.name",
                "project name must not be empty",
            ));
        }
        Ok(())
    }

    /// Validate settings (shared by `project.set_settings` and pre-journal validation).
    pub fn validate_settings(s: &ProjectSettings) -> JdResult<()> {
        if s.autosave_interval_s > 86_400 {
            return Err(JdError::invalid(
                "params.autosave_interval_s",
                "must be ≤ 86400 (a day); 0 disables autosave",
            ));
        }
        if s.preview_lod > 8 {
            return Err(JdError::invalid("params.preview_lod", "must be ≤ 8"));
        }
        if s.cache_limit_bytes < 16 * 1024 * 1024 {
            return Err(JdError::invalid(
                "params.cache_limit_bytes",
                "must be ≥ 16 MiB (one 2048² layer)",
            ));
        }
        if s.memory_budget_bytes < 256 * 1024 * 1024 {
            return Err(JdError::invalid(
                "params.memory_budget_bytes",
                "must be ≥ 256 MiB",
            ));
        }
        Ok(())
    }

    /// Replace the settings (a mutating op; callers validate and journal first via [`Self::journal`]).
    pub fn set_settings(&mut self, s: ProjectSettings) -> JdResult<()> {
        Self::validate_settings(&s)?;
        self.doc.settings = s;
        self.dirty = true;
        Ok(())
    }

    /// Rename the project (a mutating op; callers validate and journal first via [`Self::journal`]).
    pub fn set_name(&mut self, name: &str) -> JdResult<()> {
        Self::validate_name(name)?;
        self.doc.name = name.to_owned();
        self.dirty = true;
        Ok(())
    }

    /// Write all documents atomically and truncate the journal.
    ///
    /// Refuses while crash-recovery entries are pending: saving would silently discard them.
    pub fn save(&mut self) -> JdResult<()> {
        if !self.pending_journal.is_empty() {
            return Err(JdError::new(ErrorCode::E_INPUT_INVALID, "project has unreplayed journal entries")
                .why("Saving now would discard edits recorded before a crash.")
                .next("Run project.recover to replay them (then save), or delete journal/ops.ndjson to discard them deliberately.")
                .detail("pending", self.pending_journal.len() as u64));
        }
        self.doc.modified = now_rfc3339();
        self.doc.app_version = crate::APP_VERSION.to_owned();
        write_atomic(&self.root.join(PROJECT_FILE), &to_pretty(&self.doc)?)?;
        write_atomic(
            &self.root.join(&self.doc.files.graph),
            &to_pretty(&self.graph)?,
        )?;
        write_atomic(
            &self.root.join(&self.doc.files.scene),
            &to_pretty(&self.scene)?,
        )?;
        let journal = self.root.join(JOURNAL_FILE);
        if journal.exists() {
            write_atomic(&journal, b"")?;
        }
        self.pending_journal.clear();
        self.dirty = false;
        Ok(())
    }

    /// Take the pending journal entries (for replay by the op registry).
    pub fn take_pending_journal(&mut self) -> Vec<JournalEntry> {
        std::mem::take(&mut self.pending_journal)
    }

    fn acquire_lock(&mut self) -> JdResult<Option<Warning>> {
        let lock = self.root.join(LOCK_FILE);
        let mut warning = None;
        if let Ok(existing) = fs::read_to_string(&lock) {
            let holder: Option<LockInfo> = serde_json::from_str(&existing).ok();
            let me = hostname();
            match holder {
                // A lock from another machine cannot be probed; treat it as held.
                Some(h) if h.host != me => {
                    return Err(JdError::new(ErrorCode::E_PERMISSION, "project is locked from another host")
                        .why("The lock holder cannot be checked from this machine; two writers would corrupt the project.")
                        .next("Close Jenga Dunia on the other machine, or delete `.lock` if you are sure it is stale.")
                        .detail("pid", h.pid)
                        .detail("host", h.host));
                }
                Some(h) if h.pid != std::process::id() && process_alive(h.pid) => {
                    return Err(JdError::new(ErrorCode::E_PERMISSION, "project is locked by another process")
                        .why("Two writers would corrupt the project.")
                        .next("Close the other Jenga Dunia instance, or delete `.lock` if you are sure it is stale.")
                        .detail("pid", h.pid)
                        .detail("host", h.host));
                }
                Some(h) if h.pid != std::process::id() => {
                    warning = Some(
                        Warning::new(
                            "W_STALE_LOCK",
                            format!("stale lock from pid {} overridden", h.pid),
                        )
                        .with_details(serde_json::json!({ "pid": h.pid })),
                    );
                }
                _ => {}
            }
        }
        // A per-instance nonce makes lock ownership precise: dropping one Project never removes a
        // lock that a newer instance (same pid) now holds.
        self.lock_nonce = ulid_nonce();
        let info = LockInfo {
            pid: std::process::id(),
            host: hostname(),
            ts: now_rfc3339(),
            nonce: self.lock_nonce.clone(),
        };
        write_atomic(&lock, &serde_json::to_vec(&info)?)?;
        Ok(warning)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let lock = self.root.join(LOCK_FILE);
        if let Ok(s) = fs::read_to_string(&lock)
            && let Ok(info) = serde_json::from_str::<LockInfo>(&s)
            && info.nonce == self.lock_nonce
        {
            let _ = fs::remove_file(&lock);
        }
    }
}

fn ulid_nonce() -> String {
    jd_types::RequestId::new().to_string()
}

#[derive(Serialize, Deserialize)]
struct LockInfo {
    pid: u32,
    host: String,
    ts: String,
    #[serde(default)]
    nonce: String,
}

fn hostname() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "unknown".into())
}

fn process_alive(pid: u32) -> bool {
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).is_some()
}

fn read_journal(path: &Path) -> JdResult<Vec<JournalEntry>> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<JournalEntry>(line) {
            Ok(e) => entries.push(e),
            Err(e) => {
                // A torn final line is expected after a crash mid-write: stop there, keep what parsed.
                tracing::warn!(line = i + 1, error = %e, "journal line unreadable; truncating recovery here");
                break;
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn history_trims_the_oldest_snapshots_past_the_entry_and_byte_caps() {
        let mut h = History::default();
        for _ in 0..5 {
            h.record(GraphDocument::default());
        }
        let each = History::size_of(&GraphDocument::default());
        assert!(each > 0 && h.undo_bytes() == 5 * each);
        h.trim_to(100, each * 3); // bytes bind: three fit
        assert_eq!(h.depths().0, 3);
        h.trim_to(2, usize::MAX); // entries bind
        assert_eq!(h.depths().0, 2);
        h.trim_to(100, 1); // the newest always stays
        assert_eq!(h.depths().0, 1);
        assert!(
            h.undo(GraphDocument::default()).is_some()
                && h.undo(GraphDocument::default()).is_none()
        );
        assert_eq!(h.depths(), (0, 1));
    }

    #[test]
    fn create_then_open_round_trips() {
        let dir = tmp();
        let root = dir.path().join("World.jdproj");
        let created = Project::create(&root, "World", WorldSettings::default()).expect("create");
        let id = created.doc.id.clone();
        drop(created);
        let (opened, warnings) = Project::open(&root).expect("open");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(opened.doc.id, id);
        assert_eq!(opened.doc.name, "World");
        assert_eq!(opened.graph, jd_graph::nodes::starter());
        assert!(root.join("cache/.gitignore").exists());
    }

    #[test]
    fn create_refuses_non_empty_dir_and_bad_input() {
        let dir = tmp();
        fs::write(dir.path().join("x"), b"x").expect("write");
        let err =
            Project::create(dir.path(), "W", WorldSettings::default()).expect_err("non-empty");
        assert_eq!(err.code, ErrorCode::E_INPUT_INVALID);
        let err = Project::create(&dir.path().join("a"), "  ", WorldSettings::default())
            .expect_err("empty name");
        assert_eq!(err.code, ErrorCode::E_INPUT_INVALID);
        let bad = WorldSettings {
            tile_size: 100,
            ..Default::default()
        };
        let err = Project::create(&dir.path().join("b"), "W", bad).expect_err("bad world");
        assert_eq!(err.code, ErrorCode::E_INPUT_INVALID);
    }

    #[test]
    fn unknown_fields_are_preserved_and_reported() {
        let dir = tmp();
        let root = dir.path().join("p.jdproj");
        drop(Project::create(&root, "P", WorldSettings::default()).expect("create"));
        let path = root.join(PROJECT_FILE);
        let mut v: Value = serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        v["future_field"] = serde_json::json!({ "from": "v99" });
        fs::write(&path, serde_json::to_vec_pretty(&v).expect("ser")).expect("write");
        let (mut p, warnings) = Project::open(&root).expect("open");
        assert!(warnings.iter().any(|w| w.code == "W_UNKNOWN_FIELDS"));
        p.save().expect("save");
        let v: Value = serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        assert_eq!(v["future_field"]["from"], "v99");
    }

    #[test]
    fn newer_schema_is_rejected_with_stable_code() {
        let dir = tmp();
        let root = dir.path().join("p.jdproj");
        drop(Project::create(&root, "P", WorldSettings::default()).expect("create"));
        let path = root.join(PROJECT_FILE);
        let mut v: Value = serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        v["schema_version"] = serde_json::json!(99);
        fs::write(&path, serde_json::to_vec(&v).expect("ser")).expect("write");
        let err = Project::open(&root).expect_err("newer schema");
        assert_eq!(err.code, ErrorCode::E_SCHEMA_VERSION);
        assert_eq!(err.details["found"], 99);
    }

    #[test]
    fn corrupt_document_reports_project_corrupt() {
        let dir = tmp();
        let root = dir.path().join("p.jdproj");
        drop(Project::create(&root, "P", WorldSettings::default()).expect("create"));
        fs::write(root.join(GRAPH_FILE), b"{ not json").expect("write");
        let err = Project::open(&root).expect_err("corrupt");
        assert_eq!(err.code, ErrorCode::E_PROJECT_CORRUPT);
    }

    #[test]
    fn journal_survives_crash_and_is_truncated_by_save() {
        let dir = tmp();
        let root = dir.path().join("p.jdproj");
        {
            let mut p = Project::create(&root, "P", WorldSettings::default()).expect("create");
            p.journal(
                "project.set_name",
                &serde_json::json!({ "name": "Renamed" }),
            )
            .expect("journal");
            p.set_name("Renamed").expect("rename");
            // Simulate a crash: drop without save (lock is released by Drop; documents stay old).
        }
        // Torn trailing line must not break recovery.
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(root.join(JOURNAL_FILE))
            .expect("open journal");
        f.write_all(b"{\"ts\":\"x\",\"op\":\"proj")
            .expect("torn write");
        drop(f);
        let (mut p, warnings) = Project::open(&root).expect("open");
        assert_eq!(p.doc.name, "P", "unsaved rename must not be on disk");
        assert!(warnings.iter().any(|w| w.code == "W_RECOVERY_AVAILABLE"));
        assert_eq!(p.pending_journal.len(), 1);
        assert_eq!(p.pending_journal[0].op, "project.set_name");
        assert_eq!(
            p.save()
                .expect_err("save must not discard pending entries")
                .code,
            ErrorCode::E_INPUT_INVALID
        );
        let replay = p.take_pending_journal();
        assert_eq!(replay.len(), 1);
        p.save().expect("save after recovery");
        assert_eq!(
            fs::read_to_string(root.join(JOURNAL_FILE)).expect("read"),
            ""
        );
        assert!(p.pending_journal.is_empty());
    }

    #[test]
    fn sub_document_paths_cannot_escape_the_project() {
        let dir = tmp();
        let root = dir.path().join("p.jdproj");
        drop(Project::create(&root, "P", WorldSettings::default()).expect("create"));
        let outside = dir.path().join("victim.json");
        fs::write(&outside, b"{\"schema_version\":1}").expect("victim");
        let path = root.join(PROJECT_FILE);
        for bad in [
            outside.display().to_string(),
            "../victim.json".to_owned(),
            "".to_owned(),
        ] {
            let mut v: Value =
                serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
            v["files"]["graph"] = Value::String(bad.clone());
            fs::write(&path, serde_json::to_vec(&v).expect("ser")).expect("write");
            let err = Project::open(&root).expect_err("escaping path");
            assert_eq!(err.code, ErrorCode::E_PROJECT_CORRUPT, "{bad}");
        }
        assert_eq!(
            fs::read(&outside).expect("still there"),
            b"{\"schema_version\":1}"
        );
    }

    #[test]
    fn save_refuses_while_recovery_entries_are_pending() {
        let dir = tmp();
        let root = dir.path().join("p.jdproj");
        {
            let mut p = Project::create(&root, "P", WorldSettings::default()).expect("create");
            p.journal("project.set_name", &serde_json::json!({ "name": "X" }))
                .expect("journal");
        }
        let (mut p, _) = Project::open(&root).expect("open");
        let err = p.save().expect_err("must not discard");
        assert_eq!(err.code, ErrorCode::E_INPUT_INVALID);
        assert_eq!(p.take_pending_journal().len(), 1);
        p.save().expect("after taking the journal, save proceeds");
    }

    #[test]
    fn dropping_an_older_instance_keeps_the_newer_lock() {
        let dir = tmp();
        let root = dir.path().join("p.jdproj");
        let p = Project::create(&root, "P", WorldSettings::default()).expect("create");
        let (_q, _) = Project::open(&root).expect("same pid reopen");
        drop(p);
        assert!(
            root.join(LOCK_FILE).exists(),
            "newer instance's lock must survive"
        );
    }

    #[test]
    fn create_on_an_existing_file_is_invalid_input() {
        let dir = tmp();
        let f = dir.path().join("notes.txt");
        fs::write(&f, b"x").expect("write");
        let err = Project::create(&f, "P", WorldSettings::default()).expect_err("file path");
        assert_eq!(err.code, ErrorCode::E_INPUT_INVALID);
    }

    #[test]
    fn foreign_host_lock_is_treated_as_held() {
        let dir = tmp();
        let root = dir.path().join("p.jdproj");
        drop(Project::create(&root, "P", WorldSettings::default()).expect("create"));
        fs::write(
            root.join(LOCK_FILE),
            serde_json::json!({"pid": 1, "host": "other-machine", "ts": "t", "nonce": "n"})
                .to_string(),
        )
        .expect("lock");
        let err = Project::open(&root).expect_err("foreign lock");
        assert_eq!(err.code, ErrorCode::E_PERMISSION);
    }

    #[test]
    fn lock_blocks_second_writer_in_same_process_tree_only_when_alive() {
        let dir = tmp();
        let root = dir.path().join("p.jdproj");
        let p = Project::create(&root, "P", WorldSettings::default()).expect("create");
        // Same pid → allowed (re-entrant); a dead pid → stale warning.
        let (q, warnings) = Project::open(&root).expect("same pid reopen");
        assert!(warnings.is_empty());
        drop(p);
        drop(q);
        fs::write(
            root.join(LOCK_FILE),
            serde_json::json!({"pid": 4_000_000_000u32, "host": hostname(), "ts": "t", "nonce": "n"}).to_string(),
        )
        .expect("write lock");
        let (_r, warnings) = Project::open(&root).expect("stale lock");
        assert!(warnings.iter().any(|w| w.code == "W_STALE_LOCK"));
    }
}
