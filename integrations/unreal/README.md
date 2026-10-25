# Unreal Engine import (headless verification)

`jd_import_check.py` runs inside `UnrealEditor-Cmd` (`-ExecutePythonScript`, unattended, no
renderer) and reports, as JSON next to the input, what Unreal made of a Jenga Dunia export:

- `JD_PROFILE=<dir>` — a `terrain.export_profile {profile: "unreal"}` directory: every PNG goes
  through Unreal's texture importer; the report carries each asset's size and asset-registry
  `SourceFormat` (`TSF G16` for the heightmap, `TSF G8` for the weightmaps is what the profile
  promises and what `ok` requires) and the README's Z scale parsed back.
- `JD_MESH=<file>` — a `.glb` or `.fbx` from `terrain.export_mesh`: the Interchange importer;
  the report carries the StaticMesh's triangles, vertices, sections, UV channels, bounds in
  centimetres and materials, plus whatever came alongside (textures, a material instance).
  Interchange builds imported meshes as **Nanite** by default (UE 5.8
  `InterchangeGenericMeshPipeline::bBuildNanite = true`), and a Nanite mesh's LOD0 render data
  is its *fallback* — reduced to `FallbackRelativeError` 1.0 (a 524 288-triangle terrain gets a
  1 026-triangle fallback). The report's `triangles` / `vertices` are therefore the asset
  registry's `NaniteTriangles` / `NaniteVertices` (the mesh Unreal keeps and renders), with the
  fallback under `nanite`; acceptance requires them to equal what the exporter wrote.

`scripts/unreal-import-check.sh` is the wrapper: it writes a Blueprint-only `.uproject` under
`target/scratch/UnrealJD` (`DisableEnginePluginsByDefault`: only Python, Interchange and
InterchangeEditor load), finds the newest engine under `/Users/Shared/Epic Games` (or `UNREAL` =
the `UnrealEditor-Cmd` binary) and exits 1 unless the report says `ok`. A run takes 9–12 s on an
M4 (profile or mesh; 17–19 s with the engine's default plugin set, reports identical). The very
first launch of a freshly installed engine on a machine takes minutes — it is not a shader
warm-up: that run compiled no shaders, it loaded 627 engine libraries at ~0.4 s each while macOS
validated and paged in the fresh install, and every later run, in any project, loads them in well
under a second. It is paid once per machine and engine version; a `-ddc=Cold` sandbox run (empty
derived-data cache) still finishes in 21 s. `-NoShaderCompile` aborts the editor (NotNull assert)
and the `pythonscript` commandlet is not used either: it asserts inside
`AssetTools.ImportAssetTasks` because the editor subsystems the importers need are not up
(evidence: `docs/research/research-log/2026-10-22-unreal-first-run.md`).

Not covered: the Landscape actor. Unreal has no scripting entry point that builds a Landscape from
a heightmap file; that step is the editor's Landscape mode → Import from file, with the Z scale
and location the profile's README states. Known loss, read from the engine source: UE 5.8's glTF
parser does not implement `EXT_mesh_gpu_instancing`, so the instanced scatter GLB imports as plain
geometry — use the CSV / DataTable route from `terrain.scatter {engine: "unreal"}`.

## Caves

A profile exported with `caves: true` carries `cave_<n>.glb` and `caves.json`. The headless check imports each GLB through Interchange and measures how the axes mapped: +x → X (east), +z → Y (south), +y → Z (up), metres → cm, 0 m error on each axis (UE 5.8). Place the cave actor at the Landscape's location + (extent_x × 50, extent_y × 50, 0) cm, unrotated; the README in the profile gives the numbers. The mouths open through `holes.png` on the Landscape's Visibility layer (the landscape material needs a Landscape Visibility Mask node).
