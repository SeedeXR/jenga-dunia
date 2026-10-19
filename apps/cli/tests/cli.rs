#![allow(clippy::expect_used, clippy::unwrap_used)]
//! CLI-driven tests: the `jd` binary end to end, JSON envelopes and exit codes.

use assert_cmd::Command;
use serde_json::Value;

fn jd() -> Command {
    Command::cargo_bin("jd").expect("jd binary built")
}

fn last_json_line(stdout: &[u8]) -> Value {
    let text = String::from_utf8_lossy(stdout);
    let line = text
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .expect("at least one line");
    serde_json::from_str(line).expect("valid JSON envelope")
}

#[test]
fn new_then_open_round_trip_with_json_envelopes() {
    let dir = tempfile::tempdir().expect("tmp");
    let proj = dir.path().join("Hills.jdproj");

    let out = jd()
        .args(["--json", "new"])
        .arg(&proj)
        .args(["--extent", "2048", "--resolution", "1024", "--seed", "7"])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let env = last_json_line(&out.stdout);
    assert_eq!(env["op"], "project.create");
    assert_eq!(env["status"], "ok");
    assert_eq!(env["result"]["name"], "Hills");
    assert_eq!(
        env["result"]["world"]["resolution_px"],
        serde_json::json!([1024, 1024])
    );
    assert_eq!(env["result"]["world"]["seed"], 7);
    assert!(proj.join("project.json").exists());

    let out = jd()
        .args(["--json", "open"])
        .arg(&proj)
        .output()
        .expect("run");
    assert!(out.status.success());
    let env = last_json_line(&out.stdout);
    assert_eq!(env["op"], "project.open");
    assert_eq!(env["result"]["name"], "Hills");
    assert_eq!(env["warnings"], Value::Null, "clean open has no warnings");
}

#[test]
fn op_subcommand_mutates_and_saves() {
    let dir = tempfile::tempdir().expect("tmp");
    let proj = dir.path().join("p.jdproj");
    jd().arg("new").arg(&proj).assert().success();

    let out = jd()
        .args([
            "--json",
            "op",
            "project.set_name",
            "--params",
            r#"{"name":"Renamed"}"#,
            "--save",
            "--project",
        ])
        .arg(&proj)
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(
        lines
            .iter()
            .map(|v| v["op"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["project.open", "project.set_name", "project.save"]
    );
    assert_eq!(lines[2]["result"]["name"], "Renamed");
    assert_eq!(lines[2]["result"]["dirty"], false);

    let out = jd()
        .args(["--json", "open"])
        .arg(&proj)
        .output()
        .expect("run");
    assert_eq!(last_json_line(&out.stdout)["result"]["name"], "Renamed");
}

#[test]
fn errors_map_to_documented_exit_codes() {
    // Missing project → E_PROJECT_CORRUPT → exit 1
    let out = jd()
        .args(["--json", "open", "/definitely/not/here.jdproj"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    let env = last_json_line(&out.stdout);
    assert_eq!(env["status"], "error");
    assert_eq!(env["error"]["code"], "E_PROJECT_CORRUPT");
    assert!(
        env["error"]["next"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    );

    // Invalid params → E_INPUT_INVALID → exit 2
    let out = jd()
        .args([
            "--json",
            "op",
            "project.set_name",
            "--params",
            r#"{"bogus":1}"#,
        ])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        last_json_line(&out.stdout)["error"]["code"],
        "E_INPUT_INVALID"
    );

    // Human mode prints why/next on stderr
    let out = jd()
        .args(["open", "/definitely/not/here.jdproj"])
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("E_PROJECT_CORRUPT") && err.contains("next:"),
        "{err}"
    );
}

#[test]
fn schema_lists_ops_and_prints_one() {
    let out = jd().args(["schema", "--list"]).output().expect("run");
    assert!(out.status.success());
    let list: Value = serde_json::from_slice(&out.stdout).expect("json array");
    let ids: Vec<&str> = list
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|o| o["id"].as_str())
        .collect();
    for required in [
        "project.create",
        "project.open",
        "project.save",
        "project.recover",
        "diag.env",
    ] {
        assert!(ids.contains(&required), "missing {required}");
    }
    let out = jd()
        .args(["schema", "project.create"])
        .output()
        .expect("run");
    let info: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(
        info["request_schema"]["required"]
            .as_array()
            .map(|a| a.contains(&Value::String("path".into()))),
        Some(true)
    );
    jd().args(["schema", "no.such_op"]).assert().code(2);
}

#[test]
fn new_without_extension_appends_jdproj() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = jd()
        .args(["--json", "new"])
        .arg(dir.path().join("Dunes"))
        .output()
        .expect("run");
    assert!(out.status.success());
    assert!(
        dir.path()
            .join("Dunes.jdproj")
            .join("project.json")
            .exists()
    );
    assert_eq!(last_json_line(&out.stdout)["result"]["name"], "Dunes");
}

#[test]
fn rejected_mutation_leaves_no_journal_entry_and_save_refuses_pending_recovery() {
    let dir = tempfile::tempdir().expect("tmp");
    let proj = dir.path().join("p.jdproj");
    jd().arg("new").arg(&proj).assert().success();
    // Invalid name → exit 2 and NO journal entry.
    jd().args([
        "op",
        "project.set_name",
        "--params",
        r#"{"name":"  "}"#,
        "--project",
    ])
    .arg(&proj)
    .assert()
    .code(2);
    let journal = std::fs::read_to_string(proj.join("journal/ops.ndjson")).unwrap_or_default();
    assert!(
        journal.trim().is_empty(),
        "journal must stay empty after a rejected mutation: {journal}"
    );
    // Simulate a crash with a pending entry, then `--save` without `--recover` must refuse.
    std::fs::write(
        proj.join("journal/ops.ndjson"),
        "{\"ts\":\"t\",\"op\":\"project.set_name\",\"params\":{\"name\":\"Crashed\"}}\n",
    )
    .expect("write");
    let out = jd()
        .args(["--json", "op", "project.info", "--save", "--project"])
        .arg(&proj)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    let lines: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(
        lines.last().and_then(|v| v["error"]["code"].as_str()),
        Some("E_INPUT_INVALID")
    );
    // Recovery replays it and saves.
    let out = jd()
        .args(["--json", "open", "--recover"])
        .arg(&proj)
        .output()
        .expect("run");
    assert!(out.status.success());
    assert_eq!(
        last_json_line(&out.stdout)["result"]["replayed"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        last_json_line(
            &jd()
                .args(["--json", "open"])
                .arg(&proj)
                .output()
                .expect("run")
                .stdout
        )["result"]["name"],
        "Crashed"
    );
}

#[test]
fn diag_env_reports_memory_and_tools() {
    let out = jd().args(["--json", "diag", "env"]).output().expect("run");
    assert!(out.status.success());
    let env = last_json_line(&out.stdout);
    assert!(env["result"]["memory_total_bytes"].as_u64().unwrap_or(0) > 0);
    assert!(
        env["result"]["tools"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false)
    );
}
