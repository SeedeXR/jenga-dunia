# JSON API Specification (v1)

Normative companion to `docs/architecture/07-agent-interfaces.md`. JSON Schemas are generated from Rust types with `schemars` and committed under `schemas/v1/` (`jd schema --write schemas/v1`; CI fails when they are stale): `envelope.json`, `ops/index.json`, `ops/<op>.request.json` / `.result.json`, `graph/node-kinds.json`. TypeScript types for every op (`JdOps`) and node kind (`NodeKinds`) are generated from them into `apps/desktop/src/lib/ops.gen.ts` by `pnpm types:gen` (dependency-free generator, CI fails when stale). This document fixes the envelope and conventions those schemas must satisfy.

## Envelope

| Field | Request | Result | Notes |
|---|---|---|---|
| `op` | required | echoed | `<subsystem>.<verb>` |
| `schema_version` | required | echoed | integer; server rejects unsupported versions with `E_SCHEMA_VERSION` |
| `request_id` | optional (ULID) | echoed/generated | correlation across logs |
| `params` | required object | — | validated against `schemas/v1/ops/<op>.request.json`; unknown fields rejected |
| `client` | optional | — | `{kind: gui|cli|mcp|plugin, name, scopes[]}` for audit |
| `status` | — | `ok` \| `accepted` \| `error` | |
| `result` | — | object | validated against `<op>.result.json` |
| `job_id` | — | with `accepted` | ULID |
| `warnings` | — | `[{code: "W_…", message, details}]` | non-fatal |
| `error` | — | `{code, message, why, next, details}` | see `error-codes.md` |

## Events (stream)

`{ "event": "progress" | "log" | "state", "job_id", "ts", ... }` — `progress` carries `pct` (0–100), `stage`, `detail`; `state` carries the job state machine transitions (`06-jobs.md`).

## Conventions

- All numbers in SI units (metres, seconds) unless the field name carries a unit suffix (`_px`, `_deg`).
- Identifiers are ULIDs; node kinds are `namespace.name@version`.
- Paths in params are project-relative unless the client holds an `fs:read:<path>` scope; absolute paths outside scope → `E_PERMISSION`.
- Enumerations are lowercase snake_case strings.
- Binary payloads are never inlined in JSON; results reference files or (in Tauri/MCP) channel/resource handles.
- Every op declares `scope` (e.g. `project:ro`, `project:rw`, `export`, `destructive`, `network:geo`, `vision`) and `async: bool`.

## Versioning

- `schema_version` increments only on breaking changes; additive fields are allowed within a version and documented in `CHANGELOG.md`.
- Deprecated ops remain callable for one minor release with a `W_DEPRECATED` warning.

## Example op definitions (initial set)

| op | scope | async | params (summary) | result (summary) |
|---|---|---|---|---|
| `project.create` | fs:write | no | `path`, `name`, `world` | `project_id`, `path` |
| `project.open` | project:ro | no | `path` | `project` summary, `warnings` (migrations) |
| `project.save` | project:rw | no | — | `saved_at` |
| `graph.list_nodes` | project:ro | no | — | `nodes[]` with kinds/params/ports |
| `graph.add_node` | project:rw | no | `kind`, `params?`, `ui?` | `node_id` |
| `graph.connect` | project:rw | no | `from`, `to` | — |
| `graph.set_param` | project:rw | no | `node_id`, `key`, `value` | `dirty_count` |
| `graph.evaluate` | project:rw | yes | `targets[]?`, `lod`, `region?` | `outputs[]` (hashes, stats) |
| `terrain.stats` | project:ro | no | `node_id` | min/max/mean height, slope histogram |
| `sim.erode` | project:rw | yes | `node_id` or inline spec | masks produced |
| `materials.bake` | export | yes | `targets[]`, `resolution`, `out_dir` | files, loss report |
| `geometry.decimate` | project:rw | yes | `mesh_id`, `target_error` or `ratio`, `preserve` | stats before/after |
| `geometry.validate` | project:ro | yes | `mesh_id` | checks with counts |
| `io.export` | export | yes | `format`, `profile?`, `out_path`, `options` | `files[]`, `report` |
| `io.import` | project:rw | yes | `path`, `as` | ids, loss report |
| `io.validate` | fs:read | yes | `path` | validator output |
| `jobs.status` | project:ro | no | `job_id?` | job(s) |
| `jobs.cancel` | project:rw | no | `job_id` | — |
| `diag.env` | — | no | — | environment |
| `diag.logs` | project:ro | no | `since`, `level`, `limit` | entries |
| `geo.fetch_dem` | network:geo | yes | `bbox` or `center+radius`, `source`, `resolution` | heightfield node id, attribution |
| `vision.estimate_depth` | vision | yes | `image_path`, `model` | depth/point map paths, intrinsics |
