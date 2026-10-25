# Caves — research (tools, geology, methods)

Owner request (2026-10-25): generate subterranean caves of many kinds (karst with stalactites and stalagmites, caves under hills and in mountains, flooded caves under lakes, rivers and the sea, sea caves, lava tubes, mines, tunnels, catacombs), paint entrances, visualise, edit and place them well in the map; "do research on what World Creator offers and do more". Decision: [ADR-0016](../adr/0016-caves-as-an-implicit-volume-beside-the-heightfield.md).

Part 1 is the tools survey; Part 2 covers the geology per cave type and the generation methods, with the numbers the generators use.

## Part 1 — What existing tools offer

Research date: 2026-10-25. Scope: what to match and where Jenga Dunia can go further (engines: Unity, Unreal, Blender only).

Evidence tags:
- **VERIFIED**: read in a primary source (vendor docs, release notes, vendor page/blog, the paper or its Crossref abstract).
- **REPORTED**: secondary source (news article, forum post, reseller page).
- **UNVERIFIED**: searched, could not confirm.
- "Not found" means the primary docs were checked and the feature is not mentioned there. That is evidence of absence in the docs, not proof the feature cannot be done with workarounds.

---

### 1. Capability matrix

Legend: **Y** = has it (VERIFIED unless marked R), **R** = reported only, **M** = manual only (artist builds it by hand), **N** = not found in primary docs, **n/a** = not applicable.

| Tool (version checked) | Cave interior geometry | Overhangs | Terrain holes | Cave entrances | Speleothems | Underwater / flooded caves | Man-made tunnels / mines | Editing UI for caves | Export to Unity / Unreal / Blender |
|---|---|---|---|---|---|---|---|---|---|
| World Creator 2025.3 to 2026.4 | N (mask only) | N | Y (Hole Layers, painted or imported mask) | Y, as a painted hole mask ("cave entrances") | N | N | N | Paint/erase/smooth brush, area transform | Bridges for all three exist; bridge docs do not mention holes (N). Mask export format not documented |
| Gaea 2.3 / Gaea 3.0 (EA) | N | N (2.5D; "2.7D displacement" in 3.0 still exports 2.5D heightfields) | N (Gaea 3 "Terrain Clipping" in viewport and mesh export: UNVERIFIED whether this is holes) | N | N | N | N | n/a | Unreal and Houdini bridges; heightfields/meshes |
| World Machine "Dragontail Peak" (builds 4059 to 4065, 2026) | Y (VDM terrain "can create an entire cave system") | Y (VDM) | N ("VDMs don't support holes") | N (vendor: cannot leave an opening) | N | N | N | Node graph (VDM Primitive/Noise/Displace/Repair) | glTF/OBJ/GLB mesh, 32-bit EXR VDM, or hybrid heightfield + cliff mesh |
| Instant Terra (Wysilab) | N | N | N | N | N | N | N | n/a | Heightmaps, masks, FBX mesh; Unreal plugin |
| TerreSculptor (Demenzun) | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | n/a | Heightmaps (R) |
| Unity Terrain (Unity 6.x docs) | N (use ProBuilder etc.) | N | Y (Paint Holes tool, `TerrainData.SetHoles`) | M (hole + hand-built mesh) | N | N | N | Paint Holes brush | Native |
| Digger / Digger PRO for Unity (8.1, 2026-03-29) | Y (voxels + Marching Cubes, hybrid with Unity terrain) | Y ("Add" tool) | Y (cut automatically when digging, plus manual Paint Holes) | Y (dig through surface; HalfSphere brush "best for cave entrances") | Y, manual: "Stalagmite" brush (Shift for stalactites) | N | Y, manual: spline Dig for tunnels; RoundedCube brush "mine shafts"; procedural `CaveGenerator` spline | Scene-view brushes, splines, custom mesh brushes, runtime digging (PRO) | Unity only |
| Unreal Landscape (UE 5.8 docs) | N (place Static Mesh actors) | N | Y (Visibility tool + Landscape Visibility Mask node) | M | N | N | N | Visibility paint (binary) | Native |
| Voxel Plugin 2 for Unreal (2.0p8, UE 5.6/5.7) | Y (Volume stamps: distance field) | Y | n/a (one volumetric surface, no heightfield hole needed) | Y (volume stamp intersects height stamp) | N (no dedicated tool found) | N | Y, generic: spline stamps, shape stamps, mesh stamps | Stamps, sculpt actors, voxel graphs, runtime sculpt (experimental) | Unreal only; Heightmap stamps import textures |
| Blender 5.2 LTS geometry nodes | Y (procedural via SDF grids: Mesh to SDF Grid, SDF Grid Boolean, Grid/Volume to Mesh) | Y | Y (mesh boolean / delete) | M | N | N | M | Node graph; Import OpenVDB (.vdb) node | Native |
| Houdini 22.0 heightfields + VDB | Y via VDB (Convert HeightField to VDB, then VDB booleans) | Y via VDB | M | M | N | N | M | Node graph | Houdini Engine (not checked here) |
| e-on VUE (Vue 8, 2009, to Vue 2024; discontinued, free) | Y (R: 3D sculpting "overhangs and caves") | Y (R) | n/a | R | N | N | N | 3D sculpt brushes (R) | Not checked |
| Terragen (Planetside) | Render-time only (vector displacement / Redirect shader) | Y (vector displacement) | N | M | N | N | N | Shader network | Displacement geometry is not exported (docs) |
| Minecraft 1.18 (Mojang) | Y (noise caves: cheese, spaghetti, noodle; carvers, ravines) | Y | n/a (voxel world) | Y (where noise caves intersect the surface) | Y (dripstone caves biome) | Y (aquifers: local water levels, flooded caves; lava aquifers R) | Y (mineshafts) | n/a (game) | n/a |
| Research: Paris et al. 2019 (TOG) / 2021 (CGF) | Y (implicit/SDF construction trees; karst networks from geology) | Y | n/a | Y (amplifies existing heightfields) | Paper's abstract names stalagmites/stalactites as karst traits; generation of them not confirmed from the abstract | N | N | Interactive authoring of construction trees (2019) | n/a |

**Headline**: no heightfield terrain generator (World Creator, Gaea, Instant Terra) generates cave interiors. World Creator only paints a hole mask. World Machine's new VDM terrain makes overhangs and closed cave pockets but cannot make a cave mouth. Cave geometry is always added downstream, by hand (ProBuilder, static meshes) or by an engine-side voxel tool (Digger, Voxel Plugin).

---

### 2. World Creator (BiteTheBytes)

Current releases: 2026.1 (2026-01-28, REPORTED: cgchannel), 2026.2 (2026-03-03, REPORTED), 2026.3 (2026-03-18, REPORTED), 2026.4 (date not on the page). Cave-related work arrived in **2025.3** (2025-11-26, REPORTED date from CG Channel).

- **VERIFIED**: Hole Layers "enable you to add holes for caves or other structures that should pierce through the landscape by either painting directly in World Creator or importing a bitmap from an external source." Doc figure captions: "Putting a cave entry in a mountain", "Full terrain with the cave". https://docs.world-creator.com/reference/terrain/hole-layers.md
- **VERIFIED**: Hole Layer parameters: Show Mask, Import/Export of the mask, Edit; area settings X/Y, Area Scale, Rotation, Fit to Terrain, Blending, Blend Size, Blend Roundness; brushes Paint / Erase / Smooth; actions Flip X/Y, Rotate 90°, Clear (0), Fill (1); brush falloff, radius, smooth, strength, rotation, random rotation, align to draw direction. Same URL.
- **VERIFIED**: 2025.3 release notes: "Paint Holes in Terrain. Create cave entrances directly in World Creator — perfect for visualizing caves, tunnels, and dungeons. Export the cave mask to accurately place and render the entrance in any game engine or 3D application." https://docs.world-creator.com/release-notes/version-2025.x/world-creator-2025.3
- **VERIFIED (absence)**: Release notes 2026.1 to 2026.4 mention no holes, caves, tunnels or overhangs. 2026.3 added GIS imports, terrain adaptation to placed objects, object-area distributions. 2026.4 added decals, normal blending between objects and terrain. https://docs.world-creator.com/release-notes/version-2026.x/world-creator-2026.3 , https://docs.world-creator.com/release-notes/version-2026.x/world-creator-2026.4
- **VERIFIED (absence)**: The Unity, Unreal and Blender Bridge pages and the Conventional Export page (Height, Splat, Color, Normal, Mesh, Roughness, Metalness, Object details/instances, Spline, Topo, Heat, Simulation, AO, Unity Mask Map, Octane) do not mention holes, hole masks or caves. So how a hole reaches Unity's `SetHoles` or Unreal's visibility layer is **not documented**; the user exports the mask and wires it up. https://docs.world-creator.com/reference/export/bridge-tools/unity-bridge.md , https://docs.world-creator.com/reference/export/bridge-tools/unreal-bridge.md , https://docs.world-creator.com/reference/export/bridge-tools/blender-bridge.md , https://docs.world-creator.com/reference/export/conventional-export.md
- **VERIFIED**: 3D Stamp shape layer imports a custom 3D model and adds it "to the height map", so it is still heightfield data, not overhangs. https://docs.world-creator.com/reference/terrain/shape-layers/3d-stamp.md
- **REPORTED**: Digital Production: "carve holes directly into terrain for caves, tunnels or dungeons"; hole positions exported "as a mask"; no interior geometry described. https://digitalproduction.com/2025/11/28/world-creator-2025-3-adds-orbx-godot-bridge-and-caves/ . CG Channel: "paint holes into terrain to create cave or tunnel entrances", exported to engines "as a mask". https://www.cgchannel.com/2025/11/bitethebytes-releases-world-creator-2025-3/
- **UNVERIFIED**: any World Creator overhang feature (none found in docs or release notes). Official YouTube video descriptions for the hole feature were not found.

**Answer to the owner's question**: World Creator generates **no cave geometry and no overhangs**. It paints a 2D hole mask (a "cave entrance") that is shown as a hole in its viewport and exported as a mask; the engine or DCC must supply the cave.

---

### 3. Gaea (QuadSpinner)

- **VERIFIED**: QuadSpinner's Gaea page says its TOR engine works "between 2D and 3D, letting you pack higher levels of detail into a 2.5D heightfield"; product page lists "2.7D Terrains"; exports heightfields, meshes, point clouds; bridges for Unreal and Houdini. No mention of caves, overhangs or holes. https://quadspinner.com/Gaea
- **VERIFIED**: Gaea 3 page: "Gaea 3 debuts a revolutionary 2.7D Displacement system", described as moving beyond 2.5D while still exporting 2.5D heightfield data; "Triplanar Displacement"; release "estimated to be in October 2026". No caves/overhangs/holes. https://quadspinner.com/Gaea3
- **VERIFIED**: Gaea 3.0 development update (2026-06-04): Early Access July 2026; image caption "Terrain Clipping, both in viewport and Mesh export". Whether clipping means holes: **UNVERIFIED**. https://blog.quadspinner.com/gaea-3-0-development-update/
- **VERIFIED (absence)**: QuadSpinner blog post list through 2026-06 (Gaea 2.2, 2.2.7, 2.3, 3.0 posts) has no cave/overhang/hole post. https://blog.quadspinner.com/
- **REPORTED**: Gaea 2.2 (July 2025) added Glacier, IceFloe, Snowfield, Shape Primitive, Heal nodes; nothing cave-related. https://www.cgchannel.com/?p=168466

### 4. World Machine (World Machine Software)

The most important competitor move: **true 3D terrain via Vector Displacement Maps (VDM)** in the "Dragontail Peak" series (2026).

- **VERIFIED**: Build 4059 notes: "overhangs, undercuts, caves, and proper cliffs" are "first-class citizens"; devices VDM Primitive ("localspace spheres, squares, and cliffs"), VDM Noise ("true 3D displacement, not just elevation"), VDM Displace, VDM Repair ("self-intersections, cracks, and tangles"); export directly, convert to 3D mesh, or "a hybrid heightfield+cliff mesh setup"; Meshify supports VDMs; GLB from Mesh Output. Limitations: many VDM devices experimental, some devices lack VDM support, "VDM → HF+Mesh doesn't always produce a good seam between the heightfield and the 3d cliff mesh", erosion on VDM not finalized; planned mesh-to-VDM import. Page updated 2026-08-29. https://help.world-machine.com/topic/build-4059-dragontail-peak/
- **VERIFIED**: Vendor blog "Beyond Heightfields" (Stephen Schmitt, 2026-04-10): a VDM is "simply-connected genus zero. That means: no holes"; "you also can create an entire cave system if you want", but "you can't leave an blank opening somewhere (that would be a hole)"; "You can't create an arch"; VDMs output as 32-bit EXR, meshes, or hybrid; "you may not be able to use them natively" elsewhere. No Unity/Unreal/Blender specifics. https://www.world-machine.com/blog/2026/04/beyond-heightfields/
- **REPORTED**: CG Channel: current build 4065 adds macOS and Linux; exports glTF/OBJ meshes, EXR/PNG/TIFF textures; Indie $119, Pro $299. https://www.cgchannel.com/2026/07/world-machine-software-ships-world-machine-dragontail-peak/
- **REPORTED (historic)**: earlier forum consensus that heightfield WM cannot make caves/arches; users built them in ZBrush/Blender. https://forum.world-machine.com/t/caves-holes-tool/4533 , https://forum.world-machine.com/t/how-to-make-rocks-arch/6672

### 5. Instant Terra (Wysilab) and TerreSculptor

- **VERIFIED**: Instant Terra defines a terrain as a grid of vertices with heights and masks as 0..1 grids, "imported from image files and exported as image files"; the documentation index contains no cave, overhang, hole or voxel entries. Export nodes: terrain, mask, mesh (FBX), tiled. https://www.wysilab.com/OnLineDocumentation/Overview/AboutTerrainsAndMasks.html , https://www.wysilab.com/OnLineDocumentation/helpcontents.html
- **UNVERIFIED**: TerreSculptor cave/overhang/hole features. Only press coverage of TerreSculptor 2.0 being free (2020) was found. https://www.cgchannel.com/?p=111581

---

#### 6. Engine side

#### 6.1 Unity terrain holes
- **VERIFIED**: `TerrainData.SetHoles(int xBase, int yBase, bool[,] holes)`; array indexed [y,x]; `true` = surface, `false` = hole; recomputes LOD and vegetation per call, so use `SetHolesDelayLOD` then `SyncTexture` for interactive edits. https://docs.unity3d.com/ScriptReference/TerrainData.SetHoles.html
- **VERIFIED**: Paint Holes tool hides terrain "to simulate cave openings and overhangs"; build the cave with "other Editor tools, such as ProBuilder"; hole mask resolution = heightmap resolution minus 1 (1025 heightmap → 1024 mask); holes automatically excluded from the Terrain Collider; URP must enable Terrain Holes in the URP asset or holes vanish in builds; texture-based so edges can alias (hide with rock meshes or raise heightmap resolution). Docs version Unity 6.6. https://docs.unity3d.com/Manual/terrain-PaintHoles.html
- **VERIFIED**: `TerrainData.holesResolution` "Returns the Terrain holes resolution for both the data and the Texture." https://docs.unity3d.com/ScriptReference/TerrainData-holesResolution.html

#### 6.2 Digger / Digger PRO (Amandine Entertainment, Unity)
- **VERIFIED (vendor docs)**: Tools Dig ("caves, tunnels, or entrances"), Add ("overhangs"), Reset, Paint (cave textures use the terrain layers), Paint Holes ("cut holes in the terrain surface without generating cave geometry. Digger usually handles holes automatically when you Dig"), Smooth, Sharpen (beta). Brushes: Sphere (tunnels), HalfSphere ("Cave entrances", domed ceilings), RoundedCube ("Mine shafts", rooms), **Stalagmite** ("Use negative opacity or hold Shift to create stalactites"), Custom mesh brush (mesh voxelized). https://ofux.github.io/Digger-Documentation/Editing.html
- **VERIFIED (vendor docs)**: Advanced Operations: Bezier spline Dig/Add "Perfect for roads, tunnels, or rivers"; `CaveGenerator` class producing a procedural cave spline (step, stepCount, minY/maxY, altitude/horizontal variation frequency, three seeds); `SplineDecorator` spawns torches/rocks along cave paths. https://ofux.github.io/Digger-Documentation/AdvancedOperations.html
- **VERIFIED (vendor docs)**: voxels generated from the heightmap as an SDF by a pluggable `IVoxelGenerator` (Jobs + Burst); Advanced Voxel Generator has depth layers (texture, indestructible) and Perlin noise layers. https://ofux.github.io/Digger-Documentation/CustomVoxelGenerator.html
- **VERIFIED (vendor docs)**: FAQ: runtime digging is PRO only; Digger updates mesh colliders; NavMesh supported; data lives in `Assets/DiggerData` and is not included in exported Unity packages. https://ofux.github.io/Digger-Documentation/FAQ.html
- **VERIFIED (developer's own post, 2018-12-25)**: "uses voxels and Marching Cubes to create meshes", mixes them with Unity terrain instead of replacing it; PRO uses Unity's terrain holes (2019.3+). At that time caves could not be generated procedurally (since added via CaveGenerator above). https://discussions.unity.com/t/digger-caves-overhangs-for-unity-terrains/726282
- **VERIFIED (store page)**: Digger PRO 8.1, 2026-03-29, $99, Built-in/URP/HDRP on Unity 6000.0. https://assetstore.unity.com/packages/tools/terrain/digger-pro-voxel-terrain-sculpting-149753

**Relevance**: Digger is the de facto "caves on Unity terrain" answer and reads the heightmap, so a Jenga Dunia Unity export lands in a world where Digger can add to it. But Digger's caves are hand-carved (or one spline at a time); nothing generates whole cave systems, entrances and speleothems from terrain context.

#### 6.3 Unreal Engine landscape holes
- **VERIFIED**: Landscape Visibility tool "gives you a way to mask out (create holes) parts of your Landscape, for areas such as tunnels or caves"; the Landscape Material must use a **Landscape Visibility Mask** node; the separate Landscape Hole Material "is a legacy feature and is more costly to render"; painting is binary ("you only get a visible or invisible state"); caves are built "using additional Static Mesh Actors". Docs version UE 5.8. https://dev.epicgames.com/documentation/unreal-engine/landscape-visibility-tool-in-unreal-engine
- **UNVERIFIED**: whether the visibility layer can be imported from a file together with the heightmap (we did not find a doc page stating it), its resolution (per-vertex vs per-texel), and Nanite landscape behaviour with holes.
- **REPORTED**: UE-18612 (Won't Fix): landscape grass draws over visibility holes; workaround multiplies grass weight by the Landscape Visibility Mask. https://issues.unrealengine.com/issue/UE-18612

#### 6.4 Voxel Plugin 2 (Unreal)
- **VERIFIED**: docs 2.0p8 "targeted exclusively at Unreal Engine 5.6 and 5.7", actively developed and "can be buggy". https://docs.voxelplugin.com/
- **VERIFIED**: worlds are built from stamps that write **Height** (2D, one value per XY) or **Volume** (distance value per XYZ) data; "Volume stamps allow for the creation of overhangs and caves"; **Heightmap stamps** read a Heightmap asset built from ordinary textures plus weightmaps for surface types. https://docs.voxelplugin.com/knowledgebase/working-with-stamps/stamp-types/
- **VERIFIED**: shape stamps (spheres, cubes), spline stamps with per-control-point parameters, PCG spawning; generation is deterministic and on the fly (no pre-baked data); Nanite is the main render path; foliage via PCG. https://docs.voxelplugin.com/knowledgebase/working-with-stamps/stamp-types/spline-stamps.html , https://docs.voxelplugin.com/getting-started/working-with-voxel-plugin/
- **VERIFIED**: runtime sculpting is "experimental"; sculpt actors with diffing. https://docs.voxelplugin.com/knowledgebase/blueprints/runtime-edits-and-sculpting.html
- **VERIFIED**: licence $349 perpetual, sold on voxelplugin.com. https://docs.voxelplugin.com/resources/licensing.html

**Relevance**: a Jenga Dunia heightmap + weightmaps can feed a Voxel Plugin Heightmap stamp, and cave volumes could be emitted as mesh volume stamps. No preset cave types or speleothems are documented.

#### 6.5 Blender
- **VERIFIED** (Blender 5.2 LTS manual): Volume nodes include Grid to Mesh, Volume to Mesh (threshold/iso value, adaptivity, resolution modes), SDF Grid Boolean (union/difference/intersect on SDF grids), SDF Fillet, SDF Grid Offset/Mean/Median/Laplacian/Mean Curvature, Field to Grid, Voxelize Grid, Volume Cube; Mesh to SDF Grid (voxel size, band width; negative inside); an **Import OpenVDB (.vdb)** geometry node exists. https://docs.blender.org/manual/en/latest/modeling/geometry_nodes/volume/index.html , https://docs.blender.org/manual/en/latest/modeling/geometry_nodes/volume/operations/volume_to_mesh.html , https://docs.blender.org/manual/en/latest/modeling/geometry_nodes/mesh/operations/mesh_to_sdf_grid.html , https://docs.blender.org/manual/en/latest/modeling/geometry_nodes/volume/operations/sdf_grid_boolean.html
- Implication: Blender can receive a cave SDF as `.vdb` and mesh it natively (heightfield SDF minus cave SDF, then Grid to Mesh). No built-in cave generator; no vendor cave add-on was researched.

---

#### 7. Other tools

#### 7.1 Houdini (SideFX)
- **VERIFIED**: Houdini 22.0 heightfield docs, Limitations: "heightfields are flat 2D grids ... it's not possible to work on a terrain's vertical areas." https://www.sidefx.com/docs/houdini/heightfields/index.html
- **VERIFIED**: Convert HeightField SOP "Converts a 2D height field to a 3D VDB volume, polygon surface, or polygon soup surface"; VDB "takes a lot more memory than a height field"; since 16.0. https://www.sidefx.com/docs/houdini/nodes/sop/convertheightfield.html
- Not found: a SideFX or SideFX Labs cave tool. The standard workflow (convert to VDB, subtract cave VDBs, mesh) is **REPORTED** through third-party tutorials (e.g. https://lesterbanks.com/2023/05/procedural-tips-for-working-with-height-fields-and-vdbs-in-houdini/) and a user forum question about tunnels in heightfield mountains https://www.sidefx.com/forum/topic/66535 .

#### 7.2 e-on VUE
- **REPORTED** (press release republished 2009-11-04): Vue 8 added "3D terrain sculpting tools that allow for the creation of overhangs and caves directly into standard and procedural terrains." https://cgpress.org/archives/vue_8_infinite_and_vue_8_xstream_released.html
- **REPORTED**: Renderosity: sculpt pulls/inflates terrain in any direction; "Invert" replaces the old Dig tool; automatic subdivision inflates polygon counts. https://www.renderosity.com/article/14985/-
- **REPORTED**: Bentley discontinued Vue and made Vue 2023/2024 free (2024-05). https://www.cgchannel.com/2024/05/download-e-ons-plantfactory-plantcatalog-and-vue-for-free
- Not checked: Vue's own manual (not retrieved).

#### 7.3 Terragen (Planetside)
- **VERIFIED**: official tutorial: heightfields are poor at overhangs; vector displacement stores lateral offsets and handles them; ZBrush round-trip. https://docs.planetside.co.uk/wiki/Vector_Displacement_-_Terragen_to_Zbrush_and_back_again
- **VERIFIED**: displacement creates geometry at render time, not saved or exported with terrain. https://docs.planetside.co.uk/wiki/Displacement
- **REPORTED**: forum cave methods via Redirect shader / crater shader, result "more like a tunnel" (2007 to 2018). https://planetside.co.uk/forums/index.php?msg=31146 , https://planetside.co.uk/forums/index.php?msg=253461

#### 7.4 Minecraft 1.18 (Mojang) — published technique
- **VERIFIED** (snapshot 21w06a, Feb 2021): "Noise caves come in two flavors: Cheese caves. Like the holes in swiss cheese ... Spaghetti caves. Long squiggly tunnels"; old carvers and canyons still generate and combine with noise caves; "when noise caves intersect the surface they form cave entrances"; "An aquifer is an area with local water level, independent of sea level ... used during world generation to generate bodies of water inside noise caves"; magma at the bottom of underground water; underwater carvers removed in favour of aquifers. https://www.minecraft.net/en-us/article/minecraft-snapshot-21w06a
- **VERIFIED** (21w17a): "noodle caves – thinner, squigglier, and more claustrophobic variant of spaghetti caves". https://www.minecraft.net/en-us/article/minecraft-snapshot-21w17a
- **VERIFIED** (Caves & Cliffs Part II features): cheese/spaghetti/noodle caves; aquifers giving caves that are "dry, contain underground lakes or even be completely flooded"; cave biomes "lush caves and dripstone caves" can sit below surface biomes. https://www.minecraft.net/en-us/article/caves---cliffs-part-ii-the-features
- **VERIFIED**: mineshafts adapted to larger caves, supported by log pillars or chains. 21w06a / 21w07a pages above.
- **REPORTED**: lava aquifers below y0 (third-party). Henrik Kniberg's talks explaining the 3D-noise threshold technique exist on YouTube; the official upload was not confirmed.

#### 7.5 No Man's Sky, Astroneer
- **VERIFIED (session description only)**: GDC 2017 "Continuous World Generation in No Man's Sky" (Innes McKendrick): pipeline "from voxel-based world generation, through polygonization and texturing". Cave details not confirmed (talk behind GDC Vault). https://www.gdcvault.com/play/1024265/Continuous-World-Generation-in-No
- **UNVERIFIED**: Astroneer's terrain technique; no primary System Era source found.

#### 7.6 Research (relevant for "do more")
- **VERIFIED** (Crossref abstract): Paris, Galin, Peytavie, Guérin, Gain, "Terrain Amplification with Implicit 3D Features", ACM TOG 38 (2019): heightfields preclude arches/overhangs; memory-light implicit construction trees; primitives placed by Poisson sampling, shaped by shape grammars guided by stratified erosion and invasion percolation; amplifies existing heightfield terrains with "slot canyons, sea arches, stratified cliffs, fields of hoodoos, and complex karst cave networks". DOI 10.1145/3342765.
- **VERIFIED** (Crossref abstract): Paris, Guérin, Peytavie, Collon, Galin, "Synthesizing Geologically Coherent Cave Networks", Computer Graphics Forum 40(7):277-287 (2021): karst skeletons from a gridless anisotropic shortest path using inlets/outlets, faults, inception horizons, fractures, permeability contrasts; conduit geometry as an SDF construction tree with blending and warping; control over network structure and cross-sections. DOI 10.1111/cgf.14420.
- **REPORTED**: Peytavie et al. 2009 "Arches: a Framework for Modeling Complex Terrains" (hybrid volumetric/implicit, overhangs, arches, caves). https://perso.liris.cnrs.fr/egalin/Articles/2009-arches.pdf

---

### 8. Gaps we can exceed (no tool above offers these)

1. **Generated cave interiors inside a heightfield terrain generator.** WC/Gaea/Instant Terra only have masks or nothing; WM VDM has pockets but no mouths. Jenga Dunia can keep the heightfield as the surface and add a sparse **cave SDF layer** (union of tunnel/chamber primitives) that is generated, previewed and exported with the terrain.
2. **Automatic entrances.** Compute the hole mask from where the cave SDF crosses the terrain surface, instead of painting it (WC) or hand-matching a mask to a mesh (Unity, Unreal). One source of truth: move the cave, the entrance moves.
3. **Watertight mouth seam.** WM admits its heightfield+mesh seam "doesn't always" work; Unity warns of aliased hole edges. Generate the entrance lip mesh so its rim uses the exact heightfield vertices around the hole cells (Unity's mask is heightmap−1 cells, binary).
4. **Cave types as geology-aware presets**, none of which exist anywhere as presets:
   - Karst: network from inlets/outlets, bedding planes and fractures (Paris 2021 approach), chambers at junctions, surface **dolines/sinkholes** where chambers are shallow (ties into the desert/sinkhole mandate).
   - Caves under hills and in mountains: placement constrained by overburden depth and slope.
   - Lava tubes: follow downslope flow paths just under the surface, roughly constant cross-section, skylights where the roof collapses.
   - Sea caves: at the shoreline along cliff faces at sea level, wave-notch profile, partly flooded.
   - Underwater caves under lakes, rivers and ocean: flooded sections connected to existing water bodies, with per-cave water level (Minecraft's aquifer idea, but exported as water surfaces for engines).
   - Man-made: mines (straight drifts, vertical shafts, adits on slopes, timber/rail props as placement points), road/rail tunnels (spline with portals cut at both ends), catacombs (grid/maze of corridors with niches and chambers).
5. **Procedural speleothems.** Digger has one manual Stalagmite brush; nothing places them. Place stalactites on ceilings, stalagmites below them, columns where they meet, flowstone on walls, driven by depth and a drip/percolation proxy, exported as instance points.
6. **Cave visualisation**: x-ray/cutaway of the terrain, section slice planes, plan-view cave map overlay on the 2D map coloured by depth, entrance markers. No tool researched offers a map view of what is underground.
7. **Gameplay data out**: cave skeleton graph (splines + chamber nodes + entrances + water levels) exported as data for nav, spawns and lighting. Digger exposes only a spline it generated; nobody exports the network.
8. **One export bundle per engine** (nobody ships hole + interior together):
   - Unity: heightmap + `SetHoles` mask (size heightmap−1, `true`=surface) + cave mesh chunks with colliders/LODs; note URP Terrain Holes flag.
   - Unreal: heightmap + visibility layer (material needs Landscape Visibility Mask) + cave static meshes (Nanite), or a Voxel Plugin path (Heightmap stamp + mesh volume stamps).
   - Blender: terrain mesh with holes cut + cave mesh, or the cave SDF as `.vdb` for the Import OpenVDB / Grid to Mesh nodes.
9. **Non-destructive, seeded, editable caves in the layer tree**: drag a cave path on the map, change type/seed, everything (mask, mesh, speleothems, water) regenerates; WC's hole mask and Digger's sculpts are destructive bitmaps/voxel edits.
10. **Arches and natural bridges** come for free from the SDF layer; WM's VDM explicitly "can't create an arch".

Open items to research further if needed: Unreal visibility-layer import path and resolution; Nanite landscape with holes; TerreSculptor 3 features; VUE's manual on 3D sculpting; the Minecraft talk by Henrik Kniberg (official upload).

## Part 2 — Geology and generation methods

Research report, 2026-10-25. It covers the geology that should decide where and how each cave type forms (Part A), the algorithms to generate, represent and mesh caves (Part B), and a recommended Rust design (Part C) that fits the `jd-terrain::caves` kernel already in progress (`crates/jd-terrain/src/caves.rs`).

**Evidence tags.** **VERIFIED**: I read the claim in the primary source (paper PDF, official docs, crates.io/GitHub API, game data files). **REPORTED**: from a secondary source (Wikipedia, lecture notes, a search summary of an abstract, a citing paper). **UNVERIFIED**: plausible or commonly stated, but I found no source. "Not found" means I looked and found no number.

**Access limits.** Ford & Williams (2007) *Karst Hydrogeology and Geomorphology* (Wiley, DOI 10.1002/9781118684986) and Palmer (2007) *Cave Geology* (Cave Books) are not open access; I did not read them, and nothing in this report is attributed to their text. Palmer (1991) is cited from its abstract only. Where the textbooks would normally be the source, I used open papers (Kaufmann 2003, Frantz et al. 2021, Filipponi 2009, Collon et al. 2017, Paris et al. 2019/2021) instead.

---

### Part A. Geology per cave type

#### A1. Karst (solution) caves in limestone, dolomite and gypsum

**What drives them**

- Caves grow along the groundwater routes that carry the most flow and the most aggressive water. Routes that gain flow grow faster and the rest stall. Wall retreat is capped by chemical kinetics at "typically about 0.01-0.1 cm/yr". "Most caves require 10^4 - 10^5 yr to reach traversable size." VERIFIED (abstract only). Palmer 1991, GSA Bulletin 103:1–21, DOI 10.1130/0016-7606(1991)103<0001:OAMOLC>2.3.CO;2; abstract at https://digitalcommons.usf.edu/kip_articles/4015/
- **Pattern follows recharge.** "Sinkhole recharge forms branching caves with tributaries that join downstream as higher-order passages." "Diffuse recharge also forms networks and spongework." Rising thermal or H2S-rich water forms ramiform caves. VERIFIED (abstract), same source. For the generator: sinkholes as inlets give a **branchwork** converging on a spring. Diffuse recharge, or floods backing up behind a constriction, gives **mazes**.
- **Water table and base level.** Above the water table (vadose zone), conduits develop "preferentially vertically along fractures, with canyon passages linking them along inception horizons". Near and below it (phreatic zone), development "becomes progressively more horizontal-dominant". The phreatic zone "is generally directly connected to a spring (or the base-level)". When the base level was steady for a long time, conduits develop at or just below the water table. A water table that swings strongly with the seasons gives more maze patterns. Old base levels leave stacked horizontal levels. VERIFIED, Paris et al. 2021 §3 (synthesising Jouves et al. 2017), https://web.archive.org/web/2023id_/https://hal.archives-ouvertes.fr/hal-03331697/document
- **Four-state model.** Phreatic caves and water-table caves are the two end members. "The state (cave type) that develops is a function of the frequency of fissures penetrable by groundwater"; "the water-table type is the high frequency end member", and fissure frequency rises with time. Vadose caves are of "drawdown" type (following an earlier phreatic path) or "invasion" type. VERIFIED (abstract). Ford & Ewers 1978, Can. J. Earth Sci. 15:1783–1798, DOI 10.1139/e78-186, https://digitalcommons.usf.edu/kip_articles/10408/. Consequence: few fractures give deep phreatic loops (passages that dip below the water table and come back up); many fractures give passages that stay near the water table.
- **Depth of phreatic flow.** Worthington proposed that deep flow paths and deep cave genesis become more likely once the flow path is longer than about 3 km, and more so with steeply dipping strata, because water viscosity falls as geothermal heating warms it. REPORTED, from a search summary of a Geology 30(1) (2002) comment/reply record. I did not find a numeric depth law in a source I could read.
- **Inception horizons.** These are a few bedding partings that guide most of the dissolution. In the EPFL thesis behind Filipponi et al. 2009, "probably less than 10 % of the existing bedding partings of a limestone sequence are inception horizons", yet they "guide more than 70 % of the phreatic conduits". VERIFIED (thesis abstract), https://infoscience.epfl.ch/record/135612. The journal paper analysed "18 large cave systems around the world, comprising more than 1500 km of conduits". VERIFIED (abstract), Geomorphology 106:86–99, DOI 10.1016/j.geomorph.2008.09.010, https://infoscience.epfl.ch/record/134542
- **Fractures.** At the kilometre scale, fracture directions stay "almost constant", and conduits follow fracture planes. Passages formed along fractures "tend to be narrow and straight, often featuring sharp, right-angle turns". VERIFIED for the first point (Paris 2021 §4.2); REPORTED for the second (Culver & White, *Encyclopedia of Caves* 2nd ed., quoted in Ankara University lecture slides, https://acikders.ankara.edu.tr/mod/resource/view.php?id=174500).
- **Stair-step profiles.** Interbedded low-solubility layers (shale, clay, chert) perch water and put passages at the base of the soluble unit. Pits cut down through them along fractures, giving "a stairstep pattern". REPORTED, same Encyclopedia of Caves excerpt.
- **Share of land.** Karst landscapes cover "around 12% of the planet's continental surface". VERIFIED as cited in Paris 2021 (from Hartmann et al. 2014).

**Passage shapes (cross-sections)**

| Type | Where | Shape | Source |
|---|---|---|---|
| Phreatic tube | below the water table | "circular to elliptical"; the ellipse is horizontal, vertical or diagonal depending on whether it follows a bedding plane or a fracture; smooth walls with scallops | REPORTED (Encyclopedia of Caves via Ankara slides) |
| Vadose canyon | above the water table | "relatively tall and narrow … particularly where the hydraulic gradient is steep or the bedding is thick"; floor incised; rough walls; "pits represent the ultimate form of vadose passage" | REPORTED (same) |
| Keyhole | phreatic tube later drained | "a broad, smooth-walled phreatic tube sits above a narrow, rough-walled vadose canyon", formed as the water table falls | REPORTED (same) |
| Epikarst conduit | surface to vadose zone | narrow vertical; Paris 2021 uses a fixed radius of 0.5 m | VERIFIED (Paris 2021 §5.3) |
| Breakout chamber | where a tunnel crosses a weak (ghost-rock) zone | large void with collapse | VERIFIED (Paris 2021 §5.3) |
| Shaft / pit | vertical, vadose | "roughly circular" | VERIFIED (Paris 2021 §5.3) |

The canyon-to-tube transition does not always sit at the mean water table. In Bärenschacht (Switzerland) it is at the floodwater table. REPORTED (search summary of a Z. Geomorph. 2003 abstract, https://digitalcommons.usf.edu/kip_articles/4663).

**Passage dimensions (real data)**

- Frantz, Collon, Renard & Viseur 2021, Geomorphology, DOI 10.1016/j.geomorph.2020.107480, analysed 49 surveyed networks: median length 2135 m, median 269 survey points, a station every 7.5 m on average. The largest is Sieben Hengste, over 80 km. VERIFIED (full text, https://libra.unine.ch/bitstreams/44536da7-f28a-4ce2-a70e-b43788d6b063/download).
  - "The pooled mean of the equivalent radius is equal to 1.52 m"; pooled mean width/height ratio 1.10. Median of the network medians: radius 1.22 m, W/H 1.00. VERIFIED.
  - The distributions are strongly skewed and close to **log-normal**. There is "no generic distribution", and the hypothesis that conduit size is hierarchical (bigger downstream) was rejected. VERIFIED.
  - Size is **spatially correlated along a conduit**; exponential variograms fit well. Paris 2021 summarises the ranges as 50–200 m. VERIFIED (Frantz text for the correlation; Paris 2021 for the 50–200 m figure).
  - Generator rule: sample log(radius) along each passage as a 1D Gaussian process with a 50–200 m exponential correlation length, median about 1.2 m. Clamp it, and use W/H around 1 with log-normal spread. Do **not** make passages grow downstream.
- Paris 2021 uses random radii of 0.5–4.0 m (epikarst fixed at 0.5 m). VERIFIED.
- Minimum enterable size is "about 0.3 m diameter". REPORTED (White 1984 via Ankara slides).
- Speleogenesis time: a 5–15 mm proto-cave needs 3000–5000 years, and an opening of 1–10 m or more needs 5000–100,000 years. The author calls this "speculative". REPORTED (Earle, *Environmental Geology* §12.5, https://environmental-geol.pressbooks.tru.ca/chapter/origin-and-genesis-of-caves/).

**Network statistics (for validation)**

Collon et al. 2017, Geomorphology 283:122–142 (DOI 10.1016/j.geomorph.2017.01.034, corrigendum 2021), recommend 6 of 21 metrics. Paris 2021 Table 2 gives the observed range over 34 real caves. VERIFIED (Paris 2021 Table 2):

| Metric | Observed min | Observed max |
|---|---|---|
| average branch length *l* (m) | 8 | 331 |
| entropy of orientation *H_O* | 0.8 | 1.0 |
| average vertex degree *k* | 1.8 | 2.6 |
| correlation of vertex degrees *r_k* | −0.6 | −0.2 |
| average shortest path length *SPL* | 2.3 | 55.7 |
| central point dominance *CPD* | 0.0 | 0.6 |

The open-source **karstnet** Python package computes these. It is MIT licensed (VERIFIED, https://github.com/karstnet/karstnet LICENSE).

**Entrances**

These are the inlets and outlets of the karst system: sinks or dolines (point recharge), springs or resurgences "near a valley bottom" (the outlet at base level), and shafts (vertical vadose). VERIFIED for sink and spring roles (Paris 2021 §3; Earle §12.5 for the valley-bottom spring). Doline sizes are not covered here; the existing `karst.rs` sinkhole generator already owns doline shape.

#### A2. Speleothems (stalactites, stalagmites, columns, flowstone)

- **Mechanism.** CO2-rich soil water enters the cave supersaturated and precipitates calcite as CO2 degasses. Evaporation matters only near entrances. Stalactites are often hollow and stalagmites solid. Stalactites, stalagmites and flowstone take their shape from the water flow. REPORTED (Encyclopedia of Caves via Ankara slides). A drop hangs on the ceiling, builds a calcite ring and a hollow tube, then falls and builds a stalagmite below; "the opposing growth of stalactites and stalagmites might finally result in the union of the two, to form a column". VERIFIED as stated in Tortelli & Walter 2009, GRAPP, DOI 10.5220/0001787200270035 (citing Lino 2001).
- **Stalagmite growth needs vegetation.** "Stalagmite growth is only possible if the water dripping into a cave is supersaturated with calcium. Therefore the water needs high CO2 concentrations from the vegetation cover. If water seeps underground through bare rock, the CO2 concentration is too low." VERIFIED. Kaufmann 2003, EPSL 214:251–266, DOI 10.1016/S0012-821X(03)00369-8 (PDF via https://repository.geologyscience.ru/bitstreams/9a40d95d-7735-4393-af92-9eb09a34dccd/download). Placement rule: dense speleothems under soil- and vegetation-covered ground in humid climates, sparse under bare rock or arid ground. A desert cave should be mostly bare unless it is a relict.
- **Stalagmite growth rate.** In Kaufmann's model the growth rate "is always below 0.25 mm/yr" over the temperature and drip ranges considered. Measured rates of 0.01–0.05 mm/yr were found at three sites in SE England (Baker et al.). VERIFIED (Kaufmann 2003).
- **Stalagmite diameter is set by drip interval.** The equilibrium radius R0 = sqrt(V / (π δ (1 − exp(−α t_d / δ)))), where V is the drop volume (about 0.1 cm³), δ the film thickness (0.01 cm) and t_d the drip interval (1 to 10^6 s). For drip intervals above about 1000 s "a minimum equilibrium diameter is established (R0min ≈ 4 cm)", as Curl predicted. "Diameters above the 20 cm range require shorter drip intervals around 10–100 s", and "massive cone-shaped stalagmites with diameters in the meter range need both short drip intervals and low temperatures". VERIFIED (Kaufmann 2003 §2.2, Fig. 2). Note that the text labels the about-4 cm value as both R0min and a diameter, so treat it as a minimum diameter of a few centimetres.
- **Stalagmite shape law.** Each surface point grows along its normal at W_i = W0 · exp(−s_i / R0), where s_i is the arc length from the drip centre. The result is a candle (a flat-topped cylinder of radius about R0) that grows upward while conditions are constant. VERIFIED (Kaufmann 2003 Eq. 1). This is directly usable as a profile generator: integrate a 2D profile and revolve it.
- **Stalactite growth law and rate.** Short et al. 2005 (PRL 94:018501, DOI 10.1103/PhysRevLett.94.018501): the local growth rate is proportional to the local thickness of the fluid film. Shapes converge to a universal "Platonic" ideal. VERIFIED (DOI via Crossref; content via the CEIG 2024 paper below). Normal velocity v = α·l_Q·(l_Q/(r sin θ))^{1/3}, with l_Q ≈ 0.1 mm and α ≈ 1/yr, giving a length growth of about 0.1 mm/yr. VERIFIED as restated in Amorós-Tur & García-Fernández 2024 (CEIG, DOI 10.2312/ceig.20241145, CC BY, https://diglib.eg.org/bitstream/handle/10.2312/ceig20241145/07_ceig20241145.pdf). Measured natural stalactites matched the ideal shape to within 5 %. REPORTED (ScienceDaily press release, https://www.sciencedaily.com/releases/2004/12/041206193416.htm).
- **Other stalactite numbers.** Soda straws are about 4–5 mm in diameter. Stalactites average 0.13 mm/yr, the fastest reach about 3 mm/yr, and a Pb-210 study in Santana Cave (Brazil) gave 1.3 mm/yr for soda straws. The longest are 8.2 m (Jeita) and a reported 20 m (Gruta Rei do Mato). Lava stalactites reach "almost 2 meters". REPORTED (Wikipedia "Stalactite" and "Soda straw", which cite Hill & Forti *Cave Minerals of the World* and others).
- **Pairing.** A stalagmite forms on the floor directly below a dripping stalactite. Stalactites and stalagmites meet to form columns. VERIFIED (Tortelli & Walter 2009). Climate skews the pair: high, regular flow gives small or absent stalactites and fast-growing, voluminous stalagmites; low flow gives "stalactites larger than the stalagmites"; too little rain gives none. VERIFIED (Tortelli & Walter §3.2, after Lino 2001).
- **Flowstone and rimstone.** Not found with numbers. Treat as sheets on walls and floors downslope of seeps; no source-backed size rule.
- **Implication.** Over the 10^4–10^5 yr that cave formation takes, at 0.01–0.25 mm/yr a stalagmite can reach about 1–25 m in principle. The real limit is the ceiling height and how long conditions last. Use ceiling height as the hard cap and a column when stalactite length plus stalagmite height reaches the floor-to-ceiling gap. UNVERIFIED (my arithmetic on the verified rates).

Sections A3–A7 come from a second source pass with the same tagging rules.

#### A3. Lava tubes

- **Formation.** "Tubes form by the crusting over of lava channels and pahoehoe flows … drained tubes commonly exhibit 'high-lava' marks on their walls, generally flat floors, and many lava stalactites." VERIFIED, USGS glossary, https://volcanoes.usgs.gov/vsc/glossary/lava_tube.html
- **Roofing.** Greeley 1971 lists five roofing mechanisms: fused surface scum, fused crustal plates, crusts growing in from both banks, spatter levees arching over, and crusts extending the tube at flow toes. Tubes form "exclusively in pahoehoe". Earlier observers reported "tubes up to 16 m in diameter … the roof seldom remained unbroken". VERIFIED, NASA report, https://ntrs.nasa.gov/api/citations/19710009490/downloads/19710009490.pdf
- **Sizes and depth** (Sauro et al. 2020, Earth-Sci. Rev. 209:103288, DOI 10.1016/j.earscirev.2020.103288; VERIFIED):
  - "width and height vary from 0.5 m up to 30 m, developing from few centimeters below the surface up to a depth of few tens of meters".
  - Cross-sections are "arched, round, oval, or … keyhole shaped".
  - Shallow-inflation tubes are "overwhelmingly more frequent". They start as a horizontal ellipse and deepen by thermal erosion.
  - Overcrusted tubes have roofed sections "not exceeding few tens or maximum hundreds of meters in length and a few meters in diameter".
  - An exceptional fracture-guided tube (Wood Valley) sits at "almost 90 meters of depth".
- **Collapses and skylights** (VERIFIED, same paper):
  - Kazumura: mean collapse width 7.3 m vs intact tube width 10.5 m; collapse depth 5.2 m vs tube height 8.1 m; the tube stays intact along 97 % of the collapse chain.
  - Corona (Lanzarote): tube 7–27 m wide (mean 13.7 m), collapses 37.6 m wide.
  - Undara: tube 15–20 m wide, collapses up to 50–60 m wide.
  - Skylights are "rounded (L=W) … overhanging walls" and smaller than full-width collapses (jameos).
  - A USGS active-flow skylight is "about 7 meters … across" (REPORTED, https://www.usgs.gov/news/volcano-watch-skylights-mark-current-kilauea-lava-flows).
- **Kazumura Cave** (Allred & Allred 1997, JCKS 59(2):67–80; VERIFIED, https://web.archive.org/web/2023id_/https://legacy.caves.org/pub/journal/PDF/V59/V59N2-Allred.pdf):
  - "59.3 km with a vertical extent of 1098 m. Average slope … 1.9º".
  - "as much as 21 m wide and 18 m high", average cross-section 20.3 m².
  - Braids stay within about 140 m of the main tube; up to four stacked levels; "a third level is 7.5 m above the floor".
  - Thermal downcutting was measured at 10 cm/day.
  - The current 65.5 km length is REPORTED (Guinness/Wikipedia).
- **Shape vs slope.** Ape Cave (3,400 m) is "skull-shaped" along steep channels and "on gentle gradients … horizontally oval". VERIFIED, Greeley & Hyde 1971, https://ntrs.nasa.gov/api/citations/19710023544/downloads/19710023544.pdf
- **Lava stalactites.** Diameter "averages about .7 … from .4 to 1 cm"; "Lengths range from the perceptible to a meter and more." VERIFIED, Allred & Allred 1998, JCKS 60(3).
- **Not found:** a measured roof-thickness distribution, or a host-slope range beyond Kazumura's 1.9° average.
- **Generator rules.**
  - Centreline follows the lava flow's main channel at 1.9°-like gentle slopes.
  - Depth from a few metres to tens of metres; width 0.5–30 m, typical 7–20 m for big tubes; flat floor.
  - Use an oval (wide) section on gentle slopes and a tall or keyhole section on steep ones.
  - Optional stacked upper levels; braids within about 140 m of the main tube.
  - Skylights are round, smaller than the tube width, placed where the roof is thinnest. Collapse trenches can be up to 2–4× the tube width.
  - Lavacicles are 0.4–1 cm in diameter and up to about 1 m long.

#### A4. Sea caves

- **Formation.** "Most sea caves are formed along weaknesses in the rock, such as faults, fractures, or bedding/foliation planes and can occur in nearly every type of rock." Waves "exploit and enlarge … joints, faults, dikes, veins, and layers of soft rock". VERIFIED, NPS, https://www.nps.gov/subjects/caves/sea-or-littoral-caves.htm
- **Moore 1954 conditions** (as quoted in Waterstrat et al. 2010, JCKS 72(2):61–74, DOI 10.4311/jcks2009es0086; VERIFIED in Waterstrat):
  - a cliff in direct wave contact;
  - structures allowing differential erosion;
  - rock resistant enough to "prevent rapid formation of a protective beach".
  So no sea caves form behind a beach.
- **Shape** (VERIFIED, Waterstrat):
  - Entrance width ÷ maximum width = 0.933 for sea caves (vs 0.524 for flank-margin caves): "the entrance is usually the widest point".
  - Short ÷ long axis 0.701 on San Salvador. Fault-controlled caves (Santa Cruz Island) are elongate, with long/short = 2.88.
- **Example.** Painted Cave (Santa Cruz Island) is "nearly a quarter mile long and 100 feet wide, with an entrance ceiling of 160 feet". VERIFIED, https://www.nps.gov/chis/planyourvisit/santa-cruz-island.htm
- **Tidal notches.** Modern notch height 81 ± 13 cm, ancient 65 ± 19 cm. Notch height is about "0.3–3.2 times the tidal range" (citing Antonioli et al. 2015). VERIFIED, Lorscheid et al. 2017, Sci. Rep. 7:16241.
- **Blowholes and raised caves.** Blowholes form as sea caves grow landward and up into vertical shafts; raised wave-cut caves exist more than 100 ft above present sea level in Norway. REPORTED (Wikipedia).
- **Not found:** a typical length distribution (Trenhaile not read), blowhole dimensions.
- **Generator rules.**
  - Mouth at sea level (notch band about the tidal range); widest at the mouth, tapering inland.
  - Axis follows the local fracture or fault direction, or soft-rock strata.
  - Only where the cliff is in wave contact (high wave exposure, no beach) and rock hardness is above a minimum.
  - Optional blowhole shaft at the inland end; optional raised caves at old sea levels.

#### A5. Flooded and underwater caves

- **Cenotes.** In the Yucatán, "unsupported cave ceilings immediate to the surface collapsed creating new karst windows" when the sea was lower. Sea level rose "from 125 m below its present level" from about 18 ka. VERIFIED, QRSS, https://web.archive.org/web/2023id_/https://qrss.caves.org/geo.htm
- **Yucatán lengths and depths** (VERIFIED, QRSS lists, January 2025, https://web.archive.org/web/2025id_/https://qrss.caves.org/qrlong.htm):
  - Ox Bel Ha 496,804 m; Sac Actun 379,569 m, depth −119.2 m, 229 cenotes.
  - "The average depth for most caves in Quintana Roo is -15 meters."
  - Cenotes are "some tens of meters in diameter", there are about 10,000 on the peninsula, and voids reach 50–100 m below the modern water table (REPORTED, Wikipedia).
- **Sea-level change.** "121 ± 5 metres below present level during the last glacial maximum" (Fairbanks 1989, Nature 342:637, DOI 10.1038/342637a0). "a slow fall to −134 m from 29 to 21 ka BP" (Lambeck et al. 2014, PNAS, DOI 10.1073/pnas.1411762111). VERIFIED. Coastal karst formed at lowstand is now drowned, which is how submerged passages arise.
- **Flank-margin caves** (Mylroie & Carew 1990, ESPL 15(5), DOI 10.1002/esp.3290150505; VERIFIED abstract):
  - They form by "mixing of discharging freshwater with tide-pulsed incoming marine water under the flanks" of coastal ridges, as "phreatic chambers with complex interconnections and blind tubes".
  - Bahamian examples sit "between two and seven metres above current sea level", matching the last-interglacial highstand.
  - Sizes: "initially small ovid chambers … about 100 m², then interconnect … At about 1000 m² … laterally extensive, vertically restricted" (Roth 2004 thesis abstract, VERIFIED, https://scholarsjunction.msstate.edu/td/2795).
- **Blue holes.** Four origins: drowned sinkholes, dissolution along a rising halocline, upward collapse from deep voids, and bank-margin fracture. REPORTED (Mylroie, Carew & Moore 1995, DOI 10.1007/BF03175407). Dean's Blue Hole is about 202 m deep with a 25–35 m opening widening to about 100 m. REPORTED (primary source not reached).
- **Anchialine caves.** "a tidally-influenced subterranean estuary located within crevicular and cavernous karst and volcanic terrains that extends inland to the limit of seawater penetration". VERIFIED, Bishop et al. 2015, J. Crust. Biol., DOI 10.1163/1937240X-00002335.
- **Generator rules.**
  - Flooded = any void below the local water level (water table, lake, river or sea surface).
  - Coastal karst: put a horizontal "paleo-water-table" level at sea level − (0…125) m to create drowned passages. Add flank-margin chamber clusters just inland of the coast at the (paleo) sea level: oval, wide and low, many blind pockets.
  - Cenote: a collapse shaft tens of metres wide from the surface into a flooded passage.
  - Resurgences open under lakes, rivers and the sea.

#### A6. Caves under hills and mountains in non-soluble rock

- **Talus caves.** In "granites or gneiss … large, angular boulders"; "some caves have been surveyed for more than a kilometer"; Yosemite passages run "upwards of a 100 meters". VERIFIED, NPS, https://www.nps.gov/subjects/caves/talus-caves.htm
- **Crevice / landslide (tectonic) caves.** More than 600 in the Polish Flysch Carpathians; Jaskinia Miecharska about 1,800 m long and 55 m deep. REPORTED (Margielewski & Urban 2003, DOI 10.1016/S0169-555X(02)00375-6).
- **Gypsum mazes.** Optymistychna is "more than 200 km" long. Mazes are "two- to four-storey … up to 0.8 km²", with a passage density of 118–321 km/km² (average 198) and 1.6–8.0 m³ of void per metre of passage (average 3.84). VERIFIED, Klimchouk 1996, Int. J. Speleol. 25(3–4), DOI 10.5038/1827-806x.25.3.20.
- **Generator rules.**
  - On hard non-soluble rock, the "mountain cave" should be a narrow, tall fissure along a fracture or the slope-parallel crack behind a cliff (crevice cave), or a boulder-pile void at a talus slope foot. It should not be a long branchwork.
  - Gypsum gets the karst engine with maze presets: dense, multi-storey, 3–4 m³/m.

#### A7. Man-made: mines, tunnels, catacombs

**Mines**
- **Drainage gradient.** "The rule in many ore-mines is to drive the levels as flat as possible, with only just slope enough to make the water flow away … 1 in 216 to 1 in 288 is common." VERIFIED, Le Neve Foster, *A Text-book of Ore and Stone Mining* (1894), p.355, https://archive.org/details/cu31924004106203. The 1910 *Elements of Mining and Quarrying* (p.158) gives "1 in 288 to 1 in 144". Long drainage adits were much flatter: the Mansfeld adit had "a rise of 1 in 7200 … 9 feet 10 inches high (3 m.), and 6 feet across"; Gardanne "1 in 2000". VERIFIED, https://archive.org/details/elementsofmining00fostuoft.
  - **The owner's ~1:200 is close but slightly steep. Use 1:144–1:288 for levels and drifts (default 1:250), and 1:2000+ for long drainage adits.**
- **Drift sizes.** "smallest size … about 6-1/2 feet by 6 feet inside the timbers" (about 2.0 × 1.8 m); double track "at least 8 feet wide by 6-1/2 feet to 7 feet high". VERIFIED, Hoover, *Principles of Mining* (1909), https://www.gutenberg.org/ebooks/26697. Modern declines average 5 × 5 m (REPORTED). Modern ramps "typically … between 15%-17%" (VERIFIED, Sandvik technical note 2020, https://www.infrastructure-ni.gov.uk/sites/default/files/2025-01/consolidated-es-appendices-app-e7-sand-vik-equipment-technical-note.PDF).
- **Shafts** (VERIFIED, Hoover):
  - Rectangular: 3 compartments "fourteen to sixteen feet long by six to seven feet wide" (about 4.5 × 2 m); 4 compartments 21–22 × 7–8 ft; 5 compartments 26–28 × 8 ft.
  - Levels "100 feet" apart historically, later "200 or 220 feet".
  - Broken Hill stopes "each fifty feet wide".

**Road and rail tunnels**
- **FHWA-NHI-10-034 (2009)** (VERIFIED, https://www.fhwa.dot.gov/bridge/tunnel/pubs/nhi09010/tunnel_manual.pdf):
  - Grades "preferably should not exceed 4%; although grades up to 6% have been used".
  - Shapes: circular, rectangular, horseshoe.
  - Vertical clearance 16 ft (4.9 m). Two-lane curb-to-curb at least 24 ft (7.2 m) and wall-to-wall at least 30 ft (9 m); desirable 39 and 44 ft.
  - Portal "located at a point where the depth of the tunnel is suitably covered … In mountain tunnels the portal can be as close to the face of the mountain as practically constructible."
- **Portal cover.** "Tunnels usually require a minimum of one to two tunnel diameters of cover before tunneling can safely commence." VERIFIED, USACE EM 1110-2-2901 (1997) §5-6a, read from a mirror: https://asktop.net/wp/download/26/EM%201110-2-2901%20Tunnels%20and%20Shafts%20in%20Rock.pdf
- **Rail grades.**
  - EU TSI INF (Reg. 1299/2014 §4.2.3.3): up to 35 mm/m on new passenger-only P1 lines, with a 10 km average of at most 25 mm/m and no continuous 35 mm/m stretch over 6 km. VERIFIED.
  - Mixed-traffic standard 12.5 ‰. REPORTED.

**Catacombs**
- **Roman.** Galleries "averaged about two to three meters in height, and about one meter in width". The first level is "three to eight meters below the surface", with "as many as four levels reaching a depth of twenty to twenty-five meters" (Callisto and Giordani have 5). Loculi are stacked "four to eight" high. REPORTED, International Catacomb Society, https://catacombsociety.org/the-structures-of-the-catacombs/
- **Paris.**
  - The ossuary is "twenty meters underground in a maze of galleries" (VERIFIED, official site, archived).
  - The Calcaire Grossier quarries undermine "770 ha" of Paris, and the IGC inspects "200 km de galerie" (VERIFIED, https://cdn.paris.fr/paris/2021/04/07/a44836dfa06f3ed77a9e91efa826e409.pdf).
  - Inspection galleries total "près de 280 km" and were built under the streets ("sous les voies publiques"), so **the layout follows the street plan rather than a regular grid** (VERIFIED, Thomas 2009, Revue XYZ).
  - Gallery dimensions: not found.
- **Generator rules.**
  - Roman-style: 1 m × 2–3 m `Arch` galleries in a dense, roughly orthogonal-but-irregular net, 1–5 levels from 3–8 m down to 20–25 m, with stair or shaft entrances.
  - Paris-style: galleries under the road network (reuse `vector.rs` roads) at about 20 m depth.

---

### Part B. Algorithms

#### B1. Paris, Guérin, Peytavie, Collon, Galin 2021: "Synthesizing Geologically Coherent Cave Networks"

Computer Graphics Forum 40(7):277–287 (Pacific Graphics 2021), DOI 10.1111/cgf.14420. The author list is Paris, Guérin, Peytavie, **Collon**, Galin; James Gain is not an author. I read the full HAL v2 text (Wayback copy linked above). Code: https://github.com/aparis69/Karst-Synthesis, **MIT**, C++, last push 2022-07-07 (VERIFIED via GitHub API). Everything in this section is VERIFIED unless marked.

**Inputs.** A terrain H; the active water table W; inception horizons L (modelled as elevations in monoclinal settings); a permeability field π: R³→[0,1] (1 = least permeable), built as a construction tree of sphere and strata primitives; fracture orientations F (plane normals n_k with weights w_k); and key points P labelled sink (inlet), spring (outlet) or waypoint, with optional forced links.

**Step 1: gridless anisotropic shortest paths.**
1. Sample the bedrock adaptively. Start with the key points. Add Poisson-sphere samples with a *small* radius on each horizon elevation, on the water table, and where π is high. Fill the rest with a *larger* Poisson radius.
2. Connect each sample to its **n = 40** nearest neighbours. This gives enough angular resolution and avoids the "stair-step conduits due to aliasing" of grid methods.
3. The edge cost is a weighted sum of terms:
   - horizons: c_L(p) = g(|p_z − L_z| / r), with g(x) = 3x² − 2x³ on [0,1], so the cost is low near a horizon;
   - fractures: c_F(p, ṗ) = Σ_k w_k (1 − (n_k · ṗ̂)²), so moving parallel to a fracture plane is cheap;
   - permeability: c_π(p) = π(p);
   - user-forced edges get a small cost.
4. Solve with **A\***.

**Step 2: network.**
- Build the complete graph of paths between all key-point pairs, with distance d(a,b) = cost of the shortest path.
- Prune it to a **γ-skeleton** (a proximity graph): keep edge (a,b) only if no key point k has d(a,k)^γ + d(k,b)^γ < d(a,b)^γ. Lower γ gives denser networks. γ = 2 gives the Gabriel graph and γ → ∞ the relative-neighbour graph under this metric.
- **Amplification.** Scatter extra key points in the bedrock and connect them with the same path search. *Interior* points push towards mazes; *dead-end* points give branchworks.
- **Tortuosity.** Stochastic midpoint displacement with factor θ, from "a few centimeters (θ = 0) to 4 meters (θ = 1)".
- **Presets.** "Rectilinear and anastomotic mazes are created by setting γ = 0.8 and tuning orientation distributions (three axis-aligned directions for rectilinear, and 8 directions on the sphere for the looping cave)". An elongated branchwork comes from a single sink and a single spring. A superimposed (multi-level) network comes from two inception horizons and 3 amplification steps.

**Step 3: classification and size.**
- Edges are labelled keyhole, passage, epikarst, tube, canyon, bed or chamber from their **distance to the water table** and their **verticality** (Fig. 11). Vertical edges become shafts. Horizontal vadose edges become canyon or keyhole; phreatic edges become tube or bed (along soluble beds). Epikarst runs from the surface to the vadose zone.
- **Breakout chambers** appear where a tunnel crosses a high-permeability "ghost rock" zone (c_π ≈ 0).
- Radius is random in 0.5–4 m; epikarst is fixed at 0.5 m.
- The authors call this classification "simple" because "current knowledge in geomorphology does not allow the identification of precise criteria".

**Step 4: geometry (signed distance construction tree).**
- The surface is an f: R³→R that is 1-Lipschitz (a distance bound). Terrain: f_H(p) = (p_z − h(p_xy)) / sqrt(1 + λ²), where λ is the maximum terrain slope; the appendix proves this is 1-Lipschitz.
- **Straight segments** use sweep primitives: a closed piecewise-quadratic cross-section C swept along a segment ab. This is compact, under 1 kB for a 10 m tunnel. Section shapes: tube, canyon, keyhole, bed (Fig. 16).
- **Tortuous paths** use a union of Poisson spheres (R ≈ 20 cm) kept where they lie within R/2 of interpolated cross-sections, because "defining a continuous signed distance function for a sweep object … along a curve Γ is not possible in the general case". This costs about 100× more memory than a sweep.
- The final terrain is the difference H − K.
- Speleothems are **not** generated: "Small scale details such as stalactites, stalagmites, or wall irregularities could be added at the expense of a more complex construction tree."

**Performance.**
- Graph plus γ-skeleton: under 1 s. The nearest-neighbour graph takes up to 3 s.
- Examples: 0.2–1.1 km extents, 2.3–22.1 km of skeleton, 25k–92k samples, 17–47 key points, 13k–33k spheres, 2.9–7.4 MB trees.
- Polygonizing 1 × 1 × 0.4 km³ at 0.5 m "takes up to two minutes" for 30,000 nodes, with Wyvill-style polygonization.

**Validation.** All four synthetic networks fall inside the real ranges for SPL and CPD. The maze presets have *k* slightly too high, because real caves rarely have junctions of degree 4 or more.

**For Jenga Dunia.** This is the right **karst** generator: inputs map onto layers we already have (C1). Copy its *method*, not its tree evaluator. Evaluating thousands of SDF nodes per voxel is the stated bottleneck, and our kernel already uses binned primitives.

#### B2. Paris, Galin, Peytavie, Guérin, Gain 2019: "Terrain Amplification with Implicit 3D Features"

ACM TOG 38(5), Article 147, DOI 10.1145/3342765. Full text read from https://people.cs.uct.ac.za/~Jgain/wp-content/papercite-data/pdf/paris2019.pdf. VERIFIED unless marked.

- **Geology tree.** Rock resistance ρ: R³→[0,1] is a construction tree. Leaves are turbulence strata ρ(p) = t(p_z/λ0), plane primitives ρ = g∘d + t(p/λ0), and sphere/curve primitives for local hard or soft rock. Inner nodes do blend (sum), warp (folds, f∘ω⁻¹) and faults (a discontinuous warp inside a domain).
- **Heightfield implicitization.** A prism primitive f_P (falloff R) is intersected with a surface primitive f_T = σ(h(p_xy) − p_z): f = min(f_T, f_P). σ is an odd cubic with σ(0)=0, σ'(0)=1, σ(R)=2T, σ'(R)=0, so the field stops depending on the surface beyond distance R. This lets caves be carved deep without the terrain surface leaking into the field.
- **Sculpting primitives.** Point, curve, sphere and similar primitives with falloff g(r) = (1 − (r/R)²)³. Caves are *negative* primitives. Noise goes into the radius ("star-shaped" anisotropic distance d̃ = ‖p − s(p)‖ / (R + t(v(p)))) rather than being added at the root, because root noise "introduces unwanted holes and disconnected surface components".
- **Cave generation by modified invasion percolation.** Seed a queue with the terrain's **pits/sinks** (cells whose neighbours are all higher; users can add resurgences). Repeatedly pop the least-resistant candidate, with resistance perturbed by ε ≈ 0.1. Carve a negative sphere only if ρ < ρ0. Push new candidates sampled on the **lower hemisphere**, because water flows down, rejecting any whose Poisson sphere overlaps an existing candidate. The 5.2 × 5.2 km karst example used 165,773 sphere primitives, with tunnels covering 2 % of the area. The 2021 paper later criticised this: tunnels "do not exhibit realistic karstic shapes" and show "uniform circular cross-sections".
- **Sea caves and arches** come from seeding negative spheres on the surface with energy e(ρ, σ(p_z)), where σ is a sea-erosion stress that depends on elevation relative to sea level.
- **Polygonization.** Marching cubes restricted to per-column elevation bounds [a_ij, b_ij] from the tree, dilated over the one-ring for C0 continuity. Where only the heightfield applies (n(p) = 1), vertices come directly from h(p_ij). Speed-ups were up to 12×, and 1.5–2.5× from direct elevation. 3D features covered only 1–35 % of the area (Table 1). Example: karst scene 1520² × 447 grid, 14.2 s vs 188.4 s.

**For Jenga Dunia.** Two ideas carry over directly. (1) **Mesh only near the caves**; keep the heightfield as the terrain everywhere else. (2) Put **noise in the distance, not at the root**. Invasion percolation is a useful cheap "hill cave / unknown network" fallback, but it is not geologically shaped.

#### B3. Peytavie, Galin, Grosjean, Mérillou 2009: "Arches"

Computer Graphics Forum 28(2):457–467, DOI 10.1111/j.1467-8659.2009.01385.x (Crossref VERIFIED). A hybrid model: per-column stacks of material layers (vertical run-length encoded), with an implicit surface reconstructed by convolution, plus rock-pile generation. REPORTED (abstract via search; the 2021 paper's description). Not recommended for us. Paris 2019 Table 1 reports Arches using more memory than the implicit tree (e.g. 300 MB vs 3.0 MB for the Sea scene, VERIFIED). Our strata and hardness layers already give the per-column material information we would want from it.

#### B4. Speleothem generation in graphics

- **Tortelli & Walter 2009** (GRAPP, DOI 10.5220/0001787200270035, VERIFIED full text, https://www.scitepress.org/Papers/2009/17872/17872.pdf). GPU geometry-shader growth: stacked rings, with each new vertex at v' = v + height + r·N. Radius and height are driven by a 3-level rain index (none / stalactite only / dripping with stalagmite) and the drip interval, approximating Kaufmann 2003.
- **Amorós-Tur & García-Fernández 2024** (CEIG, CC BY, VERIFIED). Integrates Short's normal-velocity law in time on a discretised ceiling profile. Stalactites appear spontaneously on protuberances and converge to the ideal shape. 2D only.
- **Cui, Chow & Zhang 2011**, "Procedural generation of 3D cave models with stalactites and stalagmites". Voxel octree, noise and crack patching. REPORTED (University of Wollongong record https://ro.uow.edu.au/infopapers/3625); I could not confirm the venue.
- **Mark, Berechet, Mahlmann & Togelius 2015** (FDG), "Procedural Generation of 3D Caves for Games on the GPU": an L-system for passages, then noise-perturbed metaball carving and isosurface extraction. REPORTED (PhilArchive abstract, https://dc2.philarchive.org/rec/MARPGO). Paris 2021 says it adds stalactites but is limited by voxel resolution.
- **Takeaway.** Every graphics paper places speleothems as **separate meshes** (rings, lathe profiles) on the ceiling or floor; none voxelises them. This matches the kernel's choice. A stalagmite only centimetres across is far below any useful voxel size.

#### B5. Game techniques

**Minecraft 1.18 noise caves** (VERIFIED from the vanilla worldgen data, https://github.com/misode/mcmeta, `data` branch, `worldgen/noise_settings/overworld.json` and `density_function/overworld/**`):
- **Density.** `final_density` is positive for solid. Caves are combined with **min()** (carving), and the noodle function is min-ed in after interpolation. Cave density is sampled on 4 × 8 × 4 cells and interpolated linearly (`upscale` cell sizes 4/8/4).
- **Near-surface gate.** When `sloped_cheese` (terrain depth) is below 1.5625, i.e. near the surface, only `min(sloped_cheese, 5·entrances)` applies. Cheese, spaghetti-2D and noodle caves can therefore open to the surface **only through the entrance function**. Deeper down, the cave terms apply. This "near the surface, only dedicated entrances may cut" rule is worth copying.
- **Cheese.** A thresholded 3D noise (`cave_cheese` with y-scale 2/3, + 0.27), plus a squared `cave_layer` noise term with y-scale 8 that makes layered voids. Pillars come from a separate `pillars` function that is max-ed back in.
- **Spaghetti (3D, in `entrances`).** `max(|n1|, |n2|)` of two noises plus a thickness and roughness term. A tube exists where *two* independent noise fields are both near zero, so the intersection of two zero-isosurfaces forms a curve. A rarity noise chooses the scale (`interval_select`).
- **Noodle.** The same two-ridge idea with `noodle_ridge_a/b` at xz/y scale 2.67, thickness −0.075 − 0.025·n, limited to Y −60…320.
- **Aquifers.** 16 × 12 × 16 cells; fluid level 40·floor(Y/40) + 20 + spread; lava below Y = −10 when |noise| > 0.3; stone barriers up to about 5 blocks thick as lids. REPORTED (https://minecraft.wiki/w/Cave; I did not open the aquifer Java code).
- **Carver caves (pre-1.18 "worms").** These still exist; main rooms are 1–14 blocks tall and 5–15 wide. REPORTED (Minecraft Wiki).

**Perlin worms.** Each worm has its own noise module. A lookup segment is sampled once per body segment, and each sample's noise sets that segment's heading; segments chain off the previous one. REPORTED (libnoise example page, http://libnoise.sourceforge.net/examples/worms/index.html; a demo, not a paper). In 3D: march a point, steer yaw and pitch with low-frequency noise, and carve spheres. It is cheap, but it ignores geology.

**Cellular automata** (Johnson, Yannakakis & Togelius 2010, FDG; VERIFIED, https://www.um.edu.mt/library/oar/bitstream/123456789/22895/1/Cellular_automata_for_real-time_generation_of.pdf). Settings: 50 × 50 grid, r = 50 % random rock, Moore neighbourhood M = 1, rule "rock if neighbourhood value ≥ T = 5", n = 4 iterations; cost n·w·h·(2M+1)²; 0.41 ms per map. It is 2D and gives organic blobs with no passages-with-purpose. Useful only to roughen a chamber's outline or for breakdown floors.

**Verdict for Jenga Dunia.** Use noise-cave tricks only as *detail* (wall roughness, a "cheese" chamber field inside a chamber's bound) or as an explicit "Fantasy / game caves" preset. The two-zero-crossings spaghetti trick is a neat, cheap way to add secondary tunnels inside a bounded volume. None of these place caves sensibly relative to sinkholes, springs or the water table, which the owner explicitly wants.

#### B6. Meshing a signed distance or voxel volume

**Algorithms** (VERIFIED via a sub-agent reading the primary PDFs, Crossref and blog):

| Method | Source | Pros | Cons |
|---|---|---|---|
| Marching cubes | Lorensen & Cline 1987, DOI 10.1145/37402.37422 | simple, widely known | the 15-case table can give cracks and inconsistent topology; many thin triangles |
| MC33 (Lewiner et al.) | J. Graphics Tools 8(2) 2003, DOI 10.1080/10867651.2003.10487582 | "guaranteed to be a manifold surface, with no crack" (730 subcases) | larger table, more tests |
| Surface nets | Gibson 1998, MICCAI, DOI 10.1007/BFb0056277 | one vertex per cell, so about 4× fewer vertices than MC (Lysenko: MC 15268 vs SN 3816 vertices on a test terrain; 274.6 vs 124.3 ms on a 65³ volume); smooth; trivially parallel | can create non-manifold vertices; rounds sharp edges |
| Dual contouring | Ju, Losasso, Schaefer & Warren 2002, DOI 10.1145/566654.566586 | keeps sharp features with Hermite data (QEF); octree simplification; "no crack patching" | needs exact normals; can be non-manifold without the extra test |
| Transvoxel | Lengyel 2010 thesis, https://transvoxel.org/ | crack-free seams between LOD levels | MC-based, so MC's ambiguity remains |
| Sparse storage (VDB) | Museth 2013, ACM TOG 32(3), DOI 10.1145/2487228.2487235 | "virtually infinite" index space, O(1) average access | complex; no Rust writer |

Lysenko's comparison: https://0fps.net/2012/07/12/smooth-voxel-terrain-part-2/ (he calls it "apples-to-oranges" because it compares quads with triangles).

**Rust crates** (VERIFIED from the crates.io API and GitHub on 2026-10-25):

| Crate | Version | Licence | Notes |
|---|---|---|---|
| `fast-surface-nets` | 0.2.1 (2025-01-03) | MIT OR Apache-2.0 | Naive surface nets; negative = inside; chunks need a 1-voxel pad (18³ for 16³); about 20 M tris/s on one core (README claim) |
| `ndshape` | 0.3.0 (2022) | MIT OR Apache-2.0 | index linearisation for the above |
| `transvoxel` | 2.0.0 (2025-11-11) | MIT OR Apache-2.0 | LOD seams; "experimental" badge |
| `isosurface` | 0.0.4 (2021) | Apache-2.0 | MC, extended MC, DC, octree MC; small-index warning |
| `mcubes` | 0.1.7 (2025) | MIT | MC, tiny user base |
| `marching-cubes` | 0.1.2 | MIT | repo **archived** |
| `fidget` | 0.5.1 (2026-09-27) | **MPL-2.0** | JIT implicit evaluation + manifold dual contouring; very active |
| `saft` (Embark) | 0.34.1 (2024) | MIT OR Apache-2.0 | SDF compiler + MC mesher; GitHub repo gone (404) |
| `tessellation` | 0.11.0 (2026) | Apache-2.0 OR MIT | manifold dual contouring |
| `vdb-rs` | 0.6.0 | MIT | **reads** VDB only |
| `building-blocks` | 0.7.1 | MIT | **archived**; split into fast-surface-nets / ndshape / block-mesh |

**Recommendation.** The kernel already hand-rolls surface nets over a sparse grid restricted to the skeleton's bounds. Keep it; there is no need for a dependency. Surface nets is about 100 lines. If a crate is preferred, `fast-surface-nets` (MIT/Apache) is the maintained choice. For **sparse storage**, a `HashMap<IVec3, Chunk16>` of padded 18³ f32 blocks, allocated only where |sdf| < chunk diagonal, is enough. VDB is overkill. Add Transvoxel-style LOD only if caves larger than a few km at sub-metre voxels are needed. `deny.toml` already allows MPL-2.0 (checked), so `fidget` is licence-compatible if JIT evaluation of large SDF trees is ever needed.

#### B7. Joining a cave mesh to a heightfield terrain

**Engines** (VERIFIED from official docs via a sub-agent):

- **Unity.**
  - `TerrainData.SetHoles(xBase, yBase, bool[,])` takes "true for surface and false for hole", indexed [y,x]; https://docs.unity3d.com/ScriptReference/TerrainData.SetHoles.html.
  - The hole mask resolution is "the resolution of the Terrain's heightmap minus 1" (1025² heightmap → 1024² holes), i.e. **one flag per quad**, not per height sample; https://docs.unity3d.com/Manual/terrain-PaintHoles.html.
  - "A hole in the terrain is automatically excluded from the Terrain Collider."
  - Holes are "texture-based, so it can create aliased edges around holes". Unity's advice is to hide the edges with rock meshes and to "use other Editor tools, such as ProBuilder, to create the cave or overhang geometry".
  - URP needs Terrain Holes enabled in the pipeline asset or holes vanish in builds.
  - For interactive edits use `SetHolesDelayLOD` + `SyncTexture`.
- **Unreal Engine 5.**
  - The Landscape **Visibility** tool masks areas "for areas such as tunnels or caves … that seamlessly transition to non-landscape static mesh sections". It is binary, with "no way to have a transitional gradient".
  - The material must connect **Landscape Visibility Mask** to Opacity Mask (Masked, or Opaque with the mask connected so that only components with holes switch); https://dev.epicgames.com/documentation/unreal-engine/landscape-visibility-tool-in-unreal-engine
  - Nanite landscapes work with holes. Nanite static meshes support Opaque and Masked materials and "should generally be enabled wherever possible", so a large cave mesh can be Nanite.
  - Collision follows the holes per forum reports (REPORTED). World Partition with holes: UNVERIFIED.
- **Blender.**
  - Boolean: "Only Manifold meshes are guaranteed to give proper results"; the Manifold solver "will not work at all on non-manifold meshes"; https://docs.blender.org/manual/en/latest/modeling/modifiers/generate/booleans.html.
  - An open terrain sheet is non-manifold. So **don't boolean**: import terrain (already cut) and cave as separate objects, and weld the seam with Merge by Distance or the Weld modifier only if a single mesh is needed.
- **Formats.**
  - glTF has no hole concept: export the terrain mesh with the hole triangles removed, plus the cave mesh.
  - USD has `holeIndices` ("traditionally a feature of subdivision surfaces"); remove faces instead, for portability.
  - FBX: no documented hole feature found (UNVERIFIED).

**Seam strategy (my synthesis; UNVERIFIED as a cited practice, but it follows from the facts above).**
1. **The hole mask is authoritative and per quad.** A terrain quad is a hole if the cave SDF at the ground is below −lip at the quad centre (or at a majority of its 4 corners). This matches Unity's heightmap−1 resolution. For Unreal, the visibility weightmap's exact sampling (per vertex or per quad) was not confirmed in the docs I read, so verify it in the headless Unreal check. Note that the kernel's `entrance_holes` currently samples at cell centres of the *sample* grid; exporters must resample it to quads, which is a one-cell offset.
2. **The cave mesh overlaps the terrain rim (a skirt).** Keep cave triangles up to about 1–2 terrain cells above the ground surface around each hole instead of clipping exactly at the surface. The aliased, quad-stepped hole edge then always falls over cave wall, never over sky. This is what Unity's "hide edges with rock meshes" advice achieves, and it removes the need for matching vertices.
3. **Do not stitch vertices across meshes.** Exact vertex matching between a heightfield LOD (Unity and Unreal both re-LOD the terrain) and a cave mesh is not stable. Overlap plus a hole is how all three engines expect caves to be built.
4. **Blender and glTF/FBX/USD.** Export the terrain with the hole quads deleted, plus the cave mesh with the skirt. Optionally export a decorative rock "rim ring" mesh.

---

### Part C. Recommended design for the Rust implementation

This builds on what `crates/jd-terrain/src/caves.rs` already does: a skeleton (passages with Tube/Keyhole/Arch/Rect profiles and per-point half-sizes, flat-floored ellipsoid chambers, typed entrances, speleothems, a water level and roughness) → a binned SDF with seeded noise → surface nets on a sparse grid → a mesh clipped to the terrain → an entrance hole mask, with separate speleothem meshes. That architecture matches the literature (Paris 2021 skeleton→SDF; graphics papers' separate speleothem meshes), so the recommendations are about **generators, inputs and tests**, plus a few changes.

#### C1. Representation (keep, with four changes)

1. **Skeleton graph, not loose polylines.** Store nodes (id, position, kind ∈ {sink, spring, waypoint, junction, chamber, portal}) and edges (polyline, profile, per-point size). Generate `Passage` polylines from the edges. Benefits: entrance editing moves a node and re-routes only its edges; network metrics (B1/A1 table) become computable; flood and drainage logic can walk the graph.
2. **Size along a passage as a correlated log-normal series** (Frantz 2021): log r ~ GP(mean log 1.2 m, exponential correlation 50–200 m). W/H around 1 for tubes, < 1 for canyons. No downstream growth.
3. **Noise in the radius, not added to the field** (Paris 2019). Keep `roughness_m` small relative to the half-size so passages never pinch shut or spawn floating blobs. Test: the mesh has exactly one connected component per connected skeleton component.
4. **A Lipschitz-bounded SDF** (Paris 2021): keep primitives as true distances or distance bounds, so a sparse mesher can skip a chunk when |sdf(centre)| > chunk half-diagonal.

#### C2. Per-type generators and their inputs from existing layers

| Generator | Inputs from Jenga Dunia | Rules (sources in Part A/B) |
|---|---|---|
| **Karst (solution)** | `karst.rs` sinkholes → **inlets**; `flow.rs` springs / river and lake beds at the lowest outlet → **springs**; `strata.rs` hardness → permeability π (soft = permeable) and **inception horizons** (a few chosen bedding contacts, not all: <10 % of partings carry >70 % of conduits); water table W = elevation of the spring(s) (base level), optionally a lowered paleo-level for upper fossil levels; a global fracture set (1–3 normals) | Paris 2021: adaptive Poisson samples (dense on horizons, W, soft rock), kNN k = 40, A\* with c_L + c_F + c_π, γ-skeleton (γ = 2 branchwork from one sink to one spring; γ = 0.8 + fracture sets for mazes), amplification, midpoint tortuosity ≤ 4 m. Classify by distance to W and verticality: vadose vertical → shaft; vadose horizontal → canyon; just above W after a drop → keyhole; at or below W → tube (flooded below `water_level_m`). Chambers at junctions inside soft zones. Epikarst 0.5 m feeders from sinkholes. |
| **Hill / mountain cave** | heightfield + hardness; a cliff or valley-side portal (high slope, low elevation relative to local relief) | Same karst engine with one inlet on the plateau and an outlet portal at a cliff foot or valley side. On non-soluble rock (no karst layer), fall back to a short fissure/tectonic cave or Paris 2019 invasion percolation from pits. |
| **Sea cave** | sea-level node elevation; coastal erosion wave-exposure map; hardness | A notch at sea level into the cliff where wave exposure is high *and* rock is locally weak (Paris 2019 seeded spheres by e(ρ, σ(p_z))). Short, roughly horizontal, cross-section widest near sea level, often ending in a chamber or blowhole (dimensions: see A4). Raised sea caves at old sea levels if the user sets uplift. |
| **Flooded / underwater** | water table, lake and river beds, sea level | Any passage segment below `water_level_m` (or below the sea or lake surface) is flagged flooded. Cenote = a collapse chamber that breaks the surface over a flooded tube (entrance kind `Sinkhole`, `underwater=true` below the waterline). Flank-margin caves: chambers along the fresh/salt mixing zone just inland of the coast (see A5). Resurgence entrances under lakes, rivers and the sea. |
| **Lava tube** | `lava` MAGFLOW flow paths and thickness | Tube centreline = the flow's main channel axis (the longest high-flux path), at a depth of a fraction of the flow thickness; elliptical or flat-floored tube; skylights where the roof is thinnest (see A3). |
| **Mine** | user portal(s) on a hillside; strata (an ore bed) | Adit driven *up* into the hill at 1:144–1:288 (default 1:250; a long drainage adit 1:2000) so water drains out of the portal; drifts 2.0 × 1.8 m (historic) or 5 × 5 m (modern) along the ore bed; levels 30–67 m (100–220 ft) apart; rectangular shafts about 4.5 × 2 m to the surface; stopes as box chambers; modern ramps 15–17 %; `Rect`/`Arch` profiles, `roughness_m` small. |
| **Road / rail tunnel** | two portals, or a spline road crossing high ground (`vector.rs` roads) | Constant-grade bore: road ≤ 4 % (hard cap 6 %), rail ≤ 12.5 ‰ mixed or ≤ 35 ‰ passenger-only; `Arch`/horseshoe profile, 9 m wall-to-wall and 4.9 m clearance for two lanes; portal where cover first reaches 1–2 tunnel diameters; Unity/Unreal holes at both portals. |
| **Catacombs** | a user area, depth; optionally the road network | Roman: about 1 m × 2–3 m `Arch` galleries, irregular-orthogonal net, 1–5 levels from 3–8 m down to 20–25 m. Paris: galleries following the streets at about 20 m. Stair or shaft entrances; no speleothems; `roughness_m` ≈ 0. |

**Placement on the map (making it "sensible").** Every generator must take its anchors from existing layers rather than random positions: karst inlets from sinkholes, outlets from springs; sea caves from high wave exposure at sea level; lava tubes from MAGFLOW paths; tunnels from roads. User clicks add or override nodes with the same labels (sink, spring, waypoint, portal). Then re-run only the path search for affected edges.

#### C3. Speleothem placement (refine the existing `place_speleothems`)

- **Where.** Air-filled (above `water_level_m`) ceilings of *vadose or abandoned* passages and chambers. Weight drip-site density by an infiltration proxy: soil/vegetation cover on the surface above × (1 − aridity) × fracture intensity, scaled by the ceiling's depth below the surface (thin roof = more drips). Kaufmann: bare rock gives low CO2 and no growth. None in active phreatic tubes, mines, catacombs or tunnels (old mines could get small soda straws; a preset flag).
- **Sizes.**
  - Stalagmite diameter is set by drip interval: about 4 cm minimum for slow drips (> 1000 s), 20 cm+ for 10–100 s drips, metre-scale only for fast drips in cool caves.
  - Height = rate × age, with rate 0.01–0.25 mm/yr (stalagmites) and about 0.1 mm/yr (stalactites), capped by the ceiling gap.
  - Soda straws 4–5 mm (meshed only at close range or as instanced cards).
- **Pairing.** Each stalagmite sits under a stalactite. Climate sets the ratio: wet gives a big stalagmite and small stalactite; dry gives the reverse. If length(stalactite) + height(stalagmite) ≥ gap, make a **column**.
- **Shape.**
  - Stalagmite: lathe a profile from Kaufmann's law (integrate W_i = W0·exp(−s/R0) for N steps), giving a candle with a rounded top.
  - Stalactite: a cone/carrot matching Short's ideal (fitted lathe profile), with a soda-straw tip.
  - Noise-perturb the rings. Instancing-friendly: export a small library of meshes plus transforms (glTF `EXT_mesh_gpu_instancing`, Unity/Unreal instancing) rather than unique geometry.
- **Flowstone.** A wall decal or displaced sheet on walls below high drip density. There is no numeric source, so keep it cosmetic.

#### C4. Entrances and carving

- **Entrance kinds map to geology.** Sinkhole (karst inlet; reuse the doline shape from `karst.rs` as the funnel, and the passage starts at its throat); Portal (cliff-foot or valley-side outlet, adit, tunnel); Skylight (lava-tube roof collapse); Resurgence (spring, possibly underwater); SeaCave (sea-level notch); Shaft (vertical).
- **Carving.** The SDF union already opens the cave where a passage reaches the surface. Add a **flare** at each entrance (a larger ellipsoid or half-cone primitive) so mouths aren't tube-sized circles cut in a slope.
- **Holes.** Per-quad mask (B7 step 1), with a 1–2 cell skirt kept on the cave mesh (B7 step 2).
- **Editing.** Click-to-add an entrance = add a skeleton node with a kind; drag = move the node; re-route only incident edges (A\* on the cached sample graph). Paint = a brush mask that adds soft-rock (low π) primitives so routes are attracted there. Both are journaled graph-node params, as already planned.

#### C5. What can be tested numerically

1. **Cross-section fidelity.** For a straight passage of each profile, slice the mesh at mid-length. Measured width and height should be within one voxel of 2·half-size. Keyhole: the slot is narrower than the tube above it.
2. **Closed and manifold away from entrances.** Every edge has exactly 2 faces, except boundary edges, which occur only within the skirt band near entrance holes. Euler characteristic per component is as expected. One mesh component per connected skeleton component (no floating noise blobs).
3. **Holes only at entrances.** Every hole quad lies within (flare radius + 1 cell) of an entrance. Every entrance has ≥ 1 hole quad unless it is flagged underwater. Hole count is 0 for a cave with no surface-reaching node.
4. **Placement rules.** All sinks are on terrain pits or sinkholes; springs are at or below the lowest inlet; the karst water table equals the spring elevation; phreatic-labelled segments are ≤ W; vadose segments are > W; canyons are never below W; sea-cave mouths are within ±(tide/notch band) of sea level; lava tubes stay within the MAGFLOW footprint and below the flow surface.
5. **Hydraulic monotonicity.** Along every inlet→spring path, the vadose part never rises by more than the phreatic-loop allowance, and the path ends at a spring.
6. **Network statistics.** Compute Collon's metrics (k, r_k, SPL, CPD, l, H_O) in Rust on the generated graph. For the "branchwork" preset assert 1.8 ≤ k ≤ 2.6, −0.6 ≤ r_k ≤ −0.2 (or ≤ 0), 0 ≤ CPD ≤ 0.6, 2.3 ≤ SPL ≤ 55.7, and 8 ≤ l ≤ 331 m. Mazes may exceed k, as in Paris 2021. Karstnet (MIT, Python) can serve as an offline oracle.
7. **Passage size statistics.** Over a generated network: log(radius) is approximately normal, median within [0.5, 4] m (preset-dependent), and the lag-correlation at 50 m is > 0.5 while at 500 m it is ≈ 0.
8. **Speleothem physics.** Every stalagmite is directly below a stalactite (horizontal distance < radius) unless the stalactite is broken or absent by preset. Columns occur only where the two lengths sum to ≥ the gap. Nothing is below the water level. Stalagmite diameter is ≥ 4 cm (Kaufmann/Curl minimum). Height ≤ age × 0.25 mm/yr.
9. **Man-made constraints.** Adit grade within 1/288–1/144 and always rising inward (water exits at the portal); road tunnel grade ≤ 4 % by default (6 % hard cap); tunnel portal cover between 1 and 2 bore diameters; catacomb galleries within the 3–25 m depth band, about 1 m wide.
9b. **Lava and sea caves.** Lava tube inside the MAGFLOW footprint, depth < a few tens of metres, skylight diameter < tube width; sea-cave mouth within the notch band (about 0.3–3.2 × the tidal range around sea level), with the mouth the widest section (entrance/max width ≈ 0.9).
10. **Determinism.** Same params and seed give byte-identical meshes (blake3 of buffers, as elsewhere in the repo).
11. **Engine round-trip.** Unity: imported hole count equals the exported quad-mask count; the TerrainCollider raycast down through an entrance hits the cave floor, not the terrain. Unreal: the visibility layer weight is 0 exactly on the hole quads.

#### C6. What I could not source

- Ford & Williams and Palmer *Cave Geology* text (paywalled; not attributed).
- A numeric law for phreatic loop depth below the water table (only the qualitative four-state model and the ">3 km flow path favours deep flow" claim).
- Flowstone and rimstone dimensions.
- Lava-tube roof-thickness distribution; typical sea-cave length distribution and blowhole sizes; Paris inspection-gallery cross-section.
- World Partition behaviour with landscape holes; an FBX hole convention.

