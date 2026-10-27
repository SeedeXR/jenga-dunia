//! The commands the toolbar, the keyboard and the macOS menu bar share, with the Editor's
//! shortcuts (`build_menu` in apps/desktop/src-tauri). The keys work everywhere (egui); the menu
//! bar is macOS's (muda, the Editor's menu library).
//! ponytail: no Windows / Linux menu bar — muda's Windows one needs its `unsafe init_for_hwnd`
//! (a contained crate, as jd-macos) and its Linux one a GTK window; add them when users ask (the
//! Editor window keeps its own menus there).

use eframe::egui::{Key, KeyboardShortcut, Modifiers};
use std::sync::{Mutex, OnceLock, PoisonError};

/// A toolbar / shortcut / menu command (`StudioApp::run`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// New project… (⌘N).
    New,
    /// Open project… (⌘O).
    Open,
    /// Save (⌘S).
    Save,
    /// Import a DEM, heightmap or mesh (⌘I).
    Import,
    /// Export the heightmap (⌘E).
    Export,
    /// Undo a graph edit (⌥⌘Z).
    UndoGraph,
    /// Redo a graph edit (⇧⌥⌘Z).
    RedoGraph,
    /// Generate terrain (⌘G).
    Generate,
    /// Evaluate the graph (⌘R).
    Evaluate,
}

/// Each command's menu id, label and accelerator (the Editor's). Redo comes before Undo: egui
/// matches ⌥⌘Z inside ⇧⌥⌘Z (extra Shift is allowed), so the longer one must be tried first.
const TABLE: [(Command, &str, &str, &str); 9] = [
    (Command::New, "new", "New Project…", "CmdOrCtrl+N"),
    (Command::Open, "open", "Open Project…", "CmdOrCtrl+O"),
    (Command::Save, "save", "Save", "CmdOrCtrl+S"),
    (
        Command::Import,
        "import",
        "Import Heightmap or Mesh…",
        "CmdOrCtrl+I",
    ),
    (
        Command::Export,
        "export",
        "Export Heightmap…",
        "CmdOrCtrl+E",
    ),
    (
        Command::RedoGraph,
        "redo-graph",
        "Redo Graph Edit",
        "CmdOrCtrl+Alt+Shift+Z",
    ),
    (
        Command::UndoGraph,
        "undo-graph",
        "Undo Graph Edit",
        "CmdOrCtrl+Alt+Z",
    ),
    (
        Command::Generate,
        "generate",
        "Generate Terrain",
        "CmdOrCtrl+G",
    ),
    (
        Command::Evaluate,
        "evaluate",
        "Evaluate Graph",
        "CmdOrCtrl+R",
    ),
];

/// Every command with its egui shortcut, in the order to try them.
pub fn shortcuts() -> impl Iterator<Item = (Command, KeyboardShortcut)> {
    TABLE
        .iter()
        .filter_map(|&(c, _, _, keys)| Some((c, shortcut(keys)?)))
}

/// `CmdOrCtrl+Alt+Shift+Z` as egui's shortcut (⌘ on macOS, Ctrl elsewhere).
fn shortcut(keys: &str) -> Option<KeyboardShortcut> {
    let mut parts: Vec<&str> = keys.split('+').collect();
    let key = Key::from_name(parts.pop()?)?;
    let mut m = Modifiers::NONE;
    for p in parts {
        m |= match p {
            "CmdOrCtrl" => Modifiers::COMMAND,
            "Alt" => Modifiers::ALT,
            "Shift" => Modifiers::SHIFT,
            _ => return None,
        };
    }
    Some(KeyboardShortcut::new(m, key))
}

static QUEUE: Mutex<Vec<Command>> = Mutex::new(Vec::new());
static WAKE: OnceLock<eframe::egui::Context> = OnceLock::new();

/// The menu commands chosen since the last call.
pub fn take() -> Vec<Command> {
    std::mem::take(&mut *QUEUE.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Repaint this context when a menu command arrives (the window may be idle).
pub fn wake(ctx: &eframe::egui::Context) {
    let _ = WAKE.set(ctx.clone());
}

/// The macOS menu bar: the app menu, File, Edit (graph undo / redo — egui's text fields handle
/// their own copy and paste keys, so the system's text items stay out), Terrain and Window.
/// Call on the main thread once the window exists. A failure is logged; the keys still work.
#[cfg(target_os = "macos")]
pub fn install() {
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu, accelerator::Accelerator};
    let item = |c: Command| {
        let (_, id, label, keys) = TABLE.iter().find(|t| t.0 == c).copied()?;
        Some(MenuItem::with_id(
            id,
            label,
            true,
            keys.parse::<Accelerator>().ok(),
        ))
    };
    let build = || -> Option<Menu> {
        let sep = PredefinedMenuItem::separator;
        let app = Submenu::with_items(
            "Jenga Dunia",
            true,
            &[
                &PredefinedMenuItem::about(Some("About Jenga Dunia"), None),
                &sep(),
                &PredefinedMenuItem::services(None),
                &sep(),
                &PredefinedMenuItem::hide(Some("Hide Jenga Dunia")),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &sep(),
                &PredefinedMenuItem::quit(Some("Quit Jenga Dunia")),
            ],
        )
        .ok()?;
        let file = Submenu::with_items(
            "File",
            true,
            &[
                &item(Command::New)?,
                &item(Command::Open)?,
                &sep(),
                &item(Command::Save)?,
                &sep(),
                &item(Command::Import)?,
                &item(Command::Export)?,
                &sep(),
                &PredefinedMenuItem::close_window(None),
            ],
        )
        .ok()?;
        let edit = Submenu::with_items(
            "Edit",
            true,
            &[&item(Command::UndoGraph)?, &item(Command::RedoGraph)?],
        )
        .ok()?;
        let terrain = Submenu::with_items(
            "Terrain",
            true,
            &[&item(Command::Generate)?, &item(Command::Evaluate)?],
        )
        .ok()?;
        let window = Submenu::with_items(
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::maximize(None),
                &sep(),
                &PredefinedMenuItem::fullscreen(None),
            ],
        )
        .ok()?;
        Menu::with_items(&[&app, &file, &edit, &terrain, &window]).ok()
    };
    let Some(menu) = build() else {
        tracing::warn!("could not build the menu bar; the keyboard shortcuts still work");
        return;
    };
    menu.init_for_nsapp();
    window_menu(&menu);
    // The menu lives as long as the app.
    std::mem::forget(menu);
    MenuEvent::set_event_handler(Some(|e: MenuEvent| {
        let Some(&(c, ..)) = TABLE.iter().find(|t| e.id == t.1) else {
            return;
        };
        QUEUE.lock().unwrap_or_else(PoisonError::into_inner).push(c);
        if let Some(ctx) = WAKE.get() {
            ctx.request_repaint();
        }
    }));
}

/// The Window menu lists the open windows (the system fills it in).
#[cfg(target_os = "macos")]
fn window_menu(menu: &muda::Menu) {
    if let Some(muda::MenuItemKind::Submenu(w)) = menu.items().last() {
        w.set_as_windows_menu_for_nsapp();
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_a_shortcut_and_redo_is_tried_before_undo() {
        let all: Vec<_> = shortcuts().collect();
        assert_eq!(all.len(), TABLE.len());
        let at = |c| all.iter().position(|(x, _)| *x == c).unwrap();
        assert!(at(Command::RedoGraph) < at(Command::UndoGraph));
        let (_, redo) = all[at(Command::RedoGraph)];
        assert_eq!(
            redo,
            KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT,
                Key::Z
            )
        );
        assert_eq!(
            shortcut("CmdOrCtrl+R"),
            Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::R))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn every_accelerator_parses_for_the_menu_bar() {
        for (_, id, _, keys) in TABLE {
            assert!(
                keys.parse::<muda::accelerator::Accelerator>().is_ok(),
                "{id}: {keys}"
            );
        }
    }
}
