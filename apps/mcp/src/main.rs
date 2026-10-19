//! `jd-mcp` — Jenga Dunia MCP server.
//!
//! Exposes every `jd-core` operation as an MCP tool (same ids, same JSON
//! schemas) over stdio. Tools are filtered by the scopes granted at launch;
//! project paths are confined to `--workspace` when given. Results are returned
//! as structured content: the full `jd-core` response envelope.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use jd_core::{Context, Registry, Request, Scope};
use jd_types::{ErrorCode, JdError};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, transport::stdio};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "jd-mcp", version, about = "Jenga Dunia MCP server (stdio)")]
struct Cli {
    /// Granted scopes (repeatable): project:ro, project:rw, fs:write, export, destructive, network:geo, vision, all.
    #[arg(long = "scope", default_values_t = vec!["project:ro".to_string(), "project:rw".to_string(), "fs:write".to_string()])]
    scopes: Vec<String>,
    /// Confine project paths to this directory (recommended).
    #[arg(long)]
    workspace: Option<PathBuf>,
    /// Log level filter for stderr.
    #[arg(long, default_value = "info", env = "JD_LOG")]
    log: String,
}

#[derive(Clone)]
struct JdServer {
    registry: Registry,
    ctx: Context,
    scopes: Arc<Vec<String>>,
    workspace: Option<PathBuf>,
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::None => "none",
        Scope::ProjectRead => "project:ro",
        Scope::ProjectWrite => "project:rw",
        Scope::FsWrite => "fs:write",
        Scope::Export => "export",
        Scope::Destructive => "destructive",
        Scope::NetworkGeo => "network:geo",
        Scope::Vision => "vision",
    }
}

impl JdServer {
    fn allowed(&self, scope: Scope) -> bool {
        scope == Scope::None
            || self
                .scopes
                .iter()
                .any(|s| s == "all" || s == scope_name(scope))
    }

    /// Reject `params.path` outside the workspace. Only ops that take a `path` are affected.
    fn check_workspace(&self, params: &Value) -> Result<(), JdError> {
        let (Some(ws), Some(path)) = (&self.workspace, params.get("path").and_then(Value::as_str))
        else {
            return Ok(());
        };
        let candidate = Path::new(path);
        // Canonicalise the deepest existing ancestor so new project dirs are checked too.
        let mut probe = candidate.to_path_buf();
        while !probe.exists() {
            if !probe.pop() {
                break;
            }
        }
        let resolved = probe.canonicalize().unwrap_or(probe);
        let ws = ws.canonicalize().unwrap_or_else(|_| ws.clone());
        if resolved.starts_with(&ws) {
            Ok(())
        } else {
            Err(JdError::new(ErrorCode::E_PERMISSION, "path is outside the MCP workspace")
                .why("The server was started with --workspace, which confines project access.")
                .next("Use a path inside the workspace or restart jd-mcp with a different --workspace.")
                .detail("path", path)
                .detail("workspace", ws.display().to_string()))
        }
    }
}

fn as_object(v: &Value) -> JsonObject {
    v.as_object().cloned().unwrap_or_default()
}

fn envelope_error(op: &str, err: JdError) -> Value {
    serde_json::to_value(jd_core::Response::error(op, Default::default(), err))
        .unwrap_or(Value::Null)
}

impl ServerHandler for JdServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("jenga-dunia", jd_core::APP_VERSION).with_title("Jenga Dunia"))
            .with_instructions(
                "Jenga Dunia procedural world generation. Tools mirror the `jd` CLI operations: create/open a project \
                 (project.create / project.open), inspect it (project.info), mutate it (project.set_name), save \
                 (project.save), recover after a crash (project.recover), and inspect the host (diag.env). Every result \
                 is a JSON envelope {status, result|error, warnings}; errors carry stable E_* codes with `why` and `next`.",
            )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools = self
            .registry
            .list()
            .into_iter()
            .filter(|o| self.allowed(o.scope))
            .map(|o| {
                Tool::new(o.id, o.description, Arc::new(as_object(&o.request_schema)))
                    .with_raw_output_schema(Arc::new(as_object(&o.result_schema)))
            })
            .collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, McpError>> + Send + '_ {
        let op = request.name.to_string();
        let params = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| json!({}));
        let value = match self.registry.get(&op) {
            None => envelope_error(
                &op,
                JdError::new(ErrorCode::E_INPUT_INVALID, format!("unknown tool `{op}`")),
            ),
            Some(def) if !self.allowed(def.scope) => envelope_error(
                &op,
                JdError::new(
                    ErrorCode::E_PERMISSION,
                    format!("scope `{}` not granted", scope_name(def.scope)),
                )
                .next(format!(
                    "Restart jd-mcp with --scope {}.",
                    scope_name(def.scope)
                )),
            ),
            Some(_) => match self.check_workspace(&params) {
                Err(e) => envelope_error(&op, e),
                Ok(()) => {
                    let mut req = Request::new(&op, params);
                    req.client = Some(jd_core::envelope::Client {
                        kind: "mcp".into(),
                        name: "jd-mcp".into(),
                        scopes: (*self.scopes).clone(),
                    });
                    let resp = self.registry.invoke(&self.ctx, req);
                    serde_json::to_value(&resp).unwrap_or(Value::Null)
                }
            },
        };
        let is_error = value.get("status").and_then(Value::as_str) == Some("error");
        let result = if is_error {
            CallToolResult::structured_error(value)
        } else {
            CallToolResult::structured(value)
        };
        std::future::ready(Ok(CallToolResponse::Complete(result)))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&cli.log))
        .with_writer(std::io::stderr)
        .init();
    let server = JdServer {
        registry: Registry::with_builtins(),
        ctx: Context::default(),
        scopes: Arc::new(cli.scopes),
        workspace: cli.workspace,
    };
    tracing::info!(scopes = ?server.scopes, workspace = ?server.workspace, "jd-mcp starting on stdio");
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
