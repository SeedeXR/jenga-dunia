# Competitive Capability Matrix

Consolidated from `world-creator.md`, `gaea-world-machine.md`, `opensource-rea.md` (all sources accessed 2026-10-19). Status vocabulary per brief §1.2: **Available and verified** (AV) · **Partially supported** (PS) · **Unsupported** (U) · **Requires additional investigation** (RAI). The last column records Jenga Dunia's plan: **Suitable for integration** (SI: reuse a licence-compatible component) · **Suitable for independent implementation** (SII) · **Restricted by licensing** (RL). Marketing claims are not recorded as capabilities.

| Capability | World Creator 2026.4 | Gaea 2.3 | World Machine 4067 | TerraForge3D gen3 | ProceduralTerrains | Jenga Dunia plan |
|---|---|---|---|---|---|---|
| Platform: Windows | AV | AV (AVX2 + CUDA/HIP required) | AV | AV | AV (Electron) | AV target |
| Platform: macOS | U (WC4 planned) | U (Parallels blocked) | AV (universal, 4065+) | U (GL 4.3 compute) | PS (browser only) | AV target, Apple Silicon first (SII via wgpu/Metal) |
| Platform: Linux | U (WC4 planned) | U | AV (AppImage) | AV | PS (browser) | AV target |
| Authoring model | Layer/filter stack (AV); node view in WC4 (REPORTED) | Node graph (AV) | Node graph (AV) | Filter stack (AV) | Node graph → GLSL codegen (AV) | DAG + stack projection (SII) |
| Typed ports | n/a | RAI (undocumented) | AV (HF/Bitmap/Water/Scalar/VDM/Int) | n/a | AV | AV (SII; WM-style small type set) |
| GPU-accelerated generation | AV (Vulkan, all GPU) | AV (CUDA/HIP erosion) | U (CPU SIMD; Slang Code device) | AV (GL compute) | PS (WebGL + WebGPU pilot) | CPU-first + wgpu compute, parity tested (SII) |
| Multi-resolution preview/build | AV (level steps, 4k/8k preview) | AV (preview 512–4k, build) | AV (multi-res cache) | PS | AV (quality tiers) | AV plan: preview LOD + build + multi-res cache (SII) |
| Deterministic results | PS (seeds; cross-GPU divergence history) | PS (Erosion_2 yes; classic single-core) | AV (HR erosion) | RAI | AV (fixed-point atomics) | Contractual determinism (SII) |
| Noise types | AV (10 + fractal/warp) | AV | AV | AV (10 incl. Gabor/Phasor) | AV | SI (`fastnoise-lite`/`opensimplex2` CPU; port TF3D GLSL → WGSL) |
| Hydraulic erosion | AV (droplet + strata-aware) | AV (Erosion_2 GPU) | AV (HR model) | U in gen3 | AV (droplet ≤512²) | SII: pipes (default) + droplet + stream power |
| Thermal erosion | AV | AV | AV (mass balance, talus depth) | U | AV | SII |
| Wind / aeolian | PS (Drift: Wind filter) | PS | U | RL (unlicensed upstream) | U | SII (Paris 2019) |
| Snow / glacier | AV (snow sim) | AV (Snow, Glacier, IceFloe) | AV (Snow) | U | U | SII (P4/P5) |
| Coastal | AV (Shore filter, sea level) | AV (Sea) | AV (non-simulated) | U | U | SII heuristic (no graphics paper located) |
| Hydrology (rivers/lakes) | AV (Rivers layer, lakes via depth) | AV (Rivers, Lakes) | AV (Create Water, Flow Restructure, River) | U | PS | SII (priority-flood + D∞ + carving) |
| Multiple water levels | U (single water layer, REPORTED) | AV | AV | U | PS | AV plan (per-basin) |
| Terrain-feature masks | AV (slope/flow/cavity/occlusion/…) | AV | AV | AV (21 masks) | PS | SI (port TF3D masks) |
| Painted masks / sculpt | AV (1024² sculpt layers) | AV (Draw) | AV | AV (stroke API) | AV | SII (stroke nodes) |
| Biomes | AV | PS (via masks) | PS | AV (biome stacks) | PS | SII |
| Object scattering | AV (millions; Indie tier RAI) | AV (Trees/Shrubs/Debris) | PS (macros) | U | AV (props) | SII (Poisson + instance tables) |
| PBR materials + baking | AV (Substance, triplanar, stochastic) | AV (TextureBaker) | AV | PS | AV | SII + translation layer |
| Heightmap export EXR32/TIFF/PNG16/RAW | AV | AV (+PSD) | AV (+TER/BT/HFZ) | AV | AV (RAW16) | SI (`exr`, `image`, `tiff`) |
| Mesh export | PS (OBJ/FBX/glTF REPORTED) | AV (OBJ/FBX/DAE/glTF, PLY/XYZ) | PS (OBJ/glTF/VDM) | AV | AV (GLB/OBJ) | SI (`gltf`, own OBJ/PLY/STL) + USD (SII) + FBX (RL adapter) |
| Weightmaps / splat | AV (RGBA packing, Unity Mask Map) | AV | AV (packed RGBA + ID) | PS | PS | SII |
| Tiled builds / export | AV (tiles at export) | AV (Pro, 256k) | AV (Pro) | AV | U | AV plan from Phase 2 (core tiles) |
| Unreal bridge | AV (file-based, Landscape/WP/PCG) | AV (Gaea2Unreal, open source) | PS (guide) | U | AV (preset) | SII (manifest + importer) |
| Unity bridge | AV | PS (planned Gaea 3) | PS (guide) | U | AV (plugin MIT) | SII |
| Blender bridge | AV (Geometry Nodes) | U | U | U | AV (GPL plugin) | SII, separate GPL-compatible add-on (RL for core) |
| Houdini | AV | AV (Pro) | U | U | U | Deferred |
| Godot | AV (Terrain3D) | U | PS | U | AV (preset) | SII later (export preset) |
| USD | U | U (Gaea 3) | U | U | U | AV plan (SII) — differentiator |
| Real-world DEM | AV (GeoTIFF/HGT/DTED/…; MapTiler) | PS | PS | AV (MapTiler) | AV (Terrarium/OpenTopo) | SI (open DEMs; licence register) |
| CLI / headless | U | AV (Gaea.Build/Swarm, Pro) | AV (XML, Pro) | AV (HTTP API) | U | AV from Phase 1 |
| MCP / agent API | U (WC4 experimental) | U | U | PS (SKILL.md + OpenAPI) | U | AV from Phase 1 — differentiator |
| Documented exit codes | U | RAI | RAI | RAI | n/a | AV (spec) |
| Open project format | U (undocumented) | AV (JSON) | U (.tmd) | PS | AV (versioned JSON) | AV (spec) |
| Memory ceiling | VRAM-bound | 16–128 GB RAM recommended | RAM-gated by tier | GPU-bound | browser | ≤ 4 GB RAM (spec) |
| Licence | Proprietary; CE no export; Indie 8k | Proprietary; Community 1k non-commercial | Proprietary; Basic 1k | MIT | MIT (core), GPL (Blender) | MIT |

## Reading the matrix

- Jenga Dunia's clear wedges: cross-platform (Apple Silicon native), unrestricted export, open JSON format, CLI/MCP from day one, USD output, determinism contract, bounded memory. World Creator 4 (REPORTED, Nov 2026) and Gaea 3 narrow the platform and automation gaps — timing matters.
- Where the incumbents are strongest (erosion variety, mature scatter, bridges), the plan is independent implementation from published algorithms (`algorithms.md`) and permissively licensed references (`opensource-rea.md` §6), never from proprietary code.
