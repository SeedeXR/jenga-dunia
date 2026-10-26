# ADR-0017 — Studio (egui, engine + viewport) and Editor (Tauri, node graph): two processes over a local socket

**Status:** Accepted · 2026-10-26 (owner decision: "Option B: two processes", for robustness, speed, clean layers, error handling and logging) · supersedes ADR-0003's in-WebView viewport baseline once Phase 5 lands; refines ADR-0001's process model ("a single long-running daemon … revisit in Phase 5")

## Context

Today one Tauri process hosts the core and a React UI whose viewport is three.js inside the WebView (ADR-0003). It works, but every viewport feature lives in TypeScript against WebGPU-in-a-WebView, terrain data crosses the JS boundary, and the WebView's renderer, input and memory are outside our control. The owner chose to move the engine and the viewport into a native Rust process and keep the WebView only for what it is best at, the node graph.

Research (`docs/research/option-b-stack.md`, 2026-10-26): egui / eframe / egui-wgpu / egui_kittest 0.37.0 (MIT OR Apache-2.0) depend on wgpu `^30` — the same wgpu 30 that `jd-gpu` already uses, so no upgrade; `interprocess` 2.4.7 (0BSD OR Apache-2.0) gives Unix sockets and Windows named pipes; neither winit nor Tauri can place windows on Wayland.

## Decision

1. **Studio** (`apps/studio`, eframe + wgpu) owns the engine: the `jd-core` `Registry` and `Context`, the project, jobs, logs and the wgpu device. It renders the 3-D viewport into its own texture (own depth buffer, MSAA) shown in an egui panel, and hosts every panel except the graph — parameter editing (the inspector) lives here, next to the viewport.
2. **Editor** (the existing Tauri app, in `editor` role) shows the node graph and the layer stack. It holds no engine: its `op` command forwards the same JSON envelope (`jd_core::Request` → `Response`) to Studio. The frontend keeps calling `invoke("op")` unchanged.
3. **Protocol** (`crates/jd-ipc`): newline-delimited JSON frames over a local socket — a handshake (protocol version, role, a one-time token), calls correlated by id, replies, server-pushed events (`project_changed`, `selection`, `job`), heartbeats and an orderly goodbye. Every failure maps to a `JdError` code: `E_ENGINE_UNAVAILABLE` (new: the engine is gone or unreachable), `E_TIMEOUT`, `E_COMPAT` (protocol mismatch), `E_PERMISSION` (bad token), `E_SERIALIZE` (malformed frame). Spec: `docs/architecture/14-studio-editor-protocol.md`.
4. **Lifecycle:** Studio is the parent. It binds the socket (a file path in a 0700 per-user directory, the socket 0600; a random token), spawns the Editor with `JD_ENGINE_SOCKET` / `JD_ENGINE_TOKEN`, and supervises it (restart on crash). The Editor exits when the socket closes — the two always stop together, and a WebView crash never takes the engine's state with it.
5. **Logging and correlation:** both processes log through `tracing` with the op's `request_id` in the span; the project's `logs/ops.log` records every op with its client (`gui-editor`, `gui-studio`); crash reports as today.
6. **Migration without regressions:** the current single-process app stays the default until Studio reaches the parity matrix in `memory/plan-2026-10-26-option-b.md`; Studio + Editor ship behind a flag, phase by phase, each with its own tests.

## Alternatives rejected

- **Keep the viewport in the WebView** (status quo): leaves the hot path in TypeScript and WebGPU-in-WebView; not what the owner chose.
- **One process, egui with an embedded WebView for the graph (Option A):** a native child WebView inside a wgpu window is platform-specific and fragile (and impossible on Wayland), and a WebView crash takes the engine down.
- **Rewrite the graph editor in egui too:** the React Flow editor, its inspectors and its e2e suite work; rewriting them buys nothing for robustness.
- **Binary framing (length-prefixed, MessagePack):** large buffers no longer cross the boundary (the viewport is in Studio); NDJSON matches MCP's stdio transport and is debuggable with a text tap.
- **Tauri as the parent (sidecar):** tidier packaging today, but the WebView would own the engine's lifetime.

## Consequences

- Two UI toolkits: shared colour tokens and Host Grotesk (TTF, OFL) in egui; two test setups (Playwright for the Editor, `egui_kittest` with its wgpu renderer for Studio — Linux CI needs lavapipe).
- Anything crossing the boundary is a message: node ↔ viewport selection, keyboard shortcuts, focus, drag-and-drop.
- Two windows: on macOS two Dock icons unless the Editor runs as an accessory (which hides its menu bar); on Wayland the compositor places them.
- Packaging: the Tauri bundler provides icons, the `.jdproj` association and the macOS bundle today; with Studio as the entry point that moves to `cargo-packager` (decided in Phase 5).
- egui lays out every frame (~1–2 ms) and has open IME issues: text-heavy editing stays in the Editor's WebView.

## Evidence

`docs/research/option-b-stack.md` (versions, licences, APIs, Wayland, fonts, pitfalls — tagged); `docs/adr/0003-viewport-hosting.md`; `apps/desktop/src-tauri/src/lib.rs` (the three commands Option B forwards).
