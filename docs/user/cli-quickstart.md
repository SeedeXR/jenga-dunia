# CLI quickstart — `jd`

Status: Phase 1 (project management and diagnostics). Terrain generation, export and the desktop app arrive in Phase 2. Every output below was produced by the real binary on 2026-10-19 (macOS, Apple Silicon); ids and paths will differ on your machine.

## Build

```sh
cargo build --release -p jd-cli -p jd-mcp
./target/release/jd --help
```

## 1. Create a project

A project is a **directory** (`project.json`, `graph.json`, `scene.json`, `assets/`, `cache/`, `journal/`, …) — see `docs/spec/project-format.md`.

```
$ jd new ~/Worlds/Hills.jdproj --extent 2048 --resolution 1024 --seed 7
ok  project.create
{
  "project_id": "01M5A8NQE0HZV13SJFG7GV1B2P",
  "name": "Hills",
  "path": "~/Worlds/Hills.jdproj",
  "world": {
    "extent_m": [2048.0, 2048.0],
    "resolution_px": [1024, 1024],
    "tile_size": 512,
    "halo": 4,
    "origin": [0.0, 0.0],
    "seed": 7
  },
  "schema_version": 1,
  "dirty": false,
  "pending_journal": 0
}
```

## 2. Open it

```
$ jd open ~/Worlds/Hills.jdproj
ok  project.open
{ ... same summary ... }
```

If the app or a script crashed with unsaved changes, `open` warns `W_RECOVERY_AVAILABLE`; replay them with:

```
$ jd open ~/Worlds/Hills.jdproj --recover
```

## 3. Run any operation (scripting / agents)

Everything the GUI and the MCP server can do is an *operation*. `jd op` invokes one by id with JSON parameters; `--json` prints one envelope per line (NDJSON) so scripts can parse it.

```
$ jd op project.set_name --project ~/Worlds/Hills.jdproj --params '{"name":"Green Hills"}' --save --json
{"op":"project.open","schema_version":1,"request_id":"01M5A8NQF7RDN5JKK9ZBG8NGFG","status":"ok","result":{...}}
{"op":"project.set_name","schema_version":1,"request_id":"01M5A8NQFCYE81XEP1EM8SW2GR","status":"ok","result":{"name":"Green Hills",...}}
{"op":"project.save","schema_version":1,"request_id":"01M5A8NQFGZKV78R22PJ39KT62","status":"ok","result":{...,"dirty":false}}
```

List operations and print a schema:

```
$ jd schema --list
[
  { "id": "diag.env",       "description": "Report host environment and detected tools", "scope": "none" },
  { "id": "project.close",  "description": "Close the open project (unsaved changes stay in the journal)", "scope": "project-read" },
  { "id": "project.create", ... }, { "id": "project.info", ... }, { "id": "project.open", ... },
  { "id": "project.recover", ... }, { "id": "project.save", ... }, { "id": "project.set_name", ... }
]
$ jd schema project.create      # full request/result JSON Schema
```

## 4. Diagnostics

```
$ jd diag env
ok  diag.env
{
  "app_version": "0.1.0",
  "os": "Darwin 27.0.1",
  "arch": "aarch64",
  "cpus": 10,
  "memory_total_bytes": 17179869184,
  "memory_available_bytes": 6776995840,
  "cwd": "~/Documents/SoftwareProjects/jenga-dunia",
  "tools": [
    { "name": "blender" },
    { "name": "usdchecker", "path": "/usr/bin/usdchecker" },
    { "name": "usdcat",     "path": "/usr/bin/usdcat" },
    { "name": "usdzip",     "path": "/usr/bin/usdzip" },
    { "name": "gltf-validator" }, { "name": "gltf-transform" }, { "name": "toktx" }
  ]
}
```

A tool without `path` was not found on `PATH`; exporters that need it will report `E_DEPENDENCY_MISSING` with install hints.

## Errors and exit codes

Errors always say what happened, why it matters and what to do next, with a stable code (`docs/spec/error-codes.md`):

```
$ jd open /nowhere.jdproj
error  project.open  E_PROJECT_CORRUPT: cannot read project.json: No such file or directory (os error 2)
  why:  The project directory is missing a required document.
  next: Restore the file from a backup or recreate the project.
$ echo $?
1
```

| Exit | Meaning |
|---|---|
| 0 | ok |
| 1 | generic / internal / corrupt project |
| 2 | invalid input or unknown operation |
| 3 | unsupported or corrupt file format |
| 4 | out of memory / disk |
| 5 | cancelled or timed out |
| 6 | validation or compatibility failure |
| 7 | permission denied / licence not accepted |

Set `JD_LOG=debug` (or `--log`) for structured logs on stderr.
