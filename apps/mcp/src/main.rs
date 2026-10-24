//! `jd-mcp` — Jenga Dunia MCP server.
//!
//! Exposes every `jd-core` operation as an MCP tool (same ids, same JSON
//! schemas) over stdio. Tools are filtered by the scopes granted at launch;
//! project paths are confined to `--workspace` when given. Results are returned
//! as structured content: the full `jd-core` response envelope.
//!
//! Beyond the tools: read-only resources (the guides an agent needs to build a believable world —
//! `jd://guide/reading-a-landscape`, `jd://guide/environments` — and the node-kind and op
//! catalogues) and the `terrain_from_reference` prompt, which walks an agent from a reference
//! photo, panorama or video to a measured world.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use jd_core::{Context, Registry, Request, Scope};
use jd_types::{ErrorCode, JdError};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, Implementation, JsonObject, ListPromptsResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, Prompt, PromptArgument,
    PromptMessage, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, Role, ServerCapabilities, ServerConfig, Tool,
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

/// Read-only resources: (uri, name, description, mime type, text). Compiled in, so they always
/// match the build that serves them.
const RESOURCES: [(&str, &str, &str, &str, &str); 4] = [
    (
        "jd://guide/reading-a-landscape",
        "Reading a landscape",
        "From a reference photo, 360° panorama or video to a believable world: what a picture can and cannot tell, the steps, the reading sheet, landform recipes as node graphs, measured real-terrain exemplars and the realism checks.",
        "text/markdown",
        include_str!("../../../docs/user/reading-a-landscape.md"),
    ),
    (
        "jd://guide/environments",
        "Environments",
        "Soil, weather and climate, sand deserts, snow-capped mountains, volcanoes and lava: which nodes, which settings, what is not modelled.",
        "text/markdown",
        include_str!("../../../docs/user/environments.md"),
    ),
    (
        "jd://schema/node-kinds",
        "Node kinds",
        "Every graph node kind with its ports, description and parameter JSON schema (defaults included).",
        "application/json",
        include_str!("../../../schemas/v1/graph/node-kinds.json"),
    ),
    (
        "jd://schema/ops",
        "Operations",
        "Every operation (tool) id with its description and scope.",
        "application/json",
        include_str!("../../../schemas/v1/ops/index.json"),
    ),
];

const PROMPT_REFERENCE: &str = "terrain_from_reference";

fn reference_prompt_text(args: &JsonObject) -> String {
    let get = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let image = get("image").unwrap_or("<the reference image or video>");
    let mut text = format!(
        "Build a believable Jenga Dunia world from the reference `{image}`.\n\n\
         Work through these steps with the jd tools, and follow the guide below (also the resource `jd://guide/reading-a-landscape`):\n\
         1. `media.metadata {{path}}` on the reference: GPS, UTC time, heading, field of view, projection, and the sun. Use its `suggest` params and read its `notes`.\n\
         2. Look at the image and fill the reading sheet from the guide. Put every guess in `unknowns`; do not invent scale.\n\
         3. If the place is known, fetch real elevation with `geo.fetch_dem` (scope network:geo), set the world `crs` to the UTM zone and `origin`, and import it with `jd.input.heightmap@1 {{georeference: true}}`. Otherwise start from the landform recipe.\n\
         4. Add the processes the image shows (erosion, wind, snow, water, lava, debris) as graph nodes; shape the visible foreground with `vision.depth` (scope vision) and `jd.sculpt.strokes@1` when useful.\n\
         5. Set the sky and the snow nodes' sun from `suggest.sky`, and the climate from the reading sheet.\n\
         6. Run `terrain.measure` on the result and on the guide's exemplar for this landform at the same `search_m`, and adjust until slope distribution, geomorphon shares and hypsometric integral are close.\n\
         7. Report what you built and every assumption (the sheet's `unknowns`, scale, heading)."
    );
    if let Some(place) = get("place") {
        text.push_str(&format!("\n\nThe user says the place is: {place}."));
    }
    if let Some(intent) = get("intent") {
        text.push_str(&format!("\n\nWhat the user wants: {intent}."));
    }
    text.push_str("\n\n---\n\n");
    text.push_str(RESOURCES[0].4);
    text
}

#[derive(Clone)]
struct JdServer {
    registry: Registry,
    ctx: Context,
    scopes: Arc<Vec<String>>,
    workspace: Option<PathBuf>,
    envelope_schema: Arc<JsonObject>,
}

fn envelope_schema() -> Arc<JsonObject> {
    let schema =
        serde_json::to_value(schemars::schema_for!(jd_core::Response)).unwrap_or(Value::Null);
    Arc::new(as_object(&schema))
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
        // Relative paths are resolved by the core against the open project (exports/) — mirror that
        // here so the check sees the path that will actually be written.
        let candidate = match (Path::new(path).is_relative(), self.ctx.project_root()) {
            (true, Some(root)) => root.join("exports").join(path),
            _ => Path::new(path).to_path_buf(),
        };
        // Canonicalise the deepest existing ancestor so new project dirs are checked too.
        let mut probe = candidate.clone();
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
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_prompts().enable_resources().build())
            .with_server_info(Implementation::new("jenga-dunia", jd_core::APP_VERSION).with_title("Jenga Dunia"))
            .with_instructions(
                "Jenga Dunia procedural world generation. Tools mirror the `jd` CLI operations: projects (project.*), the \
                 node graph (graph.add_node / connect / set_param / evaluate — node kinds in the resource jd://schema/node-kinds), \
                 layers and exports (terrain.*), real-world data (geo.fetch_dem, scope network:geo), photo depth (vision.depth, \
                 scope vision), image and video metadata with the sun's position (media.metadata) and landform measures \
                 (terrain.measure). To build from a reference picture use the prompt `terrain_from_reference` and the guide \
                 jd://guide/reading-a-landscape; jd://guide/environments covers soil, weather, deserts, snow and lava. Every \
                 result is a JSON envelope {status, result|error, warnings}; errors carry stable E_* codes with `why` and `next`.",
            )
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        let prompt = Prompt::new(
            PROMPT_REFERENCE,
            Some("Build a believable world from a reference photo, 360° panorama or video: read its metadata and the sun, read the landform, ground it in a real DEM when the place is known, simulate the processes it shows, and measure the result against a real terrain of the same kind."),
            Some(vec![
                PromptArgument::new("image").with_description("Path of the photo, panorama or video (absolute, or relative to the project's assets/).").with_required(true),
                PromptArgument::new("place").with_description("Where it was taken, if the file has no GPS (a place name or lon, lat).").with_required(false),
                PromptArgument::new("intent").with_description("What the world is for, or what to change from the reference.").with_required(false),
            ]),
        );
        std::future::ready(Ok(ListPromptsResult::with_all_items(vec![prompt])))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, McpError>> + Send + '_ {
        let result = if request.name == PROMPT_REFERENCE {
            let args = request.arguments.unwrap_or_default();
            if args
                .get("image")
                .and_then(Value::as_str)
                .is_none_or(|v| v.trim().is_empty())
            {
                Err(McpError::invalid_params("`image` is required", None))
            } else {
                Ok(GetPromptResponse::Complete(
                    GetPromptResult::new(vec![PromptMessage::new_text(
                        Role::User,
                        reference_prompt_text(&args),
                    )])
                    .with_description("From a reference image to a measured world"),
                ))
            }
        } else {
            Err(McpError::invalid_params(
                format!("unknown prompt `{}`", request.name),
                None,
            ))
        };
        std::future::ready(result)
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let items = RESOURCES
            .iter()
            .map(|(uri, name, desc, mime, text)| {
                let mut r = Resource::new(*uri, *name)
                    .with_description(*desc)
                    .with_mime_type(*mime);
                r.size = Some(text.len() as u64);
                r
            })
            .collect();
        std::future::ready(Ok(ListResourcesResult::with_all_items(items)))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, McpError>> + Send + '_ {
        let result = match RESOURCES.iter().find(|r| r.0 == request.uri) {
            Some((uri, _, _, mime, text)) => {
                Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
                    vec![ResourceContents::text(*text, *uri).with_mime_type(*mime)],
                )))
            }
            None => Err(McpError::resource_not_found(
                format!("unknown resource `{}`", request.uri),
                None,
            )),
        };
        std::future::ready(result)
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
                    // Structured content is always the full envelope (status/result/error/warnings),
                    // so advertise that shape; the op's own result schema is nested under `result`
                    // and discoverable via `jd schema <op>`.
                    .with_raw_output_schema(self.envelope_schema.clone())
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
        // Checks that need `&self` happen now; the operation itself runs on a blocking thread so
        // a long evaluation never pins a tokio worker (otherwise `jobs.cancel` could starve).
        let pending: Result<Request, Value> = match self.registry.get(&op) {
            None => Err(envelope_error(
                &op,
                JdError::new(ErrorCode::E_INPUT_INVALID, format!("unknown tool `{op}`")),
            )),
            Some(def) if !self.allowed(def.scope) => Err(envelope_error(
                &op,
                JdError::new(
                    ErrorCode::E_PERMISSION,
                    format!("scope `{}` not granted", scope_name(def.scope)),
                )
                .next(format!(
                    "Restart jd-mcp with --scope {}.",
                    scope_name(def.scope)
                )),
            )),
            Some(_) => match self.check_workspace(&params) {
                Err(e) => Err(envelope_error(&op, e)),
                Ok(()) => {
                    let mut req = Request::new(&op, params);
                    req.client = Some(jd_core::envelope::Client {
                        kind: "mcp".into(),
                        name: "jd-mcp".into(),
                        scopes: (*self.scopes).clone(),
                    });
                    Ok(req)
                }
            },
        };
        let registry = self.registry.clone();
        let ctx = self.ctx.clone();
        async move {
            let value = match pending {
                Err(v) => v,
                Ok(req) => {
                    match tokio::task::spawn_blocking(move || registry.invoke(&ctx, req)).await {
                        Ok(resp) => serde_json::to_value(&resp).unwrap_or(Value::Null),
                        Err(e) => envelope_error(
                            &op,
                            JdError::new(ErrorCode::E_INTERNAL, format!("operation panicked: {e}")),
                        ),
                    }
                }
            };
            let is_error = value.get("status").and_then(Value::as_str) == Some("error");
            let result = if is_error {
                CallToolResult::structured_error(value)
            } else {
                CallToolResult::structured(value)
            };
            Ok(CallToolResponse::Complete(result))
        }
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
        envelope_schema: envelope_schema(),
    };
    tracing::info!(scopes = ?server.scopes, workspace = ?server.workspace, "jd-mcp starting on stdio");
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
