//! Jenga Dunia core.
//!
//! One core, three adapters (ADR-0001): everything the desktop app, the `jd`
//! CLI and the MCP server can do is an [`ops::Operation`] in the
//! [`ops::Registry`], invoked with the JSON envelope in [`envelope`].
//! Domain services live here; adapters only marshal.

pub mod diag;
pub mod envelope;
pub mod ops;
pub mod project;

pub use envelope::{Request, Response, Status, Warning};
pub use ops::{Context, Operation, Registry, Scope};
pub use project::{Project, ProjectDocument};

/// Version of this build, from the workspace manifest.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

// The repo's `VERSION` file is the single source of truth; Cargo, package.json and tauri.conf.json
// must match it (`scripts/check-version.mjs`). This makes a drift in Cargo.toml a compile error.
const _: () = assert!(
    version_matches(include_str!("../../../VERSION"), APP_VERSION),
    "VERSION file and Cargo.toml workspace version differ"
);

const fn version_matches(file: &str, pkg: &str) -> bool {
    let (f, p) = (file.as_bytes(), pkg.as_bytes());
    let mut n = f.len();
    while n > 0 && (f[n - 1] == b'\n' || f[n - 1] == b'\r' || f[n - 1] == b' ') {
        n -= 1;
    }
    if n != p.len() {
        return false;
    }
    let mut i = 0;
    while i < n {
        if f[i] != p[i] {
            return false;
        }
        i += 1;
    }
    true
}
