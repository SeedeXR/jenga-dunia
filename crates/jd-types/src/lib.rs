//! Shared, dependency-free types for Jenga Dunia.
//!
//! This crate holds what every other crate agrees on: identifiers, schema
//! versions, the stable error catalogue (`docs/spec/error-codes.md`), units and
//! world settings. It must never depend on another `jd-*` crate, on Tauri, on
//! wgpu or on the filesystem.

pub mod error;
pub mod ids;
pub mod version;
pub mod world;

pub use error::{ErrorCode, JdError, JdResult};
pub use ids::{JobId, NodeId, ProjectId, RequestId};
pub use version::SchemaVersion;
pub use world::{Handedness, Units, UpAxis, WorldSettings};
