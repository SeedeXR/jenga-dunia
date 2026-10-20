//! Serialisable graph document: nodes, edges, typed ports.

use std::collections::BTreeMap;

use jd_types::{ErrorCode, JdError, JdResult, NodeId, SchemaVersion};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Port data types. Only heightfields flow today; new types are added with their first producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PortType {
    /// f32 heightfield in metres (masks are heightfields in 0..1).
    Heightfield,
}

impl PortType {
    /// Can a value of `from` be connected to an input of `self`?
    pub fn accepts(self, from: PortType) -> bool {
        self == from
    }
}

/// A node kind identifier `namespace.name@version`, e.g. `jd.noise.fbm@1`.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct NodeKind(pub String);

/// A typed port declared by a node definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Port {
    /// Port name, unique within the node's inputs or outputs.
    pub name: String,
    /// Data type.
    pub ty: PortType,
    /// Inputs only: must be connected for the node to evaluate.
    #[serde(default)]
    pub required: bool,
}

/// Editor-only metadata.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UiMeta {
    /// Canvas position.
    #[serde(default)]
    pub position: [f32; 2],
    /// Display label override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A node instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Node {
    /// Kind `name@version`.
    pub kind: NodeKind,
    /// Parameters, validated against the kind's schema on every mutation.
    #[serde(default)]
    pub params: Map<String, Value>,
    /// Bypassed nodes pass their first input through unchanged.
    #[serde(default)]
    pub bypass: bool,
    /// Editor metadata.
    #[serde(default)]
    pub ui: UiMeta,
}

/// One end of an edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PortRef {
    /// Node id.
    pub node: NodeId,
    /// Port name.
    pub port: String,
}

/// A directed connection from an output port to an input port.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Edge {
    /// Source output.
    pub from: PortRef,
    /// Destination input.
    pub to: PortRef,
}

/// `graph.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Graph {
    /// Document schema version.
    pub schema_version: SchemaVersion,
    /// Nodes keyed by id (BTreeMap for stable serialisation).
    #[serde(default)]
    pub nodes: BTreeMap<NodeId, Node>,
    /// Edges.
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Timeline position, 0..1: keyed parameters (`{"keys": [[t, v], …]}`) resolve here.
    #[serde(default)]
    pub time: f32,
    /// Preserved unknown keys (forward compatibility).
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// The keyframes of a parameter value written as `{"keys": [[t, v], …]}` — `None` for a plain value.
pub fn keyed(v: &Value) -> Option<&Vec<Value>> {
    v.as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.get("keys"))
        .and_then(Value::as_array)
}

fn key_pairs(keys: &[Value]) -> Vec<(f32, &Value)> {
    let mut pairs: Vec<(f32, &Value)> = keys
        .iter()
        .filter_map(|k| {
            let a = k.as_array()?;
            Some((a.first()?.as_f64()? as f32, a.get(1)?))
        })
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    pairs
}

/// Resolve a keyed value at `time`: numbers interpolate linearly between the neighbouring keys
/// (clamped outside the first and last), anything else steps to the last key at or before `time`.
pub fn resolve_value(v: &Value, time: f32) -> Value {
    let Some(keys) = keyed(v) else {
        return v.clone();
    };
    let pairs = key_pairs(keys);
    let (Some(first), Some(last)) = (pairs.first(), pairs.last()) else {
        return Value::Null;
    };
    if time <= first.0 {
        return first.1.clone();
    }
    if time >= last.0 {
        return last.1.clone();
    }
    let i = pairs
        .iter()
        .position(|(t, _)| *t > time)
        .unwrap_or(pairs.len() - 1);
    let (a, b) = (&pairs[i - 1], &pairs[i]);
    match (a.1.as_f64(), b.1.as_f64()) {
        (Some(x), Some(y)) if b.0 > a.0 => {
            let f = ((time - a.0) / (b.0 - a.0)) as f64;
            let v = x + (y - x) * f;
            // Keep integers integral so `u32` parameters still deserialise.
            if a.1.is_i64() && b.1.is_i64() {
                Value::from(v.round() as i64)
            } else {
                Value::from(v)
            }
        }
        _ => a.1.clone(),
    }
}

/// The parameters of a node with every keyed value resolved at `time`.
pub fn resolve_keys(params: &Map<String, Value>, time: f32) -> Map<String, Value> {
    params
        .iter()
        .map(|(k, v)| (k.clone(), resolve_value(v, time)))
        .collect()
}

/// Every key time used by a node's keyed parameters (for validating each keyframe).
pub fn key_times(params: &Map<String, Value>) -> Vec<f32> {
    let mut times: Vec<f32> = params
        .values()
        .filter_map(keyed)
        .flat_map(|keys| key_pairs(keys).into_iter().map(|(t, _)| t))
        .collect();
    times.sort_by(f32::total_cmp);
    times.dedup();
    times
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            schema_version: SchemaVersion::CURRENT,
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            time: 0.0,
            extra: BTreeMap::new(),
        }
    }
}

impl Graph {
    /// Incoming edges per node, keyed by input port name (one scan over the edges).
    pub fn input_map(&self) -> BTreeMap<&NodeId, BTreeMap<&str, &PortRef>> {
        let mut m: BTreeMap<&NodeId, BTreeMap<&str, &PortRef>> = BTreeMap::new();
        for e in &self.edges {
            m.entry(&e.to.node)
                .or_default()
                .insert(e.to.port.as_str(), &e.from);
        }
        m
    }

    /// Edges whose endpoints do not exist (corrupt or partially written documents).
    pub fn dangling_edges(&self) -> Vec<&Edge> {
        self.edges
            .iter()
            .filter(|e| {
                !self.nodes.contains_key(&e.from.node) || !self.nodes.contains_key(&e.to.node)
            })
            .collect()
    }

    /// Nodes in topological order (Kahn). Edges with missing endpoints are ignored here and
    /// reported by [`Graph::dangling_edges`]; `E_GRAPH_CYCLE` lists the nodes left in the cycle.
    pub fn topological_order(&self) -> JdResult<Vec<NodeId>> {
        let live = |e: &&Edge| {
            self.nodes.contains_key(&e.from.node) && self.nodes.contains_key(&e.to.node)
        };
        let mut indegree: BTreeMap<&NodeId, usize> = self.nodes.keys().map(|id| (id, 0)).collect();
        for e in self.edges.iter().filter(live) {
            if let Some(d) = indegree.get_mut(&e.to.node) {
                *d += 1;
            }
        }
        let mut ready: Vec<&NodeId> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(id) = ready.pop() {
            order.push(id.clone());
            for e in self
                .edges
                .iter()
                .filter(live)
                .filter(|e| &e.from.node == id)
            {
                if let Some(d) = indegree.get_mut(&e.to.node) {
                    *d -= 1;
                    if *d == 0 {
                        ready.push(&e.to.node);
                    }
                }
            }
            // Deterministic order regardless of edge order: smallest id first.
            ready.sort_by(|a, b| b.cmp(a));
        }
        if order.len() != self.nodes.len() {
            let stuck: Vec<String> = indegree
                .iter()
                .filter(|(_, d)| **d > 0)
                .map(|(id, _)| id.to_string())
                .collect();
            return Err(
                JdError::new(ErrorCode::E_GRAPH_CYCLE, "graph contains a cycle")
                    .why("Nodes that depend on themselves can never be evaluated.")
                    .next("Remove one of the edges among the listed nodes.")
                    .detail("nodes", stuck),
            );
        }
        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_values_interpolate_numbers_step_everything_else_and_clamp() {
        let keys = serde_json::json!({ "keys": [[0.0, 10], [1.0, 110]] });
        assert_eq!(resolve_value(&keys, 0.5), serde_json::json!(60));
        assert_eq!(resolve_value(&keys, -1.0), serde_json::json!(10));
        assert_eq!(resolve_value(&keys, 2.0), serde_json::json!(110));
        let floats = serde_json::json!({ "keys": [[0.25, 1.0], [0.75, 2.0]] });
        assert_eq!(resolve_value(&floats, 0.5), serde_json::json!(1.5));
        let steps = serde_json::json!({ "keys": [[0.0, "d8"], [0.5, "mfd"]] });
        assert_eq!(resolve_value(&steps, 0.49), serde_json::json!("d8"));
        assert_eq!(resolve_value(&steps, 0.5), serde_json::json!("mfd"));
        assert_eq!(
            resolve_value(&serde_json::json!(7), 0.3),
            serde_json::json!(7)
        );
        assert_eq!(
            resolve_value(&serde_json::json!({ "keys": [] }), 0.3),
            Value::Null
        );
        let mut params = Map::new();
        params.insert("iterations".into(), keys.clone());
        params.insert("method".into(), steps);
        params.insert("rate".into(), serde_json::json!(0.2));
        let at = resolve_keys(&params, 0.75);
        assert_eq!(at["iterations"], serde_json::json!(85));
        assert_eq!(at["method"], serde_json::json!("mfd"));
        assert_eq!(at["rate"], serde_json::json!(0.2));
        assert_eq!(key_times(&params), vec![0.0, 0.5, 1.0]);
    }

    fn node(kind: &str) -> Node {
        Node {
            kind: NodeKind(kind.into()),
            params: Map::new(),
            bypass: false,
            ui: UiMeta::default(),
        }
    }

    fn edge(a: &NodeId, b: &NodeId) -> Edge {
        Edge {
            from: PortRef {
                node: a.clone(),
                port: "out".into(),
            },
            to: PortRef {
                node: b.clone(),
                port: "in".into(),
            },
        }
    }

    #[test]
    fn topological_order_and_cycle_detection() {
        let (a, b, c) = (NodeId::new(), NodeId::new(), NodeId::new());
        let mut g = Graph::default();
        for id in [&a, &b, &c] {
            g.nodes.insert(id.clone(), node("x@1"));
        }
        g.edges.push(edge(&a, &b));
        g.edges.push(edge(&b, &c));
        let order = g.topological_order().expect("acyclic");
        let pos = |id: &NodeId| order.iter().position(|x| x == id).expect("present");
        assert!(pos(&a) < pos(&b) && pos(&b) < pos(&c));
        g.edges.push(edge(&c, &a));
        let err = g.topological_order().expect_err("cycle");
        assert_eq!(err.code, ErrorCode::E_GRAPH_CYCLE);
        assert_eq!(err.details["nodes"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn dangling_edges_are_not_cycles() {
        let (a, b) = (NodeId::new(), NodeId::new());
        let mut g = Graph::default();
        g.nodes.insert(b.clone(), node("x@1"));
        g.edges.push(edge(&a, &b)); // `a` does not exist
        assert_eq!(g.topological_order().expect("no cycle").len(), 1);
        assert_eq!(g.dangling_edges().len(), 1);
    }
}
