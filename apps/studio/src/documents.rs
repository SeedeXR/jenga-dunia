//! Projects handed to Studio by the system: a `.jdproj` double-clicked, dropped on the Dock icon or
//! opened with "Open With" (the bundle's `CFBundleDocumentTypes`). Linux and Windows pass the path
//! on the command line (`jd-studio <project>`); on macOS it arrives as an Apple Event, received by
//! `jd-macos` (the Editor did this through Tauri's `RunEvent::Opened`).

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};

static QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static WAKE: OnceLock<eframe::egui::Context> = OnceLock::new();

/// The documents opened since the last call (the window opens the last one).
pub fn take() -> Vec<PathBuf> {
    std::mem::take(&mut *QUEUE.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Repaint this context when a document arrives (the window may be idle).
pub fn wake(ctx: &eframe::egui::Context) {
    let _ = WAKE.set(ctx.clone());
}

fn push(paths: Vec<PathBuf>) {
    QUEUE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .extend(paths);
    if let Some(ctx) = WAKE.get() {
        ctx.request_repaint();
    }
}

/// Receive the documents the system opens (macOS; elsewhere they arrive on the command line).
/// Call on the main thread before the window's event loop starts.
pub fn install() {
    jd_macos::install(push);
}
