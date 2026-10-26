//! The headless engine server runs real operations for a client end to end, and its loss is
//! reported as `E_ENGINE_UNAVAILABLE`.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use jd_core::{Request, Status};
use jd_ipc::{Client, ClientConfig};
use jd_types::{ErrorCode, JdError};
use serde_json::{Value, json};

type Events = Arc<Mutex<Vec<(String, Value)>>>;

fn studio() -> (Child, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jd-studio"))
        .arg("--headless")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn jd-studio");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("ready line");
    (child, serde_json::from_str(&line).expect("ready json"))
}

fn connect(ready: &Value) -> (Client, Events, mpsc::Receiver<JdError>) {
    let events: Events = Arc::default();
    let (tx, rx) = mpsc::channel();
    let (ev, tx) = (events.clone(), Mutex::new(tx));
    let c = Client::connect(
        ClientConfig::new(
            ready["socket"].as_str().unwrap(),
            ready["token"].as_str().unwrap(),
            "test",
        ),
        Box::new(move |t, d| ev.lock().unwrap().push((t.to_owned(), d.clone()))),
        Box::new(move |e| {
            let _ = tx.lock().unwrap().send(e.clone());
        }),
    )
    .expect("connect");
    (c, events, rx)
}

fn ok(c: &Client, op: &str, params: Value) -> Value {
    let r = c.call(Request::new(op, params)).expect("transport");
    assert_eq!(r.status, Status::Ok, "{op}: {:?}", r.error);
    r.result.expect("result")
}

fn changed_ops(events: &Events) -> Vec<String> {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let ops: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == "project_changed")
            .map(|(_, d)| d["op"].as_str().unwrap().to_owned())
            .collect();
        if ops.len() >= 2 || Instant::now() > until {
            return ops;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_engine_serves_real_operations_and_its_loss_fails_calls() {
    let (mut child, ready) = studio();
    let (c, events, lost) = connect(&ready);
    assert_eq!(c.session(), ready["session"]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Ipc.jdproj");
    ok(&c, "project.create", json!({ "path": path, "name": "Ipc" }));
    ok(&c, "terrain.generate", json!({ "layer": "height" }));
    let pv = ok(
        &c,
        "terrain.preview",
        json!({ "layer": "height", "lod": 3 }),
    );
    assert!(pv["width"].as_u64().unwrap() > 0 && !pv["bytes_base64"].as_str().unwrap().is_empty());
    assert_eq!(changed_ops(&events), ["project.create", "terrain.generate"]);
    assert!(
        c.list_ops()
            .unwrap()
            .iter()
            .any(|o| o.id == "terrain.generate")
    );
    // The project's log names the client.
    let log = std::fs::read_to_string(path.join("logs/ops.log")).unwrap();
    assert!(
        log.lines().all(|l| l.contains(r#""client":"gui-test""#)),
        "{log}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        lost.recv_timeout(Duration::from_secs(5))
            .expect("on_lost")
            .code,
        ErrorCode::E_ENGINE_UNAVAILABLE
    );
    let err = c
        .call(Request::new("project.info", json!({})))
        .expect_err("engine gone");
    assert_eq!(err.code, ErrorCode::E_ENGINE_UNAVAILABLE);
}

#[test]
fn closing_stdin_shuts_the_engine_down_cleanly() {
    let (mut child, ready) = studio();
    let (_c, events, lost) = connect(&ready);
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|(t, d)| t == "shutdown" && d["reason"] == "Studio stopped")
    );
    assert!(
        lost.recv_timeout(Duration::from_secs(5))
            .unwrap()
            .message
            .contains("Studio stopped")
    );
    #[cfg(unix)]
    assert!(!std::path::Path::new(ready["socket"].as_str().unwrap()).exists());
}
