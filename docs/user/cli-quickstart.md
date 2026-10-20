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

Long operations register a job: `jd op jobs.list` shows running jobs with progress (`done/total`, current node) and `jd op jobs.cancel --params '{"id":1}'` stops one with `E_CANCELLED` (exit code 5). In one CLI process this is mostly useful to agents driving the MCP server, where `jobs.list` / `jobs.cancel` are tools like any other.

**Detached evaluation for agents.** In a long-lived process (the MCP server, the desktop app) `graph.evaluate_async` returns immediately with `status: accepted` and a `job_id`; `jobs.list` shows its progress, `jobs.cancel {id}` stops it, and `jobs.result {id}` hands over the finished op's envelope exactly once (`state: running | done | unknown`). Driven through the real MCP server on a 1024² world:

```
graph.evaluate_async → status accepted, job_id 1, returned after 3 ms
jobs.result 1        → running 0/2 jd.noise.fbm@1 … running 1/2 jd.output.height@1
jobs.result 1        → done after 444 ms: envelope status ok, layers ["height"]
jobs.result 1        → unknown (collected once)
graph.evaluate_async (hydraulic × 2000) → job_id 2; jobs.list: 2:graph.evaluate 1/3 jd.erosion.hydraulic@1
jobs.cancel 2        → true; jobs.result 2 → done: status error, E_CANCELLED, details.job_id 2
```

Detached jobs run on a bounded pool (half the cores); extra ones wait with `state: queued` in `jobs.list`, and cancelling a queued job ends it without running. Pass `timeout_s` to `graph.evaluate_async` to have the job cancelled at a deadline — `jobs.result` then reports `E_TIMEOUT` with the job id.

(One CLI invocation is one process, so `jd op graph.evaluate_async` alone is not useful — the job dies with the process; use `jd op graph.evaluate`.)

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

(512² world.) `.glb` is the default; `.obj`, `.stl` and `.ply` work the same way (below). The `.glb` is a single grid mesh in metres, +Y up, centred on the world like the viewport, with normals, UVs and a PBR material whose base colour is a baked texture — the viewport's hypsometric ramp (lowland green → ochre → rock → snow) blended towards rock on steep slopes, one texel per vertex — so it opens looking like the app in Blender, three.js or any glTF 2.0 importer. `--no-texture` (op param `texture: false`) writes the flat material instead. The Khronos glTF Validator reports 0 errors and 0 warnings for both (CI runs it on every push); on a 512² world the texture adds 0.42 MB to a 14.7 MB mesh.

![The textured GLB rendered in an independent glTF viewer: iso and top views with green valleys, ochre mid-slopes, grey rock on steep faces and snow on the peaks](images/cli-glb-textured.png)

*The exported `terrain.glb` opened in a third-party viewer (Optima), not in Jenga Dunia — the colours come from the embedded texture alone.*

The same command writes **OBJ**, binary **STL** and binary **PLY** by extension — `jd export mesh --project X.jdproj terrain.obj` (positions, UVs, normals, `v/vt/vn` faces, no MTL), `terrain.stl` (face normals, the usual unwelded STL vertices) and `terrain.ply` (normals plus the hypsometric colours as per-vertex RGB, for MeshLab/CloudCompare/Blender). All three share the GLB's grid, frame (metres, +Y up, south at +Z) and `--step` decimation. On the 512² world at step 2 (66 049 vertices, 131 072 triangles): OBJ 11.9 MB in 0.11 s, STL 6.25 MB in 0.12 s, PLY 3.33 MB in 0.08 s (debug build); an external importer reports 0 flipped, degenerate or non-manifold faces for each.

![The PLY export rendered in an external viewer with its vertex colours](images/cli-mesh-ply.png)

**OpenUSD.** `terrain.usda` writes an ASCII USD stage (`metersPerUnit = 1`, `upAxis = "Y"`, `defaultPrim = "Terrain"`, a `Mesh` with points, triangles, vertex normals, `primvars:st` and hypsometric `primvars:displayColor`); `terrain.usdz` packages it with the system `usdzip` (Apple USD Tools on macOS 27, OpenUSD elsewhere — `E_DEPENDENCY_MISSING` names the install when it is absent). Both pass `usdchecker` ("Success!") on the 512² world at step 4 (16 641 vertices: 2.45 MB, 0.07 s for `.usda`, 0.14 s for `.usdz`). USDA is text — for large worlds export `.glb` or `.ply`, or let `usdcrush` compress the package.
 Worlds above ~1 M vertices are sampled every `step` pixels (`W_MESH_DECIMATED`); pass `--step 1` for the full grid.

### Hydrology recipe: rivers, lakes and wetness

```
$ jd --json new Rivers.jdproj --extent 4096 --resolution 512
$ jd op graph.add_node --project Rivers.jdproj --save --params '{"kind":"jd.hydrology.rivers@1","params":{"threshold":0.6,"depth_m":25}}'
$ jd op graph.add_node --project Rivers.jdproj --save --params '{"kind":"jd.output.height@1","params":{"layer":"rivers"}}'
$ jd op graph.add_node --project Rivers.jdproj --save --params '{"kind":"jd.hydrology.wetness@1"}'
$ jd op graph.add_node --project Rivers.jdproj --save --params '{"kind":"jd.output.height@1","params":{"layer":"wetness"}}'
# connect noise.out → rivers.in, rivers.out → height.in, rivers.rivers → rivers-output.in,
# rivers.out → wetness.in, wetness.out → wetness-output.in (graph.connect with from_port / to_port)
$ jd --json op graph.evaluate --project Rivers.jdproj
{"op":"graph.evaluate","status":"ok","result":{"lod":0,"duration_ms":663,
  "nodes":[{"kind":"jd.noise.fbm@1","duration_ms":197,...},{"kind":"jd.hydrology.rivers@1","duration_ms":321,...},
           {"kind":"jd.hydrology.wetness@1","duration_ms":143,...}, ...],
  "layers":{"height":"…","rivers":"…","wetness":"…"}}}
$ jd op terrain.stats --project Rivers.jdproj --params '{"layer":"rivers"}'    # min 0, max 0.844, mean 0.002
$ jd op terrain.export_heightmap --project Rivers.jdproj --params '{"layer":"rivers","path":"rivers.png","format":"png16","range_m":[0,1]}'
```

![River masks from the same ridged-noise terrain: D8 (left) and D∞ (right); the circle marks a junction where D∞ splits flow between two downslope cells](images/cli-rivers-d8-vs-dinf.png)

Measured on this 512² world (debug build): the rivers node takes 329 ms with `d8`, 333 ms with `dinf` and 372 ms with `mfd`; the share of channel cells sitting on a perfectly straight two-neighbour run is 77 % (d8), 68 % (dinf) and 20 % (mfd), and the river mask covers 3 167 / 3 288 / 4 614 cells respectively. `d8` and `dinf` stay close — the fill step and the threshold dominate the drainage pattern — so switch to `dinf` when you see parallel 45° streaks on planar slopes; `mfd` gives the widest, most natural-looking valleys at the cost of sharper channel edges.

### Sea level and coastlines

```
$ jd op graph.add_node --project Coast.jdproj --save --params '{"kind":"jd.hydrology.sea_level@1","params":{"level_m":0,"shore_m":5}}'
$ jd op graph.add_node --project Coast.jdproj --save --params '{"kind":"jd.output.height@1","params":{"layer":"ocean"}}'
$ jd op graph.add_node --project Coast.jdproj --save --params '{"kind":"jd.output.height@1","params":{"layer":"depth"}}'
# noise.out → sea_level.in, sea_level.ocean → ocean-output.in, sea_level.depth → depth-output.in
$ jd --json op graph.evaluate --project Coast.jdproj
$ jd export heightmap --project Coast.jdproj --layer ocean ocean.png
```

![Noise terrain with the ocean mask in blue; a landlocked basin below sea level is marked in red and is not ocean](images/cli-sea-level.png)

`ocean` is 1 only where the terrain is below `level_m` **and** connected to the field border through such cells, so inland basins below sea level stay dry (fill them with `jd.hydrology.fill@1` or raise them instead); `shore_m` fades the mask to 0 over that many metres above the level for soft beaches, and `depth` is the water column in metres. On the 512² world above (noise −150..350 m, level 0 m): 24 836 cells lie below 0 m, 13 691 of them are ocean and 11 176 are landlocked; the node took 73 ms (debug build).


(512² world, debug build.) `jd.hydrology.rivers` fills depressions, routes flow (`method` `d8` by default; `dinf` — Tarboton's D∞, which splits flow between two downslope neighbours and removes D8's parallel-line streaks; `mfd` — Freeman's multiple flow direction, sharing flow among all lower neighbours by slope^1.1 for the most diffuse, hillslope-like pattern) and incises channels where accumulation is high; `wetness` is the topographic wetness index. Every output node publishes its own layer, so the desktop's layer picker shows **height**, **rivers** and **wetness** side by side, and each can be exported as a mask.

### Droplet (particle) erosion

```
$ jd op graph.add_node --project Drop.jdproj --save --params '{"kind":"jd.erosion.droplet@1","params":{"droplets":400000,"seed":1}}'
$ jd op graph.add_node --project Drop.jdproj --save --params '{"kind":"jd.output.height@1","params":{"layer":"delta"}}'
# noise.out → droplet.in, droplet.out → height output, droplet.delta → delta output
```

![Hillshade of a 512² fBm terrain before and after 400 000 droplets: gullied ridges and flat deposition plains](images/cli-droplet.png)

`jd.erosion.droplet@1` is the Lagrangian model from Beyer's 2015 thesis that most terrain tools ship: each seeded droplet starts with water and speed, follows the slope with `inertia`, carries sediment up to `capacity × slope × speed × water`, erodes the deficit through a `radius` brush, deposits the surplus (and fills pits it runs into), evaporates and dies — dropping what it carries where it stops, so the mass balance holds exactly (what left the surface is deposited elsewhere or reported as lost off the map). Heights are normalised to the field's range internally, so Beyer's default `capacity`/`erode`/`deposit` behave the same on any terrain. Aim for **1–2 droplets per cell** (default 200 000): fewer only lowers slopes diffusely. Results are deterministic per `seed`. On the 512² world above 400 000 droplets took 1.13 s (release); 1024² with 1.5 M droplets 4.6 s and 48 MB. Pair with `jd.erosion.thermal@1` to soften the gully walls.

### Terraces and curvature

```
$ jd op graph.add_node --project Tc.jdproj --save --params '{"kind":"jd.filter.terrace@1","params":{"step_m":40,"softness":0.25}}'
$ jd op graph.add_node --project Tc.jdproj --save --params '{"kind":"jd.analysis.curvature@1","params":{"radius_px":6,"scale":0.0015}}'
```

![Left: terraced hillshade with 40 m treads; right: curvature map, orange convex and blue concave](images/cli-terrace-curvature.png)

`jd.filter.terrace@1` keeps each tread flat and spends the last `softness` fraction of every step on a smoothstep riser (softness 0 = hard cliffs, 1 = untouched). `jd.analysis.curvature@1` is the Laplacian of height in 1/m over a `radius_px` stencil, mapped to −1..1 by `scale` — ridges and convex breaks positive, valleys and hollows negative; on raw fBm it reacts to every bump (as above), so for rock-exposure or sediment masks blur first or use a larger radius. Both run in tens of milliseconds at 512² (39 ms and 65 ms, debug).

### Fluvial erosion (stream power)

```
$ jd op graph.add_node --project Sp.jdproj --save --params '{"kind":"jd.erosion.stream_power@1","params":{"k":0.002,"m":0.5,"iterations":20,"method":"mfd"}}'
$ jd op graph.add_node --project Sp.jdproj --save --params '{"kind":"jd.output.height@1","params":{"layer":"incision"}}'
# noise.out → stream_power.in, stream_power.out → height output, stream_power.incision → incision output
```

![Incision map (orange) over the input terrain, and the eroded output, from a 512² fBm world](images/cli-stream-power.png)

`jd.erosion.stream_power@1` is the river-carving law from geomorphology: each step lowers a cell by `K · A^m · S` where `A` is its drainage area (from `method`: d8, dinf or mfd) and `S` the slope to its steepest neighbour, solved implicitly from the outlets upstream so it is unconditionally stable — a cell never drops below its receiver, however large `K` or the step count. `uplift_m` raises the whole field per step (outlets included) for steady-state landscapes. On this 512² world (10 m cells, `k` 0.002, 20 steps, MFD) the incision map peaks at 10.4 m (mean 0.5 m) along the drainage network; 2048² × 20 steps takes 4.0 s and 321 MB in a release build (each step re-sorts the field for the drainage area). Pair it with `jd.erosion.thermal@1` for hillslope diffusion.

### Biomes from rules

```
$ jd op graph.add_node --project Bio.jdproj --save --params '{"kind":"jd.analysis.slope@1"}'
$ jd op graph.add_node --project Bio.jdproj --save --params '{"kind":"jd.biome.classify@1","params":{"rules":[
    {"name":"rock","slope":[0.6,2.0]},
    {"name":"snow","height_m":[900,1e9]},
    {"name":"beach","height_m":[-1e9,5]},
    {"name":"grassland","height_m":[5,300]},
    {"name":"forest","height_m":[300,900]}]}}'
$ jd op graph.add_node --project Bio.jdproj --save --params '{"kind":"jd.mask.class@1","params":{"id":1}}'   # rock mask
# noise.out → slope.in; noise.out → classify.height; slope.out → classify.slope; classify.biome → mask.classes
```

![Biome map of a 512² world: grey rock on steep slopes, white snow above 900 m, light and dark green for grassland and forest](images/cli-biomes.png)

`jd.biome.classify@1` tries its rules in order on every cell — each rule is a set of half-open ranges on `height_m`, `slope` (the 0..1 steepness from `jd.analysis.slope@1`) and `wetness` (`jd.hydrology.wetness@1`); the first match gives the cell that rule's 1-based id, 0 means no rule matched, and a rule that mentions an input you have not connected never matches. With no rules the temperate default above is used. `jd.mask.class@1 {id}` turns one class into a 0/1 mask for materials, scatter or export.

**Scatter instances from a mask.** `terrain.scatter` places points over the world from any 0..1 layer used as a density: one jittered candidate per `spacing_m` cell, kept with probability equal to the mask value, heights sampled from `height` (default the `height` layer), written as `.csv` (`u,v,z,r` — easting, northing from the south edge, height, a per-point 0..1 random for scale/rotation) or `.json`. Deterministic by `seed`.

```
$ jd op graph.add_node --project Bio.jdproj --save --params '{"kind":"jd.output.height@1","params":{"layer":"forest"}}'   # mask.class(5).out → it
$ jd --json op terrain.scatter --project Bio.jdproj --params '{"mask":"forest","height":"land","spacing_m":40,"seed":1,"path":"trees.csv"}'
{"op":"terrain.scatter","status":"ok","result":{"path":".../exports/trees.csv","count":10966,"bytes":431943,"spacing_m":40.0}}
```

![Biome map with yellow tree points covering exactly the forest class](images/cli-scatter.png)

On the 5 120 m world above the 40 m grid has 128² = 16 384 candidates and the forest mask covers 66.8 % of the area — 10 966 points, i.e. the expected share, in 30 ms including the file write. A spacing finer than the layer's sample spacing is refused (`spacing_m … must be ≥ the sample spacing (10 m)`). On the 512² world shown (heights −50..1200 m): rock 21.6 %, snow 4.4 %, grassland 7.2 %, forest 66.8 %; classify took 61 ms (debug). In the desktop the rules are a table in the inspector (add a row, type `min,max` ranges; see the desktop guide).

### Export for Unity or Unreal

```
$ jd --json op terrain.export_profile --project Bio.jdproj --params '{"profile":"unreal","dir":"unreal","layer":"land","masks":["forest"]}'
{"op":"terrain.export_profile","status":"ok","result":{"profile":"unreal","dir":".../exports/unreal",
  "files":["heightmap.png","heightmap.png.jd-height.json","weight_forest.png","README.md"],"size_px":[512,512],
  "settings":{"min_m":51.98,"max_m":1085.05,"unreal_z_scale":201.771,"unreal_location_z_cm":56851.2}},
  "warnings":[{"code":"W_ENGINE_SIZE","message":"Unreal's recommended overall resolutions are 127/253/505/1009/2017/4033/8129; this is 512×512 — …"}]}
$ jd op terrain.export_profile --project Bio.jdproj --params '{"profile":"unity","dir":"unity","layer":"land","masks":["forest"]}'
```

`terrain.export_profile` writes one folder an engine importer can take as-is: **Unreal** gets a 16-bit PNG heightmap with row 0 = north, one 8-bit grayscale weightmap per mask layer at the same resolution, and a README with the Landscape numbers computed from the data — Z scale = (max − min) × 100 / 512 (201.771 here for 52…1085 m) and the Z location that puts mid-grey at the mid height; **Unity** gets `heightmap.r16` (16-bit little-endian, row 0 = south, so *Flip Vertically: off*, Byte Order *Windows*), the weightmaps, and the Terrain Height (1033.07 m) plus the y offset (51.98 m). Sizes an engine does not like are reported as `W_ENGINE_SIZE` (Unity wants 2ⁿ+1 squares, Unreal 1009/2017/4033/8129 — use the world presets). The engine-side steps follow the Unity 6 manual and the Unreal 5 Landscape guide but are **UNVERIFIED** until someone imports once; the README says so.

### Bring in a real DEM or engine heightmap

```
$ jd op project.import_asset --project K.jdproj --params '{"path":"~/Downloads/srtm_tile.tif"}'
$ jd op graph.add_node --project K.jdproj --save \
    --params '{"kind":"jd.input.heightmap@1","params":{"path":"srtm_tile.tif"}}'
```

The input node reads from the project's `assets/` only (no absolute paths, no `..`), accepts PNG 16-bit, EXR, TIFF (f32/u16/u8) and headerless RAW (`size_px` required), maps quantised files through `range_m`, and resamples bilinearly onto the world grid. Wire it to filters, erosion or an output like any generator.

**GeoTIFF DEMs.** `project.import_asset` reports what a GeoTIFF says about itself — `geo.pixel_scale` (ground size of a pixel in the file's CRS units), `geo.tiepoint` (raster → model coordinates) and `geo.nodata` (`GDAL_NODATA`) — so you can size the world to match (64 px × 30 m = 1 920 m here):

```
$ jd --json op project.import_asset --project Geo.jdproj --params '{"path":"/data/dem.tif"}'
{"op":"project.import_asset","status":"ok","result":{"name":"dem.tif","bytes":12548,
  "geo":{"pixel_scale":[30.0,30.0],"tiepoint":[0,0,0,500000.0,4000000.0,0],"nodata":-9999.0}}}
```

Void cells (the file's nodata value, an explicit `nodata` parameter on the input node, or non-finite samples) are filled with the lowest valid height so erosion and hydrology never see −9999 m cliffs; a file that is entirely void is an error (`E_FORMAT_CORRUPT`). Verified on a 64×48 GeoTIFF with an 8×8 void block: `terrain.stats` of the imported layer reports min 80.08 m (the lowest real sample), max 319.99 m, 0 non-finite. Projections are not applied — the file's grid is resampled onto the world as-is; CRS-aware import comes with `jd-geo`. The same tags are **written** on export: a `.tif` from `jd export heightmap` / `jd terrain generate --out x.tif` carries `ModelPixelScaleTag` (the sample spacing), `ModelTiepointTag` (the north-west corner, from the project's `origin` + extent) and GeoKeys for the project's `crs` when it is `EPSG:<code>` (projected or geographic), and is always written north-up. Round trip checked: a project created with `origin [500000, 4000000]`, 128 px × 30 m and `crs EPSG:32637` exports a GeoTIFF whose tags read back as scale 30/30, tie point (500000, 4003840) and EPSG 32637 through `project.import_asset`; QGIS/GDAL place it accordingly. Pass `"georeference": false` in the export options for a plain TIFF.

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

Evaluating with an unconnected required input gives `E_GRAPH_MISSING_INPUT`. Before anything runs the core estimates peak memory (every node's outputs plus the heaviest kernel's scratch — hydraulic 40 B/cell, rivers 32, stream power 28, fill/wetness 24, flow 20, sculpt 12, thermal/droplet 8) and refuses with `E_RESOURCE_MEMORY` when it exceeds `settings.memory_budget_bytes` (default 4 GiB), naming the estimate, the budget and the way out (higher `lod`, smaller world, split the graph, raise the budget). `graph.move_node` (editor position) is journaled like every other change, so the desktop canvas layout survives a crash. `graph.append_param` pushes one element onto an array parameter (how the desktop records brush strokes): the journal entry holds just that element, so a long sculpt session grows the write-ahead log linearly instead of re-journaling the whole stroke list each time (20 strokes: 4.7 KB vs 22.8 KB). `graph.undo` / `graph.redo` step through the session's edit history (up to 100 steps) and are journaled as `graph.restore` with the full graph, so recovery replays them too; every `graph.*` response carries `undo_depth` / `redo_depth`. `graph.evaluate` with `lod > 0` publishes a reduced-resolution preview under `<layer>#lod<N>` (e.g. `height#lod1`) and never overwrites the full-resolution layer.

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

(512² world, debug build.) Deleting `cache/` never breaks a project; the next `graph.evaluate` recreates the layers. The cache also trims itself: every write evicts the oldest blobs (by last use) until the store fits `settings.cache_limit_bytes` in `project.json` (default 2 GB), and `jd op project.cache_gc --project X.jdproj` does the same on demand, reporting blobs/bytes kept and evicted. Change the limit (or the autosave interval / preview LOD) with `jd op project.set_settings --project X.jdproj --save --params '{"cache_limit_bytes":536870912}'` — fields left out keep their value, and values out of range are refused with `E_INPUT_INVALID`. Verified with the limit set to three 256² layers: generating five kept exactly three blobs and index entries, the newest layer still read back, the oldest reported `no heightfield layer named h1`.

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

Built-in kinds today: `jd.noise.fbm@1`, `jd.math.add@1`, `jd.math.multiply@1`, `jd.math.scale@1`, `jd.math.remap@1`, `jd.filter.blur@1`, `jd.filter.terrace@1` (quantise heights into `step_m` steps with a smoothstep riser of `softness` × step, `offset_m` shifts the grid — mesas, paddies, strata), `jd.analysis.curvature@1` (Laplacian over a `radius_px` stencil, mapped to −1..1 by `scale`: +1 convex ridges, −1 concave hollows — blur first or raise the radius for landscape-scale masks), `jd.erosion.thermal@1` (talus angle, iterations, rate — mass-conserving; 2048² × 50 steps ≈ 0.9 s release), `jd.analysis.slope@1` (0..1 steepness mask; multiply it into a layer to attenuate by slope), `jd.erosion.hydraulic@1` (virtual-pipe shallow water with conservative sediment transport; outputs `out`, `water`, `sediment`; 2048² × 100 steps ≈ 2.1 s release), `jd.erosion.droplet@1` (Beyer-style particle erosion: seeded droplets with inertia, capacity, brush erosion and deposition — gullies and fans; outputs `out` and signed `delta`; 1024² × 1.5 M droplets ≈ 4.6 s release), `jd.erosion.stream_power@1` (fluvial incision ∝ drainage area^m × slope, implicit/unconditionally stable, `k`/`m`/`uplift_m`/`iterations`/`method`; outputs `out` and `incision`; 2048² × 20 steps ≈ 4.0 s release), `jd.sculpt.strokes@1` (brush strokes as data: `points` in layer metres, `radius_m`, `mode` raise/smooth/flatten with `strength_m` or `amount`; cost is independent of brush size — a 600 m smooth stroke on 2048² takes 0.7 s in a debug build, down from 70 s), `jd.analysis.flow@1` (flow accumulation, `method` d8 / dinf / mfd, log-scaled 0..1 — rivers/wetness), `jd.hydrology.fill@1` (priority-flood depression filling; outputs the drainable field and `lakes` depth), `jd.hydrology.rivers@1` (carves channels where flow accumulates; outputs carved terrain and a `rivers` mask), `jd.hydrology.wetness@1` (topographic wetness index 0..1 for moisture/biome masks), `jd.hydrology.sea_level@1` (`ocean` mask below `level_m`, border-connected only so inland basins stay dry, soft `shore_m` edge; `depth` in metres), `jd.biome.classify@1` (ordered rules on height / slope / wetness → class-id map; default temperate set), `jd.mask.class@1` (0/1 mask of one class — feed materials and `terrain.scatter`), `jd.input.heightmap@1` (a PNG-16/EXR/TIFF/RAW file from the project's `assets/`, resampled onto the world; `project.import_asset` copies files in), `jd.output.height@1`.

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

```
$ jd --json diag bench s1 | tail -1
{"op":"diag.bench","status":"ok","result":{"scene":"s1","size_px":1024,
  "stages":[{"name":"noise.fbm","ms":30.0},{"name":"noise.ridged","ms":25.3},{"name":"math.add","ms":1.5},{"name":"erosion.thermal×50","ms":250.9}],
  "total_ms":307.6,"peak_rss_bytes":45219840,"profile":"release","content_hash":"859db903b963db8a…"}}
```

`jd diag bench` runs a benchmark scene from `docs/spec/performance-targets.md` with the real kernels (S1: 1024² fBm + ridged noise, add, 50 thermal steps) and reports wall time per stage, the process's peak resident memory and the content hash of the result — the same hash on every machine means the kernels are deterministic. `--size 2048` scales the scene; `profile` says `debug` when the binary is not optimised (those numbers are not comparable). CI runs S1 on every push as a smoke with a 15 s / 1 GB budget.

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
