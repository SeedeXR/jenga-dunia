//! The Editor supervisor's policy with stand-in executables (Unix shell scripts): a crash is
//! restarted, repeated crashes stop the restarts, a clean exit is "closed", and the Editor gets
//! the socket and token in its environment.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use jd_studio::editor::{self, Editor, Status};

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
fn crashes_are_restarted_until_they_repeat_and_a_clean_exit_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let env_out = dir.path().join("env.txt");
    let crash = script(
        dir.path(),
        "crash.sh",
        &format!(
            "echo \"$JD_ENGINE_SOCKET $JD_ENGINE_TOKEN\" > '{}'; exit 3",
            env_out.display()
        ),
    );
    let (tx, rx) = mpsc::channel();
    let mut e = Editor::start(crash, "/tmp/sock", "tok", move |s| tx.send(s).unwrap());
    assert!(matches!(e.status, Status::Running(_)));
    let mut restarts = 0;
    loop {
        let status = rx.recv_timeout(Duration::from_secs(10)).expect("exit seen");
        assert_eq!(status.code(), Some(3));
        e.exited(status);
        match &e.status {
            Status::Running(_) => restarts += 1,
            Status::GaveUp(msg) => {
                assert!(msg.contains("crashed 4 times"), "{msg}");
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(restarts, editor::MAX_CRASHES);
    assert_eq!(
        std::fs::read_to_string(&env_out).unwrap().trim(),
        "/tmp/sock tok"
    );

    let (tx, rx) = mpsc::channel();
    let ok = script(dir.path(), "ok.sh", "exit 0");
    let mut e = Editor::start(ok, "/tmp/sock", "tok", move |s| tx.send(s).unwrap());
    e.exited(rx.recv_timeout(Duration::from_secs(10)).unwrap());
    assert_eq!(e.status, Status::Closed);
}

#[test]
fn a_missing_editor_is_reported_and_a_hung_one_is_killed_on_stop() {
    let err = editor::locate(Some(PathBuf::from("/nonexistent/jd-app"))).unwrap_err();
    assert!(err.contains("does not exist"), "{err}");
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let hang = script(dir.path(), "hang.sh", "exec sleep 30");
    let mut e = Editor::start(hang, "s", "t", move |s| {
        let _ = tx.send(s);
    });
    let started = std::time::Instant::now();
    e.stop(Duration::from_millis(200));
    assert!(started.elapsed() < Duration::from_secs(5));
}
