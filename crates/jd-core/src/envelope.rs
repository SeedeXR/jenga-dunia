//! JSON request/response envelope (`docs/spec/json-api.md`).

use jd_types::{JdError, JobId, RequestId, SchemaVersion};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Who is calling, for audit logs and scope checks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Client {
    /// `gui` | `cli` | `mcp` | `plugin`.
    #[serde(default)]
    pub kind: String,
    /// Free-form client name (e.g. "claude-code").
    #[serde(default)]
    pub name: String,
    /// Granted scopes (see [`crate::ops::Scope`]).
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// An operation request.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Operation id, `<subsystem>.<verb>`.
    pub op: String,
    /// API schema version.
    pub schema_version: SchemaVersion,
    /// Correlation id; generated when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<RequestId>,
    /// Operation parameters, validated against the op's request schema.
    #[serde(default)]
    pub params: Value,
    /// Calling client, for audit and scopes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Client>,
}

impl Request {
    /// Build a request for `op` with `params` at the current schema version.
    pub fn new(op: &str, params: Value) -> Self {
        Self {
            op: op.to_owned(),
            schema_version: SchemaVersion::CURRENT,
            request_id: None,
            params,
            client: None,
        }
    }
}

/// Outcome status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Completed synchronously; `result` is set.
    Ok,
    /// Accepted as a background job; `job_id` is set.
    Accepted,
    /// Failed; `error` is set.
    Error,
}

/// A non-fatal warning (`W_…` codes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Warning {
    /// Stable warning code, e.g. `W_UNKNOWN_FIELDS`.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Machine context.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl Warning {
    /// Build a warning.
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            details: Value::Null,
        }
    }
}

/// An operation response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Response {
    /// Echo of the request op.
    pub op: String,
    /// API schema version.
    pub schema_version: SchemaVersion,
    /// Correlation id.
    pub request_id: RequestId,
    /// Outcome.
    pub status: Status,
    /// Result payload when `status == ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Job id when `status == accepted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<JobId>,
    /// Error when `status == error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JdError>,
    /// Non-fatal warnings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Warning>,
}

impl Response {
    /// A successful response.
    pub fn ok(op: &str, request_id: RequestId, result: Value, warnings: Vec<Warning>) -> Self {
        Self {
            op: op.to_owned(),
            schema_version: SchemaVersion::CURRENT,
            request_id,
            status: Status::Ok,
            result: Some(result),
            job_id: None,
            error: None,
            warnings,
        }
    }
    /// A failed response.
    pub fn error(op: &str, request_id: RequestId, error: JdError) -> Self {
        Self {
            op: op.to_owned(),
            schema_version: SchemaVersion::CURRENT,
            request_id,
            status: Status::Error,
            result: None,
            job_id: None,
            error: Some(error),
            warnings: Vec::new(),
        }
    }
}
