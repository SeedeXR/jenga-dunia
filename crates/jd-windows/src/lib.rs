//! Studio's menu bar on Windows. muda builds the menus everywhere, but a Win32 menu bar belongs to
//! a window: [`attach_menu`] hands it Studio's window handle, the one `unsafe` call, kept here so
//! the rest of the workspace can forbid `unsafe` (as jd-macos does for AppKit). Elsewhere there is
//! nothing to attach and this crate is empty.

/// Put `menu` on `window`'s title bar, dark to match Studio; `false` if the window is not a Win32
/// one or muda refused. Call on the thread that owns the window, once it exists.
#[cfg(target_os = "windows")]
pub fn attach_menu(menu: &muda::Menu, window: &impl raw_window_handle::HasWindowHandle) -> bool {
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let raw_window_handle::RawWindowHandle::Win32(win) = handle.as_raw() else {
        return false;
    };
    // SAFETY: `hwnd` is the live window eframe just created (borrowed through `window`), and this
    // runs on the thread that owns it, which is what muda requires of the handle.
    unsafe { menu.init_for_hwnd_with_theme(win.hwnd.get(), muda::MenuTheme::Dark) }.is_ok()
}
