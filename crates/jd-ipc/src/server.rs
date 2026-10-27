//! Studio's side: accepts Editor / test connections and runs their calls on the engine.

use std::io::{self, BufReader};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use interprocess::local_socket::{ListenerOptions, Stream, prelude::*};
use jd_core::{Context, Registry, Request, Response, Scope, Status};
use jd_types::{ErrorCode, JdError, RequestId};
use serde_json::{Value, json};

use crate::{
    Endpoint, Frame, FrameError, HANDSHAKE_TIMEOUT, PROTOCOL, SharedRead, name_of, new_token,
    read_frame, tokens_match, write_frame,
};

/// The engine server: one listener, a reader thread per connection, a worker thread per call.
pub struct Server {
    shared: Arc<Shared>,
    endpoint: Endpoint,
    accept: Mutex<Option<JoinHandle<()>>>,
}

struct Shared {
    registry: Registry,
    ctx: Context,
    token: String,
    session: String,
    peers: Mutex<Vec<Arc<Peer>>>,
    /// Studio's own listener for the events it sends its clients.
    local: Mutex<Option<LocalHook>>,
    next_peer: AtomicU64,
    stopping: AtomicBool,
    /// Operations running for any window (`jobs.*` aside), for the `busy` event.
    running: Mutex<usize>,
}

type LocalHook = Arc<dyn Fn(&str, &Value) + Send + Sync>;

struct Peer {
    id: u64,
    role: String,
    stream: Arc<Stream>,
    write: Mutex<()>,
}

impl Peer {
    fn send(&self, frame: &Frame) -> io::Result<()> {
        let _turn = self.write.lock().unwrap_or_else(PoisonError::into_inner);
        write_frame(&mut &*self.stream, frame)
    }
}

impl Server {
    /// Bind a fresh private endpoint with a new token and start accepting connections.
    pub fn bind(registry: Registry, ctx: Context) -> Result<Self, JdError> {
        let endpoint = Endpoint::new().map_err(|e| {
            JdError::from(e).why("The engine socket's private directory could not be created.")
        })?;
        let token = new_token()?;
        let listener = ListenerOptions::new()
            .name(name_of(&endpoint.socket)?)
            .try_overwrite(true)
            .create_sync()
            .map_err(|e| JdError::from(e).detail("socket", endpoint.socket.clone()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&endpoint.socket, std::fs::Permissions::from_mode(0o600))?;
        }
        let shared = Arc::new(Shared {
            registry,
            ctx,
            token,
            session: endpoint.session.clone(),
            peers: Mutex::new(Vec::new()),
            local: Mutex::new(None),
            next_peer: AtomicU64::new(1),
            stopping: AtomicBool::new(false),
            running: Mutex::new(0),
        });
        let sh = shared.clone();
        let accept = std::thread::Builder::new()
            .name("jd-ipc-accept".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if sh.stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    match conn {
                        Ok(stream) => {
                            let sh = sh.clone();
                            if let Err(e) = std::thread::Builder::new()
                                .name("jd-ipc-conn".into())
                                .spawn(move || serve(&sh, stream))
                            {
                                tracing::error!(error = %e, "could not start a connection thread");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "incoming connection failed");
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                    }
                }
            })?;
        tracing::info!(socket = %endpoint.socket, session = %endpoint.session, "engine server listening");
        Ok(Self {
            shared,
            endpoint,
            accept: Mutex::new(Some(accept)),
        })
    }

    /// The socket path / pipe name clients connect to ([`crate::ENV_SOCKET`]).
    pub fn socket(&self) -> &str {
        &self.endpoint.socket
    }

    /// The launch token clients must present ([`crate::ENV_TOKEN`]).
    pub fn token(&self) -> &str {
        &self.shared.token
    }

    /// The session id.
    pub fn session(&self) -> &str {
        &self.shared.session
    }

    /// Run an operation on the engine as Studio's own UI does, with the same events as a
    /// client's call.
    pub fn invoke(&self, request: Request) -> Response {
        self.shared.invoke(request)
    }

    /// Push an event to every connected client.
    pub fn emit(&self, topic: &str, data: Value) {
        self.shared.emit(topic, data);
    }

    /// Also deliver every event to `hook` in this process (Studio's window), on the thread that
    /// emitted it. Replaces an earlier hook.
    pub fn on_event(&self, hook: impl Fn(&str, &Value) + Send + Sync + 'static) {
        *self
            .shared
            .local
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(hook));
    }

    /// Tell every client (`shutdown` event, then `bye`) and stop accepting. Clients close their
    /// side on `bye`; operations still running finish.
    /// Idempotent; dropping the server does the same.
    pub fn shutdown(&self, reason: &str) {
        let Some(accept) = self
            .accept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.shared.emit("shutdown", json!({ "reason": reason }));
        for peer in self.shared.peers() {
            let _ = peer.send(&Frame::Bye {
                reason: reason.to_owned(),
            });
        }
        // Wake the accept loop so it sees `stopping`. Not joined: if the wake cannot connect,
        // quitting must not hang; the thread ends on its next accept, and dropping the endpoint
        // removes the socket directory either way.
        if let Ok(name) = name_of(&self.endpoint.socket) {
            let _ = Stream::connect(name);
        }
        drop(accept);
        tracing::info!(reason, "engine server stopped");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown("the engine server was dropped");
    }
}

impl Shared {
    fn peers(&self) -> Vec<Arc<Peer>> {
        self.peers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn invoke(&self, request: Request) -> Response {
        // `ui.select { nodes }`: a window's selection, announced to the others as `selection
        // { nodes, source }` (node ↔ viewport); the engine has nothing to do with it.
        if request.op == "ui.select" {
            let source = request.client.as_ref().map(|c| c.kind.clone());
            let nodes = request
                .params
                .get("nodes")
                .cloned()
                .unwrap_or_else(|| json!([]));
            self.emit("selection", json!({ "nodes": nodes, "source": source }));
            return Response::ok(
                "ui.select",
                request.request_id.unwrap_or_default(),
                json!({}),
                Vec::new(),
            );
        }
        let op = request.op.clone();
        let client = request.client.as_ref().map(|c| c.kind.clone());
        let scope = self.registry.get(&op).map(|o| o.scope);
        // `jobs.*` follow the running work; counting them would make polling look like work.
        let tracked = !op.starts_with("jobs.");
        if tracked {
            self.running(true);
        }
        // Panics are caught inside (an E_INTERNAL response), so the count always comes back down.
        let response = self.registry.invoke(&self.ctx, request);
        if tracked {
            self.running(false);
        }
        // ponytail: detached jobs (`accepted`) change the project later without an event of their
        // own; their progress is `jobs.list`, polled while `busy`.
        // `project.open` / `project.close` only read, but they change which project is open.
        let changes = matches!(
            scope,
            Some(Scope::ProjectWrite | Scope::FsWrite | Scope::Destructive)
        ) || matches!(op.as_str(), "project.open" | "project.close");
        if response.status == Status::Ok && op == "graph.evaluate" {
            // The report (layers, lod, duration): a window adopts an evaluation another ran.
            self.emit(
                "evaluated",
                json!({ "report": response.result, "client": client }),
            );
        }
        if response.status == Status::Ok && changes {
            self.emit(
                "project_changed",
                // `client`: who wrote (a window skips its own writes, its reply refreshes it).
                json!({ "op": op, "request_id": response.request_id, "scope": scope, "client": client }),
            );
        }
        response
    }

    /// `busy { running }` when any operation starts while none ran, and when the last ends: a
    /// window can then show (and cancel, `jobs.cancel`) work another window started. Sent under
    /// the count's lock so two flips cannot arrive out of order.
    fn running(&self, start: bool) {
        let mut n = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        let was = *n > 0;
        *n = if start { *n + 1 } else { n.saturating_sub(1) };
        if was != (*n > 0) {
            self.emit("busy", json!({ "running": *n > 0 }));
        }
    }

    fn emit(&self, topic: &str, data: Value) {
        let local = self
            .local
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(hook) = local {
            hook(topic, &data);
        }
        let frame = Frame::Event {
            topic: topic.to_owned(),
            data,
        };
        for peer in self.peers() {
            if let Err(e) = peer.send(&frame) {
                tracing::debug!(peer = peer.id, topic, error = %e, "event not delivered");
            }
        }
    }
}

/// Refuse a connection with a protocol-level error and close it.
fn refuse(stream: &Stream, error: JdError) {
    tracing::warn!(
        code = error.code.as_str(),
        "connection refused: {}",
        error.message
    );
    let _ = write_frame(&mut &*stream, &Frame::Error { error });
}

fn serve(sh: &Arc<Shared>, stream: Stream) {
    let stream = Arc::new(stream);
    let mut reader = BufReader::new(SharedRead(stream.clone()));
    // Unix only: named pipes have no read timeouts, so there a silent client just holds a thread.
    let _ = stream.set_recv_timeout(Some(HANDSHAKE_TIMEOUT));
    let (role, pid) = match read_frame(&mut reader) {
        Ok(Some(Frame::Hello {
            protocol,
            role,
            pid,
            token,
            app_version,
        })) => {
            if protocol != PROTOCOL {
                return refuse(
                    &stream,
                    JdError::new(
                        ErrorCode::E_COMPAT,
                        format!(
                            "the {role} speaks protocol {protocol}, this engine speaks {PROTOCOL}"
                        ),
                    )
                    .why("Studio and the Editor come from different builds.")
                    .next("Install Studio and the Editor from the same release.")
                    .detail("client_protocol", protocol)
                    .detail("engine_protocol", PROTOCOL)
                    .detail("client_version", app_version),
                );
            }
            if !tokens_match(&token, &sh.token) {
                return refuse(
                    &stream,
                    JdError::new(ErrorCode::E_PERMISSION, "wrong engine token")
                        .why("The connection was not started by this Studio session.")
                        .next(format!(
                            "Let Studio launch the Editor; it passes the token in {}.",
                            crate::ENV_TOKEN
                        )),
                );
            }
            tracing::debug!(%role, pid, %app_version, "hello");
            (role, pid)
        }
        Ok(Some(_)) => {
            return refuse(
                &stream,
                JdError::new(ErrorCode::E_PERMISSION, "the first frame must be `hello`")
                    .next("Send `hello` with the protocol version and token first."),
            );
        }
        Ok(None) => return tracing::debug!("connection closed before hello"),
        Err(FrameError::Bad(e)) => return refuse(&stream, e),
        Err(FrameError::Io(e)) => return tracing::debug!(error = %e, "no hello"),
    };
    let _ = stream.set_recv_timeout(None);
    let peer = Arc::new(Peer {
        id: sh.next_peer.fetch_add(1, Ordering::Relaxed),
        role,
        stream: stream.clone(),
        write: Mutex::new(()),
    });
    // Registered before `welcome`, so a client sees every event from its first frame on.
    sh.peers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(peer.clone());
    let span =
        tracing::info_span!("ipc_client", session = %sh.session, role = %peer.role, peer = peer.id);
    let _enter = span.enter();
    if let Err(e) = peer.send(&Frame::Welcome {
        protocol: PROTOCOL,
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        session: sh.session.clone(),
    }) {
        tracing::debug!(error = %e, "welcome not delivered");
    } else {
        tracing::info!(pid, "client connected");
        serve_frames(sh, &peer, &mut reader);
    }
    sh.peers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|p| p.id != peer.id);
}

fn serve_frames(sh: &Arc<Shared>, peer: &Arc<Peer>, reader: &mut BufReader<SharedRead>) {
    loop {
        let frame = match read_frame(reader) {
            Ok(Some(f)) => f,
            Ok(None) => return tracing::info!("client disconnected"),
            Err(FrameError::Io(e)) => {
                return tracing::info!(error = %e, "client connection ended");
            }
            Err(FrameError::Bad(e)) => {
                tracing::warn!(
                    code = e.code.as_str(),
                    "malformed frame from client: {}",
                    e.message
                );
                let _ = peer.send(&Frame::Error { error: e });
                return;
            }
        };
        match frame {
            Frame::Call { id, request } => {
                let (sh, peer) = (sh.clone(), peer.clone());
                let span = tracing::Span::current();
                if let Err(e) = std::thread::Builder::new()
                    .name("jd-ipc-call".into())
                    .spawn(move || span.in_scope(|| run_call(&sh, &peer, id, request)))
                {
                    tracing::error!(error = %e, "could not start a call thread");
                }
            }
            Frame::ListOps { id } => {
                let ops = sh.registry.list();
                if let Err(e) = peer.send(&Frame::Ops { id, ops }) {
                    tracing::debug!(error = %e, "ops list not delivered");
                }
            }
            Frame::Ping { seq } => {
                let _ = peer.send(&Frame::Pong { seq });
            }
            Frame::Pong { .. } => {}
            Frame::Bye { reason } => return tracing::info!(%reason, "client said goodbye"),
            other => {
                let error = JdError::new(
                    ErrorCode::E_INPUT_INVALID,
                    format!("a client may not send `{}`", frame_kind(&other)),
                );
                tracing::warn!("{}", error.message);
                let _ = peer.send(&Frame::Error { error });
                return;
            }
        }
    }
}

fn frame_kind(f: &Frame) -> String {
    serde_json::to_value(f)
        .ok()
        .and_then(|v| v["t"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn run_call(sh: &Shared, peer: &Peer, id: u64, mut request: Request) {
    request.client.get_or_insert_default().kind = format!("gui-{}", peer.role);
    let request_id = request
        .request_id
        .get_or_insert_with(RequestId::new)
        .clone();
    let op = request.op.clone();
    let span = tracing::info_span!("ipc_call", id, op = %op, request_id = %request_id);
    let _enter = span.enter();
    let response = sh.invoke(request);
    match peer.send(&Frame::Reply { id, response }) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            tracing::warn!(error = %e, "reply too large for one frame");
            let error = JdError::new(
                ErrorCode::E_SERIALIZE,
                format!("the reply to `{op}` is too large to send ({e})"),
            )
            .next("Ask for less data at once (for a preview, a coarser lod).");
            let _ = peer.send(&Frame::Reply {
                id,
                response: Response::error(&op, request_id, error),
            });
        }
        Err(e) => tracing::debug!(error = %e, "reply not delivered; the client is gone"),
    }
}
