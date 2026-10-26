//! Studio ↔ Editor protocol (ADR-0017, `docs/architecture/14-studio-editor-protocol.md`):
//! newline-delimited JSON frames over a local socket (a Unix socket file, a Windows named pipe).
//!
//! Studio runs a [`Server`] over its `Registry` + `Context`; the Editor (and test clients) use a
//! [`Client`], whose `call` carries the same `jd_core::Request` → `Response` envelope as the CLI,
//! MCP and the in-process desktop app. Every transport failure is a `JdError`:
//! `E_ENGINE_UNAVAILABLE`, `E_TIMEOUT`, `E_COMPAT`, `E_PERMISSION` or `E_SERIALIZE`.

mod client;
mod server;

pub use client::{Client, ClientConfig};
pub use server::Server;

use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use interprocess::local_socket::{GenericFilePath, Name, Stream, prelude::*};
use jd_core::ops::OperationInfo;
use jd_core::{Request, Response};
use jd_types::{ErrorCode, JdError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bumped on any incompatible frame change; a mismatch is refused with `E_COMPAT`.
pub const PROTOCOL: u32 = 1;
/// The longest frame either side accepts (bytes, newline excluded).
pub const MAX_LINE: usize = 64 << 20;
/// Environment variable carrying the socket Studio listens on, set when it launches the Editor.
pub const ENV_SOCKET: &str = "JD_ENGINE_SOCKET";
/// Environment variable carrying Studio's one-time launch token.
pub const ENV_TOKEN: &str = "JD_ENGINE_TOKEN";
/// How long either side waits for the other's handshake frame (Unix; named pipes have no read
/// timeouts).
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// One protocol frame: a JSON object tagged by `t`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Frame {
    /// Client → server, first frame on a connection.
    Hello {
        /// The client's [`PROTOCOL`].
        protocol: u32,
        /// `editor`, `test`, …; the ops log records the client as `gui-<role>`.
        role: String,
        /// Client process id, for logs.
        pid: u32,
        /// Studio's launch token.
        token: String,
        /// Client build version, for logs.
        app_version: String,
    },
    /// Server → client: handshake accepted.
    Welcome {
        /// The server's [`PROTOCOL`].
        protocol: u32,
        /// Engine build version.
        engine_version: String,
        /// Studio session id (also in the socket name).
        session: String,
    },
    /// Client → server: run one operation.
    Call {
        /// Per-connection correlation id.
        id: u64,
        /// The operation envelope.
        request: Request,
    },
    /// Server → client: an operation's envelope, ok or error.
    Reply {
        /// The call's id.
        id: u64,
        /// The operation's response.
        response: Response,
    },
    /// Client → server: list the registry's operations.
    ListOps {
        /// Correlation id.
        id: u64,
    },
    /// Server → client: the operation list.
    Ops {
        /// The request's id.
        id: u64,
        /// Every operation with its schemas and scope.
        ops: Vec<OperationInfo>,
    },
    /// Server → every client, unsolicited.
    Event {
        /// `project_changed`, `selection`, `job`, `shutdown`.
        topic: String,
        /// Topic payload.
        data: Value,
    },
    /// Liveness probe.
    Ping {
        /// Echoed by the pong.
        seq: u64,
    },
    /// Liveness answer.
    Pong {
        /// The ping's sequence number.
        seq: u64,
    },
    /// Orderly close.
    Bye {
        /// Why, for logs.
        reason: String,
    },
    /// Server → client: a protocol-level refusal; the connection closes after it.
    Error {
        /// The refusal (`E_COMPAT`, `E_PERMISSION`, `E_SERIALIZE`, …).
        error: JdError,
    },
}

/// Why a frame could not be read.
#[derive(Debug)]
pub enum FrameError {
    /// The connection failed.
    Io(io::Error),
    /// The peer sent something that is not a frame (`E_SERIALIZE`).
    Bad(JdError),
}

/// Read one frame; `Ok(None)` at end of stream. Blank lines are skipped.
pub fn read_frame(r: &mut impl BufRead) -> Result<Option<Frame>, FrameError> {
    read_frame_capped(r, MAX_LINE)
}

fn read_frame_capped(r: &mut impl BufRead, cap: usize) -> Result<Option<Frame>, FrameError> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = r
            .by_ref()
            .take(cap as u64 + 1)
            .read_until(b'\n', &mut line)
            .map_err(FrameError::Io)?;
        if n == 0 {
            return Ok(None);
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        } else if line.len() > cap {
            return Err(FrameError::Bad(
                JdError::new(
                    ErrorCode::E_SERIALIZE,
                    format!("a frame is longer than the {} MiB limit", cap >> 20),
                )
                .next("Send less data per message.")
                .detail("limit_bytes", cap),
            ));
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        return serde_json::from_slice(&line)
            .map(Some)
            .map_err(|e| FrameError::Bad(JdError::from(e).detail("frame", "malformed")));
    }
}

/// Write one frame and flush. A frame over [`MAX_LINE`] is refused with `InvalidData` before
/// anything is written, so the connection stays usable.
pub fn write_frame(w: &mut impl Write, frame: &Frame) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(frame).map_err(io::Error::other)?;
    if bytes.len() > MAX_LINE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} MiB, over the {} MiB frame limit",
                bytes.len() >> 20,
                MAX_LINE >> 20
            ),
        ));
    }
    bytes.push(b'\n');
    w.write_all(&bytes)?;
    w.flush()
}

/// A shared stream that reads through `&Stream`, so a `BufReader` can own it while writers
/// hold clones (interprocess supports concurrent read and write on one stream).
struct SharedRead(Arc<Stream>);

impl Read for SharedRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self.0).read(buf)
    }
}

/// The socket name for a path (a socket file on Unix, `\\.\pipe\…` on Windows).
pub fn name_of(socket: &str) -> io::Result<Name<'_>> {
    socket.to_fs_name::<GenericFilePath>()
}

/// Where a Studio session listens: a fresh `0700` directory holding the socket on Unix (removed
/// on drop), a session-named pipe on Windows.
#[derive(Debug)]
pub struct Endpoint {
    /// Socket path / pipe name, as passed in [`ENV_SOCKET`].
    pub socket: String,
    /// Random session id.
    pub session: String,
    dir: Option<PathBuf>,
}

impl Endpoint {
    /// A new, private endpoint with a random session id.
    pub fn new() -> io::Result<Self> {
        let session = random_hex(8)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let base = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
                .unwrap_or_else(std::env::temp_dir);
            // Socket paths are limited to ~104 bytes (macOS); macOS's per-user temp dir is long.
            let base = if base.as_os_str().len() > 60 {
                PathBuf::from("/tmp")
            } else {
                base
            };
            let dir = base.join(format!("jenga-dunia-{session}"));
            // `create` (not `create_all`) fails if the directory exists, so nobody else owns it.
            std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
            let socket = dir
                .join("engine.sock")
                .into_os_string()
                .into_string()
                .map_err(|_| io::Error::other("the temp directory path is not UTF-8"))?;
            Ok(Self {
                socket,
                session,
                dir: Some(dir),
            })
        }
        #[cfg(not(unix))]
        Ok(Self {
            socket: format!(r"\\.\pipe\jenga-dunia-{session}"),
            session,
            dir: None,
        })
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// A one-time launch token (128 random bits, hex).
pub fn new_token() -> io::Result<String> {
    random_hex(16)
}

fn random_hex(bytes: usize) -> io::Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Token comparison in time independent of where the strings differ.
fn tokens_match(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// The per-user directory where Studio and the Editor write their logs (`jd-studio.log`,
/// `jd-app.log`): `~/Library/Logs/<identifier>` on macOS, `%LOCALAPPDATA%\<identifier>\logs` on
/// Windows, `$XDG_DATA_HOME/<identifier>/logs` elsewhere.
pub fn log_dir(identifier: &str) -> Option<std::path::PathBuf> {
    if cfg!(target_os = "macos") {
        Some(
            std::path::PathBuf::from(std::env::var_os("HOME")?)
                .join("Library/Logs")
                .join(identifier),
        )
    } else if cfg!(target_os = "windows") {
        Some(
            std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
                .join(identifier)
                .join("logs"),
        )
    } else {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
            })?;
        Some(base.join(identifier).join("logs"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frames_round_trip_and_oversized_or_malformed_lines_are_refused() {
        let frames = [
            Frame::Call {
                id: 7,
                request: Request::new("project.info", serde_json::json!({})),
            },
            Frame::Event {
                topic: "project_changed".into(),
                data: serde_json::json!({ "op": "graph.set_param" }),
            },
            Frame::Error {
                error: JdError::new(ErrorCode::E_COMPAT, "protocol 2 vs 1"),
            },
        ];
        let mut wire = Vec::new();
        for f in &frames {
            write_frame(&mut wire, f).expect("write");
        }
        assert!(wire.ends_with(b"\n"));
        assert!(String::from_utf8_lossy(&wire).starts_with(r#"{"t":"call","id":7,"#));
        let mut r = Cursor::new(wire);
        for f in &frames {
            let back = read_frame(&mut r).expect("read").expect("frame");
            assert_eq!(
                serde_json::to_value(&back).expect("json"),
                serde_json::to_value(f).expect("json")
            );
        }
        assert!(read_frame(&mut r).expect("eof").is_none());

        let long = format!("{{\"t\":\"ping\",\"seq\":1}}{}\n", " ".repeat(100));
        let Err(FrameError::Bad(e)) = read_frame_capped(&mut Cursor::new(long), 64) else {
            panic!("an oversized line must be refused");
        };
        assert_eq!(e.code, ErrorCode::E_SERIALIZE);
        let Err(FrameError::Bad(e)) = read_frame(&mut Cursor::new("{\"t\":\"nope\"}\n")) else {
            panic!("an unknown frame must be refused");
        };
        assert_eq!(e.code, ErrorCode::E_SERIALIZE);
        assert!(
            tokens_match("abc", "abc") && !tokens_match("abc", "abd") && !tokens_match("a", "ab")
        );
    }
}
