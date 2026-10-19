# TODO — Backlog and Milestones

Conventions: `- [ ]` planned · `- [x]` done · `- [~]` in progress (add owner) · `- [e]` experimental · ~~struck~~ changed requirement, with reason and replacement below it. Keep phases aligned with `docs/roadmap.md`.

## Round 1 — Research & single source of truth (2026-10-19)
- [x] Archive brief → `docs/research/brief/`
- [x] `memory/` files: agent_profile, session_start, philosophy, project_context, porting, mindmap, instruction, todo, handover_session
- [x] Architecture docs 00–13 with Mermaid (29 diagrams validated by `scripts/check-mermaid.mjs`)
- [x] Research reports in `docs/research/`: world-creator, gaea-world-machine, opensource-rea, formats-sdks, stack, algorithms, geo-vision
- [~] `docs/research/materials-geomorphology.md` (texture libraries, soil/rock spectra, environmental shaders, geomorphic processes) — research agent running at session end
- [x] `competitive-matrix.md`, `licensing-register.md`, `evidence-register.md`
- [x] ADRs 0001–0012 (0003, 0004, 0012 Proposed; others Accepted)
- [x] Specs: project-format, json-api, error-codes, performance-targets, interoperability-matrix
- [x] `docs/README.md` index; README, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT
- [x] Mermaid + link validation scripts; CI workflow (docs job on macOS/Ubuntu/Windows)
- [ ] Commit + push to `SeedeXR/jenga-dunia` main
- [ ] Research follow-ups listed in `docs/research/evidence-register.md`

## Phase 1 — Foundation (next)
- [ ] Cargo workspace skeleton (`jd-types`, `jd-diag`, `jd-jobs`, `jd-cache`, `jd-core`, `jd-cli`, `jd-mcp`) — no empty crates
- [ ] Project format v1 implementation + migrations + journal + autosave + tests
- [ ] Operation registry + schemars JSON schema generation committed under `schemas/v1/`
- [ ] CLI `jd new|diag|schema` with `--json` and exit codes
- [ ] MCP server over stdio with `project.*`, `diag.*` tools and scope gate
- [ ] Viewport feasibility prototype: native wgpu surface in Tauri window vs WebGPU/WebGL in WebView; measure; close ADR-0003
- [ ] CI: Rust matrix (macOS arm64, Ubuntu, Windows), `cargo deny`, nextest, S1 bench smoke
- [ ] Fixtures directory with malformed-file corpus seed
- [ ] `graphify` knowledge graph generation added to session-end checklist and CI (deterministic output)

## Phase 2 — Core terrain authoring
- [ ] `jd-terrain` tiles + halos + noise nodes + filters + analysis nodes
- [ ] `jd-graph` evaluator, cache, dirty sets, preview/build LOD
- [ ] Heightmap import/export (EXR, PNG16, TIFF, RAW r16/r32) with profile conventions
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
- [ ] FBX import (ufbx) + optional FBX SDK export adapter (out-of-process)

## Phase 5 — Scale & hardening
- [ ] Streaming, origin rebasing, 16k+ worlds under 4 GB
- [ ] GPU kernels + parity tests
- [ ] `jd-geo` (DEM/OSM/contours with attribution) and `jd-vision` (MoGe-2, DA3 via ONNX; licence gate)
- [ ] Fuzzing, signed releases, diagnostic bundles
- [ ] Acceptance scenario §20 green

## Deferred (with reasons)
- OpenVDB adapter — no justified volumetric use case before the SDF/voxel module exists (brief §9.1 "where justified").
- In-process `cdylib` plugins — out-of-process covers licensing isolation; ABI stability cost not justified yet.
- Shared daemon for agents to drive a running GUI — single-process adapters first.
