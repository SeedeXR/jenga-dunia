# Project Context — Jenga Dunia

**Repository:** https://github.com/SeedeXR/jenga-dunia (MIT)
**Source brief:** `docs/research/brief/WorldCreator-JengaDunia.md` (the master engineering, research and development prompt; authoritative for scope)
**Status (2026-10-19):** Round 1 — research and documentation only. No application code yet.

## Purpose

Jenga Dunia ("build the world" in Swahili) is an open-source, commercially usable, professional-grade **procedural world generation and environmental simulation** desktop application. It combines the strongest capabilities of existing procedural terrain tools (World Creator — primary reference — Gaea, World Machine, TerraForge3D and others) under a coherent, maintainable, standards-compliant architecture suitable for long-term open-source development.

Target workflows: game development, VR/XR, film, architectural visualisation, simulation, digital heritage, scientific visualisation, web 3D.

## Objectives

1. Generate geographically plausible, visually convincing, technically optimised 3D environments.
2. Non-destructive node-graph authoring with deterministic, cached, incremental evaluation.
3. Physically informed simulation (hydraulic/thermal/wind erosion, hydrology, weather, water/ocean, snow) with stated assumptions and fidelity.
4. Geometry quality: validated topology, decimation, LODs, measurable statistics.
5. PBR material authoring with a translation layer to glTF PBR, UsdPreviewSurface, MaterialX/OpenPBR and baked textures.
6. First-class interoperability: glTF/GLB, USD/USDA/USDC/USDZ, FBX (via verified adapter), OBJ, STL, PLY, EXR, PNG/TIFF, RAW heightfields, GeoTIFF, OpenVDB (where justified); verified in Blender, Maya, Unreal, Unity and web viewers; round-trip (JD → Blender → JD → engine).
7. Real-world input: location → DEM/contours/OSM features → terrain; images/video → depth (MoGe-2 single image, Depth Anything 3 multi-view) → terrain structure; lighting/colour analysis.
8. Agent-first: CLI, MCP server and versioned JSON schemas from Phase 1, sharing the Rust core with the GUI.
9. Resource efficiency: ≤ 4 GB RAM typical, measured; tiled/streaming large worlds.
10. Observability, stable error codes, security (Tauri capabilities, hardened parsers, scoped agent permissions, audit trail).

## Platform-native mandate (owner, 2026-10-19)

The app must feel native on each OS and use its raw hardware: **macOS** — Metal via wgpu, unified memory, Apple Silicon first; native window chrome, menu bar, ⌘ shortcuts, file associations, Quick Look-friendly USDZ. **Windows** — DX12 (fallback Vulkan) for NVIDIA GPUs, CPU SIMD path. **Linux** — Vulkan for NVIDIA, CPU path. CPU kernels are always present; GPU kernels are measured against them. Development and performance measurement happen on this Mac; Windows/Linux are validated through CI and reported honestly as `UNVERIFIED` where no hardware ran them.

## Technology stack (fixed unless an ADR justifies change)

| Layer | Choice |
|---|---|
| Desktop shell | Tauri 2 |
| Core | Rust (workspace of crates; see `docs/architecture/01-system-architecture.md`) |
| Frontend | TypeScript (framework per ADR-0004), Tailwind CSS, Lucide icons, Host Grotesk |
| Theme | Dark-first: black, charcoal, graphite, greys, white |
| GPU | wgpu (Metal / Vulkan / DX12), WGSL |
| Automation | CLI (`jd`), MCP server (`jd-mcp`), versioned JSON schemas |
| Tests | Rust unit/property/integration/snapshot/bench, frontend unit, interop validators, CI on macOS arm64 + Linux + Windows |

## Expected deliverables

**Research & architecture:** research reports + evidence register, competitive matrix, algorithm references, architecture spec with Mermaid diagrams, ADRs, interoperability matrix, licensing register.
**Development:** Rust core, desktop app, renderer/compute, graph engine, CLI, MCP server, format adapters, tests, benchmarks.
**Quality & ops:** error-code catalogue, logging/diagnostics docs, security model, profiling methodology, export-validation reports, fixtures, example projects.
**Community:** user/developer docs, contribution guide, extension API docs, roadmap, release notes.

## Evaluation criteria (definition of success)

A user can create a complex, convincing environment; edit it non-destructively; simulate environmental processes; optimise geometry and materials; export to industry formats; use the result reliably in Blender/Unity/Unreal/web; and reproduce the whole workflow through GUI, CLI and MCP.

**Mandatory acceptance scenario** (brief §20): mountains + valleys + river + coast → erosion + hydrology → ≥ 2 biomes → PBR via masks → lighting/atmosphere/weather → LOD meshes → baked textures/masks → GLB + USD/USDZ + FBX(adapter) → validators pass → Blender import → Unity/Unreal/web tests → losses recorded → reproduced via CLI and MCP → logs, cancellation and recovery verified. Nothing is "production-ready" until its acceptance tests pass; unavailable SDKs are marked `UNVERIFIED`.

## Phased roadmap (summary — detail in `docs/roadmap.md`)

1. **Research & foundation** — competitive analysis, licensing, architecture, project format, Rust core skeleton, rendering feasibility prototype, CLI/MCP foundations, CI/testing. *(Round 1 = research/docs part of this phase.)*
2. **Core terrain authoring** — noise/terrain generation, graph engine, viewport, editing, heightmaps, basic PBR, first exports, persistence.
3. **Simulation & procedural environments** — erosion, hydrology, biomes, scatter, weather, water/ocean, materials.
4. **Production interoperability** — formats, SDK adapters, Blender workflow, Unity/Unreal profiles, material conversion, validation, decimation/LOD.
5. **Scale & hardening** — streaming, caching, GPU optimisation, agent expansion, cross-platform validation, security, release.

## Long-term goals

A credible open-source alternative for professional procedural terrain and world authoring that combines artistic freedom, engineering rigour, scientific grounding, automation and dependable interoperability in one application, and a codebase maintainable by humans and agents alike (deterministic knowledge graph via `graphify`, living docs, ADRs).
