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
    /// Set when the job is cancelled; long kernels poll it and return `E_CANCELLED`.
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
    /// Peak scratch memory the kernel needs per cell, bytes, as a function of its params
    /// (outputs are counted separately).
    scratch: ScratchFn,
    /// The node's output is its input (an `Arc` clone), so it costs no memory of its own.
    aliases_input: bool,
}

type ScratchFn = Arc<dyn Fn(&Map<String, Value>) -> f32 + Send + Sync>;

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
            scratch: Arc::new(|_| 0.0),
            aliases_input: false,
        }
    }

    /// Declare the kernel's peak scratch memory per cell (bytes) for the pre-flight estimate.
    pub fn with_memory(self, scratch_bytes_per_cell: f32) -> Self {
        self.with_memory_fn(move |_| scratch_bytes_per_cell)
    }

    /// [`Self::with_memory`] decided from the node's params (a tiled kernel needs far less).
    pub fn with_memory_fn(
        mut self,
        scratch: impl Fn(&Map<String, Value>) -> f32 + Send + Sync + 'static,
    ) -> Self {
        self.scratch = Arc::new(scratch);
        self
    }

    /// The node republishes its input field (no copy): the estimate counts no output for it.
    pub fn aliasing_input(mut self) -> Self {
        self.aliases_input = true;
        self
    }

    /// Bytes this node's outputs occupy on `cells` samples (0 when it aliases its input).
    pub fn output_bytes(&self, cells: u64) -> u64 {
        if self.aliases_input {
            0
        } else {
            self.outputs.len().max(1) as u64 * 4 * cells
        }
    }

    /// Estimated peak bytes to evaluate this node on `cells` samples with `params`: every output
    /// field plus the kernel's scratch.
    pub fn estimate_bytes(&self, cells: u64, params: &Map<String, Value>) -> u64 {
        self.output_bytes(cells) + ((self.scratch)(params) as f64 * cells as f64) as u64
    }

    /// Add a semantic check on top of the schema check (ranges, ordering…): it runs before a
    /// mutation is journaled, so the editor hears about a bad value immediately, not at evaluation.
    pub fn with_check<P, F>(mut self, f: F) -> Self
    where
        P: for<'de> serde::Deserialize<'de>,
        F: Fn(&P) -> JdResult<()> + Send + Sync + 'static,
    {
        let schema_check = self.check.clone();
        self.check = Arc::new(move |p: &Map<String, Value>| {
            schema_check(p)?;
            let typed: P = serde_json::from_value(Value::Object(p.clone()))
                .map_err(|e| JdError::invalid("params", e.to_string()))?;
            f(&typed)
        });
        self
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
