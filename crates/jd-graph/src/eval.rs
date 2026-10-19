//! Incremental, content-hashed evaluation.

use std::collections::{BTreeMap, HashMap};
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
    /// Heightfield in metres (masks are heightfields in 0..1).
    Field(Arc<Heightfield>, blake3::Hash),
}

impl Value {
    /// Wrap a field, hashing it once.
    pub fn field(hf: Heightfield) -> Self {
        let hash = hf.content_hash();
        Value::Field(Arc::new(hf), hash)
    }
    /// Borrow the field.
    pub fn as_field(&self) -> &Arc<Heightfield> {
        match self {
            Value::Field(f, _) => f,
        }
    }
    /// Content hash for caching.
    pub fn content_hash(&self) -> blake3::Hash {
        match self {
            Value::Field(_, h) => *h,
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
}

impl Cache {
    /// A cache holding at most `cap` node results (0 = unbounded).
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Arc::default(),
            cap,
        }
    }
    fn get(&self, key: &[u8; 32]) -> Option<NodeOutputs> {
        self.inner.lock().ok()?.get(key).cloned()
    }
    fn put(&self, key: [u8; 32], v: NodeOutputs) {
        if let Ok(mut g) = self.inner.lock() {
            if self.cap > 0 && g.len() >= self.cap {
                g.clear(); // ponytail: wholesale eviction; LRU when the disk cache arrives
            }
            g.insert(key, v);
        }
    }
}

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

/// Evaluates graphs against a registry and cache.
pub struct Evaluator<'a> {
    registry: &'a Registry,
    cache: &'a Cache,
}

/// Values produced by every node: `(node id, port) → value`.
pub type Outputs = BTreeMap<(NodeId, String), Value>;

impl<'a> Evaluator<'a> {
    /// Build an evaluator.
    pub fn new(registry: &'a Registry, cache: &'a Cache) -> Self {
        Self { registry, cache }
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
            if let Err(e) = def.check_params(&node.params) {
                errs.push(e.detail("node", id.to_string()));
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
        let started = Instant::now();
        let order = self.validate(graph)?;
        let ctx = EvalCtx {
            lod_world: lod_world(world, lod),
            assets_dir: assets_dir.map(|p| p.to_path_buf()),
        };
        let lod_world_bytes = serde_json::to_vec(&ctx.lod_world)?;
        let input_map = graph.input_map();
        let mut produced: BTreeMap<NodeId, NodeOutputs> = BTreeMap::new();
        let mut reports = Vec::with_capacity(order.len());
        for id in &order {
            let node = &graph.nodes[id];
            let def = self.registry.get(&node.kind)?;
            let mut inputs: NodeOutputs = BTreeMap::new();
            let mut hasher = blake3::Hasher::new();
            hasher.update(node.kind.0.as_bytes());
            hasher.update(&serde_json::to_vec(&node.params)?);
            hasher.update(&[node.bypass as u8, lod]);
            hasher.update(&lod_world_bytes);
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
            let (outs, cached) = match self.cache.get(&key) {
                Some(outs) => (outs, true),
                None => {
                    let outs = if node.bypass {
                        bypass(def, &inputs)
                    } else {
                        def.evaluate(&ctx, &node.params, &inputs)
                    }
                    .map_err(|e| {
                        e.detail("node", id.to_string())
                            .detail("kind", node.kind.0.clone())
                    })?;
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
            });
            produced.insert(id.clone(), outs);
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
