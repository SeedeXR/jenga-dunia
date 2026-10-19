//! Procedural graph engine (`docs/architecture/04-graph-engine.md`, ADR-0002).
//!
//! A [`Graph`] is a typed DAG of [`Node`]s joined by [`Edge`]s. A [`Registry`] of
//! [`NodeDef`]s knows how to validate and evaluate each node kind. Evaluation is
//! incremental: every node's inputs (kind@version, params, world, lod, upstream
//! hashes) are content-hashed with blake3; unchanged hashes are served from the
//! [`Cache`] without recomputation. The engine has no UI, no filesystem and no
//! Tauri dependency so the CLI and MCP evaluate graphs headless.

pub mod eval;
pub mod model;
pub mod nodes;
pub mod registry;

pub use eval::{Cache, EvalReport, Evaluator, NodeReport, Outputs, Value};
pub use model::{Edge, Graph, Node, NodeKind, Port, PortRef, PortType, UiMeta};
pub use registry::{EvalCtx, NodeDef, NodeDefInfo, Registry};
