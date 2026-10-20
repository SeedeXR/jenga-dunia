# Acceptance scenario (brief §20) — run 2026-10-20T19:21:16.196Z

Machine: darwin arm64, node v26.11.1; `jd` = target/debug/jd; world 512² / 5120 m.

## Build

Graph: 33 nodes evaluated in 14029 ms (17654 ms wall incl. CLI startups). Layers: biome, floor, hardness, height, mask_forest, mask_grassland, mask_rock, ocean, rivers, road, seabed, shore, sinkholes, snow, yardang.

```
height: min 32.5 max 1265.1 mean 657.3 m
rivers: min 0.0 max 0.9 mean 0.0 m
ocean: min 0.0 max 1.0 mean 0.0 m
shore: min 0.0 max 1.0 mean 0.0 m
biome: min 1.0 max 6.0 mean 1.8 m
snow: min 0.0 max 1.0 mean 0.0 m
mask_forest: min 0.0 max 1.0 mean 0.1 m
seabed: min 0.0 max 3.0 mean 0.0 m
floor: min 32.5 max 1265.1 mean 657.3 m
hardness: min 0.2 max 1.0 mean 0.4 m
sinkholes: min 0.0 max 1.0 mean 0.0 m
yardang: min 32.5 max 1264.5 mean 655.9 m
road: min 0.0 max 1.0 mean 0.0 m
```
## Exports

| file | vertices | triangles | bytes | ms |
|---|---|---|---|---|
| terrain_lod0.glb | 262144 | 522242 | 20345836 | 1349 |
| terrain_lod1.glb | 66302 | 130560 | 6245712 | 12066 |
| terrain_lod2.glb | 33661 | 65279 | 3895572 | 14592 |
| terrain.usda | 16641 | 32768 | 2556027 | 112 |
| terrain.usdz | 16641 | 32768 | 2556449 | 125 |
| terrain.fbx | 262144 | 522242 | 25551996 | 375 |

## §20 items

| # | item | status | evidence |
|---|---|---|---|
| 1 | Landscape with mountains, valleys, a river and a coastal region | **VERIFIED** | noise → stream power (mfd) → rivers (dinf) → coastal erosion → sea level 350 m: height: min 32.5 max 1265.1 mean 657.3 m; ocean fraction 2.3 % (border-connected sea)<br>ocean floor: seabed classes up to 3 (sand / rock / mud / reef) and bathymetry sediment fill raised the mean floor from 657.27 to 657.28 m |
| 2 | Procedural erosion and hydrological modifications | **VERIFIED** | stream-power incision + river carving + coastal cut/fill in the same graph; evaluation 14029 ms<br>desert family on the same terrain: karst sinkholes (floor mask max 1.00, 0.05 % of the world), wind abrasion with strata hardness (yardang: min 32.5 max 1264.5 mean 655.9 m), a spline road corridor 0.51 % of the world |
| 3 | At least two distinct biomes | **VERIFIED** | biome classes present: max class id 6 (6 rules), class masks forest/grassland/rock published, snow cover mean 0.007 |
| 4 | PBR materials from procedural masks | **PARTIAL** | PBR maps baked from masks of the terrain itself (height ramp + slope, normal, roughness) and viewport PBR texturing; a material graph assigning user materials per biome mask is not built yet |
| 5 | Lighting, atmosphere and weather | **PARTIAL** | sky.bake: Preetham daylight HDRI 1024×512 EXR + preview, zenith rgb 0.089/0.181/0.399; desktop sun + turbidity drive the same sky on the dome<br>shared weather state: project.set_settings {wind 270° → 90°} re-evaluates the snow node, which has no wind of its own (cover hash 523e21bb → bce97fb4, mean 1.19 % → 1.19 %); dunes, yardangs and the wind sea follow the same setting<br>timeline: snow_line_m keyed 950 → 700 over t = 0..1; snow cover 1.19 % at t = 0 → 3.85 % at t = 1 (journaled graph.set_time, cached per time)<br>clouds: a seeded cover layer on the sky bake/preview (cloud_cover); fog and rain are not modelled |
| 6 | Optimised meshes with multiple LODs | **VERIFIED** | LOD chain: 522242 → 130560 → 65279 triangles (quadric decimation, targets ¼ and ⅛), 26658 ms |
| 7 | Baked texture and mask outputs | **VERIFIED** | terrain.textures: albedo/normal/roughness 512×512 PNGs in 503 ms<br>exr heightmap 1033617 bytes<br>png16 heightmap 480973 bytes<br>tiff32 heightmap 1048898 bytes<br>ocean.bake: water_depth.png, water_colour.png, water_foam.png, wave_normal.png, wave_displacement.png 512×512, max depth 292.7 m, waves Hs 3.54 m / L 137 m |
| 8 | Export to GLB/glTF | **VERIFIED** | GLB with normals, UVs, tangents, albedo+normal+roughness maps: 20345836 bytes<br>scatter instances GLB (EXT_mesh_gpu_instancing): 1207 points, 49892 bytes<br>rock generator: 2 rocks of 320 triangles; 5933 rock instances on the rock mask, 265844 bytes |
| 9 | USD / USDZ | **VERIFIED** | USDA 2556027 bytes, USDZ 2556449 bytes (step 4) |
| 10 | FBX through a verified implementation | **VERIFIED** | FBX binary 7.4 (own writer): 262144 vertices, 25551996 bytes<br>FBX read back by our own reader with zero height difference |
| 11 | Validate exports against format specifications | **VERIFIED** | terrain_lod0.glb: 0 errors, 0 warnings, 0 infos, 0 hints; 522242 triangles, 262144 vertices<br>terrain_lod1.glb: 0 errors, 0 warnings, 0 infos, 0 hints; 130560 triangles, 66302 vertices<br>terrain_lod2.glb: 0 errors, 0 warnings, 0 infos, 0 hints; 65279 triangles, 33661 vertices<br>trees.glb: 0 errors, 0 warnings, 4 infos, 0 hints; 6 triangles, 5 vertices<br>rocks.glb: 0 errors, 0 warnings, 4 infos, 0 hints; 320 triangles, 960 vertices<br>rocks/rock_0.glb: 0 errors, 0 warnings, 1 infos, 0 hints; 320 triangles, 960 vertices<br>usdchecker terrain.usda: Success!<br>FBX round trip through our reader: height − fbx_in in [0.0000, 0.0000] m |
| 12 | Import into Blender | **VERIFIED** | Blender terrain_lod0.glb: 1 objects, 262144 vertices, 5110×5110×1232.5 m, 1 materials, 3 images<br>Blender terrain.fbx: 1 objects, 262144 vertices, 5110×1232.5×5110 m, 1 materials, 1 images<br>Blender trees.glb: 1207 objects, 5 vertices, 0.5×0.5×0.8 m, 1 materials, 0 images<br>Blender rocks.glb: 5933 objects, 960 vertices, 3.6×3.3×2.6 m, 1 materials, 0 images |
| 13 | Unity, Unreal and a browser viewer | **PARTIAL** | Unity 6000.4.8f1 headless import of the unity profile: TerrainData 513² from a 512² r16, size 5120×1233×5120 m, sampled height 76.9…1227.5 m, 4 terrain layers from the weightmaps; notes: source 512×512 is not a 2ⁿ+1 square: TerrainData resolution 513, the extra row/column is left at the edge value<br>unreal profile written: 7 files (engine-side import not run on this machine)<br>Unity import verified headlessly (above); Unreal: no engine build on this machine (the Epic launcher only); the browser viewer is the desktop app's three.js viewport (WebGPU), which renders the same layers but does not load the exported GLB |
| 14 | Geometry, scale, material fidelity, texture integrity, scene structure | **VERIFIED** | scale: GLB footprint 5110 m vs 5110 m expected (cell centres); FBX footprint 5110 m; textures 3 images on the GLB material |
| 15 | Record losses, performance, compatibility | **VERIFIED** | this report records the numbers; conversion losses: PNG16 quantisation (W_EXPORT_LOSS), USD without textures (step 4), FBX albedo only (no normal/roughness maps) |
| 16 | Reproduce through the CLI | **VERIFIED** | every step above ran through `jd --json op …` on Acceptance.jdproj |
| 17 | Reproduce through MCP | **VERIFIED** | MCP: 46 tools; project.open ok; graph.list 37 nodes<br>MCP graph.evaluate lod 1: ok in 6096 ms |
| 18 | Logs, cancellation and recovery | **PARTIAL** | detached job 2 cancelled: E_CANCELLED (ok means it finished before the cancel landed, which the fast lod-1 cache allows)<br>recovery: 1 pending journal entry replayed by `jd open --recover`, graph.json now has snow_line_m = 900<br>logs: every CLI run logs through tracing (JD_LOG); no per-project log files yet (stderr only) |

Summary: 14 VERIFIED, 4 PARTIAL. Project and exports: `/Users/alexmkwizu/Documents/SoftwareProjects/jenga-dunia/target/scratch/accept-full`.
