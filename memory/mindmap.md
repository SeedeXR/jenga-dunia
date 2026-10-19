# Mindmap — Jenga Dunia System Map

Continuously evolving high-level map. Update when structure changes. Detailed diagrams live in `docs/architecture/`.

```mermaid
mindmap
  root((Jenga Dunia))
    Adapters
      jd-app Tauri shell
        WebView UI TypeScript Tailwind Lucide
        IPC commands channels events
        Native viewport surface ADR-0003
      jd-cli clap
      jd-mcp rmcp
    jd-core API
      Operation registry + JSON schemas
      Project service .jdproj migrations autosave journal
      Undo redo commands
    Domain
      jd-graph
        Typed ports DAG validation
        Hash cache dirty sets
        Preview vs build LOD
      jd-terrain
        f32 tiles with halos
        Noise fBm ridged voronoi spectral
        Filters analysis slope curvature flow AO
        Stroke sculpt nodes
      jd-sim
        Hydraulic particle grid stream-power
        Thermal wind coastal snow glacier
        Hydrology priority-flood rivers lakes
        Weather state water ocean FFT
      jd-materials
        Material graph PBR
        Translation glTF USD MaterialX
        Baking
      jd-scatter
        Biomes Poisson instances
      jd-geometry
        Meshing grid RTIN TIN dual contouring
        Topology validation
        QEM decimation LOD MikkTSpace
      jd-scene CSR
      jd-io
        glTF USD OBJ STL PLY EXR PNG TIFF RAW GeoTIFF
        FBX import ufbx export adapter
        Validators loss reports
      jd-geo DEM OSM contours
      jd-vision ONNX MoGe-2 DA3 lighting colour
      jd-gpu wgpu device WGSL kernels offscreen renders CPU parity
      Viewport three.js WebGPU WebGL2 fallback in WebView
    Infrastructure
      jd-jobs scheduler cancellation budgets
      jd-cache blake3 mmap sqlite LRU
      jd-diag tracing error codes metrics bundles
      jd-plugins manifests scopes out-of-process
      jd-types ids units transforms versions
    Bridges
      Blender add-on separate licence
      Unity Unreal profiles
      Bridge manifest JSON
    Docs
      architecture 00-13
      adr
      research + evidence + licensing
      spec project-format error-codes performance interop
      memory agent files
    Quality
      Tests unit property integration snapshot
      Benches scenes S1-S8 peak RSS
      CI macOS Linux Windows
      Validators glTF usdchecker Blender headless
```

## Dependency direction (summary)

adapters → jd-core → domain crates → jd-types; infrastructure (jobs, cache, diag, plugins) used by jd-core and domain; only jd-gpu touches wgpu; only jd-io/jd-geo/jd-vision/jd-cache do I/O.

## Workflow map

generate → simulate → analyse → texture → scatter → compose → render/preview → optimise → export → validate → bridge.
