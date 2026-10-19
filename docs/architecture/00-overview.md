# 00 — Architecture Overview

Status: **Design (Round 1, 2026-10-19)**. Nothing described here is implemented yet; see `docs/roadmap.md` for phases and `memory/todo.md` for the backlog. Diagrams are Mermaid and are the single source of truth for structure — update them with the code.

## Context (C4 level 1)

```mermaid
C4Context
  title Jenga Dunia — system context
  Person(artist, "Artist / TD", "Authors worlds in the desktop app")
  Person(dev, "Pipeline developer", "Scripts batch generation via CLI")
  System_Ext(agent, "AI agent (MCP client)", "Claude Code, IDE agents, custom automations")
  System(jd, "Jenga Dunia", "Procedural world generation & environmental simulation")
  System_Ext(dcc, "DCC tools", "Blender, Maya")
  System_Ext(engines, "Game engines", "Unreal, Unity")
  System_Ext(web, "Web viewers", "three.js, Babylon.js, model-viewer")
  System_Ext(geo, "Geospatial services", "DEM tiles, OSM/Overpass, land cover")
  System_Ext(models, "Vision models (optional, downloaded)", "MoGe-2, Depth Anything 3")
  Rel(artist, jd, "GUI")
  Rel(dev, jd, "CLI / JSON")
  Rel(agent, jd, "MCP tools & resources")
  Rel(jd, dcc, "glTF / USD / OBJ / FBX-adapter; Blender bridge add-on")
  Rel(jd, engines, "Heightmaps, weightmaps, GLB/USD/FBX, engine profiles")
  Rel(jd, web, "Optimised GLB (meshopt, KTX2)")
  Rel(jd, geo, "HTTPS fetch, cached, attributed")
  Rel(jd, models, "Local inference via ONNX Runtime (opt-in)")
```

## Containers (C4 level 2)

```mermaid
C4Container
  title Jenga Dunia — containers
  Person(user, "User")
  System_Ext(agent, "MCP client")
  Container_Boundary(app, "Jenga Dunia") {
    Container(ui, "Desktop frontend", "TypeScript + Tailwind in Tauri WebView", "Node editor, viewport host, inspectors, job monitor")
    Container(tauri, "Tauri shell", "Rust", "Windows, IPC commands/events/channels, capability ACL, tile streaming to the viewport")
    Container(core, "jd-core API", "Rust", "Versioned operations: project, graph, terrain, sim, materials, geometry, io, jobs")
    Container(engine, "Domain crates", "Rust", "graph, terrain, sim, materials, geometry, io, geo, vision, gpu")
    Container(cli, "jd CLI", "Rust (clap)", "Same operations, JSON output, exit codes")
    Container(mcp, "jd-mcp", "Rust (rmcp)", "MCP tools/resources, stdio + streamable HTTP, scoped permissions")
    ContainerDb(project, "Project on disk", ".jdproj (versioned JSON) + cache/ (content-addressed tiles)", "")
  }
  Rel(user, ui, "")
  Rel(ui, tauri, "IPC (typed commands, progress channels)")
  Rel(tauri, core, "")
  Rel(cli, core, "")
  Rel(mcp, core, "")
  Rel(agent, mcp, "MCP")
  Rel(core, engine, "")
  Rel(core, project, "read/write, autosave, recovery")
```

## Key architectural rules

| Rule | Consequence |
|---|---|
| **One core, three adapters.** GUI, CLI and MCP call `jd-core` operations with identical JSON schemas. | No business logic in the frontend, Tauri command handlers, CLI subcommands or MCP tool bodies — they marshal only. |
| **Rust owns computation.** | The WebView never computes terrain; it receives previews (textures/meshes) through channels. |
| **Graph engine is UI-independent.** | Graph evaluation is a library with no Tauri/WebView dependency; CLI and MCP evaluate graphs headless. |
| **Tiles are the unit of work.** | Heightfields and masks are f32 tiles (default 512², configurable) with halos; jobs stream tiles; memory is bounded. |
| **Determinism by construction.** | Seeds are explicit, PRNG is PCG/xoshiro, parallel reductions are ordered; CPU/GPU paths have documented tolerances. |
| **Interop is data, not shader code.** | Internal WGSL never leaves the app; exports carry glTF PBR / UsdPreviewSurface / MaterialX + baked textures with a loss report. |
| **Untrusted input everywhere.** | Imported files, plugins, downloaded assets and agent inputs go through hardened parsers and scoped capabilities. |
| **Measured performance.** | Every kernel has a bench; CI tracks peak RSS; the 4 GB ceiling is a test, not a hope. |

## Document map

| Doc | Content |
|---|---|
| `01-system-architecture.md` | Crate workspace, layering, dependency rules |
| `02-subsystems.md` | The 15 subsystems and their interactions |
| `03-data-flow.md` | Core data model (tiles, scene), end-to-end flows |
| `04-graph-engine.md` | Node graph model, evaluation, caching, dirty propagation |
| `05-rendering-compute.md` | Viewport hosting, render pipeline, compute dispatch, CPU/GPU parity |
| `06-jobs.md` | Scheduler, job state machine, cancellation, resource budgets |
| `07-agent-interfaces.md` | CLI, MCP, JSON envelope, permissions, audit |
| `08-io-formats.md` | Canonical scene representation, import/export adapters, conversion correctness |
| `09-plugins.md` | Extension architecture and isolation of licence-incompatible adapters |
| `10-security.md` | Threat model, Tauri capabilities, supply chain |
| `11-diagnostics.md` | Logging, error codes, telemetry, diagnostic bundles |
| `12-ui-workspace.md` | Workspace layout, design language, interaction principles |
| `13-geo-vision.md` | Location-to-terrain and image/video-to-terrain pipelines |
