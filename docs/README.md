# Jenga Dunia — Documentation Index (Single Source of Truth)

Round 1 (2026-10-19): research and architecture only; no code. Diagrams are Mermaid and validated in CI (`scripts/check-mermaid.mjs`). Every claim about external tools carries a VERIFIED / REPORTED / UNVERIFIED tag in the research reports.

## Start
- [`../memory/session_start.md`](../memory/session_start.md) — mandatory onboarding for agents and contributors
- [`../memory/project_context.md`](../memory/project_context.md) — purpose, deliverables, success criteria
- [`roadmap.md`](roadmap.md) — phases and exit criteria
- [`research/brief/WorldCreator-JengaDunia.md`](research/brief/WorldCreator-JengaDunia.md) — the master brief (authoritative scope)

## User guides (`user/`)
[Index](user/README.md) — [CLI quickstart](user/cli-quickstart.md) · [MCP setup](user/mcp-setup.md) · [Blender bridge](user/blender.md) · [Acceptance report (§20)](spec/acceptance-report.md)

## Architecture (`architecture/`)
| # | Doc |
|---|---|
| 00 | [Overview — context, containers, rules](architecture/00-overview.md) |
| 01 | [System architecture — crates, layering, process model](architecture/01-system-architecture.md) |
| 02 | [Subsystems and interactions](architecture/02-subsystems.md) |
| 03 | [Data model and data flow](architecture/03-data-flow.md) |
| 04 | [Procedural graph engine](architecture/04-graph-engine.md) |
| 05 | [Rendering and compute pipeline](architecture/05-rendering-compute.md) |
| 06 | [Jobs, cancellation, budgets, cache](architecture/06-jobs.md) |
| 07 | [Agent interfaces — ops, JSON, CLI, MCP](architecture/07-agent-interfaces.md) |
| 08 | [Import/export and canonical scene](architecture/08-io-formats.md) |
| 09 | [Plugins and extensions](architecture/09-plugins.md) |
| 10 | [Security model](architecture/10-security.md) |
| 11 | [Diagnostics, logging, errors](architecture/11-diagnostics.md) |
| 12 | [UI workspace and design language](architecture/12-ui-workspace.md) |
| 13 | [Geo and vision modules](architecture/13-geo-vision.md) |
| 14 | [Studio ↔ Editor protocol](architecture/14-studio-editor-protocol.md) |

## Decisions (`adr/`)
[ADR index](adr/README.md) — 0001 one core/three adapters · 0002 DAG graph · 0003 viewport hosting (prototype gate) · 0004 frontend stack · 0005 f32 tiles · 0006 erosion order · 0007 determinism · 0008 performance methodology · 0009 out-of-process plugins · 0010 REA/clean-room policy · 0011 project as directory · 0012 format strategy

## Specifications (`spec/`)
- [Project format `.jdproj`](spec/project-format.md)
- [JSON API envelope and ops](spec/json-api.md)
- [Error-code catalogue](spec/error-codes.md)
- [Performance targets and benchmark scenes](spec/performance-targets.md)
- [Interoperability matrix](spec/interoperability-matrix.md)

## Research (`research/`)
- [Evidence register](research/evidence-register.md) — provenance and confidence per report, follow-ups
- [Competitive capability matrix](research/competitive-matrix.md)
- [World Creator (primary reference)](research/world-creator.md)
- [Gaea and World Machine](research/gaea-world-machine.md)
- [Open-source terrain tools and REA](research/opensource-rea.md)
- [File formats, SDKs, engines, DCCs](research/formats-sdks.md)
- [Technology stack — Tauri, wgpu, Rust crates, MCP, CI](research/stack.md)
- [Algorithms — terrain, erosion, hydrology, LOD, materials, water, sky](research/algorithms.md)
- [Geospatial data and vision models](research/geo-vision.md)
- [Materials, textures and geomorphology references](research/materials-geomorphology.md)
- [Dependency and licensing register](research/licensing-register.md)

## Memory (`../memory/`)
`agent_profile.md` · `session_start.md` · `philosophy.md` · `project_context.md` · `instruction.md` · `mindmap.md` · `porting.md` · `todo.md` · `handover_session.md`
