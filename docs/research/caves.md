# Caves — research (tools, geology, methods)

Owner request (2026-10-25): generate subterranean caves of many kinds (karst with stalactites and stalagmites, caves under hills and in mountains, flooded caves under lakes, rivers and the sea, sea caves, lava tubes, mines, tunnels, catacombs), paint entrances, visualise, edit and place them well in the map; "do research on what World Creator offers and do more". Decision: [ADR-0016](../adr/0016-caves-as-an-implicit-volume-beside-the-heightfield.md).

Part 1 is the tools survey; part 2 (geology and generation methods) follows.

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
