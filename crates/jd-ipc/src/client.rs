//! The Editor's (and tests') side: calls into Studio's engine, receives its events, and notices
//! when it goes away.

use std::collections::HashMap;
use std::io::{self, BufReader};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use interprocess::local_socket::{Stream, prelude::*};
use jd_core::ops::OperationInfo;
use jd_core::{Request, Response};
use jd_types::{ErrorCode, JdError, RequestId};
use serde_json::Value;

use crate::{
    Frame, FrameError, HANDSHAKE_TIMEOUT, PROTOCOL, SharedRead, name_of, read_frame, write_frame,
};

/// Called for every server event with its topic and data (on the client's reader thread).
pub type EventHook = Box<dyn Fn(&str, &Value) + Send + Sync>;
/// Called once when the engine is lost (not when the client closes on purpose).
pub type LostHook = Box<dyn Fn(&JdError) + Send + Sync>;

/// How to reach the engine.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Socket path / pipe name ([`crate::ENV_SOCKET`]).
    pub socket: String,
    /// Studio's launch token ([`crate::ENV_TOKEN`]).
    pub token: String,
    /// `editor`, `test`, …
    pub role: String,
    /// How long a call may wait for its reply (`E_TIMEOUT` after).
    pub call_timeout: Duration,
    /// Ping interval.
    pub heartbeat: Duration,
    /// Silence after which the engine counts as lost.
    pub dead_after: Duration,
}

impl ClientConfig {
    /// Defaults from the protocol spec: 10 min per call, ping every 2 s, lost after 6 s.
    pub fn new(socket: impl Into<String>, token: impl Into<String>, role: &str) -> Self {
        Self {
            socket: socket.into(),
            token: token.into(),
            role: role.to_owned(),
            call_timeout: Duration::from_secs(600),
            heartbeat: Duration::from_secs(2),
            dead_after: Duration::from_secs(6),
        }
    }

    /// From [`crate::ENV_SOCKET`] / [`crate::ENV_TOKEN`]; `None` when no socket is set.
    pub fn from_env(role: &str) -> Option<Self> {
        let socket = std::env::var(crate::ENV_SOCKET)
            .ok()
            .filter(|s| !s.is_empty())?;
        let token = std::env::var(crate::ENV_TOKEN).unwrap_or_default();
        Some(Self::new(socket, token, role))
    }
}

/// A connection to Studio's engine. Calls may run concurrently from any thread.
pub struct Client {
    shared: Arc<Shared>,
    session: String,
}

enum Answer {
    Response(Box<Response>),
    Ops(Vec<OperationInfo>),
}

struct Shared {
    cfg: ClientConfig,
    writer: Mutex<Option<Arc<Stream>>>,
    pending: Mutex<HashMap<u64, Sender<Result<Answer, JdError>>>>,
    lost: Mutex<Option<JdError>>,
    next_id: AtomicU64,
    last_seen: Mutex<Instant>,
    closing: AtomicBool,
    on_event: EventHook,
    on_lost: LostHook,
}

fn unavailable(socket: &str, what: impl std::fmt::Display) -> JdError {
    JdError::new(
        ErrorCode::E_ENGINE_UNAVAILABLE,
        format!("engine unavailable: {what}"),
    )
    .why("Studio, which runs the engine, is not running, refused the connection or went away.")
    .next("Start Jenga Dunia Studio; it launches the Editor.")
    .detail("socket", socket)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Client {
    /// Connect and shake hands. Fails with `E_ENGINE_UNAVAILABLE` (nothing listening),
    /// `E_COMPAT` (protocol mismatch) or `E_PERMISSION` (wrong token).
    pub fn connect(
        cfg: ClientConfig,
        on_event: EventHook,
        on_lost: LostHook,
    ) -> Result<Self, JdError> {
        let fail = |e: io::Error| unavailable(&cfg.socket, e);
        let stream = Arc::new(Stream::connect(name_of(&cfg.socket).map_err(fail)?).map_err(fail)?);
        let _ = stream.set_recv_timeout(Some(HANDSHAKE_TIMEOUT));
        write_frame(
            &mut &*stream,
            &Frame::Hello {
                protocol: PROTOCOL,
                role: cfg.role.clone(),
                pid: std::process::id(),
                token: cfg.token.clone(),
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        )
        .map_err(fail)?;
        let mut reader = BufReader::new(SharedRead(stream.clone()));
        let session = match read_frame(&mut reader) {
            Ok(Some(Frame::Welcome {
                protocol, session, ..
            })) if protocol == PROTOCOL => session,
            Ok(Some(Frame::Welcome { protocol, .. })) => {
                return Err(JdError::new(
                    ErrorCode::E_COMPAT,
                    format!("the engine speaks protocol {protocol}, this client {PROTOCOL}"),
                )
                .next("Install Studio and the Editor from the same release."));
            }
            Ok(Some(Frame::Error { error })) => return Err(error),
            Ok(Some(_)) => return Err(unavailable(&cfg.socket, "the engine did not welcome us")),
            Ok(None) => return Err(unavailable(&cfg.socket, "closed during the handshake")),
            Err(FrameError::Io(e)) => return Err(fail(e)),
            Err(FrameError::Bad(e)) => return Err(e),
        };
        let _ = stream.set_recv_timeout(None);
        tracing::info!(socket = %cfg.socket, %session, role = %cfg.role, "connected to the engine");
        let shared = Arc::new(Shared {
            cfg,
            writer: Mutex::new(Some(stream)),
            pending: Mutex::new(HashMap::new()),
            lost: Mutex::new(None),
            next_id: AtomicU64::new(1),
            last_seen: Mutex::new(Instant::now()),
            closing: AtomicBool::new(false),
            on_event,
            on_lost,
        });
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("jd-ipc-read".into())
            .spawn(move || sh.read_loop(reader))?;
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("jd-ipc-heartbeat".into())
            .spawn(move || sh.heartbeat())?;
        Ok(Self { shared, session })
    }

    /// The Studio session id.
    pub fn session(&self) -> &str {
        &self.session
    }

    /// Run one operation on the engine. `Err` is a transport failure (`E_ENGINE_UNAVAILABLE`,
    /// `E_TIMEOUT`, `E_SERIALIZE`); an operation's own failure is `Ok` with `status: error`.
    pub fn call(&self, mut request: Request) -> Result<Response, JdError> {
        let request_id = request
            .request_id
            .get_or_insert_with(RequestId::new)
            .clone();
        let op = request.op.clone();
        let span = tracing::info_span!("engine_call", op = %op, request_id = %request_id);
        let _enter = span.enter();
        let started = Instant::now();
        match self
            .shared
            .round_trip(&op, |id| Frame::Call { id, request })?
        {
            Answer::Response(r) => {
                let r = *r;
                tracing::debug!(ms = started.elapsed().as_millis() as u64, status = ?r.status, "reply");
                Ok(r)
            }
            Answer::Ops(_) => Err(JdError::new(
                ErrorCode::E_INTERNAL,
                "ops list answered a call",
            )),
        }
    }

    /// The engine's operation list (ids, descriptions, scopes, schemas).
    pub fn list_ops(&self) -> Result<Vec<OperationInfo>, JdError> {
        match self
            .shared
            .round_trip("list_ops", |id| Frame::ListOps { id })?
        {
            Answer::Ops(ops) => Ok(ops),
            Answer::Response(_) => Err(JdError::new(
                ErrorCode::E_INTERNAL,
                "a call answered list_ops",
            )),
        }
    }

    /// The error that ended the connection, if it has ended.
    pub fn lost(&self) -> Option<JdError> {
        lock(&self.shared.lost).clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        let _ = self.shared.send(&Frame::Bye {
            reason: "client closed".into(),
        });
        self.shared.lose(unavailable(
            &self.shared.cfg.socket,
            "the client closed the connection",
        ));
    }
}

impl Shared {
    fn send(&self, frame: &Frame) -> io::Result<()> {
        match &*lock(&self.writer) {
            Some(stream) => write_frame(&mut &**stream, frame),
            None => Err(io::ErrorKind::NotConnected.into()),
        }
    }

    fn round_trip(&self, what: &str, frame: impl FnOnce(u64) -> Frame) -> Result<Answer, JdError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        {
            // Checked under the pending lock: `lose` marks the loss before draining pending,
            // so a call either sees the loss here or is drained.
            let mut pending = lock(&self.pending);
            if let Some(e) = lock(&self.lost).clone() {
                return Err(e);
            }
            pending.insert(id, tx);
        }
        if let Err(e) = self.send(&frame(id)) {
            lock(&self.pending).remove(&id);
            if e.kind() == io::ErrorKind::InvalidData {
                return Err(JdError::new(
                    ErrorCode::E_SERIALIZE,
                    format!("the `{what}` request is too large to send ({e})"),
                )
                .next("Send less data in one request."));
            }
            let err = lock(&self.lost)
                .clone()
                .unwrap_or_else(|| unavailable(&self.cfg.socket, e));
            self.lose(err.clone());
            return Err(err);
        }
        match rx.recv_timeout(self.cfg.call_timeout) {
            Ok(answer) => answer,
            Err(RecvTimeoutError::Timeout) => {
                lock(&self.pending).remove(&id);
                tracing::warn!(id, what, "call timed out");
                Err(JdError::new(
                    ErrorCode::E_TIMEOUT,
                    format!(
                        "`{what}` got no reply within {} s",
                        self.cfg.call_timeout.as_secs_f32()
                    ),
                )
                .why("The engine is still working on it, or it is stuck.")
                .next("Check the running tasks in Studio; the operation may still complete.")
                .detail("timeout_ms", self.cfg.call_timeout.as_millis() as u64))
            }
            Err(RecvTimeoutError::Disconnected) => Err(lock(&self.lost)
                .clone()
                .unwrap_or_else(|| unavailable(&self.cfg.socket, "the connection ended"))),
        }
    }

    fn answer(&self, id: u64, answer: Answer) {
        match lock(&self.pending).remove(&id) {
            Some(tx) => {
                let _ = tx.send(Ok(answer));
            }
            None => tracing::debug!(id, "late answer for a call that timed out"),
        }
    }

    /// Mark the engine lost (first cause wins): close our side, fail every pending call, tell
    /// the owner unless it closed on purpose.
    fn lose(&self, err: JdError) {
        {
            let mut lost = lock(&self.lost);
            if lost.is_some() {
                return;
            }
            *lost = Some(err.clone());
        }
        lock(&self.writer).take();
        let pending: Vec<_> = lock(&self.pending).drain().collect();
        for (_, tx) in pending {
            let _ = tx.send(Err(err.clone()));
        }
        if self.closing.load(Ordering::SeqCst) {
            tracing::debug!("connection to the engine closed");
        } else {
            if err.details.get("orderly").is_some() {
                tracing::info!("{}", err.message);
            } else {
                tracing::warn!(
                    code = err.code.as_str(),
                    "engine connection lost: {}",
                    err.message
                );
            }
            (self.on_lost)(&err);
        }
    }

    fn read_loop(&self, mut reader: BufReader<SharedRead>) {
        loop {
            let frame = match read_frame(&mut reader) {
                Ok(Some(f)) => f,
                Ok(None) => {
                    return self.lose(unavailable(
                        &self.cfg.socket,
                        "the engine closed the connection",
                    ));
                }
                Err(FrameError::Io(e)) => return self.lose(unavailable(&self.cfg.socket, e)),
                Err(FrameError::Bad(e)) => return self.lose(e),
            };
            *lock(&self.last_seen) = Instant::now();
            match frame {
                Frame::Reply { id, response } => {
                    self.answer(id, Answer::Response(Box::new(response)))
                }
                Frame::Ops { id, ops } => self.answer(id, Answer::Ops(ops)),
                Frame::Event { topic, data } => {
                    tracing::debug!(%topic, "event");
                    (self.on_event)(&topic, &data);
                }
                Frame::Ping { seq } => {
                    let _ = self.send(&Frame::Pong { seq });
                }
                Frame::Bye { reason } => {
                    let msg = format!("the engine ended the session ({reason})");
                    return self.lose(unavailable(&self.cfg.socket, msg).detail("orderly", true));
                }
                Frame::Error { error } => return self.lose(error),
                Frame::Pong { .. } | Frame::Welcome { .. } => {}
                Frame::Hello { .. } | Frame::Call { .. } | Frame::ListOps { .. } => {
                    tracing::warn!("the engine sent a client-only frame; ignored");
                }
            }
        }
    }

    fn heartbeat(&self) {
        let mut seq = 0;
        loop {
            std::thread::sleep(self.cfg.heartbeat);
            if lock(&self.lost).is_some() {
                return;
            }
            let silent = lock(&self.last_seen).elapsed();
            if silent > self.cfg.dead_after {
                let msg = format!("no answer for {:.1} s", silent.as_secs_f32());
                return self.lose(unavailable(&self.cfg.socket, msg));
            }
            seq += 1;
            if let Err(e) = self.send(&Frame::Ping { seq }) {
                return self.lose(unavailable(&self.cfg.socket, e));
            }
        }
    }
}
