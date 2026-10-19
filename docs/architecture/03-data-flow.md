# 03 — Data Model and Data Flow

## Core data types (in `jd-types` / `jd-terrain` / `jd-scene`)

```mermaid
classDiagram
  class Heightfield {
    +width: u32
    +height: u32
    +spacing_m: f32
    +origin: DVec2
    +vertical_range: (f32,f32)
    +tiles: TileGrid~f32~
    +crs: Option~CrsInfo~
  }
  class Tile~T~ {
    +coord: TileCoord(x,y,lod)
    +size: u32  (default 512)
    +halo: u32  (default 4)
    +data: Box[T]  row-major, len=(size+2*halo)^2
    +hash: Blake3
  }
  class Mask {
    +tiles: TileGrid~f32~  0..1
    +semantic: MaskKind (slope|flow|wear|deposit|snow|paint|biome|custom)
  }
  class Flow {
    +direction: TileGrid~u8~  D8
    +accumulation: TileGrid~f32~
  }
  class Mesh {
    +positions: Vec~Vec3~
    +normals: Vec~Vec3~
    +tangents: Vec~Vec4~  MikkTSpace
    +uv0: Vec~Vec2~
    +colors: Option~Vec~Rgba~~
    +indices: Vec~u32~
    +submeshes: Vec~Submesh~
    +lods: Vec~MeshLod~
    +stats: MeshStats
  }
  class Material {
    +id: MaterialId
    +graph: MaterialGraph
    +resolved: PbrParams
    +textures: Vec~TextureRef~
  }
  class Scene {
    +schema_version: u32
    +units: metersPerUnit
    +up_axis: Y|Z
    +handedness: Right|Left
    +nodes: Vec~SceneNode~
    +meshes, materials, textures, instancers, lights, environment
    +metadata: Map
  }
  Heightfield --> Tile
  Mask --> Tile
  Flow --> Tile
  Scene --> Mesh
  Scene --> Material
```

Decisions:
- **f32 heightfields** (not u16, not f64). u16 is an export quantisation; f64 is only used for world-space origins (`DVec2`) to support large worlds with origin rebasing.
- **Tiles with halos** so filters (blur, erosion stencils, slope) run per tile without global synchronisation; halo width is the max stencil radius of the node (default 4, nodes declare their radius).
- **Content hash per tile** enables region-based recomputation and cache reuse across sessions.
- **Row-major, +X east, +Y north in heightfield space**; conversion to engine axes happens only in `jd-io` (see `08-io-formats.md`).

## End-to-end flow: generate → simulate → texture → mesh → export

```mermaid
flowchart LR
  seed["Seed + params"] --> N["Noise/shape nodes"]
  N --> HF0["Heightfield v0 (tiles)"]
  HF0 --> ER["Erosion nodes<br/>(hydraulic/thermal/wind)"]
  ER --> HF1["Heightfield v1"]
  ER --> MK["Masks: flow, wear, deposit, sediment"]
  HF1 --> AN["Analysis: slope, curvature, aspect, AO"]
  HF1 --> HY["Hydrology: fill, flow, rivers, lakes"]
  AN & MK & HY --> BI["Biome / material masks"]
  BI --> MT["Material graph → PBR layers"]
  MT --> BK["Bake: albedo, normal, roughness, AO, height, splat/weight maps"]
  HF1 --> MS["Meshing: grid/RTIN/TIN + stitching"]
  MS --> TV["Topology validation"]
  TV --> DC["Decimation + LOD chain + MikkTSpace"]
  HF1 & BI --> SCT["Scatter: instances"]
  DC & BK & SCT --> CSR["Canonical Scene (CSR)"]
  CSR --> EX["Exporters: GLB / USD(Z) / OBJ / FBX-adapter / heightmaps / weightmaps"]
  EX --> VAL["Validators: glTF Validator, usdchecker, Blender headless import"]
  VAL --> REP["Export report: losses, stats, validation results"]
```

## Sequence: interactive parameter change in the GUI

```mermaid
sequenceDiagram
  participant UI as Frontend (WebView)
  participant T as jd-app (Tauri cmd)
  participant C as jd-core
  participant G as jd-graph
  participant J as jd-jobs
  participant R as Viewport (WebView)
  UI->>T: graph.set_param {node, key, value}
  T->>C: op("graph.set_param", req)
  C->>G: mutate + mark dirty (node → downstream tiles)
  G-->>C: DirtySet {nodes, tiles}
  C->>J: submit EvalJob(dirty set, priority=interactive, preview_lod=2)
  J-->>T: JobAccepted {job_id}
  T-->>UI: {job_id}
  loop per tile
    J->>G: evaluate tile (cache hit? skip)
    G-->>J: tile result + hash
    J->>R: upload tile to GPU heightmap texture (region)
    J-->>UI: progress event (channel) {job_id, pct, tiles_done}
  end
  J-->>UI: JobCompleted {stats}
  UI->>R: request frame (viewport already updated)
```

Preview evaluation runs at a reduced LOD (e.g. 1/4 resolution) for interactivity; full-resolution evaluation is a separate job triggered on idle or on explicit "build", as in the reference tools' preview/build split.

## Sequence: headless CLI build

```mermaid
sequenceDiagram
  participant U as User / script
  participant CLI as jd CLI
  participant C as jd-core
  participant J as jd-jobs
  participant IO as jd-io
  U->>CLI: jd build project.jdproj --output out/ --format glb,usdz,r16 --json
  CLI->>C: op("project.open") → op("graph.evaluate", {full=true})
  C->>J: submit (CLI streams progress to stderr for TTY or NDJSON with --json)
  J-->>C: results
  C->>IO: export per format, validators optional (--validate)
  IO-->>CLI: ExportReport (losses, validation)
  CLI-->>U: exit 0 + JSON report on stdout (or non-zero with error envelope)
```

## Memory budget model (design target, to be measured)

| Item | Size at 4096² | Notes |
|---|---|---|
| One f32 heightfield | 64 MiB | 4096² × 4 B |
| One f32 mask | 64 MiB | per mask; typical pipeline keeps 4–8 live → 256–512 MiB |
| Erosion working set (virtual pipes: height, water, sediment, 4 flux, velocity ×2) | ~9 × 64 MiB ≈ 576 MiB | tiled execution bounds live set to tiles-in-flight × 9 × tile size |
| Mesh (full-res grid, 16.7 M verts) | ~0.6–1 GiB | never materialised whole: meshed per tile; LODs reduce |
| Baked textures (4k, 5 maps, RGBA8) | 320 MiB | written streaming to disk |
| GPU heightmap/LOD textures | 64–200 MiB | R32F + mips |

Conclusion: at 4k a naive "everything resident" pipeline already reaches ~2 GB; at 8k it would be ~8 GB. Hence tiling + cache + streaming are mandatory from Phase 2, and CI must assert peak RSS on the benchmark scenes (`docs/spec/performance-targets.md`).
