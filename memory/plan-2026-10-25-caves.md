# Plan — caves (owner, 2026-10-25)

Owner: "what about generating caves, paint entrances into caves, subterranean different geographical
and man made caves, from underground caves made of stalactites and stalagmites, caves under hills and
in mountains, to underwater caves in lakes, rivers and ocean; do research on what World Creator offers
and do more, like actually generating subterranean caves and being able to visualize, edit and put
well in the map".

Every stage: unit tests and / or specs, full gates, a commit (Opus 5.5 trailer), push, hosted CI.

| # | Stage | Status | Verification |
|---|-------|--------|--------------|
| 0 | Research: World Creator + tools (holes / caves / overhangs), cave geology + generation algorithms → `docs/research/caves.md`; ADR-0016 (caves as a node + an implicit volume beside the heightfield) | done `373cb8c`, `6b1978d` | sources tagged VERIFIED / REPORTED |
| 1 | Kernel `jd-terrain::caves`: cave system data (passages with cross-section profiles, chambers, entrances, speleothems, water level) → signed distance with noise → sparse surface-nets mesh clipped to the terrain; entrance hole mask; speleothem meshes | done `373cb8c` | unit: closed manifold mesh, cross-section sizes, holes only at entrances, stalactite above stalagmite |
| 2 | Generators per kind: karst (sinkholes → springs over inception horizons and fractures, phreatic tubes below the water table, vadose canyons above, chambers, speleothems, flooded parts), lava tube (under a flow path, skylights), sea cave (at sea level into cliffs, arches), hill / mountain cave, mine (adit 1:200, drifts, shafts), tunnel (two portals, grade limit), catacombs (shallow grid) | done `373cb8c`, research numbers `6b1978d` | unit per kind with the geology numbers from the research |
| 3 | Graph node `jd.cave.system@1` (params = the editable cave; outputs terrain + `holes`) and ops `caves.mesh` (viewport arrays) / `caves.export` (GLB, FBX, JSON skeleton aligned with the terrain export) | done `373cb8c` | ops tests; schemas regenerated |
| 4 | Desktop: Caves panel (kind presets, entrance painting by clicking the terrain, entrance list, regenerate), caves drawn in the viewport with an x-ray / cut-away of the terrain, flooded parts tinted, speleothems; e2e spec | done `eb41c01` | e2e + screenshot viewed |
| 5 | Engines: profiles carry the cave meshes and the entrance holes; Unity import places the cave mesh under the terrain (holes via SetHoles), Unreal (Visibility layer + static mesh), Blender bridge imports caves | done `56e25a4` | headless Unity / Unreal / Blender checks |
| 6 | Docs (user guide with captures), parity note, todo + memory | done | captures viewed |

## Decisions
- Caves are a graph node, not a separate document: params are journaled, undoable, saved and edited by
  the generated inspector; geometry is regenerated deterministically from them.
- Mesh coordinates follow the terrain export: x = u − extent_x/2 (east), y = height, z = extent_y/2 − v.

## Log
- start: plan written; two research agents running (tools survey, geology + algorithms).
- 373cb8c kernel + generators + node + ops + ADR-0016 + research part 1; 6b1978d sizes from the
  literature + research part 2; eb41c01 desktop Caves panel; 56e25a4 engines (Unity entrances
  within 2.25 m of the placed mesh, Unreal axis mapping exact, Blender ray down the mouth).
- Not taken (follow-ups, from the research): speleothem density from a vegetation / soil layer;
  maze presets (gypsum, flood mazes); flank-margin caves and cenotes as collapses into flooded
  passages; lava-tube braids and stacked levels; sea-cave blowholes and raised caves at old sea
  levels; karstnet network statistics as a validation test; an acceptance-report item.
