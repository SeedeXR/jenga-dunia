# Driving Jenga Dunia from an AI agent (MCP)

`jd-mcp` exposes every `jd` operation as an MCP tool over stdio — same ids, same JSON schemas, same error codes. Tested with an MCP client against the real binary (`apps/mcp/tests/stdio.rs`).

## Build

```sh
cargo build --release -p jd-mcp
```

## Register with Claude Code

```sh
claude mcp add --transport stdio jenga-dunia -- /absolute/path/to/target/release/jd-mcp --scope all --workspace ~/Worlds
```

Or in a project `.mcp.json` (other MCP clients use the same `mcpServers` shape):

```json
{
  "mcpServers": {
    "jenga-dunia": {
      "command": "/absolute/path/to/target/release/jd-mcp",
      "args": ["--scope", "project:ro", "--scope", "project:rw", "--scope", "fs:write", "--workspace", "${JD_WORKSPACE:-./worlds}"]
    }
  }
}
```

## Scopes and workspace (agent safety)

| Flag | Effect |
|---|---|
| `--scope project:ro` | read-only tools (`project.open`, `project.info`, `project.close`) |
| `--scope project:rw` | mutations (`project.set_name`, `project.save`, `project.recover`) |
| `--scope fs:write` | `project.create` |
| `--scope all` | everything registered (future: `export`, `destructive`, `network:geo`, `vision`) |
| `--workspace DIR` | any `path` parameter outside `DIR` is refused with `E_PERMISSION` |

Tools the server was not granted are **not listed**; calling one anyway returns a structured error envelope with `E_PERMISSION`, never a crash. Default scopes when none are given: `project:ro`, `project:rw`, `fs:write`.

## What an agent sees

`tools/list` returns one tool per operation with its request schema as `inputSchema` and the result schema as `outputSchema`. `tools/call` returns the full Jenga Dunia envelope as structured content:

```json
{ "op": "project.open", "schema_version": 1, "request_id": "01M5…", "status": "ok",
  "result": { "project_id": "…", "name": "Hills", "path": "…", "world": { … }, "dirty": false, "pending_journal": 1 },
  "warnings": [ { "code": "W_RECOVERY_AVAILABLE", "message": "1 unsaved operation(s) found in the journal; run project.recover to replay them" } ] }
```

Errors carry `code`, `message`, `why`, `next` and `details`, so an agent can self-correct (e.g. fix parameters against the schema, or call `project.recover`).

**Resources and a prompt.** Beyond the tools, the server offers read-only resources and one prompt, so an agent can learn the craft as well as the API:

| Resource | What it holds |
|---|---|
| `jd://guide/reading-a-landscape` | From a reference photo, panorama or video to a believable world: the steps, the reading sheet, landform recipes as node graphs, measured real-terrain exemplars and realism checks ([the same guide](reading-a-landscape.md)) |
| `jd://guide/environments` | Soil, weather, sand deserts, snow-capped mountains and lava: which nodes and settings ([the same guide](environments.md)) |
| `jd://schema/node-kinds` | Every node kind with ports, description and parameter schema |
| `jd://schema/ops` | Every operation with its description and scope |

The prompt `terrain_from_reference {image, place?, intent?}` walks the agent through `media.metadata`, reading the image, `geo.fetch_dem` when the place is known, the simulation nodes, the sky, and `terrain.measure` against an exemplar, and ends with a report of every assumption. It needs the scopes of the tools it names: `network:geo` for the DEM and `vision` for photo depth. The resources are compiled into the binary, so they always match the build. Integration test: `apps/mcp/tests/stdio.rs` lists and reads them and gets the prompt through a real MCP client.

## Example session (Claude Code)

> "Create a 4 km world called Savannah in my workspace, rename it to Serengeti, save, and tell me the project id."

The agent calls `project.create` → `project.set_name` → `project.save` → reads `result.project_id`. Every call is logged with its `request_id` (`JD_LOG=info` on stderr).
