//! Stable error codes and the error envelope shared by GUI, CLI and MCP.
//!
//! The catalogue is `docs/spec/error-codes.md`; `ErrorCode::ALL` and the
//! snapshot test in `jd-core` keep the two in sync.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Stable, machine-readable error codes. Never renumber or rename; append only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[allow(non_camel_case_types, missing_docs)]
pub enum ErrorCode {
    E_INPUT_INVALID,
    E_GRAPH_CYCLE,
    E_GRAPH_TYPE_MISMATCH,
    E_GRAPH_MISSING_INPUT,
    E_NODE_UNKNOWN,
    E_FORMAT_UNSUPPORTED,
    E_FORMAT_CORRUPT,
    E_SERIALIZE,
    E_SCHEMA_VERSION,
    E_DEPENDENCY_MISSING,
    E_GEOMETRY_INVALID,
    E_NUMERIC_UNSTABLE,
    E_SHADER_COMPILE,
    E_GPU_DEVICE,
    E_RESOURCE_MEMORY,
    E_RESOURCE_DISK,
    E_PERMISSION,
    E_TIMEOUT,
    E_CANCELLED,
    E_COMPAT,
    E_PROJECT_CORRUPT,
    E_NETWORK,
    E_LICENSE_NOT_ACCEPTED,
    E_INTERNAL,
    E_ENGINE_UNAVAILABLE,
}

impl ErrorCode {
    /// Every code, in catalogue order.
    pub const ALL: &[ErrorCode] = &[
        ErrorCode::E_INPUT_INVALID,
        ErrorCode::E_GRAPH_CYCLE,
        ErrorCode::E_GRAPH_TYPE_MISMATCH,
        ErrorCode::E_GRAPH_MISSING_INPUT,
        ErrorCode::E_NODE_UNKNOWN,
        ErrorCode::E_FORMAT_UNSUPPORTED,
        ErrorCode::E_FORMAT_CORRUPT,
        ErrorCode::E_SERIALIZE,
        ErrorCode::E_SCHEMA_VERSION,
        ErrorCode::E_DEPENDENCY_MISSING,
        ErrorCode::E_GEOMETRY_INVALID,
        ErrorCode::E_NUMERIC_UNSTABLE,
        ErrorCode::E_SHADER_COMPILE,
        ErrorCode::E_GPU_DEVICE,
        ErrorCode::E_RESOURCE_MEMORY,
        ErrorCode::E_RESOURCE_DISK,
        ErrorCode::E_PERMISSION,
        ErrorCode::E_TIMEOUT,
        ErrorCode::E_CANCELLED,
        ErrorCode::E_COMPAT,
        ErrorCode::E_PROJECT_CORRUPT,
        ErrorCode::E_NETWORK,
        ErrorCode::E_LICENSE_NOT_ACCEPTED,
        ErrorCode::E_INTERNAL,
        ErrorCode::E_ENGINE_UNAVAILABLE,
    ];

    /// CLI process exit code for this error (`docs/architecture/07-agent-interfaces.md`).
    pub fn exit_code(self) -> i32 {
        use ErrorCode::*;
        match self {
            E_INPUT_INVALID
            | E_GRAPH_CYCLE
            | E_GRAPH_TYPE_MISMATCH
            | E_GRAPH_MISSING_INPUT
            | E_NODE_UNKNOWN => 2,
            E_FORMAT_UNSUPPORTED | E_FORMAT_CORRUPT => 3,
            E_RESOURCE_MEMORY | E_RESOURCE_DISK => 4,
            E_TIMEOUT | E_CANCELLED => 5,
            E_GEOMETRY_INVALID | E_COMPAT => 6,
            E_PERMISSION | E_LICENSE_NOT_ACCEPTED => 7,
            _ => 1,
        }
    }

    /// The code as it appears on the wire (`"E_INPUT_INVALID"`).
    pub fn as_str(self) -> &'static str {
        static_name(self)
    }
}

fn static_name(code: ErrorCode) -> &'static str {
    use ErrorCode::*;
    match code {
        E_INPUT_INVALID => "E_INPUT_INVALID",
        E_GRAPH_CYCLE => "E_GRAPH_CYCLE",
        E_GRAPH_TYPE_MISMATCH => "E_GRAPH_TYPE_MISMATCH",
        E_GRAPH_MISSING_INPUT => "E_GRAPH_MISSING_INPUT",
        E_NODE_UNKNOWN => "E_NODE_UNKNOWN",
        E_FORMAT_UNSUPPORTED => "E_FORMAT_UNSUPPORTED",
        E_FORMAT_CORRUPT => "E_FORMAT_CORRUPT",
        E_SERIALIZE => "E_SERIALIZE",
        E_SCHEMA_VERSION => "E_SCHEMA_VERSION",
        E_DEPENDENCY_MISSING => "E_DEPENDENCY_MISSING",
        E_GEOMETRY_INVALID => "E_GEOMETRY_INVALID",
        E_NUMERIC_UNSTABLE => "E_NUMERIC_UNSTABLE",
        E_SHADER_COMPILE => "E_SHADER_COMPILE",
        E_GPU_DEVICE => "E_GPU_DEVICE",
        E_RESOURCE_MEMORY => "E_RESOURCE_MEMORY",
        E_RESOURCE_DISK => "E_RESOURCE_DISK",
        E_PERMISSION => "E_PERMISSION",
        E_TIMEOUT => "E_TIMEOUT",
        E_CANCELLED => "E_CANCELLED",
        E_COMPAT => "E_COMPAT",
        E_PROJECT_CORRUPT => "E_PROJECT_CORRUPT",
        E_NETWORK => "E_NETWORK",
        E_LICENSE_NOT_ACCEPTED => "E_LICENSE_NOT_ACCEPTED",
        E_INTERNAL => "E_INTERNAL",
        E_ENGINE_UNAVAILABLE => "E_ENGINE_UNAVAILABLE",
    }
}

/// The error envelope: what happened, why it matters, what to do next, plus machine context.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, thiserror::Error,
)]
#[error("{code:?}: {message}")]
pub struct JdError {
    /// Stable code from the catalogue.
    pub code: ErrorCode,
    /// What happened.
    pub message: String,
    /// Why it matters.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// What the user or agent can do next.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub next: String,
    /// Machine-readable context (path, node, tile, limit, actual, …).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub details: Map<String, Value>,
}

impl JdError {
    /// Start an error with a code and message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            why: String::new(),
            next: String::new(),
            details: Map::new(),
        }
    }
    /// Attach the "why it matters" sentence.
    pub fn why(mut self, why: impl Into<String>) -> Self {
        self.why = why.into();
        self
    }
    /// Attach the "what to do next" sentence.
    pub fn next(mut self, next: impl Into<String>) -> Self {
        self.next = next.into();
        self
    }
    /// Attach one machine-readable detail.
    pub fn detail(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.details.insert(key.to_owned(), value.into());
        self
    }
    /// Convenience for invalid input at a JSON path.
    pub fn invalid(path: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorCode::E_INPUT_INVALID, message).detail("path", path)
    }
}

impl From<serde_json::Error> for JdError {
    fn from(e: serde_json::Error) -> Self {
        JdError::new(ErrorCode::E_SERIALIZE, e.to_string())
            .why("A document or message could not be read or written as JSON.")
            .next("Check the file or request for syntax errors; if it is a project file, see the recovery journal.")
    }
}

impl From<std::io::Error> for JdError {
    fn from(e: std::io::Error) -> Self {
        let code = match e.kind() {
            std::io::ErrorKind::PermissionDenied => ErrorCode::E_PERMISSION,
            std::io::ErrorKind::StorageFull => ErrorCode::E_RESOURCE_DISK,
            _ => ErrorCode::E_INTERNAL,
        };
        JdError::new(code, e.to_string()).detail("io_kind", format!("{:?}", e.kind()))
    }
}

/// Result alias used across the workspace.
pub type JdResult<T> = Result<T, JdError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_serialize_as_their_names() {
        for code in ErrorCode::ALL {
            let v = serde_json::to_value(code).expect("serializes");
            assert_eq!(v, Value::String(code.as_str().to_owned()));
            let back: ErrorCode = serde_json::from_value(v).expect("round-trips");
            assert_eq!(back, *code);
        }
    }

    #[test]
    fn all_lists_every_variant_once() {
        let mut seen = std::collections::HashSet::new();
        for c in ErrorCode::ALL {
            assert!(seen.insert(c.as_str()), "duplicate {c:?}");
        }
        assert_eq!(seen.len(), ErrorCode::ALL.len());
    }

    #[test]
    fn exit_codes_follow_the_spec() {
        assert_eq!(ErrorCode::E_INPUT_INVALID.exit_code(), 2);
        assert_eq!(ErrorCode::E_FORMAT_CORRUPT.exit_code(), 3);
        assert_eq!(ErrorCode::E_RESOURCE_MEMORY.exit_code(), 4);
        assert_eq!(ErrorCode::E_CANCELLED.exit_code(), 5);
        assert_eq!(ErrorCode::E_COMPAT.exit_code(), 6);
        assert_eq!(ErrorCode::E_PERMISSION.exit_code(), 7);
        assert_eq!(ErrorCode::E_INTERNAL.exit_code(), 1);
    }
}
