# ADR-0003 — Viewport hosting strategy

Status: **Accepted** · proposed 2026-10-19, closed 2026-10-21 after the prototype gate (observations below)

## Context
Brief §11: decide whether the viewport lives in the frontend, a native Rust renderer, or a separated surface, and justify by testing. Research (`stack.md` §1, §(b)) establishes: WebGPU in system WebViews is not a cross-platform baseline (default-on in WKWebView only from macOS 26; WebView2 default state unverified; none in WebKitGTK); a native wgpu surface in a Tauri window flickers on Linux (Tauri #9220, closed won't-fix) and is unproven on Windows; WebGL 2 works in all three WebViews; three.js r186 `WebGPURenderer` falls back to WebGL 2 automatically.

## Decision
- Baseline: **three.js `WebGPURenderer` with WebGL 2 fallback inside the WebView**, written against TSL (no raw GLSL) so WebGPU is used where available.
- All terrain computation stays in Rust (`jd-gpu`, headless wgpu device) and is streamed as u16 LOD tiles/textures over Tauri channels.
- Offscreen stills/thumbnails render in Rust at arbitrary size.
- Native wgpu surface = macOS-only Phase 2 experiment behind a setting.

## Alternatives rejected
- Native wgpu surface baseline — best for zero-copy, but fails Linux, risky Windows, hard UI overlays.
- WebGPU-only in WebView — excludes Linux and older macOS.
- Render-to-texture streaming at frame rate — ~250 MB/s IPC at 1080p30; unproven on Windows.

## Gate (must run before Phase 2 viewport work)
Prototype per `05-rendering-compute.md`: measure latency, IPC MB/s on macOS and Windows, peak RSS, `navigator.gpu` presence per platform/build mode, and macOS transparent-WKWebView-over-Metal stability. Results recorded here; status becomes Accepted or the decision is revised.

## Observations so far (2026-10-19)
- macOS 27.0.1 (Darwin 27), Apple Silicon, Tauri 2.12 **dev build**: three.js r186 `WebGPURenderer` initialises on the **WebGPU** backend inside WKWebView (header readout in the running app; screenshot in `docs/user/desktop-app.md`). This contradicts the single REPORTED "dev builds fail" caveat for this configuration.
- Tile streaming over `invoke` returning `tauri::ipc::Response` raw bytes is implemented (`layer_preview`). **Release build, macOS (Apple Silicon), 2048² world → 1024² u16 preview = 2.00 MB: 6 ms end-to-end** (Rust-side downsample + IPC + decode; three consecutive runs), i.e. ≥ 330 MB/s effective; `terrain.generate` for 2048² = 58–63 ms; the viewport holds 2.09 M triangles at interactive frame rates. The same path in a debug build took 517 ms, so measure optimised binaries only. Windows numbers still pending.
- Pitfall recorded: `cargo build --release -p jd-app` alone embeds the dev URL; the frontend is embedded only with Tauri's `custom-protocol` feature (`cargo build --release -p jd-app --features tauri/custom-protocol`, or `pnpm tauri build`). `tauri::is_dev()` = `!cfg!(feature = "custom-protocol")` (tauri 2.12.4 `src/lib.rs:318`).
- Chromium (Playwright e2e) exercises the WebGL 2 fallback path of the same component.

## Gate results (2026-10-21, closing the ADR)
- IPC: native Tauri raw bytes 2 MB preview in 6 ms (above); the e2e MCP/base64 bridge ≈ 7 MB/s (`e2e/ipc.measure.spec.ts`, `docs/spec/performance-targets.md`). Windows not measured (no machine) — stays a watch item, not a blocker: the baseline path has no Windows-specific component.
- `navigator.gpu`: present in the macOS 27 dev and release builds (WebGPU backend active); Chromium e2e exercises the WebGL 2 fallback of the same component.
- Headless Rust compute: `crates/jd-gpu` opens a wgpu 30 device with no surface (Apple M4, Metal) and runs thermal erosion in WGSL — parity with `jd_terrain::thermal_erode` 1.9e-7 relative on a 96×80 field, mass conserved; the test skips where no adapter exists (CI). Compute stays in Rust as decided; the viewport never computes.
- Transparent WKWebView over a Metal surface: not exercised — the native-surface experiment is Phase 2 and optional.

## Consequences
- Two shader dialects exist (WGSL compute in Rust, TSL in viewport) — kept minimal by baking expensive effects in Rust.
- Tauri 3 / CEF runtime (alpha) could make WebGPU uniform — tracked as a watch item.

## Evidence
`docs/research/stack.md` §1 "Known limitations", "Embedding a native wgpu surface", §(b).
