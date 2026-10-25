# ADR-0016 — Caves as an implicit volume beside the heightfield

**Status:** Accepted · 2026-10-25 (owner request: "actually generating subterranean caves and being able to visualize, edit and put well in the map") · refines ADR-0005 (the heightfield stays the terrain)

## Context

A heightfield has one height per cell, so it cannot hold a void under the surface. The owner asked for real caves — karst caves with stalactites and stalagmites, caves under hills and mountains, flooded caves under lakes, rivers and the sea, sea caves, lava tubes, mines, tunnels and catacombs — generated, editable and placed sensibly in the world. The survey (`docs/research/caves.md`) found that World Creator's hole layers (2025.3) paint a 2-D mask only, Gaea and Instant Terra have no caves, World Machine's 2026 3-D terrain cannot cut a cave mouth, and engine tools (Digger PRO for Unity, Voxel Plugin for Unreal) carve caves by hand in the engine. No terrain generator generates a cave system that is joined to its terrain.

## Decision

1. **The terrain stays a heightfield.** A cave system is a separate, implicit volume: a skeleton of passages (polylines with a cross-section profile — tube, keyhole, arch, rectangle — and a size per point), chambers and entrances, turned into a signed distance with seeded wall noise (`jd-terrain::caves`). It is meshed with surface nets on a sparse voxel grid that only covers the skeleton, clipped where it rises above the ground.
2. **The terrain opens where the void breaks its surface.** The same signed distance, sampled at each terrain cell's surface point, gives the hole mask (`holes` output), which the existing hole plumbing carries to the GLB / FBX mesh, Unity's `SetHoles` and Unreal's Visibility layer. A mouth narrower than a terrain cell is widened to about one cell, so it can open the terrain at all.
3. **A cave is a graph node.** `jd.cave.system@1` holds the editable cave as params (kind, seed, placed entrances, size, reach, cover, levels, sea level, speleothems, roughness), reads the terrain and optional `water` and `inlets` inputs, and outputs the terrain, the `holes` mask and a `plan` footprint. Params are journaled, undoable, saved and edited by the generated inspector like every node; the 3-D geometry is regenerated deterministically from them by `caves.mesh` (viewport arrays) and `caves.export` (GLB / FBX).
4. **Geology drives placement.** One generator per kind (`jd-terrain::cavegen`): karst routes sinkholes to a spring over inception horizons and fracture sets with conduit reuse (Paris et al. 2021), lava tubes follow a flow path a few metres down, sea caves cut cliffs at sea level, mines drain at 1 in 200, tunnels check their grade, catacombs are a shallow maze. A water-depth input floods what lies under lakes, rivers and the sea.
5. **Speleothems are separate meshes**, placed at drip sites on dry ceilings and paired with stalagmites below; they are thinner than any useful voxel.
6. **Meshes share the terrain export frame** (x east, y up, z south, centred), so a cave file drops under the terrain mesh, the engine terrain or the Blender import without a transform.

## Alternatives rejected

- **A full voxel terrain** (World Machine 2026's route): every operator, export and engine path assumes a heightfield (ADR-0005); a voxel terrain would also lose the engines' native terrain systems, which is what the engines want.
- **Holes only** (World Creator's route): leaves the cave to be built by hand in the engine, which is what the owner asked us to do instead.
- **Caves as a separate project document with their own ops**: duplicates the journal, undo and inspector the graph already has.
- **Marching cubes or dual contouring**: surface nets gives a closed, smooth mesh with one vertex per cell and no case tables; sharp man-made edges are softened by about half a voxel, acceptable for drifts and tunnels at 0.25–1 m voxels.

## Consequences

- The cave mesh and the terrain mesh are separate objects; the seam at a mouth is an overlap (cave walls run one voxel above the ground and the terrain keeps a quarter-cell lip), not a stitched edge.
- On a coarse terrain grid an entrance is at least one cell wide.
- Cave geometry depends on the terrain it was generated in: a preview at a coarse LOD can differ in detail from the full-resolution export, so exports always regenerate at LOD 0.
- Large systems are meshed at a voxel size chosen from a triangle budget (400 k for the viewport, 1 M for export) unless `voxel_m` is given.

## Evidence

`crates/jd-terrain/src/caves.rs` and `cavegen.rs` unit tests (closed manifold tube at its size, profile shapes, holes only at the mouth, speleothems on dry ceilings, one test per cave kind with the geology numbers); `crates/jd-core/src/ops.rs` (`a_cave_node_opens_the_terrain_and_meshes_and_exports_its_cave`); Blender renders of a karst branchwork through the terrain; `docs/research/caves.md`.
