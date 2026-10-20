# Unity import (headless verification)

`Editor/JengaDuniaImport.cs` rebuilds a Unity Terrain from a `terrain.export_profile` directory —
`heightmap.r16` + its `.jd-height.json` sidecar, every `weight_<name>.png`, `materials.json`
(colours per layer) and `trees_unity.json` (from `terrain.scatter {engine: "unity"}`) — and
writes `jd-unity-import.json` next to it with what Unity actually got: TerrainData resolution,
terrain size, sampled height range, terrain layers, tree count, warnings.

Run it without opening Unity:

```
scripts/unity-import-check.sh <profile dir> [extent_x,extent_z]
```

The script creates a scratch project under `target/scratch/UnityJD` on first use, copies the
Editor script into it and runs `Unity -batchmode -nographics -executeMethod JengaDunia.Import.Run`.
It needs a Unity editor (`UNITY=/path/to/Unity` or the newest under
`/Applications/Unity/Hub/Editor`) with an activated licence; the acceptance scenario
(`scripts/acceptance.mjs`) runs it when one is found and marks item 13 from Unity's report.

Conventions it relies on (the same the profile README states): the r16 is little-endian with
row 0 = south, so `heights[y, x]` needs no flip; weightmaps have row 0 = north and are flipped;
weights are normalised per texel; `nx`/`nz` in `trees_unity.json` are the normalised
`TreeInstance.position`, the height is sampled from the terrain. A source that is not a 2ⁿ+1
square is placed on the next valid resolution with the edge value padding the last row/column —
the report says so.

`scripts/unity-import-check.sh --fbx <file.fbx>` runs the `Fbx` method instead: it copies the file into the scratch project, lets Unity's `ModelImporter` (the Autodesk FBX SDK behind it) import it and reports meshes, vertices, triangles and bounds — the cross-check for `jd export mesh terrain.fbx`.

Verified on Unity 6000.4.8f1 (macOS) with the acceptance world — see `docs/user/cli-quickstart.md`.
MIT, like the rest of the repository.
