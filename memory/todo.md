# TODO — Backlog and Milestones

Conventions: `- [ ]` planned · `- [x]` done · `- [~]` in progress (add owner) · `- [e]` experimental · ~~struck~~ changed requirement, with reason and replacement below it. Keep phases aligned with `docs/roadmap.md`.

## Round 1 — Research & single source of truth (2026-10-19)
- [x] `VERSION` file (0.0.1) mirrored into every manifest, checked in CI (`scripts/check-version.mjs`) and at compile time (`jd-core`); one edition, no tiers (ADR-0013)
- [x] Archive brief → `docs/research/brief/`
- [x] `memory/` files: agent_profile, session_start, philosophy, project_context, porting, mindmap, instruction, todo, handover_session
- [x] Architecture docs 00–13 with Mermaid (29 diagrams validated by `scripts/check-mermaid.mjs`)
- [x] Research reports in `docs/research/`: world-creator, gaea-world-machine, opensource-rea, formats-sdks, stack, algorithms, geo-vision
- [x] `docs/research/materials-geomorphology.md` (texture libraries, soil/rock spectra, environmental shaders, geomorphic processes, GIS software licences) + reference-layer architecture wired into 02/04
- [x] `competitive-matrix.md`, `licensing-register.md`, `evidence-register.md`
- [x] ADRs 0001–0012 (0003, 0004, 0012 Proposed; others Accepted)
- [x] Specs: project-format, json-api, error-codes, performance-targets, interoperability-matrix
- [x] `docs/README.md` index; README, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT
- [x] Mermaid + link validation scripts; CI workflow (docs job on macOS/Ubuntu/Windows)
- [ ] Commit + push to `SeedeXR/jenga-dunia` main
- [ ] Research follow-ups listed in `docs/research/evidence-register.md`
- [x] Installer-surface inventory of `installables/` (WM Basic 4067, Gaea 2.3.1.0) → `docs/research/research-log/2026-10-19-installer-inventory.md`
- [x] CI: `actions/checkout@v5` + `actions/setup-node@v5` (Node 24); `~/.npm/_npx` cached (`actions/cache@v4`)
- [~] Phase 2 backlog additions from WM inventory: undo ✔ (cap 100 snapshots; paging/archive ☐); live preview at ≤ 512² preview LOD ✔ (target frame-time setting ☐)

## Phase 1 — Foundation (in progress, Session 2)
- [x] Cargo workspace: `jd-types`, `jd-core`, `apps/cli` (`jd`), `apps/mcp` (`jd-mcp`) — real content, 28 tests (unit/property/CLI-driven/MCP-driven)
- [x] Project format v1: directory layout, atomic writes, unknown-key preservation, schema-version gate, write-ahead journal + crash recovery (`project.recover`), pid lock with stale detection
- [x] Operation registry with schemars JSON schemas; snapshot test of all op schemas (`crates/jd-core/src/snapshots/`)
- [~] Generated schemas committed under `schemas/v1/` ✔ (`jd schema --write`, CI fails when stale; 24 ops × request/result + envelope + node kinds); TS type generation ✔ (`apps/desktop/scripts/gen-types.mjs` → `src/lib/ops.gen.ts`, CI-checked; `state.ts` `run()` typed by the `JdOps` map and strict TS on ✔)
- [x] CLI `jd new|open|op|schema|diag env` with `--json` NDJSON envelopes and spec exit codes
- [x] MCP server over stdio: tools from registry, scope gate (`--scope`), `--workspace` confinement, structured envelopes
- [x] Autosave: debounced `project.save` 2 s after the last journaled edit (desktop; e2e-verified); explicit interval preference ☐
- [~] jobs: progress + cancel ✔ (`jd-core::jobs`, `jobs.list`/`jobs.cancel`, header progress + Cancel, E_CANCELLED in kernels); queues/budgets/timeouts/detached execution ☐ · `jd-cache` ✔ (layer-level; tile-level/LRU ☐)
- [~] Viewport feasibility prototype: three.js WebGPU/WebGL2 in Tauri WebView ✔ (WebGPU observed active on macOS 27 dev build), u16 tile streaming over raw-bytes IPC ✔; measure IPC MB/s + latency on macOS and Windows ☐; Rust headless wgpu compute ☐; close ADR-0003 ☐
- [x] Desktop app skeleton (`apps/desktop`, `jd-app`): project panel, noise inspector, three.js viewport, export, log; vitest component tests; Playwright e2e bridged to the real core via jd-mcp
- [~] Desktop: native file dialogs (tauri-plugin-dialog) ✔ (native check pending — display was locked), macOS menu bar + ⌘ shortcuts ✔, `.jdproj` launch-arg open ✔ / bundle file association ✔ (unsigned `.app`+`.dmg`, Apple-Event open verified) / `LSTypeIsPackage` + exported UTI ✔ (plain `open X.jdproj` launches the app), window state persistence ✔ (verified natively)
- [~] Desktop tutorials: native annotated screenshots ✔ (tutorial-01/02/03 re-shot from the v0.0.1 build 2026-10-20; 04 sculpt is the e2e capture — clicks need Accessibility), native recording ✔ (graph-erosion-native.mp4) + first-terrain.mp4, Playwright-recorded graph-editor.mp4 ✔; Edit-menu capture ☐ (System Events click did not open it); place in the Figma file once the connector OAuth is completed by the owner ☐
- [x] CI: Rust matrix (macOS, Ubuntu, Windows) fmt/clippy/test/release-smoke + cargo-deny job; docs job with npx cache
- [ ] S1 bench smoke in CI (needs a kernel)
- [ ] Fixtures directory with malformed-file corpus seed
- [ ] `graphify` knowledge graph generation added to session-end checklist and CI (deterministic output)
- [ ] Platform-native checklist (owner mandate): macOS menu bar/⌘ shortcuts/file association for `.jdproj`, Metal via wgpu; Windows DX12/NVIDIA; Linux Vulkan/NVIDIA; CPU path everywhere

## Phase 2 — Core terrain authoring
- [~] `jd-terrain`: tiles + halos ✔, noise generators ✔ (OpenSimplex2/2S, Perlin, Value, ValueCubic, Cellular × none/fbm/ridged/ping-pong), filters/analysis nodes: blur ✔, slope mask ✔ (`jd.analysis.slope@1`); terrace/curvature/flow ☐
- [x] Ops `terrain.generate` / `terrain.stats` / `terrain.export_heightmap` + CLI `jd terrain generate --out` / `jd export heightmap`; MCP end-to-end test
- [~] `jd-graph`: typed DAG ✔, validation (types/cycles/required/unknown) ✔, blake3 content-hash incremental evaluation + in-memory cache ✔, LOD ✔, bypass ✔, 7 node kinds ✔, journaled `graph.*` ops with pre-journal structural validation (trial copy + `validate_structure`) ✔, `graph.move_node` ✔, lod>0 → `<layer>#lod<N>` ✔, node-editor UI (React Flow: add/connect/drag/delete/inspector, e2e-tested incl. enum selects and rounded f32 defaults) ✔; disk cache (`jd-cache`) layer-level ✔ (CLI can export a graph result across invocations); tile-level blobs + SQLite index + LRU eviction ☐, region/tile-level dirty sets ☐, progress/cancel ✔ (async detached jobs ☐), undo/redo ✔ (snapshot history, journaled `graph.restore`)
- [~] Heightmap export: EXR f32 ✔, PNG16 ✔, RAW r16/r32 ✔ (+ JSON sidecar, flip + range conventions, round-trip tests); TIFF f32 ✔ (`tiff32`); foreign heightmap import ✔ (`jd.input.heightmap@1` + `project.import_asset`; GeoTIFF pixel scale / tie point / nodata ✔, nodata filled with the floor; CRS reprojection ☐ → `jd-geo`); Unity/Unreal size presets (2ⁿ+1) ✔ (project panel preset picker)
- [~] Stroke sculpt nodes: `jd.sculpt.strokes@1` ✔ (smooth-falloff raise/lower polylines), desktop Sculpt mode ✔ (raycast strokes, brush panel, one undo step per stroke); brush ring cursor ✔; smooth/flatten brushes ✔; per-stroke bbox skip + summed-area smooth (2048², 600 m smooth: 70.5 s → 0.68 s debug) ✔; stroke files under `strokes/` ☐
- [~] GLB export ✔ (`terrain.export_mesh` / `jd export mesh`: grid mesh, normals, UVs, flat PBR material; Khronos validator 0 errors) + glTF Validator in CI ✔; textured PBR material ☐
- [ ] Node editor UI, generated inspectors, task monitor, diagnostics console
- [ ] Viewport terrain LOD + PBR + sky

## Phase 3 — Simulation
- [~] Thermal ✔ (CPU Jacobi, `jd.erosion.thermal@1`, mass-conserving; GPU/tile-local ☐) · hydraulic (grid/virtual pipes) ✔ (`jd.erosion.hydraulic@1`, CPU, closed boundary, concentration cap ✔; drains/layers/GPU ☐) · hydraulic (particle) ☐ · stream-power ☐ · wind ☐ · coastal ☐ · snow ☐
- [~] Hydrology: D8 flow accumulation ✔ (`jd.analysis.flow@1`), priority-flood fill ✔ (`jd.hydrology.fill@1`, outputs drainable field + `lakes`), rivers ✔ (`jd.hydrology.rivers@1`: fill → D8 accumulation → smoothstep incision, `rivers` mask); TWI ✔ (`jd.hydrology.wetness@1`); D∞ ✔ (`method: dinf` on flow + rivers nodes; wetness D8-only); sea level ✔ (`jd.hydrology.sea_level@1`: border-connected `ocean` + `depth`); viewport sea plane ✔ (follows the sea_level node); MFD, hydrology-first generation ☐
- [ ] Biomes + scatter + instance export
- [ ] Weather state, timeline, water/ocean viewport
- [ ] Material graph + baking

## Phase 4 — Interoperability
- [ ] USD/USDA/USDC/USDZ writer + usdchecker; OBJ/STL/PLY; GeoTIFF
- [ ] QEM decimation, LOD chains, MikkTSpace, topology validation + stats
- [ ] Material translation layer + loss reports
- [ ] Blender bridge add-on (separate licence) + headless round-trip test
- [ ] Unity/Unreal export profiles (heightmap sizes, weightmaps, z-scale) + documented import procedures
- [ ] FBX SDK export adapter (out-of-process) for fbx we build our own high performance accurate reader and writer from sratch you can learn from optima it's in Software Projects , but also you can verify against blender headless , api , autodesk fbx  , maya , unity and unreal we build our own fbx dsk reader

## Phase 5 — Scale & hardening
- [ ] Streaming, origin rebasing, 16k+ worlds under 4 GB
- [ ] GPU kernels + parity tests
- [ ] `jd-geo` (DEM/OSM/contours with attribution) and `jd-vision` (MoGe-2, DA3 via ONNX; licence gate)
- [ ] Fuzzing, signed releases ( no signed releases since we don't have an apple developer account.), diagnostic bundles
- [ ] Acceptance scenario §20 green

## Deferred (with reasons)
- OpenVDB adapter — no justified volumetric use case before the SDF/voxel module exists (brief §9.1 "where justified").
- In-process `cdylib` plugins — out-of-process covers licensing isolation; ABI stability cost not justified yet.
- Shared daemon for agents to drive a running GUI — single-process adapters first.
