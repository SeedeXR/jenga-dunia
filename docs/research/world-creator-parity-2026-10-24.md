> Audit run 2026-10-24 by a research agent against the repo at commit c38c8ed and the live World
> Creator docs. **Since then:** Debris and lava (gap 10) shipped as `jd.erosion.debris@1` and
> `jd.volcanic.lava@1` (9bab3e2, `simulation-debris-lava.md`); a landform-class mask
> (`jd.analysis.geomorphon@1`) and `terrain.measure` shipped with the reference-image work.
> **2026-10-25:** gap 3 partly closed (`jd.math.min/max/lerp@1` blend nodes with opacity,
> `jd.analysis.aspect@1` sun-direction mask, `jd.analysis.occlusion@1`), gap 4 closed for stamps
> and primitives (`jd.shape.stamp@1`, `jd.shape.primitive@1` with crater, plateau, ramp, cone,
> dome), gap 5 partly (domain warp on `jd.noise.fbm@1`, `jd.filter.edge_fade@1` = zero edge /
> border blend), gap 12 partly (per-node presets, a layer stack in the desktop), gap 13's cheap
> part (an emissive channel in materials). Still open: image-based materials (gap 1), own models
> in scatter (gap 2), pattern masks and curve/smoothstep effects (3), polygon layers (4), crater /
> inflate / denoise filters (5), tiled export and the Godot / Houdini bridges (6), map export
> formats (7), holes (8), river editing (9), multiple water bodies (11).
> **2026-10-25, afternoon:** closed — own models in scatter (2: `terrain.scatter {model}`), pattern
> masks and curve / smoothstep (3: `jd.pattern@1`, `jd.math.curve@1`), polygon layers (4:
> `jd.shape.polygon@1`), inflate / deflate and denoise (5: `jd.filter.inflate@1`,
> `jd.filter.denoise@1`), tiled export (6: `terrain.export_profile {tiles}`), map formats (7: RGBA
> splat packs, `ao.png` + AO in the ORM map's R, ASC / XYZ heightmaps), holes (8: `holes` on the mesh
> and profile exports, `holes.png`, Unity SetHoles), river springs (9: `springs` on
> `jd.hydrology.rivers@1`), lakes drawn as water (11: a published `lakes` layer in the viewport).
> Image-based materials (1) closed the same afternoon: `texture {albedo, normal, tile_m, px}` per
> layer on the detail-tile path (PNG only; triplanar / stochastic blending stay the engine's job).
> The Godot / Houdini / C4D bridges (6) are out of scope: the owner's rule (2026-10-25) is Unreal,
> Unity and Blender only. Spline I/O (9) closed the same day (`terrain.export_splines` → GeoJSON,
> Blender `jd.import_splines`). Workflow conveniences (12) partly: per-node presets, hover
> thumbnails, arithmetic in number fields, a layer tree with search, a layer-type icon strip.
> Rendering (13) partly: HDRI import, ACES exposure / bloom / vignette, stills, flythrough frames;
> no path tracer (none for WebGPU in three 0.186). The World Creator video's own tools (brush and
> grid transforms, brush alphas, sculpt-layer settings, terrain actions, framed block, compass /
> ruler, octave bars) also landed on 2026-10-25. Still open: localisation and a crash reporter
> (12), a path tracer and camera-path editing beyond linear keys (13).

# Jenga Dunia vs World Creator: feature parity audit (2026-10-24)

**Short answer: no.** Jenga Dunia has not covered every World Creator feature. It matches or beats World Creator on simulation, hydrology, automation, mesh export and determinism. It still falls short on four things that sit in the middle of World Creator's workflow:

1. **Image-based materials.** World Creator uses texture sets, Substance and Megascans, with triplanar, stochastic and height blending. We have flat colour, roughness and a procedural grain tile.
2. **Scattering your own 3D models.**
3. **Placed shape layers.** Stamps, primitives, polygons and landscape presets that you position in an area.
4. **A wide library of filters and masks.**

Rendering and presentation is the other big gap: path tracer, HDRI lighting, post effects, cameras and screenshots.

**How to read the evidence columns**
- **WCR §x** is a section of `docs/research/world-creator.md`.
- **NK** is a node kind in `schemas/v1/graph/node-kinds.json`. **OPS** is an op in `schemas/v1/ops/index.json`.
- **TODO Lnn** is a line in `memory/todo.md`. **CLI "…"** and **DESK "…"** are headings in `docs/user/cli-quickstart.md` and `docs/user/desktop-app.md`.
- **Have** means equivalent. **Partial** says exactly which part exists. **Exceeds** means we go measurably further.

**What I checked live on 2026-10-24**
- **`https://docs.world-creator.com/llms.txt`:** the newest release notes are still 2026.4; there is no 2026.5 or WC4 page.
  - Two Effects pages are not in our research doc's §3.4 list: **Contrast** and **Smear**.
  - There is a **Scene → Object Collection** page. Quote: *"The Object Collection allows you to place objects either individually or using a scatter brush."* and *"Object Collections operate on the whole terrain (Object Layers work on the biome level)"*.
- **`reference/export/conventional-export.md`:** *"In World Creator, you can export your terrain as a mesh."* No mesh formats are named, so OBJ/FBX/glTF stays REPORTED. `.dae` is named only for splines.
- **`reference/scene.md`:** Water has just Surface Height, Sea Level, Clarity, Color, Coast Foam and Coast Foam Range.
- **WC4:** still press-only (CG Channel 2026-10-12, Digital Production 2026-10-13). Nothing from WC4 is counted as a World Creator feature below.

---

## 1. Parity table

### Terrain shapes and generators
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Noise: Perlin, Simplex, Value, Voronoi | WCR §3.1, `reference/terrain/noises.md` | Have | NK `jd.noise.fbm@1` kind = perlin, open_simplex2(_s), value(_cubic), cellular | Our cellular is F1 distance only. WC's Voronoi also has cells and distance-to-edge modes. |
| Noise: Billow, Gabor, Phasor, Wave, White | WCR §3.1 | Partial | `ping_pong` fractal ("terraced/billowy", `crates/jd-terrain/src/noise.rs`) | Only a billow-like fractal exists. Gabor, Phasor, Wave and White are missing. |
| Fractal fBm/ridged, octaves, lacunarity, gain | WCR §3.1 | Have | NK `jd.noise.fbm@1` fractal, octaves, lacunarity, gain | |
| Domain warp | WCR §3.1 | Missing | `noise.rs` doc comment says "domain warp", but `NoiseParams` (L57–85) has no warp field | The doc comment is stale. |
| Terrain seed, same seed gives same shape | WCR §2.4 | Exceeds | DESK "Tutorial: first terrain" (content hash), TODO L37 (S1 hash identical on 3 OSes) | WC does not promise identical results across GPUs (WCR §2.4). |
| Landscape shape layer (Alpine, Canyon, Crater, Dunes, Island, Volcano…) | WCR §3.2 | Partial | DESK "Desert landscapes" (noise Presets: Mountains, Ridged alpine, Rolling hills, Desert plain); NK `jd.hydrology.network@1` | We have 4 noise presets. Crater, volcano, island, crack and furrow shapes are missing. |
| Sculpt layer (brushes) | WCR §3.2 | Have | NK `jd.sculpt.strokes@1` (raise/smooth/flatten), DESK "Tutorial: sculpting" | Our strokes are replayable data. WC's sculpt grid is capped at 1024². |
| 2D stamp (heightmap placed in an area: transform, blend op) | WCR §3.2 | Partial | NK `jd.input.heightmap@1` | It resamples onto the whole world. No position, scale, rotation or blend area. |
| 3D stamp from a model | WCR §3.2 | Partial | NK `jd.input.heightmap@1` (binary FBX rasterised over its footprint) | FBX only, whole world, not placeable. |
| Primitive layer (circles, gradients, polygons) | WCR §3.2 | Missing | none | |
| Polygon layer (inside/outside height and slope) | WCR §3.2 | Missing | none | |
| Shape op Overwrite/Add/Min/Max | WCR §2.2 | Partial | NK `jd.math.add@1`, `jd.math.multiply@1`, `jd.math.scale@1` | No min, max or lerp nodes. |
| Hydrology-first terrain generation | not in WC | Exceeds | NK `jd.hydrology.network@1`, CLI "Hydrology first" | |

### Erosion and simulation
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Hydraulic erosion (droplet) | WCR §3.3, `…/basic-erosion/hydraulic.md` | Have | NK `jd.erosion.droplet@1` | |
| Hydraulic erosion (grid/sediment) | WCR §3.3 | Exceeds | NK `jd.erosion.hydraulic@1` (pipes, GPU, rain/drains/soil/hardness inputs) | |
| Thermal (inside WC's Hydraulic block) | WCR §3.3 | Have | NK `jd.erosion.thermal@1` (GPU, tiled) | |
| Strata layers with varying erodibility (Hydraulic Sediment) | WCR §3.3 | Have | NK `jd.terrain.strata@1` → hardness inputs on thermal, stream power and hydraulic; CLI "Strata" | |
| Ridged/Soft/Thin/Wide/Sediment Flows filters | WCR §3.3 | Partial | NK `jd.erosion.stream_power@1`, droplet | Different models. These named looks have no presets. |
| Sediment: Talus / Fill Soft / Mud | WCR §3.3 | Partial | Talus = thermal; Fill ≈ `jd.hydrology.fill@1` | No Mud. |
| Rain maps and erosion/soil maps | WCR §3.3 (2023.4) | Have | `jd.climate.rain@1` rain; hydraulic water/sediment/soil outputs; droplet `delta`; stream_power `incision` | |
| Fluid sim (viscosity water→lava, fills depressions) | WCR §3.8, `simulation-layers/fluid.md` | Partial | `jd.hydrology.fill@1` lakes; hydraulic water output | Lakes and water depth exist. No viscosity/lava flow sim. |
| Snow sim (fall, melt, diffusion, avalanche) | WCR §3.8 | Have | NK `jd.climate.snowpack@1` (degree-day melt, drift, avalanche, albedo), `jd.climate.snow@1` | Ours is also keyable over the timeline. |
| Sand sim (granular settle) | WCR §3.8 | Have | NK `jd.erosion.aeolian@1` (repose angle, wind transport) | |
| Debris sim (2D splats / 3D meshes scattered by a sim) | WCR §3.8 | Partial | OPS `terrain.rocks` + `terrain.scatter {rock}` | Rocks are scattered from a mask. Nothing is physically simulated. |
| Paintable simulation layers / sim masks | WCR §1.1 (2023.4) | Partial | Any mask can be painted with strokes `clamp` (DESK "Paint density") | |
| Coastal, karst, yardang, stream power, orographic rain | not in WC | Exceeds | NK `jd.erosion.coastal@1`, `jd.erosion.karst@1`, `jd.erosion.yardang@1`, `jd.erosion.stream_power@1`, `jd.climate.rain@1` | WC's Shore filter is a heightfield effect (WCR §3.3). |

### Filters
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Add/Set | WCR §3.3 | Have | `jd.math.add@1`, `jd.math.scale@1` | |
| Border Blend, Zero Edge | WCR §3.3 | Missing | none | |
| Flatten | WCR §3.3 | Partial | sculpt `flatten`; `jd.terrain.spline@1 mode=flatten` | No area/height flatten filter. |
| Curve, Cutoff | WCR §3.3 | Partial | `jd.math.remap@1` (linear + clamp) | No curve node. |
| Heightmap filter | WCR §3.3 | Have | `jd.input.heightmap@1` | |
| Voronoi filter | WCR §3.3 | Partial | cellular noise | Generator only, not a filter on existing terrain. |
| Smooth / blur | WCR §3.3 | Have | `jd.filter.blur@1` | Box blur only. |
| Terrace Simple / Irregular / Steep | WCR §3.3 | Partial | `jd.filter.terrace@1` (step, softness, offset) | No irregular step spacing. |
| Strata effect | WCR §3.3 | Have | `jd.terrain.strata@1` + terrace (CLI "Strata") | |
| Dunes / Wind drift | WCR §3.3 | Exceeds | aeolian (bimodal winds, vegetation), yardang | Simulated, not a stylised filter. |
| Shore | WCR §3.3 | Exceeds | `jd.erosion.coastal@1` | |
| Crater, Inflate/Deflate/Balloon, Denoise, Spike Removal, Kuwahara, Hexagons, Blocks, Swirl, Distortion, Squeeze, Scatter, Washed Off, Rugged, Smooth Ridges, Angle Blur, Ridged-as-filter | WCR §3.3, llms.txt | Missing | none | |
| Arid: Canyon, Chipped, Cliffs, Rocky × 6 | WCR §3.3 | Partial | Mesas, buttes and stepped canyon walls via strata + thermal + stream power (CLI "Strata") | No one-click arid presets. |
| Angle Break | WCR §3.3 | Missing | none | |

### Distributions (masks) and effects
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Height, slope, angle, steepness | WCR §3.4 | Have | `jd.analysis.slope@1`; height ranges in `jd.biome.classify@1`; remap | |
| Curvature | WCR §3.4 | Have | `jd.analysis.curvature@1` | |
| Cavity | WCR §3.4 | Partial | curvature negative side (`radius_px`) | |
| Flow | WCR §3.4 | Exceeds | `jd.analysis.flow@1` (d8/dinf/mfd), `jd.hydrology.wetness@1` (TWI) | |
| Sea Level | WCR §3.4 | Exceeds | `jd.hydrology.sea_level@1` (ocean, depth, exposure, current) | |
| Occlusion, Sun Direction, Angle 3D, Height Gradient, Roughness, Melt, Rocks, Rocky, Rocky Edges | WCR §3.4 | Missing | none | Sun aspect exists only inside the snow nodes, not as a mask. |
| Noise distributions | WCR §3.4 | Have | `jd.noise.fbm@1` with a 0..1 `height_range_m` | |
| Pattern: Dots, Grid, Maze, Spiral, Splatter | WCR §3.4 (2026.2) | Missing | none | |
| Layer: Mask, Path, Rivers, Simulation, Filter Change | WCR §3.4 | Have | spline `mask`, rivers `rivers`, hydraulic water/sediment, delta = `add(b, scale(a,−1))` | Any published layer works as a mask (TODO L61). |
| Layer: Area, Polygon, Model, Object Area | WCR §3.4 | Missing | none | |
| Effects: Blur, Clamp, Invert, Remap | WCR §3.4 | Have | blur; `jd.math.remap@1` (clamp; invert = swapped range) | |
| Effects: Curve, Smoothstep, Step, Distortion, Directional Blur, Denoise, Edge, Splatter, Noise Offset, Repeat Value, flows, Voronoi, **Contrast**, **Smear** | WCR §3.4; llms.txt (Contrast, Smear) | Missing | none | |
| Mask combine Multiply/Add/Subtract/Min/Max + Opacity | WCR §2.2 | Partial | add, multiply, scale | No min/max node. |
| Heatmap view of any distribution | WCR §3.4 | Have | DESK "Hydrology layers in the viewport" (colour by layer) | |

### Biomes, materials and texturing
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Biomes placed by distributions | WCR §3.5 | Have | `jd.biome.classify@1` + `jd.mask.class@1`; rules table (DESK) | |
| Per-biome filters/objects | WCR §3.5 | Partial | You can wire it in the graph (mask × filtered branch) | No per-biome container or lerp node. |
| Biome Layers (paint biome + sculpt at once) | WCR §3.5 | Partial | painted mask → classify `mask` band | |
| Biome map export | WCR §5.1 | Have | class map is a layer → `terrain.export_heightmap` | |
| Material per mask: tint, roughness | WCR §3.6 | Have | `terrain.textures {materials}`, DESK "Materials" | |
| Metalness, emission | WCR §3.6 | Missing | none | |
| Texture sets, ZIP packages, Substance .sbsar, Megascans/Fab | WCR §3.6 | Missing | Only procedural detail tiles (TODO L61) | |
| Gradient materials (sat-map by distribution) | WCR §3.6 | Partial | fixed hypsometric base ramp (DESK "Workflow") | Not user-editable. |
| Triplanar, stochastic, height blending, displacement | WCR §3.6 | Missing | TODO L61 "texture library / triplanar / stochastic ☐" | |
| Tile size / detail texture | WCR §3.6 | Have | `detail {tile_m, bump…}` (TODO L61) | |
| Colour erosion (fluvial advection) | WCR §3.3 | Missing | none | |
| Decals (2026.4) | WCR §1.1 | Missing | none | |
| Shape-layer colour into materials | WCR §3.2 | Missing | Imported colour maps are not used | |

### Distribution and scatter
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Density from distributions, seed, instance distance | WCR §3.7 | Have | OPS `terrain.scatter` (mask, spacing_m, jitter, seed) | |
| Overwrite radius (layers exclude each other) | WCR §3.7 | Have | `terrain.scatter {avoid}` | |
| Custom models (.glb/.gltf/.fbx/.obj) | WCR §3.7 | Partial | GLB holds a cone proxy or our generated rock (`rock`) | You cannot import your own model. Engine CSV/JSON carries points only. |
| Align to normal, offsets, per-axis scale/rotation ranges | WCR §3.7 | Partial | `ScatterPoint {u,v,z,r,age}` (`crates/jd-terrain/src/scatter.rs`): yaw + 0.75–1.25 scale from `r` | No normal and no ranges. |
| Sub-objects | WCR §3.7 | Missing | none | |
| Instance export (CSV + JSON) | WCR §5.1 | Exceeds | CSV/JSON/GLB `EXT_mesh_gpu_instancing` + Unreal DataTable / Unity JSON (TODO L55) | |
| Detail objects as masks for engine scatter | WCR §3.7 | Have | `terrain.export_profile {masks}` weightmaps | |
| Object Collection (place by hand or scatter brush) | live `reference/scene/object-collection.md` | Partial | Paint density brush (DESK "Paint density") | No single placement or gizmos. |
| Terrain adapts to objects; camera-bound high density; VRAM-scaled count | WCR §3.7 | Missing | Inline cap of 200 000 points (DESK) | |
| Growth over time | not in WC | Exceeds | `terrain.scatter {time, follow_time}` | |

### Water, rivers, lakes and ocean
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Procedural river network carving | WCR §3.8 | Have | `jd.hydrology.rivers@1`, `jd.hydrology.network@1` (valley shapes) | |
| Hand-placed springs, per-node width/depth, floodplain | WCR §3.8 | Partial | `valley_width_m`, `valley_shape` on network | Rivers cannot be edited by hand. |
| Lakes at depressions | WCR §3.8 | Have | `jd.hydrology.fill@1` `lakes` | |
| River/path spline export (.dae) | WCR §3.8, live conventional-export | Missing | none | |
| Water render: sea level, clarity, colour, coast foam | live `reference/scene.md` | Exceeds | sea_level `visibility_m`, `tint`; foam; OPS `ocean.waves`, `ocean.bake` | |
| Multiple water bodies (WC itself: one) | WCR §3.8 | Partial | lake depth exists as a layer | The viewport draws only the sea plane (no lake rendering in `apps/desktop/src`). |
| Seabed, bathymetry, refraction, currents | not in WC | Exceeds | `jd.biome.seabed@1`, `jd.ocean.bathymetry@1` | |

### Weather, sky and lighting
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Sun angle / time of day | live `scene.md` | Have | DESK "Sky and sun" | |
| Sky, atmosphere | WCR §7 | Have | Preetham sky (OPS `sky.preview`, `sky.bake`), turbidity | |
| Volumetric clouds | WCR §7 | Partial | `cloud_cover`, `cloud_scale_deg` (2D noise layer, not volumetric) | |
| Fog / volumetric dust | WCR §7 | Partial | Fog slider (viewport only) | |
| HDRI environment lighting (load an HDRI) | WCR §7 | Missing | We bake HDRIs; we do not load them | |
| Max light bounces, shadow softness | live `scene.md` | Missing | none | |
| Shared wind / weather state | not in WC | Exceeds | `WorldSettings.wind` (TODO L59) | |

### Sculpting, brushes and stamps
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Raise/lower/smooth/flatten brushes | WCR §10 | Have | `jd.sculpt.strokes@1` | |
| Custom brush shapes, grid tools, pen/tablet rework | WCR §10, §1.1 | Missing | none | Falloff is smooth only. |
| Mask painting (paint/erase/smooth, import bitmap) | WCR §3.5 | Partial | Strokes with `clamp` on any mask | A bitmap can be imported as a heightmap and used as a mask. No erase/fill/flip tools. |
| Hole layers (caves) | WCR §3.5 | Missing | none | |

### Splines and roads
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Path layer: width, falloff, flatten | WCR §3.8 | Have | `jd.terrain.spline@1` (flatten/cut/raise, corridor mask), DESK "Roads" | |
| Width noise, closed paths, procedural width modes | WCR §3.8 | Missing | none | |
| Spline import/export | WCR §3.8 | Missing | Points are node params only | |
| Materials along the path | WCR §3.8 | Have | spline `mask` → materials and scatter | |

### Real-world data and import
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| MapTiler DEM streaming | WCR §3.9 | Have | OPS `geo.fetch_dem` (Terrarium / Copernicus / OpenTopography), DESK "Real world" | Ours needs no paid subscription. |
| Satellite colour imagery | WCR §3.2 | Missing | none | |
| GeoTIFF, HGT, DTED, ASC, VRT, XYZ import | WCR §3.9 | Partial | GeoTIFF with CRS reprojection (TODO L46) | HGT, DTED, ASC, VRT and XYZ are missing. |
| RAW/R16/EXR/PNG stamps | WCR §3.9 | Have | `jd.input.heightmap@1` | |
| OSM features, photo → terrain | not in WC | Exceeds | OPS `geo.fetch_features`, `vision.depth` | |

### Export formats and engine bridges
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Heightmap RAW/TIFF/PNG/EXR | WCR §5.1 | Partial | `terrain.export_heightmap` exr, png16, raw_r16, raw_r32, tiff32 | No 8/16-bit TIFF, PNG8, half EXR, or JPEG/TGA/BMP. |
| XYZ / ASC | WCR §5.1 | Missing | none | |
| GeoTIFF with CRS on export | not in WC | Exceeds | TODO L64 | |
| Mesh export | live conventional-export (formats unnamed) | Exceeds | glb, fbx, obj, stl, ply, usda, usdc, usdz + QEM + LODs (TODO L64–65) | |
| Splat map (RGBA packed) | WCR §5.1 | Partial | one 8-bit PNG per layer (`terrain.export_profile`) | |
| Colour, normal, roughness maps | WCR §5.1 | Have | `terrain.textures` | |
| Metalness, AO, topo, relief maps; Unity Mask Map | WCR §5.1 | Missing | `profiles.rs` L257 loss note on mask map | |
| Heat / simulation maps | WCR §5.1 | Have | Any layer can be exported | |
| HDRI render export | WCR §1.1 (2025.2) | Partial | `sky.bake` | Sky only, not a scene render. |
| Incremental export naming, export presets, tile count readout | WCR §5.1 | Missing | none | |
| Tiled export (`heightmap_x_y`) | WCR §4, §5.2 | Missing | No tile split in `export.rs` / `profiles.rs` | |
| Unity bridge | WCR §5.2 | Have | `terrain.export_profile unity` + `integrations/unity/Editor/JengaDuniaImport.cs` (TerrainData, layers, trees) | Single terrain; no split threshold. |
| Unreal bridge (Landscape, layers, PCG, World Partition) | WCR §5.2 | Partial | `export_profile unreal` (G16 height, G8 weights, Z scale) | You create the Landscape actor by hand (`integrations/unreal/README.md`). No PCG, no World Partition. |
| Blender bridge | WCR §5.2 | Partial | `integrations/blender/jenga_dunia_bridge` (imports the mesh via `jd export mesh`) | No materials-as-nodes or Geometry Nodes instancing. |
| Houdini, Cinema 4D, Godot/Terrain3D bridges | WCR §5.2 | Missing | none | |
| ORBX / Octane | WCR §5.1 | Missing | none | |
| FBX SDK verification, USD, Maya check | not in WC | Exceeds | `fbx.sdk_check`, TODO L64, L69–70 | |

### Rendering and viewport
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Path tracer + denoiser | WCR §7 | Missing | three.js raster (DESK) | WCR §13 says not to chase this. |
| Render modes (shape / material / cinematic) | WCR §7 | Partial | Textured (PBR) on/off toggle | |
| Overlays: heatmap, topo, grid | WCR §7 | Partial | heatmap = colour by layer | No topo (contours ☐, TODO L75). No grid found. |
| Post: tonemapping, LUTs, DOF, bloom, vignette | WCR §7 | Missing | none | |
| Cameras, flythrough sequences, screenshots, reference mannequin | WCR §7, §10 | Missing | `timeline.export` writes heightmap frames, not rendered images | |
| Preview size (4k, 8k High VRAM) | WCR §4 | Partial | Preview detail 1024² / 2048² (TODO L50) | |
| Horizon / infinite extension | not in WC | Exceeds | `terrain.horizon` | |

### Workflow, UI and project
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| Non-destructive layer stack | WCR §2.2 | Exceeds | Typed DAG + node editor (TODO L45) | WC4 adds nodes (REPORTED). |
| Realtime generator, debounce, staged recompute | WCR §2.4 | Have | Live preview ⚡, live target ms, bypass, "only <output>" (DESK) | |
| Per-filter presets with thumbnails, hover preview, templates, sample browser | WCR §10 | Missing | Noise presets only | |
| Expressions in numeric fields (2026.4) | WCR §2.4 | Missing | none | |
| Save as package (portable) | WCR §11 | Have | `.jdproj` + `assets/` (`project.import_asset`) | |
| Incremental save / backups | WCR §11 | Exceeds | journal + autosave + `project.recover` (TODO L25, L30) | |
| Undo | not documented by WC | Have | 100-snapshot undo (TODO L21) | |
| Localisation (12 languages) | WCR §7 | Missing | none | |
| Crash reporter | WCR §1.1 (2026.2) | Partial | `diag.bundle` (manual) | |
| Platform | WCR §7 (Windows only) | Exceeds | macOS native verified; Windows/Linux build in CI (TODO L36) | Windows/Linux desktop not verified on a machine (TODO L32). |
| Licence tiers / export caps | WCR §1.2 | Exceeds | One edition, no tiers (TODO L6, ADR-0013) | |

### Automation, scripting and API
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| CLI / headless / API | WCR §6 (none) | Exceeds | `jd` CLI, `jd-mcp`, JSON schemas, generated TS types (TODO L27–29) | |
| Batch / MCP | WCR §6 (WC4 only, REPORTED) | Exceeds | MCP server, `graph.evaluate_async`, `jobs.*` | |

### Performance and tiling
| World Creator feature | WC evidence | Status | Our evidence | Notes |
|---|---|---|---|---|
| All generation on the GPU | WCR §2.1 | Partial | GPU for thermal + hydraulic; CPU elsewhere by design (TODO L74) | |
| Level-step pyramid | WCR §2.3 | Partial | Graph LOD + gate pattern (CLI "Erode a 16k world") | A different design, per WCR §13 "do differently" #2. |
| 16k worlds | WCR §4 | Have | 16k² under 4 GB, measured (TODO L73) | |
| Background full-res generation | WCR §2.4 | Partial | `graph.evaluate_async` | Not automatic after a preview. |
| Tile/region incremental re-eval, memory budget, cache GC | not in WC | Exceeds | TODO L31, L45 | |

---

## 2. Missing and Partial items, ranked by how central they are to World Creator's workflow

The ranking follows World Creator's own walkthrough order (WCR §10): shape → filters with distributions → simulations → objects → lighting → package → export/bridge. Items used in every project rank above optional ones.

1. **Image-based materials (Missing).** World Creator users texture every terrain with texture sets, Substance or Megascans, using triplanar, stochastic and height blending. *Sketch:* add `textures {albedo, normal, roughness, height}` (assets) to the material layer schema in `jd-terrain` (beside `detail`); bake in `terrain.textures` / `export_mesh`; do height blend there; add triplanar and stochastic to the viewport's TSL node material; carry them in `export_profile` like the detail tiles.
2. **Your own 3D models in scatter, plus align to normal and scale/rotation ranges (Partial).** This is the "Add Objects" step of the walkthrough. *Sketch:* `terrain.scatter {model: <asset .glb>}` instancing the user mesh through the existing `EXT_mesh_gpu_instancing` path; add `align_normal`, `scale_range` and `rot_range` to `ScatterParams` and a normal to `ScatterPoint`; draw the mesh in the viewport's `InstancedMesh`.
3. **Mask library gaps: occlusion, sun direction/aspect, cavity, pattern masks, curve/smoothstep, min/max/lerp (Missing/Partial).** Every filter, material and object in World Creator is driven by these distributions. *Sketch:* add `jd-graph` nodes `jd.analysis.occlusion@1`, `jd.analysis.aspect@1`, `jd.math.{min,max,lerp,curve,smoothstep}@1`, `jd.pattern.*@1`, each a small kernel in `jd-terrain`.
4. **Placed shape layers: stamp with transform, primitives, polygon, crater/volcano/island presets (Partial/Missing).** These are World Creator's "craft a shape" step. *Sketch:* `jd.shape.stamp@1` (asset + centre/size/rotation/blend op, reusing `import.rs` resampling), `jd.shape.primitive@1`, `jd.shape.polygon@1`, plus more entries in the desktop Preset list.
5. **Filter library gaps: domain warp/distortion, crater, inflate/deflate, denoise/spike removal, border blend/zero edge, irregular terraces (Missing).** These are used to detail most terrains. *Sketch:* expose a `warp {amp_m, feature_m, octaves}` field in `NoiseParams` (the module doc already claims it), then add `jd.filter.*` nodes in `jd-terrain`.
6. **Engine bridge depth: tiled export, Unreal Landscape/PCG/World Partition, Godot/Houdini/C4D (Partial/Missing).** Sync/bridge is the last step of every World Creator project. *Sketch:* `terrain.export_profile {tiles: n}` writing `height_x_y` + per-tile weights, and new profiles `godot` (Terrain3D) and `houdini` (EXR heightfield + mask layers). An Unreal editor utility for the Landscape actor needs the editor (no headless entry point, per `integrations/unreal/README.md`).
7. **Map export gaps: RGBA-packed splat, AO, metalness, topo/relief, Unity Mask Map, XYZ/ASC, 8-bit/half formats (Partial/Missing).** *Sketch:* extend `terrain.export_heightmap` formats and `terrain.textures` outputs (AO from a horizon/occlusion kernel shared with item 3); add mask-map packing in `profiles.rs`; contours via the `contour` crate already noted at TODO L75.
8. **Painted masks, biome layers and holes (Partial/Missing).** *Sketch:* add erase/fill tools to the strokes UI; `jd.mask.hole@1` → mesh export drops masked triangles; holes texture in the profiles.
9. **River editing and spline I/O (Partial/Missing).** *Sketch:* a `springs` param on `jd.hydrology.rivers@1` / network; a `.dae` or GeoJSON spline export op reading `jd.terrain.spline@1` points and the river network.
10. **Simulation layers: fluid viscosity/lava, debris (Partial).** *Sketch:* lava = a shallow-water variant reusing the hydraulic pipes with viscosity; debris = scatter from droplet `delta` deposition areas with `terrain.rocks`.
11. **Lakes drawn as water, i.e. multiple water bodies (Partial).** *Sketch:* the viewport builds per-basin water from the `fill.lakes` layer the way `buildSeaGeometry` does.
12. **Workflow conveniences: per-node presets with thumbnails, hover preview, expressions in fields, localisation, automatic crash reporter (Missing).** These help onboarding, not output. *Sketch:* desktop-only work in `apps/desktop` (presets as JSON param sets per kind; expressions evaluated client-side before `graph.set_param`).
13. **Rendering: path tracer, HDRI lighting import, post effects, cameras/flythrough/screenshots (Missing).** This is the least central for export users: World Creator says post and environment effects are not exportable (WCR §7), and WCR §13 "do differently" #11 advises against chasing them. *Sketch:* the cheap part only — load an HDRI as the three.js environment, tonemapping/exposure, off-screen PNG screenshot, camera keys on the existing timeline.

---

## 3. Jenga Dunia capabilities World Creator lacks

Absence on the World Creator side is per WCR §6, §12 and the live docs index checked 2026-10-24.

- **Headless and automation:** the `jd` CLI and the `jd-mcp` server, with every op in `schemas/v1/ops/index.json` under a JSON schema; async jobs with cancel, priorities and memory budget (TODO L31). World Creator has none (WCR §6); WC4 only promises them (REPORTED).
- **Deterministic content hash on every OS** (TODO L37) and a journaled project with crash recovery and undo (TODO L21, L25).
- **Hydrology:** priority-flood lakes, D8/D∞/MFD flow, TWI wetness, and hydrology-first terrain (`jd.hydrology.network@1`).
- **Coast and ocean:** coastal erosion with longshore drift and wave budget; sea exposure, refraction and currents; seabed classes; bathymetry infill; JONSWAP waves; water texture bakes (`ocean.waves`, `ocean.bake`).
- **Climate:** orographic rain with rain shadow, snowfall split by temperature lapse rate, snowpack with degree-day melt, albedo and avalanching, and a shared world wind.
- **Geomorphology:** stream-power erosion, karst sinkholes with underground drains, yardangs, Werner dunes with bimodal winds and vegetation, strata hardness.
- **Timeline:** any parameter can be keyframed, with frame export (`graph.set_time`, `timeline.export`) and scatter growth over time.
- **Meshes and interop:** GLB/FBX/OBJ/STL/PLY/USDA/USDC/USDZ; QEM decimation and LOD chains; MikkTSpace tangents; topology reports; FBX checked against the Autodesk SDK (`fbx.sdk_check`); a Maya check; GeoTIFF export with CRS; CRS reprojection on import; world origin rebasing.
- **Data sources:** free DEM fetch (Terrarium, Copernicus) with attribution files on every export; OSM features (`geo.fetch_features`); terrain from a photo via MoGe-2 / DA3 (`vision.depth`).
- **Scale tooling:** tile-level incremental re-evaluation, the gate pattern for 16k worlds, and a cache with garbage collection.
- **Platform and licence:** a native macOS app, open source, one edition with no export caps (TODO L6). World Creator is Windows-only, and its Indie tier caps export at 8k (WCR §1.2).

**Caveat on our own docs:** `crates/jd-terrain/src/noise.rs` line 4 says the module has "domain warp", but `NoiseParams` has no warp field. Either the comment or the feature needs fixing.

Sources: [CG Channel WC4](https://www.cgchannel.com/?p=179124), [Digital Production WC4](https://digitalproduction.com/2026/10/13/world-creator-4-planetary-terrain/), [WC docs index](https://docs.world-creator.com/llms.txt), [Object Collection](https://docs.world-creator.com/reference/scene/object-collection.md), [Conventional Export](https://docs.world-creator.com/reference/export/conventional-export.md), [Scene](https://docs.world-creator.com/reference/scene.md). No files were modified.