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

#[tokio::test]
async fn the_reference_prompt_resources_metadata_and_measure_work_through_mcp() {
    use rmcp::model::{GetPromptRequestParams, ReadResourceRequestParams, ResourceContents};
    let dir = tempfile::tempdir().expect("tmp");
    let ws = dir.path().to_path_buf();
    let client = connect(&["--scope", "all", "--workspace", ws.to_str().expect("utf8")]).await;

    // Resources: the guides and catalogues, readable as text.
    let res = client.list_all_resources().await.expect("resources");
    let uris: Vec<&str> = res.iter().map(|r| r.uri.as_str()).collect();
    assert_eq!(
        uris,
        [
            "jd://guide/reading-a-landscape",
            "jd://guide/environments",
            "jd://schema/node-kinds",
            "jd://schema/ops"
        ]
    );
    let read = client
        .read_resource(ReadResourceRequestParams::new(
            "jd://guide/reading-a-landscape",
        ))
        .await
        .expect("read");
    let ResourceContents::TextResourceContents { text, .. } = &read.contents[0] else {
        panic!("text")
    };
    assert!(
        text.contains("## The reading sheet") && text.contains("Exemplars"),
        "guide text"
    );
    let read = client
        .read_resource(ReadResourceRequestParams::new("jd://schema/node-kinds"))
        .await
        .expect("read");
    let ResourceContents::TextResourceContents { text, .. } = &read.contents[0] else {
        panic!("text")
    };
    let kinds: Value = serde_json::from_str(text).expect("json");
    assert!(
        kinds
            .as_array()
            .expect("array")
            .iter()
            .any(|k| k["kind"] == "jd.volcanic.lava@1")
    );
    assert!(
        client
            .read_resource(ReadResourceRequestParams::new("jd://nope"))
            .await
            .is_err()
    );

    // The prompt: needs `image`, carries the steps and the guide.
    let prompts = client.list_all_prompts().await.expect("prompts");
    assert_eq!(
        prompts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["terrain_from_reference"]
    );
    let mut args = Map::new();
    args.insert("image".into(), json!("ref.jpg"));
    args.insert("place".into(), json!("Sossusvlei, Namibia"));
    let got = client
        .get_prompt(GetPromptRequestParams::new("terrain_from_reference").with_arguments(args))
        .await
        .expect("prompt");
    let msg = serde_json::to_string(&got.messages).expect("json");
    assert!(
        msg.contains("media.metadata")
            && msg.contains("terrain.measure")
            && msg.contains("Sossusvlei"),
        "{msg}"
    );
    assert!(
        client
            .get_prompt(GetPromptRequestParams::new("terrain_from_reference"))
            .await
            .is_err(),
        "image is required"
    );

    // media.metadata on a tiny MP4 that only states where and when (Android ©xyz, mvhd).
    let bx = |ty: &[u8; 4], payload: &[u8]| -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend(ty);
        v.extend(payload);
        v
    };
    // 2026-06-21 10:00:00 UTC in seconds since 1904-01-01.
    let created: u32 = 3_864_880_800;
    let mut mvhd = vec![0u8; 4];
    mvhd.extend(created.to_be_bytes());
    mvhd.extend(created.to_be_bytes());
    mvhd.extend(1000u32.to_be_bytes());
    mvhd.extend(5000u32.to_be_bytes());
    mvhd.extend([0u8; 80]);
    let mut xyz = vec![0, 18, 0x15, 0xc7];
    xyz.extend(b"-24.7500+015.3500/");
    let file = [
        bx(b"ftyp", b"isom\0\0\0\0isom"),
        bx(
            b"moov",
            &[bx(b"mvhd", &mvhd), bx(b"udta", &bx(b"\xa9xyz", &xyz))].concat(),
        ),
    ]
    .concat();
    let clip = ws.join("clip.mp4");
    std::fs::write(&clip, file).expect("write");
    let env = call(&client, "media.metadata", json!({ "path": clip })).await;
    assert_eq!(env["status"], "ok", "{env}");
    let r = &env["result"];
    assert_eq!(
        (r["kind"].as_str(), r["duration_s"].as_f64()),
        (Some("video"), Some(5.0))
    );
    assert_eq!(
        r["suggest"]["geo_fetch_dem"]["center"],
        json!([15.35, -24.75])
    );
    assert!(
        r["sun"]["elevation_deg"].as_f64().expect("sun") > 30.0,
        "midwinter morning in the Namib: {r}"
    );
    assert!(r["suggest"]["sky"]["sun_azimuth_deg"].is_number());

    // terrain.measure on a fresh world.
    let env = call(
        &client,
        "project.create",
        json!({ "path": ws.join("m.jdproj"), "name": "M" }),
    )
    .await;
    assert_eq!(env["status"], "ok", "{env}");
    assert_eq!(
        call(&client, "graph.evaluate", json!({})).await["status"],
        "ok"
    );
    let env = call(&client, "terrain.measure", json!({ "layer": "height" })).await;
    assert_eq!(env["status"], "ok", "{env}");
    let shares: f64 = env["result"]["geomorphons"]
        .as_object()
        .expect("shares")
        .values()
        .filter_map(Value::as_f64)
        .sum();
    assert!(
        (shares - 1.0).abs() < 1e-6 && env["result"]["relief_m"].as_f64().expect("relief") > 0.0,
        "{env}"
    );
    client.cancel().await.expect("shutdown");
}
