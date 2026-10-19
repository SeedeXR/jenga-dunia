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
    /// Preserved unknown keys (forward compatibility).
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            schema_version: SchemaVersion::CURRENT,
            nodes: BTreeMap::new(),
            edges: Vec::new(),
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
