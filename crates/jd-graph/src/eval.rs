//! Incremental, content-hashed evaluation.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use jd_terrain::Heightfield;
use jd_types::{ErrorCode, JdError, JdResult, NodeId, WorldSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::model::{Graph, PortType};
use crate::registry::{EvalCtx, Registry};

/// A value flowing through the graph.
#[derive(Debug, Clone)]
pub enum Value {
    /// Heightfield in metres (also carries masks: 0..1).
    Field(Arc<Heightfield>),
    /// A number.
    Scalar(f64),
}

impl Value {
    /// Port type of this value.
    pub fn port_type(&self) -> PortType {
        match self {
            Value::Field(_) => PortType::Heightfield,
            Value::Scalar(_) => PortType::Scalar,
        }
    }
    /// Borrow as a field or fail with `E_GRAPH_TYPE_MISMATCH`.
    pub fn field(&self) -> JdResult<&Arc<Heightfield>> {
        match self {
            Value::Field(f) => Ok(f),
            other => Err(JdError::new(
                ErrorCode::E_GRAPH_TYPE_MISMATCH,
                format!("expected a field, got {:?}", other.port_type()),
            )),
        }
    }
    /// Borrow as a scalar or fail.
    pub fn scalar(&self) -> JdResult<f64> {
        match self {
            Value::Scalar(s) => Ok(*s),
            other => Err(JdError::new(
                ErrorCode::E_GRAPH_TYPE_MISMATCH,
                format!("expected a scalar, got {:?}", other.port_type()),
            )),
        }
    }
    /// Content hash for caching.
    pub fn content_hash(&self) -> blake3::Hash {
        match self {
            Value::Field(f) => f.content_hash(),
            Value::Scalar(s) => blake3::hash(&s.to_le_bytes()),
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
    fn get(&self, key: &[u8; 32]) -> Option<BTreeMap<String, Value>> {
        self.inner.lock().ok()?.get(key).cloned()
    }
    fn put(&self, key: [u8; 32], v: BTreeMap<String, Value>) {
        if let Ok(mut g) = self.inner.lock() {
            if self.cap > 0 && g.len() >= self.cap {
                g.clear(); // ponytail: wholesale eviction; LRU when the disk cache arrives
            }
            g.insert(key, v);
        }
    }
    /// Number of cached results.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or(0)
    }
    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
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

/// Values produced by output nodes: `(output node id, port) → value`.
pub type Outputs = BTreeMap<(NodeId, String), Value>;

impl<'a> Evaluator<'a> {
    /// Build an evaluator.
    pub fn new(registry: &'a Registry, cache: &'a Cache) -> Self {
        Self { registry, cache }
    }

    /// Validate the whole graph without evaluating: kinds known, ports exist, types compatible,
    /// required inputs connected, no cycles.
    pub fn validate(&self, graph: &Graph) -> JdResult<Vec<NodeId>> {
        let order = graph.topological_order()?;
        for (id, node) in &graph.nodes {
            let def = self.registry.get(&node.kind)?;
            let inputs = graph.inputs_of(id);
            for port in &def.inputs {
                match inputs.get(port.name.as_str()) {
                    None if port.required && !node.bypass => {
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
                    None => {}
                    Some(from) => {
                        let src = graph.nodes.get(&from.node).ok_or_else(|| {
                            JdError::new(
                                ErrorCode::E_INPUT_INVALID,
                                format!("edge from unknown node {}", from.node),
                            )
                            .detail("node", from.node.to_string())
                        })?;
                        let src_def = self.registry.get(&src.kind)?;
                        let out = src_def.output(&from.port).ok_or_else(|| {
                            JdError::new(
                                ErrorCode::E_INPUT_INVALID,
                                format!(
                                    "node {} ({}) has no output `{}`",
                                    from.node, src.kind.0, from.port
                                ),
                            )
                            .detail("node", from.node.to_string())
                            .detail("port", from.port.clone())
                        })?;
                        if !port.ty.accepts(out.ty) {
                            return Err(JdError::new(
                                ErrorCode::E_GRAPH_TYPE_MISMATCH,
                                format!(
                                    "cannot connect {:?} output `{}` to {:?} input `{}`",
                                    out.ty, from.port, port.ty, port.name
                                ),
                            )
                            .next("Insert a conversion node or pick a compatible port.")
                            .detail("from", from.node.to_string())
                            .detail("to", id.to_string()));
                        }
                    }
                }
            }
            for name in inputs.keys() {
                if def.input(name).is_none() {
                    return Err(JdError::new(
                        ErrorCode::E_INPUT_INVALID,
                        format!("node {} ({}) has no input `{}`", id, node.kind.0, name),
                    )
                    .detail("node", id.to_string())
                    .detail("port", (*name).to_owned()));
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
        let started = Instant::now();
        let order = self.validate(graph)?;
        let lod_world = lod_world(world, lod);
        let ctx = EvalCtx {
            world,
            lod,
            lod_world,
        };
        let mut produced: BTreeMap<NodeId, BTreeMap<String, Value>> = BTreeMap::new();
        let mut reports = Vec::with_capacity(order.len());
        for id in &order {
            let node = &graph.nodes[id];
            let def = self.registry.get(&node.kind)?;
            // Gather inputs (already produced, since we walk in topological order).
            let mut inputs: BTreeMap<String, Value> = BTreeMap::new();
            let mut hasher = blake3::Hasher::new();
            hasher.update(node.kind.0.as_bytes());
            hasher.update(&serde_json::to_vec(&node.params)?);
            hasher.update(&[node.bypass as u8, lod]);
            hasher.update(&serde_json::to_vec(&ctx.lod_world)?);
            for port in &def.inputs {
                if let Some(from) = graph.inputs_of(id).get(port.name.as_str()) {
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
                        bypass(def, &inputs)?
                    } else {
                        def.evaluate(&ctx, &node.params, &inputs)?
                    };
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

/// World settings at a reduced level of detail (resolution / 2^lod, same extent).
pub fn lod_world(world: &WorldSettings, lod: u8) -> WorldSettings {
    let div = 1u32 << lod.min(12);
    WorldSettings {
        resolution_px: [
            world.resolution_px[0].div_ceil(div).max(1),
            world.resolution_px[1].div_ceil(div).max(1),
        ],
        tile_size: world.tile_size.clamp(64, 512),
        halo: world.halo.min(world.tile_size.clamp(64, 512) / 2),
        ..world.clone()
    }
}

fn bypass(
    def: &crate::registry::NodeDef,
    inputs: &BTreeMap<String, Value>,
) -> JdResult<BTreeMap<String, Value>> {
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

/// Helper for ops: JSON view of outputs (hashes only).
pub fn outputs_summary(outputs: &Outputs) -> Json {
    Json::Array(
        outputs
            .iter()
            .map(|((id, port), v)| serde_json::json!({ "node": id, "port": port, "type": v.port_type(), "hash": v.content_hash().to_hex().to_string() }))
            .collect(),
    )
}
