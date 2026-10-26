# Option B stack: egui viewport app + Tauri node-graph editor over a local socket

Researched 2026-10-26. Tags: **VERIFIED** = read in a primary source (crates.io API, the published crate source/README, an upstream repo file at the release tag, or official docs). **REPORTED** = a secondary source or an issue tracker. **INFERENCE** = my own conclusion from verified facts.
Sources were read from the published `.crate` tarballs and the egui `0.37.0` git tag (changelogs, demo).

## 0. What the repo pins today (VERIFIED, from the repo)

| Item | Value | Where |
|---|---|---|
| rustc | 1.98.1. There is no `rust-toolchain` file; `rust-version = "1.98"` | `Cargo.toml` `[workspace.package]`, `rustc --version` |
| wgpu | `30` → 30.0.1 locked (naga 30.0.1); `default-features = false, features = ["wgsl","metal","vulkan","dx12"]` | `crates/jd-gpu/Cargo.toml`, `Cargo.lock` |
| tauri | `2.12` → 2.12.4 locked (2.12.5 is out). tao 0.37.1, wry 0.57.0, webkit2gtk 2.0.2 | `apps/desktop/src-tauri/Cargo.toml`, `Cargo.lock` |
| tokio | 1.53.3 | `Cargo.lock` |
| raw-window-handle | 0.6.2 | `Cargo.lock` |
| lints | `unsafe_code = "forbid"` workspace-wide | `Cargo.toml` |
| jd-gpu device | Its own `wgpu::Instance::default()` plus a HighPerformance adapter, `required_limits: adapter.limits()` (needed for storage bindings over 128 MiB) | `crates/jd-gpu/src/lib.rs` |
| Envelope | `Request { op, schema_version, request_id: Option<RequestId>, params, client }`, `Response { op, request_id, status, result, job_id, error, warnings }`. **The correlation id already exists.** | `crates/jd-core/src/envelope.rs` |
| Jobs | Long ops return `status: accepted` + `job_id`. Callers **poll** `jobs.list` for progress; there is no push today | `crates/jd-core/src/jobs.rs` |
| Fonts / palette | `@fontsource-variable/host-grotesk` (woff2 only). Dark tokens: `--color-jd-bg-0 #0c0f15`, `bg-1 #151a24`, `bg-2 #1d2331`, `bg-3 #283043`, `line #2b3346`, `fg-0 #eef1f7`, `fg-1 #bec6d5`, `fg-2 #8a94a9`, `accent #6b8cff`, `primary #3f63f2`, `warn #f5c451`, `error #ff7b72` | `apps/desktop/package.json`, `apps/desktop/src/index.css` |

## 1. egui / eframe versions and the wgpu consequence

### Versions (VERIFIED, `cargo info` plus the crates.io API)
- `egui` / `eframe` / `egui-wgpu` / `egui-winit` / `egui_kittest`: **0.37.0** (released 2026-10-15). MIT OR Apache-2.0. MSRV 1.95, which is below our 1.98. https://crates.io/crates/eframe/0.37.0
- The `egui-wgpu 0.37.0` dependency list has **`wgpu ^30.0`**, and `eframe 0.37.0` has `wgpu ^30.0` (optional). Source: `https://crates.io/api/v1/crates/egui-wgpu/0.37.0/dependencies`.
- The egui-wgpu CHANGELOG says "0.36.0 – Upgrade wgpu to v30 (#8289)". The 0.37.0 release kept wgpu 30. https://github.com/emilk/egui/blob/0.37.0/crates/egui-wgpu/CHANGELOG.md
- eframe 0.37 uses **winit `^0.30.13`**. winit 0.31 is still beta (0.31.0-beta.4, 2026-10-24).
- The `eframe` default features include `wgpu` (glow is opt-in), as well as `accesskit`, `wayland`, `x11`, `system_fonts`, `clipboard` and `default_fonts`.

### The wgpu-version consequence: **no version jump** (VERIFIED, INFERENCE for the feature note)
- eframe 0.37 and jd-gpu share **one** `wgpu 30.0.x` in the lockfile. crates/jd-gpu does not change.
- Watch out for feature unification. `eframe/wgpu` turns on `egui-wgpu/default`, which turns on `wgpu/default` + `wgpu/webgl`. wgpu's default set is `std, parking_lot, dx12, metal, gles, vulkan, wgsl, webgpu` (VERIFIED, `wgpu-30.0.1/Cargo.toml`). Any build that includes the egui app therefore compiles jd-gpu's wgpu with GLES (and a few more backends) as well. This only costs compile time and binary size; nothing breaks. To avoid it, depend on `eframe = { version = "0.37", default-features = false, features = ["wgpu_no_default_features", "accesskit", "wayland", "x11", "default_fonts", "clipboard"] }` and pick the wgpu backends yourself. Keep GLES if you want the Linux fallback.
- Upgrade cadence (VERIFIED from the CHANGELOG): egui moved wgpu 28 → 29 in 0.34.0 (2026-03), 29.0.1 in 0.34.2, and 30 in 0.36.0 (2026-08). egui follows each wgpu major within about 1–3 months. **INFERENCE:** the workspace and egui must move to wgpu 31 together. Pin jd-gpu's wgpu to whatever eframe uses and upgrade both in one PR.

### Sharing one GPU device (VERIFIED APIs, INFERENCE recommendation)
- `egui_wgpu::WgpuSetup::Existing(WgpuSetupExisting { instance, adapter, device, queue })` lets eframe run on a device you created. `WgpuSetupCreateNew` has `device_descriptor: Arc<dyn Fn(&Adapter) -> DeviceDescriptor>`, `native_adapter_selector` and `power_preference` (`egui-wgpu-0.37.0/src/setup.rs`).
- `wgpu::Device` and `wgpu::Queue` are `#[derive(Clone)]` in wgpu 30 (`src/api/device.rs:19`, `queue.rs:14`).
- **Recommendation:** let eframe create the device with `device_descriptor = |a| DeviceDescriptor { required_limits: a.limits(), .. }`, because jd-gpu needs adapter limits for big storage buffers. Then hand `render_state.device/queue` to a new `jd_gpu::Gpu::from_device(device, queue)`. Erosion output buffers can then feed the viewport's vertex/height textures with **no CPU readback**. Going the other way, by passing jd-gpu's headless device into `Existing`, is riskier: jd-gpu picks its adapter without a `compatible_surface`, so that adapter may not be able to present to the window (INFERENCE).

### Rendering a 3D terrain viewport inside an egui panel
egui documents two options (VERIFIED, egui README "How do I render 3D stuff in an egui area?"): `Shape::Callback` and render-to-texture.

**A. Paint callback (`egui_wgpu::CallbackTrait`)** (VERIFIED, `egui-wgpu-0.37.0/src/renderer.rs` + `crates/egui_demo_app/src/apps/custom3d_wgpu.rs` @0.37.0)
- `Callback::new_paint_callback(rect, impl CallbackTrait)` → `ui.painter().add(...)`.
- There are three stages: `prepare(device, queue, screen_descriptor, egui_encoder, &mut CallbackResources) -> Vec<CommandBuffer>`, then `finish_prepare`, then `paint(info, &mut RenderPass<'static>, &CallbackResources)`. All command buffers go to the GPU in a single submit.
- Pipelines and buffers live in `render_state.renderer.write().callback_resources` (a type map) because they must outlive the render pass.
- egui sets the viewport to the callback rect (`info.viewport_in_pixels()`) and scissors it.
- **You draw into egui's own pass,** so your pipeline must match egui's target format, MSAA count and depth format. Depth exists only if you set `NativeOptions::depth_buffer` (default 0, "egui doesn't need the depth buffer"). MSAA comes from `NativeOptions::multisampling` (default 0) and applies **to the whole UI pass**. egui's own pipeline has depth writes off.
- Colour: egui prefers a **non-sRGB** framebuffer (`Rgba8Unorm`/`Bgra8Unorm`). It warns on an sRGB one and does gamma itself (`preferred_framebuffer_format`, `egui.wgsl`), so your shader must write gamma-encoded colour.

**B. Render-to-texture → `TextureId`** (VERIFIED APIs, INFERENCE recommendation): **recommended for the terrain viewport.**
- Use `Renderer::register_native_texture(device, &view, filter) -> TextureId` and `update_egui_texture_from_wgpu_texture(...)` (on resize). The doc says the texture **must be `Rgba8Unorm`**.
- Steps:
  1. Own your colour target (`Rgba8Unorm`, created with `view_formats: [Rgba8UnormSrgb]` so you can render through an sRGB view and get gamma encoding in hardware; INFERENCE).
  2. Own a `Depth32Float` texture and an MSAA colour texture (4×) that resolves into the colour target.
  3. Recreate them when the panel's `rect.size() * pixels_per_point` changes.
  4. Encode the pass in a callback's `prepare` (using `egui_encoder`, so it is submitted with egui's frame) or submit your own command buffer from `App::ui`.
  5. Show it with `ui.image((tex_id, size))` or `ui.allocate_rect(rect, Sense::click_and_drag())` plus `painter.image(...)` for orbit, pan and zoom.
- Why this beats A for a pro viewport: you get your own depth and MSAA without forcing MSAA on the UI. You can also do separate-resolution rendering (render at reduced resolution while orbiting), GPU picking (read back an ID buffer), screenshots and the "export view" feature from the same target, and deterministic kittest snapshots of just the viewport.
- Repaint: egui is reactive ("only repaints when there is interaction ... or an animation", VERIFIED README). Call `ctx.request_repaint()` while the camera moves or tiles stream in.

## 2. egui_kittest (VERIFIED, `egui_kittest-0.37.0` source + README + CHANGELOG @0.37.0)
- Version **0.37.0**, MIT OR Apache-2.0. Features: `wgpu`, `snapshot`, `video`, `eframe`, `x11`. It depends on `kittest ^0.5.0` (AccessKit-based) and `wgpu ^30`.
- Queries (kittest 0.5 `Queryable`): `get_by_label`, `get_by_label_contains`, `get_by_role`, `get_by_role_and_label`, `get_by_value`, plus `get_all_*` and `query_*` (Option) variants. Nodes support `.click()`, `.hover()`, `.type_text()`, and `accesskit_node().toggled()` etc. The harness has `key_press`, `key_press_modifiers`, `hover_at`, `drag_at` and `run()`/`step()`.
- Harnesses: `Harness::new_ui(|ui| ..)` for widgets, and `Harness::new_eframe(|cc| App::new(cc))` / `HarnessBuilder::build_eframe` for the whole app. With the `wgpu` feature, the harness injects a real `wgpu_render_state` into `CreationContext` and `Frame` (`src/wgpu.rs:136-137`). **Paint callbacks and render-to-texture run in tests.**
- Snapshots: `harness.snapshot("name")` writes `tests/snapshots/*.png`. Comparison uses dify YIQ `threshold` (default 0.6) and `max_failed_pixels` (default 0), with per-OS overrides in `kittest.toml`. Update with `UPDATE_SNAPSHOTS=true` (failing only) or `=force`. Gitignore `*.diff.png` / `*.new.png`.
- Headless/CI: `default_wgpu_setup()` uses `without_display_handle()` and **prefers CPU adapters** (lavapipe/llvmpipe, WARP), then discrete GPUs. You need Mesa lavapipe on Linux CI; macOS runners use Metal.
- New in 0.37:
  - The test **fails if any input widget has no accessible name**. Opt out with `with_accessibility_check(false)`.
  - **Missing glyphs panic by default** (`MissingGlyphPolicy`), so load Host Grotesk in the harness via `with_context_setup` (also new).
  - Tests can be recorded to mp4: env `KITTEST_RECORD=1|open`, feature **`video`**, needs ffmpeg. The README calls the feature "recording" and the top-level CHANGELOG calls the env var `EGUI_KITTEST_RECORD`, but the code says `KITTEST_RECORD` (`src/recording.rs:24`).
- Limits:
  - The README itself says image snapshots differ across GPU, driver and backend. MSAA sample placement, texture filtering, float rounding and derivatives are all implementation-defined. Its advice: "avoid enabling MSAA whenever it's not explicitly tested", and prefer regular or `insta` tests.
  - Only the root viewport is driven: the code handles `ViewportId::ROOT` input and commands (VERIFIED in `lib.rs`; INFERENCE that multi-viewport is unsupported).
  - kittest **cannot see the Tauri editor process**. Cross-process tests must go through the socket protocol, or through Playwright on the webview.
- Related (VERIFIED, top-level CHANGELOG 0.35.0): eframe has an `inspection` feature. `EGUI_INSPECTION=1` listens on port 5719 and exposes the AccessKit tree. `egui_mcp` (rerun-io/kittest_inspector) lets an agent drive a running egui app, which is useful for this repo's agent workflows.

## 3. Local IPC

### `interprocess` (VERIFIED, `interprocess-2.4.7` README + source)
- **2.4.7** (2026-10-23), licence **`0BSD OR Apache-2.0`**. Both are permissive and compatible with this repo's MIT licence. MSRV 1.75. About 4.6M recent downloads; maintenance badge "passive". Platform support is "Explicit" (CI) on Windows, Linux and macOS.
- `local_socket` is the flagship API: a Unix domain socket on Unix and a **named pipe on Windows**. It adds **no framing** ("the bytes you write are the exact bytes that come out the other end").
- Tokio support comes from feature `tokio`: `interprocess::local_socket::tokio::{prelude::*, Stream}`, `ListenerOptions::new().name(name).create_tokio()`, and `listener.accept().await`.
- Name types:
  - `GenericNamespaced`: Windows `\\.\pipe\<name>`. Linux uses the **abstract namespace** (≤107 bytes). Other Unixes (macOS) get `/tmp/<name>`.
  - `GenericFilePath`: a real socket path on Unix (or a `\\.\pipe\...` path on Windows).
- `ListenerOptions` has `try_overwrite(true)` (to reclaim a "corpse socket"), `reclaim_name`, `nonblocking`, Unix `mode(0o600)` (fchmod before bind, so there is no umask race, since 2.3.0) and a Windows `security_descriptor`. `PeerCreds` gives the peer's pid/uid where the OS supports it.
- **Security (do not skip):**
  - Linux abstract sockets ignore filesystem permissions, so any local process can connect (REPORTED, `man 7 unix`, https://man7.org/linux/man-pages/man7/unix.7.html).
  - A Windows named pipe's default DACL "grant[s] full control to LocalSystem, administrators, and the creator owner ... and read access to members of the Everyone group and the anonymous account" (VERIFIED, https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights).
  - **Recommendation:** on Unix use `GenericFilePath` under `$XDG_RUNTIME_DIR` (Linux; directory mode 0700) or `$TMPDIR` (macOS, per-user) with `mode(0o600)`. On Windows use a random pipe name. On every platform, the parent passes a **random one-time token** in the child's environment, and the first frame must present it. Optionally check `PeerCreds` pid against the spawned child.

### Alternatives
- `parity-tokio-ipc` **0.9.0**: last release **2021-07-06**, repo last push 2024-02 (not archived). MIT/Apache-2.0. Stale; do not use (VERIFIED, crates.io + GitHub API).
- Plain std + tokio: `tokio::net::UnixListener` on Unix and `tokio::net::windows::named_pipe` on Windows. Both are safe APIs, present in the locked tokio 1.53.3 (VERIFIED, `src/net/windows/named_pipe.rs`). That is no new dependency but **two code paths** plus your own name/permission handling. interprocess wraps exactly this.
- `unsafe_code = "forbid"` applies to our crates only. interprocess and tokio use unsafe internally, as wgpu already does.

### Framing: newline-delimited JSON (NDJSON)
- Compact `serde_json` output never contains a raw newline, because JSON escapes control characters in strings. So one JSON object per `\n` line is unambiguous.
- This is the framing MCP stdio uses: "Messages are delimited by newlines, and MUST NOT contain embedded newlines" (VERIFIED, https://modelcontextprotocol.io/specification/2025-06-18/basic/transports). It is also what `apps/mcp` already speaks via rmcp `transport-io`.
- Tooling: `tokio::io::BufReader::lines()` is enough. `tokio_util::codec::LinesCodec::new_with_max_length(..)` adds a **line length cap**; use one, because unbounded lines are a memory DoS. tokio-util 0.7.20 is MIT.
- Length-prefixed framing (`LengthDelimitedCodec`, or LSP's `Content-Length:` headers) only pays off for **binary payloads**. In Option B the heightfield tiles stay inside the egui process, so nothing large crosses the socket. Node-preview thumbnails can be small base64 PNGs, or a path into jd-cache on disk. **Start with NDJSON;** switch to length-prefixed only if a binary stream appears.

### Request/response correlation and server push (INFERENCE, following the JSON-RPC/MCP pattern)
- Requests already carry `request_id`. The client keeps `HashMap<RequestId, oneshot::Sender<Response>>`; a single reader task routes each `Response` by `request_id`. Many requests can be in flight on one connection, and responses may arrive out of order.
- Push events use a third frame shape that can never be confused with a `Response`, for example `{"event":"job.progress","data":{...}}` or `{"event":"graph.changed",...}`. This mirrors JSON-RPC notifications, which have no `id`.
- Wrap all frames in one tagged enum: `#[serde(untagged)] enum Frame { Request(Request), Response(Response), Event(Event) }`, or tag it explicitly with `"kind"`. Add a `hello` handshake carrying `schema_version` + token.
- Because job polling via `jobs.list` already exists, push is an optimisation and can come later.

## 4. Tauri 2 as the socket client, and process supervision

### Tauri app as client (VERIFIED APIs / docs, INFERENCE design)
- **Keep the frontend unchanged.** In `setup()`, spawn a task with `tauri::async_runtime::spawn` that connects the interprocess tokio `Stream`. It holds an `mpsc::Sender<Request>` and the pending-map in managed state (`app.manage(..)`).
- The existing `#[tauri::command] async fn op(state, request) -> Result<Response, JdError>` (`src-tauri/src/lib.rs:31`) changes from `registry.invoke(..)` to "send over socket and await the oneshot". The webview keeps calling `invoke('op', ...)`.
- Push events to the webview:
  - `app.emit("engine-event", payload)`. The Tauri docs warn: "The event system is not designed for low latency or high throughput situations."
  - For streams, use **`tauri::ipc::Channel<T>`**, which is "designed to be fast and deliver ordered data" (VERIFIED, https://v2.tauri.app/develop/calling-frontend/). The frontend creates `new Channel()` and passes it to a `subscribe` command.
- `layer_preview` currently returns raw bytes via `tauri::ipc::Response` (`lib.rs:47`). With the engine elsewhere, either drop it (the viewport moves to egui) or serve small previews over the socket.

### Who launches whom
- **tauri-plugin-shell sidecar** is for **Tauri as parent** (VERIFIED, `tauri-plugin-shell` 2.5.0 source + https://v2.tauri.app/develop/sidecar/):
  - Binaries are listed in `bundle.externalBin` and must be named with the target triple (`my-sidecar-aarch64-apple-darwin`; get it from `rustc --print host-tuple`).
  - `app.shell().sidecar("name")?.spawn()` returns `(Receiver<CommandEvent>, CommandChild)`.
  - The plugin **kills tracked children on `RunEvent::Exit`** (`lib.rs:163-170`). `set_process_group(true)` (new in the 2.x line) kills the POSIX process group or Windows job object.
  - Stable is **2.5.0**; `3.0.0-alpha.3` is the top crates.io version, so pin `2.5`. Tauri 3.0.0-alpha.5 also exists; stay on 2.12.x.
- **egui as parent with `std::process::Command`** (INFERENCE, matches "egui app owns the engine"):
  - The egui process is the server and the main app. It spawns the editor with env `JD_SOCKET=<name>`, `JD_TOKEN=<random>`, and on Linux `XDG_ACTIVATION_TOKEN` (see §5).
  - **Stop together:** the editor exits when its socket reads EOF, which also covers a crash or kill of the parent on every OS without job objects. The egui app calls `child.kill()` and `wait()` on exit, and on child EOF it can respawn the editor or quit.
  - This needs no shell plugin and no capability config.
- **Packaging trade-off (INFERENCE):** the Tauri bundler currently provides icons, `fileAssociations` (`.jdproj`), Assets.car and the macOS bundle.
  - Either keep Tauri as the bundled entry point, launching egui as a sidecar and forwarding file-open events over the socket.
  - Or bundle the egui binary with **cargo-packager 0.11.8** (Apache-2.0 OR MIT, CrabNebula) and ship the Tauri editor binary inside it.
  - Decide this before writing launch code. Process direction and packaging are coupled.
- **Single instance:** `tauri-plugin-single-instance` stable **2.5.4** (Linux via DBus, with a `DBUS_ID` note for Flatpak/Snap; register it first; VERIFIED README). In egui-parent mode the egui app owns single-instance. The simplest check is "try to connect to the well-known socket; if a live server answers, forward argv and exit" (INFERENCE), which reuses interprocess and needs no extra crate.

### Window placement, two windows side by side
- **macOS / Windows / X11:** both sides can place windows.
  - egui: `ViewportBuilder::with_position`, `ViewportCommand::OuterPosition`; read the current frame via `ctx.input(|i| i.viewport().outer_rect)`.
  - Tauri: `WebviewWindow::set_position` / `set_size`.
  - The egui app reads its monitor and outer rect and sends the desired editor rect over the socket, or passes it as args at spawn.
  - The repo already uses `tauri-plugin-window-state` 2.5, which persists position. Keep that only for the editor, or let egui decide.
- **macOS, two processes = two Dock icons, two menu bars, two Cmd-Tab entries** (INFERENCE from process model).
  - Tauri has `app.set_activation_policy(ActivationPolicy::Accessory)` and `set_dock_visibility(false)` (VERIFIED, `tauri-2.12.4/src/app.rs:658,678`).
  - Accessory apps have no Dock icon **and no menu bar** (REPORTED, Apple `NSApplication.ActivationPolicy.accessory` docs, https://developer.apple.com/documentation/appkit/nsapplication/activationpolicy/accessory). The editor would then rely on the egui app's menu bar, or on in-window menus.

## 5. Wayland specifics (winit 0.30.13 / tao 0.37.1 source, VERIFIED)
- **No positioning:** `set_outer_position` on Wayland is "Unsupported", `outer_position`/`inner_position` "Always returns NotSupportedError" (winit `src/window.rs:665-735`). egui's `with_position`: "Android / Wayland: Unsupported". tao (Tauri's windowing) is the same: `set_outer_position` "Unsupported", `outer_position` "Has no effect, since Wayland doesn't support a global coordinate system".
  **→ "Two windows side by side" cannot be enforced on Wayland by either process.** Options: let the user's compositor tile; size both windows to half the monitor and accept compositor placement; or fall back to X11/XWayland (`WINIT_UNIX_BACKEND`/`GDK_BACKEND=x11`), which costs fractional scaling quality. Cross-process parenting protocols (xdg-foreign) are not exposed by winit/tao (INFERENCE).
- **Focus handoff:**
  - Wayland blocks focus stealing, and `request_user_attention` "requires `xdg_activation_v1`".
  - eframe 0.37 added `Frame::request_activation_token()` / `take_activation_token()`. Pass the token to the child in `XDG_ACTIVATION_TOKEN` (X11: `DESKTOP_STARTUP_ID`) so the spawned editor gets focus. eframe also now reads the token from its own env on window creation (CHANGELOG 0.37 #8406; `epi.rs:866`).
  - The `.desktop` file needs `StartupNotify=true`, and `with_app_id` must match the `.desktop` id. On GNOME/Wayland the app icon comes from the `.desktop` file (egui issue #3992 "App icon not working on GNOME/Wayland", open, REPORTED).
  - Re-focusing the egui window when the user clicks a node in the editor needs a token from the editor side. A GTK-side token from Tauri is not exposed (INFERENCE).
- **Decorations:** on GNOME (Mutter), which has no server-side decorations, winit draws client-side decorations via sctk-adwaita (`wayland-csd-adwaita` default feature). KDE/Sway give SSD. eframe can draw its own title bar (`with_decorations(false)`), but window move/resize on Wayland must go through `ViewportCommand::StartDrag`, never manual positioning.
- **Open egui Wayland issues worth knowing** (REPORTED, `gh issue list -R emilk/egui`):
  - #8776: hidden viewports never paint, and screenshots fail.
  - #8761: the default clear colour is 70% alpha, so frames blend.
  - #8523: the event loop spins at idle.
  - #8249: keyboard is ignored on Hyprland.
  - #7959: drag/resize leaves the mouse stuck.
  - #1563: file drag-and-drop does not work.
  - #8142: a `RequestActivationToken` viewport command was requested.
- **Tauri on Linux/Wayland:**
  - WebKitGTK shows blank windows or Error 71 on NVIDIA. The official fixes, in order: `nvidia_drm.modeset=1`, `__NV_DISABLE_EXPLICIT_SYNC=1`, `WEBKIT_DISABLE_DMABUF_RENDERER=1`, `WEBKIT_DISABLE_COMPOSITING_MODE=1` (VERIFIED, https://v2.tauri.app/develop/debug/linux-graphics/).
  - The editor and the viewport may use **different GPU paths** (WebKitGTK GL vs wgpu Vulkan) on the same machine, so test both on NVIDIA.

## 6. Fonts and theming
- **Host Grotesk TTF:** egui's font stack is `skrifa 0.44` (parsing) + `vello_cpu` (raster) (VERIFIED, `epaint-0.37.0/Cargo.toml`). It takes `.ttf`/`.otf`. The repo only has **woff2** (fontsource), which egui cannot read; this is INFERENCE, since epaint has no woff handling.
  - Use Google Fonts' variable TTFs: `ofl/hostgrotesk/HostGrotesk[wght].ttf` (81,784 B) and `HostGrotesk-Italic[wght].ttf`. Licence OFL-1.1, so embedding is fine (VERIFIED, https://github.com/google/fonts/tree/main/ofl/hostgrotesk).
- **Loading** (VERIFIED, `egui-0.37.0/src/context.rs`, `epaint text/*`):
  - `ctx.add_font(FontInsert::new("host-grotesk", FontData::from_static(include_bytes!(..)), vec![InsertFontFamily { family: FontFamily::Proportional, priority: FontPriority::Highest }]))`.
  - Or the full `ctx.set_fonts(FontDefinitions)`.
  - It applies "right away when no font is loaded yet" (0.37). Call it in the `CreationContext` and in the kittest `with_context_setup`.
- **Variable weight:** `FontData::variation_axes()`. `FontTweak { coords: VariationCoords, .. }` sets a per-font default `wght`; register the same bytes twice (e.g. "hg-regular" `wght=400`, "hg-semibold" `wght=600`) under named `FontFamily::Name(..)`. `TextFormat.coords` gives per-run variation.
  - `FontId` has no weight field (`font_id.rs`: "TODO(emilk): weight (bold), italics"). Bold therefore means a separate family. Row height ignores variation coords (TODO in `fonts.rs:392`).
- **0.37 system fonts:** `system_fonts` (a default eframe feature, `egui_system_font_provider` via fontique) supplies fallback glyphs and colour emoji. Keep it on for user-typed names in other scripts.
- **Palette:**
  - Start from `Visuals::dark()`, then set `panel_fill = #151a24` (bg-1), `window_fill = #1d2331` (bg-2), `extreme_bg_color = #0c0f15` (bg-0, text fields/viewport background) and `faint_bg_color = #1d2331`.
  - Widget states: `widgets.noninteractive.bg_stroke = #2b3346` (line), `widgets.inactive.weak_bg_fill = #283043`, `hovered` brighter, `active` `#3f63f2` (primary).
  - Also `selection.bg_fill = #3f63f2`, `selection.stroke = #6b8cff`, `hyperlink_color = #6b8cff`, `override_text_color = None` with `widgets.*.fg_stroke = #eef1f7` / `#bec6d5`, and `warn_fg_color = #f5c451`, `error_fg_color = #ff7b72`.
  - Apply with `ctx.set_visuals_of(Theme::Dark, v)` and `ctx.set_theme(Theme::Dark)`.
  - 0.37 added an **experimental** `StyleProvider` with class-based widget themes behind the `experimental` feature. Don't build on it yet.
- **Single source of truth (INFERENCE):** generate the egui palette and the CSS tokens from one table, as the repo already does for schemas, so the two windows cannot drift.

## 7. Known pitfalls of egui for a pro 3D tool
- **Immediate-mode CPU cost** (VERIFIED, egui README "Disadvantages"):
  - "a full layout each frame". Huge scroll areas are slow; lay out only what's visible (`ScrollArea::show_rows`). Typical cost is "1-2 ms per frame", and an idle app costs nothing.
  - Layout has a first-frame jitter that egui papers over with a size cached from the previous frame, or an extra pass via `request_discard`.
  - Open issues (REPORTED): #8093 text layout performance regression, #7885 bad performance with many windows, #7761 a WGPU backend transition tracking issue (performance).
  - The node graph lives in the webview in Option B, which avoids egui's weakest case.
- **"Not native, breaking changes"** (VERIFIED, README): "If you want a GUI that looks native, egui is not for you. If you want something that doesn't break when you upgrade it, egui isn't for you (yet)." Expect API churn about every 6–8 weeks; 0.34 to 0.37 took seven months.
- **Text input / IME** (REPORTED):
  - 0.35 overhauled IME (underline preedit, `Memory::owns_ime_events`), and egui-winit calls `set_ime_allowed` / `set_ime_cursor_area` (VERIFIED, `egui-winit/src/lib.rs:1272-1297`).
  - Still open: #3532 IME input broken, #2529 Japanese via fcitx5, #7975 Linux X11 + Fcitx5 preedit missing, #7974 macOS Korean hanja, #2317 Windows 11 IME switching.
  - Mitigation: the text-heavy surfaces (graph editor, node parameter text) stay in the webview. The egui side has short numeric fields.
- **Keyboard and pointer routing between viewport shortcuts and widgets:** check `ctx.egui_wants_keyboard_input()` / `egui_wants_pointer_input()` / `egui_is_using_pointer()` before treating keys or drags as camera input (VERIFIED, `context.rs:3472-3485`). Otherwise typing "w" in a field moves the camera.
- **Accessibility:**
  - AccessKit is always on in egui (0.34 removed the feature flag; eframe's `accesskit` feature wires the platform adapter).
  - `accesskit_winit 0.34.2` has adapters for macOS, Windows **and Unix AT-SPI** (`accesskit_unix`) (VERIFIED, crates.io deps). The egui README still says "Windows and macOS", which is stale.
  - 0.37 improved the trees and replaced `WidgetType` with `accesskit::Role`.
  - Gaps (REPORTED): #8671, where an occluded or minimized window never answers AccessKit; #8700, where `egui_extras::Table` exposes no row/cell roles; #3647, a `labelled_by` panic.
  - **The 3D viewport itself is opaque to screen readers.** Give it an accessible name and expose camera/selection state as labelled widgets (kittest now enforces names on inputs).
- **GPU/threading:** never block the UI thread on jd-core evaluation. The README recommends channels with `try_recv`, `poll_promise`, or `tokio::sync::watch`. Long jobs already return `accepted` + `job_id`.
- **Multi-viewport:** egui can open extra native windows (`show_viewport_deferred`), but Wayland support for hidden or extra viewports is buggy (#8776). The editor is in its own process anyway.

## Sources (primary unless marked)
- crates.io API: `/api/v1/crates/{egui-wgpu,eframe,egui-winit,egui_kittest}/0.37.0/dependencies`; version lists for egui-wgpu, tauri, tauri-plugin-shell, tauri-plugin-single-instance, interprocess, winit; parity-tokio-ipc metadata.
- Crate sources read: egui/eframe/egui-wgpu/egui-winit/egui_kittest/epaint 0.37.0, kittest 0.5.0, wgpu 30.0.1, interprocess 2.4.7, parity-tokio-ipc 0.9.0, tauri 2.12.4, tauri-plugin-shell 2.5.0, tauri-plugin-single-instance 2.5.4, winit 0.30.13, tao 0.37.1, tokio 1.53.3.
- https://github.com/emilk/egui/blob/0.37.0/CHANGELOG.md, `crates/eframe/CHANGELOG.md`, `crates/egui-wgpu/CHANGELOG.md`, `crates/egui_kittest/{README,CHANGELOG}.md`, `crates/egui_demo_app/src/apps/custom3d_wgpu.rs`
- https://v2.tauri.app/develop/calling-frontend/ · https://v2.tauri.app/develop/sidecar/ · https://v2.tauri.app/develop/debug/linux-graphics/
- https://modelcontextprotocol.io/specification/2025-06-18/basic/transports
- https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights
- https://github.com/google/fonts/tree/main/ofl/hostgrotesk
- REPORTED: egui issue tracker (numbers above, `gh issue list -R emilk/egui`, 2026-10-26); `man 7 unix` (abstract namespace); Apple ActivationPolicy docs.
