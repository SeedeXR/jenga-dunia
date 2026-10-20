# Desktop app (Phase 2, early)

Status: first working build. One window, native macOS title bar, dark theme (Host Grotesk, Lucide icons). The layout follows World Creator's left-to-right workflow — project on the left, viewport in the centre, terrain/noise controls and export on the right, operations log at the bottom — and will grow into the full workspace of `docs/architecture/12-ui-workspace.md` (node editor, sculpting, materials, environment, jobs).

![Jenga Dunia desktop app, empty project state](images/desktop-empty.png)

*Real screenshot of the running app on macOS (2026-10-20, v0.0.1). The header reports the viewport backend: here **WebGPU** inside the system WebView.*

![Jenga Dunia with a generated 2048² terrain in the viewport](images/desktop-generated.png)

*After **Terrain → Generate Terrain** (⌘G) driven through the native menu bar on a 512² project (release build, Apple Silicon): generation 15 ms, 0.5 MB preview to the WebGPU viewport in 1 ms, 522 242 triangles. Earlier measurement at 2048²: generation 58–63 ms, 2 MB preview in 6 ms; `exports/height.exr` validated by macOS ImageIO as 2048×2048, 32 bits/sample.*

![File menu: New Project…, Open Project…, Save, Export Heightmap…](images/desktop-menu-file.png)
![Terrain menu: Generate Terrain, Evaluate Graph](images/desktop-menu-terrain.png)

*The Edit menu adds **Undo Graph Edit** (⌥⌘Z) and **Redo Graph Edit** (⇧⌥⌘Z) below the WebView's Undo/Redo.*

## Native integration

- Menu bar: **Jenga Dunia** (About, Services, Hide, Quit) · **File** (New… ⌘N, Open… ⌘O, Save ⌘S, Export Heightmap… ⌘E, Close Window ⌘W) · **Edit** (Undo/Redo/Cut/Copy/Paste/Select All) · **Terrain** (Generate ⌘G, Evaluate Graph ⌘R) · **Window** (Minimize, Zoom, Full Screen). Menu actions run the same operations as the buttons.
- Native file dialogs (tauri-plugin-dialog): **New…** asks where to create the `.jdproj` directory, **Open…** picks an existing one, **Export Heightmap…** is a save dialog with EXR / PNG 16-bit / RAW filters. The folder buttons next to the Path and File fields open the same dialogs; typed paths still work (and are what the tests drive).
- Launch with a project: `jd-app ~/Worlds/Hills.jdproj` (or double-click a `.jdproj` once the bundle's file association is registered) opens it before the window appears.
- Window position, size and maximised state are restored on the next launch (`tauri-plugin-window-state`; verified natively: move/resize → Quit → relaunch restores the exact bounds).
- Dark window theme, native title bar, Host Grotesk typography.

## Run from source

```sh
cargo build -p jd-mcp            # not needed by the app; used by the UI end-to-end tests
cd apps/desktop && pnpm install && pnpm tauri dev
```

## Build the app bundle

```sh
cd apps/desktop && pnpm tauri build --no-sign
# → target/release/bundle/macos/Jenga Dunia.app (18 MiB) and dmg/Jenga Dunia_0.0.1_aarch64.dmg (7 MiB)
```

The bundle is **unsigned** (no Apple developer account yet): macOS Gatekeeper will ask you to confirm the first launch of a downloaded copy; a locally built bundle runs directly. `src-tauri/Info.plist` (merged by the bundler) declares `.jdproj` as a **package** document type (`LSTypeIsPackage`, UTI `com.seedexr.jengadunia.project`), so Finder treats the project folder as a single document owned by Jenga Dunia: double-click, **Open With**, or a plain `open My.jdproj` launches the app with that project — macOS hands the document over as an Apple Event, which the app handles alongside the `jd-app My.jdproj` launch argument used on Linux/Windows and from the CLI. Verified on macOS 27 (2026-10-20) after `lsregister -f "Jenga Dunia.app"`: `open H512.jdproj` started the app, opened the project and live-previewed it; the app menu reads **Jenga Dunia**.

Logs: `~/Library/Logs/com.seedexr.jengadunia/jd-app.log` on macOS (`$XDG_DATA_HOME/com.seedexr.jengadunia/logs` on Linux, `%LOCALAPPDATA%\com.seedexr.jengadunia\logs` on Windows), plus stderr; `JD_LOG=debug` raises the level.

## Tutorial: first terrain in four steps

![Step-by-step: create a project, generate, export, read the log](images/tutorial-01-create-project.png)

1. **Path** — type the project directory (created on disk as a `.jdproj` folder), or use the folder button / **File → Open…** for a native dialog.
2. **Preset** — Unity 1025²/2049²/4097², Unreal 2049²/4033² or plain squares fill resolution and extent.
3. **New (⌘N)** — or **Open (⌘O)** for an existing project; the header shows `name — resolution² · extent m`.
4. **Generate (⌘G)** — after choosing lattice, fractal, seed, feature size, octaves, lacunarity, gain and height range.
5. **Export heightmap (⌘E)** — `.exr`/`.tif`/`.r32` (exact), `.png`/`.r16` (16-bit, warns about quantisation).
6. **Log** — every operation with warnings and errors (stable code, why, what to do next).

![After generating: viewport readout, layer stats, export](images/tutorial-02-generated.png)

1. Viewport readout: backend (WebGPU here), triangle count, preview size and transfer time. Shading is a preview: altitude tint (valley green → rock → snow), steep faces towards rock grey, sky with distance fog.
2. Layer statistics: min/max/mean height, generation time and the content hash — the same seed and parameters always give the same hash.
3. Export writes `exports/<file>` plus a `.jd-height.json` sidecar (size, range, flip, byte order); the folder button opens a native save dialog.
4. **Sculpt** switches the viewport to brush strokes (tutorial below).

*Annotations are generated from the real screenshots by `apps/desktop/scripts/annotate.mjs` from the `.annot.json` specs next to the images, so they can be regenerated whenever the UI changes.*

**Screen recording:** [`videos/first-terrain.mp4`](videos/first-terrain.mp4) (14 s, 1440×892) — the real app opened with a project, **Terrain → Generate Terrain** and **File → Export Heightmap…** triggered through the native menu bar, terrain appearing in the WebGPU viewport, `height.exr` written. Recorded with macOS `screencapture -V`; the menu actions were driven by System Events so the recording is reproducible.

**Screen recording (v0.0.1):** [`videos/graph-erosion-native.mp4`](videos/graph-erosion-native.mp4) (16 s, 1440×892) — the current build opened on a `noise → hydraulic erosion → height` project: the live preview shows the eroded terrain as the project loads, **Terrain → Generate Terrain** replaces it with the raw noise field, **Terrain → Evaluate Graph** brings the eroded result back from cache. Native window, release build, menu actions driven by System Events.

## Tutorial: the node graph

Every project starts with a two-node graph (`Noise → Height`), the same `graph.json` the CLI and MCP server edit. The panel under the viewport is that graph; **Hide graph** collapses it.

![Node graph: add, evaluate, edit, connect, inspect](images/tutorial-03-graph.png)

1. **Add** — pick a kind from the list (`graph.kinds` drives it) and click **Add**; the node is placed to the right of the graph and the view re-fits.
2. **Undo · Redo · ⚡ · Evaluate** — the toolbar's history buttons, the live-preview toggle and full-resolution evaluation, also **Terrain → Evaluate Graph (⌘R)**. While an evaluation runs, the header shows the job (`graph.evaluate · node 2/4 hydraulic · 3.2 s`) with a **Cancel** button; cancelling stops at the next node or erosion iteration and logs `E_CANCELLED` — nothing is published. With **live preview** (⚡, on by default) every edit re-evaluates at preview resolution (≤ 512², published as `height#lod<N>`) so the viewport follows your changes; unchanged nodes come from cache. A half-wired graph pauses the live preview with a `live preview waiting: input … not connected` line instead of an error. Export always evaluates at full resolution first.
3. **Nodes** — click to select, drag to move (the position is saved in the project), **Delete**/**Backspace** removes the node and its edges. The ↶ ↷ buttons (or **Edit → Undo/Redo Graph Edit**, ⌥⌘Z / ⇧⌥⌘Z) step through the edit history; ⌘Z alone stays with the text fields. The capture shows a `noise → hydraulic erosion → height` graph (the hydraulic node overlaps the noise node because it was added from the CLI without a position).
4. **Inspector** — the selected node's parameters, generated from the kind's JSON schema (enums become drop-downs, defaults show as placeholders). Leaving a field applies `graph.set_param`; an invalid value is rejected by the core before anything is journaled and the reason appears in the log.
5. **Viewport readout** — backend, triangle count and preview transfer for the evaluated layer (here the hydraulically eroded 512² terrain, live-previewed when the project opened). When the graph publishes more than one layer (e.g. a second output node named `water` fed by the hydraulic node's `water` port, or `slope` from the slope mask), a **layer** picker appears in the header; the viewport and stats follow the chosen layer, and live previews keep showing it.

6. **Sculpt** — enters sculpt mode (next tutorial).

Undo and redo are operations like everything else (`graph.undo` / `graph.redo`, journaled as `graph.restore`), so a crash in the middle of undoing recovers to exactly what you saw. Connect nodes by dragging from an output handle (right) to an input handle (left); an input holds one edge, so dropping a new connection replaces the old one. Cycles, wrong ports and type mismatches are refused by the core (`E_GRAPH_CYCLE`, `E_INPUT_INVALID`, `E_GRAPH_TYPE_MISMATCH`). Evaluating while a required input is unconnected reports `E_GRAPH_MISSING_INPUT` with the node and port.

**Autosave.** Every edit is journaled first (crash-safe), and `settings.autosave_interval_s` seconds after the last edit (project.json, default 120; `0` disables) the app saves the project on its own: the log shows `project.save: ok`, the project panel's **dirty** returns to `false` and **journal** to `0`, and `graph.json` on disk is current. Press **Save** (⌘S) to save immediately. The UI-driven test `apps/desktop/e2e/hydrology.spec.ts` runs with the interval set to 2 s and checks the saved `graph.json` and the truncated journal. (For debugging, `?autosave=<ms>` in the dev URL overrides the interval.)

![Project panel with the Settings section: Autosave (s), Cache limit (MB), Preview LOD](images/desktop-settings.png)

*Settings live in the project panel: **Autosave (s)**, **Cache limit (MB)**, **Preview LOD** and **Memory budget (GB)** (evaluations whose pre-flight estimate exceeds it are refused with `E_RESOURCE_MEMORY` — the default 4 GB is the project's hard ceiling) are `project.json → settings`, edited through `project.set_settings` — a journaled mutation like everything else, so a crash mid-edit recovers it. The core validates every value (cache limit ≥ 16 MB, autosave ≤ a day, LOD ≤ 8) and a refused value snaps back to the stored one with the reason in the log. The UI-driven test `apps/desktop/e2e/settings.spec.ts` sets autosave to 1 s, tries an 8 MB cache (refused), edits a node and watches the project save itself.*

![Terrain menu with Generate Terrain and Evaluate Graph](images/desktop-menu-terrain.png)

**Screen recording:** [`videos/graph-editor.mp4`](videos/graph-editor.mp4) (17 s, 1440×900) — the whole flow above: edit the seed, add a blur node, connect it by dragging the handles, move a node, evaluate, delete with the keyboard, undo and redo. It is the UI-driven test `apps/desktop/e2e/graph.spec.ts` running against the real core, recorded with `JD_VIDEO=1 pnpm exec playwright test e2e/graph.spec.ts` (actions slowed to human pace), so it is regenerated whenever the editor changes.

## Tasks and diagnostics

![Project panel: Tasks (no running jobs) and Diagnostics with host, cache and bench results](images/desktop-diagnostics.png)

*Below the settings the project panel lists **Tasks** — every running job with its progress and a **Cancel** button (the header shows the first one; detached MCP jobs appear here too) — and **Diagnostics**: **Environment** (`diag.env`: OS, CPU count, memory), **Cache GC** (`project.cache_gc`: blobs and bytes kept, evicted, the project's limit) and **Bench 256²** (`diag.bench` S1 at a small size: per-stage milliseconds and peak memory; the CLI's `jd diag bench s1` is the comparable benchmark). Every result is an ordinary op envelope, so the log shows the same `ok`/error lines agents see. UI-driven test: `apps/desktop/e2e/diagnostics.spec.ts`.*

## Sky and sun

![Viewport with the gradient sky and a low warm sun; the Sun panel shows azimuth 140° and elevation 12°](images/desktop-sun.png)

*The viewport draws a gradient sky dome (deep blue zenith → pale horizon, dark below) and lights the terrain with a sun you steer from the **Sun** panel (1): azimuth is the compass direction the light comes from, elevation its height above the horizon — below 20° the light turns warm and dimmer, like World Creator's time-of-day. The header reports the current direction (2). The sun is a view setting, not part of the project. UI-driven test: `apps/desktop/e2e/terrain.spec.ts` drags the elevation to 12° and checks the readout.*

![Viewport textured with baked maps: green lowlands, ochre slopes, grey rock and snow with fine relief; the Sun panel's Textured (PBR) toggle is on and the header reads pbr 256×256](images/desktop-pbr.png)

**Screen recording:** [`videos/terrain-pbr-import.mp4`](videos/terrain-pbr-import.mp4) (16 s, 1440×900) — new project, Generate, the PBR-textured viewport with the toggle switched off and on, the sun lowered, an EXR export, and the same file imported back as a heightmap input node that is wired to the output and evaluated. It is the UI-driven test `apps/desktop/e2e/terrain.spec.ts` recorded with `JD_VIDEO=1`, so it is regenerated whenever the flow changes.

*Textured terrain: with **Textured (PBR)** on (1, default) the viewport shades the height layer with the same maps the GLB export carries — albedo, tangent-space normal and roughness baked from the full-resolution field by `terrain.textures` (one texel per sample, capped at 1024 per side for the viewport). The normal map restores the relief that the preview LOD drops on large worlds, so a 4096² world previewed at 1024² still shows its fine ridges; the header reports the map size (2). Off, the terrain falls back to the height-ramp vertex colours; a **colour** overlay always wins over the maps. Maps are re-baked after each evaluation of a height-like layer, not for masks. UI-driven test: the terrain spec checks the readout and the toggle both ways.*

## Hydrology layers in the viewport

![Rivers project: carved terrain with the layer picker showing height, rivers and wetness](images/desktop-rivers.png)

*Native capture (v0.0.1): a `noise → rivers → height` graph with two extra outputs (`rivers`, `wetness`) built from the CLI (see the hydrology recipe in the CLI quickstart) and opened in the app. The header's **layer** picker lists every published layer; the viewport shows the carved terrain, and picking `rivers` or `wetness` shows those masks with their own stats.*

![Viewport coloured by the rivers mask while the height geometry stays; header colour select set to rivers](images/desktop-colour-by-layer.png)

*Colour by layer: the header's **colour** select paints the terrain with any published layer instead of the height ramp — masks (rivers, wetness, slope, scatter densities) as a dark-to-blue gradient, class maps (biomes) with one hue per class — while the geometry stays the height layer. It is how you check a mask against the relief without leaving the 3D view; the header line reports `colour <layer>`. UI-driven test: `apps/desktop/e2e/hydrology.spec.ts`.*

![Project panel with the Import row filled with an EXR path, and a new heightmap input node in the graph reading it](images/desktop-import.png)

*Import a heightmap or mesh: the **Import** row in the Project panel (or **File ▸ Import Heightmap or Mesh…**, ⌘I, which opens the native file dialog) takes a PNG 16-bit, EXR, TIFF, RAW or binary FBX file, copies it into the project's `assets/` through `project.import_asset`, and adds a `jd.input.heightmap@1` node that reads it by name. Wire the node into an output and evaluate: the e2e test exports the generated terrain as EXR, imports it back this way and checks the layer's min/max/mean are the same again. An FBX mesh (sculpted in Blender, for instance) is rasterised over its footprint onto the world grid; quantised PNG/RAW files need their height range set on the node.*

![Biomes project with a snow node wired from the noise and published as a second output; the terrain is coloured by the snow mask, white on the high shaded slopes](images/desktop-snow.png)

*Snow cover: `jd.climate.snow@1` turns a height layer into a 0..1 cover mask — a soft **snow line** (`snow_line_m` ± `transition_m`), retention that falls to zero at `max_slope_deg`, sun-facing slopes melting (`sun_azimuth_deg`, `sun_strength`) and windward slopes scoured (`wind_azimuth_deg`, `wind_strength`). Publish its `snow` port as a layer and it works everywhere a mask does: **colour** overlay (shown), scatter density, or an input to biome rules. The UI-driven biomes test adds the node, lowers the snow line to 300 m, wires and evaluates it, then colours the terrain by it.*

![Scatter panel set to the rivers mask with instances drawn along the rivers in the viewport; header readout shows the instance count](images/desktop-scatter.png)

**Screen recording:** [`videos/hydrology-scatter-sea.mp4`](videos/hydrology-scatter-sea.mp4) (35 s, 1440×900) — the whole hydrology flow: add a rivers node, switch its method in the inspector, wire it, evaluate, publish the rivers mask as a second output, colour the terrain by it, scatter instances along the rivers and export them as GLB, then add a sea-level node and raise the water to 250 m with its foam shoreline. The UI-driven test `apps/desktop/e2e/hydrology.spec.ts` recorded with `JD_VIDEO=1`.

*Scatter instances: the **Scatter** panel (right column, under Sun) takes any published 0..1 layer as a density **Mask**, a **Spacing** in metres and a **Seed**; **Place** calls `terrain.scatter` without a path, so the points come back inline and the viewport draws them as green cones (one `InstancedMesh`, cone height the larger of 0.35 × spacing and 1.2 % of the world extent, so they read at any world size; yaw and a 0.75–1.25 scale from each point's `r`, exactly what the GLB carries). The header readout shows `· N instances`, and the panel keeps the count. Instances follow the graph: after every evaluation they are re-placed while the mask layer is still published and dropped when it is not. **Export .glb** writes `exports/<mask>.glb` — one 1 m proxy pyramid instanced with `EXT_mesh_gpu_instancing` (see the CLI quickstart for the Blender-verified import). Inline placement is capped at 200 000 points; beyond that the core asks for a path or a larger spacing.*

![Node editor: rivers node selected, inspector with the method select set to dinf](images/desktop-rivers-method.png)

*Inside the app: pick `jd.hydrology.rivers@1` in the kinds dropdown and **Add** (1), click the node (2) and the schema-driven inspector shows its parameters; `method` is a select built from the node's schema — switch it from `d8` to `dinf` (3) and the node label updates (`method=dinf`) after the core journals `graph.set_param`. `jd.hydrology.sea_level@1` adds the same way and exposes two output handles, `ocean` and `depth`. Numbers that come from the core as 32-bit floats are shown rounded (`0.55`, not `0.550000011920929`). Captured by the UI-driven test `apps/desktop/e2e/hydrology.spec.ts`.*

![Viewport with depth-shaded water at 250 m: pale foam along the shore, turquoise shallows, navy deep water; sea_level node selected](images/desktop-sea-level.png)

![Inspector with the biome rules table: one row named scree with a slope range, add-row button](images/desktop-biome-rules.png)

*Rules as a table: for parameters that are lists of records (biome rules) the inspector shows a table — **+ row** adds a rule, each cell applies on blur (`min,max` for ranges, empty for "no constraint"), the bin removes a row. Every edit is one journaled `graph.set_param`; a value the core refuses (an inverted range) is reported in the log before anything is journaled and the table keeps the last good value. Recorded brush strokes stay a count — they are data, not something to type. UI-driven test: `apps/desktop/e2e/biomes.spec.ts`.*

*Sea in the viewport: as soon as the graph contains a `jd.hydrology.sea_level@1` node (1) the viewport draws water at its `level_m` — edit the value (2) and it moves with it; the header's viewport line reports the height (boxed, top right). The surface is shaded by the depth of the terrain under each vertex: a pale foam band within ~1 % of the height range of the shore, translucent turquoise shallows darkening to opaque navy with an e-folding depth of 12 % of the height range, and nothing drawn over land (alpha 0), so the shoreline reads without a custom shader (`buildSeaGeometry`, unit-tested). Water is only drawn while the **height** layer is shown (masks like `ocean` or `rivers` are 0..1 and would hide under it). Same e2e test.*

![Sea-level node inspector with visibility_m set to 4 and a tint; the water is an opaque blue-green with the sand only showing in the shallowest metre](images/desktop-water-clarity.png)

*Water clarity: the sea-level node carries `visibility_m` (the depth at which the bottom fades out — about 30 m for crystal-clear tropical water, 15 for open ocean, 3 for a murky estuary) and `tint`, the colour of what is suspended in it (blue-green plankton, brown silt, green algae). The viewport colours each vertex with Beer–Lambert transmittance per channel — pure water's absorption (red 0.4, green 0.07, blue 0.005 per metre, Pope & Fry 1997) scaled by 15 m / visibility over the light's path down and back — so the sandy bottom shows through clear shallows, red disappears first, blue carries deepest, and the tint takes over as depth or turbidity grows; opacity rises as the bottom disappears. Set visibility to 4 m (1) and the bay turns opaque blue-green while the shoreline still shows sand through the first metre. Both values are ordinary node parameters, so they are journaled, saved and read by the CLI. Unit-tested (`sea.test.ts`: clear vs murky at 10 m, deep water converging on the tint); the hydrology e2e sets the value through the inspector.*

## Tutorial: sculpting

1. Click **Sculpt** (top of the viewport). If the graph has no stroke node yet, a `jd.sculpt.strokes` node is inserted in front of the output and selected; the button reads **Sculpting…** and a **Brush** panel appears on the right: **mode** (raise/lower by a strength in metres — negative digs; **smooth** blends towards the local mean; **flatten** pulls towards the height under the first point of the stroke), radius, and strength or amount (0–1).
2. Move over the terrain: a red ring the size of the brush follows the pointer on the surface. Drag to paint. The pointer is projected onto the surface and the path becomes one stroke (`points` in layer metres, radius, strength) appended to the node's `strokes` parameter — a journaled `graph.set_param`, so the live preview updates and **Undo** removes the whole stroke.
3. Click **Sculpting…** to go back to orbiting. Strokes are data in `graph.json`: the CLI and MCP see the same node, and re-evaluating anywhere replays them identically.

![Sculpt mode: Sculpt button, brush panel, stroke node](images/tutorial-04-sculpt.png)

*This capture is the UI-driven test (`e2e/sculpt.spec.ts`, Chromium + the real core) rather than the native window; the native screenshots are re-shot when the display is available.*

## Workflow

1. **Project** — **File → New… (⌘N)** and choose where the `.jdproj` directory goes, or type a path (e.g. `~/Worlds/Hills.jdproj`) and click **New**; **Open… (⌘O)** or the folder button picks an existing project. Set extent (m) and resolution (px) before creating, or pick a **Preset** (Unity 1025²/2049²/4097², Unreal 2049²/4033², plain 512²–4096²); engines want 2ⁿ+1 samples so quads line up with their patch grids. The header shows `name — resolution² · extent m`; the panel shows the project id, dirty flag and pending journal entries.
2. **Terrain · Noise** — choose lattice and fractal, seed, feature size, octaves, lacunarity, gain, height range; click **Generate**. Stats (min/max/mean, time, content hash) appear under the button and the viewport shows the terrain with a preview shading: hypsometric tint by altitude (valley green → ochre → rock → snow), steep faces towards rock grey, sky background with distance fog. Drag to orbit, wheel to zoom. Real materials come from the material graph later.
3. **Export** — **File → Export Heightmap… (⌘E)** or the folder button opens a save dialog (the extension picks the format: `.exr`, `.tif`, `.png`, `.r16`, `.r32`); typing a relative name and clicking **Export heightmap** writes into the project's `exports/`. Every export gets a `.jd-height.json` sidecar. Quantised formats log a `W_EXPORT_LOSS` warning.
4. **Save** — writes `project.json`/`graph.json`/`scene.json` atomically and clears the journal.

Every button calls the same operation the CLI and MCP server expose (`jd schema --list`), so anything you do here can be scripted.

## How the UI is tested

| Level | What | Where |
|---|---|---|
| Component (jsdom) | the real `App` with `invoke` mocked at the Tauri boundary: create → generate → stats → preview → export warning; error display | `apps/desktop/src/App.test.tsx` (`pnpm test`) |
| End-to-end (Chromium) | the real frontend driven by Playwright with `invoke` bridged to the **real Rust core** via `jd-mcp` (MCP over stdio): project files appear on disk, the viewport reports its backend and triangle count, a 16-bit PNG is written | `apps/desktop/e2e/terrain.spec.ts` (`pnpm test:e2e`) |
| End-to-end, node graph | starter graph drawn from `graph.json`; edit a parameter, add a node, connect it by dragging handles, drag a node, evaluate, delete with the keyboard; the project's `journal/ops.ndjson` is read back and must list exactly `set_param, add_node, connect, move_node, remove_node` | `apps/desktop/e2e/graph.spec.ts` |
| Native window | screenshots of the real Tauri app on macOS; interaction recording requires Accessibility permission for the automating process | this page |

Known limits of this build: no sculpting, one output layer, no undo of graph edits yet (the journal records them). The tutorial images are the real app annotated by `scripts/annotate.mjs`; the Figma file linked in the brief is the design reference and receives the same screenshots once the connector is authorised.
