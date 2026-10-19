# Engineering Philosophy — Jenga Dunia

## Why this document exists

The reference tools (World Creator, Gaea, World Machine) are Windows-native, closed-source applications. Jenga Dunia is not a port of any of them; it is an independent, clean-room, cross-platform design that must run natively and efficiently on macOS (Apple Silicon first), Windows and Linux. This file states the beliefs that shape every architectural choice so that contributors and agents make consistent decisions without re-litigating them.

## 1. Understand the problem the incumbents solve, not their code

We study public documentation, published papers, demonstrations and black-box interoperability behaviour of existing tools to learn **what professionals need** (32-bit float pipelines, tiled builds, erosion flow/wear/deposit masks, engine bridges, deterministic rebuilds). We do not copy proprietary code, bypass technical protection measures or reuse reverse-engineered implementations. REA (see `docs/research/opensource-rea.md`) is used only on software we own or are authorised to inspect, and only to inform clean-room designs. The evidence trail lives in `docs/research/evidence-register.md`.

## 2. Native, lightweight, platform-aware

- **Rust core** owns all computation: noise, erosion, hydrology, meshing, decimation, materials, I/O. The UI is a thin, replaceable shell.
- **Apple Silicon first.** Unified memory means CPU↔GPU copies are cheap in bandwidth but not free in API cost; Metal via wgpu is the primary GPU backend. We design for it, then validate Vulkan (Linux) and DX12 (Windows). We do not assume feature parity across backends; capability is detected at runtime and degraded gracefully.
- **Memory ceiling of 4 GB** for typical 4k–8k workloads. Achieved through tiling with halos, f32 (not f64) heightfields, streaming, memory-mapped caches, bounded intermediate buffers and explicit GPU resource lifetimes. Measured, not assumed.
- **Shaders** are authored once in WGSL for the internal viewport; portable materials are expressed as data (glTF PBR, UsdPreviewSurface, MaterialX/OpenPBR where supported) and baked textures. We never promise identical shader execution across engines.

## 3. Correctness over appearance

A convincing terrain that exports non-manifold, degenerate or mis-scaled geometry is a defect. Topology validation, MikkTSpace tangents, explicit coordinate-system transforms (up-axis, handedness, units) and loss-reporting converters are foundational, not polish.

## 4. Non-destructive, deterministic, recoverable

Everything is a node graph with typed ports, versioned parameters and deterministic seeds. Evaluation is incremental, cached by content hash, cancellable, and reproducible on the same hardware. Projects are versioned JSON with migrations, autosaved, and recoverable after a crash. CPU and GPU paths may differ in the last bits; tolerances are defined, not ignored.

## 5. One core, many faces

GUI, CLI and MCP are three adapters over one `jd-core` API with versioned JSON schemas. If it can be clicked it can be scripted. Agents get scoped, auditable access, never raw OS access.

## 6. Research must change the code

Research reports exist to produce decisions (ADRs) and implementation priorities. A report that does not alter a design or a backlog item is incomplete.

## 7. Lazy senior engineering

The best code is the code not written. Reuse maintained, licence-compatible crates. No speculative abstractions. Delete before adding. Deliberate shortcuts are marked `ponytail:` with their ceiling and upgrade path. But never be lazy about understanding: read the whole flow before changing it.

## 8. Commercial open source, honestly

MIT core. Every dependency's licence is verified and registered. GPL/copyleft, proprietary SDKs (Autodesk FBX SDK) and non-commercial model weights live outside the core as optional, separately-installed adapters, clearly labelled. Attribution obligations (OSM ODbL, DEM providers) are rendered in the app and in exports' metadata.

## 9. Observability and security are features

Structured logs, stable error codes, job telemetry, diagnostic bundles with redaction, Tauri capability scoping, hardened parsers for untrusted files, signed releases, supply-chain audits. These are built in Phase 1, not retrofitted.

## 10. Finish whole workflows

Deliver complete, testable end-to-end paths (generate → erode → texture → optimise → export → validate → import elsewhere) rather than many partial features. The acceptance scenario in `project_context.md` is the yardstick.
