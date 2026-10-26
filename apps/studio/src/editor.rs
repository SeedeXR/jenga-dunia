//! Launching and supervising the Editor, the Tauri node-graph window (ADR-0017 §4): Studio
//! spawns it with the socket and token in its environment, notices when it exits, and restarts
//! it after a crash (at most [`MAX_CRASHES`] within [`CRASH_WINDOW`]). The Editor exits by itself
//! when Studio's socket closes, so the two always stop together.

use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Crashes tolerated within [`CRASH_WINDOW`] before Studio stops restarting the Editor.
pub const MAX_CRASHES: usize = 3;
/// The window over which crashes are counted.
pub const CRASH_WINDOW: Duration = Duration::from_secs(60);

/// What the Editor is doing, for the status line.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Running with this process id.
    Running(u32),
    /// The user closed it (exit status 0).
    Closed,
    /// It crashed too often; Studio stopped restarting it.
    GaveUp(String),
    /// It could not be started.
    Failed(String),
}

/// Find the Editor binary: `explicit`, else `JD_EDITOR`, else `jd-app` next to this executable.
pub fn locate(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = explicit.or_else(|| std::env::var_os("JD_EDITOR").map(PathBuf::from)) {
        return if p.is_file() {
            Ok(p)
        } else {
            Err(format!("the Editor `{}` does not exist", p.display()))
        };
    }
    let exe = std::env::current_exe().map_err(|e| format!("cannot find jd-studio itself: {e}"))?;
    let sibling = exe.with_file_name(format!("jd-app{}", std::env::consts::EXE_SUFFIX));
    if sibling.is_file() {
        Ok(sibling)
    } else {
        Err(format!(
            "the Editor (jd-app) is not next to jd-studio at {}; build it (`cargo build -p jd-app`) or set JD_EDITOR",
            sibling.display()
        ))
    }
}

/// The supervised Editor process.
pub struct Editor {
    path: PathBuf,
    socket: String,
    token: String,
    child: Arc<Mutex<Option<Child>>>,
    crashes: Vec<Instant>,
    /// Current state.
    pub status: Status,
    on_exit: Arc<dyn Fn(ExitStatus) + Send + Sync>,
}

impl Editor {
    /// Start the Editor at `path` against this socket. `on_exit` runs (on a watcher thread)
    /// when the process ends; the owner then calls [`Editor::exited`].
    pub fn start(
        path: PathBuf,
        socket: &str,
        token: &str,
        on_exit: impl Fn(ExitStatus) + Send + Sync + 'static,
    ) -> Self {
        let mut e = Self {
            path,
            socket: socket.to_owned(),
            token: token.to_owned(),
            child: Arc::default(),
            crashes: Vec::new(),
            status: Status::Closed,
            on_exit: Arc::new(on_exit),
        };
        e.spawn();
        e
    }

    /// (Re)launch the process.
    pub fn spawn(&mut self) {
        let child = Command::new(&self.path)
            .env(jd_ipc::ENV_SOCKET, &self.socket)
            .env(jd_ipc::ENV_TOKEN, &self.token)
            .stdin(Stdio::null())
            .spawn();
        let child = match child {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("could not start the Editor `{}`: {e}", self.path.display());
                tracing::error!("{msg}");
                self.status = Status::Failed(msg);
                return;
            }
        };
        let pid = child.id();
        tracing::info!(pid, path = %self.path.display(), "editor started");
        self.status = Status::Running(pid);
        *self.child.lock().unwrap_or_else(PoisonError::into_inner) = Some(child);
        // ponytail: a 250 ms try_wait poll; `wait()` would own the child and rule out `kill`.
        let (slot, on_exit) = (self.child.clone(), self.on_exit.clone());
        let watcher = std::thread::Builder::new()
            .name("editor-watch".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(250));
                    let mut guard = slot.lock().unwrap_or_else(PoisonError::into_inner);
                    let Some(child) = guard.as_mut() else { return };
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            guard.take();
                            drop(guard);
                            return on_exit(status);
                        }
                        Ok(None) => {}
                        Err(e) => return tracing::warn!(error = %e, "cannot watch the editor"),
                    }
                }
            });
        if let Err(e) = watcher {
            tracing::warn!(error = %e, "could not start the editor watcher");
        }
    }

    /// The process ended with `status`: a clean exit means the user closed it; anything else is
    /// a crash, restarted unless it keeps crashing. Returns a line for Studio's log.
    pub fn exited(&mut self, status: ExitStatus) -> String {
        if status.success() {
            tracing::info!("editor closed");
            self.status = Status::Closed;
            return "The Editor was closed.".into();
        }
        let now = Instant::now();
        self.crashes
            .retain(|t| now.duration_since(*t) < CRASH_WINDOW);
        self.crashes.push(now);
        if self.crashes.len() > MAX_CRASHES {
            let msg = format!(
                "The Editor crashed {} times within {} s ({status}); not restarting it. Use Open Editor to try again.",
                self.crashes.len(),
                CRASH_WINDOW.as_secs()
            );
            tracing::error!("{msg}");
            self.status = Status::GaveUp(msg.clone());
            return msg;
        }
        tracing::warn!(%status, "editor crashed; restarting");
        self.spawn();
        format!("The Editor stopped unexpectedly ({status}); restarted it.")
    }

    /// Open it again after the user closed it or Studio gave up.
    pub fn reopen(&mut self) {
        self.crashes.clear();
        self.spawn();
    }

    /// Wait up to `grace` for it to exit (it does once the socket closes), then kill it.
    pub fn stop(&mut self, grace: Duration) {
        let until = Instant::now() + grace;
        loop {
            let mut guard = self.child.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(child) = guard.as_mut() else { return };
            if matches!(child.try_wait(), Ok(Some(_))) {
                guard.take();
                return;
            }
            if Instant::now() >= until {
                tracing::warn!("editor did not exit; killing it");
                let _ = child.kill();
                let _ = child.wait();
                guard.take();
                return;
            }
            drop(guard);
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
