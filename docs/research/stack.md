# Jenga Dunia — Technology Stack Research Report

**Date:** 2026-10-19
**Scope:** Tauri + Rust core + TypeScript frontend (Tailwind, Lucide, Host Grotesk), CLI + MCP server sharing the Rust core. macOS Apple Silicon first, then Windows/Linux. 4 GB RAM ceiling for typical workloads. MIT-licensed project.
**Local toolchain observed:** rustc 1.98.1, node 26, pnpm 10.33. Current stable rustc per `static.rust-lang.org/dist/channel-rust-stable.toml` is **1.99.1 (2026-10-13)** — the local toolchain is one minor behind; every crate below has an MSRV at or below 1.92, so 1.98.1 is fine.

## Evidence labels

- **VERIFIED** — read directly from crates.io / npm registry API, docs.rs, official docs, or the GitHub repo/API on 2026-10-19.
- **REPORTED** — stated by a secondary source (blog, search summary, issue comment, mirror) and not independently confirmed.
- **UNVERIFIED** — could not confirm; treat as a hypothesis to test in the prototype.

All crate versions/licenses below were pulled from the crates.io API (`max_stable_version` and the `license` field of the newest release) unless noted. npm versions/licenses from `registry.npmjs.org` `dist-tags.latest`.

---

## 1. Tauri 2.x

### Version, license, maintenance — VERIFIED

| Item | Value |
|---|---|
| `tauri` | **2.12.4** (2026-10-14), `Apache-2.0 OR MIT`, MSRV 1.90 |
| `tauri-build` | 2.7.3 |
| `tauri-cli` / `@tauri-apps/cli` | 2.12.3 |
| `@tauri-apps/api` | 2.12.4 (MIT OR Apache-2.0) |
| Maintenance | Very active: 2.12.2 → 2.12.4 all within 2026-10-09..14; 37.5M downloads |
| **Tauri 3** | `3.0.0-alpha.0` (2026-09-13) through `3.0.0-alpha.5` (2026-10-14) on crates.io; a new **`tauri-runtime-cef` 3.0.0-alpha.6** crate ("Tauri runtime interface for Chromium Embedded Framework") exists — VERIFIED on crates.io. A May 2026 digest mentions alpha.7 and CEF data-dir fixes (REPORTED). No public roadmap/timeline for 3.0 stable found (UNVERIFIED). |

Takeaway: build on **2.12.x**. Watch Tauri 3's CEF runtime closely — a bundled Chromium would make WebGPU uniform across all three OSes and would change the viewport decision below (at the cost of ~100+ MB bundle size, REPORTED/UNVERIFIED).

### Architecture — VERIFIED (Tauri docs)

Rust backend process + system WebView via `wry`/`tao`: **WKWebView** (macOS/iOS), **WebView2** (Windows, Chromium-based; preinstalled on Win10 1803+ / Win11; Evergreen bootstrapper otherwise), **WebKitGTK 4.1** (Linux). Linux build deps (official prerequisites page):

```
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

macOS: Xcode CLT (`xcode-select --install`) suffices for desktop. Windows: MSVC Build Tools "Desktop development with C++", `stable-msvc` toolchain, optional VBSCRIPT feature only for MSI.

### IPC — VERIFIED (docs + docs.rs)

- **Commands** (`#[tauri::command]`): default return path serializes to JSON; docs warn this "can slow down your application" for large data.
- **Raw binary responses**: return `tauri::ipc::Response::new(Vec<u8>)`. `InvokeResponseBody` has exactly two variants: `Json(String)` and `Raw(Vec<u8>)`, with `From<Vec<u8>>` → Raw and `From<String>` → Json. The IPC body is **either** raw bytes **or** JSON, never mixed — so a binary frame must carry its own header (or you send metadata in a separate call).
- **Raw binary requests**: pass `ArrayBuffer`/`Uint8Array` as the `invoke` payload (+ headers as 3rd arg); Rust side takes `tauri::ipc::Request` and matches `InvokeBody::Raw`.
- **Channels** (`tauri::ipc::Channel<T>`): the documented mechanism for ordered, high-throughput streaming (progress, chunked file transfer example uses `Channel<&[u8]>` with 4096-byte chunks). Frontend: `new Channel<T>()`, set `onmessage`, pass as an argument.
- **Events** (`emit`/`listen`): JSON strings, explicitly "not designed for low latency or high throughput."
- **Async commands**: `async fn` runs on a separate task; non-async commands run on the main thread unless `#[tauri::command(async)]`. Borrowed args (`&str`, `State<'_, T>`) need the `Result` return-type workaround.
- **Performance numbers** (REPORTED, anecdotal): Tauri discussion #11915 — ~5 ms for 10 MB binary on macOS vs ~200 ms on Windows (WebView2), older and non-scientific; shared memory only possible on WebView2 and was found slow. Third-party `tauri-conduit` self-benchmarks claim ~2.3 ms (invoke) vs ~0.2 ms (their binary path) for 64 KB, Rust-dispatch only. **No official benchmark exists** — the prototype must measure.

### Capabilities / permissions (ACL) — VERIFIED

- Files in `src-tauri/capabilities/*.json|toml` (or inline in `tauri.conf.json`). Fields: `identifier`, `description`, `windows` (labels, `"*"`), `webviews`, `permissions[]`, `platforms[]`, `remote.urls[]`.
- Permissions are identifiers like `core:window:allow-set-title`, `fs:default`; multiple capabilities for one window merge.
- **fs scopes** (`tauri-plugin-fs` 2.7.1): `allow`/`deny` arrays of `{ path }` with globs (`$APPDATA/*`, `$APPDATA/**/*`); `deny` wins; `..` traversal forbidden; base-dir vars: `$APPDATA $APPCONFIG $APPLOCALDATA $APPCACHE $APPLOG $HOME $DOCUMENT $DOWNLOAD $DESKTOP $PICTURE $VIDEO $AUDIO $RESOURCE $TEMP $CACHE $CONFIG $DATA $LOCALDATA $LOG $PUBLIC $FONT $EXE $RUNTIME $TEMPLATE`. `fs:default` = recursive read of app dirs + `mkdir`, plus `deny-default` on webview data dirs. Dotfiles need `requireLiteralLeadingDot: false` or explicit paths.
- Linux/Android caveat: Tauri cannot distinguish iframe vs window origin for `remote` capabilities.

Recommendation for Jenga Dunia: keep the webview's own fs scope minimal (`$APPDATA/**`, `$APPCACHE/**`), and route project open/save through **Rust commands that take a path chosen via `tauri-plugin-dialog`** — the Rust core already owns I/O, so the frontend never needs broad fs permissions.

### Official plugins — VERIFIED versions (all `Apache-2.0 OR MIT`, MSRV 1.90)

| Plugin | Rust crate | npm | Use |
|---|---|---|---|
| fs | 2.7.1 | 2.7.1 | scoped file access from JS (keep minimal) |
| dialog | 2.8.3 | 2.8.3 | open/save dialogs |
| shell | 2.5.0 | 2.5.0 | spawn sidecars (restricted args) |
| log | 2.10.1 | 2.10.1 | bridge JS console ↔ Rust `log` |
| store | 2.5.0 | 2.5.0 | user prefs KV |
| updater | 2.14.0 | 2.14.0 | signed updates |
| window-state | 2.5.0 | 2.5.0 | persist window geometry |
| process | 2.4.0 | — | relaunch/exit |
| opener | 2.7.2 | — | open URLs/paths in system apps |

Per-platform support matrix could not be extracted from the docs page (table indicators stripped) — UNVERIFIED, but all listed plugins are desktop-first.

### Multi-window / multi-webview — VERIFIED

`examples/multiwindow` and `examples/multiwebview` exist in the Tauri repo. Child webviews inside one window (`Window::add_child`) require the **`unstable`** crate feature — VERIFIED in the 2.12.4 feature list (`unstable`, `tracing`, `specta`, `devtools`, `protocol-asset`, `macos-private-api`, `isolation`, `dynamic-acl`, ...). The `specta` feature is worth enabling: it pairs with `tauri-specta` to generate TypeScript bindings for commands (REPORTED). For a node editor + viewport app, prefer **one window, one webview** and let the frontend manage panes.

Tauri 3 alpha (VERIFIED crates.io): `tauri` 3.0.0-alpha.5 adds `gtk3`/`gtk4` features and `tauri-runtime-cef` 3.0.0-alpha.6 lives in the official `tauri-apps/tauri` repo alongside `tauri-runtime-wry` 3.0.0-alpha.5 — i.e. CEF is a first-party *alternative* runtime, not (yet) a replacement for wry.

### Sidecars — VERIFIED

`bundle.externalBin` in `tauri.conf.json`; binaries must be named `<name>-<target-triple>` (`rustc --print host-tuple`). Spawn via `tauri_plugin_shell::ShellExt` → `app.shell().sidecar("name")` or JS `Command.sidecar(...)`; permissions `shell:allow-spawn`/`allow-execute` with `"sidecar": true` and optional fixed/regex-validated args.

**Recommendation for CLI/MCP:** do **not** make the CLI/MCP a sidecar of the GUI. Ship `jenga` (CLI) and `jenga-mcp` as separate binaries from the same workspace; the GUI links the core crates directly (in-process, no IPC). Optionally bundle `jenga-mcp` via `externalBin` so the GUI can offer "Register with Claude Code" (writes `.mcp.json`), but the GUI itself never needs to shell out to it.

### Bundling / signing / notarization (macOS) — VERIFIED

- Developer ID Application cert (Account Holder only can create). CI: export `.p12` → base64 → `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`; identity via `APPLE_SIGNING_IDENTITY` or `bundle.macOS.signingIdentity`.
- Notarization is **required** for Developer ID builds: either App Store Connect API (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`) or Apple ID (`APPLE_ID`, `APPLE_PASSWORD` app-specific, `APPLE_TEAM_ID`). `--skip-stapling` for a first pass.
- Ad-hoc: `"signingIdentity": "-"` (useful on Apple Silicon dev builds; users still get the Gatekeeper prompt).
- Entitlements/hardened runtime details are not on that page (UNVERIFIED here; Tauri adds hardened runtime by default — REPORTED).

### Updater signing — VERIFIED

Mandatory, cannot be disabled. `tauri signer generate -w ~/.tauri/jenga.key`; `TAURI_SIGNING_PRIVATE_KEY` (+ `_PASSWORD`) at build time; `plugins.updater.pubkey` = key **content**; `bundle.createUpdaterArtifacts: true` emits `.sig` files; endpoints support `{{target}}`, `{{arch}}`, `{{current_version}}`; static `latest.json` with `platforms.<os>-<arch>.{url,signature}`. Requires Rust ≥ 1.90.

### Known limitations: GPU / WebGPU inside the WebView (2026)

| Platform | WebGPU in system WebView | Evidence |
|---|---|---|
| macOS 26 (WKWebView) | **Enabled by default** on macOS/iOS/visionOS 26 (gpuweb Implementation-Status wiki; Tauri issue #6381 comment 2025-11-24 confirms it works in **production** builds; one user reported it did **not** work in `tauri dev` builds). Not available on macOS ≤ 15 — Safari feature flags do not apply to WKWebView (Apple forum reply). | VERIFIED (status wiki) / REPORTED (dev-build caveat) |
| Windows (WebView2) | Chromium ≥ 113 ships WebGPU; Tauri issue #6381 (2023) says WebView2 stable needed `additionalBrowserArgs: "--enable-unsafe-webgpu"`. Whether current WebView2 stable enables it by default is **UNVERIFIED** — must test. | REPORTED/UNVERIFIED |
| Linux (WebKitGTK) | **No WebGPU.** 2023 maintainer statement "not supported, nobody working on it"; nothing newer found. Chromium itself on Linux only enables WebGPU by default for Intel Gen12+ and NVIDIA/Wayland. | REPORTED |
| WebGL 2 | Available in all three WebViews. | REPORTED (common knowledge, not re-verified) |

Conclusion: **WebGPU-in-WebView is not a cross-platform baseline for Tauri 2 in 2026.** It is fine as a macOS-26-only fast path. WebGL 2 is the only portable in-WebView GPU API.

### Embedding a native wgpu surface in a Tauri window — what is actually supported

**VERIFIED on docs.rs (tauri 2.12.4):**
- `tauri::Window` and `tauri::WebviewWindow` both implement `raw_window_handle::HasWindowHandle` + `HasDisplayHandle` (raw-window-handle **0.6.2**, the same major wgpu 30 requires — `^0.6.2`). So `instance.create_surface(window)` compiles directly.
- `WebviewWindow::with_webview(|platform_webview| ...)` runs on the main thread and exposes the native webview (`ns_window`/WKWebView on macOS, WebView2 controller on Windows, WebKitGTK on Linux). `Window::run_on_main_thread`. Linux-only `gtk_window()` / `default_vbox()`.
- No `hwnd()`/`ns_view()` convenience methods were found on the `Window` docs page in the portion read — use `window_handle().as_raw()` and match `RawWindowHandle::AppKit/Win32/Xlib/Wayland`.

**Existing examples (VERIFIED by reading source):**
- `FabianLars/tauri-v2-wgpu` — ports the wry wgpu example: in `setup`, `instance.create_surface(window)` on the main `WebviewWindow`, draws a triangle, manages `surface/device/queue` via `app.manage`, redraws on `RunEvent::WindowEvent`. Archived Dec 2024, 2 commits. The webview sits on top with a transparent background.
- `tauri-apps/wry/examples/wgpu.rs` — winit window + wgpu surface cleared to `TRANSPARENT`, transparent WebView layered via `with_transparent(true)`.
- `Djjanks/tauri-rust-render`, `EddieRydell/donder`, `kellertobias/tosklight` — named in a third-party issue as running a bare Tauri window with its own wgpu loop; not inspected (UNVERIFIED).

**Known problems (VERIFIED, Tauri issue #9220, closed "not planned" 2024-03-28; duplicate reported again April 2026):**
- On **Linux**, GTK owns the window's drawing loop; a wgpu surface on the same X11/GTK surface **flickers/fights** with the webview. Maintainer: cannot be fixed while GTK is required; the child-X11-window trick "doesn't work on Wayland at all"; suggested `gtk::GlArea` + `gtk::Fixed` with manual drawing (no Rust crate support).
- Windows: WebView2 runs as a child HWND and **does not use the app's D3D device**; transparent child webviews via `parent_raw` showed black backgrounds (issue #12450, REPORTED).
- macOS: the FabianLars example renders; no documented issue found, but a transparent WKWebView over a Metal layer is the least-tested configuration (UNVERIFIED stability under window resize / fullscreen / multiple displays).

**Alternative: render-to-texture → frontend.** Render offscreen with wgpu into a texture, copy to a `MAP_READ` staging buffer, `map_async`, then push bytes to the webview via `Channel<Vec<u8>>` or `Response::new(bytes)` and blit into a `<canvas>` (`ImageData`/`createImageBitmap`) or a WebGL texture. Cost at 1920×1080 RGBA8 = 8.3 MB/frame; at 30 fps that is ~250 MB/s over IPC, which the macOS numbers above suggest is feasible but Windows (~200 ms/10 MB anecdotal) suggests is not. Also doable as a shared-memory or local HTTP/WebSocket stream (custom protocol handler `tauri::Builder::register_asynchronous_uri_scheme_protocol` — REPORTED) to bypass the JSON/IPC layer.

Trade-offs are tabulated in the final section.

---

## 2. wgpu

### Version, license, status — VERIFIED

- `wgpu` **30.0.1** (2026-08-22), `MIT OR Apache-2.0`, MSRV **1.87**. MSRV policy since v29: never above "stable − 3", bumps are breaking. 30.0.0 was 2026-07-01; wgpu releases a breaking major roughly every 2–3 months (29.0.0 2026-03-19). Plan for a **major upgrade every quarter** — isolate wgpu behind one crate.
- `naga` 30.0.1 (same cadence). New `naga-types` crate in v30; `wgpu-naga-bridge` crate in v29.
- `wgpu-hal` 30.0.1.
- Backends: Metal, Vulkan, DX12, GLES/WebGL2, WebGPU (via `web-sys`), plus `noop`. The browser backend crashed on failed `requestAdapter()` before 30.0.1 (fixed).

### Headline changes relevant here — VERIFIED (GitHub releases)

- v30: optional vertex buffer slots (`buffers: &[Option<VertexBufferLayout>]`), experimental `i16/u16` in WGSL (`SHADER_I16`, Vulkan/Metal/DX12), `ComputePass::transition_resources`, per-vertex attributes on Metal/DX12, combined buffer-argument limit (affects Metal), HDR/surface color-space selection, integer shader I/O now requires explicit `@interpolate(flat)`.
- v29: `get_current_texture` → `CurrentSurfaceTexture` enum; `enable primitive_index;`; cooperative load/store; optional bind group layouts; `WriteOnly` mapped buffers; Metal fence crash in the macOS sandbox fixed (29.0.2) — relevant for a notarized/sandboxed app.
- 30.0.1: Metal color-space constants resolved dynamically; Vulkan fence validation fix.

### Compute, WGSL, features/limits — VERIFIED (docs.rs `wgpu::Features`, 30.0.1)

- Compute shaders in WGSL are core. Relevant features: `TIMESTAMP_QUERY` (+`_INSIDE_PASSES`), `SUBGROUP` (native only), `SHADER_F16` (`enable f16;`, Vulkan/Metal/DX12/WebGPU), `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` (needed for read/write storage on more formats, native only), `MAPPABLE_PRIMARY_BUFFERS` (native; "only helps on systems with shared CPU/GPU memory and can hurt performance elsewhere"), `IMMEDIATES` (replaces push constants), `EXPERIMENTAL_COOPERATIVE_MATRIX` (8×8 f32, Metal/Vulkan), experimental mesh shaders and ray queries.
- Requesting an unsupported feature **panics** — always intersect with `adapter.features()` and build a capability report (`jenga doctor`).
- `Limits`: use `adapter.limits()` and clamp; terrain compute mostly needs `max_storage_buffer_binding_size` (default 128 MiB — a 4096² f32 heightfield is 64 MiB, 8192² is 256 MiB → tile) and `max_compute_workgroup*`.

### Apple Silicon unified memory & readback — VERIFIED (docs.rs `Buffer`) + REPORTED

- Readback flow: `COPY_DST|MAP_READ` staging buffer → `encoder.copy_buffer_to_buffer` → `submit` → `slice.map_async(MapMode::Read, cb)` → `device.poll(...)` (callback runs on the polling thread and must be short) → `get_mapped_range()` → drop views → `unmap()`. Helpers: `wgpu::util::DownloadBuffer::read_buffer`, and `map_buffer_on_submit` on encoders/passes (schedules the map at submit, avoiding a separate `map_async`).
- Docs: mapping "is not free"; may be shared memory or require a transfer depending on device. On Apple Silicon, `MAPPABLE_PRIMARY_BUFFERS` lets a storage buffer also be mapped directly (UMA fast path) — but the Metal backend's handling of hazards means it is still not free; benchmark (REPORTED/UNVERIFIED speedup).
- Readback batching strategy: one staging ring (2–3 buffers), batch many tiles per `submit`, map once per batch; never map per tile.

### Shader hot-reload — REPORTED

No built-in. Pattern: `notify` 8.2.0 (CC0-1.0) watches `shaders/*.wgsl`, re-create the pipeline on change; validate with `naga` first so a bad shader never panics the app (`naga::front::wgsl::parse_str` + `naga::valid::Validator`). Ship shaders embedded via `include_str!` in release.

### wgpu vs direct APIs — VERIFIED versions, REPORTED assessment

| Crate | Version | License | Note |
|---|---|---|---|
| `vulkano` | 0.35.2 (2025-08-16) | MIT OR Apache-2.0 | Vulkan only; useless on macOS without MoltenVK. |
| `ash` | 0.38.0 (2024-04-01) | MIT OR Apache-2.0 | Raw Vulkan; no release in 2.5 years. |
| `metal` | 0.33.0 (2025-12-17) | MIT OR Apache-2.0 | macOS-only. |

wgpu is the only option that covers Metal + DX12 + Vulkan with one WGSL codebase. Direct Metal is only justified later for Apple-only features (e.g. Metal 4 tensors). **Recommendation: wgpu.**

### Renderer crates — VERIFIED versions

| Crate | Version | License | Status |
|---|---|---|---|
| `rend3` | 0.3.0 (2022-02-12) | MIT/Apache/Zlib | **Archived 2025-06-07**, "MAINTENANCE MODE". Do not use. |
| `bevy_render` | 0.20.0 (2026-10-08) | MIT OR Apache-2.0 | Active but entangled with Bevy ECS/app; standalone use outside a Bevy `App` is impractical (REPORTED). |
| `three-d` | 0.19.0 (2026-04-17) | MIT | Active, OpenGL/WebGL-based (REPORTED), not wgpu. |
| `kiss3d` | 0.47.0 (2026-10-04) | BSD-3-Clause | Active, OpenGL-based (REPORTED); toy-level. |

**Recommendation:** no engine. Write a small wgpu renderer crate for the viewport (heightfield LOD mesh, PBR-lite, splat materials). For terrain the renderer is dominated by one custom pipeline anyway; engines add upgrade friction (bevy_render tracks wgpu majors with lag).

---

## 3. Frontend

### Versions — VERIFIED (npm)

| Package | Version | License |
|---|---|---|
| `react` / `react-dom` | 19.3.1 | MIT |
| `svelte` | 5.57.2 | MIT |
| `solid-js` | 1.9.17 | MIT |
| `vue` | 3.5.44 | MIT |
| `@xyflow/react` (React Flow) | **12.12.0** (2026-09-24) | MIT |
| `@xyflow/svelte` (Svelte Flow) | 1.7.0 | MIT |
| `rete` | 2.0.6 (2025-06-30) / `rete-react-plugin` 2.1.2 | MIT |
| `baklavajs` | 2.8.1 (2025-11-02) | MIT |
| `@comfyorg/litegraph` | 0.17.2 (2025-08-06) | MIT |
| `litegraph.js` | 0.7.18 (2024-01-08, stale) | MIT |
| `three` / `@types/three` | 0.186.1 / 0.186.0 | MIT |
| `@babylonjs/core` | 9.31.0 | Apache-2.0 |
| `tailwindcss` / `@tailwindcss/vite` | **4.3.3** | MIT |
| `vite` | **8.3.4** (Rolldown-only bundler since 8.0, 2026-03-12) | MIT |
| `typescript` | **7.0.2** (Go-native compiler, stable ~July 2026; programmatic API unstable until 7.1 — REPORTED) | Apache-2.0 |
| `lucide-react` / `lucide` | 1.59.0 | **ISC** |
| `lucide-svelte` | 1.0.1 (2026-03-23, lags) | ISC |
| `radix-ui` (unified) | 1.8.1 | MIT |
| `shadcn` CLI | 4.22.1 | MIT |
| `zustand` | 5.0.15 | MIT |
| `@fontsource-variable/host-grotesk` | 5.3.0 | **OFL-1.1** |
| `vitest` | 5.0.5 | MIT |
| `playwright` | 1.64.1 | Apache-2.0 |
| `@biomejs/biome` | 2.5.15 | MIT OR Apache-2.0 |
| `@modelcontextprotocol/sdk` (TS, for test clients) | 1.32.1 | MIT |

### Framework choice

Tauri is framework-agnostic; the deciding factor is the **node editor**. React Flow 12.12 is the most actively maintained, best documented, MIT, with custom node/edge rendering and virtualization; Svelte Flow is the same team but a smaller ecosystem. Rete.js is more "engine-like" (its own graph model) and slower-moving (core 2.0.6 from mid-2025). baklavajs is Vue-only. litegraph.js is dead upstream; the ComfyUI fork (`@comfyorg/litegraph`) is Comfy-specific.

**Recommendation: React 19 + React Flow 12 + Zustand + Tailwind 4 + shadcn/ui (Radix) + lucide-react.** Vite 8, TypeScript 7, Biome, Vitest, Playwright for e2e against `tauri dev`. shadcn/ui's Tailwind v4 support is REPORTED (it shipped v4/React 19 support in early 2025 and the CLI is at 4.x); not re-verified.

Host Grotesk: listed on Google Fonts (page title VERIFIED), OFL-1.1 (Fontsource VERIFIED; copyright "The Host Grotesk Project Authors", built on Poppins). Self-host via `@fontsource-variable/host-grotesk` — OFL permits bundling in an MIT app (keep the OFL text in `THIRD_PARTY_LICENSES`). Adobe Fonts' boilerplate about paid licensing is marketing noise, not the font's license.

### 3D in the WebView

three.js r186: `WebGPURenderer` with automatic **WebGL 2 fallback** (official manual, VERIFIED), `forceWebGL: true` for testing, TSL → WGSL/GLSL; raw GLSL `ShaderMaterial` does not run on the WebGPU backend (REPORTED). Babylon 9.31 is Apache-2.0 and heavier. For a terrain viewport, three.js with `WebGPURenderer` + TSL gives one codebase that uses WebGPU on macOS 26 and falls back to WebGL 2 on Linux/Windows automatically — that fallback is exactly why it's attractive here.

---

## 4. Core Rust crates

All versions/licenses VERIFIED from crates.io unless flagged.

### Noise

| Crate | Version | License | Last release | Assessment |
|---|---|---|---|---|
| `noise` | 0.9.0 | Apache-2.0/MIT | 2024-03-23 | Pure Rust, scalar, trait-based `NoiseFn` combinators; quality good, slow (no SIMD); 2.5 years stale. |
| `fastnoise-lite` | 1.1.1 | MIT | 2024-03-05 | Port of FastNoiseLite; scalar; simple; stale but complete. |
| `fastnoise2` / `fastnoise2-sys` | 0.4.0 / 0.2.0 | MIT (FastNoise2 upstream also MIT, VERIFIED via GitHub API; upstream pushed 2026-10-18) | 2026-01-21 | **Fused SIMD node graph** (SSE2/4.1/AVX2/AVX512/NEON/WASM via FastSIMD, runtime dispatch). Builds C++17 via CMake (`build-from-source`) or links a prebuilt via `FASTNOISE2_LIB_DIR`. API: `SafeNode::from_encoded_node_tree(...)`, `gen_uniform_grid_2d(buf, x, y, w, h, freq, seed)`. Fastest option by far (REPORTED: 10–100× over scalar), but adds a C++ toolchain to every build/CI matrix. |
| `simdnoise` | 3.1.6 | Apache-2.0/MIT | 2020-05-30 | Dead. |
| `bracket-noise` | 0.8.7 | MIT | 2022-10-04 | FastNoise port, scalar, stale. |
| `libnoise` | 1.2.0 | MIT | 2025-04-29 | Pure Rust, generic-dimension, chainable adapters, has some SIMD-friendly layout (REPORTED); modest activity. |

**Recommendation:** Two tiers. (1) **GPU-first:** implement Perlin/Simplex/Worley/FBM/ridged/domain-warp in **WGSL compute** — this is the primary path for the viewport and the main reason to have wgpu. (2) **CPU fallback / headless CLI:** `fastnoise2` behind a feature flag (`noise-fastnoise2`) with `libnoise` or `fastnoise-lite` as the no-C++ default. Keep both behind one `NoiseSource` trait in `jd-terrain`. Do not take `noise` 0.9 as a hard dependency — it's a maintenance risk.

### Math / parallelism / SIMD

- `glam` 0.34.1 (MIT OR Apache-2.0, 2026-10-06) — SIMD-backed Vec/Mat, `bytemuck` support, the de-facto wgpu companion. **Use.**
- `nalgebra` 0.35.0 (Apache-2.0) — only if `parry3d` (0.31.1, Apache-2.0) is adopted for collision/BVH queries. Avoid two math stacks; `parry` is heavyweight for terrain.
- `rayon` 1.12.0 (MIT OR Apache-2.0, 2026-04-14) — tile-level data parallelism. **Use.**
- SIMD: `std::simd` still **nightly-only** (Shnatsel, "State of SIMD in Rust 2026", 2025-09-25: "nightly-only, occasional breaking changes", weak float functions). Stable options: `wide` 1.7.1 (Zlib/Apache/MIT; fixed width, no runtime multiversioning), `pulp` 0.22.3 (MIT; verbose multiversioning, built for faer), **`fearless_simd` 1.1.0** (2026-10-06, linebender; "most complete all-in-one", runtime multiversioning, least `unsafe`; no trig yet; MSRV 1.89). `multiversion` crate for auto-vectorized loops.
  **Recommendation:** rely on auto-vectorization + `rayon` first; add `fearless_simd` only for hot kernels (erosion inner loops) after profiling. Don't hand-SIMD noise on CPU — that's what `fastnoise2` or the GPU is for.

### Images

- `image` 0.25.10 (MIT OR Apache-2.0) — PNG/JPEG/TIFF/EXR via features; 16-bit PNG for heightmaps.
- `exr` 1.74.2 (**BSD-3-Clause**) — OpenEXR 32-bit float heightfields/splat masks; BSD-3 is MIT-compatible (attribution).
- `tiff` 0.11.3 (MIT) — 32-bit float GeoTIFF-style export (no geo tags; add via a small writer if needed).
- `half` 2.7.1 — f16 for compact exports.

### Mesh processing / QEM decimation

| Crate | Version | License | Notes |
|---|---|---|---|
| `meshopt` | **0.7.0** (2026-10-17) | MIT OR Apache-2.0 (upstream meshoptimizer: MIT, VERIFIED GitHub API) | Bindings to zeux/meshoptimizer: `simplify` module (QEM-based simplifier with attribute/border options — meshoptimizer's documented algorithm, REPORTED for the exact Rust fn names), `optimize` (vertex cache/overdraw/fetch), `clusterize`, `clusterlod`, `encoding`, `remap`, `stripify`. C++ build via `cc`. **Best QEM option, battle-tested.** |
| `baby_shark` | 0.3.12 (2025-09-14; repo pushed 2025-10-05, 98 stars) | MIT | Pure Rust: `IncrementalDecimator` edge-collapse with `QuadricError` type (so QEM, VERIFIED by type name), criteria `ConstantErrorDecimationCriteria`/`BoundingSphereDecimationCriteria`; isotropic remeshing, booleans, voxel remeshing/offset, STL/OBJ/PLY. One maintainer. |
| `halfedge` | 0.3.0 (2026-09-08) | ? (crates.io license not fetched) | New half-edge library advertising QEM simplification; too new to depend on. UNVERIFIED quality. |
| `tri-mesh` | 0.6.1 (2024-09-02) | MIT | Half-edge mesh; stale. |
| `parry3d` | 0.31.1 | Apache-2.0 | Collision/BVH, not decimation. |
| `mesh-simplifier` | — | — | **Does not exist on crates.io** (could not verify). |

**Recommendation:** `meshopt` for decimation/LOD/vertex-cache optimization of exported terrain meshes; `baby_shark` only if pure-Rust is required or remeshing is needed. Heightfield → mesh is a grid triangulation you write yourself; QEM is only for export-time LOD.

### Memory mapping / serialization / schemas

- `memmap2` 0.9.11 (MIT OR Apache-2.0) — tile cache files.
- `serde` 1.0.229, `serde_json` 1.0.154, `rmp-serde` 1.3.1 (MIT), `postcard` 1.1.3 (MIT OR Apache-2.0), `rkyv` 0.8.18 (MIT), `ron` 0.12.2, `toml` 1.1.8.
- **`bincode`: do not use.** The crates.io `3.0.0` (2025-12-16) is a **tombstone release** — its README says the project is unmaintained (maintainer harassment incident), the `lib.rs` deliberately fails to compile, the GitHub repo is archived (2025-08-15) and moved to sourcehut. Suggested successors: `wincode` 0.6.3 (Apache-2.0, 2026-10-13, bincode-compatible wire format — VERIFIED crates.io), `postcard`, `rkyv`. (VERIFIED on docs.rs/crate/bincode/3.0.0.)
- **Recommendation:** `postcard` for compact internal cache blobs (stable wire format, `no_std`, serde), `serde_json` for project documents (human-diffable, git-friendly), `rkyv` only if zero-copy tile loading proves necessary.
- `schemars` **1.2.3** (MIT) — JSON Schema from Rust types; rmcp 3.5.1 depends on `schemars ^1.0` (VERIFIED crates.io deps), so one `schemars` 1.x serves both the API crate and MCP tool schemas. `jsonschema` 0.59.0 (MIT) for validating untrusted project files / MCP inputs at trust boundaries.

**Versioned document format pattern (REPORTED best practice):**
```rust
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ProjectFile { pub schema_version: u32, #[serde(flatten)] pub body: ProjectV3 }
```
Read `schema_version` first via `serde_json::Value`, run `migrate_v1_to_v2`, `..._v2_to_v3` as pure functions over `Value` (never over typed structs — old structs rot), then deserialize. Emit the schema (`schemars::schema_for!`) into `schemas/project.v3.json` in CI and snapshot it with `insta` so schema drift is a reviewed diff. Store `schema_version` + app version + a `blake3` of the graph for cache invalidation.

### CLI / logging / errors / async

- `clap` 4.6.7 (MIT OR Apache-2.0) derive; `clap_complete` for shells.
- `tracing` 0.1.44, `tracing-subscriber` 0.3.23 (`json` feature for structured logs; `EnvFilter`), `tracing-appender` 0.2.5 (rolling files, non-blocking). Profiling: `tracing-tracy` 0.12.0 (MIT/Apache-2.0; needs a Tracy GUI of matching protocol version — REPORTED), `puffin` 0.20.0 (in-app frame profiler, egui/http viewer), `tracing-chrome` 0.7.2 (Perfetto traces). Use `tracing` spans as the single instrumentation API and swap layers.
- Errors: `thiserror` 2.0.21 for library crates, `anyhow` 1.0.104 in binaries, `miette` 7.6.0 (**Apache-2.0**) for pretty CLI diagnostics (optional; its license is fine for MIT). **Stable error codes:** define `pub enum ErrorCode { JD0001_GraphCycle, ... }` with `#[non_exhaustive]`, expose `code()` on every public error, serialize as `{ code, message, details }` — the same struct is the MCP tool error payload and the Tauri command error. Never leak `anyhow` across the API crate boundary.
- Async: `tokio` 1.53.3 (MIT) — required by Tauri, rmcp, and HTTP transport. CPU work stays on `rayon`; bridge with `tokio::task::spawn_blocking` or a oneshot.

### Jobs / cancellation — REPORTED patterns

- `tokio-util` 0.7.20 `CancellationToken` for the async side; pass a cloned token into each job.
- Inside `rayon` kernels, check an `Arc<AtomicBool>` (or `token.is_cancelled()`) every N rows/tiles and return `Err(Cancelled)`; `rayon` has no preemption, so granularity = check interval.
- GPU side: cancellation = don't submit the next batch; in-flight submissions complete (bounded by batch size — keep batches ≤ ~16 ms of GPU time).
- Progress: `tokio::sync::watch` or `mpsc` → Tauri `Channel`, CLI `indicatif` 0.18.6, MCP `notify_progress`. One `JobProgress { job_id, fraction, stage, eta }` struct feeds all three.

### Content-addressed cache

- `blake3` 1.8.7 (CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception) — hash node params + upstream hashes → cache key.
- Index store: `redb` **4.4.0** (MIT OR Apache-2.0, 2026-10-13, pure Rust, ACID, single file) vs `rusqlite` 0.40.2 (MIT, bundled SQLite, battle-tested) vs `sled` 0.34.7 (last release 2024-10-11; 1.0 perpetually alpha — avoid). `cacache` 13.1.0 (Apache-2.0, 2024-11-26) is a ready-made content-addressed store but stale.
- **Recommendation:** `redb` for the cache index (key → blob path/offset, size, last-use) + content blobs as individual `postcard`/`zstd` files or one append-only arena with `memmap2`. SQLite is the fallback if we need ad-hoc queries.

### Compression

- `zstd` 0.14.3 (**BSD-3-Clause**, binds libzstd via `zstd-sys`, C build) — best ratio for on-disk tiles; levels 1–3 for cache.
- `lz4_flex` 0.14.0 (MIT, pure Rust) — fastest for in-memory/IPC frames. Use `lz4_flex` for IPC/cache hot path, `zstd` for exports/archives.

### Testing / quality

- `proptest` 1.12.0, `insta` 1.49.0 (Apache-2.0; snapshot JSON schemas, CLI output, small heightfield checksums), `criterion` 0.8.2, `divan` 0.1.21 (lighter, 2025-04-10), `cargo-fuzz` 0.13.2 (nightly, libFuzzer — fuzz project-file parser and graph loader), `cargo-nextest` 0.9.148, `cargo-deny` 0.20.2, `cargo-audit` 0.22.2, `cargo-about` 0.9.2 (generates THIRD_PARTY_LICENSES — required because of BSD-3 (exr, zstd), ISC (lucide), OFL (font), Apache-2.0 (nalgebra/parry/miette/insta)).

`deny.toml` allow-list for an MIT app: `MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, CC0-1.0, Unicode-3.0, MPL-2.0 (if any), OFL-1.1 (fonts)`; deny `GPL-*`, `AGPL-*`, `LGPL-*` unless explicitly reviewed (WebKitGTK is linked by the system, not vendored, so it does not enter the cargo graph).

---

## 5. MCP in Rust

### rmcp — VERIFIED

- `rmcp` **3.5.1** (2026-10-05), license **Apache-2.0** (crates.io), MSRV 1.88, repo `modelcontextprotocol/rust-sdk` (4,029 stars, pushed 2026-10-19). Implements MCP **2026-07-28** and stays compatible with 2025-11-25 and earlier (README).
- Features: `server` (default), `client`, `macros` (default: `#[tool]`, `#[prompt]`), `schemars` (opt-in, JSON Schema for tools; depends on `schemars ^1.0`), `auth`, `auth-enterprise-managed`, `elicitation`; transports `transport-io` (server stdio), `transport-child-process` (client stdio), `transport-async-rw`, `transport-streamable-http-server` (`StreamableHttpService`, axum/hyper), `transport-streamable-http-client[-reqwest]`; TLS via rustls by default.
- Tool definition: `#[tool_router]` on an `impl` block + `#[tool(description = "...")]` per method; `Parameters<T>` where `T: Deserialize + JsonSchema` generates `inputSchema` from field names/types/doc comments; `#[tool_handler]` (or `#[tool_router(server_handler)]` shortcut) wires `ServerHandler`. Prompts: `#[prompt_router]`/`#[prompt]`/`#[prompt_handler]`. Resources: `ServerCapabilities::builder().enable_resources()` + `list_resources`/`read_resource` (`ResourceContents::text|blob`) + `list_resource_templates`.
- Progress: `context.peer.notify_progress(ProgressNotificationParam::new(token, value).with_total(..).with_message(..))`; token comes from the request `_meta.progressToken` (REPORTED — README doesn't show extraction).
- Cancellation: `notify_cancelled(CancelledNotificationParam)` either side; server handles `on_cancelled(params.request_id)`. Tasks (SEP-1686 → now the `io.modelcontextprotocol/tasks` extension) are supported per docs search (REPORTED); exact 3.5 API UNVERIFIED.
- Serve: `let s = Server.serve(stdio()).await?; s.waiting().await?;`

### MCP spec — VERIFIED (modelcontextprotocol.io changelog)

Current revision **2026-07-28** (supersedes 2025-11-25). Major changes: protocol is **stateless** — no `initialize` handshake, no `Mcp-Session-Id`; every request carries `_meta` protocol version + client capabilities; new mandatory `server/discover`; `subscriptions/listen` replaces GET/SSE + resource subscribe; `ping`, `logging/setLevel`, `notifications/roots/list_changed` removed; tasks moved to an extension (`tasks/get` polling, `tasks/update`); **Multi Round-Trip Requests** (`resultType: "input_required"`) replace server-initiated `roots/list`/`sampling`/`elicitation`; all results need `resultType`; SSE resumability removed; list results need `ttlMs`/`cacheScope`; tool schemas may use any JSON Schema 2020-12 keywords. **Deprecated: Roots, Sampling, Logging** (12-month window) — "pass directories via tool parameters, resource URIs, or server configuration instead of Roots; log to stderr".

Implication: design Jenga Dunia's MCP tools as **stateless with explicit handles** (`project_id`/`job_id` as tool args), implement `server/discover`, do not build on Roots — accept an allow-listed workspace directory via server config / CLI flag (`jenga-mcp --workspace <dir>`), and log to stderr.

### Client registration — VERIFIED (Claude Code docs)

```
claude mcp add --transport stdio jenga -- jenga-mcp --workspace ./worlds
claude mcp add --transport http jenga http://127.0.0.1:7777/mcp
```
Scopes: local (`~/.claude.json`), project (`.mcp.json`, approval required), user. `.mcp.json`:
```json
{ "mcpServers": { "jenga": { "command": "jenga-mcp", "args": ["--workspace", "${JENGA_WORKSPACE:-./worlds}"] } } }
```
Env expansion `${VAR}` / `${VAR:-default}`; output capped at 25k tokens by default (`MAX_MCP_OUTPUT_TOKENS`), HTTP responses > 16 MiB fail → **return heightmaps as file paths/resource URIs, not inline base64.** Other clients (Claude Desktop, Cursor, VS Code) use the same `mcpServers` JSON shape (REPORTED).

### Security guidance

- stdio server: the client controls the process; restrict file access to the configured workspace root; canonicalize + reject `..`; refuse symlinks escaping the root.
- HTTP server: bind `127.0.0.1` only, require the spec's `Mcp-Method`/`Mcp-Name` headers (handled by rmcp), add a bearer token minted at launch; never expose on 0.0.0.0 without `auth`.
- Validate every tool input with `jsonschema` against the same schema we publish; map errors to stable codes.

### Alternatives — VERIFIED versions

`mcp-sdk` 0.0.3 (2025-01-20, Apache-2.0) and `mcpr` 0.2.3 (2025-03-16, MIT) — both dormant and pre-date three spec revisions. **Use `rmcp`.**

---

## 6. GitHub Actions CI

### Runner labels — VERIFIED (actions/runner-images README, 2026-10-19)

| OS | Labels | Arch | Status |
|---|---|---|---|
| macOS 26 | `macos-latest`, `macos-26`, `macos-26-xlarge` | arm64 | GA (since 2026-02-26); `macos-latest` migrated 15→26 from 2026-06-15 |
| macOS 26 Intel | `macos-latest-large`, `macos-26-intel`, `macos-26-large` | x64 | GA |
| macOS 15 | `macos-15`, `macos-15-xlarge` / `macos-15-intel`, `macos-15-large` | arm64 / x64 | GA |
| macOS 14 | `macos-14*` | — | **Deprecated** (retirement 2026) |
| Xcode 27 | `xcode-27`, `xcode-27-xlarge` | arm64 | Preview |
| Ubuntu 26.04 | `ubuntu-26.04`, `ubuntu-26.04-arm` | x64 / arm64 | Available (preview-ish); third-party blog says `ubuntu-latest` → 26.04 rolls out 2026-10-19..11-19 (REPORTED) |
| Ubuntu 24.04 | `ubuntu-latest`, `ubuntu-24.04`, `ubuntu-24.04-arm` | x64 / arm64 | GA |
| Ubuntu 22.04 | `ubuntu-22.04`, `ubuntu-22.04-arm` | | GA |
| Windows Server 2025 | `windows-latest`, `windows-2025`, `windows-2025-vs2026` | x64 | GA; VS 2026 image since 2026-06-15 |
| Windows Server 2022 | `windows-2022` | x64 | GA (VS 2022) |
| Windows 11 ARM | `windows-11-arm`, `windows-11-vs2026-arm` | arm64 | GA |

Pin explicit versions (`macos-26`, `ubuntu-24.04`, `windows-2025`) — `*-latest` moves under you. Build Linux on **24.04** (WebKitGTK 4.1 ABI widely deployed); test on 26.04 in a non-blocking job.

### Actions — VERIFIED (GitHub releases API)

| Action | Latest | Purpose |
|---|---|---|
| `tauri-apps/tauri-action` | **action-v1.0.0** (2026-06-29); v0.6.x line before | build + attach bundles + `latest.json` |
| `Swatinem/rust-cache` | v2.9.2 (2026-08-06) | cargo cache |
| `dtolnay/rust-toolchain` | `@stable` / `@1.98.1` (tag `v1`, moving) | toolchain |
| `taiki-e/install-action` | v2.87.29 (2026-10-16) | nextest, cargo-deny, cargo-about binaries |
| `EmbarkStudios/cargo-deny-action` | v2.1.1 (2026-07-13) | license/advisory gate |
| `googleapis/release-please-action` | v5.0.0 (2026-04-22) | conventional-commit releases |

### Workflow sketch

```yaml
strategy:
  matrix:
    include:
      - { os: macos-26,     target: aarch64-apple-darwin,     args: "--target aarch64-apple-darwin" }
      - { os: macos-26,     target: x86_64-apple-darwin,      args: "--target x86_64-apple-darwin" }   # or universal
      - { os: ubuntu-24.04, target: x86_64-unknown-linux-gnu, args: "" }
      - { os: windows-2025, target: x86_64-pc-windows-msvc,   args: "" }
steps:
  - uses: actions/checkout@v5
  - uses: dtolnay/rust-toolchain@stable
    with: { targets: ${{ matrix.target }} }
  - uses: Swatinem/rust-cache@v2
    with: { workspaces: ". -> target" }
  - if: runner.os == 'Linux'
    run: sudo apt-get update && sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
  - uses: pnpm/action-setup@v4
  - run: pnpm install --frozen-lockfile
  - uses: tauri-apps/tauri-action@v1
    env:
      GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
      APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
      APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
      APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
      APPLE_ID: ${{ secrets.APPLE_ID }}
      APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
      APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
      KEYCHAIN_PASSWORD: ${{ secrets.KEYCHAIN_PASSWORD }}
    with: { tagName: v__VERSION__, releaseName: "Jenga Dunia v__VERSION__", args: ${{ matrix.args }} }
```

Separate PR workflow: `cargo nextest run --workspace`, `cargo clippy -D warnings`, `cargo deny check`, `pnpm biome ci`, `pnpm vitest`, plus a **GPU-less** CI mode (`JD_GPU=noop` → wgpu `noop` backend or CPU fallback) since hosted runners have no usable GPU for Metal/Vulkan (REPORTED: macOS runners run virtualized without Metal acceleration; Linux has llvmpipe/lavapipe only if installed — add `mesa-vulkan-drivers` for a software Vulkan smoke test).

Versioning: release-please (conventional commits → changelog + tag, bumps `Cargo.toml`/`tauri.conf.json`/`package.json` via `extra-files`). Dependabot supports `cargo` and `npm` ecosystems; Renovate also supports both plus grouped updates (REPORTED). Windows code signing (Authenticode/Azure Trusted Signing) is optional for an OSS v1 — document SmartScreen warnings instead (REPORTED).

---

## 7. Memory / performance under 4 GB

Budget framing (REPORTED arithmetic, not a measured claim): a WebView + Node-editor frontend typically idles at 150–400 MB; Rust side gets ~3 GB. A 4096² heightfield is 64 MiB as f32 / 32 MiB as u16 — fine. An 8192² is 256 MiB f32; with 4–6 intermediate buffers per node and a graph of 20 nodes, naïve full-res evaluation blows the budget. Hence:

- **Tiling:** process in tiles of **512² or 1024²** with a halo (apron) of 8–32 px for neighborhood ops (blur, erosion, flow). 1024² f32 = 4 MiB per buffer; a 16-tile rayon batch × 6 buffers ≈ 400 MB peak. Powers of two align with GPU workgroup sizes (8×8 / 16×16) and mip chains.
- **Storage:** keep **f32 in flight**, store **u16 (normalized, with per-tile min/max) or f16** at rest/in cache; keep f32 for final export of erosion results where quantization banding matters. Flow/sediment masks as u8.
- **Cache on disk, mmap in:** cache blobs per `(node_hash, tile_xy, lod)`; `memmap2` for zero-copy read; `lz4_flex` for cache compression (fast decompress), `zstd` for project archives.
- **Streaming evaluation:** evaluate the graph **per tile** (pull-based), release upstream tiles as soon as all consumers are done (refcount), and only materialize full-res results for nodes that are pinned/exported. Preview at LOD 1–2 (quarter res) continuously; full-res on demand.
- **GPU readback batching:** staging ring buffer of 2–3 × (tile bytes × batch); one `map_async` per batch; `map_buffer_on_submit` where possible; never per-tile.
- **Allocator:** `mimalloc` 0.1.52 (MIT, pure C via `cc`, works on all three OSes, lower fragmentation on many mid-size allocs — REPORTED) vs `tikv-jemallocator` 0.7.0 (MIT/Apache-2.0; not supported on MSVC targets — REPORTED). **Pick `mimalloc`** for the desktop app; keep it feature-gated so the MCP binary can stay on system malloc for minimal size.
- **Observability:** `tracing` spans with a custom layer that records RSS deltas per span (via `sysinfo` 0.39.6 (MIT) `Process::memory()` or `peak_alloc` 0.3.0 (MIT) as a global-allocator wrapper measuring peak heap). Expose `jenga doctor --memory` and a Tauri "resource monitor" panel. Add a `JD_MEM_BUDGET_MB` soft limit: the job scheduler refuses to start a new tile batch if `current + estimate > budget`, and spills to disk cache instead.
- WebView memory: avoid holding large `Float32Array`s in JS; stream preview tiles as compressed textures and keep only the LOD needed for the viewport.

---

## 8. Depth / AI inference in Rust (brief, for a later MCP "vision" feature)

| Crate | Version | License | Note |
|---|---|---|---|
| `ort` | **2.0.0-rc.15** (2026-10-19) — still RC; rc.14 binds ONNX Runtime 1.31; MSRV 1.92 | MIT OR Apache-2.0 (ONNX Runtime: MIT) | CoreML EP: `features = ["coreml"]`, `ep::CoreML::default().with_compute_units(ComputeUnits::CPUAndNeuralEngine).with_subgraphs(true).build().error_on_failure()`; unsupported ops silently fall back to CPU unless `error_on_failure`. rc.11 dropped Intel-mac binaries, min macOS 13.4. |
| `candle-core` | 0.11.0 (2026-06-26) | MIT OR Apache-2.0 | HF; Metal backend; needs model re-implemented in Rust (no ONNX import). |
| `burn` | 0.22.0 (2026-10-06) | MIT OR Apache-2.0 | Has ONNX → Burn import (`burn-import`, REPORTED), wgpu backend — attractive for consistency with our wgpu stack, but ViT-L import coverage UNVERIFIED. |
| `tract-onnx` | 0.23.9 (2026-10-15) | MIT OR Apache-2.0 | Pure-Rust ONNX, CPU only; slow for ViT-L. |

Models:
- **MoGe-2**: code MIT (microsoft/MoGe; DINOv2 parts Apache-2.0); weights `Ruicheng/moge-2-vit{s,b,l}[-normal]` tagged **MIT** on Hugging Face (VERIFIED via HF API). Official ONNX exports exist (`moge-2-vit*-normal-onnx`), opset 14, FP32, dynamic H/W + `num_tokens` (static variant 518×518/1800 tokens). The ONNX graph is **only `forward()`**: outputs affine point map, normal map, float mask, metric scale — focal/shift recovery and reprojection must be reimplemented in Rust (VERIFIED docs/onnx.md).
- **Depth Anything 3**: code Apache-2.0; weights **mixed**: `DA3-SMALL`, `DA3-BASE`, `DA3METRIC-LARGE`, `DA3MONO-LARGE` are Apache-2.0; `DA3-LARGE*`, `DA3-GIANT*`, `DA3NESTED*` are **CC BY-NC 4.0** (VERIFIED README). No official ONNX; community exports (`Heliosoph/da3-*-onnx`, `Jens-Duttke/...`) are fixed-shape (multiples of 14, 518 default) and output raw depth (REPORTED).

Feasibility: an `ort` + CoreML path for MoGe-2 ViT-S/B (35M/104M params) on Apple Silicon is realistic within the 4 GB budget; ViT-L FP32 (~1.3 GB weights) is borderline. CoreML may not cover all transformer ops → partial CPU fallback is likely; benchmark with `with_compute_units(All)` vs `CPUOnly`. Keep this in a separate optional crate/binary so `ort`'s native library download/linking never touches the core build. Risks: `ort` is perpetually RC (API churn each rc, e.g. CoreML option renames in rc.10); DA3 large weights are non-commercial.

---

## (a) Recommended dependency table

| Crate | Version | License (SPDX) | Purpose | Risk |
|---|---|---|---|---|
| `tauri` | 2.12.4 | Apache-2.0 OR MIT | desktop shell | Low; Tauri 3/CEF on horizon (watch) |
| `tauri-plugin-{fs,dialog,shell,log,store,updater,window-state,process,opener}` | see §1 | Apache-2.0 OR MIT | platform plugins | Low |
| `wgpu` + `naga` | 30.0.1 | MIT OR Apache-2.0 | GPU compute + viewport | Medium: breaking major every ~3 months; isolate |
| `raw-window-handle` | 0.6.2 | MIT OR Apache-2.0 OR Zlib | surface creation | Low (shared by tauri & wgpu) |
| `glam` | 0.34.1 | MIT OR Apache-2.0 | math | Low |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | GPU struct layout | Low |
| `rayon` | 1.12.0 | MIT OR Apache-2.0 | CPU parallelism | Low |
| `fearless_simd` (optional) | 1.1.0 | ? (linebender crates are Apache-2.0 OR MIT — UNVERIFIED for this crate) | hot CPU kernels | Medium: 1.0 is one month old |
| `fastnoise2` (optional feature) | 0.4.0 | MIT | fast CPU noise | Medium: C++/CMake in build; small user base |
| `libnoise` or `fastnoise-lite` | 1.2.0 / 1.1.1 | MIT | pure-Rust CPU noise fallback | Low/Medium (slow activity) |
| `image` | 0.25.10 | MIT OR Apache-2.0 | PNG/TIFF/EXR I/O | Low |
| `exr` | 1.74.2 | BSD-3-Clause | 32-bit float export | Low (attribution) |
| `tiff` | 0.11.3 | MIT | float TIFF | Low |
| `half` | 2.7.1 | MIT OR Apache-2.0 | f16 storage | Low |
| `meshopt` | 0.7.0 | MIT OR Apache-2.0 | QEM decimation / LOD / vertex cache | Low (C++ via cc) |
| `memmap2` | 0.9.11 | MIT OR Apache-2.0 | tile cache mmap | Low |
| `serde`, `serde_json` | 1.0.229 / 1.0.154 | MIT OR Apache-2.0 | documents, API | Low |
| `postcard` | 1.1.3 | MIT OR Apache-2.0 | cache blobs | Low |
| `schemars` | 1.2.3 | MIT | JSON Schema (API + MCP) | Low |
| `jsonschema` | 0.59.0 | MIT | validate inputs | Low |
| `clap` | 4.6.7 | MIT OR Apache-2.0 | CLI | Low |
| `tracing`, `tracing-subscriber`, `tracing-appender` | 0.1.44 / 0.3.23 / 0.2.5 | MIT | logging | Low |
| `tracing-tracy` / `puffin` (dev) | 0.12.0 / 0.20.0 | MIT/Apache-2.0 | profiling | Low |
| `thiserror`, `anyhow` | 2.0.21 / 1.0.104 | MIT OR Apache-2.0 | errors | Low |
| `miette` (cli only) | 7.6.0 | Apache-2.0 | diagnostics | Low |
| `tokio`, `tokio-util` | 1.53.3 / 0.7.20 | MIT | async, CancellationToken | Low |
| `blake3` | 1.8.7 | CC0-1.0 OR Apache-2.0 (…) | cache keys | Low |
| `redb` | 4.4.0 | MIT OR Apache-2.0 | cache index | Low/Medium (format changes across majors) |
| `zstd` | 0.14.3 | BSD-3-Clause | archive compression | Low (C) |
| `lz4_flex` | 0.14.0 | MIT | fast compression | Low |
| `rmcp` | 3.5.1 | Apache-2.0 | MCP server | Medium: spec churn (2026-07-28 was a big break) |
| `mimalloc` | 0.1.52 | MIT | allocator (app) | Low |
| `sysinfo` | 0.39.6 | MIT | resource monitor | Low |
| `peak_alloc` (dev/diag) | 0.3.0 | MIT | peak heap | Low |
| `notify` | 8.2.0 | CC0-1.0 | shader/project hot reload | Low |
| `proptest`, `insta`, `criterion`/`divan` (dev) | 1.12.0 / 1.49.0 / 0.8.2 / 0.1.21 | MIT/Apache | tests, benches | Low |
| `ort` (optional, separate crate) | 2.0.0-rc.15 | MIT OR Apache-2.0 | depth inference | High: RC API churn, native lib |
| **Avoid** `bincode` 3.0.0 | — | — | tombstone release | — |
| **Avoid** `rend3`, `sled`, `simdnoise`, `mcp-sdk`, `mcpr` | — | — | archived/dormant | — |

Frontend: `react` 19.3.1, `@xyflow/react` 12.12.0, `three` 0.186.1, `tailwindcss` 4.3.3, `vite` 8.3.4, `typescript` 7.0.2, `lucide-react` 1.59.0 (ISC), `radix-ui` 1.8.1 / `shadcn` 4.22.1, `zustand` 5.0.15, `@fontsource-variable/host-grotesk` 5.3.0 (OFL-1.1), `@tauri-apps/api` 2.12.4, `vitest` 5.0.5, `playwright` 1.64.1, `@biomejs/biome` 2.5.15.

---

## (b) Three viewport-hosting options compared

| Criterion | **A. Native wgpu surface in the Tauri window** (webview transparent on top) | **B. WebGPU in the WebView** (three.js WebGPURenderer) | **C. WebGL 2 in the WebView** (three.js, Rust streams geometry/textures) |
|---|---|---|---|
| macOS (Apple Silicon) | Works (FabianLars example; `HasWindowHandle` on `WebviewWindow` VERIFIED). Transparent WKWebView over Metal layer: stability UNVERIFIED (resize/fullscreen). | Works on macOS **26+** only (VERIFIED default-on); not on macOS 14/15 WKWebView. Dev-build caveat REPORTED. | Works everywhere. |
| Windows | WebView2 is a child HWND with its own compositor; transparent child webview issues (REPORTED #12450). Risky. | Chromium has WebGPU; WebView2 may need `--enable-unsafe-webgpu` via `additionalBrowserArgs` (REPORTED 2023; current default UNVERIFIED). | Works. |
| Linux | **Flickers** — GTK owns the draw loop (VERIFIED #9220, closed not-planned; still reported April 2026). Effectively unsupported. | **No WebGPU in WebKitGTK** (REPORTED, no counter-evidence). | Works (WebKitGTK WebGL2; perf varies with Mesa). |
| GPU compute ↔ render sharing | Best: terrain compute and render share one `wgpu::Device`; zero copies. | Compute must happen in JS/WGSL in the WebView **or** results cross IPC. Rust-side wgpu compute cannot share buffers with the WebView's GPU context. | Same as B: everything from Rust crosses IPC (geometry/textures). |
| IPC load | None for pixels; UI events only. | Heightfield tiles → WebView (u16 tiles, LOD) — tens of MB per full-res sync; manageable with LOD streaming. | Same as B. |
| UI integration (node editor overlays, gizmos) | Hard: two compositors; overlay = transparent webview; hit-testing/click-through is custom. | Trivial: one DOM. | Trivial. |
| Shader codebase | WGSL only (one place). | WGSL via TSL (duplicate of Rust-side compute shaders). | GLSL via TSL (duplicate). |
| Future-proof | Tauri 3 may change window internals; CEF runtime could change the picture. | Tauri 3 + CEF would make this uniform across OSes. | Stable, boring. |
| Risk | **High** cross-platform; medium even on macOS. | Medium (platform coverage), low (engineering). | Low. |

**Recommendation:** **Start with C, designed so B is a free upgrade.** Use three.js `WebGPURenderer` (which falls back to WebGL 2 automatically); write the viewport against TSL/three abstractions, not raw GLSL, so the same code uses WebGPU on macOS 26 and WebGL 2 elsewhere. Keep *all* terrain compute in Rust/wgpu (headless device, no surface) and stream **u16 heightfield tiles + splat/normal textures at LOD** over `tauri::ipc::Channel` (raw bytes), never full-res f32 unless requested. This gives one Rust compute path shared by GUI/CLI/MCP and a boring, portable viewport.

Treat **A** as a Phase-2 macOS-only "pro viewport" experiment, not the baseline — the Linux flicker issue is closed as won't-fix and Windows is unproven.

**Prototype test to confirm (1–2 days):**
1. Tauri 2.12 app, headless `wgpu` device in Rust generating a 4096² FBM heightfield in a WGSL compute shader, tiled 1024², read back via staging ring.
2. Stream tiles as u16 over `Channel<Vec<u8>>` to a three.js `WebGPURenderer` scene (CDLOD or simple quadtree mesh, displacement from a `DataTexture`). Measure: (i) end-to-end latency from "parameter change" to "pixels" at LOD 2 and LOD 0; (ii) MB/s IPC throughput on macOS **and** Windows (the ~200 ms/10 MB anecdote is the number to kill or confirm); (iii) peak RSS (target < 1.5 GB for this test); (iv) `navigator.gpu` presence on macOS 26 prod build, macOS 26 `tauri dev`, Windows 11 WebView2 default vs `--enable-unsafe-webgpu`, Ubuntu 24.04 WebKitGTK (expect absent → WebGL 2 fallback engaged).
3. Side experiment (macOS only): the FabianLars pattern — `create_surface(webview_window)` + transparent webview; check resize, fullscreen, external display, and sleep/wake for 30 minutes. If it is rock-solid and IPC in step 2 is a bottleneck, promote A for macOS behind a setting.

---

## (c) Recommended Cargo workspace layout

```
jenga-dunia/
├── Cargo.toml                  # [workspace], resolver = "3", shared [workspace.dependencies] + lints
├── deny.toml, about.toml, rust-toolchain.toml (stable, 1.98.1 pinned)
├── crates/
│   ├── jd-core/            # domain types: Heightfield<T>, Tile, TileGrid, LOD, units, ErrorCode, JobId,
│   │                       # ProgressEvent, cancellation handle, `schema_version` + migration traits. No I/O, no GPU.
│   ├── jd-graph/           # node-graph engine: NodeDef registry, typed ports, DAG validation (cycle/type errors
│   │                       # → stable codes), topological scheduling, per-tile pull evaluation, dirty tracking,
│   │                       # blake3 content hashing of (node params + upstream hashes). Backend-agnostic via
│   │                       # `trait Executor { fn run(&self, op: &Op, inputs: &[TileRef], ctx) -> Result<TileRef> }`.
│   ├── jd-gpu/             # the ONLY crate that depends on wgpu/naga: device/adapter selection, feature/limits
│   │                       # report, staging ring + batched readback, WGSL module registry (include_str! +
│   │                       # dev hot-reload via notify), compute kernels shared by terrain/simulation.
│   ├── jd-terrain/         # noise (WGSL + CPU fallback via NoiseSource trait: fastnoise2 feature / libnoise),
│   │                       # fbm/ridged/warp, terraces, slope/curvature/flow accumulation, combine/mask ops.
│   ├── jd-sim/             # simulation: hydraulic/thermal erosion, sediment, snow, rivers — tiled with halos,
│   │                       # deterministic seeds, GPU kernels in jd-gpu + CPU reference impls for tests.
│   ├── jd-materials/       # splat/weight maps, biome classification, color ramps, texture synthesis params,
│   │                       # material graph outputs (albedo/roughness/normal) as tiles.
│   ├── jd-geometry/        # heightfield → mesh (grid, quadtree/CDLOD chunks), normals/tangents, meshopt
│   │                       # decimation + LOD, mesh validation; no file formats here.
│   ├── jd-io/              # formats: project file (serde_json, schema_version, migrations), PNG16/TIFF32/EXR
│   │                       # heightmaps, OBJ/glTF/PLY mesh export, RAW/r16; tile cache (redb index + mmap blobs,
│   │                       # postcard + lz4/zstd); content-addressed store keyed by blake3.
│   ├── jd-jobs/            # job scheduler: tokio runtime + rayon pool bridge, CancellationToken, memory budget
│   │                       # (sysinfo/peak_alloc), progress fan-out (watch/mpsc), priority (preview > export).
│   ├── jd-api/             # the public, versioned surface shared by GUI/CLI/MCP: request/response structs
│   │                       # with serde + schemars, error envelope {code,message,details}, JSON Schema export
│   │                       # (`jd-api schema` → schemas/*.json, snapshot-tested with insta). Thin facade over
│   │                       # jd-graph/jd-jobs/jd-io. No tauri, no rmcp, no clap.
│   ├── jd-cli/             # bin `jenga`: clap 4 derive, subcommands build/export/doctor/schema/cache, tracing
│   │                       # to stderr/JSON, indicatif progress, miette diagnostics. Depends only on jd-api.
│   ├── jd-mcp/             # bin `jenga-mcp`: rmcp 3 server (stdio default, streamable-http opt-in on 127.0.0.1),
│   │                       # tools derived from jd-api types via schemars, resources = project/heightmap URIs,
│   │                       # progress + cancellation forwarding, --workspace root allow-list, stderr logging.
│   └── jd-vision/          # OPTIONAL, excluded from default-members: ort + CoreML, MoGe-2/DA3 ONNX wrappers,
│                           # post-processing (focal/shift recovery). Feature-gated into jd-mcp as `vision`.
├── apps/
│   └── desktop/            # Tauri app
│       ├── src-tauri/      # crate `jd-desktop`: tauri 2.12, commands = thin wrappers over jd-api, Channel-based
│       │                   # progress/tile streaming (ipc::Response raw bytes), capabilities/*.json, mimalloc,
│       │                   # tauri-plugin-{dialog,fs,log,store,updater,window-state,process,opener}.
│       └── src/            # React 19 + React Flow + three.js + Tailwind 4 + shadcn + lucide; TS types generated
│                           # from schemas/*.json (json-schema-to-typescript) so the frontend never hand-types API.
├── schemas/                # generated JSON Schemas (committed, snapshot-tested)
├── shaders/                # WGSL sources (include_str! by jd-gpu; hot-reload in dev)
└── .github/workflows/      # ci.yml (nextest, clippy, deny, biome, vitest), release.yml (tauri-action v1 matrix)
```

Dependency direction (enforced with `cargo deny` bans or a `cargo-modules`/`cargo-workspace-lints` check): `jd-core ← jd-graph ← {jd-terrain, jd-sim, jd-materials, jd-geometry} ← jd-io ← jd-jobs ← jd-api ← {jd-cli, jd-mcp, jd-desktop}`; `jd-gpu` is used only by terrain/sim/materials/geometry and is behind a `gpu` feature so `jd-mcp`/`jd-cli` can build a CPU-only variant for CI and servers.

Naming: crates `jd-*`, binaries `jenga` and `jenga-mcp`, bundle id `ai.bsa.jengadunia` (placeholder — choose before first signed build; it is baked into updater/keychain paths).

---

## Gaps / things that could not be verified

1. Whether current **WebView2 stable** exposes `navigator.gpu` by default or still needs `--enable-unsafe-webgpu` (only 2023 evidence).
2. **WebKitGTK WebGPU** status in 2026 (only a 2023 "not planned" statement; no newer source either way).
3. Stability of a **transparent WKWebView over a wgpu Metal surface** on macOS 26 (one archived example, no issue reports found either way).
4. The **dev-build-only WebGPU failure** on macOS 26 reported in Tauri #6381 (single user report).
5. **Tauri 3** scope/timeline and whether CEF becomes the default runtime (only alpha crates + a digest).
6. `fearless_simd` and `halfedge` licenses (not fetched); `meshopt` exact Rust fn names for simplification (module exists, fn list not read).
7. **Official IPC throughput numbers** for Tauri raw-bytes responses — none exist; anecdotal only.
8. Per-platform support matrix of official Tauri plugins (docs table unreadable via fetch).
9. `rmcp` 3.5 tasks-extension API and progress-token extraction details (README silent).
10. Depth Anything 3 ONNX exports are community-made; op coverage under CoreML UNVERIFIED.

## Sources (primary)

- crates.io API (`/api/v1/crates/<name>`), npm registry, `static.rust-lang.org/dist/channel-rust-stable.toml`, GitHub REST API (`/repos/<org>/<repo>`, `/releases/latest`), Hugging Face `/api/models/<id>`.
- docs.rs: `tauri` 2.12.4 (`WebviewWindow`, `Window`, `ipc::Response`, `ipc::InvokeResponseBody`), `wgpu` 30.0.1 (`Features`, `Buffer`), `rmcp` 3.5.1, `bincode` 3.0.0, `baby_shark` decimation, `meshopt` 0.7.0, `fastnoise2` 0.4.0.
- Tauri docs: calling-frontend, calling-rust, security/capabilities, develop/sidecar, distribute/sign/macos, plugin/updater, plugin/file-system, start/prerequisites, develop/plugins. Tauri issues #6381, #9220; repo `examples/`; `FabianLars/tauri-v2-wgpu` source.
- modelcontextprotocol.io 2026-07-28 changelog; `modelcontextprotocol/rust-sdk` README; code.claude.com/docs/en/mcp.
- github.com/actions/runner-images README; GitHub changelogs 2026-02-26 (macOS 26) and 2026-05-14 (image migrations).
- gfx-rs/wgpu releases; gpuweb Implementation-Status wiki; Apple Developer Forums thread 770862.
- Shnatsel, "State of SIMD in Rust 2026" (2026-09-25); vite.dev "Announcing Vite 8" (2026-03-12); TypeScript 7.0 coverage (InfoQ, VS Magazine).
- microsoft/MoGe README + docs/onnx.md; ByteDance-Seed/Depth-Anything-3 README; ort.pyke.io execution-providers; pykeio/ort releases.
- Fontsource Host Grotesk; fonts.google.com/specimen/Host+Grotesk.
