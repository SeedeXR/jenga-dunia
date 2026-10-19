# ADR-0011 — Project is a directory with versioned JSON + disposable cache

Status: Accepted · 2026-10-19

## Context
Brief §3.3 requires versioned, portable, update-resilient, crash-safe projects with caching and recovery. Gaea's JSON `.terrain` is marketed for source control; World Machine's `.tmd` and World Creator's project format are opaque (`competitive-matrix.md`).

## Decision
`.jdproj` is a **directory** (`docs/spec/project-format.md`): `project.json`, `graph.json`, `scene.json` (pretty, sorted keys, `schema_version` each), `assets/` (content-addressed), `strokes/`, `journal/` (write-ahead ops), `cache/` (disposable, content-addressed, SQLite index), `.lock`. Unknown keys preserved. Migrations tested per version. A zipped `.jdpkg` for sharing is a later feature.

## Alternatives rejected
- Single binary file — not diffable, atomic writes harder, cache entangled.
- Single JSON with embedded base64 assets — huge files, slow saves.
- SQLite as the whole project — opaque to git; keep SQLite only for the cache index.

## Consequences
- Git-friendly projects; cache in `.gitignore` template.
- Directory-as-document needs OS integration later (macOS bundle flag) — cosmetic.

## Evidence
`docs/research/gaea-world-machine.md` D4.27; brief §3.3, §14.4.
