# Interoperability Matrix

Auditable per-format status (brief §9.5). Status values: `verified` (validator/import test output attached in `docs/research/export-validation/`), `experimental`, `planned`, `unverified` (tooling unavailable), `unsupported`. **All rows are `planned` today — no exporter exists yet.** Phases per ADR-0012. A format appearing here is not a claim of compatibility until its status says `verified`.

| Format | Import | Export | Round-trip | Geometry | Materials | Textures | Hierarchy | Instancing | Animation | Known losses | Dependencies | Licence | Validation | Status |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| glTF 2.0 / GLB | P1 | P1 | P1 | tri meshes, u16/u32 idx, MikkTSpace tangents | PBR MR, texture_transform, emissive_strength | PNG/JPEG (P1), KTX2 (P3) | nodes/scenes | EXT_mesh_gpu_instancing (P2) | no | no displacement/terrain layers; UV sets ≤ 2 practical | hand-written GLB today (`jd-terrain::mesh`); `gltf`/`meshopt` when textures/LOD land | MIT/Apache-2.0 | gltf-validator (in CI, `apps/desktop/scripts/validate-glb.mjs`), gltf-transform | **export shipped 2026-10-20**: grid mesh, normals, UVs, flat PBR; textures/instancing planned |
| USDA | — | P1 | P2 | UsdGeomMesh | UsdPreviewSurface | path refs | Xform prims | PointInstancer (P2) | no | DCC shader graphs | own writer | — | usdchecker, usdcat | planned |
| USDC / USDZ | P2 | P2 | P2 | same | UsdPreviewSurface; MaterialX (P3) | png/jpg/exr inside usdz (stored, 64-B aligned) | same | same | no | usdz size limits for ARKit | `openusd`, `zip` | MIT + TOST-1.0 | usdchecker --arkit, usdzip | planned |
| FBX | P2 (`ufbx`) | **✔ own binary 7.4 writer (`jd-terrain::fbx`, quads, normals, UVs, Phong + embedded texture; verified in Blender 5.2 + Optima)**; SDK adapter opt-in only | no | meshes, normals, UVs | Phong/Lambert; PBR lossy | embedded/ref | nodes | none standard | no | proprietary; materials lossy | `ufbx`; Blender; opt SDK | MIT/Unlicense; GPL (bridge, separate); proprietary (SDK, user-installed) | none open | planned |
| OBJ/MTL | P1 ✔ (OBJ, no MTL) | P1 | P1 | polys, normals, UVs | MTL basic | refs | groups | no | no | no units/axis/hierarchy | `tobj` | MIT | — | planned |
| STL | P1 | P1 | P1 | tris | no | no | no | no | no | geometry only | `stl_io` | MIT | — | planned |
| PLY | P1 | P1 | P1 | verts/faces/colours | no | no | no | no | no | — | `ply-rs` | MIT | — | planned |
| EXR f32 heightmap | P1 | P1 | P1 | raster | — | — | layers | — | — | engines don't import EXR terrain directly | `exr` | BSD-3 | exrinfo | planned |
| PNG16 / TIFF 16/32f | P1 | P1 | P1 | raster | — | — | — | — | — | PNG no float | `image`, `tiff` | MIT/Apache | identify | planned |
| RAW r16/r32 + sidecar | P1 | P1 | P1 | raster | — | — | — | — | — | endian/flip ambiguity (toggles + sidecar) | none | — | engine import dialogs | planned |
| Weightmaps / splat | P1 | P1 | P1 | raster | — | — | — | — | — | engine-specific semantics | `image` | MIT/Apache | — | planned |
| GeoTIFF | P2 | P3 | P3 | raster + georef | — | — | — | — | — | no reprojection until `proj` (P4) | `tiff` | MIT | gdalinfo/listgeo (external) | planned |
| MaterialX .mtlx | P4 subset | P3 | no | — | OpenPBR / UsdPreviewSurface / gltf_pbr nodedefs | refs | nodegraphs | — | — | no Unity consumer; occlusion/displacement mapping losses | `quick-xml` | MIT | mxvalidate (external) | planned |
| OpenVDB | deferred | deferred | — | volumes | — | — | — | — | — | — | — | Apache-2.0 | vdb_print | unsupported (deferred) |
| 3D Tiles | — | P4 optional | — | glTF tiles | — | — | tileset.json | — | — | — | own | OGC | 3d-tiles-validator | planned |

## Target application tests (brief §9.6, §10)

| Target | Procedure | Automation | Status |
|---|---|---|---|
| Blender 5.x | headless `blender -b -P import_check.py` imports GLB/USD/OBJ, reports counts, re-exports | CI on runners with Blender | planned |
| Unity 6 | `terrain.export_profile unity`: RAW r16 (LE, row 0 south) + 8-bit weightmaps + README with Terrain Height / y offset; glTFast imports GLB | manual checklist in the README; `UNVERIFIED` until a Unity runner exists | shipped (export side) |
| Unreal 5.8 | `terrain.export_profile unreal`: PNG16 (row 0 north) + 8-bit weightmaps at heightmap resolution + README with Z scale = range × 100 / 512 and Z location; `W_ENGINE_SIZE` off the 1009…8129 sizes; Interchange glTF/USD | manual checklist in the README; `UNVERIFIED` until automated | shipped (export side) |
| Maya 2027 | `mayapy` USD/FBX import script | manual | planned |
| Web | three.js r186, Babylon 9.x, model-viewer load GLB (meshopt, KTX2) | Playwright in CI | planned |
| Apple | `usdchecker --arkit`, Quick Look | macOS runner | planned |
