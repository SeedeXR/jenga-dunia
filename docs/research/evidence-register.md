# Evidence Register

Index of research artefacts, their provenance and confidence. Each report tags claims `VERIFIED` (official docs/source fetched), `REPORTED` (third party), `UNVERIFIED`/`INFERRED`; see each file's legend. All sources accessed **2026-10-19**. No binaries of proprietary tools were inspected (ADR-0010).

| Report | Scope | Method | Key verified facts | Known gaps (see report) |
|---|---|---|---|---|
| `world-creator.md` | World Creator 2026.4, WC4 announcement | official docs (GitBook `.md` endpoints), EULA, buy page, release notes; press for WC4 | Windows-only; C#/.NET 9 + Vulkan; layer stack; level-step pyramid; export formats; file-based bridges; no CLI/API; pricing/EULA | project format undocumented; WC4 details REPORTED only; Unreal Z-scale not documented |
| `gaea-world-machine.md` | Gaea 2.3, World Machine 4067 | official docs/purchase pages, release notes, forums | pricing/tiers; GPU/CPU models; CLI (Gaea.Build, WM XML); typed ports (WM); outputs | several Gaea 2 doc pages 404; `.tmd` opaque; exit codes undocumented |
| `opensource-rea.md` | TerraForge3D, ProceduralTerrains, REA, other OSS | README/LICENSE/source tree fetches | licences (MIT / GPL plugin / unlicensed upstreams); architectures; REA capabilities + disclaimer | procedural-terrains.com SPA unreadable; weigert licences unclear; HAL ids unresolved |
| `formats-sdks.md` | formats, SDKs, engines, DCCs | crates.io/docs.rs/GitHub, vendor docs, local binaries (Blender 5.2, Maya 2027, FBX SDK 2020.3.9, Apple USD Tools 0.25.11) | crate versions/licences; `openusd` write support; FBX EULA; Unity/Unreal heightmap rules; Blender extension GPL requirement | usdz alignment with `openusd`; Unity RAW endian semantics; Unreal Interchange extension list |
| `stack.md` | Tauri, wgpu, frontend, crates, MCP, CI | crates.io/npm/GitHub releases, Tauri issues, runner-images README | Tauri 2.12.4; wgpu 30; WebGPU-in-WebView status; rmcp 3.5.1 / MCP 2026-07-28; runner labels; bincode 3 tombstone | WebView2 WebGPU default; WebKitGTK status; IPC throughput numbers |
| `vue-features.md` | e-on VUE 2023/2024 (discontinued, free) — terrains, EcoSystems, atmospheres, materials, splines, exports | Bentley e-on page, CG Channel 2024, Wikipedia 2023 feature list, CG Press release notes, Elephorm/Renderosity for terrain-editor effects | discontinuation + free licence (VERIFIED); 2024 preview features (VERIFIED); feature inventory (REPORTED) → ranked list of features no open tool ships (timeline erosion, dynamic ecosystems, strata/desert, sky HDRI, splines, infinite extension) | e-on manual offline; dynamic EcoSystem rule syntax and Function Editor node set not found |
| `algorithms.md` | noise, erosion, hydrology, LOD, decimation, materials, water, sky, scatter, determinism | paper lookups (ACM DL, arXiv, HAL, project pages), crates.io | ~55 citations resolved; selection table; data layout + determinism plan | coastal erosion (no graphics paper); some canonical DOIs from memory [M] |
| `geo-vision.md` | DEM/OSM/land cover licences and APIs; MoGe-2, DA3, alternatives; runtimes | licence pages, API docs, HF model cards, GitHub | Copernicus licence text; OpenTopography key policy; Google/MapTiler/Mapbox exclusions; OSM ODbL; model licences | NASA pages 403; MoGe-2 weight LICENSE file; DA3-LARGE-1.1 licence conflict; no Apple Silicon benchmarks |
| `materials-geomorphology.md` | texture libraries, soil/rock spectra, soil/geology/climate datasets, water/sky/wind/snow/dust shader techniques, geomorphic processes, GIS software licences | licence pages, dataset portals, papers, crates.io | bundle-safe CC0 textures = ambientCG, Poly Haven, TextureCan; USGS Spectral Library v7 CC0; SoilGrids/GLiM/Macrostrat/GSDS30 CC BY; WorldClim non-commercial (excluded); only WhiteboxTools + Landlab are MIT; AgX repo unlicensed (port from Godot MIT) | Fab EULA 403; Köppen Beck 2023 CC BY vs NC link conflict; roughness values unmeasured for all materials; Munsell table licence |
| `research-log/2026-10-19-installer-inventory.md` | World Machine Basic 4067 (.pkg), Gaea 2.3.1.0 (.exe) | inventory-only inspection with native tools (pkgutil, otool, file, 7z, innoextract); no decompilation; REA not run (ADR-0010) | WM: Qt 6.11.1, OpenGL+Metal, Slang 2026.7.1, plugin device bundles, binary `.tmd`, memory model in `world.ini`, EULA/privacy terms; Gaea: Inno Setup 7 bootstrapper, built 2026-08-10 | Gaea payload not extractable with innoextract 1.9 |
| `competitive-matrix.md` | cross-tool synthesis | derived from the above | — | inherits gaps |
| `licensing-register.md` | dependencies, tools, data, models | derived from the above | — | re-verify versions at adoption |

## Confidence policy

- A capability enters the competitive matrix as "Available and verified" only with a `VERIFIED` source in the underlying report.
- Versions are snapshots; any adoption re-verifies on crates.io/npm and records the date in the register.
- Gaps are backlog items (`memory/todo.md` → research follow-ups), not footnotes.

## Research follow-ups (from the reports' gap lists)

1. Confirm MoGe-2 weight-repo LICENSE file; resolve DA3-LARGE-1.1 licence conflict.
2. Test WebView2 `navigator.gpu` default; re-check WebKitGTK WebGPU status.
3. Verify `openusd` usdz alignment and `usdchecker --arkit` on real output.
4. Verify Unity RAW byte-order semantics and Unreal Interchange glTF extension list with real installs.
5. Obtain Mei 2007 PDF to confirm virtual-pipe equations; resolve [M] DOIs.
6. Clarify weigert repos' licences before any code reuse (else implement from blog prose).
7. Read procedural-terrains.com terms in a browser.
8. Benchmark MoGe-2 / DA3 via ort + CoreML on Apple Silicon.
