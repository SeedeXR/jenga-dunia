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
| P6 | Design pass (owner, 2026-10-27: "a clean well designed ui like zed is or figma is"): the floating panels kept (polished), status bar + log dock, sections and controls, neutral palette, the floating viewport icons kept in place but polished (states, tooltips, responsive); both UIs together. Spec: `docs/research/research-log/2026-10-27-zed-figma-ui.md` | in progress | parity captures before / after; e2e + kittest green |

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
  them in the same commit, its e2e unchanged. (2) Live evaluation: each window evaluates its own
  edits at the desktop's adaptive preview lod, the other adopts the result (`evaluated` event;
  revised from "moves to Studio", see the live-evaluation slice below).
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
- P4 slice 9 — the left column complete: Real world (geo.fetch_dem → the world in the DEM's CRS →
  a heightmap node wired into the height output → evaluate, one chain; tested against a loopback
  Terrarium server by re-running the test as a child with JD_GEO_TERRARIUM_URL, as the workspace
  forbids `unsafe` set_var), Reference image (media.metadata, Use sun, Terrain here), Layers (the
  desktop's LayerStack: chain + grouped side branches, search, bypass, up/down, bin, `+`), Settings
  (language, the project's settings committed on leaving a field, Preview detail, Live target,
  origin, CRS, wind), Tasks (jobs.list polled every 250 ms while busy, Cancel), Diagnostics.
  Shared with the desktop: i18n.json (both UIs read it); core ops graph.unsplice and
  graph.swap_in_chain (all or nothing via mutate_all — the old client-side swap could leave the
  graph half-rewired when it met a generator). Studio remembers language / preview detail / live
  target (eframe persistence; window size and egui memory not persisted, as the desktop). Fixed:
  a widget asking for more than the available width grew the left panel every frame (guarded by
  a layout-stability check in the first window test).
- P3 / P4 slice 10 — the ground tools: the viewport's armed tool (primary drag = a stroke of
  ground points, right drag orbits, the desktop's red ring, Ctrl/⌘ + wheel radius, Shift + wheel
  strength); Drawing = Sculpt | Road | Cave(id). Sculpt: graph.sculpt_target (new core op, the
  desktop's strokesNodeFor / transformsNodeFor, shared) → strokes (raise / lower / smooth /
  flatten, alphas) into the strokes node, transforms (rotate / scale / tilt / push / align) into
  the transforms node; the Brush section (mode, fields, alphas, radius, strength / amount,
  height, hardness, layer settings, Fit to terrain, shortcuts) while a tool is armed, as the
  desktop's `sculpting`. Road: one drag → jd.terrain.spline@1 (12 m), then orbit. Paint density:
  the mask's strokes node clamped 0..1, raise 0.5. Cave entrances placed by click. Studio's
  strokes go to the strokes node chosen when sculpting started (the desktop selects the
  transforms node after a transform stroke and then stops painting — a desktop follow-up).
  Fixed: 11 icon files were never registered (the alphas drew circles) → every_icon_file_is_registered.
- P3 / P4 slice 11 — the Grid tool: the chip (puts the ground tools away), the overlay (cell lines
  draped on the preview 0x8fb0ff 55 %, selected cells as fans 0x5b8cff 45 %, lifted 0.2 %; a new
  unlit blended triangle pipeline `flat`), the selection (click toggles, drag adds, Alt + drag
  removes, right drag orbits), the Grid section (cells 8–64 clear the selection, Selected,
  Smoothness, Height + Set height, Clear, Step + Raise / Lower — square stamps in the strokes node,
  one stroke — and Duplicate / Rotate / Scale / Tilt / Align of the selection's bounding block into
  the transforms node, the shortcuts). P3 viewport and P4 panels are now complete.
- Selection sync and the Editor's live graph: `ui.select { nodes }` → the server's `selection
  { nodes, source }` (jd-ipc answers it; the desktop alone answers it locally). The Editor selects
  what Studio selects, sends its own selection, and reloads its graph on another client's graph.*
  write (it listened to no engine event before — Studio's stack / sculpt / splice edits did not
  show in the Editor's graph). Studio follows the Editor's selection (stack row highlight).
- Live evaluation (decision 2 revised): every jd-ipc peer is a window (`gui-<role>`), so there is
  no headless writer for Studio to evaluate for; instead each window evaluates its own edits and
  the server broadcasts `evaluated { report, client }` so the other window adopts it. Bug fixed on
  the way: the Editor's graph reload on a Studio edit ran the Editor's own live evaluation too
  (two evaluations per edit); a reload for another window's edit no longer evaluates. Studio's
  preview LOD now takes the Live target's adaptive bias (the desktop's adaptLodBias, unit-tested),
  moved by any evaluation at the current preview LOD.
  Next: P5 (default switch, packaging, docs).
- P5 slice 1 — packaging: cargo-packager 0.11.8 (`[package.metadata.packager]` in apps/studio,
  `scripts/package.sh`), Studio the main executable, the Editor the embedded helper "Jenga Dunia
  Editor.app" (own bundle id). Found by running the bundle: (1) with `jd-app` beside `jd-studio`
  in Contents/MacOS, the Editor process also counts as the app — `quit` and a Finder open reached
  the Editor instead of Studio — hence the helper; (2) winit 0.30.13 *does* own the application
  delegate (its doc comment says otherwise; replacing it panics), so `crates/jd-macos` (unsafe
  allowed, like jd-fbxsdk) installs an `aevt/odoc` handler through NSAppleEventManager from a
  will-finish-launching observer; (3) Studio logged only to stderr — it now also writes
  `jd-studio.log` beside `jd-app.log` (`jd_ipc::log_dir`, shared); (4) Studio's open / create did
  not evaluate (the desktop's do) — now chained with a preview evaluation; (5) the Editor's
  `project-opened` re-read of Studio's opens evaluated again — replaced by `followProject`.
  Verified with the packaged app: open, second open, quit (see docs/user/desktop-app.md).
  Retiring the WebView viewport — owner, 2026-10-27: "I want a powerful 3dview from egui and it's
  panels ui and ux shouldn't [be compromised]". Approved: the Editor keeps the graph and layer
  stack; Studio is the only 3-D view and panel set. Before deleting: a capability-by-capability
  audit desktop → Studio, every gap closed in Studio first. Then the design pass (P6) covers
  Studio and the graph window only.
- P5 slice 2 — closing the gaps the audit found (desktop → Studio, 2026-10-27; ~90 % already
  matched). Each closed in Studio, tested, before the WebView viewport goes:
  G1 shortcuts (⌘N/O/S/G/R/E/I, ⌥⌘Z, ⇧⌥⌘Z, ⌘W) and the native menu bar — the tooltips promise them;
  G2 Esc cancels jobs, header job progress (op · node n/N · s, Cancel), Tasks shows every job
     (Editor's, MCP's) and without a project;
  G3 material detail tiles drawn in the viewport (the Materials "detail" has no visible effect);
  G4 side panels collapse to a strip; G5 panel sizes and window size / position remembered;
  G6 the Graph chip brings a running Editor to the front;
  G7 viewport hints per tool (sculpt, grid, road, cave entrance);
  G8 pinch / ⌘ + wheel zoom with no tool armed, middle drag orbits;
  G9 selection reaches the Editor (cave summary click, Sculpt start);
  G10 Diagnostics without a project; G11 number fields step with the arrow keys;
  G12 a crash report for Studio (panic hook → log + diag).
  Closed (slice 2a): G7 hints per tool (chrome::hint_rows, unit-tested); G8 pinch / ⌘ + wheel
  zoom and middle-drag orbit; G2 the server's `busy { running }` event (flips under a lock, jobs.*
  not counted; ipc test mutation-checked) → Studio polls jobs.list while any window works, the
  header's job line (op · node n/N kind · s, Cancel, Esc to abort; job_line unit-tested), Esc
  cancels every job outside text fields; G10 Tasks and Diagnostics without a project (window
  test). The audit's "Tasks shows the Editor's jobs" was beyond the desktop (its Tasks polls only
  around its own ops) — needed now that two windows share one engine. The protocol doc listed a
  `job` event that was never sent; replaced by `busy`.
  Closed (slice 2b): G1 — `menu.rs`: one command table (the Editor's ids, labels, accelerators);
  the toolbar, the keys (egui consume_shortcut, every platform; Redo tried before Undo since egui
  lets ⇧ through) and the macOS menu bar (muda 0.20, already in the lock via Tauri; installed from
  main.rs on the main thread) all call `StudioApp::run`, gated by `can_run` (the toolbar's enabled
  states). The Edit menu leaves the system's copy / paste items out: their selectors would go to
  winit's view and could swallow egui's own ⌘C / ⌘V. Verified on macOS 27 with System Events:
  menu bar "File, Edit, Terrain, Window"; File ▸ Save ran project.save; Terrain ▸ Evaluate Graph
  started the 4096² evaluation. Window test: ⌘S saves, ⌘R evaluates.
  ponytail: no Windows / Linux menu bar (muda's needs `unsafe init_for_hwnd` / a GTK window); the
  keys work there and the Editor window keeps its menus.
  Closed (slice 2c): G4 the side panels collapse to a 24 px strip and come back (the desktop's
  buttons and tooltips; `jd.layout.dock` persisted; window test: the viewport gains the room);
  G5 window position / size (eframe `persist_window`) and panel sizes (egui memory) come back on
  the next launch — the comments saying the desktop does not were wrong (tauri-plugin-window-state,
  `jd.layout.<panel>`). Verified on macOS: moved / resized, Quit Jenga Dunia, relaunch → same
  bounds. The app menu's Hide / Quit read "jd-studio" (muda's default is the process name) — named
  "Jenga Dunia".
  Closed (slice 2d): G3 the material detail tiles — `terrain.textures` `details[]` and
  `detail_weights_png_base64` decoded into `viewport::Maps`; bindings 7–15 (weights, four
  albedos, four normals) and a wrapping sampler (16; the main one clamps v for the sky); the
  terrain shader mixes each tile over the macro maps by its weight channel, repeated extent ÷
  tile_m (the desktop's TSL `mix`), normals mixed before unpacking. Tests: the materials window
  test reaches `detail_tiles() == 1` from a real bake; a render test with a checker tile at
  weight 255 vs 0 changes > 10 % of the viewport (mutation: blend off → 0 pixels, fails). A first
  render test compared before / after the re-bake and passed with the blend off — the bake itself
  changes the pixels — so it was replaced.
  Closed (slice 2e): G6 the protocol's `raise { role }` — Studio's Graph button (enabled while the
  Editor runs) emits it, the Editor unminimises, shows and focuses its window. Verified on macOS:
  Editor minimised (System Events AXMinimized), Studio's button clicked through accessibility
  (found by its accessible label), AXMinimized → false. G9 one `select_node` path (layer stack,
  Sculpt's strokes node, the cave line — now clickable, the selected cave's box in the accent)
  announces `selection` to the Editor; caves window test (mutation-checked).
  Closed (slice 2f): G11 text fields holding a number step with ↑ / ↓ (Shift × 10) by their last
  decimal place (`look::step_number`, unit + window test; ponytail: not each field's own `step`);
  G12 `crash::install` — a panic on the window's (main) thread is logged and written by
  `diag.crash_report` (engine ops report their own; the hook skips other threads, where the
  engine may hold locks); tests/crash.rs: one report for `main`, none for a worker.
  All twelve audit gaps closed. Next: retire the WebView viewport and panels (P5).
- P5 slice 3 — retiring the WebView viewport and panels. Map: memory/p5-retire-map-2026-10-27.md
  (every e2e spec → KEEP / DELETE / MIXED, Studio coverage, the gaps; then which gaps engine tests
  already cover). Order, so no coverage is lost:
  (a) port the gaps: 7 engine behaviours with no test → jd-core tests (look ↔ scene.json and
      reopen; add_simulation debris / sand; sculpt_target {layer, clamp}; project.cache_gc;
      project.write_export confinement; settings / CRS through save → reopen; scatter follow_time);
      UI-only gaps → Studio window tests (sculpt / alphas / sculptlayer done: one test);
  (b) KEEP / MIXED specs re-based on core setup (`project.create` before goto, as
      parity-capture did), log checks → core reads; bridge.ts `wire` / `evaluateAndWait` too;
  (c) App.tsx down to the graph + layer stack (+ engine events, menu handler, crash reporter),
      TerrainView, sea / camera / previewLod tests and DELETE specs removed; CI and pre-push run
      the remaining specs;
  (d) docs (desktop-app.md's spec references), the figures in the design pass (S4).
  Progress (a): engine — crates/jd-core/tests/desktop_flows.rs (the 7). Studio — sculpt / alphas /
  sculptlayer (one test); grid (right drag keeps the selection, Set height → max exactly 700,
  Raise 25 → 725, ⌥⌘Z back to 700, half-cell radius); jobs (header line, Cancel → E_CANCELLED,
  Esc aborts the next); caves (an entrance's ×, Show caves off); road (one splice, the node
  selected). Found while porting: Studio did not select a spliced node (the desktop's
  spliceBefore callers do — road, actions, the layer strip): now selected in the chain reply.
  Batches 3–4: preview detail → fetch LOD (`fetch_lod`, unit test); settings defaults and an
  out-of-range cache refused with the field back to 2048; Still → exports/stills/*.png; the
  detail tile 8 m default, 4 m re-bakes; the Desert plain preset (1200 m, 3 octaves, Generate
  keeps 0..40 m); diagnostics lines (cpus / GB, blobs · limit 2048 MB, S1 256² … thermal×50);
  scatter growing with the timeline (0 at t = 0, > 100 at t = 1 after the Editor's set_time +
  evaluate).
  Batch 5: the sun / sky / clouds / fog readouts follow their sliders; PNG export 16-bit with
  W_EXPORT_LOSS; an EXR export back through Import as a heightmap node. Fix found by the port:
  Studio's sliders (widgets::Range, VBar) lost the keyboard focus after one ← / ↓ step — egui
  moves the focus with the arrows unless the focused widget claims them (its own Slider does);
  both now set a focus-lock filter for the arrows.
  Batch 6: the sea from its node (sea 0 → 250 m, waves 9.8 m / 380 m at 20 m/s, water maps
  export), the shoreline readout and the ruler's max = the layer's, lakes from a fill node; grid
  Scale (factor 0) / Rotate / Align ops in order; the push brush's grab op.
  Batch 7: the reference clip (place, time, sun; Use sun → a north-east morning sun); an opened
  project's saved look in the panels; the strip's cone a removable shape layer (the generator
  not); a half-wired graph's live preview waiting without an error; Paint density's clamped
  strokes node on the mask, the brush armed. Fixes found by the port: (1) Studio's Noise section
  did not mirror the noise node — an Editor edit (seed 9) stayed 0 in the panel, and Generate
  wrote it back over the edit; now `sync_noise` adopts the node's params when they change (the
  desktop's syncNoise); (2) a chained (live) evaluation of a half-wired graph logged a red error;
  now "live preview waiting: …" as info, as the desktop's liveEvaluate.
  For the retirement (c): the Editor's own log panel goes with its panels — it still needs a
  place to say what its live evaluations wait on and what failed (a status line in the graph
  window), designed with S3.
  (b)+(c) done (2026-10-27): the WebView viewport and panels retired. The Editor (App.tsx) is the
  layer stack beside the graph (+ inspector), a header (brand, project, job line with Cancel and
  Esc), a status bar with the last log line and a closed log dock; it keeps the engine events,
  the menu handler, the crash reporter and Esc. state.ts pruned to the graph / project / live
  evaluation / jobs / log (1 400 → ~370 lines: no previews, maps, sky, scatter, caves, sculpt,
  grid, flythrough). Removed: TerrainView.tsx, camera.ts, previewLod.ts, i18n.ts (+ tests; Studio
  owns i18n.json now: apps/studio/assets/i18n.json), api.ts's preview / texture / sky / HDRI
  helpers, 23 Playwright specs (covered by Studio window tests and jd-core desktop_flows) and
  tileServer.ts. The 15 kept specs create their project through the core (`createProject`), read
  layers / stats through it (`coreOp`, `layerStats`), and do Studio's side of a workflow (a
  simulation layer, a cone, painted density) through the core + reload. App.test.tsx: the
  Editor-level tests (menus, noise mirror, Studio sync, status bar / log).
  Found: the layer stack's + offers kinds that need params (jd.shape.primitive: "missing field
  `shape`") — follow-up for S3. scripts/studio-icons.mjs now reads Studio's icon list (it read the
  desktop's App.tsx imports). Owner, same day: no mountain icon — Generate is the word alone, the
  strip's cone button draws lucide `cone`.
- P6 S1, first part (2026-10-27): the neutral palette in both UIs (bg 141414 / 1c1c1c / 262626 /
  333333, line 2c2c2c, fg ececec / b8b8b8 / 858585, accent 4b9cf5, primary 1d72e8; the token test
  keeps them matched); Studio's status bar (24 px: the last log line, one click target that opens
  the log dock — closed by default — and the viewport readout, moved out of the header, now
  36 px); sections set apart by a hairline after the first, 12 px headings; buttons and fields
  filled without borders, 4 px corners; the floating cards keep their place with a 10 px radius
  and a quieter shadow (the Editor's CSS the same). Window test for the status bar.
  Next in S1: the floating icons' states / tooltips with shortcuts / narrow-viewport behaviour,
  then the Editor's graph window (S3).
  Open (flake, 2026-10-27): `e2e/paint.spec.ts` failed once inside a full gate run (instance count
  stayed 925 after the density stroke; 30 s poll). Not reproduced in 5 full suites + 12 parallel
  repeats; trace lost (overwritten by reruns — next time copy `test-results/` first). Ruled out:
  overlapping live evaluations publishing stale layers (jd-core refuses: `W_STALE_GRAPH` on a
  revision change) and a non-graph write making the stroke's evaluation stale (only graph.* /
  set_settings bump the revision). Unchecked: the stroke's evaluation dropped by the desktop's
  `graphGen` check with no follow-up evaluation, and `placeScatter` racing `adopt`.
  Studio's chains each run on their own thread, so its evaluations can overlap too (harmless for
  the same reason, but wasted work under fast strokes).
- Desktop ↔ Studio brush parity: after a transform stroke the desktop selected the transforms
  node and its sculpt strokes silently stopped (they followed the selection only). It now
  remembers the strokes node Sculpt / Paint density chose (`sculptNode`, Studio's `sculpt_node`):
  the brush paints into the selected strokes node, else that one; the selection still moves.
