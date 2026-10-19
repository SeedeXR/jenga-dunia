# Project Format Specification (`.jdproj`)

Status: Design v1 (schema_version 1). Changes require an ADR and a migration.

## Goals
Portable, human-diffable, resilient to application updates, recoverable after crashes, cache-independent.

## Layout on disk

```
MyWorld.jdproj/               # a directory, not a single file (Finder/Explorer show as folder; "package" on macOS via bundle flag later)
├─ project.json               # document root (schema_version, metadata, settings, graph ref, scene ref)
├─ graph.json                 # procedural graph (nodes, edges, params, seed, world settings)
├─ scene.json                 # canonical scene (composition, materials, environment, cameras)
├─ assets/                    # user-imported assets (textures, models, stamps, masks, DEMs) — content-addressed filenames + manifest
│  └─ manifest.json
├─ strokes/                   # sculpt/paint stroke data (binary, zstd) referenced by stroke nodes
├─ cache/                     # disposable: content-addressed tiles + index.sqlite (gitignored by default)
├─ exports/                   # default export target (user-configurable)
├─ journal/                   # crash-recovery journal (append-only ops since last save)
└─ .lock                      # advisory lock with pid/host
```

Rationale: a directory keeps large binary assets out of JSON, allows partial writes to be atomic per file, and lets the cache be deleted safely. `ponytail:` a single-file zip "package" export (`.jdpkg`) is a later feature for sharing.

## `project.json`

```json
{
  "schema_version": 1,
  "app_version": "0.0.1",
  "id": "01JABCDEF...",
  "name": "MyWorld",
  "created": "2026-10-19T10:00:00Z",
  "modified": "2026-10-19T10:30:00Z",
  "units": { "metersPerUnit": 1.0, "upAxis": "Y", "handedness": "right" },
  "world": { "extent_m": [8192, 8192], "resolution_px": [8192, 8192], "tile_size": 512, "halo": 4, "origin": [0.0, 0.0], "crs": null, "seed": 42 },
  "files": { "graph": "graph.json", "scene": "scene.json", "assets": "assets/manifest.json" },
  "settings": { "autosave_interval_s": 120, "cache_limit_bytes": 2147483648, "preview_lod": 2 },
  "attribution": [ { "source": "Copernicus DEM GLO-30", "text": "© DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved", "url": "..." } ],
  "extensions": {}
}
```

- `schema_version` governs the whole project; each sub-document also carries its own `schema_version`.
- Unknown keys are **preserved** on load/save (forward compatibility) and reported as warnings.
- `extensions` is namespaced (`"com.example.plugin": {...}`) for plugins.
- `attribution` accumulates data-source obligations and is written into export metadata.

## `graph.json`

See `docs/architecture/04-graph-engine.md`. Node `kind` is `namespace.name@version`. Params are JSON values validated against the node's schema. Stable key ordering on save (BTreeMap) for clean diffs.

## `scene.json` (Canonical Scene Representation)

See `docs/architecture/08-io-formats.md`. Declares units/up-axis/handedness explicitly; everything else is converted at export.

## Migrations

`jd-core` holds `migrate(doc, from_version) -> doc` steps, tested with fixtures for every historical version. Opening a newer document than supported fails with `E_SCHEMA_VERSION` (never silently).

## Autosave and recovery

- Every mutating op is appended to `journal/ops.ndjson` before being applied (write-ahead). On save, the journal is truncated.
- Autosave writes to `*.json.tmp` then renames (atomic on all three platforms).
- On open, if a journal exists and is newer than the save, the user is offered recovery (replay).
- `.lock` prevents concurrent edits; stale locks (dead pid) are overridden with a warning.

## Determinism inputs

The tuple (`graph.json`, `world`, `app_version` node versions, asset hashes, `preview_lod`) fully determines outputs for the CPU path; GPU path differences are bounded per node (`docs/spec/performance-targets.md` → determinism tolerances).
