# ADR-0003 — Viewport hosting strategy

Status: **Proposed** (accepted provisionally; closes after the Phase 1 prototype gate) · 2026-10-19

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

## Consequences
- Two shader dialects exist (WGSL compute in Rust, TSL in viewport) — kept minimal by baking expensive effects in Rust.
- Tauri 3 / CEF runtime (alpha) could make WebGPU uniform — tracked as a watch item.

## Evidence
`docs/research/stack.md` §1 "Known limitations", "Embedding a native wgpu surface", §(b).
