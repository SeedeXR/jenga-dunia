# Plan — Option B: Studio (egui, engine + viewport) + Editor (Tauri, graph) — owner, 2026-10-26

Owner: "Option B: two processes … I am doing this change to ensure everything is robust, fast and
ensure you ground yourself, no regressions, proper error handling, proper communication between
layers, proper logging and more." Decision: ADR-0017; protocol: docs/architecture/14-…; research:
docs/research/option-b-stack.md.

**Owner UI rule (2026-10-26):** "make the ui both for tauri and egui are similar no changes should
happen in the ui" — Studio reproduces today's desktop UI 1:1 (layout, sections, labels, field order,
buttons, lucide icons, tokens, Host Grotesk, readouts, log format); verified by side-by-side captures
against the Tauri build. **One deliberate change:** "the slides should be sleek and cool like the ones
in light room" — a Lightroom-style slider, identical in both UIs (spec below).

Rule for every phase: the current single-process app stays the default and keeps passing its full
gate (Playwright e2e, acceptance) until Phase 5 switches over; each phase lands with its own tests,
full gates, a commit, push, hosted CI.

| Phase | Scope | Status | Verification |
|---|---|---|---|
| P0 | ADR-0017, protocol spec, stack research, this plan with the parity matrix | done | docs + mermaid check |
| P1 | `crates/jd-ipc` (frames, local socket, client + server, heartbeats, errors, tracing spans); `E_ENGINE_UNAVAILABLE`; `apps/studio --headless` engine server (Registry + Context, `project_changed` events); the Tauri `op` / `ops_list` / `layer_preview` forward to Studio when `JD_ENGINE_SOCKET` is set (unchanged otherwise) | done | jd-ipc unit tests (spec list); studio integration test (real ops end to end, kill → E_ENGINE_UNAVAILABLE) |
| P2 | Studio window: eframe 0.37 + wgpu viewport (render-to-texture, depth, MSAA), height ramp + sun shading, orbit camera, layer select, log panel, project panel; spawns + supervises the Editor; shared device with jd-gpu; Host Grotesk + dark tokens; egui_kittest | done | kittest (panels, snapshot of the viewport), manual run on macOS |
| P2b | Chrome parity: header (brand mark, version, project readout, lucide toolbar, viewport readout), floating `jd-panel` cards with `▾` section headings and label/field rows, viewport frame, log card (monospace, local time, `op: ok`); Lightroom slider widget in egui **and** React (replacing `input[type=range]` everywhere) | todo | side-by-side captures; kittest + vitest for the slider (drag, fine drag, double-click reset, keyboard) |
| P3 | Viewport parity: PBR maps, sky (Preetham / HDRI), sea + lakes + waves, overlays (masks, heat), scatter instances, caves + x-ray + entrance markers, framed block + shoreline, horizon skirt, fog / rain / post, compass / ruler / hints, sculpt brush (raycast strokes, cursor), grid tool, road drawing, cave entrance placing, stills + flythrough | todo | kittest per feature; pixel checks against today's three.js captures |
| P4 | Panels to egui: inspector (param editing), Project, Reference image, Real world, Settings (language, LODs, cache), Tasks, Diagnostics, Brush, Grid, Sun, Scatter, Caves, Actions, Simulation layers, Materials, Noise, Export; selection sync node ↔ viewport; shortcuts + menus (muda) | todo | kittest per panel; Editor e2e against Studio |
| P5 | Switch: Studio + Editor the default; packaging (cargo-packager: icons, `.jdproj`, bundle); retire the WebView viewport; docs, tutorials, captures, acceptance | todo | full suite on the new default; acceptance 19/19 |

## Parity matrix (today's single-process app → where it goes)

Viewport (TerrainView.tsx, 1 430 lines): preview mesh + LOD · hypsometric ramp + slope rock · PBR
maps (albedo / normal / roughness / emissive) · overlay ramps (masks, class palettes, heat) · sky dome
(gradient / Preetham / HDRI) · sun light + shadows · sea plane (clarity, tint, Gerstner waves, surf
foam) · lakes · rain · fog · post (exposure, bloom, vignette) · framed block + rim + shoreline ·
horizon skirt · scatter instances (cones / rocks) · caves (walls, speleothems, markers, x-ray glow) ·
sculpt (raycast strokes, brush ring, radius / strength keys) · grid tool (cells, drag select) · road
drawing · cave entrance placing · orbit / fit / fullscreen · stills · flythrough (Catmull-Rom keys) →
**Studio**.
Chrome: compass strip, height ruler, hints, header readouts, layer strip (icon buttons) → **Studio**.
Panels (App.tsx sections): Project, Reference image, Layers (tree), Settings, Tasks, Diagnostics,
Brush, Grid, Sun, Scatter, Caves, Actions, Simulation layers, Materials, Terrain · Noise, Export →
**Studio**. Graph (GraphPanel), layer stack (LayerStack) → **Editor**. Node inspector → **Studio**
(owner's note: parameter editing next to the viewport).

## Lightroom-style slider (both UIs)
Spec and sources: `docs/research/research-log/2026-10-26-lightroom-slider.md` (Spectrum tokens,
VERIFIED: 2 px track, 16 px ring handle with 2 px border thickening when pressed, 4 px track gap,
label on top with value at the end, double-click handle / Esc resets, arrow keys step).

## Decisions
- Studio is the parent; the Editor exits when the socket closes (a WebView crash never loses engine state).
- NDJSON frames (MCP's transport), 64 MiB line cap; no bulk pixels cross (the viewport is in Studio).
- Packaging decided in P5 (cargo-packager with Studio as the entry point).

## Log
- P0 written.
- P1 done. `crates/jd-ipc` (interprocess 2.4.7: `split` is deprecated — the stream is shared in an
  `Arc`, `&Stream` reads and writes concurrently, on Windows too; named pipes have NO read
  timeouts, so the server relies on end of stream and only the client runs the 6 s heartbeat);
  `apps/studio` `jd-studio --headless` (ready line on stdout, serves until stdin closes);
  `ops.log` lines now carry `client`. Verified by hand on macOS: `jd-app` with `JD_ENGINE_SOCKET`
  ran 10 ops through Studio (logged `gui-editor`, viewport rendered the remote preview);
  closing Studio's stdin closed the Editor quietly; `kill -9` of Studio showed the "engine lost"
  dialog and the Editor exited. Frontend does not listen to `engine-event` yet (P2/P4: reload on
  `project_changed`, selection sync). Detached jobs emit no `project_changed` yet (P2 `job` events).
- P2 done (functional skeleton, not yet the desktop look — that is P2b). `jd-studio [project]`
  opens the window and launches/supervises `jd-app` (restart after a crash, ≤ 3 per 60 s; SIGKILL
  test: restarted in 5 ms, reconnected 20 ms later); `jd_gpu::Gpu::install` gives the kernels the
  window's device; `terrain.layers` op (cache index minus `node:` ports); server announces
  `project.open`/`close`, and the Editor turns those into its existing `project-opened` re-read
  (verified: Editor re-read 2 ms after Studio's open). kittest (wgpu) covers pixels, orbit, a
  write from another client refreshing Studio, and accessibility; Linux CI installs lavapipe.
  `deny.toml`: epaint_default_fonts font-licence exception; zbus 5 "unmaintained" ignores
  (accesskit_unix, no upgrade).
