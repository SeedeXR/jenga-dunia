# Error Code Catalogue

Single source of truth for stable error codes used by the Rust core, GUI, CLI (exit codes) and MCP. Codes never change meaning; new codes are appended. Every error carries `message` (what happened), `why` (why it matters), `next` (what to do), `details` (machine context).

| Code | Category (brief §15.2) | Meaning | Typical `details` | CLI exit |
|---|---|---|---|---|
| `E_INPUT_INVALID` | Invalid input | Request failed schema/semantic validation | `path`, `expected`, `actual` | 2 |
| `E_GRAPH_CYCLE` | Invalid input | Graph contains a cycle | `nodes[]` | 2 |
| `E_GRAPH_TYPE_MISMATCH` | Invalid input | Edge connects incompatible port types | `from`, `to`, `from_type`, `to_type` | 2 |
| `E_GRAPH_MISSING_INPUT` | Invalid input | Required input port unconnected | `node`, `port` | 2 |
| `E_NODE_UNKNOWN` | Invalid input | Node kind not registered (plugin missing?) | `kind`, `version` | 2 |
| `E_FORMAT_UNSUPPORTED` | Unsupported formats | Format/feature not supported by any registered adapter | `format`, `feature` | 3 |
| `E_FORMAT_CORRUPT` | Unsupported formats | File failed to parse or violates its spec | `format`, `offset`, `reason` | 3 |
| `E_SERIALIZE` | Serialization failures | Document/graph/scene could not be (de)serialised | `type`, `schema_version` | 1 |
| `E_SCHEMA_VERSION` | Serialization failures | Document schema newer than this build, or migration failed | `found`, `supported` | 1 |
| `E_DEPENDENCY_MISSING` | Missing dependencies | Required external tool/library/plugin absent (Blender, FBX adapter, validator, model weights) | `dependency`, `how_to_install` | 1 |
| `E_GEOMETRY_INVALID` | Invalid geometry | Topology validation failed and operation requires valid geometry | `checks_failed[]`, `counts` | 6 |
| `E_NUMERIC_UNSTABLE` | Numerical instability | NaN/Inf or divergence detected in a kernel | `node`, `tile`, `iteration` | 1 |
| `E_SHADER_COMPILE` | Shader compilation | WGSL/naga compile or pipeline creation failed | `shader`, `log` | 1 |
| `E_GPU_DEVICE` | GPU device errors | Adapter missing, device lost, feature/limit unsupported | `backend`, `feature`, `limit` | 1 |
| `E_RESOURCE_MEMORY` | Insufficient memory | Job estimate or live RSS exceeds the configured budget | `estimated_bytes`, `limit_bytes`, `suggestion` | 4 |
| `E_RESOURCE_DISK` | Insufficient disk space | Not enough space for cache/export | `needed_bytes`, `available_bytes`, `path` | 4 |
| `E_PERMISSION` | Permission failures | Scope/capability denied (fs path, network host, destructive op) | `scope_required`, `path` | 7 |
| `E_TIMEOUT` | Timeouts | Job exceeded its deadline | `deadline_ms`, `elapsed_ms` | 5 |
| `E_CANCELLED` | Cancelled | Job cancelled by user/agent | `job_id` | 5 |
| `E_COMPAT` | Compatibility failures | Target profile cannot represent the data (e.g. Unity heightmap size not 2^n+1) | `target`, `constraint`, `actual` | 6 |
| `E_PROJECT_CORRUPT` | Corrupted project data | `.jdproj` or cache index unreadable; recovery attempted | `path`, `recovered`, `journal` | 1 |
| `E_NETWORK` | — | Remote fetch failed (DEM/OSM/model download) | `host`, `status`, `retryable` | 1 |
| `E_LICENSE_NOT_ACCEPTED` | — | Optional component requires accepting its licence first | `component`, `license`, `url` | 7 |
| `E_INTERNAL` | — | Unexpected failure (bug); diagnostic bundle suggested | `panic_message?` | 1 |

Rules:
- Adapters map crate-level `thiserror` enums to these codes at the `jd-core` boundary; crates never invent ad-hoc string codes.
- Warnings (non-fatal) use the same code namespace with `W_` prefix (e.g. `W_EXPORT_LOSS`, `W_VALIDATOR_MISSING`) and are listed in result envelopes.
- Tests: a snapshot test asserts the catalogue in code matches this table (generated section below once code exists).
