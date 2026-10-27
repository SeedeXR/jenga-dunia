//! Jenga Dunia Studio (ADR-0017): the process that owns the engine — the `jd-core` registry and
//! context, served to the Editor over `jd-ipc` — and its window: the 3-D viewport and panels.

pub mod app;
pub mod chrome;
pub mod documents;
pub mod editor;
pub mod i18n;
pub mod look;
pub mod menu;
pub mod post;
pub mod theme;
pub mod viewport;
pub mod widgets;

pub use app::{Options, StudioApp};
