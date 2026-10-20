# e-on VUE — feature study and what Jenga Dunia can take from it

Owner request (2026-10-20): "learn from Vue and incorporate features not present in other softwares". This note records what VUE (e-on software, later Bentley Systems) offers, with evidence, and maps it onto Jenga Dunia's backlog. Clean-room rule as everywhere (ADR-0010): only public descriptions were read — no VUE binaries, SDKs or file formats were inspected, and nothing here derives from them.

Legend: `VERIFIED` = read on an official e-on/Bentley page or a primary-source article quoting it; `REPORTED` = third-party review, tutorial or forum; `INFERRED` = our reading. All sources accessed 2026-10-20.

## Status of the product

- `VERIFIED` VUE 2023 and PlantFactory 2023 are "the last official releases published by e-on software"; Bentley offers "free perpetual access to VUE, PlantFactory, and PlantCatalog", no further development or hotfixes, Omniverse connectors removed ([Bentley e-on page](https://www.bentley.com/products/e-on-software)).
- `VERIFIED` The 2024 builds (VUE 2024 / PlantFactory 2024) shipped as free beta "technology previews" — Cycles rendering through a USD Hydra bridge, SmartClouds library, OpenVDB conversion of MetaClouds, experimental Apple-Silicon builds ([CG Channel, 2024-05-10](https://www.cgchannel.com/2024/05/check-out-the-new-features-in-vue-2024-and-plantfactory-2024/)).
- `INFERRED` A discontinued, closed-source tool with a 27-year feature set and no successor leaves its users without a maintained home; an open terrain tool that covers the terrain-side subset is the opportunity. The parts we should *not* chase: VUE's own renderer, plant modelling (PlantFactory), DCC plug-ins.

## Feature inventory (VUE 2023)

Source for this section unless noted: the feature list on [Wikipedia: E-on Vue](https://en.wikipedia.org/wiki/E-on_Vue) (`REPORTED`, a third-party summary of the 2023 release), cross-checked where possible.

| Area | VUE feature | Evidence | Jenga Dunia today | Gap / opportunity |
|---|---|---|---|---|
| Terrains | Heightfield, procedural, **infinite** and **planetary** terrains; "real-world terrains" | `REPORTED` (Wikipedia); infinite = "a very large, 200 km square terrain" and planetary terrains in 7.5 (`REPORTED`, [CG Press 7.5](https://cgpress.org/archives/vue_7_5_xstream_and_infinite_features_announced.html)) | Heightfield worlds up to 4 GB budget; GeoTIFF import | **Infinite / planetary extension** is Phase 5 streaming + origin rebasing; a procedural "beyond the edge" fill is cheap and nobody else ships it in an open tool |
| Terrains | 3D terrain sculpting (overhangs, caves) since Vue 8 | `REPORTED` ([CG Press Vue 8](https://cgpress.org/archives/vue_8_infinite_and_vue_8_xstream_released.html)) | Heightfield sculpt only | Out of scope for a heightfield core (ADR); overhang *meshes* could come from a voxel layer later |
| Terrains | Heightfield terrains with hydrological and thermal simulation whose parameters can be **animated over time** ("simulate terrain erosion over time") | `REPORTED` ([CG Press 2016 preview](https://cgpress.org/archives/vue-2016-preview.html)) | Thermal/hydraulic/stream-power/coastal/aeolian nodes, one state | **Timeline**: keyframe node params and scrub erosion steps (todo "Weather state, timeline") — matches VUE's one terrain feature others lack |
| Terrains | Terrain editor effects: fluvial and diffusive erosion, craters, peaks, strata filter (rock hardness, layer spacing), pebbles | `REPORTED` ([Elephorm guide](https://www.elephorm.com/formation/apprendre-e-vue-9-la-creation-denvironnements-3d/guide-complet-de-lediteur-de-terrain-dans-vue), [Elephorm strata](https://www.elephorm.com/tuto-e-vue-9/fractal-strata), [Renderosity threads](https://api.renderosity.com/forums/threads/2834640/print)) | Erosion nodes, terrace node, sculpt brushes | **Strata (hardness layers)** feeds mesas/buttes for the desert item; **craters** and **pebble** scatter are one-node additions |
| EcoSystems | Material-based, global and **dynamic** EcoSystems; 360° population; painting instances; EcoParticles; populations "that can evolve over time"; multiple global EcoSystem layers saved as EcoPainter brushes (2016) | `REPORTED` (Wikipedia; [CG Press 7.5](https://cgpress.org/archives/vue_7_5_xstream_and_infinite_features_announced.html); [CG Press 2016](https://cgpress.org/archives/e-on-software-releases-vue-infinite-and-xstream-2016.html)) | Biome rules → class masks → scatter; CSV/JSON/GLB instances | **Painted density layers** (EcoPainter-style brushes on masks) and **multi-layer populations with exclusion** are the gaps; a dynamic population (growth/decline over the timeline) is unique |
| Atmosphere | Standard, **spectral** and photometric atmosphere models; presets; procedural volumetric cloud layers; standalone 3D **MetaClouds**; mesh-to-cloud; cloud morphing; OpenVDB in/out; **sky export as HDRI** | `REPORTED` (Wikipedia); SmartClouds library and OpenVDB conversion `VERIFIED` (Bentley page, CG Channel) | Gradient sky dome, steerable sun | **Physical sky + HDRI export** for the viewport and engines is small and useful; cloud layers as a viewport feature later; MetaClouds/VDB out of scope |
| Materials | Node-based procedural materials ("Function Editor" with node frames in 2024), PBR, Substance, volumetric materials / hypertextures, Ptex | `VERIFIED` (Bentley page: "Function Editor"), `REPORTED` (Wikipedia) | Baked albedo/normal/roughness; biome class masks | Todo "Material graph + baking": altitude/slope/moisture-driven layering with roughness variation, exported as masks + PBR — a **function graph for materials** is the planned form |
| Modeling | Primitives, metablobs/**hyperblobs** (baked export), splines, **road construction toolkit**, **random rock generator** with rock export | `REPORTED` (Wikipedia; spline toolkit in Vue 10 per [CG Channel](https://www.cgchannel.com/2011/11/e-on-software-announces-vue-10-features)) | None | **Spline tools** (roads, rivers, paths that cut/flatten terrain) and a **rock generator** (meshes for scatter proxies) are gaps relative to World Creator too |
| Animation | Automatic wind and breeze; localized wind per plant / population; ventilators; time spline editor | `REPORTED` (Wikipedia) | Wind direction only drives dunes and waves | Wind as a scene state shared by dunes, waves, snow and scatter (todo "Weather state") |
| Rendering | Ray tracer and GPU path tracer, GI, caustics, 360/VR panoramas, network rendering (HyperVue / RenderCows) | `REPORTED` (Wikipedia; path tracer 2016 `REPORTED` CG Press) | Three.js viewport (WebGPU) | Not chased: Jenga Dunia exports to renderers; a **360° panorama capture** of the viewport is the cheap part |
| Export | Single-asset and full-scene export, camera export (FBX, Nuke.chan), Python API, GoZ bridge, Export Central (2024) | `VERIFIED` (Bentley page: "New Export Central Panel" with "global and local export overrides") | GLB/FBX/OBJ/STL/PLY/USD, engine profiles, MCP/CLI | A **Python API** equivalent exists through MCP + CLI; an "export central" page that runs all exports with overrides is a UI item |

## What nobody else ships (and fits a terrain tool)

Ranked by value for Jenga Dunia, with the smallest honest version of each:

1. **Erosion / weather timeline** (`REPORTED` VUE 2016 heightfield animation). Keyframe node parameters and simulation step counts on a timeline; scrub to any time; export frames. Smallest version: a `time` scalar on the graph, nodes read `steps = f(time)`, the desktop gets a slider and a record button. Builds on the existing journaled graph; no competitor among open tools has it.
2. **Painted ecosystem layers that evolve** (`REPORTED` dynamic EcoSystems). Density-paint brushes on biome masks (the sculpt brush machinery already exists), several population layers with exclusion radii, growth over the timeline.
3. **Desert family with strata** (`REPORTED` strata filter) — ties to the owner's desert mandate: hardness layers for mesas/buttes, plus karst sinkholes, wadis, playas, yardangs.
4. **Physical sky and HDRI export** (`REPORTED` sky export as HDRI). Hosek–Wilkie or Preetham sky in the viewport, written as an EXR environment for engines.
5. **Spline toolkit** (`REPORTED` Vue 10 splines: roads, EcoSystems along splines, terrain effects). Splines that flatten/cut terrain and carry scatter.
6. **Procedural infinite extension** (`REPORTED` infinite terrains). Beyond-the-border continuation from the noise node's seed, for horizon fill.
7. **Rock generator** for scatter proxies; **crater** and **pebble** nodes.

Not adopted: VUE's renderer, cloud modelling/VDB, plant modelling, DCC plug-ins, 3D sculpting of overhangs.

## Gaps in this study

- e-on's own manual is no longer online (info.e-onsoftware.com redirects to Bentley's summary page); feature wording above comes from Wikipedia and trade press, tagged `REPORTED`.
- Dynamic EcoSystem rule syntax and the Function Editor's node set were not found in any accessible source.
