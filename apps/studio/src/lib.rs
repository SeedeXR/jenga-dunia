//! Jenga Dunia Studio (ADR-0017): the process that owns the engine — the `jd-core` registry and
//! context, served to the Editor over `jd-ipc` — and its window: the 3-D viewport and panels.

pub mod app;
pub mod chrome;
pub mod editor;
pub mod look;
pub mod post;
pub mod theme;
pub mod viewport;
pub mod widgets;

pub use app::{Options, StudioApp};
