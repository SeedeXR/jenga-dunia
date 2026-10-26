//! The protocol against real local sockets (`docs/architecture/14-studio-editor-protocol.md`, Tests).
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::io::BufReader;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use interprocess::local_socket::{ListenerOptions, Stream, prelude::*};
use jd_core::{Context, Registry, Request, Scope, Status};
use jd_ipc::{Client, ClientConfig, Endpoint, Frame, Server, name_of, read_frame, write_frame};
use jd_types::{ErrorCode, JdError};
use serde_json::{Value, json};

/// A registry with a few test operations.
fn registry() -> Registry {
    let mut r = Registry::default();
    r.register::<Value, Value, _>("test.echo", "echo", Scope::None, |_, p| Ok((p, vec![])));
    r.register::<Value, Value, _>("test.sleep", "sleep then echo", Scope::None, |_, p| {
        std::thread::sleep(Duration::from_millis(p["ms"].as_u64().unwrap_or(0)));
        Ok((p, vec![]))
    });
    r.register::<Value, Value, _>(
        "test.write",
        "a project write",
        Scope::ProjectWrite,
        |_, p| Ok((p, vec![])),
    );
    r.register::<Value, Value, _>("test.fail", "always fails", Scope::None, |_, _| {
        Err(JdError::new(ErrorCode::E_INPUT_INVALID, "nope"))
    });
    r.register::<Value, Value, _>(
        "test.big",
        "a reply over the frame cap",
        Scope::None,
        |_, _| Ok((json!("x".repeat(jd_ipc::MAX_LINE + 1)), vec![])),
    );
    r
}

fn server() -> Server {
    Server::bind(registry(), Context::default()).expect("bind")
}

type Events = Arc<Mutex<Vec<(String, Value)>>>;

/// A client recording its events; `lost` receives the loss.
fn client(
    s: &Server,
    cfg: impl FnOnce(&mut ClientConfig),
) -> (Client, Events, mpsc::Receiver<JdError>) {
    let mut c = ClientConfig::new(s.socket(), s.token(), "test");
    cfg(&mut c);
    connect(c)
}

fn connect(c: ClientConfig) -> (Client, Events, mpsc::Receiver<JdError>) {
    let events: Events = Arc::default();
    let (tx, rx) = mpsc::channel();
    let ev = events.clone();
    let tx = Mutex::new(tx);
    let client = Client::connect(
        c,
        Box::new(move |t, d| ev.lock().unwrap().push((t.to_owned(), d.clone()))),
        Box::new(move |e| {
            let _ = tx.lock().unwrap().send(e.clone());
        }),
    )
    .expect("connect");
    (client, events, rx)
}

fn wait_for(events: &Events, topic: &str) -> Value {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if let Some((_, d)) = events.lock().unwrap().iter().find(|(t, _)| t == topic) {
            return d.clone();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no `{topic}` event");
}

#[test]
fn a_call_round_trips_and_operation_errors_keep_the_connection() {
    let s = server();
    let (c, _, _) = client(&s, |_| {});
    assert_eq!(c.session(), s.session());
    let r = c
        .call(Request::new("test.echo", json!({ "a": 1 })))
        .expect("call");
    assert_eq!(r.status, Status::Ok);
    assert_eq!(r.result, Some(json!({ "a": 1 })));
    let r = c
        .call(Request::new("test.fail", json!({})))
        .expect("op errors are data");
    assert_eq!(r.status, Status::Error);
    assert_eq!(r.error.expect("error").code, ErrorCode::E_INPUT_INVALID);
    let r = c.call(Request::new("no.such_op", json!({}))).expect("call");
    assert_eq!(r.error.expect("error").code, ErrorCode::E_INPUT_INVALID);
    let r = c
        .call(Request::new("test.big", json!({})))
        .expect("too big is data too");
    assert_eq!(r.error.expect("error").code, ErrorCode::E_SERIALIZE);
    let ops = c.list_ops().expect("ops");
    assert!(ops.iter().any(|o| o.id == "test.echo"));
    assert!(
        c.call(Request::new("test.echo", json!(2)))
            .expect("still fine")
            .status
            == Status::Ok
    );
    assert!(c.lost().is_none());
}

#[test]
fn a_wrong_token_and_a_protocol_mismatch_are_refused() {
    let s = server();
    let err = Client::connect(
        ClientConfig::new(s.socket(), "not-the-token", "test"),
        Box::new(|_, _| {}),
        Box::new(|_| {}),
    )
    .err()
    .expect("refused");
    assert_eq!(err.code, ErrorCode::E_PERMISSION);

    let raw = Stream::connect(name_of(s.socket()).unwrap()).expect("connect");
    write_frame(
        &mut &raw,
        &Frame::Hello {
            protocol: 999,
            role: "test".into(),
            pid: 1,
            token: s.token().into(),
            app_version: "x".into(),
        },
    )
    .unwrap();
    match read_frame(&mut BufReader::new(&raw)).expect("read") {
        Some(Frame::Error { error }) => {
            assert_eq!(error.code, ErrorCode::E_COMPAT);
            assert_eq!(error.details["client_protocol"], 999);
        }
        other => panic!("expected an E_COMPAT refusal, got {other:?}"),
    }

    let err = Client::connect(
        ClientConfig::new(format!("{}-missing", s.socket()), s.token(), "test"),
        Box::new(|_, _| {}),
        Box::new(|_| {}),
    )
    .err()
    .expect("nothing listens there");
    assert_eq!(err.code, ErrorCode::E_ENGINE_UNAVAILABLE);
}

#[test]
fn a_hundred_concurrent_calls_are_matched_to_their_replies() {
    let s = server();
    let (c, _, _) = client(&s, |_| {});
    let c = Arc::new(c);
    let threads: Vec<_> = (0..100u64)
        .map(|i| {
            let c = c.clone();
            std::thread::spawn(move || {
                // Later calls finish first, so replies arrive out of order.
                let r = c
                    .call(Request::new(
                        "test.sleep",
                        json!({ "i": i, "ms": (100 - i) % 7 * 5 }),
                    ))
                    .expect("call");
                assert_eq!(r.result.expect("result")["i"], i);
            })
        })
        .collect();
    for t in threads {
        t.join().expect("thread");
    }
}

#[test]
fn a_call_that_outlives_its_timeout_fails_and_the_connection_survives() {
    let s = server();
    let (c, _, _) = client(&s, |c| c.call_timeout = Duration::from_millis(200));
    let err = c
        .call(Request::new("test.sleep", json!({ "ms": 1000 })))
        .expect_err("timeout");
    assert_eq!(err.code, ErrorCode::E_TIMEOUT);
    assert_eq!(
        c.call(Request::new("test.echo", json!(1)))
            .expect("call")
            .status,
        Status::Ok
    );
}

#[test]
fn project_writes_and_emitted_events_reach_every_client() {
    let s = server();
    let (a, ev_a, _) = client(&s, |_| {});
    let (_b, ev_b, _) = client(&s, |_| {});
    a.call(Request::new("test.echo", json!(1)))
        .expect("read-only op");
    a.call(Request::new("test.write", json!(1)))
        .expect("write op");
    for ev in [&ev_a, &ev_b] {
        let d = wait_for(ev, "project_changed");
        assert_eq!(d["op"], "test.write");
        assert_eq!(d["scope"], "project-write");
        assert!(d["request_id"].is_string());
    }
    // Only the write announced a change.
    assert_eq!(
        ev_a.lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == "project_changed")
            .count(),
        1
    );
    s.emit(
        "selection",
        json!({ "nodes": ["n1"], "source": "viewport" }),
    );
    assert_eq!(wait_for(&ev_b, "selection")["nodes"][0], "n1");
}

#[test]
fn shutdown_tells_clients_and_fails_later_calls() {
    let s = server();
    let (c, ev, lost) = client(&s, |_| {});
    s.shutdown("studio quit");
    assert_eq!(wait_for(&ev, "shutdown")["reason"], "studio quit");
    let e = lost.recv_timeout(Duration::from_secs(5)).expect("on_lost");
    assert_eq!(e.code, ErrorCode::E_ENGINE_UNAVAILABLE);
    assert!(e.message.contains("studio quit"), "{}", e.message);
    assert_eq!(e.details["orderly"], true);
    let err = c
        .call(Request::new("test.echo", json!(1)))
        .expect_err("gone");
    assert_eq!(err.code, ErrorCode::E_ENGINE_UNAVAILABLE);
}

/// A hand-rolled engine: welcomes the client, then runs `then` with the connection.
fn fake_engine(
    then: impl FnOnce(Stream) + Send + 'static,
) -> (Endpoint, std::thread::JoinHandle<()>) {
    let ep = Endpoint::new().unwrap();
    let listener = ListenerOptions::new()
        .name(name_of(&ep.socket).unwrap())
        .create_sync()
        .unwrap();
    let t = std::thread::spawn(move || {
        let conn = listener.incoming().next().unwrap().unwrap();
        let mut r = BufReader::new(&conn);
        assert!(matches!(
            read_frame(&mut r).unwrap(),
            Some(Frame::Hello { .. })
        ));
        write_frame(
            &mut &conn,
            &Frame::Welcome {
                protocol: jd_ipc::PROTOCOL,
                engine_version: "x".into(),
                session: "s".into(),
            },
        )
        .unwrap();
        then(conn);
    });
    (ep, t)
}

#[test]
fn an_engine_that_dies_mid_call_fails_the_pending_call_at_once() {
    let (ep, t) = fake_engine(|conn| {
        // Read the call, then vanish without replying.
        let mut r = BufReader::new(&conn);
        assert!(matches!(
            read_frame(&mut r).unwrap(),
            Some(Frame::Call { .. })
        ));
    });
    let (c, _, lost) = connect(ClientConfig::new(&ep.socket, "t", "test"));
    let started = Instant::now();
    let err = c
        .call(Request::new("x.y", json!({})))
        .expect_err("engine died");
    assert_eq!(err.code, ErrorCode::E_ENGINE_UNAVAILABLE);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        lost.recv_timeout(Duration::from_secs(1))
            .expect("on_lost")
            .code,
        ErrorCode::E_ENGINE_UNAVAILABLE
    );
    t.join().unwrap();
}

#[test]
fn an_engine_that_hangs_is_declared_lost_within_the_heartbeat_window() {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ep, t) = fake_engine(move |conn| {
        // Alive but silent: never answers a ping or a call.
        let _ = stop_rx.recv();
        drop(conn);
    });
    let mut cfg = ClientConfig::new(&ep.socket, "t", "test");
    cfg.heartbeat = Duration::from_millis(50);
    cfg.dead_after = Duration::from_millis(300);
    let (c, _, lost) = connect(cfg);
    let started = Instant::now();
    let err = c
        .call(Request::new("x.y", json!({})))
        .expect_err("engine hung");
    assert_eq!(err.code, ErrorCode::E_ENGINE_UNAVAILABLE);
    assert!(err.message.contains("no answer"), "{}", err.message);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(lost.recv_timeout(Duration::from_secs(1)).is_ok());
    stop_tx.send(()).unwrap();
    t.join().unwrap();
}
