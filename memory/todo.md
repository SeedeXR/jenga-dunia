# TODO — Backlog and Milestones

Conventions: `- [ ]` planned · `- [x]` done · `- [~]` in progress (add owner) · `- [e]` experimental · ~~struck~~ changed requirement, with reason and replacement below it. Keep phases aligned with `docs/roadmap.md`.

## Round 1 — Research & single source of truth (2026-10-19)
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
- [ ] CI: bump `actions/checkout` and `actions/setup-node` off Node 20 (deprecation warning in run 2); cache `~/.npm/_npx` for mermaid-cli
- [ ] Phase 2 backlog additions from WM inventory: undo/history paging with archive cap; preview target frame time setting

## Phase 1 — Foundation (in progress, Session 2)
- [x] Cargo workspace: `jd-types`, `jd-core`, `apps/cli` (`jd`), `apps/mcp` (`jd-mcp`) — real content, 28 tests (unit/property/CLI-driven/MCP-driven)
- [x] Project format v1: directory layout, atomic writes, unknown-key preservation, schema-version gate, write-ahead journal + crash recovery (`project.recover`), pid lock with stale detection
- [x] Operation registry with schemars JSON schemas; snapshot test of all op schemas (`crates/jd-core/src/snapshots/`)
- [ ] Commit generated schemas under `schemas/v1/` (currently only in the insta snapshot) + TS type generation
- [x] CLI `jd new|open|op|schema|diag env` with `--json` NDJSON envelopes and spec exit codes
- [x] MCP server over stdio: tools from registry, scope gate (`--scope`), `--workspace` confinement, structured envelopes
- [ ] Autosave timer + `project.save` on job completion (needs jobs)
- [ ] `jd-jobs` + `jd-cache` crates (with the first real long-running op, Phase 2 `graph.evaluate`)
- [~] Viewport feasibility prototype: three.js WebGPU/WebGL2 in Tauri WebView ✔ (WebGPU observed active on macOS 27 dev build), u16 tile streaming over raw-bytes IPC ✔; measure IPC MB/s + latency on macOS and Windows ☐; Rust headless wgpu compute ☐; close ADR-0003 ☐
- [x] Desktop app skeleton (`apps/desktop`, `jd-app`): project panel, noise inspector, three.js viewport, export, log; vitest component tests; Playwright e2e bridged to the real core via jd-mcp
- [ ] Desktop: native file dialogs (tauri-plugin-dialog), macOS menu bar + ⌘ shortcuts, `.jdproj` open-with, window state persistence
- [ ] Desktop tutorials: Figma-annotated screenshots + recordings (needs Accessibility permission for input automation, or record manually)
- [x] CI: Rust matrix (macOS, Ubuntu, Windows) fmt/clippy/test/release-smoke + cargo-deny job; docs job with npx cache
- [ ] S1 bench smoke in CI (needs a kernel)
- [ ] Fixtures directory with malformed-file corpus seed
- [ ] `graphify` knowledge graph generation added to session-end checklist and CI (deterministic output)
- [ ] Platform-native checklist (owner mandate): macOS menu bar/⌘ shortcuts/file association for `.jdproj`, Metal via wgpu; Windows DX12/NVIDIA; Linux Vulkan/NVIDIA; CPU path everywhere

## Phase 2 — Core terrain authoring
- [~] `jd-terrain`: tiles + halos ✔, noise generators ✔ (OpenSimplex2/2S, Perlin, Value, ValueCubic, Cellular × none/fbm/ridged/ping-pong), filters/analysis nodes ☐
- [x] Ops `terrain.generate` / `terrain.stats` / `terrain.export_heightmap` + CLI `jd terrain generate --out` / `jd export heightmap`; MCP end-to-end test
- [ ] `jd-graph` evaluator, cache, dirty sets, preview/build LOD
- [~] Heightmap export: EXR f32 ✔, PNG16 ✔, RAW r16/r32 ✔ (+ JSON sidecar, flip + range conventions, round-trip tests); TIFF ☐; foreign heightmap import without sidecar ☐; Unity/Unreal size presets (2ⁿ+1) ☐
- [ ] Stroke sculpt nodes
- [ ] Basic PBR material + GLB export + glTF Validator in CI
- [ ] Node editor UI, generated inspectors, task monitor, diagnostics console
- [ ] Viewport terrain LOD + PBR + sky

## Phase 3 — Simulation
- [ ] Hydraulic (particle), hydraulic (grid), stream-power, thermal, wind, coastal, snow
- [ ] Hydrology nodes; per-basin lakes
- [ ] Biomes + scatter + instance export
- [ ] Weather state, timeline, water/ocean viewport
- [ ] Material graph + baking

## Phase 4 — Interoperability
- [ ] USD/USDA/USDC/USDZ writer + usdchecker; OBJ/STL/PLY; GeoTIFF
- [ ] QEM decimation, LOD chains, MikkTSpace, topology validation + stats
- [ ] Material translation layer + loss reports
- [ ] Blender bridge add-on (separate licence) + headless round-trip test
- [ ] Unity/Unreal export profiles (heightmap sizes, weightmaps, z-scale) + documented import procedures
- [ ] FBX import (ufbx) + optional FBX SDK export adapter (out-of-process) for fbx we build our own high performance accurate reader and writer from sratch you can learn from optima it's in Software Projects , but also you can verify against blender headless , api , autodesk fbx  , maya , unity and unreal

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
