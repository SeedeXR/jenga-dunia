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
- P2b slice 1: the Lightroom slider in both UIs (React `Range` on the native input — keyboard,
  a11y and Playwright `fill()` unchanged — with Spectrum's 2 px track, ring handle, track gap,
  double-click reset; egui `widgets::Range` drawn the same, kittest-tested); Studio chrome ported
  1:1: header (brand mark SVG, version, `name — res² · extent m`, lucide toolbar rendered from the
  desktop's own lucide-react by `scripts/studio-icons.mjs`, viewport readout, layer picker),
  `jd-panel` cards, `▾` sections, `Field` rows, Project card (Path/Name/Extent/Resolution/Preset,
  New/Open/Save, Import, id/dirty/journal), log card (local time, `describe` format); native
  dialogs via rfd; Still writes exports/stills/ from the render target; the desktop's camera
  (θ 0.6, φ 1.0, r 1.3·size, fov 50°, both buttons orbit, wheel zoom), ramp stops, slope rock,
  absolute heights and south-row orientation (Studio's mesh had been mirrored N/S).
  Side-by-side (`e2e/parity-capture.spec.ts` vs the window test's `capture_for_docs`): left to do —
  P3: the desktop's default look is textured PBR (`terrain.textures`), sky dome (`sky.preview`),
  framed block + rim, viewport chrome (inner toolbar, compass, ruler, hints, layer strip);
  P4: right column (Terrain · Noise, Export, …) and the rest of the left (Real world, Reference
  image, Layers, Settings). The graph area is the Editor window by design.
- P3 slice 1 (the default look): three's MeshStandardMaterial ported to WGSL (Lambert / π,
  GGX + height-correlated Smith + Schlick, roughness floor 0.0525 + geometry roughness), the
  desktop's sun (1.6 · (0.55 + 0.45 sin el), amber below 20°) and hemisphere light (0.9 · …,
  0xdfe8ff / 0x3a3630), linear fog on view depth (2–12× size, 0xb9c7d9), the sky dome (ray to a
  40× size sphere; `sky.preview` equirect 512 at exposure 0.15, else the gradient), baked PBR maps
  (`terrain.textures`, CPU mip chains, tangent space +x east +y north on the mesh normal),
  the framed block (walls 0x2a2f36 to min − max(10, 15 % relief), teal 0x3fd0c9 rim lines).
  Header readout now equal to the desktop's (… · pbr 1024×1024 · sky 512×256 · box · preview …).
  Owner: the Horizon button's icon is now owner-supplied `horizon.svg` (desktop component +
  Studio asset).
  Next in P3: viewport chrome (inner toolbar: home / still / fullscreen / frame; Grid, Horizon,
  Road, Sculpt, Hide graph; compass strip; height ruler; hints; layer strip), then sea, lakes,
  scatter, caves, overlays, rain, post, sculpt / grid / road / cave tools, flythrough.
- P3 slice 2 (viewport chrome): fit / still / fullscreen / frame (top left), the compass strip
  (heading = −θ, distance), the height ruler (tickStep, Max / Min, negatives warn), the hints, the
  tool buttons (Grid, Horizon, Road, Sculpt — disabled until their slices — and Graph, which opens
  the Editor) and the layer strip (cone, terraces, smooth, thermal, hydraulic, rivers splice
  before the output exactly as `spliceBefore`, then evaluate at the preview lod).
  **Decisions:** (1) workflows with logic (add simulation / cave layers with their materials and
  published layers, sculpt / road / grid node set-up) move into jd-core ops that both UIs call,
  instead of a second Rust copy of state.ts (one implementation, one test); the desktop switches to
  them in the same commit, its e2e unchanged. (2) Live evaluation moves to Studio (engine side)
  for edits from either window, at the desktop's adaptive preview lod; until then each window
  evaluates after its own edits.
- P4 slice 1 (decision 1 executed): `graph.splice`, `graph.add_simulation`, `graph.add_cave` in
  jd-core (unit-tested); the desktop's spliceBefore / addSimulation / addCave now call them (its
  e2e tree, caves, road, actions, simulation pass; road asserts `graph.splice: ok`), and Studio's
  layer strip is fully live through the same ops. Bug fixed on the way: adding a simulation twice
  published its layer twice and broke evaluation; re-published layers now merge with max.
  Studio's live evaluation uses the desktop's previewLod (⌈log₂(res / 512)⌉).
- P3 slice 3: the Horizon skirt (terrain.horizon 3× / 256², ramp × 0.85, no maps, the world's
  footprint cut out, drawn first) — toggle, readout `horizon 256²`, side-by-side checked with
  JD_PARITY_HORIZON / JD_STUDIO_HORIZON captures; the header readout now truncates beside the
  toolbar instead of running under it. Left in P3: Grid, Road, Sculpt tools (raycasting), sea,
  lakes, scatter instances, caves + x-ray, colour overlays, rain, post, stills of flythroughs.
- P3 slice 4 (the sea): sea-level node → sea grid coloured by depth (the desktop's waterColour,
  Beer–Lambert sand + tint, foam band / surf zone), transparent pass (roughness 0.2, metalness
  0.05) with Gerstner waves from ocean.waves in the vertex shader (animated under the desktop's
  ≤ 70 000-vertex rule), the white shoreline (marching squares), the ruler's sea mark, the readout in
  the desktop's full order (… sea · waves · fog · sun · pbr · sky · box · shore · horizon · preview).
  Side-by-side checked (JD_PARITY_SEA capture). Terrain picking (`Viewport::pick`, ray march +
  bisection, unit-tested) is in for the tools; Sculpt / Grid wait for their P4 panels.
- P4 slice 2 (Sun card A) + P3 lakes / rain: the right column with the Sun section — Textured
  (PBR) (desktop-style checkbox), azimuth, elevation, turbidity, clouds, fog, rain — with the
  desktop's 400 ms look write / 150 ms sky re-bake; Studio's own look writes re-read project.info
  only (no terrain refresh). Lakes (fill depth, sea look, sea_fs) and rain streaks (seeded as the
  desktop, 1.2 % fall per 60 Hz frame) in the viewport; readout lakes / rain fields. Order found:
  scatter / caves / overlays / post depend on right-column panels → port them panel by panel with
  their viewport effects. Next: Sun card B (HDRI, exposure / bloom / vignette post pipeline,
  flythrough keys), then Scatter, Caves, Actions, Simulation layers, Materials, Noise, Export.

- P4 slice 3 (Sun card B) + P3 post / flythrough: HDRI on the dome (load, Use sun, ×); the scene
  renders into Rgba16Float (as three's WebGPU renderer always does) and `post.rs` finishes it —
  three r186 BloomNode (high pass 0.85, five separable-blur mips, radius 0.4), the desktop's
  vignette, ACES Filmic with the exposure while any effect is on; Exposure / Bloom / Vignette
  sliders (session only, like the desktop) and the `post …` readout field. Camera row: + key /
  N keys / × / frames, Record (frames along the keys → exports/flythrough/frame_NNNN.png by one
  writer thread, the desktop's summary line), Play (4 s), Still, key list (heading · up ·
  distance, Go, remove); `Camera::along` = lib/camera.ts orbitAt (Catmull-Rom, unwrapped theta,
  log radius). CI: the e2e log cap (200 lines) made `logCount` plateau in long specs → log
  entries carry `seq`, logCount reads the newest.
- P4 slice 4 (Scatter) + P3 instances: Mask (published layers) / Spacing / Seed / Grow, Place,
  model, Export .glb, count; the look's scatter restored from and written to project.json; points
  re-placed after each terrain show while the mask is published (refreshScatter). Viewport: one
  instanced pipeline (inst_vs, terrain shading) with three's ConeGeometry(0.3, 1, 6) or the
  desktop's rockGeometry (IcosahedronGeometry(0.5, 1) + its LCG in JS double arithmetic), sized
  max(spacing·0.35, extent·0.012) × 0.75..1.25 × age; readout `N instances`. Parity capture
  JD_PARITY_SCATTER / JD_STUDIO_SCATTER: 10,609 instances on both. Paint density waits for the
  sculpt tool (Brush panel). Fixes found on the way: Studio's own look write ran a full refresh
  (preview + map re-bake per slider step) → Tag::Look logs only; a map bake landing after PBR
  off re-applied the maps → dropped; the columns' overlay scroll bar covered the right-aligned
  controls once a column scrolled → the right padding moved inside the scroll area
  (look::scroll_card).
- P3 colour overlays: the toolbar's `colour` picker beside `layer` (height ramp or any published
  layer); its preview at the terrain's LOD recolours the mesh with the desktop's overlayColor
  (class maps 0..32 → golden-angle HSL palette, `heat` → lava ramp, else the mask gradient), the
  maps off while it is on (readout `colour X`, no `pbr`); dropped when the layer is no longer
  published.
- P4 slice 5 (Caves) + P3 caves / x-ray: kind + Add cave (graph.add_cave via add_layer), Show
  caves, Map → Footprint (colour cave_plan), X-ray 0..0.9, one box per cave node (line, .glb /
  .fbx via caves.export, entrances with remove via graph.set_param, notes). caves.mesh per node
  after each terrain show (stale fetches dropped by generation); viewport: walls blue-green below
  the water level else rock, cream speleothems, three's SphereGeometry(r, 12, 8) entrance
  markers; x-ray = three's order (markers, caves with no depth test and their glow, then the
  terrain blended at 1 − x-ray, depth written below 0.5), the block's walls and rim hidden.
  Readout `caves N tris · x-ray N %`. Parity capture JD_PARITY_CAVE / JD_STUDIO_CAVE: same cave
  and markers; triangle counts differ only by LOD (the desktop shows its live preview at lod 1 —
  live evaluation in Studio is still to port). Entrance placing (Place) waits for the viewport
  click tools. CI: wire()'s 2.5 s log poll was too short on the runner (one log query ≈ 1.5 s
  while the page renders; trace-checked) → 10 s.
- P4 slice 6 (Actions, Simulation layers): Flip X / Flip Y / Rot ⟲ / Rot ⟳ / Flatten
  (graph.splice of jd.filter.action@1 via add_layer's new Add::Action), + snow / debris / sand /
  lava (graph.add_simulation, its material rows written by the core) and the note; the first
  published mask becomes the colour once evaluated (the desktop's pendingColor, also for the
  layer strip). Found on the way: Studio's fonts had no ⟲ ⟳ ⌘ ⌥ ⇧ ⁿ (the desktop gets them from
  the system) → a 6 KB OFL subset of STIX Two Math as the lowest-priority fallback, and
  tests/fonts.rs renders every non-ASCII character of src/ string literals under kittest's
  panic-on-missing-glyph; project_changed now carries `client`, and Studio skips its own (its
  replies refresh it) — a refresh mid-chain had baked maps for a mask not yet evaluated.
- P4 slice 7 (Materials): one row per layer of the look's stack — mask (published layers or
  everywhere), base colour and emitted light (egui's sRGB-byte colour button = the desktop's
  hexOf / rgbOf), roughness, detail tile + metres (committed on leaving the field), remove — and
  `+ layer`; Studio keeps the stack itself (restored from the look, written after the 400 ms
  debounce, maps re-baked on every edit). Core: MaterialSpec albedo / roughness / emissive are
  f64 now — f32 read back as 0.8999999761581421 in both UIs and in the published schema default.
- P4 slice 8 (Terrain · Noise, Export) — the right column is complete:
  Noise: Preset / Lattice / Fractal / Seed / Feature size / Octaves / Lacunarity / Gain (session-
  only, the desktop's defaults), Strength and the octave bars (widgets::VBar, the native accent
  slider) applied on release, Height min / max, Generate (toolbar ⌘G too: the panel into the noise
  node's differing params, journaled, then a full evaluation; terrain.generate without a noise
  node), the layer's stats grid, Measure landform + readout. Export: file + picker (shared with
  the toolbar), Export heightmap, Holes, Engine Unity/Unreal × tiles (terrain.export_profile, caves
  along), Mesh (cave_holes by default), HDRI bake, water maps (sea node), timeline frames.
  Exports and bakes use Tag::Export (busy, logged, no refresh — the desktop's `run`); before, each
  re-ran the whole refresh and re-baked the maps. look::wide_button centres full-width buttons.
  The symbol font backs Monospace too (the log shows the engine's "2ⁿ+1").
  Next: the left column (Real world, Reference image, Layers, Settings); the viewport click tools
  (cave entrances, sculpt, grid, road); live evaluation; then P5.
