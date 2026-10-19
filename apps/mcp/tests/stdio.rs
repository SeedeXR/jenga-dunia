#![allow(clippy::expect_used, clippy::unwrap_used)]
//! MCP-driven tests: spawn the real `jd-mcp` binary over stdio and drive it with an rmcp client.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResponse};
use rmcp::transport::TokioChildProcess;
use serde_json::{Map, Value, json};

async fn connect(args: &[&str]) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_jd-mcp"));
    cmd.args(args).env("JD_LOG", "warn");
    let transport = TokioChildProcess::new(cmd).expect("spawn jd-mcp");
    ().serve(transport).await.expect("MCP handshake")
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: Value,
) -> Value {
    let arguments: Map<String, Value> = args.as_object().cloned().unwrap_or_default();
    let resp = client
        .call_tool_once(CallToolRequestParams::new(name.to_owned()).with_arguments(arguments))
        .await
        .expect("tool call transport ok");
    match resp {
        CallToolResponse::Complete(r) => r.structured_content.expect("structured envelope"),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn lists_tools_with_schemas_filtered_by_scope() {
    let client = connect(&["--scope", "project:ro"]).await;
    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(names.contains(&"project.open"), "{names:?}");
    assert!(names.contains(&"diag.env"), "{names:?}");
    assert!(
        !names.contains(&"project.set_name"),
        "project:rw not granted: {names:?}"
    );
    assert!(
        !names.contains(&"project.create"),
        "fs:write not granted: {names:?}"
    );
    let open = tools
        .iter()
        .find(|t| t.name == "project.open")
        .expect("project.open");
    assert_eq!(open.input_schema.get("required"), Some(&json!(["path"])));
    assert!(open.output_schema.is_some());
    client.cancel().await.expect("shutdown");
}

#[tokio::test]
async fn create_rename_recover_through_mcp() {
    let dir = tempfile::tempdir().expect("tmp");
    let ws = dir.path().to_path_buf();
    let proj = ws.join("mcp.jdproj");
    let client = connect(&["--scope", "all", "--workspace", ws.to_str().expect("utf8")]).await;

    let env = call(
        &client,
        "project.create",
        json!({ "path": proj, "name": "Mcp" }),
    )
    .await;
    assert_eq!(env["status"], "ok", "{env}");
    let env = call(&client, "project.set_name", json!({ "name": "Renamed" })).await;
    assert_eq!(env["result"]["name"], "Renamed");
    // Simulate a crash: close without saving, reopen in a fresh server.
    let env = call(&client, "project.close", json!({})).await;
    assert_eq!(env["status"], "ok");
    client.cancel().await.expect("shutdown");

    let client = connect(&["--scope", "all", "--workspace", ws.to_str().expect("utf8")]).await;
    let env = call(&client, "project.open", json!({ "path": proj })).await;
    assert_eq!(
        env["result"]["name"], "Mcp",
        "unsaved rename is not on disk"
    );
    assert!(
        env["warnings"]
            .as_array()
            .map(|w| w.iter().any(|x| x["code"] == "W_RECOVERY_AVAILABLE"))
            .unwrap_or(false),
        "{env}"
    );
    let env = call(&client, "project.recover", json!({})).await;
    assert_eq!(env["result"]["replayed"].as_array().map(Vec::len), Some(1));
    let env = call(&client, "project.info", json!({})).await;
    assert_eq!(env["result"]["name"], "Renamed");

    // Outside the workspace → E_PERMISSION, as a structured error envelope.
    let env = call(
        &client,
        "project.open",
        json!({ "path": "/tmp/outside.jdproj" }),
    )
    .await;
    assert_eq!(env["status"], "error");
    assert_eq!(env["error"]["code"], "E_PERMISSION");
    client.cancel().await.expect("shutdown");
}

#[tokio::test]
async fn agent_can_generate_and_export_terrain() {
    let dir = tempfile::tempdir().expect("tmp");
    let ws = dir.path().to_path_buf();
    let proj = ws.join("terrain.jdproj");
    let client = connect(&["--scope", "all", "--workspace", ws.to_str().expect("utf8")]).await;
    let env = call(
        &client,
        "project.create",
        json!({ "path": proj, "name": "T", "world": { "extent_m": [512.0, 512.0], "resolution_px": [128, 128], "tile_size": 64, "halo": 2, "origin": [0.0, 0.0], "seed": 1 } }),
    )
    .await;
    assert_eq!(env["status"], "ok", "{env}");
    let env = call(
        &client,
        "terrain.generate",
        json!({ "noise": { "seed": 7, "fractal": "ridged" } }),
    )
    .await;
    assert_eq!(env["status"], "ok", "{env}");
    assert_eq!(env["result"]["stats"]["non_finite"], 0);
    let env = call(
        &client,
        "terrain.export_heightmap",
        json!({ "path": "agent.r16", "format": "raw_r16" }),
    )
    .await;
    assert_eq!(env["status"], "ok", "{env}");
    assert_eq!(
        std::fs::metadata(proj.join("exports/agent.r16"))
            .expect("r16")
            .len(),
        128 * 128 * 2
    );
    // Export outside the workspace is refused even though `export` scope is granted.
    let env = call(
        &client,
        "terrain.export_heightmap",
        json!({ "path": "/tmp/escape.r16", "format": "raw_r16" }),
    )
    .await;
    assert_eq!(env["error"]["code"], "E_PERMISSION", "{env}");
    client.cancel().await.expect("shutdown");
}

#[tokio::test]
async fn denied_scope_is_a_stable_error_not_a_crash() {
    let client = connect(&["--scope", "project:ro"]).await;
    let env = call(&client, "project.set_name", json!({ "name": "x" })).await;
    assert_eq!(env["error"]["code"], "E_PERMISSION");
    let env = call(&client, "no.such_tool", json!({})).await;
    assert_eq!(env["error"]["code"], "E_INPUT_INVALID");
    client.cancel().await.expect("shutdown");
}
