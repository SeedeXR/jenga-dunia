# Plan — caves (owner, 2026-10-25)

Owner: "what about generating caves, paint entrances into caves, subterranean different geographical
and man made caves, from underground caves made of stalactites and stalagmites, caves under hills and
in mountains, to underwater caves in lakes, rivers and ocean; do research on what World Creator offers
and do more, like actually generating subterranean caves and being able to visualize, edit and put
well in the map".

Every stage: unit tests and / or specs, full gates, a commit (Opus 5.5 trailer), push, hosted CI.

| # | Stage | Status | Verification |
|---|-------|--------|--------------|
| 0 | Research: World Creator + tools (holes / caves / overhangs), cave geology + generation algorithms → `docs/research/caves.md`; ADR-0016 (caves as a node + an implicit volume beside the heightfield) | in progress | sources tagged VERIFIED / REPORTED |
| 1 | Kernel `jd-terrain::caves`: cave system data (passages with cross-section profiles, chambers, entrances, speleothems, water level) → signed distance with noise → sparse surface-nets mesh clipped to the terrain; entrance hole mask; speleothem meshes | todo | unit: closed manifold mesh, cross-section sizes, holes only at entrances, stalactite above stalagmite |
| 2 | Generators per kind: karst (sinkholes → springs over inception horizons and fractures, phreatic tubes below the water table, vadose canyons above, chambers, speleothems, flooded parts), lava tube (under a flow path, skylights), sea cave (at sea level into cliffs, arches), hill / mountain cave, mine (adit 1:200, drifts, shafts), tunnel (two portals, grade limit), catacombs (shallow grid) | todo | unit per kind with the geology numbers from the research |
| 3 | Graph node `jd.cave.system@1` (params = the editable cave; outputs terrain + `holes`) and ops `caves.mesh` (viewport arrays) / `caves.export` (GLB, FBX, OBJ aligned with the terrain export) | todo | ops tests; schemas regenerated |
| 4 | Desktop: Caves panel (kind presets, entrance painting by clicking the terrain, entrance list, regenerate), caves drawn in the viewport with an x-ray / cut-away of the terrain, flooded parts tinted, speleothems; e2e spec | todo | e2e + screenshot viewed |
| 5 | Engines: profiles carry the cave meshes and the entrance holes; Unity import places the cave mesh under the terrain (holes via SetHoles), Unreal (Visibility layer + static mesh), Blender bridge imports caves | todo | headless Unity / Unreal / Blender checks |
| 6 | Docs (user guide with captures), parity note, acceptance item, todo + memory | todo | captures viewed |

## Decisions
- Caves are a graph node, not a separate document: params are journaled, undoable, saved and edited by
  the generated inspector; geometry is regenerated deterministically from them.
- Mesh coordinates follow the terrain export: x = u − extent_x/2 (east), y = height, z = extent_y/2 − v.

## Log
- start: plan written; two research agents running (tools survey, geology + algorithms).
