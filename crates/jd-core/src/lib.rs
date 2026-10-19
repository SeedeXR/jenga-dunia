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
