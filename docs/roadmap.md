# Roadmap

Phases follow the brief §19. Each phase ends with a complete, testable end-to-end workflow, not isolated features. Backlog detail and status live in `memory/todo.md`; this file is the stable high-level plan.

```mermaid
gantt
  title Jenga Dunia phases (indicative, dependency-ordered; no dates committed)
  dateFormat YYYY-MM-DD
  axisFormat %b %Y
  section P1 Research & foundation
  Research + architecture docs (this round)      :done, p1a, 2026-10-13, 2026-10-19
  Workspace skeleton, jd-types/diag/jobs/cache    :p1b, 2026-10-20, 21d
  Project format v1 + migrations + autosave       :p1c, after p1b, 14d
  Viewport feasibility prototype (ADR-0003 gate)  :p1d, after p1b, 21d
  CLI + MCP foundations (op registry, schemas)    :p1e, after p1b, 21d
  CI matrix, cargo-deny, benches, fixtures        :p1f, 2026-10-20, 14d
  section P2 Core terrain authoring
  jd-terrain noise/filters/analysis (tiled)       :p2a, after p1c, 28d
  jd-graph evaluator + cache + dirty tracking     :p2b, after p1c, 28d
  Viewport (terrain LOD, PBR, sky)                :p2c, after p1d, 35d
  Sculpt/stroke nodes                              :p2d, after p2b, 14d
  Heightmap import/export (EXR/PNG/TIFF/RAW)       :p2e, after p2a, 10d
  Basic materials + glTF/GLB export                :p2f, after p2b, 21d
  Node editor UI + inspectors                      :p2g, after p1e, 35d
  section P3 Simulation & environments
  Hydraulic (particle, grid), thermal, wind        :p3a, after p2b, 35d
  Hydrology (fill, flow, rivers, lakes)            :p3b, after p3a, 21d
  Biomes + scatter                                  :p3c, after p3b, 21d
  Weather state + water/ocean viewport             :p3d, after p2c, 28d
  Material graph + baking                           :p3e, after p2f, 28d
  section P4 Production interoperability
  USD/USDA/USDC/USDZ + OBJ/STL/PLY + GeoTIFF        :p4a, after p3e, 28d
  Decimation (QEM), LOD chains, MikkTSpace, validation :p4b, after p3e, 28d
  Material translation (glTF/USD/MaterialX) + loss reports :p4c, after p4a, 21d
  Blender bridge, Unity/Unreal profiles, FBX adapter :p4d, after p4a, 28d
  Validators (glTF Validator, usdchecker, Blender headless) in CI :p4e, after p4a, 14d
  section P5 Scale & hardening
  Streaming large worlds, origin rebasing          :p5a, after p4b, 35d
  GPU kernels + parity tests                        :p5b, after p3a, 35d
  Geo (DEM/OSM) + vision (MoGe-2/DA3) modules       :p5c, after p4c, 35d
  Security hardening, fuzzing, signed releases      :p5d, after p4e, 21d
  Acceptance scenario (brief §20) green             :milestone, p5e, after p5d, 0d
```

## Phase exit criteria

| Phase | Exit criteria |
|---|---|
| P1 | Docs reviewed; workspace builds on 3 OSes; `jd diag env` works; viewport prototype decision recorded in ADR-0003 with measurements; op registry generates CLI + MCP + TS types; S1 bench runs in CI |
| P2 | Generate a 4k terrain with noise + filters in GUI, CLI and MCP from the same project; view it; export EXR/R16/GLB validated; project reopens after crash via journal |
| P3 | Erosion + hydrology + biomes + scatter + weather preview on S2; masks exported; determinism tests pass |
| P4 | Interop matrix rows for glTF, USD, OBJ, heightmaps, weightmaps marked verified with validator output; Blender headless round-trip test; Unity/Unreal import procedures documented and tested where tooling exists |
| P5 | 16k tiled world under 4 GB; GPU vs CPU parity; acceptance scenario §20 fully green or with explicitly listed UNVERIFIED items; signed releases |

## Classification of backlog items

`completed` · `in-progress` · `experimental` (usable, not production-ready) · `planned` · `deferred` (with reason). Experimental features are labelled in the UI and in `jd --features`.
