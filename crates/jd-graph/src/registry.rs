//! Node definitions and their registry.

use std::collections::BTreeMap;
use std::sync::Arc;

use jd_types::{ErrorCode, JdError, JdResult, WorldSettings};
use serde_json::{Map, Value};

use crate::eval::Value as EvalValue;
use crate::model::{NodeKind, Port};

/// Context handed to a node's evaluate function.
pub struct EvalCtx {
    /// World settings at the requested level of detail (resolution reduced, extent/tiling unchanged).
    pub lod_world: WorldSettings,
    /// The open project's `assets/` directory, when evaluating inside a project (file-reading
    /// nodes resolve relative paths here and never outside it).
    pub assets_dir: Option<std::path::PathBuf>,
}

type EvalFn = Arc<
    dyn Fn(
            &EvalCtx,
            &Map<String, Value>,
            &BTreeMap<String, EvalValue>,
        ) -> JdResult<BTreeMap<String, EvalValue>>
        + Send
        + Sync,
>;
type CheckFn = Arc<dyn Fn(&Map<String, Value>) -> JdResult<()> + Send + Sync>;

/// How a node kind is validated and evaluated.
#[derive(Clone)]
pub struct NodeDef {
    /// Kind, `name@version`.
    pub kind: NodeKind,
    /// Human description.
    pub description: &'static str,
    /// Inputs.
    pub inputs: Vec<Port>,
    /// Outputs.
    pub outputs: Vec<Port>,
    /// JSON Schema of `params`.
    pub param_schema: Value,
    check: CheckFn,
    eval: EvalFn,
}

impl NodeDef {
    /// Build a definition whose params deserialise into `P` (schema and check derive from it).
    pub fn new<P, F>(
        kind: &str,
        description: &'static str,
        inputs: Vec<Port>,
        outputs: Vec<Port>,
        eval: F,
    ) -> Self
    where
        P: for<'de> serde::Deserialize<'de> + schemars::JsonSchema,
        F: Fn(
                &EvalCtx,
                &Map<String, Value>,
                &BTreeMap<String, EvalValue>,
            ) -> JdResult<BTreeMap<String, EvalValue>>
            + Send
            + Sync
            + 'static,
    {
        let check: CheckFn = Arc::new(|p: &Map<String, Value>| {
            serde_json::from_value::<P>(Value::Object(p.clone()))
                .map(|_| ())
                .map_err(|e| {
                    JdError::invalid("params", e.to_string())
                        .why("Parameters must match the node kind's schema.")
                        .next("See graph.kinds for the schema.")
                })
        });
        Self {
            kind: NodeKind(kind.into()),
            description,
            inputs,
            outputs,
            param_schema: serde_json::to_value(schemars::schema_for!(P)).unwrap_or(Value::Null),
            check,
            eval: Arc::new(eval),
        }
    }

    /// Validate params against the kind's parameter type (used before journaling mutations).
    pub fn check_params(&self, params: &Map<String, Value>) -> JdResult<()> {
        (self.check)(params)
    }

    /// Evaluate with validated params and typed inputs.
    pub fn evaluate(
        &self,
        ctx: &EvalCtx,
        params: &Map<String, Value>,
        inputs: &BTreeMap<String, EvalValue>,
    ) -> JdResult<BTreeMap<String, EvalValue>> {
        (self.eval)(ctx, params, inputs)
    }

    /// Output port by name.
    pub fn output(&self, name: &str) -> Option<&Port> {
        self.outputs.iter().find(|p| p.name == name)
    }

    /// Input port by name.
    pub fn input(&self, name: &str) -> Option<&Port> {
        self.inputs.iter().find(|p| p.name == name)
    }
}

/// Machine-readable description of a node kind.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct NodeDefInfo {
    /// Kind.
    pub kind: NodeKind,
    /// Description.
    pub description: String,
    /// Inputs.
    pub inputs: Vec<Port>,
    /// Outputs.
    pub outputs: Vec<Port>,
    /// Param schema.
    pub param_schema: Value,
}

/// Registry of node kinds.
#[derive(Clone, Default)]
pub struct Registry {
    defs: BTreeMap<NodeKind, NodeDef>,
}

impl Registry {
    /// Register a definition (replacing an existing kind of the same name@version).
    pub fn register(&mut self, def: NodeDef) {
        self.defs.insert(def.kind.clone(), def);
    }

    /// Look up a kind.
    pub fn get(&self, kind: &NodeKind) -> JdResult<&NodeDef> {
        self.defs.get(kind).ok_or_else(|| {
            JdError::new(ErrorCode::E_NODE_UNKNOWN, format!("unknown node kind `{}`", kind.0))
                .why("The project references a node this build does not provide (missing plugin or newer version).")
                .next("Install the plugin that provides it, or replace the node.")
                .detail("kind", kind.0.clone())
        })
    }

    /// All definitions.
    pub fn list(&self) -> Vec<NodeDefInfo> {
        self.defs
            .values()
            .map(|d| NodeDefInfo {
                kind: d.kind.clone(),
                description: d.description.to_owned(),
                inputs: d.inputs.clone(),
                outputs: d.outputs.clone(),
                param_schema: d.param_schema.clone(),
            })
            .collect()
    }

    /// The registry with every built-in node.
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        crate::nodes::register_all(&mut r);
        r
    }
}
