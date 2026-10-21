//! Incremental, content-hashed evaluation.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use jd_terrain::Heightfield;
use jd_types::{ErrorCode, JdError, JdResult, NodeId, WorldSettings};
use serde::{Deserialize, Serialize};

use crate::model::Graph;
use crate::registry::{EvalCtx, NodeDef, Registry};

/// A value flowing through the graph. The content hash is computed once when the value is
/// produced so cache keys never re-read whole fields.
#[derive(Debug, Clone)]
pub enum Value {
    /// Heightfield in metres (masks are heightfields in 0..1), its content hash and the per-tile
    /// hashes the content hash is built from (the tile-level dirty set reads them for free).
    Field(Arc<Heightfield>, blake3::Hash, Arc<Vec<blake3::Hash>>),
}

impl Value {
    /// Wrap a field, hashing it once (per tile, then the tile hashes together — the same
    /// construction as [`Heightfield::content_hash`]).
    pub fn field(hf: Heightfield) -> Self {
        let tiles: Vec<blake3::Hash> = hf.tiles().iter().map(|t| t.content_hash()).collect();
        let mut h = blake3::Hasher::new();
        for t in &tiles {
            h.update(t.as_bytes());
        }
        Value::Field(Arc::new(hf), h.finalize(), Arc::new(tiles))
    }
    /// Borrow the field.
    pub fn as_field(&self) -> &Arc<Heightfield> {
        match self {
            Value::Field(f, _, _) => f,
        }
    }
    /// Content hash for caching.
    pub fn content_hash(&self) -> blake3::Hash {
        match self {
            Value::Field(_, h, _) => *h,
        }
    }
    /// Wrap a shared field (no copy), hashing it once.
    pub fn from_arc(hf: Arc<Heightfield>) -> Self {
        let tiles: Vec<blake3::Hash> = hf.tiles().iter().map(|t| t.content_hash()).collect();
        let mut h = blake3::Hasher::new();
        for t in &tiles {
            h.update(t.as_bytes());
        }
        Value::Field(hf, h.finalize(), Arc::new(tiles))
    }
    /// A field that never enters a cache (a crop handed to a kernel): no hashing.
    fn untracked(hf: Heightfield) -> Self {
        Value::Field(
            Arc::new(hf),
            blake3::Hash::from_bytes([0; 32]),
            Arc::new(Vec::new()),
        )
    }
    /// Per-tile content hashes, row-major like [`Heightfield::tiles`].
    pub fn tile_hashes(&self) -> &Arc<Vec<blake3::Hash>> {
        match self {
            Value::Field(_, _, t) => t,
        }
    }
}

/// Cached outputs of one node, keyed by output port.
type NodeOutputs = BTreeMap<String, Value>;

/// In-memory cache of node outputs keyed by the node's input hash.
/// ponytail: memory-only with a count cap; the content-addressed disk cache (`jd-cache`) lands with jobs.
#[derive(Default, Clone)]
pub struct Cache {
    inner: Arc<Mutex<HashMap<[u8; 32], NodeOutputs>>>,
    cap: usize,
    /// What each local node last saw and produced, per (node, lod): the tile-level dirty set is
    /// the tiles whose input hashes differ from this.
    memo: Arc<Mutex<HashMap<(NodeId, u8), Memo>>>,
}

/// A local node's last run: its params key, its inputs' per-tile hashes and its outputs.
#[derive(Clone)]
struct Memo {
    params_key: [u8; 32],
    tiles: BTreeMap<String, Arc<Vec<blake3::Hash>>>,
    dims: BTreeMap<String, (u32, u32, u32, u32)>,
    outputs: NodeOutputs,
}

impl Cache {
    /// A cache holding at most `cap` node results (0 = unbounded).
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Arc::default(),
            cap,
            memo: Arc::default(),
        }
    }
    fn get(&self, key: &[u8; 32]) -> Option<NodeOutputs> {
        self.inner.lock().ok()?.get(key).cloned()
    }
    fn put(&self, key: [u8; 32], v: NodeOutputs) {
        if let Ok(mut g) = self.inner.lock() {
            if self.cap > 0 && g.len() >= self.cap {
                g.clear(); // ponytail: wholesale eviction; LRU when the disk cache arrives
                if let Ok(mut m) = self.memo.lock() {
                    m.clear(); // the memo pins whole outputs: it never outlives the results
                }
            }
            g.insert(key, v);
        }
    }
    fn memo_get(&self, id: &NodeId, lod: u8) -> Option<Memo> {
        self.memo.lock().ok()?.get(&(id.clone(), lod)).cloned()
    }
    fn memo_put(&self, id: &NodeId, lod: u8, m: Memo) {
        if let Ok(mut g) = self.memo.lock() {
            // Bounded by bytes, not entries: every memo pins a whole output per port.
            let bytes = |m: &Memo| -> u64 {
                m.outputs
                    .values()
                    .map(|v| v.as_field().width() as u64 * v.as_field().height() as u64 * 4)
                    .sum()
            };
            let held: u64 = g.values().map(bytes).sum();
            if held + bytes(&m) > MEMO_BYTES_CAP {
                g.clear(); // ponytail: wholesale, like the result cache
            }
            g.insert((id.clone(), lod), m);
        }
    }
}

/// Re-run a local node on the tiles whose inputs changed since its last run (plus the stencil
/// halo) and splice the result into its previous outputs. `None` when there is no usable
/// memo, the inputs changed shape, or more than half the tiles moved — the caller then runs
/// the node whole. Bit-identical to the whole run by the stencil argument: an output cell at
/// distance ≤ halo from a changed input cell is inside the kept region, and the kept region
/// sits a further halo inside the computed one, so every kept cell saw exactly the inputs it
/// would have seen.
fn partial_evaluate(
    def: &NodeDef,
    ctx: &EvalCtx,
    params: &serde_json::Map<String, serde_json::Value>,
    inputs: &NodeOutputs,
    memo: &Memo,
    halo: u32,
) -> JdResult<Option<(NodeOutputs, u32)>> {
    if memo.tiles.len() != inputs.len() {
        return Ok(None);
    }
    let mut dirty: Vec<bool> = Vec::new();
    let (mut w, mut h, mut ts, mut fh) = (0u32, 0u32, 0u32, 0u32);
    for (port, v) in inputs {
        let f = v.as_field();
        let (Some(old), Some(dims)) = (memo.tiles.get(port), memo.dims.get(port)) else {
            return Ok(None);
        };
        if *dims != (f.width(), f.height(), f.tile_size(), f.halo()) {
            return Ok(None);
        }
        (w, h, ts, fh) = *dims;
        let now = v.tile_hashes();
        if now.len() != old.len() {
            return Ok(None);
        }
        if dirty.is_empty() {
            dirty = vec![false; now.len()];
        }
        for (d, (a, b)) in dirty.iter_mut().zip(now.iter().zip(old.iter())) {
            *d |= a != b;
        }
    }
    let n_dirty = dirty.iter().filter(|d| **d).count();
    if n_dirty == 0 {
        return Ok(Some((memo.outputs.clone(), 0)));
    }
    // Bounding box of the dirty tiles in cells, then the kept and computed regions.
    let tx = w.div_ceil(ts);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (i, d) in dirty.iter().enumerate() {
        if *d {
            let (cx, cy) = ((i as u32 % tx) * ts, (i as u32 / tx) * ts);
            x0 = x0.min(cx);
            y0 = y0.min(cy);
            x1 = x1.max((cx + ts).min(w));
            y1 = y1.max((cy + ts).min(h));
        }
    }
    let keep = (
        x0.saturating_sub(halo),
        y0.saturating_sub(halo),
        (x1 + halo).min(w),
        (y1 + halo).min(h),
    );
    let comp = (
        keep.0.saturating_sub(halo),
        keep.1.saturating_sub(halo),
        (keep.2 + halo).min(w),
        (keep.3 + halo).min(h),
    );
    let (cw, ch) = (comp.2 - comp.0, comp.3 - comp.1);
    // Two strokes at opposite corners span the whole field: then the whole run is cheaper.
    if (cw as u64) * (ch as u64) * 2 > (w as u64) * (h as u64) {
        return Ok(None);
    }
    let mut cropped: NodeOutputs = BTreeMap::new();
    for (port, v) in inputs {
        let f = v.as_field();
        let mut sub = Heightfield::new(cw, ch, ts, fh)?;
        sub.fill_with(|x, y| f.get(comp.0 + x, comp.1 + y));
        cropped.insert(port.clone(), Value::untracked(sub));
    }
    let outs = def.evaluate(ctx, params, &cropped)?;
    let mut spliced: NodeOutputs = BTreeMap::new();
    for (port, v) in outs {
        let Some(prev) = memo.outputs.get(&port) else {
            return Ok(None);
        };
        let (part, prev) = (v.as_field(), prev.as_field());
        if (prev.width(), prev.height()) != (w, h) || (part.width(), part.height()) != (cw, ch) {
            return Ok(None);
        }
        let mut full: Heightfield = (**prev).clone();
        for y in keep.1..keep.3 {
            for x in keep.0..keep.2 {
                full.set(x, y, part.get(x - comp.0, y - comp.1));
            }
        }
        full.rebuild_halos();
        spliced.insert(port, Value::field(full));
    }
    Ok(Some((spliced, n_dirty as u32)))
}

/// The tile memo holds at most this many bytes of pinned outputs (1 GiB).
const MEMO_BYTES_CAP: u64 = 1 << 30;

/// Per-node evaluation report.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NodeReport {
    /// Node id.
    pub node: NodeId,
    /// Kind.
    pub kind: String,
    /// Input hash (hex) — the cache key.
    pub hash: String,
    /// Served from cache?
    pub cached: bool,
    /// Milliseconds spent (0 when cached).
    pub duration_ms: u64,
    /// For a local node re-run on its changed tiles only: how many tiles were recomputed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_tiles: Option<u32>,
}

/// Whole-graph evaluation report.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EvalReport {
    /// Level of detail evaluated.
    pub lod: u8,
    /// Reports in evaluation order.
    pub nodes: Vec<NodeReport>,
    /// Total milliseconds.
    pub duration_ms: u64,
}

/// Progress callback: (nodes done, nodes total, kind about to run).
pub type ProgressFn = Arc<dyn Fn(usize, usize, &str) + Send + Sync>;

/// Cancellation flag and progress sink for one evaluation (`docs/architecture/06-jobs.md`).
#[derive(Clone)]
pub struct JobCtl {
    /// Set to cancel; checked before every node and inside long kernels.
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Called before each node.
    pub progress: ProgressFn,
}

impl Default for JobCtl {
    fn default() -> Self {
        Self {
            cancel: Arc::default(),
            progress: Arc::new(|_, _, _| {}),
        }
    }
}

/// Evaluates graphs against a registry and cache.
pub struct Evaluator<'a> {
    registry: &'a Registry,
    cache: &'a Cache,
    layers: Option<crate::registry::LayerLoader>,
    only: Option<Vec<String>>,
}

/// Values produced by every node: `(node id, port) → value`.
pub type Outputs = BTreeMap<(NodeId, String), Value>;

impl<'a> Evaluator<'a> {
    /// Build an evaluator.
    pub fn new(registry: &'a Registry, cache: &'a Cache) -> Self {
        Self {
            registry,
            cache,
            layers: None,
            only: None,
        }
    }

    /// Give `jd.input.layer@1` nodes a way to read the project's published layers.
    pub fn with_layers(mut self, layers: crate::registry::LayerLoader) -> Self {
        self.layers = Some(layers);
        self
    }

    /// Evaluate only the output nodes publishing these layer names (and their ancestors).
    pub fn only_outputs(mut self, names: Option<Vec<String>>) -> Self {
        self.only = names.filter(|n| !n.is_empty());
        self
    }

    /// The nodes an `only_outputs` selection needs: the named output nodes and everything that
    /// feeds them (the gate pattern: a coarse chain evaluated at a high lod, the fine chain at
    /// lod 0, in one graph). `None` = every node.
    fn selected(&self, graph: &Graph) -> JdResult<Option<BTreeSet<NodeId>>> {
        let Some(names) = &self.only else {
            return Ok(None);
        };
        let input_map = graph.input_map();
        let mut needed: BTreeSet<NodeId> = BTreeSet::new();
        let mut stack: Vec<NodeId> = graph
            .nodes
            .iter()
            .filter(|(_, n)| {
                n.kind.0.starts_with("jd.output.height@")
                    && names.contains(&crate::nodes::output_layer_name(&n.params))
            })
            .map(|(id, _)| id.clone())
            .collect();
        if stack.is_empty() {
            return Err(JdError::new(
                ErrorCode::E_INPUT_INVALID,
                format!("no output node publishes any of {names:?}"),
            )
            .next("Name layers that an output node publishes, or leave `outputs` out."));
        }
        while let Some(id) = stack.pop() {
            if !needed.insert(id.clone()) {
                continue;
            }
            if let Some(ins) = input_map.get(&id) {
                stack.extend(ins.values().map(|p| p.node.clone()));
            }
        }
        Ok(Some(needed))
    }

    /// Every structural problem in the graph (none for a valid one): unknown kinds, params that do
    /// not match the kind, dangling edges, unknown ports, port type mismatches, cycles. Does **not**
    /// require inputs to be connected (a graph under construction is structurally valid). Returning
    /// all problems lets a mutation on an already-broken document be judged by what it *adds*.
    pub fn structure_errors(&self, graph: &Graph) -> Vec<JdError> {
        let mut errs = Vec::new();
        for e in graph.dangling_edges() {
            errs.push(
                JdError::new(
                    ErrorCode::E_INPUT_INVALID,
                    format!(
                        "edge references a missing node ({} → {})",
                        e.from.node, e.to.node
                    ),
                )
                .next("Remove the edge or restore the node.")
                .detail("from", e.from.node.to_string())
                .detail("to", e.to.node.to_string()),
            );
        }
        if let Err(e) = graph.topological_order() {
            errs.push(e);
        }
        let inputs = graph.input_map();
        for (id, node) in &graph.nodes {
            let def = match self.registry.get(&node.kind) {
                Ok(d) => d,
                Err(e) => {
                    errs.push(e.detail("node", id.to_string()));
                    continue;
                }
            };
            // Keyed parameters must be valid at every keyframe, not just where the timeline sits.
            for t in std::iter::once(graph.time).chain(crate::model::key_times(&node.params)) {
                if let Err(e) = def.check_params(&crate::model::resolve_keys(&node.params, t)) {
                    errs.push(
                        e.detail("node", id.to_string())
                            .detail("time", t.to_string()),
                    );
                    break;
                }
            }
            for (name, from) in inputs.get(id).into_iter().flatten() {
                let Some(port) = def.input(name) else {
                    errs.push(
                        JdError::new(
                            ErrorCode::E_INPUT_INVALID,
                            format!("node {} ({}) has no input `{}`", id, node.kind.0, name),
                        )
                        .detail("node", id.to_string())
                        .detail("port", (*name).to_owned()),
                    );
                    continue;
                };
                // Missing source node / unknown source kind are already reported above.
                let Some(src) = graph.nodes.get(&from.node) else {
                    continue;
                };
                let Ok(src_def) = self.registry.get(&src.kind) else {
                    continue;
                };
                let Some(out) = src_def.output(&from.port) else {
                    errs.push(
                        JdError::new(
                            ErrorCode::E_INPUT_INVALID,
                            format!(
                                "node {} ({}) has no output `{}`",
                                from.node, src.kind.0, from.port
                            ),
                        )
                        .detail("node", from.node.to_string())
                        .detail("port", from.port.clone()),
                    );
                    continue;
                };
                if !port.ty.accepts(out.ty) {
                    errs.push(
                        JdError::new(
                            ErrorCode::E_GRAPH_TYPE_MISMATCH,
                            format!(
                                "cannot connect {:?} output `{}` to {:?} input `{}`",
                                out.ty, from.port, port.ty, port.name
                            ),
                        )
                        .next("Insert a conversion node or pick a compatible port.")
                        .detail("from", from.node.to_string())
                        .detail("to", id.to_string()),
                    );
                }
            }
        }
        errs
    }

    /// Structural validation — what every mutation must preserve. Fails with the first problem
    /// from [`Evaluator::structure_errors`]; returns the topological order when sound.
    pub fn validate_structure(&self, graph: &Graph) -> JdResult<Vec<NodeId>> {
        if let Some(e) = self.structure_errors(graph).into_iter().next() {
            return Err(e);
        }
        graph.topological_order()
    }

    /// Full validation: structure plus every required input connected (unless bypassed).
    pub fn validate(&self, graph: &Graph) -> JdResult<Vec<NodeId>> {
        let order = self.validate_structure(graph)?;
        let inputs = graph.input_map();
        for (id, node) in &graph.nodes {
            let def = self.registry.get(&node.kind)?;
            // A bypassed node forwards its first input, so that one is required even if optional.
            let required: Vec<&crate::model::Port> = if node.bypass {
                def.inputs.first().into_iter().collect()
            } else {
                def.inputs.iter().filter(|p| p.required).collect()
            };
            for port in required {
                if inputs
                    .get(id)
                    .and_then(|m| m.get(port.name.as_str()))
                    .is_none()
                {
                    return Err(JdError::new(
                        ErrorCode::E_GRAPH_MISSING_INPUT,
                        format!(
                            "input `{}` of {} ({}) is not connected",
                            port.name, id, node.kind.0
                        ),
                    )
                    .next("Connect an output of a compatible type to this input.")
                    .detail("node", id.to_string())
                    .detail("port", port.name.clone()));
                }
            }
        }
        Ok(order)
    }

    /// Evaluate every node at `lod`; returns outputs of all nodes plus the report.
    pub fn evaluate(
        &self,
        graph: &Graph,
        world: &WorldSettings,
        lod: u8,
    ) -> JdResult<(Outputs, EvalReport)> {
        self.evaluate_with(graph, world, lod, None)
    }

    /// [`Evaluator::evaluate`] inside a project: file-reading nodes resolve paths in `assets_dir`.
    /// ponytail: node hashes do not include asset file contents, so a changed file is picked up
    /// after a param change or a new session; a content hash per asset comes with `jd-io`.
    pub fn evaluate_with(
        &self,
        graph: &Graph,
        world: &WorldSettings,
        lod: u8,
        assets_dir: Option<&std::path::Path>,
    ) -> JdResult<(Outputs, EvalReport)> {
        self.evaluate_job(graph, world, lod, assets_dir, &JobCtl::default())
    }

    /// Pre-flight memory estimate for evaluating `graph` at `lod`: every node's outputs stay
    /// alive until consumed, so the sum of all output fields is the floor; the largest kernel
    /// scratch sits on top. Conservative by design — it is a gate, not a profiler.
    pub fn estimate_bytes(&self, graph: &Graph, world: &WorldSettings, lod: u8) -> JdResult<u64> {
        let lw = lod_world(world, lod);
        let cells = lw.resolution_px[0] as u64 * lw.resolution_px[1] as u64;
        let selected = self.selected(graph)?;
        let mut outputs = 0u64;
        let mut scratch = 0u64;
        for (id, node) in &graph.nodes {
            if selected.as_ref().is_some_and(|s| !s.contains(id)) {
                continue;
            }
            let def = self.registry.get(&node.kind)?;
            let out = def.output_bytes(cells);
            outputs += out;
            scratch = scratch.max(def.estimate_bytes(cells, &node.params) - out);
        }
        Ok(outputs + scratch)
    }

    /// [`Evaluator::evaluate_with`] under a job: progress per node, cancellable between nodes
    /// and inside long kernels (`E_CANCELLED`, nothing published).
    pub fn evaluate_job(
        &self,
        graph: &Graph,
        world: &WorldSettings,
        lod: u8,
        assets_dir: Option<&std::path::Path>,
        job: &JobCtl,
    ) -> JdResult<(Outputs, EvalReport)> {
        let started = Instant::now();
        let order = self.validate(graph)?;
        let ctx = EvalCtx {
            lod_world: lod_world(world, lod),
            assets_dir: assets_dir.map(|p| p.to_path_buf()),
            cancel: job.cancel.clone(),
            layers: self.layers.clone(),
        };
        let order = match self.selected(graph)? {
            None => order,
            Some(needed) => order.into_iter().filter(|id| needed.contains(id)).collect(),
        };
        let lod_world_bytes = serde_json::to_vec(&ctx.lod_world)?;
        let input_map = graph.input_map();
        let mut produced: BTreeMap<NodeId, NodeOutputs> = BTreeMap::new();
        let mut reports = Vec::with_capacity(order.len());
        for (done, id) in order.iter().enumerate() {
            let node = &graph.nodes[id];
            if job.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(JdError::new(ErrorCode::E_CANCELLED, "evaluation cancelled")
                    .why("The job's cancel flag was set before this node ran.")
                    .next("Nothing was published; re-run when ready.")
                    .detail("node", id.to_string()));
            }
            (job.progress)(done, order.len(), &node.kind.0);
            let def = self.registry.get(&node.kind)?;
            // Keyframes resolve at the graph's timeline position; the hash sees the resolved values,
            // so every time position caches on its own.
            let params = crate::model::resolve_keys(&node.params, graph.time);
            let mut inputs: NodeOutputs = BTreeMap::new();
            let mut hasher = blake3::Hasher::new();
            hasher.update(node.kind.0.as_bytes());
            hasher.update(&serde_json::to_vec(&params)?);
            hasher.update(&[node.bypass as u8, lod]);
            hasher.update(&lod_world_bytes);
            // A layer read from the project is part of the key through its content hash, so a
            // re-evaluated coarse chain invalidates the fine one that reads it.
            if node.kind.0 == "jd.input.layer@1"
                && let Some(name) = params.get("name").and_then(|v| v.as_str())
                && let Some(load) = &ctx.layers
                && let Some(hf) = load(name)?
            {
                hasher.update(hf.content_hash().as_bytes());
            }
            // Results computed inside a project may depend on its assets: never share them
            // across projects (or with project-less evaluations).
            if let Some(dir) = &ctx.assets_dir {
                hasher.update(dir.to_string_lossy().as_bytes());
                // File-reading nodes: a re-imported asset (new size/mtime) invalidates the result.
                if node.kind.0.starts_with("jd.input.")
                    && let Some(p) = node.params.get("path").and_then(|v| v.as_str())
                    && let Ok(meta) = std::fs::metadata(dir.join(p))
                {
                    hasher.update(&meta.len().to_le_bytes());
                    if let Ok(m) = meta.modified()
                        && let Ok(d) = m.duration_since(std::time::UNIX_EPOCH)
                    {
                        hasher.update(&d.as_nanos().to_le_bytes());
                    }
                }
            }
            let params_key: [u8; 32] = *hasher.clone().finalize().as_bytes();
            for port in &def.inputs {
                if let Some(from) = input_map.get(id).and_then(|m| m.get(port.name.as_str())) {
                    let v = produced
                        .get(&from.node)
                        .and_then(|outs| outs.get(&from.port))
                        .ok_or_else(|| {
                            JdError::new(
                                ErrorCode::E_INTERNAL,
                                format!("upstream {}.{} missing", from.node, from.port),
                            )
                        })?;
                    hasher.update(port.name.as_bytes());
                    hasher.update(v.content_hash().as_bytes());
                    inputs.insert(port.name.clone(), v.clone());
                }
            }
            let key: [u8; 32] = *hasher.finalize().as_bytes();
            let t0 = Instant::now();
            let local = if node.bypass {
                None
            } else {
                def.local_halo(&params)
            };
            let mut partial_tiles = None;
            let (outs, cached) = match self.cache.get(&key) {
                Some(outs) => (outs, true),
                None => {
                    // A local node whose inputs changed in a few tiles only re-runs on those.
                    let partial = match (local, self.cache.memo_get(id, lod)) {
                        (Some(halo), Some(memo)) if memo.params_key == params_key => {
                            partial_evaluate(def, &ctx, &params, &inputs, &memo, halo).map_err(
                                |e| {
                                    e.detail("node", id.to_string())
                                        .detail("kind", node.kind.0.clone())
                                },
                            )?
                        }
                        _ => None,
                    };
                    let outs = match partial {
                        Some((outs, n)) => {
                            partial_tiles = Some(n);
                            outs
                        }
                        None => if node.bypass {
                            bypass(def, &inputs)
                        } else {
                            def.evaluate(&ctx, &params, &inputs)
                        }
                        .map_err(|e| {
                            e.detail("node", id.to_string())
                                .detail("kind", node.kind.0.clone())
                        })?,
                    };
                    if local.is_some() {
                        self.cache.memo_put(
                            id,
                            lod,
                            Memo {
                                params_key,
                                tiles: inputs
                                    .iter()
                                    .map(|(p, v)| (p.clone(), v.tile_hashes().clone()))
                                    .collect(),
                                dims: inputs
                                    .iter()
                                    .map(|(p, v)| {
                                        let f = v.as_field();
                                        (
                                            p.clone(),
                                            (f.width(), f.height(), f.tile_size(), f.halo()),
                                        )
                                    })
                                    .collect(),
                                outputs: outs.clone(),
                            },
                        );
                    }
                    self.cache.put(key, outs.clone());
                    (outs, false)
                }
            };
            reports.push(NodeReport {
                node: id.clone(),
                kind: node.kind.0.clone(),
                hash: blake3::Hash::from_bytes(key).to_hex().to_string(),
                cached,
                duration_ms: if cached {
                    0
                } else {
                    t0.elapsed().as_millis() as u64
                },
                partial_tiles,
            });
            produced.insert(id.clone(), outs);
        }
        // A cancel that lands during the last node must still mean "nothing published".
        if job.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(JdError::new(ErrorCode::E_CANCELLED, "evaluation cancelled")
                .why("The job's cancel flag was set while the last node ran.")
                .next("Nothing was published; re-run when ready."));
        }
        let mut outputs = Outputs::new();
        for (id, outs) in produced {
            for (port, v) in outs {
                outputs.insert((id.clone(), port), v);
            }
        }
        Ok((
            outputs,
            EvalReport {
                lod,
                nodes: reports,
                duration_ms: started.elapsed().as_millis() as u64,
            },
        ))
    }
}

/// World settings at a reduced level of detail: resolution / 2^lod, everything else unchanged
/// (identity at lod 0, so graph output tiling matches the project world exactly).
pub fn lod_world(world: &WorldSettings, lod: u8) -> WorldSettings {
    if lod == 0 {
        return world.clone();
    }
    let div = 1u32 << lod.min(12);
    WorldSettings {
        resolution_px: [
            world.resolution_px[0].div_ceil(div).max(1),
            world.resolution_px[1].div_ceil(div).max(1),
        ],
        ..world.clone()
    }
}

fn bypass(def: &NodeDef, inputs: &NodeOutputs) -> JdResult<NodeOutputs> {
    let first_in = def
        .inputs
        .first()
        .and_then(|p| inputs.get(&p.name))
        .cloned();
    let mut outs = BTreeMap::new();
    if let (Some(v), Some(out)) = (first_in, def.outputs.first()) {
        outs.insert(out.name.clone(), v);
    }
    Ok(outs)
}
