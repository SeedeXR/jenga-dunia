//! Jenga Dunia desktop shell.
//!
//! Three commands, no domain logic (ADR-0001):
//! - `op`: invoke any registry operation with the JSON envelope (identical to CLI/MCP).
//! - `ops_list`: operation descriptions + schemas for generated inspectors.
//! - `layer_preview`: a downsampled u16 heightfield as raw bytes for the viewport
//!   (`tauri::ipc::Response`, no JSON encoding of large buffers).

use jd_core::{Context, Registry, Request, Response};
use jd_terrain::preview::preview_u16;
#[cfg(target_os = "macos")]
use tauri::menu::PredefinedMenuItem;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager, State};

/// Shared registry + session context.
pub struct AppState {
    registry: Registry,
    ctx: Context,
}

fn join_error(e: tauri::Error) -> jd_types::JdError {
    jd_types::JdError::new(
        jd_types::ErrorCode::E_INTERNAL,
        format!("background task failed: {e}"),
    )
}

/// Operations run on a blocking thread so terrain work never stalls the window's event loop.
#[tauri::command]
async fn op(
    state: State<'_, AppState>,
    mut request: Request,
) -> Result<Response, jd_types::JdError> {
    request.client.get_or_insert_default().kind = "gui".into();
    let (registry, ctx) = (state.registry.clone(), state.ctx.clone());
    tauri::async_runtime::spawn_blocking(move || registry.invoke(&ctx, request))
        .await
        .map_err(join_error)
}

#[tauri::command]
fn ops_list(state: State<'_, AppState>) -> Vec<jd_core::ops::OperationInfo> {
    state.registry.list()
}

#[tauri::command]
async fn layer_preview(
    state: State<'_, AppState>,
    layer: String,
    lod: u8,
) -> Result<tauri::ipc::Response, jd_types::JdError> {
    let ctx = state.ctx.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        ctx.with_layer(&layer, |hf| Ok(preview_u16(hf, lod).to_bytes()))
    })
    .await
    .map_err(join_error)??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Native menu bar: platform-conventional App/File/Edit/Window menus with standard accelerators.
fn build_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let new_item = MenuItemBuilder::with_id("new", "New Project…")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let open_item = MenuItemBuilder::with_id("open", "Open Project…")
        .accelerator("CmdOrCtrl+O")
        .build(app)?;
    let save_item = MenuItemBuilder::with_id("save", "Save")
        .accelerator("CmdOrCtrl+S")
        .build(app)?;
    let generate_item = MenuItemBuilder::with_id("generate", "Generate Terrain")
        .accelerator("CmdOrCtrl+G")
        .build(app)?;
    let evaluate_item = MenuItemBuilder::with_id("evaluate", "Evaluate Graph")
        .accelerator("CmdOrCtrl+R")
        .build(app)?;
    // Graph undo/redo get their own items: the predefined Undo/Redo drive the WebView's text editing.
    let undo_graph = MenuItemBuilder::with_id("undo-graph", "Undo Graph Edit")
        .accelerator("CmdOrCtrl+Alt+Z")
        .build(app)?;
    let redo_graph = MenuItemBuilder::with_id("redo-graph", "Redo Graph Edit")
        .accelerator("CmdOrCtrl+Alt+Shift+Z")
        .build(app)?;
    let export_item = MenuItemBuilder::with_id("export", "Export Heightmap…")
        .accelerator("CmdOrCtrl+E")
        .build(app)?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&new_item)
        .item(&open_item)
        .separator()
        .item(&save_item)
        .separator()
        .item(&export_item)
        .separator()
        .close_window()
        .build()?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .item(&undo_graph)
        .item(&redo_graph)
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let terrain = SubmenuBuilder::new(app, "Terrain")
        .item(&generate_item)
        .item(&evaluate_item)
        .build()?;
    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .fullscreen()
        .build()?;
    let builder = MenuBuilder::new(app);
    #[cfg(target_os = "macos")]
    let builder = {
        let about = PredefinedMenuItem::about(app, Some("About Jenga Dunia"), None)?;
        let app_menu = SubmenuBuilder::new(app, "Jenga Dunia")
            .item(&about)
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;
        builder.item(&app_menu)
    };
    builder
        .item(&file)
        .item(&edit)
        .item(&terrain)
        .item(&window)
        .build()
}

/// Build and run the application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    // Logging first: macOS can deliver the document-open Apple Event before `setup` runs.
    init_logging(&context.config().identifier);
    let state = AppState {
        registry: Registry::with_builtins(),
        ctx: Context::default(),
    };
    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            // `jd-app <path.jdproj>` (CLI / Linux / Windows file association) opens that project
            // before the window appears; the frontend reads it back with `project.info` on mount.
            // macOS delivers double-clicked documents as an Apple Event instead — see
            // `RunEvent::Opened` below.
            if let Some(path) = std::env::args().skip(1).find(|a| a.ends_with(".jdproj")) {
                open_project(&app.state::<AppState>(), &path, "launch arguments");
            }
            app.set_menu(build_menu(app.handle())?)?;
            // Menu actions are forwarded to the frontend as a `menu` event carrying the item id;
            // the UI performs the action through the same ops as its buttons.
            app.on_menu_event(|app, event| {
                let id = event.id().0.clone();
                if let Err(e) = app.emit("menu", id.clone()) {
                    tracing::warn!(error = %e, menu = %id, "menu event not delivered");
                }
            });
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        // Window position/size/maximised state survive relaunches (native app feel).
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![op, ops_list, layer_preview])
        .build(context)
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "tauri failed to start");
            std::process::exit(1);
        })
        .run(|handle, event| {
            // macOS: Finder double-click / `open -a` hands the document over as an Apple Event,
            // before or after the window exists. Open it and tell the frontend to re-read.
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let tauri::RunEvent::Opened { urls } = event {
                let state = handle.state::<AppState>();
                for url in urls {
                    tracing::info!(%url, "document open request");
                    // Directory documents arrive with a trailing slash, so test the extension.
                    let Ok(p) = url.to_file_path() else { continue };
                    let is_project = p.extension().is_some_and(|e| e == "jdproj");
                    if is_project
                        && let Some(path) = p.to_str()
                        && open_project(&state, path, "Finder")
                        && let Err(e) = handle.emit("project-opened", path.to_owned())
                    {
                        tracing::warn!(error = %e, "project-opened event not delivered");
                    }
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            let _ = (handle, event);
        });
}

/// Logs go to stderr and to the platform log directory — macOS `~/Library/Logs/<identifier>/`,
/// Linux `$XDG_DATA_HOME/<identifier>/logs`, Windows `%LOCALAPPDATA%\<identifier>\logs` (the
/// same places Tauri's `app_log_dir` resolves to) — so a bundled app's behaviour can be read
/// back. `JD_LOG` sets the filter (default `info`).
fn init_logging(identifier: &str) {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    let filter = tracing_subscriber::EnvFilter::try_from_env("JD_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let file = log_dir(identifier).and_then(|dir| {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("jd-app.log"))
            .ok()
    });
    let result = match file {
        Some(f) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(std::io::stderr.and(std::sync::Mutex::new(f)))
            .try_init(),
        None => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init(),
    };
    if let Err(e) = result {
        eprintln!("logging already initialised: {e}");
    }
}

fn log_dir(identifier: &str) -> Option<std::path::PathBuf> {
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

/// Open a project in the shared core; returns true on success (failures are logged, not fatal).
fn open_project(state: &AppState, path: &str, source: &str) -> bool {
    let resp = state.registry.invoke(
        &state.ctx,
        Request::new("project.open", serde_json::json!({ "path": path })),
    );
    match resp.error {
        None => {
            tracing::info!(%path, source, "opened project");
            true
        }
        Some(e) => {
            tracing::warn!(%path, source, code = e.code.as_str(), "could not open project: {}", e.message);
            false
        }
    }
}
