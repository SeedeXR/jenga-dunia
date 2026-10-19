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

## 4. Generate terrain and export a heightmap

`jd terrain generate` opens the project, generates a noise heightfield for the project's world (extent, resolution, tiles from `project.json`) and optionally exports it in one go. The format follows the extension: `.exr` (32-bit float, exact), `.tif` (32-bit float TIFF, exact — GIS/DCC tools), `.png` (16-bit, quantised), `.r16` (Unity/Unreal raw), `.r32` (raw float).

```
$ jd terrain generate --project ~/Worlds/Hills.jdproj --seed 1 --fractal ridged --out ridges.exr
ok  project.open
{ ... }
ok  terrain.generate
{
  "layer": "height",
  "size_px": [4096, 4096],
  "tile_size": 512,
  "halo": 4,
  "stats": { "min": 42.47209, "max": 500.00003, "mean": 221.9, "non_finite": 0 },
  "content_hash": "…blake3…",
  "duration_ms": 157
}
ok  terrain.export_heightmap
{
  "path": "~/Worlds/Hills.jdproj/exports/ridges.exr",
  "sidecar": "~/Worlds/Hills.jdproj/exports/ridges.exr.jd-height.json",
  "format": "exr", "size_px": [4096, 4096], "range_m": [42.47209, 500.00003],
  "quantisation_step_m": 0.0, "flip_y": true, "bytes": 62666077
}
```

Options: `--kind open_simplex2|open_simplex2s|perlin|value|value_cubic|cellular`, `--fractal none|fbm|ridged|ping_pong`, `--feature-size <m>`, `--octaves 1..16`, `--min/--max <m>`, `--layer <name>`.

Quantised formats warn so nothing is lost silently, and the sidecar records the range needed to reconstruct heights:

```
$ jd terrain generate --project ~/Worlds/Hills.jdproj --out h.png
warning  W_EXPORT_LOSS: Png16 quantises heights to 0.0064 m steps; range recorded in the sidecar
```

Same seed → same `content_hash`, on any thread count and tile size (tested). Measured on an Apple Silicon laptop (10 cores): 4096² generation 157 ms, whole command 0.42 s, peak RSS 235 MB; the EXR is read by macOS ImageIO as 4096×4096, 32 bits/sample, and the PNG as 16-bit grayscale.

### Export the terrain as a mesh (glTF binary)

```
$ jd --json export mesh --project M.jdproj terrain.glb
{"op":"terrain.export_mesh","status":"ok","result":{"path":"M.jdproj/exports/terrain.glb","format":"glb",
  "vertices":262144,"triangles":522242,"step":1,"bytes":14656644,"extent_m":[2048.0,2048.0]}}
```

(512² world.) The `.glb` is a single grid mesh in metres, +Y up, centred on the world like the viewport, with normals, UVs and a flat PBR material — it opens directly in Blender, three.js or any glTF 2.0 importer, and the Khronos glTF Validator reports 0 errors (CI runs it on every push). Worlds above ~1 M vertices are sampled every `step` pixels (`W_MESH_DECIMATED`); pass `--step 1` for the full grid.

### Bring in a real DEM or engine heightmap

```
$ jd op project.import_asset --project K.jdproj --params '{"path":"~/Downloads/srtm_tile.tif"}'
$ jd op graph.add_node --project K.jdproj --save \
    --params '{"kind":"jd.input.heightmap@1","params":{"path":"srtm_tile.tif"}}'
```

The input node reads from the project's `assets/` only (no absolute paths, no `..`), accepts PNG 16-bit, EXR, TIFF (f32/u16/u8) and headerless RAW (`size_px` required), maps quantised files through `range_m`, and resamples bilinearly onto the world grid. Wire it to filters, erosion or an output like any generator.

## 5. Build a procedural graph

Terrain is really a **graph** (`graph.json` in the project): generator → filters → output. Every graph change is a journaled operation (crash-safe, undoable later); evaluation is incremental — unchanged nodes are served from cache by content hash.

```
$ jd op graph.kinds | head          # node kinds with ports and parameter schemas
$ jd --json op graph.add_node --project G.jdproj --save \
    --params '{"kind":"jd.noise.fbm@1","params":{"seed":9,"fractal":"ridged","height_range_m":[0,300]}}'
{"op":"graph.add_node", ..., "result":{"id":"01M5AYXA8NG9R58WS7RRM2RMRM"}}
$ jd op graph.add_node --project G.jdproj --save --params '{"kind":"jd.filter.blur@1","params":{"radius":2}}'
$ jd op graph.add_node --project G.jdproj --save --params '{"kind":"jd.output.height@1"}'
$ jd op graph.connect  --project G.jdproj --save --params '{"from":"<noise id>","to":"<blur id>","to_port":"in"}'
$ jd op graph.connect  --project G.jdproj --save --params '{"from":"<blur id>","to":"<output id>","to_port":"in"}'
$ jd --json op graph.evaluate --project G.jdproj --params '{"lod":0}'
{"op":"graph.evaluate","status":"ok","result":{"lod":0,"duration_ms":20,
  "nodes":[{"kind":"jd.noise.fbm@1","duration_ms":15,"cached":false,"hash":"…"},
           {"kind":"jd.filter.blur@1","duration_ms":3,"cached":false,"hash":"…"},
           {"kind":"jd.output.height@1","duration_ms":0,"cached":false,"hash":"…"}],
  "layers":{"height":"55fcde70c90c7fd7…"}}}
```

(512² world, release build.) Every mutation is validated **before** it is journaled, so a rejected request never touches the project: unknown kinds (`E_NODE_UNKNOWN`), parameters that do not match the kind's schema, missing nodes or ports, type mismatches (`E_INPUT_INVALID` / `E_GRAPH_TYPE_MISMATCH`) and cycles (`E_GRAPH_CYCLE`). Errors name the node:

```
$ jd --json op graph.set_param --project G.jdproj --params '{"id":"<noise id>","key":"octaves","value":"many"}'
{"op":"graph.set_param","status":"error","error":{"code":"E_INPUT_INVALID",
  "message":"invalid type: string \"many\", expected u8","why":"Parameters must match the node kind's schema.",
  "next":"See graph.kinds for the schema.","details":{"path":"params","node":"01JDGRAPH000000000000N01SE"}}}
$ jd --json op graph.connect --project G.jdproj --params '{"from":"<noise id>","from_port":"nope","to":"<output id>","to_port":"in"}'
{"op":"graph.connect","status":"error","error":{"code":"E_INPUT_INVALID",
  "message":"node 01JDGRAPH000000000000N01SE (jd.noise.fbm@1) has no output `nope`", ...}}
```

Evaluating with an unconnected required input gives `E_GRAPH_MISSING_INPUT`. `graph.move_node` (editor position) is journaled like every other change, so the desktop canvas layout survives a crash. `graph.undo` / `graph.redo` step through the session's edit history (up to 100 steps) and are journaled as `graph.restore` with the full graph, so recovery replays them too; every `graph.*` response carries `undo_depth` / `redo_depth`. `graph.evaluate` with `lod > 0` publishes a reduced-resolution preview under `<layer>#lod<N>` (e.g. `height#lod1`) and never overwrites the full-resolution layer.

Layers persist in the project's disposable cache (`cache/layers/<blake3>.jdl` + `index.json`, see `docs/spec/project-format.md`), so one invocation can evaluate and the next one export — the same path the desktop app and MCP server use:

```
$ jd --json op graph.evaluate --project G.jdproj
{"op":"graph.evaluate","status":"ok","result":{"lod":0,"duration_ms":194,"layers":{"height":"5e12d1d98c5c7402…"}}}
$ ls G.jdproj/cache/layers
5e12d1d98c5c7402….jdl  index.json
$ jd --json op terrain.export_heightmap --project G.jdproj --params '{"path":"height.exr","format":"exr"}'
{"op":"terrain.export_heightmap","status":"ok","result":{"path":"G.jdproj/exports/height.exr",
  "sidecar":"G.jdproj/exports/height.exr.jd-height.json","format":"exr","size_px":[512,512],
  "range_m":[181.76539611816406,392.8346862792969],"quantisation_step_m":0.0, ...}}
```

(512² world, debug build.) Deleting `cache/` never breaks a project; the next `graph.evaluate` recreates the layers.

The first simulation node is thermal erosion. Insert it between the starter noise and output (ids from `graph.list`):

```
$ jd --json op graph.add_node --project K.jdproj --save \
    --params '{"kind":"jd.erosion.thermal@1","params":{"talus_deg":33,"iterations":50}}'
$ jd op graph.connect --project K.jdproj --save --params '{"from":"<noise id>","to":"<thermal id>","to_port":"in"}'
$ jd op graph.connect --project K.jdproj --save --params '{"from":"<thermal id>","to":"<output id>","to_port":"in"}'
$ jd --json op graph.evaluate --project K.jdproj
{"op":"graph.evaluate","status":"ok","result":{"lod":0,"duration_ms":952,
  "nodes":[{"kind":"jd.noise.fbm@1","duration_ms":64,...},{"kind":"jd.erosion.thermal@1","duration_ms":887,...},
           {"kind":"jd.output.height@1","duration_ms":0,...}],"layers":{"height":"…"}}}
```

(2048² world, 50 steps, release build on the M-series reference Mac: 0.98 s wall, 130 MB peak RSS; 512² takes ~65 ms.) Material steeper than `talus_deg` slides to lower neighbours; the step is mass-conserving, so `terrain.stats` reports the same mean before and after.

Built-in kinds today: `jd.noise.fbm@1`, `jd.math.add@1`, `jd.math.multiply@1`, `jd.math.scale@1`, `jd.math.remap@1`, `jd.filter.blur@1`, `jd.erosion.thermal@1` (talus angle, iterations, rate — mass-conserving; 2048² × 50 steps ≈ 0.9 s release), `jd.analysis.slope@1` (0..1 steepness mask; multiply it into a layer to attenuate by slope), `jd.erosion.hydraulic@1` (virtual-pipe shallow water with conservative sediment transport; outputs `out`, `water`, `sediment`; 2048² × 100 steps ≈ 2.1 s release), `jd.sculpt.strokes@1` (brush strokes as data: `points` in layer metres, `radius_m`, `mode` raise/smooth/flatten with `strength_m` or `amount`), `jd.analysis.flow@1` (D8 flow accumulation, log-scaled 0..1 — rivers/wetness), `jd.input.heightmap@1` (a PNG-16/EXR/TIFF/RAW file from the project's `assets/`, resampled onto the world; `project.import_asset` copies files in), `jd.output.height@1`.

## 6. Diagnostics

```
$ jd diag env
ok  diag.env
{
  "app_version": "0.0.1",
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
