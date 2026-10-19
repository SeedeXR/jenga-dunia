# Open-source terrain/erosion references and REA — technical research report

Prepared for: Jenga Dunia (Rust/Tauri terrain tool, MIT, commercially usable)
Date: 2026-10-19
Method: GitHub REST API (`gh api`), raw file reads from `raw.githubusercontent.com`, crates.io API, npm registry, vendor docs via WebFetch, WebSearch for gaps.

## Evidence labels used throughout

- **VERIFIED** — read directly from the repo (API metadata, LICENSE, README, source files, shader files) or the vendor's own docs on this date.
- **REPORTED** — stated by the project's README/docs or a secondary source, not independently checked against code.
- **UNVERIFIED** — inferred, from training knowledge, or could not be fetched (source returned 404/503/cert error).

License rule applied: MIT/Apache-2.0/BSD/CC0/MPL-2.0 → reusable in an MIT project with attribution (MPL: keep modified MPL files under MPL). GPL/LGPL/AGPL → study only, never vendor or statically link into Jenga Dunia. No license file → all rights reserved; study only, no copying.

---

## 1. TerraForge3D — https://github.com/Jaysmito101/TerraForge3D

### 1.1 Identity and activity (VERIFIED via GitHub API, 2026-10-19)

| Field | Value |
|---|---|
| License | **MIT** (`LICENSE.md`, "Copyright (c) 2022 Jaysmito Mukherjee") |
| Default branch | `gen3` (branches: `gen2`, `gen3`, `gen3-cmake-integration`, `gh-pages`, `master`, `v2.3`, `vulkan`, `website`) |
| Last push | 2026-10-12 (commits: "add Directional Roughness and Directional Smooth filters with shader implementations") |
| Commits on gen3 | ~1,116; contributors ~10 |
| Stars / forks | 1,252 / 113 |
| Language | C++20 (CMake ≥3.12), C17 |
| Last tagged release | **v2.3, 2022-03-29** — no release since; gen3 is unreleased, source-only |
| `master` last commit | 2022-03-30 (the v2.x code line; the README on `gen3` is still the old v2.x README) |
| `vulkan` branch | last commit 2022-10-07, dormant |

Important: the README still describes the v2.x product (OpenCL, node editor with 40+ nodes, "Physically Based Erosion"). **The gen3 branch is a near-complete rewrite and does not currently contain any of those three things.** Treat README feature claims as REPORTED for v2.3 and NOT applicable to gen3 unless listed below.

### 1.2 Architecture — gen3 (VERIFIED from source tree, 828 files)

Build: single CMake target; vendored submodules: glfw, imgui, imgui-node-editor, glm, avir (image resampler), delaunator-cpp, dirent, cpp-httplib, nlohmann/json, stb, libwebp, fmt, tinyexr, glad (`gl:core=4.6`), spdlog. Requires OpenSSL. CI: `linux.yml` (ubuntu-latest, xvfb) and `windows.yml` (windows-2022). CMake has an `APPLE` branch naming the binary `tf3d`, but GLAD is generated for **OpenGL 4.6 core** and all generation shaders are `#version 430` compute shaders.

Source modules (`TerraForge3D/src/`):

- `Base/` — window, GL wrappers (Shader, ShaderStorageBuffer, Texture2D/2DStorage, FrameBuffer, AsyncTextureReadback), Mesh/Model/ModelImporter, ImGui GLFW/OpenGL backends, NodeEditor wrapper (`Base/NodeEditor/NodeEditor.cpp` exists, but no generation node graph uses it in gen3).
- `Generators/` — `BaseNoiseGenerator`, `BiomeBaseShapeGenerator`, `BiomeCustomBaseShape`, `BiomeManager`, `BiomeMixer`/`SimpleBiomeMixer`, `DEMBaseShapeGenerator` + `DEM/` (tile source, downloader, cache, renderer), `Filters/` (BiomeFilter, Catalog, Definition, Stack), `Masks/` (BaseMaskGenerator, MaskLayer, MaskRasterizer, MaskTool), `GenerationManager`, `GenerationWorker`, `GeneratorData`, `GeneratorDataStatistics`, `HeightfieldPyramid`, `HeightfieldRayQuery`, `SlopeGenerator`.
- `Renderer/` — HeightmapRenderer, ObjectRenderer, SeaRenderer, RendererSky, RendererLights, TerrainSelfShadow, PlanarShadowCache, HeightfieldAmbientCache, HeightfieldGICache, WireframeRenderer, TextureSlotRenderer.
- `Exporters/` — OBJ, PLY, STL, GLTF, Collada; texture exporters PNG, WebP, EXR, RAW; `Serializer`.
- `Inspector/` — data-driven property panels compiled from JSON under `Binaries/Data/inspectors/**` (schema, presets, shader uniforms).
- `Job/` — JobSystem/JobManager/Thread.
- `API/` — HTTP API (cpp-httplib + OpenSSL) with JSON-schema'd operations under `Binaries/Data/Api/Schemas/Operations/{Generation,Lights,Sea,Sky,Terrain,Viewport}`; serves `/api/v1/openapi.json`, `/api/docs` playground, and an agent-oriented `SKILL.md` (`/api/skill.md`). This is a notable design: the app is remote-controllable by an LLM agent over HTTP.

Generation pipeline model (VERIFIED from shader layout): **not a node graph.** It is a *biome stack*: Base Shape (one of 15 GLSL shape algorithms) → Base Noise (fBm over 10 noise primitives) → optional custom GLSL base shape → ordered **Filter Stack** per biome → **Biome Mixer** → masks. Everything is a GLSL compute shader on an `image2D` field (`TF3D_FIELD_FORMAT` = R32F default, R16F optional; `GeneratorData.h`).

### 1.3 Terrain representation and resolution (VERIFIED)

- Square single-channel float field textures (R32F/R16F), tile-based.
- `GenerationManagerSettings.cpp`: tile resolution must be a power of two in `[16, 1<<20]`; seed-texture resolution `[4, 1<<20]`; `kMaximumTileCount = 1,000,000`; effective cap is `GL_MAX_TEXTURE_SIZE` queried at runtime (typically 16384 or 32768).
- `HeightfieldPyramid` (min/max pyramid compute) + `HeightfieldRayQuery` for GPU ray queries against the heightfield (used by self-shadow, AO, GI caches).

### 1.4 Generation methods implemented in gen3 (VERIFIED from `Binaries/Data/shaders/`)

Noise primitives (`common/noise_2d.glsl`): BILLOW, SIMPLEX, PERLIN, GABOR, VALUE, PHASOR, RIDGED, VORONOI, WHITE, WORLEY (10). fBm driver (`generation/base_noise/noise_gen.glsl`): up to 16 octaves with per-octave strengths, lacunarity, persistence, rotation matrix per octave, domain warp (2 extra noise evaluations), per-octave anti-aliasing weight based on pixels-per-feature, 4 mix modes (add/mul/mul-add/replace), mask with LOD.

Base shapes (15): canyons, classic, cliff, cracks, crater, dunes, flat, islands, karst, mesas, mountain, rolling_hills, terraces, volcano, waves. Plus DEM base shape (real-world elevation via **api.maptiler.com** tiles — requires a MapTiler key; `DEM/map_gen.glsl`).

Filters (19): denoise — box_blur, flat_area_smooth (bilateral), gaussian_smooth, guided_smooth, laplacian_smooth, median; shaping — clip, curve, directional_roughness, directional_smooth, dome, flatten, heightmap, inflate_deflate, pixelate, ridged, set_value, smoothen, swirl.

Calculated masks (21): ambient_occlusion, aspect, coastline, curvature, distance_from_border, distance_from_coast, distance_from_point_path, dots, exposure, flatness, flow_wetness, grid, height_contour, height_range, procedural_noise, radial_gradient, ridge_valley, roughness, slope_ramp, slope_range, spiral. Plus painted mask strokes (API: BeginStroke/AppendStroke/FinishStroke/UndoStroke).

Rendering: PBR heightmap renderer, sky (equirect→cubemap, irradiance/specular/BRDF LUT compute), sea shader, terrain self-shadow, planar shadow, heightfield AO and GI caches (compute).

**Erosion in gen3: NONE.** No erosion shader, filter, or inspector exists in the gen3 tree (grep for `eros` returns nothing under gen3; the only hit is issue #55 from 2023-11 about the v2.x build). `Third Party License/` still lists `tinyerode.md` (MIT, 2021 "Taylor") and `opencl.md`, inherited from v2.x.

### 1.5 Erosion in the v2.x line (`master`, 2022) (VERIFIED file-level, algorithm REPORTED from headers)

- `Binaries/Data/compute/erosion.glsl` (GLSL 430 compute, SSBOs `map[]`, `brushIndices[]`, `brushWeights[]`, `randomIndices[]`; uniforms `inertia`, `sedimentCapacityFactor`, `minSedimentCapacity`, `depositSpeed`, `erodeSpeed`, `evaporateSpeed`, `gravity`, `startSpeed`, `startWater`, `maxLifetime`) and `Filters/ErosionFilter.cpp` (CPU variant, same parameter names). The variable names and `CalculateHeightAndGradient` function are a line-for-line match of **Sebastian Lague's `Erosion.compute`** (MIT) → this is the Hans Beyer droplet model.
- `Binaries/Data/kernels/advanced_erosion.cl` (OpenCL): contains `WindParticle` and `cascade()` with 8-neighbour offsets — structurally matches **Nick McDonald's SimpleWindErosion** (no license file in that repo; see §4.4). Use with caution: TerraForge3D relicensed it as MIT but the upstream has no license, so provenance is unclear. **Do not copy `advanced_erosion.cl`.**
- v2.x node editors: CPU node editor with FastNoiseLite nodes (Perlin, OpenSimplex2/2S, Cellular, Value, ValueCubic), GPU noise layer generator, shader-node editor (`shader_nodes/*.json`), Lua/Wren scripting, muparser. v2.x vendored `FastNoiseLite.h` (MIT) and `assimp`.

### 1.6 Export formats

- gen3 (VERIFIED from `Exporters/`): mesh OBJ, PLY, STL, glTF, Collada; textures PNG, WebP, EXR (tinyexr), RAW float. Project serialization via `Serializer.cpp` (nlohmann/json). The README's "GLB" claim is for v2.x; gen3 has `GLTFExporter.cpp` (whether it writes binary .glb was not checked — UNVERIFIED).

### 1.7 Platform support

- Windows and Linux: CI-built (VERIFIED).
- macOS: open issue #38 "MacOS support" (2022) and #57 "macOS build fails to launch: Requested OpenGL version 4.3, got version 4.1" (2023-12, open). Since gen3 is entirely GL 4.3+ compute shaders, **macOS is effectively unsupported** (VERIFIED from issues + shader versions).

### 1.8 Maturity assessment

- Solo-maintainer project with bursts of very active development (Oct 2026 commits), but no binary release in 3.5 years, README badly out of date, contributing guide boilerplate. gen3 is well-structured (data-driven inspectors, JSON-schema'd HTTP API, compute pipeline) but is a moving target with no versioned release.
- Code quality of gen3 shaders is good and self-contained (each filter is one GLSL file + one inspector JSON) — easy to read and port.

### 1.9 Legally reusable under MIT

- Everything authored by Jaysmito101 under `TerraForge3D/src`, `Binaries/Data/shaders`, `Binaries/Data/inspectors`, `Binaries/Data/Api` → **reusable with attribution** (keep the MIT notice).
- Vendored third parties have their own licenses (all permissive: glfw zlib, imgui MIT, glm MIT, stb MIT/Unlicense, tinyexr BSD-3, libwebp BSD-3, fmt MIT, spdlog MIT, OpenSSL Apache-2.0, avir MIT, delaunator MIT) — VERIFIED only by directory names; check each before vendoring.
- Exception: `advanced_erosion.cl` (v2.x) — derived from an unlicensed upstream; treat as study-only.
- Most valuable to port: `noise_2d.glsl` (10 primitives incl. Gabor/Phasor), `noise_gen.glsl` (anti-aliased fBm), the 21 calculated-mask shaders, `heightfield/minmax_pyramid` + `pyramid_ray_query` (fast heightfield ray queries for shadows/AO), the 15 base-shape shaders, and the data-driven inspector/JSON-schema approach for Tauri UI.

---

## 2. ProceduralTerrains — https://github.com/ZyFou/ProceduralTerrains / https://procedural-terrains.com

### 2.1 Identity and activity (VERIFIED via GitHub API + repo files)

| Field | Value |
|---|---|
| License | **MIT** core (`LICENSE`, "Copyright (c) 2026 ZyFou"). **Blender extension is GPL-3.0-or-later** (`plugins/blender/procedural_terrains/blender_manifest.toml`: `license = ["SPDX:GPL-3.0-or-later"]`). Unity package: MIT (REPORTED by README; `plugins/unity/.../LICENSE.md` exists, not read). |
| Created / last push | 2026-06-10 / 2026-10-19 (daily activity) |
| Commits / contributors | ~415 / 2 |
| Stars / forks | 634 / 86 |
| Language / stack | JavaScript (plain JS, no TypeScript), React 18, Vite 5, three.js **0.185.1**, `@xyflow/react` (node editor UI), fflate, leaflet; Electron 44 for desktop (`electron-forge package --platform=win32 --arch=x64` only) |
| Releases | v1.7.1 (2026-09-01), v1.8.4 (2026-09-22), `v0.1.0-alpha.1` npm pre-release (2026-10-19); `package.json` version 1.9.10 |
| Website | procedural-terrains.com is a client-rendered SPA (WebFetch returned only "Loading…"); content could not be read — UNVERIFIED. Deployed from `main` via `.github/workflows/deploy.yml`. |

### 2.2 Architecture (VERIFIED from tree, 859 files, and README)

- `src/engine/` — framework-agnostic WebGL engine; `Engine.js` is a documented god-object (roadmap: 5,476 lines, 257 methods, written 2026-07-02 against v0.14.1).
- `src/engine/terrain/` — `TerrainBoard.js` (tile mode), `InfiniteWorld.js` + `InfiniteTerrainClipmap/Culling/Batches`, `PlanetWorld.js` (cube-sphere) + `PlanetHeightBaker/Sampler`, `ChunkGeometry`, `LodMorph`, `MergeQuadtree`, `GpuHeightSampler`, `TerrainHeightBaker`, `TerrainExporter`, `RealWorld*` (heightmap, buildings, cities, markers), `detail/` (detail pages), `surface/` (texture atlas + surface graph), `noise/`, `erosion/`, `graph/`.
- `src/engine/terrain/noise/NoiseStack.js` + `noiseStackCodegen.js` — typed, serializable noise layers **compiled to GLSL** (terrain height is a pure function of (x, z, seed, params) evaluated per-fragment/vertex on the GPU).
- `src/engine/terrain/graph/` — `GraphDocument`, `GraphRegistry`, `GraphCompiler`: a node graph (UI via `@xyflow/react`, `src/components/nodes/NodeWorkspace.jsx`) compiled to the same GLSL pipeline.
- `src/engine/terrain/erosion/` — `erosionSim.js` (pure CPU), `erosion.worker.js` (Web Worker), `erosionWebGPU.js` (WebGPU compute backend, "Phase 4 pilot"), `ErosionField.js`, `ErosionPresets.js`.
- `src/engine/water/` — realistic water material, planar reflection pass, waves/optics/lighting GLSL, water masks and export.
- `src/export/` — `ExportPresetManager`, `ExportValidator`, presets Unity/Unreal/Godot/Blender/Three; `runtime/HeightfieldEncoding.js` (24-bit packed height → Unity RAW16 encoder) and `RuntimeTerrainDocument.js` (`.ptrterrain`).
- `src/creator/` — splines (`SplineEditor`, `SplineMaskBaker`), history, analysis. `src/paint/` — height/biome/prop paint.
- `api/` — optional self-hostable Node/MySQL accounts+cloud-save service (local-first app works without it).
- `plugins/unity/` (C#, Unity 6000.3+) and `plugins/blender/` (Python, Blender 5.2+): both can generate natively and import `.ptrterrain`/ZIP packages.
- Tests: `tests/`, `vitest`, Playwright perf harness. Roadmap doc admits "Zero automated tests (until now — see Phase 0)" as of July 2026.

GPU usage: WebGL (GLSL ES 1.0 by deliberate compat choice per `docs/ARCHITECTURE-ROADMAP.md`); WebGPU only for the erosion compute pilot (falls back to the CPU worker). No OpenGL/Vulkan — browser only.

### 2.3 Terrain representation and resolution (VERIFIED)

- Live terrain is **not a baked heightmap**; height is evaluated in-shader from the compiled noise/graph. Baking to a heightmap happens for erosion, export, and real-world import.
- Erosion grid resolutions are fixed by quality preset: preview 192, balanced 256, high 384, ultra **512** (`ErosionPresets.js`). This is a hard practical ceiling for erosion detail.
- Export mesh resolution is configurable (README); heights packed as 24-bit in RGBA, decoded to 16-bit RAW for Unity.

### 2.4 Generation methods (VERIFIED from `noiseTypes.js`, `GraphRegistry.js`, `erosionSim.js`)

Noise layer types: legacy ("Classic Terrain"), fbm, ridged (sharpness), billow, value (linear/smooth/smoother), white (smoothing), constant, voronoi (jitter; Euclidean/Manhattan/Chebyshev; cell value / dist-center / dist-edge / edge-lines), crater, dune (wind dir, ripples), flow/river (direction, width, meander); modifiers: domainWarp, terrace; per-layer params include `erosion` (a noise-space "erosion" look, 0..1) and `selfWarp`. Max octaves 8 (warp 6).

Graph nodes: deterministicNoise, mountain, mountainRange, ridge, island, singleCrater, canyon, duneSea, currentTerrain, classicTerrain, domainWarp ("Organic Warp"), shaper, riverCarve, combine, math, remap, terrace, stratify, geologyDetail, **thermalErosion**, **naturalErosion**, terrainGradient, slopeTint, moistureTint, colorGrade, terrainOutput.

Erosion (`erosionSim.js`, header comment): "Hydraulic erosion is the droplet model (Hans Beyer / Sebastian Lague)" + thermal relaxation (talus angle) + optional smoothing + master-strength blend; deterministic via mulberry32 PRNG; brush-radius erosion (same `buildBrush` idea as Lague). Outputs analysis masks: flow, erosion, deposition, sediment, slope. WebGPU backend uses batched droplets with fixed-point integer atomics (HEIGHT_SCALE 65536) for determinism. Defaults: 60k droplets, lifetime 30, radius 3, 30 thermal iterations.

Real-world data (`RealWorldHeightmap.js`): AWS/Mapzen **Terrarium** elevation tiles (`elevation-tiles-prod.s3...amazonaws.com/terrarium`), OpenTopo, and ESRI World_Imagery tiles. Note: ESRI imagery tiles have their own terms; Terrarium tiles are open (Mapzen/AWS Open Data) — REPORTED, verify terms before shipping.

### 2.5 Export formats (VERIFIED README + source)

GLB/GLTF, OBJ meshes (skirts, base slabs), baked color/normal/height/splat/collision maps, water surface meshes + depth/shoreline/foam masks, `terrain.json`, `project.ptrterrain`; presets: Unity Terrain (RAW16), Unreal Landscape, Godot Terrain3D, Blender Scene, Three.js. Quick PNG screenshot/heightmap.

### 2.6 Platform support

Any modern browser (WebGL2 required; WebGPU optional). Electron desktop build targets **Windows x64 only** (VERIFIED from npm script). Unity 6000.3+, Blender 5.2+ plugins.

### 2.7 Maturity

Very young (4 months), very fast-moving, 2 contributors, shipping releases every few weeks, has a candid architecture roadmap. Feature-rich but architecture is admittedly fragile (god-object, flat params). Good as an *idea and UX* reference (noise-stack→GLSL codegen, export presets per engine, production check validator, deterministic WebGPU erosion), weaker as a stable code dependency.

### 2.8 Legally reusable under MIT

- All `src/**`, `api/**`, `electron/**`, Unity plugin → MIT, reusable with attribution.
- `plugins/blender/**` → **GPL-3.0-or-later: study only.** (Relevant if Jenga Dunia ships a Blender add-on: it must be GPL anyway because it links `bpy` — but code must not flow from it back into the MIT core.)
- Built-in PBR textures under `public/textures/terrain` fetched by `tools/materials.mjs` — provenance/licensing not checked (UNVERIFIED); do not copy assets.
- Most portable pieces: `erosionSim.js` (clean, pure, deterministic; ~easy Rust port), `erosionWebGPU.js` (WGSL batching + fixed-point atomics strategy — directly applicable to `wgpu`), `noiseTypes.js` registry pattern, `HeightfieldEncoding.js` (Unity RAW16), export preset/validator design.

---

## 3. REA — https://github.com/morluto/rea (rea.tools)

### 3.1 What it is (VERIFIED from README, package.json, architecture.mermaid, source tree)

REA ("Reverse Engineer Anything") is an **MCP server + CLI** (`rea-agents` on npm, v6.7.0 published 2026-10-18; npm package first created 2026-07-12; repo created 2026-04-14) written in **TypeScript (Node ≥22.19 / ≥24.11 / 26+)** that gives a coding agent (Claude Code, Codex, Cursor, Gemini CLI, Grok Build, any MCP client) tools to inspect software artifacts and runtime behaviour. It is **agent-driven, not LLM-containing**: REA itself runs no model; the agent calls REA tools over MCP and REA returns findings "with the evidence and limitations behind each conclusion." Its own skill `.agents/skills/reverse-engineer-anything` supplies investigation instructions.

Analysis modes (VERIFIED from README table + `src/` dirs `adb android apktool application artifacts browser captures cli composition config contracts cutter domain dotnet electron evaluation evm filesystem firmware flutter frida gdb ghidra go hopper ida inspector javascript jeb kuna main native objdump pcap process providers reference rizin rust server signals sqlite wasm windows`):

- **Static native**: pseudocode/assembly/strings/symbols/xrefs via bring-your-own **Hopper, Ghidra (12.1.4 bridge), IDA (via ida-pro-mcp), Kuna, Rizin, Cutter, GNU objdump**; offline ELF layout via pwntools; Go and Rust binary metadata (cargo-auditable) built-in; .NET CIL metadata built-in (static only); WASM via WABT; EVM bytecode via evmole; 16-bit DOS via Ghidra.
- **Static app/package**: JavaScript/Electron (ASAR, modules, source maps, IPC boundaries — AST-only, no execution), Android APK (headless JADX + JDK), Apktool resources, Flutter snapshot provenance, Apple bundle anatomy, firmware (Binwalk/Unblob), SQLite databases.
- **Dynamic / runtime**: process capture with PTY (Linux/macOS), browser observation via CDP/Playwright, V8 Inspector capture, GDB/MI and pwndbg sessions, **Frida** (optional dep, LGPL-2.0 w/ WxWindows exception), ADB device control (packages, logs, screen captures), mitmproxy/HAR/SAZ network captures, pcap/TShark, sigrok logic-analyser recordings, recorded Linux crash dumps.
- Everything runs **locally**; the agent's model provider sees tool results.

Stars 330,786 / forks 83,689 / open issues 1,239 / contributors ~262 (VERIFIED API numbers on 2026-10-19; README itself celebrates "300,000 GitHub stars"). Last push 2026-10-19; releases roughly every 1–4 days (6.5.0 on 10-14, 6.6.0 and 6.7.0 on 10-18). It is a large, extremely active project, not sparse.

Maintainer identity: GitHub org/user `morluto`; rea.tools names no company or individual (VERIFIED via site fetch). Discord + X links only.

### 3.2 License and third-party terms (VERIFIED)

- REA-authored code: **MIT** (`LICENSE`, "Copyright (c) 2026 morluto").
- `THIRD_PARTY_NOTICES.md`: Ghidra Apache-2.0, JADX Apache-2.0, jadx-headless-mcp Apache-2.0, ghidra-nativeaot MIT, evmole MIT, Frida LGPL-2.0+exception (optional), MinGW-w64 runtime (GPL-3.0 with GCC Runtime Library Exception) statically linked in the Windows Node-API addon.
- Commercial engines (IDA Pro/Hex-Rays, Hopper, JEB) are **not bundled**; README/DISCLAIMER: "You must legally obtain and be authorized to use commercial tools… Cracked copies, stolen keys, unauthorized key sharing, and bypassing license restrictions are not authorized."
- `DISCLAIMER.md` (full text read): project "does not support, and will not help with" circumventing DRM/license checks/anti-cheat/TPMs; game cheats; bypassing auth/bot protection; **"copying proprietary software or reproducing its protected code and assets"**; evading AI-provider policies; reselling AI accounts. Also: "Obtaining source-like output or an analysis report does not grant rights to the original program, its data, or the generated output."

### 3.3 Requirements (VERIFIED README)

Node.js + npm; `npx rea-agents setup` registers the MCP server and installs workflow instructions into the chosen agents. Per target: Hopper/Ghidra/IDA for native; JDK + headless JADX for APKs; Chrome-family browser for websites; adb for devices; pwntools/TShark/sigrok/binwalk on Linux for the respective modes; on Linux a procps-compatible `ps` for process-ownership checks. Windows/macOS/Linux supported with per-mode caveats (several modes Linux-only).

### 3.4 Relevance to Jenga Dunia

REA has **no terrain functionality** and should not be a dependency. Its only legitimate roles for this project are:

1. **Interop / format research on artifacts we are entitled to inspect** — e.g. confirming the byte layout of our *own* exported `.r16`/RAW/EXR/glTF files, or inspecting third-party *open-source* tools (TerraForge3D, Terrain3D) whose licenses permit study, to answer "how does the importer parse this file?" where docs are thin.
2. **Debugging our own Tauri app**: Electron/JS analysis modes work on a built Tauri frontend bundle; process capture on our own binary; Rust binary metadata (cargo-auditable) for SBOM checks of our own releases.
3. **Black-box compatibility testing** of our exports against engines (Unity/Unreal/Godot) by observing *behaviour* (file accepted/rejected, logs), without decompiling the engine.
4. Security review of our own release binaries (symbols leaked, dependency inventory).

A clean-room separation note is in §7 (policy draft). The README's headline pitch ("See a feature in an app that you want in your own product… build a version for your project") is exactly the use-case that is risky for a commercial MIT project when the target is proprietary — the DISCLAIMER explicitly forbids "reproducing its protected code and assets," and in most jurisdictions decompiling a competitor to copy a feature's implementation exposes the project to copyright and EULA claims even when the output is re-typed.

---

## 4. Other notable terrain/erosion projects

### 4.1 Terrain Erosion 3 Ways — dandrino (VERIFIED API + README + source headers)

- MIT (`LICENSE.txt`), Python/NumPy/SciPy, last push 2022-11-22, 964 stars, inactive.
- Three methods: (1) **grid-based hydraulic simulation** (`simulation.py`, 512² grid, vectorized; water/sediment layers, capacity, deposit/erode, downhill transport, evaporation, plus "slippage" = angle-of-repose thermal step) — header cites ranmantaru.com (2011 blog) and HAL `inria-00402079`, which WebSearch resolves to **Šťava, Beneš, Brisbin, Křivánek, "Interactive Terrain Modeling Using Hydraulic Erosion", SCA 2008** (REPORTED; HAL returned 503). (2) **GAN** (progressive growing of GANs, Karras et al. 2017, trained on USGS NED tiles). (3) **River network synthesis** (`river_network.py`: Poisson-disc points, Delaunay, downhill tree, heights from stream order) — README points to HAL `hal-01262376` for a hybrid approach; WebSearch identifies the paper as **Génevaux, Galin, Guérin, Peytavie, Beneš, "Terrain Generation Using Procedural Models Based on Hydrology", ACM TOG 2013** (REPORTED; the HAL id found was hal-01339224, so the README's id may be a typo).
- Reusable: yes (MIT); the simulation step list is a clear spec for a Rust port; the river-network approach is a good "macro structure first" strategy.

### 4.2 SebLague/Hydraulic-Erosion (VERIFIED)

- MIT, C# + Unity compute shaders (`Assets/Scripts/ComputeShaders/Erosion.compute`, `HeightMap.compute`, `Noise.hlsl`, `Erosion.cs`), last push 2024-01-04, 1,045 stars.
- Algorithm: droplet/particle hydraulic erosion. README cites `firespark.de/.../implementation of a methode for hydraulic erosion.pdf` (Hans Theobald Beyer, bachelor thesis, TU München 2015 — REPORTED; the PDF host has an expired TLS cert and the thesis could not be fetched) and ranmantaru.com.
- Parameters: inertia, sedimentCapacityFactor, minSedimentCapacity, erodeSpeed, depositSpeed, evaporateSpeed, gravity, maxLifetime, erosionRadius (brush), initial speed/water. Both TerraForge3D v2.x and ProceduralTerrains are descendants of this exact code.
- Reusable: yes (MIT). The compute shader is a trivial WGSL port; the CPU C# is a trivial Rust port.

### 4.3 Nick McDonald (weigert) projects

| Repo | License | Status | Notes |
|---|---|---|---|
| SimpleErosion | README says "MIT License" but **no LICENSE file** (API `license: null`) | last push 2020-04 | particle hydraulic erosion on a grid; blog: weigert.vsos.ethz.ch |
| SimpleHydrology | README says "MIT License", **no LICENSE file** | last push 2023-03 | extends particle erosion with discharge/momentum maps → streams, meandering; flooding/pools removed Jan 2023 (buggy); blog nickmcd.me/2020/04/15/procedural-hydrology |
| SimpleWindErosion | **no license at all** (null) | 2023-05 | aeolian: abrasion, suspension, cascading; source of TerraForge3D's `advanced_erosion.cl` |
| SoilMachine | README says "MIT License", **no LICENSE file** | 2023-06 | layered multi-soil terrain (RLE sediment sections per cell), modular particle base class (wind + hydraulic), "erosion chains"; article nickmcd.me/2022/04/15/soilmachine |
| soillib | **LGPL-3.0** (LICENSE file present) | 2025-03 | successor: C++23 + CUDA, Python bindings (`pip install soillib`, Linux only), GeoTIFF I/O; maintained by Erosiv studio |

Legal reading: a README sentence "MIT License" without the license text is a weak grant (no copyright holder, no warranty disclaimer). Treat SimpleErosion/SimpleHydrology/SoilMachine as **"reusable only after obtaining the license text or written confirmation"**; until then, study-only and re-implement from the blog posts (the algorithms are described in prose and are not copyrightable). soillib: LGPL → not vendorable; dynamic linking from Rust is theoretically possible but it's CUDA/Linux-only, so irrelevant. SimpleWindErosion: all rights reserved → study only.

### 4.4 Rust-ecosystem crates (VERIFIED via crates.io API, 2026-10-19)

| Crate | Version / date | License | Downloads | What it offers |
|---|---|---|---|---|
| `noise` | 0.9.0 / 2024-03-23 (repo pushed 2025-02) | Apache-2.0 OR MIT | 2.8 M | Perlin, PerlinSurflet, OpenSimplex, Simplex (N-D), SuperSimplex, Value, Worley; fractals Fbm, Billow, RidgedMulti, HybridMulti, BasicMulti; modifiers Abs/Clamp/Curve/Exponent/Negate/ScaleBias/Terrace; combiners Add/Blend/Max/Min/Multiply/Power/Select; Turbulence, Displace, Checkerboard, Cylinders; `NoiseFn` trait composable as a graph. Not SIMD; slowest of the group but most complete and composable (docs.rs read). |
| `fastnoise-lite` | 1.1.1 / 2024-03-05 | MIT | 196 k | Official Rust port in Auburn/FastNoiseLite: OpenSimplex2, OpenSimplex2S, Cellular, Perlin, Value, ValueCubic; fractal FBm/Ridged/PingPong; domain warp (OpenSimplex2 / basic grid); 2D/3D. Single file, no deps. Repo active (pushed 2026-10-17). |
| `fastnoise2` + `fastnoise2-sys` | 0.4.0 / 2026-01-21 | MIT | 7.7 k | Safe wrapper over FastNoise2 (C++17 SIMD node-graph noise, MIT; upstream pushed 2026-10-18). Requires a C++ build (cmake) — adds Tauri build complexity but gives SIMD and an encoded-node-tree string format usable as a serialized "noise graph". |
| `bracket-noise` | 0.8.7 / 2022-10 | MIT | 386 k | Older Rust port of FastNoise (pre-Lite). Superseded by `fastnoise-lite`. |
| `simdnoise` | 3.1.6 / 2020-05 | Apache-2.0 OR MIT | 81 k | SIMD (runtime-detected) Simplex/FBm/Ridge/Turbulence/Cellular. Unmaintained since 2020. |
| `libnoise` (cookiephone) | 1.2.0 / 2025-04 | MIT | 25 k | Composable generators incl. Simplex/Worley/Value, fractals, adapters. Maintained. |
| `opensimplex2` | 1.1.0 / 2024-01 | CC0-1.0 | 6.9 k | Port of KdotJPG OpenSimplex2 (public domain). |
| `noisy_bevy` | 0.14.0 / 2026-06 | MIT | 95 k | Simplex/fBm as both Rust and WGSL shader functions — handy if we want identical CPU/GPU noise in `wgpu`. Bevy-coupled. |
| `symbios-ground` | 0.4.1 / 2026-09-06 | MIT | 1.3 k | Pure-Rust terrain engine: Diamond-Square, fBm value noise, Voronoi terracing; **droplet hydraulic erosion (with lake pooling, delta deposition) and talus thermal erosion**; splat mapping; bilinear height/normal queries; `TiledHeightMap` infinite streaming; serde. Young, 1 author (REPORTED from README). |
| `fastlem` | 0.1.4 / 2024-01 | **MPL-2.0** | 7.5 k | Simplified **landscape evolution model** (stream-power erosion on an irregular/Delaunay mesh), "in development". MPL-2.0 is file-level copyleft; usable as a dependency. |
| `martini` (reearth/terrain-codec) | 0.2.0 / 2026-05 | MIT OR Apache-2.0 | 3 k | RTIN mesh generation from heightmaps (Mapbox Martini algorithm) → LOD meshes for export/preview. |
| `terr` | 0.1.2 / 2019 | MIT OR Apache-2.0 | 4.8 k | Midpoint displacement, diamond-square. Dead. |
| `terrain` | — | — | — | Name is taken by an unrelated full-text search crate. **`terrain-rs` does not exist on crates.io** (VERIFIED). |

Not crates but Rust projects:
- **fintelia/terra** — Apache-2.0, large-scale planet terrain renderer (Rust, wgpu/vulkan), **archived** (pushed 2026-06). Study only for streaming/LOD; not a terrain *authoring* tool.
- **kaylendog/kosmos** — "Modular procedural terrain generator powered by Rust and WebGPU", node graph compiled to one WGSL compute shader, Bevy renderer; **GPL-3.0**, 7 stars, last push 2023-09. Study only (and small).
- **"Wilderness"** — no Rust procedural-terrain project of that name was found (WebSearch; the only hit was redblobgames/mapgen4, a TypeScript "wilderness map generator"). UNVERIFIED/likely does not exist.

### 4.5 Blender A.N.T. Landscape (VERIFIED from blender/blender-addons `ant_landscape/__init__.py`, `eroder.py`, `ant_functions.py`)

- **GPL-2.0-or-later** (SPDX header, Blender Foundation 2017-2022). Author Jimmy Hazevoet; eroder by Michel Anders and Ian Huish. Version 0.1.8; bl_info for 2.80. (In Blender 4.2+ it moved to the Extensions platform — REPORTED.)
- 19 noise types (Multi Fractal, Ridged MFractal, Hybrid MFractal, Hetero Terrain, fBm, Turbulence, Distorted Noise, Marble, Shattered hTerrain, Strata hTerrain, Another Noise, vlNoise turbulence, vlNoise hTerrain, Distorted hTerrain, Double MultiFractal, Noise Rocks, Slick Rock, Planet Noise, Blender Texture) and 22 "effects" (gradient, waves, zigzag, … cracks, rock, lunar craters, etc.).
- `eroder.py`: NumPy grid with water/sediment/scour/flowrate/capacity/avalanced layers = a grid-based hydraulic + thermal ("avalanche") model.
- Study only. The *Musgrave* multifractal family (MultiFractal, RidgedMF, HybridMF, HeteroTerrain) is public-domain mathematics (Musgrave, "Texturing & Modeling", 1994) and can be re-implemented from the book/papers, not from this code.

### 4.6 Godot Terrain3D (VERIFIED README + readthedocs)

- **MIT**, C++ GDExtension for Godot 4, 4,371 stars, pushed 2026-10-17, active.
- Regions 64 m – 65.5 km total; up to 1024 regions (32×32), 32 textures, 10 LODs, foliage instancer; sculpt/holes/paint colour+wetness.
- Import: any Godot-readable image (exr, png, …) + r16/raw; export exr, png, jpg, webp, r16, .res/.tres; recommends **exr (RGB, 16/32-bit float) or r16/raw**; 8-bit PNG explicitly unsuitable; control map is proprietary. This is the authoritative spec for a "Godot Terrain3D" export preset.

### 4.7 Houdini HeightField tools (feature list only; VERIFIED from sidefx docs, Houdini 22.0)

Node families: Height Field, HeightField Noise, File, Lidar Import, Output, Quick Shade, Scatter, Tile Split/Splice; Layer, Project, Paint, Pattern; Blur, Distort by Noise, Clip, Terrace; **Erode** (hydraulic + thermal; params Erosion Feature Size, Erodability, Flow Force, Bank Angle, Rainfall Coverage, Slope Influence, Erosion/Deposition/Removal/Evaporation Rate; thermal Weathering Force, Cut Angle, Repose Angle; outputs `height`, `sediment`, `debris`, `flow`, `flowdir`), **Slump**; Draw Mask, Mask by Feature (elevation/aspect/slope), Mask by Geometry, Mask Noise; Transform, Visualize, Remap, Resample, Wrangle/VOP, Copy Layer, Layer Properties, Layer Clear. Representation: 2D volumes, named height layers (e.g. `height`, bedrock/soil) + 0–1 mask layers. Commercial; docs cite no papers. Use as a *feature checklist and naming vocabulary* only.

### 4.8 Unreal Landscape + PCG (VERIFIED from Epic docs, UE 5.8)

- Landscape heightmap import: 16-bit PNG, r8, **r16** (raw 16-bit, plus JSON sidecar for `.raw`); heights stored 16-bit, range −256..255.992 × Z scale (default 100). Sizes: sections ≤256² vertices; quads per section 2ⁿ−1 (63/127/255); recommended overall sizes 127², 253², 505², 1009², 2017², 4033², **8129²**; "Epic recommends a maximum of 1024 Landscape Components."
- PCG: node graph over *points* (transform, bounds, density, steepness, seed, attributes); Get Landscape Data → Surface Sampler → Transform/Filter → Static Mesh Spawner; subgraphs/templates/parameters. Source is under the Unreal EULA (source-available, not OSS) — REPORTED; do not copy.
- Use: export preset "Unreal Landscape" = r16 at one of the recommended sizes + z-scale metadata.

### 4.9 Unity Terrain Tools package (VERIFIED from docs 5.0.6)

- Tools: Sculpt (Bridge, Clone, Noise, Terrace), Effects (Contrast, Sharpen Peaks, Slope Flatten), **Erosion (Hydraulic, Thermal, Wind)**, Transform (Pinch, Smudge, Twist), Smooth, Stamp, Paint Texture/Details; Brush Mask Filters (filter stacks: procedural masks); Terrain Toolbox (import heightmap, presets, batch settings, import/export splatmaps & heightmaps, altitude heatmap); Noise Editor.
- License: **Unity Companion License** ("for Unity-dependent projects") → not reusable; study UI/feature set only. Unity Terrain heightmaps are RAW 16-bit (little-endian) — the Unity RAW16 encoder in ProceduralTerrains is a tested reference.

---

## 5. Capability matrix

Status vocabulary: Available and verified · Partially supported · Unsupported · Requires additional investigation · Suitable for integration · Suitable for independent implementation · Restricted by licensing.

| Capability | TerraForge3D (gen3) | ProceduralTerrains | Other notable (best source) | Jenga Dunia recommendation |
|---|---|---|---|---|
| Noise primitives (Perlin/Simplex/Value/Worley) | Available and verified (10 GLSL primitives incl. Gabor, Phasor) | Available and verified (fbm/ridged/billow/value/voronoi/white) | `fastnoise-lite` MIT, `noise` MIT/Apache — Available and verified | Suitable for integration (`fastnoise-lite` CPU; port TF3D `noise_2d.glsl` to WGSL for GPU) |
| fBm / multifractal / domain warp | Available and verified (16 octaves, per-octave AA) | Available and verified (8 octaves, self-warp, terrace mod) | `noise` crate (Fbm/Ridged/Hybrid/Billow) | Suitable for integration |
| Node graph authoring | Unsupported in gen3 (biome filter stack instead; v2.x had one) | Available and verified (`GraphRegistry` → GLSL compile; xyflow UI) | kosmos (GPL, Restricted by licensing); FastNoise2 encoded node trees (MIT) | Suitable for independent implementation (graph → WGSL codegen, following PT's design) |
| Droplet hydraulic erosion (Beyer/Lague) | Unsupported in gen3; v2.x GLSL port Available and verified | Available and verified (CPU worker + WebGPU, ≤512²) | SebLague MIT (compute), symbios-ground MIT (Rust) | Suitable for integration (port Lague/PT to Rust + WGSL) |
| Grid/shallow-water hydraulic erosion (virtual pipes / Šťava) | Unsupported | Unsupported | dandrino MIT (NumPy); Houdini (commercial, feature ref) | Suitable for independent implementation (from Šťava 2008 / Mei 2007 papers) |
| Thermal erosion / talus | Unsupported in gen3 | Available and verified (thermal pass, graph node) | symbios-ground MIT; dandrino "slippage" | Suitable for integration |
| Wind/aeolian erosion | v2.x OpenCL kernel — Restricted by licensing (unlicensed upstream) | Unsupported (dune noise only) | SimpleWindErosion (no license) | Suitable for independent implementation from McDonald's blog description |
| Particle hydrology (streams, pools, discharge) | Unsupported | Partially supported (riverCarve, flow noise, river tools) | SimpleHydrology (README-only MIT) — Requires additional investigation | Suitable for independent implementation |
| Layered soil / stratigraphy | Unsupported | Partially supported (`stratify`, `geologyDetail` nodes, shader-only) | SoilMachine (README-only MIT), soillib LGPL | Requires additional investigation |
| Landscape evolution model (stream power) | Unsupported | Unsupported | `fastlem` MPL-2.0 | Suitable for integration (MPL ok as dependency) |
| Terrain analysis masks (slope, curvature, AO, flow-wetness, coast distance…) | Available and verified (21 compute masks) | Partially supported (erosion masks: flow/deposition/sediment/slope; analysis manager) | Houdini Mask by Feature (feature ref) | Suitable for integration (port TF3D mask GLSL to WGSL) |
| Painted masks / splines / brushes | Available and verified (stroke API) | Available and verified (paint layers, splines, props) | Terrain3D sculpt tools | Suitable for independent implementation |
| Real-world DEM import | Available and verified (MapTiler tiles, API key) | Available and verified (AWS Terrarium, OpenTopo, ESRI imagery) | Terrain3D imports r16/exr | Suitable for integration (Terrarium tiles; check ESRI terms) |
| Heightfield GPU ray query / self-shadow / AO / GI | Available and verified (min-max pyramid compute) | Partially supported (shader-side normals, planar reflections) | — | Suitable for integration (port pyramid shaders) |
| Sea/water rendering | Available and verified | Available and verified (realistic water, masks, export) | — | Suitable for independent implementation |
| Sky / PBR preview | Available and verified | Available and verified (sky, clouds, atmosphere, planet) | — | Partially supported in plan; independent implementation |
| Infinite / streamed world | Unsupported (tiles, up to 1e6 tiles via API) | Available and verified (clipmap, merge quadtree) | fintelia/terra (Apache, archived) | Requires additional investigation (likely out of scope v1) |
| Planet / cube-sphere | Unsupported | Available and verified | terra | Unsupported (out of scope) |
| Mesh export (OBJ/glTF/PLY/STL/DAE) | Available and verified | Available and verified (GLB/GLTF, OBJ, skirts, slabs) | `martini` MIT for RTIN LOD meshes | Suitable for integration (`gltf`, `obj` crates + martini) |
| Heightmap export (PNG16/EXR/RAW/r16) | Available and verified (PNG, WebP, EXR, RAW float) | Available and verified (RAW16 Unity, packed PNG) | Terrain3D/Unreal specs verified | Suitable for integration (`exr`, `png`, `image` crates) |
| Engine presets (Unity/Unreal/Godot/Blender) | Unsupported (generic exports) | Available and verified (5 presets + validator) | Terrain3D/Unreal/Unity docs | Suitable for independent implementation using verified specs |
| Engine plugins (Unity/Blender importers) | Unsupported | Available and verified (Unity MIT; Blender GPL) | — | Blender add-on: Restricted by licensing (must be GPL; keep separate repo) |
| Tiled high-res baking (>16k) | Available and verified (tiles up to GL max texture) | Unsupported (≤512 erosion; export res configurable) | Houdini Tile Split/Splice | Suitable for independent implementation |
| HTTP/agent API | Available and verified (OpenAPI + SKILL.md) | Unsupported | REA (different domain) | Suitable for independent implementation (Tauri command layer + optional local HTTP) |
| Undo/redo, project files | Partially supported (Serializer; API revisions) | Available and verified (versioned JSON, migrations) | — | Suitable for independent implementation |
| macOS support | Unsupported (GL 4.3 compute) | Partially supported (browser only; Electron Windows-only) | Terrain3D via Godot | wgpu/Metal via Tauri — Suitable for independent implementation |
| GPU compute backend | OpenGL 4.3+ compute (VERIFIED) | WebGL (GLSL1) + WebGPU pilot for erosion | kosmos WGSL (GPL) | `wgpu` + WGSL (Suitable for independent implementation; port GLSL filters) |

---

## 6. Reusable building blocks for Jenga Dunia

Concrete, license-checked items, ordered by value. "Port" means re-implement in Rust/WGSL with attribution in `THIRD_PARTY_NOTICES`; "depend" means add a crate.

**Noise (CPU)**
1. Depend: `fastnoise-lite` 1.1.1 (MIT) — fast, zero-dep, OpenSimplex2/2S, Cellular, Perlin, Value/ValueCubic, fractals, domain warp. Default CPU noise.
2. Depend: `noise` 0.9 (MIT/Apache-2.0) — for the composable `NoiseFn` combinators (Select/Blend/Terrace/Curve/Turbulence) when building a CPU evaluation path for the node graph; or as a reference implementation of HybridMulti/RidgedMulti.
3. Optional: `fastnoise2` 0.4 (MIT; C++ build) if SIMD throughput on CPU matters and we accept a cmake step in Tauri CI. Its encoded node-tree strings are a ready-made serialization for noise graphs.
4. Depend (public domain): `opensimplex2` (CC0).

**Noise (GPU, WGSL)**
5. Port: TerraForge3D `Binaries/Data/shaders/common/noise_2d.glsl` (MIT) — 10 primitives incl. Gabor and Phasor, and `noise_gen.glsl` — fBm with per-octave strengths, rotation, warp, and pixels-per-feature anti-aliasing (prevents shimmer at high octave counts). Straight GLSL→WGSL.
6. Alternative: `noisy_bevy` (MIT) ships WGSL simplex/fbm sources identical to its Rust versions — copy the WGSL functions (attribution) even without Bevy.

**Erosion**
7. Port: SebLague `Erosion.compute` + `Erosion.cs` (MIT) → Rust (CPU) and WGSL (GPU) droplet hydraulic erosion. Use ProceduralTerrains `erosionWebGPU.js` (MIT) as the design for deterministic parallel droplets: batched droplets against a height snapshot, fixed-point i32 `atomicAdd` accumulation, floor clamp between batches, CPU-precomputed start positions (mulberry32) and brush kernel. Also port its thermal pass and the flow/erosion/deposition/sediment/slope mask outputs.
8. Depend or port: `symbios-ground` 0.4 (MIT) — pure-Rust HydraulicErosion (with lake pooling) + ThermalErosion + Diamond-Square + splat mapper + bilinear height/normal queries. Small enough to vendor pieces; evaluate quality before depending (young crate).
9. Port: dandrino `simulation.py` (MIT) as the **grid-based** (non-droplet) hydraulic model with angle-of-repose slippage — gives Houdini-like "whole-field" erosion; implement as a WGSL compute ping-pong (water, sediment, height textures). For the theory, cite Šťava et al. 2008 and Mei, Decaudin, Hu 2007 ("Fast Hydraulic Erosion Simulation and Visualization on GPU", virtual-pipes) — the latter is not cited by any repo examined here (UNVERIFIED as a dependency, known in the literature).
10. Depend (MPL-2.0): `fastlem` for a stream-power landscape-evolution "macro" pass if we want geologically plausible drainage at continental scale. Keep unmodified or keep modifications under MPL in their own files.
11. Independent implementation from prose: McDonald's particle hydrology (discharge + momentum maps → meanders) and wind erosion (abrasion/suspension/cascade) — implement from the blog posts, not the repos (license text missing).

**Analysis masks / heightfield utilities**
12. Port: TerraForge3D `masks/calculated_mask/*.glsl` (21 masks) and `heightfield/minmax_pyramid`, `pyramid_ray_query`, `self_shadow`, `ambient`, `gi` compute shaders (MIT) → WGSL. These give slope/aspect/curvature/flow-wetness/coast-distance masks and fast heightfield ray queries for AO/shadows in the viewport.
13. Port: TerraForge3D 19 filter shaders (guided/bilateral/laplacian/median smoothing, terrace/curve/clip/dome/swirl/directional roughness) — each is a single small GLSL compute file.

**Export / interchange**
14. Depend: `image`/`png` (16-bit PNG), `exr` (OpenEXR float), `gltf` + `gltf-json` or `mesh-tools` for glTF/GLB, `obj` crate, `martini` (MIT/Apache) for RTIN LOD meshes.
15. Port: ProceduralTerrains `HeightfieldEncoding.js` (MIT) Unity RAW16 encoder (little-endian u16, row order conventions) and its per-engine preset + "Production Check" validator design.
16. Specs (no code needed): Terrain3D exr-RGB-float / r16 + region size rules; Unreal r16 + 2ⁿ+1 recommended sizes (…1009, 2017, 4033, 8129) + z-scale; Unity RAW16.

**Real-world DEM**
17. AWS Terrarium tiles (`elevation-tiles-prod.s3.amazonaws.com/terrarium/{z}/{x}/{y}.png`, decode `(R*256+G+B/256)-32768`) as used by ProceduralTerrains; confirm the AWS Open Data terms. Avoid shipping ESRI imagery without checking ESRI terms. MapTiler (TerraForge3D) needs a per-user API key.

**Architecture patterns worth copying (ideas, not code)**
18. TerraForge3D: data-driven inspectors (JSON schema → UI → shader uniforms), JSON-schema'd operations exposed as OpenAPI + `SKILL.md` for agents, tile-based baking up to GL max texture.
19. ProceduralTerrains: typed noise-layer registry → shader codegen; deterministic world = f(x, z, seed, params); versioned project JSON with migrations; quality tiers with GPU-tier detection.

**Do not reuse (license)**: ANT Landscape (GPL-2.0+), ProceduralTerrains Blender plugin (GPL-3.0+), kosmos (GPL-3.0), soillib (LGPL-3.0), SimpleWindErosion (no license), TerraForge3D v2.x `advanced_erosion.cl` (derived from it), Unity Terrain Tools (Unity Companion License), Unreal Landscape/PCG source (Unreal EULA), Houdini (commercial).

---

## 7. REA usage policy draft (for Jenga Dunia contributors)

**Purpose.** REA (MIT, morluto/rea) is an MCP/CLI toolkit that lets coding agents inspect binaries, apps, and runtime behaviour. This policy defines the only ways it may be used in connection with Jenga Dunia.

**Permitted uses**
1. Inspecting software we own or author: Jenga Dunia builds (Tauri binary, frontend bundle, Rust crates), our test fixtures, and our exported files — for debugging, SBOM/dependency inventory (cargo-auditable), symbol/strip verification, crash analysis, and performance tracing.
2. Inspecting open-source software whose license permits study (MIT/Apache/BSD/MPL/GPL/LGPL are all fine to *read*), to understand file formats or behaviours where docs are insufficient. Note: reading GPL/LGPL code via REA does not change the rule that it must not be copied into our MIT codebase.
3. Black-box interoperability testing: observing whether Unity, Unreal, Godot, Blender, or other targets accept our exports (logs, screenshots, process output, network captures of our own traffic). Observing behaviour and documented/public file formats is allowed; decompiling the engine to extract its code is not.
4. Security research on our own releases and dependencies, and analysis of malware/suspicious files encountered in the course of the project, inside an isolated VM.
5. Verifying third-party contributions (e.g. that a PR's binary asset contains what it claims).

**Prohibited uses**
1. Decompiling, disassembling, or tracing any proprietary terrain tool (Gaea, World Machine, World Creator, Houdini, Unity Terrain Tools, Unreal Landscape/PCG, Instant Terra, etc.) to learn or reproduce their algorithms, shaders, UI, file formats beyond publicly documented ones, or assets. Feature parity is achieved from public docs, papers, and open-source code only.
2. Circumventing any technical protection measure, license check, trial limitation, DRM, anti-cheat, or access control, on any software — including commercial decompilers REA integrates with (IDA, Hopper, JEB must be legitimately licensed if used at all).
3. Copying, transcribing, paraphrasing, or "re-typing" pseudocode, decompiled output, extracted shaders, or assets from any non-permissively-licensed target into Jenga Dunia. Agent-generated "clean" reimplementations derived from decompilation are still derivative works.
4. Analysing any software, device, account, or service without written authorization from its owner, or in breach of its EULA/ToS (many EULAs prohibit reverse engineering even where statute allows interoperability exceptions).
5. Capturing or retaining other people's network traffic, credentials, or personal data.

**Clean-room rules when REA is used on third-party software (permitted cases 2–3)**
- Record the target, its license, the question asked, and the REA commands/tool calls in `docs/research-log/`.
- Output of REA (pseudocode, listings, captures) never goes into the source tree, PRs, issues, or commit messages. Keep it in the research log or discard.
- If any proprietary target is ever examined for an interoperability exception (e.g. EU Directive 2009/24/EC art. 6 / US DMCA §1201(f)), the person running REA writes a **functional specification** only (formats, protocols, observed behaviour), and a *different* contributor implements from that spec. The spec is reviewed by a maintainer before implementation. Prefer not to do this at all.
- Run REA inside a VM/container without access to project credentials; treat all analysed files as untrusted.

**Tool licensing**
- REA is MIT; it is a dev-time tool and is never a runtime or build dependency of Jenga Dunia. Frida (LGPL) and the Windows addon (GPL runtime with exception) are REA's concern, not ours, as long as REA is not vendored.
- Commercial engines (IDA/Hopper/JEB) are optional; if a contributor uses one, they must hold a valid license; no shared keys.

**Responsibility.** Whoever directs an agent to use REA is responsible for the commands it runs (per REA's own DISCLAIMER). Maintainers may reject contributions that cannot show a clean provenance.

---

## 8. Gaps and items requiring further investigation

1. **procedural-terrains.com** content (pricing, accounts, hosted terms) could not be read (SPA). Check in a browser whether the hosted service imposes terms on exports.
2. **Hans Beyer thesis** PDF host has an expired certificate; the thesis's full citation (title/year/institution) is REPORTED only.
3. **HAL paper ids** in dandrino's README: inria-00402079 and hal-01262376 could not be resolved on HAL (503); identifications (Šťava 2008; Génevaux 2013) rest on WebSearch of the surrounding links.
4. **weigert repos** (SimpleErosion, SimpleHydrology, SoilMachine) have README-only MIT statements — request a LICENSE file or confirmation from the author before copying any code; otherwise re-implement from the blog posts.
5. **TerraForge3D gen3** has no release; API/schema layout may change. The v2.x node editor and erosion are gone; an "erosion filter" may return (the filter stack is designed to take compute-shader filters). Watch the repo.
6. **TerraForge3D vendored licenses** were identified by directory names only; verify each `Third Party License/*.md` before relying on them.
7. **ProceduralTerrains built-in textures** (`public/textures/terrain`) and the ESRI World_Imagery tile use — provenance/terms unverified.
8. **`symbios-ground`** and **`fastlem`** quality is untested here; benchmark before adopting.
9. **Mei et al. 2007 (virtual pipes)** and **Musgrave multifractals**: referenced from general knowledge, not from any fetched source — cite from the original papers when implementing.
10. **REA**: the 330k-star figure is what the GitHub API returns today for a 6-month-old repo; it is unusual but VERIFIED as an API value. No company behind it is identified; treat its security posture (it executes targets, attaches debuggers, launches Ghidra/Hopper) with care — run in a VM.

## Sources (primary)

- https://api.github.com/repos/Jaysmito101/TerraForge3D ; raw files on branches `gen3` and `master`; issues #38, #55, #56, #57
- https://api.github.com/repos/ZyFou/ProceduralTerrains ; `README.md`, `LICENSE`, `package.json`, `docs/ARCHITECTURE-ROADMAP.md`, `src/engine/terrain/{noise/noiseTypes.js,erosion/*.js,graph/GraphRegistry.js,RealWorldHeightmap.js}`, `src/export/runtime/HeightfieldEncoding.js`, `plugins/blender/procedural_terrains/blender_manifest.toml`
- https://api.github.com/repos/morluto/rea ; `README.md`, `LICENSE`, `DISCLAIMER.md`, `THIRD_PARTY_NOTICES.md`, `package.json`, `docs/architecture.mermaid`; https://registry.npmjs.org/rea-agents ; https://rea.tools/
- https://github.com/dandrino/terrain-erosion-3-ways (README, simulation.py, river_network.py) ; https://github.com/SebLague/Hydraulic-Erosion ; https://github.com/weigert/{SimpleErosion,SimpleHydrology,SimpleWindErosion,SoilMachine,soillib}
- crates.io API for noise, fastnoise-lite, fastnoise2(-sys), bracket-noise, simdnoise, libnoise, opensimplex2, noisy_bevy, symbios-ground, fastlem, martini, terr ; https://docs.rs/noise/latest/noise/ ; https://github.com/Auburn/FastNoiseLite/tree/master/Rust ; https://github.com/Lemonzyy/fastnoise2-rs ; https://github.com/kaylendog/kosmos ; https://github.com/fintelia/terra
- https://raw.githubusercontent.com/blender/blender-addons/main/ant_landscape/{__init__.py,eroder.py,ant_functions.py}
- https://github.com/TokisanGames/Terrain3D ; https://terrain3d.readthedocs.io/en/stable/docs/import_export.html
- https://www.sidefx.com/docs/houdini/model/heightfields.html ; https://www.sidefx.com/docs/houdini/nodes/sop/heightfield_erode.html
- https://dev.epicgames.com/documentation/en-us/unreal-engine/landscape-technical-guide-in-unreal-engine ; https://dev.epicgames.com/documentation/en-us/unreal-engine/procedural-content-generation-overview
- https://docs.unity3d.com/Packages/com.unity.terrain-tools@5.0/manual/TableOfContents.html ; .../license/LICENSE.html
- Paper lookups: https://cs.purdue.edu/cgvlab/www/publications/Stava08SCA ; https://history.siggraph.org/?p=108417 (Génevaux et al. 2013)
