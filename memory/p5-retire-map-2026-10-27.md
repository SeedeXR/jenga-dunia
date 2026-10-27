# P5 retirement map (2026-10-27, Explore agent; spot-check before relying)

Every spec today drives the Editor through panels that are being removed. Even the specs marked KEEP need their setup and some assertions rewritten.

## Shared setup that goes away

All 38 specs create the project with `project-path`, `Extent (m)`, `Resolution (px)` and `new-project` (App.tsx:576, 612). Most also wait on `header` text, the `log` (App.tsx:1620), `layer-stats` (1499) or the `generate` button (1495). None of these survive.

`bridge.ts` is **MIXED**:
- **Keep:** `connectCore`, `installBridge`, `fitView`, `selectNode`.
- **Rework `wire()`:** it confirms a connection by counting `graph.connect: ok` lines in the log (bridge.ts:105-112). It should poll `graph.list` instead.
- **Rework `logCount` and `evaluateAndWait`:** both read the log, and `evaluateAndWait` can also click the `generate` button (bridge.ts:152-169). The evaluate path can keep using `graph-evaluate` (GraphPanel) and confirm by polling the core.

A replacement setup already exists: `parity-capture.spec.ts:89-96` creates the project with `core.callTool("project.create")` before `page.goto`, and the Editor adopts it on mount.

`tileServer.ts` is only used by `geo.spec.ts`, so it goes with it. Studio has its own copy at window.rs:1239.

Only these test IDs survive:
- **GraphPanel:** `graph-panel`, `graph-kind`, `graph-add`, `graph-evaluate`, `graph-live`, `graph-only`, `graph-undo`, `graph-redo`, `graph-time`, `graph-time-value`, `graph-inspector`, `node-preset`, `node-thumb`, `param-*`.
- **LayerStack:** `layer-chain`, `layer-add`, `layer-add-kind`, `layer-eye-*`, `layer-up-*`, `layer-down-*`, `layer-remove-*`, `layer-others`, `layer-search`, `layer-group-*`, `layer-op-*`.

`layer-strip`, `strip-*`, `sim-*`, `action-*` and `layer-pick`/`color-pick` all live in App.tsx and go.

## 1. Spec classification

Studio coverage names functions in `apps/studio/tests/window.rs` unless a file is given. "Gaps" lists the spec lines that have no Studio equivalent.

| Spec | Verdict | Studio coverage / gaps |
|---|---|---|
| actions | DELETE | `actions_and_simulation_layers_add_their_nodes_in_one_click` (Flip X only). **Gaps:** :37 a flip keeps the height range; :39-40 rotate_left; :41-46 Flatten makes min = max; :35/:40 each action shows as a stack row. |
| alphas | DELETE | `sculpt_paints_strokes_and_transforms_and_the_road_lays_a_spline` (plain raise stroke). **Gaps:** :31 seven-alpha palette; :32-33 crater selected; :39-43 the stroke records `alpha: "crater"`. |
| biomes | MIXED | **Keep:** :32-63 rules table in the inspector (add row, name/slope, bad range refused, journal array — replace log checks :41/:56 with journal reads); :68-90 snow node, placeholder, wire, output layer. **Delete:** :92-95 colour pick → `the_colour_picker_draws_a_layer_on_the_terrain_and_back`; :97-104 materials → `the_materials_section_edits_the_stack_and_rebakes_the_maps`. **Gap:** :101 material colour edit triggers a rebake. |
| caves | DELETE | `the_caves_section_adds_shows_x_rays_maps_and_exports_a_cave`. **Gaps, test 1:** :32/:36 kind "mine" (Studio only adds karst); :37 three output layers `cave_holes`/`cave_plan`/`height`; :41 hint mode "cave" (only the unit test `chrome.rs::the_hints_follow_the_armed_tool_like_the_desktop`); :59-61 remove an entrance from the list. **Gaps, test 2:** :83 karst added from the layer strip; :86 speleothems and water in the summary; :95-96 "show caves" off removes them. |
| chrome | DELETE | `the_viewport_chrome_inserts_layers_frames_and_fits` (Fit, Frame), `the_window_shows_the_project_renders_its_terrain_and_orbits` (header, orbit), `the_editors_shortcuts_run_the_toolbar_commands` (⌘S, not the button), unit tests `chrome.rs::the_hints_follow_the_armed_tool_like_the_desktop` and `ruler_steps_and_labels_print_like_the_desktop`. **Gaps:** :26-27 brand mark image; :36-40 ruler max equals the layer max; :55 toolbar has 11 buttons; :56-57 Save button; :69-76 "shore N" readout from a sea-level node with no evaluation; :79-85 hint mode orbit/grid/sculpt in the window (unit test only). |
| climate | MIXED, mostly KEEP | **Keep:** :31-57 rain node, schema placeholders, wire, publish `rain`, evaluate (drop the log check at :58). **Delete:** :59-62 colour picker → `the_colour_picker_draws_a_layer_on_the_terrain_and_back`. |
| desert | MIXED | **Keep:** :42-92 aeolian and karst inspectors, wiring, `sinkholes` output, evaluate. **Delete:** :34-40 → `the_noise_section_generates_tunes_octaves_and_measures`. **Gaps:** :35-36 "Desert plain" preset fills 1200 m and 3 octaves; :37 seed field; :40 starter graph does not overwrite the range (37.4 m). :95-96 colour pick is covered by the colour picker test. **Gaps:** :97-100 switching the layer picker changes the stats; :102-103 wheel zoom. |
| diagnostics | DELETE | `tasks_and_diagnostics_work_before_a_project_is_open`, `settings_commit_on_leaving_language_tasks_and_diagnostics` (Environment button, "host"). **Gaps:** :25 "N cpus · N GB" line; :35-37 cache GC result; :39-42 bench result (S1 256², thermal×50). |
| docking | DELETE | `the_side_panels_collapse_to_a_strip_and_come_back`. **Gaps:** :36 and :52-53 collapsed state survives a reload; :72-79 and :90-92 Preview detail 2048² gives lod 0 / 8 MB / ~8.3 M tris (Studio has the setting at app.rs:2694 but no test). Graph docking beside the viewport (:44-58) no longer applies. |
| gate | KEEP | Graph only: `jd.input.layer`, inspector, `graph-only` (3 options) restricts evaluation. Rewrite :34 (log) and :60-61 (layer picker) as core checks. |
| geo | DELETE | `real_world_fetches_a_dem_and_wires_it_into_the_height`. **Gaps:** :40-42 typed lon/lat/radius; :52 DEM asset file name; :55 origin above 4,000,000; :56-57 attribution source; :60 stats show the fetched heights; :63-65 saved project.json has CRS and attribution; :71-74 second fetch refused with no tile requests; :78-79 ATTRIBUTION.txt in the export. |
| graph | KEEP, with trims | **Keep:** :32-60 starter graph, inspector, unchanged field not journaled, expressions, hover thumbnail; :66-70 add node; :74-98 drag-connect and move; :100 evaluate; :107-129 Delete key, undo/redo, exact journal. **Drop:** :62 the Noise panel mirrors the node (**gap**, Studio does not test it); :64 and :101-102 viewport tris and stats (Studio refresh is covered by `a_write_from_the_editor_refreshes_the_window`); :72 "live preview waiting: input `in`" (**gap**: it is only shown in the log). |
| grid | DELETE | `the_grid_tool_selects_cells_and_levels_raises_and_transforms_them`. **Gaps:** :34 cells per side = 16; :41-46 hovered cell reported; :63-67 right-drag orbits without changing the selection; :70-73 Set height 600 makes the max exactly 600.0 m; :77-79 Raise 25 gives 625; :87/:89 stroke params `target_m`, `radius_m` 32, `strength_m` 25; :91-92 undo reverts a grid operation. |
| hdri | DELETE | Covered by `the_sun_card_writes_the_look_and_toggles_the_maps` (load, hdri readout, Use sun, clear) and `the_export_section_writes_every_kind_of_export` (Bake). No gaps. |
| hydrology | MIXED | **Keep:** :35-83 rivers inspector (0.55 placeholder, method options d8/dinf/mfd), wiring, second output; :104-110 sea-level handles; :119-124 visibility param; :127-131 wind param; :145-170 seabed graph; :185-209 bathymetry graph. **Delete, covered:** :84-88 colour → colour picker test; :91-96 and :101-102 scatter → `the_scatter_section_places_saves_and_exports_instances`. **Delete, gaps:** :112 and :117 sea readout follows `level_m` (only unit test `viewport.rs::the_sea_shades_by_depth_and_draws_its_shoreline` and the ignored `capture_sea`); :132 "waves 9.8 m / 380 m"; :134-136 water export (`ocean.bake`); :99 and :139 wheel zoom; :180 draining the sea; :210-211 layer picker "floor" shows 230.1 m; :217-229 autosave actually runs (journal 0, not dirty, graph.json written). |
| i18n | DELETE | `settings_commit_on_leaving_language_tasks_and_diagnostics` (Kiswahili headings), unit test `i18n.rs::translates_like_the_desktop_and_passes_unknown_strings_through`. **Gaps:** :32-33 language survives a reload; :41-47 an uncaught UI error writes `logs/crash_*.json` with source `ui:window`. The crash handler is Editor-level (App.tsx:403-416) and could be kept, but this spec checks it through the log (:42). |
| ipc.measure | DELETE | A measurement, not a gate. No Studio equivalent. Referenced by docs/spec/performance-targets.md:58 and docs/adr/0003-viewport-hosting.md:29. |
| jobs | MIXED | **Keep:** :26-50 Live toggle off, hydraulic node at 2000 iterations, handle drags. **Delete:** :52-65 the header job and Cancel. Only the job line's format is tested in Studio (`app.rs::the_header_job_line_reads_like_the_desktop`). **Gaps:** :54-57 a running job shows progress and Cancel gives E_CANCELLED; :62-65 Esc aborts (handler at App.tsx:420-428; Studio has an Escape path in app.rs but no test). |
| lakes | MIXED | **Keep:** :32-50 fill node and `lakes` output. **Gap:** :52 "lakes N cells" readout (only unit test `viewport.rs::lakes_stand_on_the_ground_where_the_fill_is_wet`). |
| look | DELETE | Partly covered by the materials, sun card, scatter and export tests (`the_materials_section_edits_the_stack_and_rebakes_the_maps`, `the_sun_card_writes_the_look_and_toggles_the_maps`, `the_scatter_section_places_saves_and_exports_instances`, `the_export_section_writes_every_kind_of_export`). **Gaps:** :40-45 Save writes scene.json with material, sun azimuth 200, spacing 25; :68-77 reopening restores them in the panel; :49 and :53 Unreal profile with materials.json; :55-59 2×2 tiles with tiles.json and `tile_1_1/heightmap.r16`. |
| materials-detail | DELETE | `detail_tiles_blend_over_the_maps_by_their_weights`, `the_materials_section_edits_the_stack_and_rebakes_the_maps` (detail on). **Gaps:** :36 material colour; :41-44 tile field default 8, set to 4, rebakes. :47-52 is a core-only check. |
| network | KEEP | Inspector (mouths default south, slope placeholder, river_order), Remove node, wiring. Drop :59-63 colour (covered by the colour picker test) and rewrite the log checks at :49/:60. |
| octaves | DELETE | `the_noise_section_generates_tunes_octaves_and_measures` (params written). **Gaps:** :44 muting an octave changes the evaluated range; :50-51 strength 0 flattens to 250 m. |
| paint | MIXED | **Keep:** :32-53 wetness node and output layer. **Delete:** :56-60 scatter (covered by the scatter test). **Gaps:** :63-67 Paint density adds a strokes node with clamp 0…1 and arms sculpt; :69-76 a stroke raises the instance count. Studio has the button (app.rs) but no test. |
| parity-capture | DELETE | Its purpose (Editor-vs-Studio viewport parity) ends. The Studio side is `capture_for_docs` and `capture_sea` (both ignored). Their doc comments at window.rs:1687-1690 and 1785-1786 name this spec and need updating. |
| reference | DELETE | `the_reference_section_reads_an_image` (plain PNG, "no GPS"). **Gaps:** :43-46 video metadata gives place, time and sun; :47 Terrain button enabled; :48-54 Use sun sets azimuth and elevation. |
| render | DELETE | `the_post_sliders_darken_the_corners_bloom_and_expose`, `the_camera_row_keys_records_and_plays_a_flythrough`. **Gap:** :39-44 Still writes `exports/stills/*.png` larger than 1 KB (Studio has a Still button, chrome.rs/app.rs:4743, untested). |
| road | DELETE | `sculpt_paints_strokes_and_transforms_and_the_road_lays_a_spline` (road arms, width 12, tool off). **Gaps:** :50-51 exactly one `graph.splice`; :53 height hash changes; :47 the new node is selected in the inspector. |
| sculpt | DELETE | `sculpt_paints_strokes_and_transforms_and_the_road_lays_a_spline` (strokes node, radius 150, Ctrl+wheel ×1.1, rotate, toggle off). **Gaps:** :36 strength field; :46-49 brush position reported on hover; :58 stats change; :61-62 undo removes the stroke; :71-76 points inside the extent; :89-92 Shift+wheel changes strength; :93-95 and :111 right-drag paints nothing; :97-99 and :115 lower mode records negative strength with hardness; :114 lower half of the screen is south. |
| sculptlayer | DELETE | **Not covered at all.** :34-35 layer settings and "whole terrain" area; :46-51 Height scale 0 hides the strokes. The control exists at app.rs:3647. |
| settings | DELETE | `settings_commit_on_leaving_language_tasks_and_diagnostics` (autosave commits on blur). **Gaps:** :32-35 defaults for cache/LOD/memory; :37-48 panel resize and its persistence; :63-66 out-of-range cache refused and the field reverts; :54-55 and :68-72 wind; :74-81 origin and CRS; :90-97 autosave fires after 1 s and project.json holds the settings. |
| simlayers | MIXED | **Keep:** :47-48 `node-preset` "spring thaw" (a GraphPanel control). The `sim-*` setup must become core calls. **Delete:** Snow → `the_viewport_chrome_inserts_layers_frames_and_fits` and `actions_and_simulation_layers_add_their_nodes_in_one_click`. **Gaps:** :51-67 Lava (7 nodes, 6 edges, lava/heat material masks, emissive #ff590d, colour heat); :72-82 Debris and Sand, with the chain ending at aeolian. |
| simulation | MIXED, mostly KEEP | **Keep:** :40-91 debris and lava inspectors, wiring, `heat` output, core heat stats. **Delete:** :92-95 colour picker (covered). |
| stack | KEEP | LayerStack: add, bypass, reorder, remove, side branch, lerp row. Rewrite :29-30 (generate/stats), :51/:69 (log) and :76 (`sim-snow` becomes `graph.add_simulation`). Studio's own stack is covered separately by `the_layer_stack_adds_bypasses_reorders_and_removes`. |
| terrain | DELETE (both tests) | Covered by `the_window_shows_the_project_renders_its_terrain_and_orbits`, `the_noise_section_generates_tunes_octaves_and_measures` (Generate, Measure), `the_sun_card_writes_the_look_and_toggles_the_maps` (PBR toggle), `the_viewport_chrome_inserts_layers_frames_and_fits` (horizon 256), `the_export_section_writes_every_kind_of_export` (EXR). **Gaps, test 1:** :23 Generate disabled with no project; :30-32 New project in the UI writes project.json and shows the header; :34 seed; :43-45 backend, tris and preview MB readout; :55-62 sun elevation slider gives "sun 12°/140°"; :64 sky 512×256; :66-72 clouds 60 %; :80 fog readout; :81 turbidity 3; :98-103 PNG16 export with W_EXPORT_LOSS and 16-bit depth; :109-130 EXR import becomes a heightmap node and the stats match. **Gap, test 2:** :135-143 E_PERMISSION outside the workspace shown with a next step. |
| timeline | MIXED | **Keep:** :32-55 and :62-74 time slider, keyframed `0:1, 1:300` (⏱2 keys), set_time, undo. Read the hash at :56-58/:70 from core `terrain.stats` instead of the panel. **Delete:** :94-99 frames export → `the_export_section_writes_every_kind_of_export`. **Gaps:** :75-92 Grow with timeline (0 instances at t = 0, more than 100 at t = 1); :100 export leaves the time at 0. |
| transforms | DELETE | Duplicate (grid test) and rotate brush (sculpt test). **Gaps:** :43-49 grid Scale 0, Rotate, Align; :50 transforms row in the stack; :56-63 push brush and its "grab" hint. |
| tree | MIXED | **Keep:** :32-56 LayerStack search, groups, collapse, simulation group. The `strip-*` clicks become core calls. **Delete:** the strip itself; terrace and snow are covered by `the_viewport_chrome_inserts_layers_frames_and_fits`. **Gaps:** :30 strip has 11 buttons; :35-38 cone becomes a removable shape layer. |

## 2. Vitest files that test removed code

| File | Status |
|---|---|
| `src/App.test.tsx` | 8 tests, all driving App panels. Tests at :125, :157, :219, :238, :305 and :317 go. The test at :268 (selection sync with Studio, graph reload, no double evaluation) and the graph part of :200 are Editor-level and should be rewritten to render the slimmed App. |
| `src/viewport/sea.test.ts` | Imports `buildFrame`, `buildSeaGeometry`, `buildShoreline`, `gerstner`, `waterColour` from TerrainView. Delete. |
| `src/lib/camera.test.ts` | `orbitAt` is used by state.ts flythrough only, so it goes with the viewport. |
| `src/lib/previewLod.test.ts` | `adaptLodBias` is the viewport preview LOD (state.ts:56). Delete; Studio covers it with `app.rs::the_live_target_moves_the_preview_lod_like_the_desktop`. |
| `src/lib/i18n.test.ts` | i18n is imported only by App.tsx:11 and state.ts:55; GraphPanel does not use it. Keep only if the Editor's graph gets translated. |
| `src/lib/api.test.ts` | `decodePreview` is still used by GraphPanel's node thumbnails (GraphPanel.tsx:21). Keep. |
| `src/lib/expr.test.ts`, `src/components/Range.test.tsx` | Both used by GraphPanel (:22, :24). Keep. |

## 3. Scripts, docs and CI that reference the specs

- **CI:** `.github/workflows/ci.yml:158-168` installs Chromium, runs `pnpm test:e2e` (the whole folder) and uploads `apps/desktop/test-results`.
- **Pre-push:** `scripts/pre-push.sh:22` runs `pnpm test:e2e`.
- **package.json:** `apps/desktop/package.json:12` (`test:e2e`), `:13` (`annotate`).
- **Playwright config:** `apps/desktop/playwright.config.ts` (`JD_VIDEO` recordings, 1440×900).
- **Spec named directly:** docs/spec/performance-targets.md:58 and docs/adr/0003-viewport-hosting.md:29 (`ipc.measure`); docs/user/cli-quickstart.md:367 (`timeline`).
- **docs/user/desktop-app.md**, by line:
  - 88 docking, settings
  - 108 hydrology
  - 114 render
  - 116 hdri
  - 120 and 122 settings
  - 126 graph and JD_VIDEO
  - 130 and 134 stack, tree
  - 140 diagnostics
  - 146 terrain
  - 154 terrain and JD_VIDEO
  - 160 i18n
  - 164 octaves
  - 166 actions
  - 168 jobs
  - 170 chrome
  - 174 lakes
  - 182 hydrology
  - 202 hydrology and JD_VIDEO
  - 208 paint
  - 212 hydrology
  - 218 and 280 biomes
  - 232 network
  - 236 gate
  - 246 reference
  - 258 road
  - 266 caves
  - 274 simlayers
  - 288 and 294 timeline
  - 300 desert
  - 302 JD_VIDEO
  - 314 sculpt
  - 316 sculptlayer
  - 318 alphas
  - 326 grid
  - 329 transforms
  - 336 look
  - the test table at 343-348 (terrain, graph, grid, sculpt, simlayers, look, stack, reference)
  - `annotate.mjs` at 78 and 350
- **Studio:** window.rs:1687-1690 and 1785-1786 name `e2e/parity-capture.spec.ts` and `JD_PARITY_*`.
- **Figma tooling:** `scripts/figma/README.md` and `build_pages.py` describe `annotate.mjs` (not run by any gate).
- **Plan:** docs/research/research-log/2026-10-27-zed-figma-ui.md:55-58 (this retirement is P5; "S4 = docs and the guide figures recaptured").

## 4. How the user-guide figures are made

I could not find a count of 95 anywhere. The repo has 101 PNGs in `docs/user/images/` and 99 unique images embedded in the guides: desktop-app.md 43, cli-quickstart.md 55, environments.md 5, blender.md 1, with some overlap.

**Desktop-app figures come from Playwright screenshots of the Editor's viewport and panels:**
1. Specs save them with `page.screenshot({ path: testInfo.outputPath("<name>.png") })`, each marked "tutorial image source", into `apps/desktop/test-results/<test-dir>/`.
2. They are copied by hand to `docs/user/images/desktop-*.png`. No script writes into `docs/user/images`.
3. 24 have `.annot.json` files rendered by `pnpm annotate` (`apps/desktop/scripts/annotate.mjs`, a Chrome overlay).

Confirmed links from `.annot.json` `src` paths:
- `desktop-materials` ← biomes
- `desktop-desert` ← desert
- `desktop-network` ← network
- `desktop-timeline-growth` ← timeline
- `desktop-gate` ← gate
- `tutorial-03-graph` ← `desktop-graph.png`
- `tutorial-04-sculpt` ← `desktop-sculpt.png`
- Others start from raws in `/tmp/jd-scratch`: docking, geo, materials-detail.

The remaining links are inferred from file names (e.g. pbr/horizon/import ← terrain; rivers/scatter/sea/seabed/bathymetry ← hydrology). The menu bar and icon images are native macOS screenshots, and the videos are `JD_VIDEO=1` recordings.

**CLI and environment figures** are core renders annotated from `/tmp/jd-scratch` or `target/scratch` raws, with no UI involved.

**Studio has no per-feature figure pipeline yet.** The only image producers are `capture_for_docs` and `capture_sea` (`JD_STUDIO_CAPTURE` / `JD_STUDIO_*`), which render one fixed project. Once the specs that take the screenshots are deleted, the ~40 desktop-app figures have no source until Studio capture steps are added (slice S4).

---

# Gap coverage outside the e2e folder (second agent, 2026-10-27)

I checked all 38 specs. 14 of them list gaps. The spec gaps break down like this:

- **About half are UI-only.** They cover readouts, field defaults, hover, persistence of the layout and language, and how strokes and ops get built.
- **About 22 are covered by core tests.**
- **11 are uncovered engine behaviour, or covered only at the kernel level.** These are listed at the end.

`scripts/acceptance.mjs` counts as a gate. CI runs it (`ci.yml:157`), any op that fails throws, and the script exits 1 on a FAILED result. Unless a path is given, `ops.rs` means `crates/jd-core/src/ops.rs`.

## Per spec

**actions**
- :37 a flip keeps the height range → COVERED-CORE (partial) → `masks.rs:273 actions_flip_turn_and_flatten_with_row_zero_south`. It asserts the mirrored cell `fx(0,0)=3`, not min and max.
- :39-40 rotate_left → COVERED-CORE → same test (`l.get(3,3)`, and right undoes left).
- :41-46 Flatten makes min = max → COVERED-CORE → same test, around :291 (every cell equals the mean).
- :35/:40 each action shows as a stack row → UI-ONLY.

**alphas**
- :31 seven-alpha palette; :32-33 crater selected → UI-ONLY.
- :39-43 the stroke records `alpha:"crater"` → UI-ONLY (the UI builds the stroke). The crater kernel is covered by `sculpt.rs:578 alphas_pattern_the_stroke_crater_raises_a_rim_round_a_bowl`.

**biomes**
- :101 a material colour edit triggers a rebake → UI-ONLY. Blending is covered by `mesh.rs:2118`.

**caves**
- :32/:36 kind "mine" → COVERED-CORE (partial). `ops.rs:6760 workflow_ops_splice_simulate_and_cave_as_the_desktop_did` adds karst and maze only. The mine kernel is `cavegen.rs:2552`.
- :37 outputs `cave_holes`/`cave_plan` → COVERED-CORE → `ops.rs:6950`.
- :41 hint mode "cave"; :59-61 remove an entrance (a plain set_param); :83 karst from the strip; :86 summary readout; :95-96 show-caves off → UI-ONLY. The speleothem and water kernels are `caves.rs:1419` and `cavegen.rs:2229`.

**chrome**
- :26-27, :36-40, :55, :56-57, :69-76, :79-85 → all UI-ONLY. Save itself is covered by `project.rs:998`.

**desert**
- :35-36 the preset is a UI table (`App.tsx:99`, `app.rs:335`); :37 the seed field; :40 Generate pushes the panel into the noise node (`state.ts:1307`, `app.rs:1806`) → all UI-ONLY. The range is respected by `noise.rs:517`.
- :97-100 the layer picker; :102-103 wheel zoom → UI-ONLY.

**diagnostics**
- :25 the cpus/GB line → UI-ONLY. Engine values are checked by `diag.rs:124 env_info_reports_sane_values` and `cli.rs:387`.
- :35-37 cache GC → UNCOVERED-ENGINE. No test calls `project.cache_gc`. The kernel is covered by `jd-cache/src/lib.rs:382`.
- :39-42 bench → COVERED-CORE → `diag.rs:103 bench_s1_runs_small_and_is_deterministic`, plus the CI bench step (`ci.yml:104`). The stage names are not asserted.

**docking**
- :36, :52-53 collapse survives a reload; :72-79, :90-92 preview px → lod → UI-ONLY. The LOD bias is covered by `app.rs:6249`.

**geo**
- :40-42 typed lon/lat/radius → UI-ONLY. The centre/radius path is only exercised for `fetch_features` (`geo.rs:637`).
- :52 asset name → COVERED-CORE → `geo.rs:531`.
- :55 origin > 4,000,000 → COVERED-CORE (indirect). `geo.rs:511` feeds `res.origin` into set_settings, and the heights only land at 6270–6290 when the origin is right (:624). The number itself is not asserted.
- :56-57 attribution source → COVERED-CORE (partial). It checks the count and that the text holds the registry URL (`geo.rs:543-550`), not the `source` field.
- :60 stats show the fetched heights → COVERED-CORE → `geo.rs:624`.
- :63-65 project.json:
  - Attribution survives save and reopen → COVERED-CORE → `geo.rs:658`.
  - CRS on disk → UNCOVERED-ENGINE (see the list below).
- :71-74 second fetch refused → COVERED-CORE (partial) → `geo.rs:573`. It asserts an Error status only, not `E_INPUT_INVALID`. "No requests" is not asserted, though the check runs before the fetcher (`geo.rs:320`).
- :78-79 ATTRIBUTION.txt → COVERED-CORE → `geo.rs:631`.

**graph**
- :62 the Noise panel mirrors the node; :72 the "live preview waiting" log line → UI-ONLY. Missing-input errors are covered by `ops.rs:4078` and `nodes.rs:2562`.

**grid**
- :34, :41-46, :63-67, :87/:89 (the UI builds the stroke params) → UI-ONLY.
- :70-73 Set height gives exactly 600 → COVERED-CORE → `sculpt.rs:868 hard_square_stamps_level_their_cells_to_a_target_and_nothing_between_them`.
- :77-79 Raise 25 gives 625 → COVERED-CORE (partial). A round raise is `sculpt.rs:775`; a square hard raise is not tested.
- :91-92 undo of a grid op → COVERED-CORE (generic). Every `graph.*` mutate records history (`ops.rs:869-886`), and undo is tested via move at `ops.rs:4249`.

**hydrology**
- :112, :117 sea readout; :99/:139 zoom; :180 drain readout → UI-ONLY.
- :132 waves 9.8 m / 380 m → COVERED-CORE → `ocean.rs:689`, assertions at :721 and :723.
- :134-136 water export → COVERED-CORE (partial). `acceptance.mjs:203` runs `ocean.bake` and requires status ok, but does not check for `wave_normal.png`. The kernel is `ocean.rs:1132`.
- :210-211 floor 230.1 m → COVERED-CORE (behaviour, not the number) → `bathymetry.rs:314`, and `acceptance.mjs:170-172` (floor mean vs height mean).
- :217-229 autosave: the timer is UI-ONLY. Save truncating the journal is covered by `project.rs:998`. graph.json after recover is checked by `acceptance.mjs:444-449`.

**i18n**
- :32-33 language survives a reload → UI-ONLY.
- :41-47 crash report: the `ui:window` handler is UI-ONLY. The engine's `diag.crash_report` is covered by `apps/studio/tests/crash.rs:8` (source `ui:studio`) and `ops.rs:7045`.

**jobs**
- :54-57 the progress and Cancel UI is UI-ONLY. The engine cancel to `E_CANCELLED` is covered by `ops.rs:4599`, assertion at :4676.
- :62-65 Esc aborts → UI-ONLY.

**lakes**
- :52 the "lakes N cells" readout → UI-ONLY. The fill is covered by `flow.rs:593`.

**look**
- :40-45 Save writes scene.json → UNCOVERED-ENGINE. `ops.rs:3176` only checks `project.info` in memory.
- :49/:53 Unreal profile with materials.json → COVERED-CORE (partial). `profiles.rs:685` tests materials.json on the Unity profile; Unreal is tested without materials (`profiles.rs:779`).
- :55-59 tiles.json and `tile_1_1/heightmap.r16` → COVERED-CORE (partial). `profiles.rs:624` covers tiles.json with Unreal's `heightmap.png`. Nothing tests the Unity tiles or the op-level `tiles` param.
- :68-77 reopening restores the look:
  - The panel half is UI-ONLY.
  - The engine half (`project.open` returning the saved look) is UNCOVERED-ENGINE.

**materials-detail**
- :36; :41-44 (tile default 8 and rebake) → UI-ONLY. The detail tile is covered by `mesh.rs:1975` and `detail.rs:296`.

**octaves**
- :44 muting an octave changes the range → COVERED-CORE → `noise.rs:333 octave_weights_of_one_match_the_plain_fbm_and_zeros_leave_the_first_octave`.
- :50-51 strength 0 → mid height → COVERED-CORE → same test, :393.

**paint**
- :63-67 Paint density adds a clamped strokes node and arms sculpt:
  - Arming sculpt is UI-ONLY.
  - `graph.sculpt_target {layer, clamp}` and the strokes-node `clamp` are UNCOVERED-ENGINE. Only `{kind}` is tested (`ops.rs:6838`).
- :69-76 a stroke raises the instance count → COVERED-CORE by parts only (`sculpt.rs:775`, `scatter.rs:581`).

**reference**
- :43-46 video metadata gives place, time and sun → COVERED-CORE → `jd-media/src/lib.rs:1279 a_360_video_gives_duration_place_and_projection`, and :1015 for the sun.
- :47; :48-54 → UI-ONLY.

**render**
- :39-44 Still:
  - The frame capture is UI-ONLY.
  - `project.write_export` (`ops.rs:5904`, confined to exports/) is UNCOVERED-ENGINE: no test.

**road**
- :47; :50-51 (one splice is client behaviour) → UI-ONLY.
- :53 the hash changes → COVERED-CORE → `spline.rs:231`.

**sculpt**
- :36, :46-49, :71-76, :89-92, :93-95/:111, :97-99/:115 (the UI records the stroke), :114 → UI-ONLY.
- :58 stats change → COVERED-CORE → `sculpt.rs:775`, `nodes.rs:2872`.
- :61-62 undo → COVERED-CORE (generic, as for grid :91-92).

**sculptlayer**
- :34-35 → UI-ONLY.
- :46-51 Height scale 0 hides the strokes → COVERED-CORE → `sculpt.rs:510`, assertion at :538.

**settings**
- :32-35 defaults → UI-ONLY. Only the cache default is asserted, in `ops.rs:3786`.
- :37-48 → UI-ONLY.
- :63-66 cache refused → COVERED-CORE → `ops.rs:3786` (the field revert is UI).
- :54-55/:68-72 wind → COVERED-CORE → `ops.rs:2938`.
- :74-81 origin/CRS → COVERED-CORE → `ops.rs:3022`.
- :90-97 autosave:
  - The timer is UI-ONLY.
  - project.json holding the settings and world is UNCOVERED-ENGINE (see the list).

**simlayers**
- :51-67 Lava → COVERED-CORE → `ops.rs:6760`:
  - published `[heat, lava]`;
  - heat emissive G 0.35 (:6940);
  - 3 look materials (:6944);
  - evaluate ok.

  The glow bake is `mesh.rs:2161` and the lava kernel `lava.rs:509`. The 7/6 node and edge counts are not asserted.
- :72-82 Debris and Sand → UNCOVERED-ENGINE.

**terrain**
- :23 → UI-ONLY. The engine refusal with no project is `ops.rs:6659`.
- :30-32 → UI-ONLY. Create is covered by `project.rs:925`.
- :34, :43-45, :55-62, :64, :66-72, :80, :81 → UI-ONLY. Engine coverage for the sky: `sky.rs:564/645/706`, `acceptance.mjs:205`.
- :98-103 PNG16 export → COVERED-CORE → `cli.rs:272` (W_EXPORT_LOSS at :368, bit depth 16 at :375) and `ops.rs:6702`.
- :109-130 EXR import → COVERED-CORE → `export.rs:748` (EXR export then `import_heightmap`, exact) and `ops.rs:4474` (import_asset → heightmap node, PNG16). The node is added by the client.
- :135-143 E_PERMISSION outside the workspace → COVERED-CORE → `apps/mcp/tests/stdio.rs:95-103`, through project.open; the e2e used create. The next-step text in `main.rs:170` is not asserted.

**timeline**
- :75-92 Grow with timeline → COVERED-CORE (kernel) → `scatter.rs:443`. Op-level `follow_time` is uncovered.
- :100 export leaves the time at 0 → COVERED-CORE → `ops.rs:3091`, assertion at :3158.

**transforms**
- :43-49 Scale 0, Rotate, Align → COVERED-CORE → `transform.rs:288/305/355` (kernel; the UI builds the ops).
- :50; :56-63 the hint and push construction → UI-ONLY. The push kernel is `transform.rs:355`.

**tree**
- :30; :35-38 → UI-ONLY. The primitive kernel is `stamp.rs:469`.

## UI-ONLY gaps
actions :35/:40 · alphas :31, :32-33, :39-43 · biomes :101 · caves :41, :59-61, :83, :86, :95-96 · chrome (all) · desert :35-36, :37, :40, :97-100, :102-103 · diagnostics :25 · docking (all) · geo :40-42 · graph :62, :72 · grid :34, :41-46, :63-67, :87/:89 · hydrology :112/:117, :99/:139, :180, the autosave timer in :217-229 · i18n :32-33, the `ui:window` handler in :41-47 · jobs :54-57 (the UI half), :62-65 · lakes :52 · look :68-77 (the panel half) · materials-detail :36, :41-44 · paint :63-67 (arming sculpt) · reference :47, :48-54 · render :39-44 (the capture) · road :47, :50-51 · sculpt :36, :46-49, :71-76, :89-92, :93-95/:111, :97-99/:115, :114 · sculptlayer :34-35 · settings :32-35, :37-48, the timer in :90-97 · terrain :23, :30-32, :34, :43-45, :55-62, :64, :66-72, :80, :81 · transforms :50, :56-63 · tree :30, :35-38

## UNCOVERED-ENGINE gaps (need a core test)
1. **look :40-45 and :68-77** (engine half): `project.set_look` → save → `scene.json` holds the materials, `sun.azimuth_deg` and `scatter.spacing_m`, and `project.open` returns them. No test reads scene.json back or reopens the look.
2. **simlayers :72-82**: `graph.add_simulation` with `debris` and `sand`. That means:
   - splicing in front of the output;
   - debris publishing its mask and material (the `Simulation::Debris` arm, `ops.rs:1894`);
   - the chain ending at aeolian.

   Only snow and lava are tested.
3. **paint :63-67**: `graph.sculpt_target {layer, clamp}`, which should create a clamped strokes node in front of the mask's output, and the strokes-node `clamp` param (`nodes.rs:567`).
4. **diagnostics :35-37**: the `project.cache_gc` op result (blobs, bytes, evicted, limit). The kernel is covered.
5. **render :39-44**: `project.write_export`. Nothing tests that it writes under exports/ or refuses absolute and `..` paths.
6. **geo :63-65 and settings :90-97**: no test reads `world.crs`, `origin`, `wind` or `settings.*` back from project.json after a save or reopen. The comment at `ops.rs:3071` says "Persisted through save → reopen", but that test never reopens. Risk is low because project.json is written whole (`project.rs:725`).
7. **timeline :75-92** (op level): the `follow_time` flag in `terrain.scatter`, which reads `graph.time` (`ops.rs:6429`). Only the kernel is tested.

Weak spots worth tightening while you're there:
- geo :71-74 asserts an Error status, not `E_INPUT_INVALID`.
- geo :56-57 never checks the attribution `source` field.
- look :55-59 has no op-level `tiles` test and no Unity tile files.
- caves :32/:36 never calls `add_cave` with `mine`.
- hydrology :134-136 doesn't assert the output files, only that `ocean.bake` returns ok.