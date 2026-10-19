# World Creator (BiteTheBytes GmbH) — Technical Competitive Research Report

**Prepared for:** Jenga Dunia (open-source Rust/Tauri procedural terrain tool)
**Date accessed for all sources:** 2026-10-19
**Primary vendor site:** https://www.world-creator.com/en/index.phtml
**Primary docs:** https://docs.world-creator.com (GitBook; every page also served as Markdown by appending `.md`; machine index at https://docs.world-creator.com/llms.txt)

## Evidence legend

| Tag | Meaning |
|---|---|
| **VERIFIED** | Seen directly in official BiteTheBytes material (world-creator.com, docs.world-creator.com, EULA, release notes). |
| **REPORTED** | Third-party source (CG Channel, Digital Production, 80.lv, renderu, CGPress, forum/review posts). |
| **INFERRED** | My own inference from the above; not stated anywhere explicitly. |
| **UNVERIFIED** | Could not be found; stated as a gap. |

Where numbers conflict between sources, both are given and the conflict is flagged. No version numbers or features are invented; everything is traceable to a URL below.

---

## 0. Executive summary

- World Creator (WC) is a Windows-only, fully GPU-driven, **layer/filter-stack** (not node-graph) real-time terrain generator written in C#/.NET 9 on a custom Vulkan renderer. The current stable release is **World Creator 2026.4** (released 2026-04-24). A major successor, **World Creator 4**, was announced 2026-10-12 for a November 2026 release and adds a node view alongside layers, planetary-scale terrain, a rasterized real-time renderer next to the existing path tracer, batch processing, an MCP server, and (planned) macOS/Linux builds. (VERIFIED for 2026.4 and platform; REPORTED for WC4 details.)
- The key architectural idea is the **Level Step system**: the terrain is generated as a pyramid from 2x2 px upward, doubling each level; every filter has a per-level strength curve. This is what lets erosion feel "real-time". WC4 reportedly replaces it with direct heightfield sculpting.
- Terrain = one base terrain + stacked **Biomes**; each biome holds Filters, Materials, Objects, and Simulations; everything can be masked by a **Distribution stack** (noise/pattern/terrain-feature/layer masks) with its own **Effect stack**. Shape Layers (Sculpt, 2D/3D Stamp, Landscape, MapTiler, Path, Polygon, Primitive, Rivers) define the base shape.
- Representation: heightfield in metres with pixel precision 1 m down to 1/8 m; non-power-of-two sizes allowed; recommended max 16,384^2, 32,768^2 "may work"; viewport preview capped at 4k (8k in High VRAM Mode). Export heightmaps as RAW/EXR (16/32-bit float), TIFF (8/16/32), PNG (8/16) plus XYZ/ASC; meshes OBJ/FBX/glTF; splines as .dae; object instances as CSV + JSON.
- Bridges (Unity, Unreal, Blender, Cinema 4D, Houdini, Godot/Terrain3D) are **file-based**: a `Sync` folder with `bridge.xml` + `heightmap_x_y.raw` (uint16) + colormap/texturemap PNG + `splatmap_x_y.tga` (RGBA = 4 layers) + assets; the DCC-side plugin reads the XML and builds native terrain. No sockets, no live link.
- No CLI, API, or scripting exists today (UNVERIFIED-negative: nothing in docs or press). Batch processing + MCP server are WC4 promises.
- Pricing (live buy page, 2026-10-19): Indie $59/yr or $149 perpetual (revenue < $100k, export capped at 8k); Professional $99/yr or $289; Studio $179/yr or $499; Enterprise $529/yr or $1,499. Free export-disabled **Community Edition** (4096^2 sandbox). EULA says generated output ("Customer Assets") belongs exclusively to the user.
- Recurring user complaints (historic and current): crashes (especially AMD GPUs, denoiser), export-folder confusion, single water layer, windowed-only renders, Indie tier feature stripping, slow Unreal bridge syncs on large maps, old Unity-plugin era bugs and poor support.

---

## 1. Versions, release history, pricing, licensing

### 1.1 Lineage (VERIFIED unless noted)

Source: https://docs.world-creator.com/world-creator-story.md ; https://www.cgchannel.com/2022/04/bitethebytes-ships-world-creator-3/ ; https://cgpress.org/archives/world-creator-3-released.html

| Era | Name | Notes |
|---|---|---|
| 2006 | GeoControl (Johannes Rosenberg) | CPU terrain generator; BiteTheBytes later took it over. |
| 2008 | BiteTheBytes GmbH founded (Fulda, Germany) | Core tech "CLODDY" (continuous LOD). |
| ~2011 | World Creator 1 | PureBasic; "slow". Steam listing app 314730 still exists. |
| 2015 | World Creator for Unity (Asset Store) | "first GPU-based terrain generator". Legacy Unity plugin (Standard/Professional packages still listed). |
| 2017 | World Creator 2 (standalone, Windows + macOS) | Unity-based standalone. REPORTED (CG Channel): had tiled terrain, macOS 10.13.4+ support. |
| 2019–2021 | "World Creator 3" effort | Story page: custom engine/renderer/UI in **C# + Vulkan**; story page says it "failed" as originally planned. |
| 2022-03-04 | World Creator 3 = **2022.1** | REPORTED: "redeveloped from scratch to remove all dependencies on Unity"; 33 filters; single terrain capped at 4096^2 at launch; tiling, 3D asset dressing, lights/cameras not yet ported from WC2. |
| 2022-04-12 | 2022.2 | Fast Render mode, Unity + Unreal (UE4) bridges. |
| 2023-08-18 | 2023.2 | "Cleaner, more modern UI"; release notes recorded from here on. |
| 2023-09-15 / 2023-11-17 | 2023.3 / 2023.4 | 2023.4: simulation layers (paintable), rain/erosion maps, export tile count readout, seam fixes for tiled export. |
| 2024-04-05 | 2024.1 | Rivers + Polygon shape layers, spline import/export, HDRI lighting, .NET 8 (+15%), licensing moved to web server. |
| 2024-06-15 | 2024.2 | Matcap render mode, 3D-stamp from model, topo export, "true size" export for 32-bit float EXR/RAW. |
| 2024-11-15 | 2024.3 | Landscape/Primitive layers, Debris + Snow sims, many filters (Strata, Chipped, Shore, Kuwahara…), AO/heatmap/sim export, incremental export, Substance baked in sync, sculpt layers up to 1024^2, 8 new UI languages. |
| 2025-09-23 (REPORTED date) | 2025.1 | New Biome system, custom 3D object import + scattering, Megascans, unified Materials, ray-trace denoiser, World Scaler, Scene layer, camera flythroughs, LUTs, R16 export, 140 PBR scans + asset pack (Pro+). Licensing overhaul (Studio tier). |
| 2025.2–2025.6 | point releases | 2025.2: fluvial colour advection, XYZ/ASC export, HDRI render export, custom sync folder, texture compression (-30% VRAM), **Vulkan memory-leak fix**. 2025.3: ORBX export, hole painting (caves), Godot bridge (dev), relief map export. 2025.4–2025.6: bug fixes. |
| 2026-01-26 | 2026.1 | Pen/tablet rework, Smooth filter, **.NET 9** (+10%). |
| 2026-02-19 | 2026.2 | 4x viewport terrain detail (shows >4k), Pattern distributions (Dots/Grid/Maze/Spiral/Splatter), AgX tonemapping, crash reporter, sand sim up to 250% faster. |
| 2026-03-12 | 2026.3 | GIS import: GeoTIFF, HGT, DTED, Arc/Info ASCII Grid, VRT, XYZ; preset import from old versions; terrain adapts to placed objects; object-area distribution; camera-bound high-density scattering. |
| 2026-04-24 | **2026.4 (current stable)** | Decals, expression parsing in numeric fields, object shadow settings, terrain normal blending, VRAM-based max object count, Community Edition. |
| Announced 2026-10-12, due Nov 2026 | **World Creator 4** | REPORTED (CG Channel, Digital Production, renderu): new core on updated Cloddy SDK, planets, node view + layers, direct heightfield sculpting replaces level steps, >110 noise types, rasterized real-time renderer + path tracer with DLSS 4.5, batch processing, MCP server, macOS/Linux planned, version numbering abandons year scheme. |

News feed (VERIFIED, https://www.world-creator.com/en/news.phtml) shows bridge cadence in 2026: Unreal 5.7 bridge with PCG object transfer (Jan 28), Unreal 5.8 bridge (Jun 23), Unity 6.4 bridge (Mar 27), Blender 5.1/5.2 bridges (Mar 27 / Jul 25), Godot 4.6 bridge (Jun 16), Blender bridge moved to Geometry Nodes (Jul 1). Homepage claims "hardware raytracing and DLSS 5 support" are coming in 2026 (marketing; version unspecified).

### 1.2 Pricing (live buy page, 2026-10-19) — VERIFIED

Source: https://www.world-creator.com/en/buy.phtml (all prices USD, ex-VAT; "Windows only" on every tier)

| Tier | Eligibility | Yearly | Perpetual (12 mo maintenance) | Renewal/yr (buy page) | Renewal/yr (docs purchase page) | Lifetime | Export cap | Support |
|---|---|---|---|---|---|---|---|---|
| Indie | revenue/funding < $100k | $59 | $149 | $45 | $29 | $379 | Up to 8k | 1–5 business days |
| Professional | individual / single-person business ≤ $200k | $99 | $289 | $89 | $59 | $729 | Unlimited | 24–48 h |
| Studio | multi-employee company ≤ $1M | $179 | $499 | $149 | $119 | $1,249 | Unlimited | 24 h guaranteed |
| Enterprise | > $1M (mandatory) | $529 | $1,499 | $449 | $449 | $3,749 | Unlimited | 24 h, account mgmt |

Conflicts flagged:
- Renewal prices differ between the buy page and https://docs.world-creator.com/getting-started/purchase.md (second-last column). FAQ (https://docs.world-creator.com/faq/purchase-and-license.md) says renewal = "30% of the listed price". Treat the buy page as authoritative.
- CG Channel's 2026.4 article (2026-04-28, https://www.cgchannel.com/2026/04/world-creator-2026-4-is-out-with-a-new-free-community-edition/) listed Indie $99/$49, Pro $199/$99, Studio $399/$199 — lower than today's page. Either a launch promotion or since reverted. REPORTED; today's page wins.
- 2025.1 era (REPORTED, https://www.cgchannel.com/2025/09/bitethebytes-releases-world-creator-2025-1/): Indie $149 with cap cut to $50k/yr and **no object scattering**, Pro $249, Studio $499, Enterprise $1,669–$1,699; subscriptions dropped for Indie/Pro. 2026.3 (REPORTED) "reduced the price and raised the Indie revenue cap" (now $100k).
- Whether Indie still lacks object scattering in 2026.4: UNVERIFIED. The EULA (2025-09-22) still says Indie has "feature limits apply" and no bundled Assets; the buy page comparison table's per-tier checkmarks did not survive fetching.

Other licence facts (VERIFIED):
- Community Edition (launched with 2026.4): full features, sandbox up to 4096 x 4096, save/load, **export and sync-export disabled**, non-commercial/learning only. (https://www.world-creator.com/en/index.phtml)
- Academic: 30% discount for students/teachers/institutions on proof; classroom/site packages on request.
- Floating: EULA §8.7 — all licences floating, 2 simultaneous activations per seat, max 3 resets/month. Offline licence files only for Studio/Enterprise (12-month validity). Site licence = unlimited seats at one physical location.
- Yearly plans stop working when they lapse; perpetual keeps last-eligible version.
- Volume: ~10% (5–9 seats), 15% (10–19), 25–30% (50+).

### 1.3 Licensing of generated output — VERIFIED

Source: https://www.world-creator.com/en/eula.phtml (Last updated 22 September 2025)
- §11.4 / §12.1: "Customer Assets belong exclusively to you … BiteTheBytes makes no claims over your Customer Assets, whether used privately or commercially." Customer Assets = content created with the Software, with or without embedded WC Assets.
- §9: bundled "World Creator Assets" (textures, 3D models, brushes, HDRs, bridge tools, samples) may be used only when integrated into Customer Assets; no standalone redistribution or asset-store upload, even modified.
- §8.3: reverse engineering prohibited except where legally mandated. (This report therefore does not inspect project file internals.)
- Buy page repeats "You can sell your assets, games, or renders commercially" on every tier, including Indie.

---

## 2. Core architecture (as publicly documented)

### 2.1 Runtime stack
- **GPU-only generation**: FAQ: WC "performs all its tasks entirely on the GPU"; "a pure GPU terrain and landscape creation tool". VERIFIED (https://docs.world-creator.com/faq/general.md).
- **Graphics API: Vulkan.** VERIFIED by three independent official sources: 2025.2 release notes fix "a Vulkan API memory leak"; the General FAQ crash entry explains a GamePP-installed "buggy Vulkan layer prevents World Creator from creating a Vulkan context" (log: `Value cannot be null. (Parameter 'context')`); the GitBook doc-assistant summarises "World Creator uses Vulkan … rather than DirectX or OpenGL". Older WC1/WC2 needed DirectX 11.1 (simulation mode needed Win8+/DX11.1). The legacy WC2 Mac build required Metal.
- **Language/runtime: C# on .NET** — .NET 8 in 2024.1, .NET 9 in 2026.1 (prereqs auto-installed). VERIFIED. Stack traces in the FAQ show a `WorldCreator3` namespace (VERIFIED, minor).
- **Engine**: INFERRED — custom engine descended from the 2019 "WC3" C#/Vulkan effort; 2022.x line "removed all Unity dependencies" (REPORTED by CGPress/Digital Production 2022). Denoiser library is Intel's (2026.2/2026.3 notes: "Intel Denoiser library" crash). Not Unity, not Unreal.
- **WC4 (REPORTED)**: new core "powered by the updated Cloddy SDK" (BiteTheBytes' own continuous-LOD tech, originally used for GPS/helicopter sims), enabling planetary terrain; 9 GB USGS Mars DEM navigable in real time.

### 2.2 Layer stack vs node graph
- WC 2022.x–2026.x is a **hierarchical layer stack**, not a node graph. Tree View: Terrain → Biomes → (Filters, Materials, Objects, Simulations); Shape Layers, Mask Layers, Hole Layers, Biome Layers sit beside. VERIFIED (https://docs.world-creator.com/walkthrough/introduction/main-panel.md, https://docs.world-creator.com/walkthrough/terrain-setup/understanding-terrains.md).
- Stacks are evaluated **bottom-up**; this is stated for Materials ("work bottom-up"), Distributions ("processed bottom-up … can be children of other distributions, which then affect their parents"), and Effects ("applied bottom-up"). Filters share the same model ("similar to the filter system"). VERIFIED.
- Each filter has: Presets, **Level Step Strength** (per-level bars), General Strength, **Combine Operation**, Invert, plus a Distribution (mask) stack. Shape layers have Operation = Overwrite/Add/Min/Max, Interpolation (Linear/Catmull-Rom/Nearest for texture-based), Height Offset/Scale, Blending, Level Step insertion point. Distributions combine with Multiply/Add/Subtract/Min/Max + Opacity. VERIFIED.
- Legacy difference: WC2 (Unity-based) also layer-based with a "Flow map"/filter stack (REPORTED, old forum posts); no evidence WC ever had a node graph before WC4. WC4 "adds a node view alongside the layer system" (REPORTED: CG Channel, renderu — "nodes supplement layers rather than replacing them").
- Hobbyist/press framing (REPORTED, Microbion blog, tripo3d roundup): WC's filter stack is "more intuitive than node graphs" for artists but gives "less detailed node control than Houdini or Gaea".

### 2.3 Level Step system (the real-time trick) — VERIFIED
Source: https://docs.world-creator.com/walkthrough/terrain-setup/level-step-system.md
- Generation starts at a 2x2 px grid and doubles per level (4x4, 8x8, …). Level 8 = 256^2 (65,536 px); level 11 = 2048^2 (4.19 M px).
- Each filter (and shape layer/base shape) exposes a per-level strength curve; low/mid levels define macro shape, high levels add detail. Erosion at low levels needs fewer iterations for sediment to travel the same distance, so most of the expensive work happens at low resolution and is **upsampled** rather than recomputed at full res.
- Shape layers "become sharper at higher level steps because they blend with the terrain at higher resolution"; filters inserted high up have fewer levels to act on.
- Simulation layers: each simulation's sediment layer "is upsampled across level steps, and each step can simulate it further"; Iterations are set **per level step**.
- WC4 (REPORTED) abandons this in favour of direct heightfield sculpting — a signal that the level-step pyramid has artist-facing costs (parameters that behave differently per level; sculpt detail interacting with upsampling).

### 2.4 Parameter evaluation, caching, seeds
- **Realtime Generator** toggle: when on, every change regenerates automatically; when off a red **Generate** button appears. Options: Filter Live Preview (hover previews), Gen Full Res in Background (viewport-visible region first, remainder in background), Show Intermediate Results, Wait for Base Shapes, Use Generator Start Delay (debounce on heavy scenes), Performance Scale (GPU work per UI frame), High VRAM Mode (viewport preview 4k → 8k). VERIFIED (https://docs.world-creator.com/reference/options/generator.md).
- 2026.4: "Automatic material recomputes are disabled in Shape Design Mode" — evidence of staged recompute (shape → filters → materials → objects), each toggleable in the toolbar (Generate Filters / Generate Materials / Generate Objects). VERIFIED.
- **Seeds**: a terrain-level seed ("same seed gives the same shape"), plus per-noise/filter/distribution/object seeds. Determinism across GPUs is not claimed; indeed 2025.3 fixed "Splat Map export on AMD GPUs so output is … consistent across hardware", implying cross-vendor divergence had existed. VERIFIED (notes) / INFERRED (non-determinism risk).
- **Expressions**: 2026.4 allows mathematical expressions in numeric fields. VERIFIED.
- Caching/dirty-tracking internals: UNVERIFIED (not documented).

---

## 3. Terrain generation features

### 3.1 Noise (VERIFIED, https://docs.world-creator.com/reference/terrain/noises.md)
Types: Billow, Gabor, Perlin, Phasor, Ridged, Simplex, Value, Voronoi (F1 distance, cells, F1 fBm, distance-to-edge), Wave, White; Worley (legacy, distributions only). Common params: Offset X/Z, Anchor X/Z, Rotation, Seed, Amplitude, Frequency; Fractal = single/fBm/ridged with Octaves, Lacunarity, Gain, Fine Detail (ridged); Domain warp = none/recursive/fixed with Octaves/Amplitude/Frequency (warp uses multi-octave Perlin). WC4 claims >110 noise types (REPORTED).

### 3.2 Shape layers (VERIFIED)
Sculpt (brush + grid tools, up to 1024^2 internal res, bake/import/export), 2D Stamp (heightmap + optional colour map, Set Height Range, flip, convert to sculpt), 3D Stamp (imported model as stamp, textures applied), Landscape (Alpine, Canyon, Cliff, Cracks, Crater, Crater Field, Dunes, Furrows, Island, Mountain 1/2, Noise, Single Crack, Volcano, each with parameters), MapTiler (real-world DEM + satellite colour via MapTiler service token; height/colour resolution; watermark on free plan), Path, Polygon (vertices with inside/outside height & slope), Primitive (circles, gradients, polygons), Rivers. All have Area transform (X/Y, scale, rotation, Fit to Terrain, Blending, Blend Size, Blend Roundness). Shape layer colour can feed materials ("Use Shape Layer Color").

### 3.3 Filters (VERIFIED, https://docs.world-creator.com/reference/terrain/biome/filters.md)
- General: Add/Set, Border Blend (circular/rectangular), Flatten, Zero Edge.
- Design: Curve, Cutoff, Heightmap, Voronoi.
- Effect: Angle Blur, Balloon, Blocks, Crater, Deflate, Denoise, Distortion, Hexagons, Inflate, Kuwahara, Ridged, Rugged, Scatter, Shore, Smooth (2026.1), Smooth Ridges, Spike Removal, Squeeze, Strata, Swirl, Washed Off.
- Noise: Billow, Gabor, Perlin, Phasor, Ridged, Simplex, Value, Voronoi, Wave, White.
- Arid: Canyon, Chipped, Cliffs, Dunes, Rocky Cliffs/Hard/Layers/Plateaus/Sharp/Wide.
- Terrace: Irregular, Simple, Steep. Drift: Angle Break, Wind.
- Basic Erosion: **Hydraulic**, Ridged Flows, Rocky, Soft Flows.
- Advanced Erosion: **Hydraulic Sediment**, Sediment Flows, Thin Flows, Wide Flows (v1s deprecated).
- Sediment: Fill Soft, Mud, Talus.
- Hydraulic (basic) parameters reveal a **droplet/particle** model: Iterations, Viscosity, Irregularity, Evaporation, Surface Erosion, Angle Dependence, Level Erosion Multiplier, Max Erosion Depth; Rain Strength/Probability; Angle Offset, Sediment Loss/Deposit/Fill; Wind Direction/Strength/Randomness; **Thermal** block (Speed, Threshold, Smoothness, Max Thermal Depth, Deposit, Add/Sub balance, Type = two computation modes). Hydraulic Sediment adds Suspend Loss, Erosion Sample Offset, Erosion Height Range, Flow Rotation, thermal slope/sedimentation angles, and **procedural strata layers with varying erodibility** (Layer Strength, Scale, Height Offset, Smoothing, Variance noise). 2023.4 added exportable **Rain Maps and Erosion/Soil Maps** for erosion filters.
- Fluvial colour advection ("Color Erosion", 2025.2) = gradient material + Smear effect driven by flow.

### 3.4 Distributions (masks) and Effects (VERIFIED)
Source: https://docs.world-creator.com/reference/terrain/distributions.md , …/effects.md
- Noise: Billow, Fill, Gabor, Noise Value, Perlin, Phasor, Ridged, Simplex, Value, Voronoi, Wave, White, Worley; Texture Noise (imported tile), Layered Noise.
- Pattern (2026.2): Dots, Grid, Maze, Spiral, Splatter.
- Layer: Area, Border Blend, Filter Change (delta before/after a filter), Mask, Model, Path, Polygon, Rivers, Simulation, Object Area (2026.3).
- Terrain: Angle, Angle 3D, Cavity, Curvature, **Flow** (hydraulic particle sim: Fill, Iterations, Rain Strength, Evaporation, Delta, Dampening, Softness), Height, Height Gradient, Melt, Occlusion, Rocks, Rocky, Rocky Edges, Roughness, Sea Level, Slope, Steepness, Sun Direction.
- Effects on any distribution (bottom-up stack, Opacity + Weight): Angle Blur, Blur, Clamp, Curve, Denoise, Directional Blur, Distortion, Edge, Fast Flow, Invert, Noise Offset, Operation, Remap, Repeat Value, Simple Flow, Small Flow, Smoothstep, Splatter, Step (2025.2), Voronoi.
- Heatmap view shows any distribution; heatmaps exportable (2024.3).

### 3.5 Masks, holes, biomes (VERIFIED)
- Mask Layers: painted (Paint/Erase/Smooth brushes, custom brushes, falloff) or imported bitmap; Flip/Rotate/Clear/Fill; Area transform; referenced by filters/materials/objects via distributions. Resolution unspecified (2025.4 fixed masks "scaled incorrectly when precision changed").
- Hole Layers (2025.3): painted/imported hole mask for caves; export format not documented.
- Biomes (2025.1 model): one base terrain, biomes stacked on top; each biome has its own filters/materials/objects/simulations; placed either by **Biome Layers** (paint + simultaneous sculpt: Raise/Lower/Flatten Paint) or by a per-biome distribution stack; "Overwrite Biomes" and "Overwrite Objects" toggles; base biome cannot have distributions (2026.2); biome distribution exportable as 2D map. Biome presets shippable.

### 3.6 Materials / texturing (VERIFIED, https://docs.world-creator.com/reference/terrain/biome/materials.md)
- Per-material: Tint, Metalness, Roughness, Use Shape Layer Color, Emission. Texture types: None, Set (auto-import by filename suffix `_Color`, `_Normal`…), Package (ZIP), **Substance (.sbsar, exposed params editable)**, Gradient (second "Gradient Distribution" stack, sat-map library with hundreds of gradients, Saturation/Contrast/Brightness, Gradient Detail noise).
- Shared: Normal Strength/Flip Y, Is Gloss, Tile Size/Offset, **Triplanar** (shader-only, not exportable), **Stochastic** anti-tiling (shader-only but can be baked into colour map), **Height Blending** via displacement map, Displacement + Height Scale. Materials bottom-up; masked by distributions. Megascans/Fab supported (2025.1). Ambient-occlusion material; decals (2026.4); texture compression on load (~30% VRAM).
- Output: Splat maps (4 layers per RGBA texture, or RGB, or single-channel per layer), baked Color map (optionally tints only), Normal (orientation presets), Roughness, Metalness, AO, Unity Mask Map (R=metal, G=AO, A=smoothness).
- Limits: Octane bridges cap at 12 layers; Terrain3D (Godot) blends differently; Sollumz (GTA) 4 textures/poly. No documented global splat-layer cap in WC itself (UNVERIFIED).

### 3.7 Objects / vegetation scattering (VERIFIED, https://docs.world-creator.com/reference/terrain/biome/objects.md)
- Formats: **.glb recommended**, .gltf/.fbx/.obj supported. Object Type = Instance (exported/synced as point cloud) or Detail (exported as mask for engine-side scattering). Params: distribution seed, Align to Normal, Normal Offset, Height Offset, Density, Instance Distance, Overwrite Radius (+smooth), Push, uniform/per-axis Scale/Rotation ranges, Use Distribution Density/Scale/Gradient; Sub Objects (radius, amount, density, seed, percentage); material overrides, Two Sided, Alpha Mode, terrain blending by height/normal; "Is Entity" flag for engine GameObject spawn. 2026.3: terrain adapts to placed objects; camera-bound high-density scattering. 2026.4: VRAM-scaled max object count; object shadows.
- Marketing: "4.5 Million scattered 3D objects" (home page), "millions of objects" (features page). REPORTED (Microbion): 16.7 M trees x 97k polys handled in viewport.
- LOD/impostors: UNVERIFIED (not documented).

### 3.8 Water / rivers / lakes / roads (VERIFIED)
- **Rivers shape layer** (2024.1): springs placed by Shift-click; network built from springs (Build From Springs / Generate Stream / Rebuild); per-node Width/Depth multipliers, Flow (sum of parents; Flow Multiplier/Set Flow/Min Flow); global Depth/Width/Curve Amplitude/Repetition; carved **valleys** with floodplain (Width, Depth, Wall Shape, Breakup, Floodplain width/flatness, per-level carving strength); **Max Lake Depth** ends a river at a local depression forming a lake; Sea Level ends rivers; Export Spline → .dae.
- **Fluid simulation layer**: particle/sediment style; Viscosity (water→lava), Speed, Wind, Spawn Level/Amount; "fills local depressions to form lakes" texturable by the Simulation distribution. Snow (snowfall/melt/diffusion/avalanche), Sand (granular settle), Debris (2D splats, custom textures, or 3D meshes; count per level step). Simulation masks import/export; simulation height-change export.
- Rendering water: Scene → Water: Surface Height, Sea Level, Clarity, Color, Coast Foam + range. REPORTED limitation (Microbion): only **one water layer** per scene.
- **Path shape layer**: nodes with width/height, Falloff + curve, width noise, Closed, Flatten, procedural width modes; spline import (2024.1) and export; materials can align to path layers "for simple roads" (2024.2). No dedicated road-mesh/decal-road tool.

### 3.9 Real-world data (VERIFIED)
MapTiler streaming layer (subscription; free tier watermarked) + file import of GeoTIFF, HGT, DTED, Arc/Info ASCII Grid, VRT, XYZ (2026.3), RAW/R16/EXR/PNG stamps. Export to ASC/XYZ for ARMA/DayZ.

### 3.10 Texture synthesis
No generative/AI texture synthesis today (UNVERIFIED-negative; story page lists "AI generation support" only as planned). Stochastic sampling + gradient sat-maps + fluvial colour advection are the anti-repetition tools.

---

## 4. Terrain representation and limits

| Item | Value | Status / source |
|---|---|---|
| Units | metres; size = width x length in m; non-power-of-two allowed (e.g. 463x271, 3674x2839) | VERIFIED https://docs.world-creator.com/walkthrough/terrain-setup/terrain-size-and-detail.md |
| Precision | 1 m/px default; 1/2, 1/4, 1/8 m options; 2026.1 fixed "terrain precision below 1 m" render bugs; 2026.1 mentions "4 m precision" for object export (coarser than 1 m also exists) | VERIFIED |
| Recommended max resolution | 16,384 x 16,384 px; 32,768^2 "may work, untested"; larger "not recommended" | VERIFIED (Unreal bridge page + docs assistant) |
| Launch-era limit | 2022.1 capped at 4096^2 single terrain | REPORTED (CG Channel 2022) |
| Viewport preview | 4k; 8k with High VRAM Mode; 2026.2 "up to 4x more terrain detail … resolutions above 4K now show" | VERIFIED |
| Export cap | Indie 8k; others "Unlimited" | VERIFIED buy page |
| Tiling | Export tile count derived from size x precision; reported in Export panel (2023.4); seams across tiles fixed 2023.4/2024.3; bridges deliver `heightmap_x_y.raw` per tile; Unity bridge "Split Threshold" sets tile grid; Blender bridge can merge tiles | VERIFIED |
| Height precision | Internal: not documented (UNVERIFIED). Export: 16- or 32-bit; docs recommend ≥16-bit and 32-bit if stepping visible; 32-bit float can exceed 0–1 ("true size" export 2024.2) | VERIFIED |
| GPU memory | Export warns when it may exceed VRAM (2024.1); VRAM-scaled object count (2026.4); Temp Directory used for "terrain data on larger terrains"; High VRAM Mode toggle | VERIFIED |
| LOD/streaming | Viewport-visible region generated first, rest in background; no runtime streaming format; WC4 promises MapTiler streaming + Cloddy LOD core | VERIFIED / REPORTED |
| Sculpt layer res | up to 1024^2 (2024.3) | VERIFIED |

---

## 5. Export formats and bridges

### 5.1 Conventional export (VERIFIED, https://docs.world-creator.com/reference/export/conventional-export.md)

| Format | Bit depths | Channels | Suggested use |
|---|---|---|---|
| RAW (docs misspell "Row") | 16, 32 | 4 | heightmaps (R16 import/export; 2024.3 RAW import assumes 16-bit little-endian) |
| TIFF | 8, 16, 32 | 4 | height/colour/splat |
| PNG | 8, 16 | 4 | height/colour/splat |
| EXR | 16, 32 | 4 | heightmaps |
| JPEG / TGA / BMP | 8 | 3/4/3 | colour/splat |
| XYZ, ASC | n/a | — | ARMA/DayZ (2025.2) |
| Mesh | OBJ, FBX, glTF (REPORTED CG Channel 2025.1; features page lists mesh export) | — | docs: 1024^2 heightmap ≈ 2 MB vs mesh ≈ 56 MB; "for Nanite use heightmap" |
| Spline | .dae (Paths + Rivers) | — | |
| ORBX | Octane package (Octane editions) | — | |

Map types: Height (auto-normalise on/off with custom min/max), Splat (RGBA packing options), Color (baked; optional tint-only), Normal (orientation options), Roughness, Metalness, AO, Topo, Relief, Heat (any distribution), Simulation (sediment height change), Biome map, Rain/Erosion maps, Object Details (`DetailInfo.json` + per-layer splat), Object Instances (`InstanceInfo.json` + per-layer CSV point cloud; translation x1024, scale x ModelScale), Unity Mask Map, HDRI render, camera animation image sequences. Incremental export via naming-convention placeholder. Export presets saveable. Default export folder under `Documents\World Creator` (custom folder option; historically a source of confusion — REPORTED OTOY thread).

### 5.2 Sync-Export / Bridge Tools (VERIFIED, https://docs.world-creator.com/reference/export/bridge-tools.md)
- Click **Sync** in WC → writes to `[User]\Documents\World Creator\Sync\` (custom folder since 2025.2):
  - `bridge.xml` — resolution, size, min/max height, tile count, splat layer details (also `sync.xml` per 2026.4 notes: Baseshape Color/Emissive toggles).
  - `heightmap_x_y.raw` — **16-bit unsigned raw** per tile.
  - `colormap_x_y.png` — colour multiply + gradients; `texturemap_x_y.png` — fully baked colour.
  - `splatmap_x_y.tga` — up to 4 layers in R→A order (more layers → more files).
  - `Assets/` — referenced models/textures (Substance baked to plain textures since 2024.3).
- Pure file hand-off; the DCC plugin parses XML and rebuilds. No socket/live link documented.
- **Unreal Bridge** (plugin folder `WorldCreatorBridge`, Window → World Creator Bridge; bridge v2.0; UE 5.7 with PCG object transfer Jan 2026; UE 5.8 bridge Jun 2026): Read XML Data → Landscape Scale, Section Size, Sections per Component, **World Partition** toggle (grid size, world scale, streaming proxies), Import Layers (each WC material → Landscape layer with blending material), Import Materials, objects via **PCG** (detail + instance meshes), Reimport Objects, Reset Placement, Reset Terrain on Resync (keep Edit Layers if off). Max recommended 16k. Z-scale formula not documented. Launch (2022.2) bridge was UE4-only.
- **Unity Bridge** (.unitypackage; Unity 6.x/2022/2023, 6.4 bridge Mar 2026): Terrain Asset Name, Import Layers (terrain layers vs baked colour), Split Threshold (tile grid), World Scale, Material Type (pipeline/custom). Full object sync since Mar 2026. Maps to native Unity Terrain tiles (INFERRED from "terrain tiles" + Unity Mask Map export). URP/HDRP not named in docs.
- **Blender Bridge** (N-panel "WCBridge"; Blender 5.1/5.2 bridges; Geometry Nodes since Jul 2026): mesh with height scale, optional interpolation (single-tile only), subdivision, Decimate, merge tiles; materials as node groups or baked colormap; objects via Geometry Nodes point-cloud/procedural instancing; Reset toggles preserve edits only if names kept; "Modding" output → GTA V via Sollumz/Codewalker (.dds textures).
- **Houdini Bridge** (.hdanc, Python 3 builds ≥ 19.0): output Heightfield (with splat layers as HF mask layers) or displaced Grid; World/Height scale; materials per renderer (Octane example, 12-layer cap); node must stay unlocked.
- **Cinema 4D Bridge**: Standard (no layers), Redshift (full layers), Octane (12 layers); subdivision for viewport/render; optional Octane voxel displacement plane.
- **Godot Bridge** (addon `wc_bridge`, requires **Terrain3D**; Godot 4.6 bridge Jun 2026): Sync Terrain, Vertex Spacing, Height/World Scale; objects imported since Jul 2026; Terrain3D blends sharper than WC.
- Also listed as targets (home page): Octane, CryEngine, Photoshop, Krita, Clarisse, DAZ, 3ds Max, Maya, Quixel Mixer, Substance Designer, Unigine, Lightwave, D5 — via conventional files, not bridges. Roblox/Fortnite/RDR2 "in development" (marketing).

---

## 6. Automation / scripting

| Capability | Status |
|---|---|
| CLI / headless | **Not found** in docs, FAQ, release notes, or press for any shipped version. UNVERIFIED-negative. |
| Scripting API (Python/C#) | Not found. UNVERIFIED-negative. |
| Batch export | Only "Incremental export" (placeholder naming) and Export Presets. Multi-project batch: none shipped. |
| Expressions | 2026.4: mathematical expressions in numeric fields. VERIFIED. |
| Docs API | GitBook `?ask=` endpoint for docs only (not app). |
| WC4 | "Batch processing" and an "integrated MCP server to connect external AI tools (Claude, ChatGPT)" announced, described as experimental. REPORTED (CG Channel, Digital Production, renderu). |
| Licence check | Online login required (EULA §4.3); offline licence files only Studio/Enterprise — relevant for any future headless use. VERIFIED. |

---

## 7. Rendering and platform

- **Viewport modes** (toolbar, VERIFIED): High Quality (path traced) / Textured (fast) / Matcap ("Shape Design") render modes; later renamed Shape Design / Material Design / Cinematic. Overlays: Heatmap, Topo, Grid. Toggles: Sky, Clouds, Water, Displacement, Denoiser.
- **Path tracer**: "built-in real-time path tracer" with GI/reflections; Max Light Bounces, Sample Rejection, Pixels per Frame, Auto Performance, Fast Frames, Limit Render Samples, Max Rendered Objects; denoiser (Fast/Balanced/High, prefilter, interval, blending; Intel library). Home page: "Unlimited Detail viewport previews (8k)". VERIFIED.
- **Atmosphere/sky**: Sun angle/pick, Shadow Softness, Sun/Sky strength, HDRI env map + env lighting; Atmosphere (strength/scale/height/clamp); Clouds (coverage, density, scales, height, border, light scattering, atmosphere mist "Volumetric Dust"). Post: Exposure, Tone Mapping (AgX since 2026.2), LUTs, Brightness/Contrast/Saturation, Vignette, Sharpen, DOF, Bloom. Cameras: FBX import, slots 0–9, linear flythrough image-sequence render, HDRI render export. Screenshots PNG/… with bit depth, transparent background, HDR. Post/environment effects are "for visual purposes only" and not exportable. VERIFIED.
- **DLSS / HW ray tracing**: not in any shipped release notes; WC4 promises DLSS 4.5; homepage says "DLSS 5" coming. REPORTED/marketing.
- **Platform**: Windows 10+ for 2026.x (buy page "Windows only" every tier; FAQ min lists Win7 64-bit for 2022–2025 lines). No macOS since WC2 (WC2 had Metal build). No Linux ever. WC4: macOS + Linux "planned", Linux "pushing to have ready for launch". VERIFIED / REPORTED.
- **Localisation**: EN, KO, JA, ZH (+ PT, PL, IT, HU, FR, ES, TR since 2024.3), loaded from web server.

---

## 8. Hardware requirements (VERIFIED, https://docs.world-creator.com/faq/general.md)

| Line | Minimum | Recommended |
|---|---|---|
| WC 2022–2024, and 2025+ Indie | Win7 x64, i5 4th-gen, **GTX 1080 4 GB**, 8 GB RAM, 1 GB disk | Win10 x64, i7 6th-gen, **RTX 2080 8 GB**, 16 GB RAM, 4 GB disk |
| WC 2025+ Pro/Studio/Enterprise | same but **RTX 2080 4 GB** | same but **RTX 4080 8 GB** |
| WC1/WC2 | GTX 980 2 GB, DX 11.1 (Win8+ for simulation mode) | GTX 1080 4 GB |
| WC2 Mac (legacy) | macOS 10.13.4, Metal-capable GPU, 8 GB | 16 GB |

Note the VRAM figures are internally inconsistent (2080 "4 GB", 4080 "8 GB" — those cards ship with 8/16 GB). Press (REPORTED) states 2026.x requires Windows 10+. AMD GPUs supported but a recurring source of fixes (2024.1 "all AMD-related issues fixed", 2025.2/2026.2 AMD denoiser black screens, 2025.3 AMD splat export).

---

## 9. Known limitations and user complaints

Official/VERIFIED (from release notes and docs):
- Frequent crash fixes across 2023–2026 (export crashes, VRAM leaks, Vulkan leak, AMD, denoiser library, Thai locale, High VRAM toggle + export). 2026.2 added automatic crash reporting.
- Single terrain per project; tiles exist only at export. No runtime streaming.
- Base biome cannot carry distributions (2026.2).
- Triplanar not exportable; stochastic only via baked colour map; post effects not exportable.
- Interpolation in Blender bridge only for single-tile terrains (seams otherwise).
- Mask Layers scaling bug with precision change (fixed 2025.4) and sculpt layers not matching terrain size (fixed 2024.3) — indicates resolution-coupling fragility.
- MapTiler requires paid subscription for watermark-free data.
- Online licence login required; Indie export capped at 8k; Indie historically lost object scattering (2025.1).

REPORTED (third party):
- Microbion hands-on 2025.1 (https://www.microbion.co.uk/html/blog_10_10_25_worldcreator_2025.php): windowed app only — renders larger than the screen impossible; **only one water layer** (no lake + stream); animation "quite basic"; lighting limited to sun/sky/HDRI/emissive; occasional AMD issues.
- Unreal forum (https://forums.unrealengine.com/t/regarding-the-issue-with-the-world-creator-bridge-tool/2522426): 15 km x 10 km sync took ~15 h without World Partition and >24 h with it (support said <1 h expected).
- OTOY forum (https://render.otoy.com/forum/viewtopic.php?t=82909): WC 2023.4 32-bit TIFF export "failed" — files went to fixed Documents folder, not chosen project folder; leftover .raw temp files.
- Unity Asset Store reviews (legacy plugin 2.7.x, https://assetstore.unity.com/packages/tools/terrain/world-creator-professional-55073/reviews): "does not work in current Unity", flat terrain, lost textures/masks on save, tile noise mismatch, preview ≠ final, no HDRP/URP, poor/no support, refund requests. (Old product; included because it shaped reputation.)
- Steam reviews WC1 (https://steamcommunity.com/app/314730/reviews/): crashes losing work, dated/clunky UI, single-core CPU, 4096 cap, infrequent updates.
- 2026 listicles (low confidence): "narrow export options", erosion needs tuning to avoid artifacts, weak large-world tiling/streaming, less node control than Gaea/Houdini.
- Press: WC3 shipped 2 years late; story page admits the original WC3 rewrite "failed". Release cadence historically slow with delayed annual majors, changed to monthly-ish in 2026.
- No Reddit threads surfaced via search (gap).

---

## 10. UI/UX structure (VERIFIED)

Sources: https://docs.world-creator.com/walkthrough/introduction/workspace.md , …/main-panel.md , …/toolbar.md , …/key-bindings-and-shortcuts.md
- **Single window**: 3D viewport (left/centre) + **Main Panel** (right). HUD: compass (top), height meter (left), hover readout (height/coords). F3 hides UI. Reference mannequin (M key) for scale.
- Main Panel = Application bar (Main Menu, New, Load, Save, Save Incremental, Save As, **Sync**, Screenshot, About; "EARLY ACCESS" badge on betas) + **Tree View** (Terrain → Biomes → Filters/Materials/Objects; Shape/Mask/Hole/Biome/Simulation layers; Scene; Export; Presets; Options) + **Properties View** (context panel: Seed, per-level strength bars under sliders, Start Level/Level Step, distribution and effect sub-stacks) + Status bar (generator progress, GPU stats).
- Tree interactions: drag to reorder, drag into viewport to delete, Ctrl-drag duplicate, double-click rename, plus to add, folder to group, circular arrow to swap filter/material type, search field, expand/collapse all. Drag-and-drop any layer into a distribution slot; drag presets in/out of the Presets tree.
- Toolbar (left edge of panel): render modes, Heatmap/Topo/Grid, Sky/Clouds/Water/Displacement/Denoiser, Realtime Generator, Filter Live Preview, Generate Filters/Materials/Objects, Focus, open Screenshot/Export folders, GPU Info.
- Modal **Edit modes** per layer type with brush tools (radius/smooth/strength via Ctrl+scroll combos), grid selection tools, node editing for Path/Polygon/Rivers (Shift-click add, Ctrl-right-click remove, Alt-drag height, R rotate), object placement gizmos (T/R/Z).
- Walkthrough order ("what makes it easy"): Terrain setup → Craft a shape (Landscape/Sculpt) → Add filters (erosion, chisel) with distributions → Run simulations (sand, debris) → Add objects → Lighting/clouds → Save as Package → Export/Bridge. Hover-preview of filters and presets, templates, online sample browser, per-filter presets with thumbnails, and instant regeneration are the explicit ease-of-use levers. Tablet/pen support reworked 2026.1.

---

## 11. Project file format (public docs only)

- Two save types: **Project** and **Package**; Package embeds all textures and "relevant links" so it opens on another machine (https://docs.world-creator.com/walkthrough/save-and-export/save-as-package.md). Incremental saves (Ctrl+Alt+S) and backup copies exist (2024.3 fixed backups not saving textures). 2024.3: "save files optimized to reduce load times". Presets importable from previous versions (2026.3).
- File extensions: **not stated anywhere public** (UNVERIFIED). Sync output is XML + RAW/PNG/TGA (§5.2) — the only openly documented structured format.
- EULA prohibits reverse engineering; no inspection performed.

---

## 12. Capability matrix

| Capability | Status | Evidence URL |
|---|---|---|
| GPU real-time generation (Vulkan, .NET 9) | Available and verified | https://docs.world-creator.com/faq/general.md ; https://docs.world-creator.com/release-notes/version-2025.x/world-creator-2025.2.md ; …/world-creator-2026.1.md |
| Layer/filter stack (non-destructive) | Available and verified | https://docs.world-creator.com/walkthrough/introduction/main-panel.md |
| Node graph editor | Unsupported today; announced for WC4 (Nov 2026) | https://www.cgchannel.com/2026/10/bitethebytes-unveils-world-creator-4/ |
| Level-step multi-resolution pipeline | Available and verified | https://docs.world-creator.com/walkthrough/terrain-setup/level-step-system.md |
| Deterministic seeds | Partially supported (seeds exist; cross-GPU determinism not claimed) | https://docs.world-creator.com/walkthrough/terrain-setup/terrain-size-and-detail.md |
| Noise types (10 + fBm/ridged/warp) | Available and verified | https://docs.world-creator.com/reference/terrain/noises.md |
| Hydraulic + thermal erosion (droplet) | Available and verified | https://docs.world-creator.com/reference/terrain/biome/filters/basic-erosion/hydraulic.md |
| Strata/sediment-aware erosion | Available and verified | https://docs.world-creator.com/reference/terrain/biome/filters/advanced-erosion/hydraulic-sediment.md |
| Fluid/snow/sand/debris simulations | Available and verified | https://docs.world-creator.com/reference/terrain/simulation-layers.md |
| Rivers with carved valleys + lakes | Available and verified | https://docs.world-creator.com/reference/terrain/shape-layers/rivers.md |
| Roads/paths | Partially supported (path layer + material alignment; no road mesh) | https://docs.world-creator.com/reference/terrain/shape-layers/path.md |
| Terrain-feature masks (slope, flow, cavity, occlusion…) | Available and verified | https://docs.world-creator.com/reference/terrain/distributions.md |
| Painted masks / holes | Available and verified | https://docs.world-creator.com/reference/terrain/mask-layers.md ; …/hole-layers.md |
| Biomes | Available and verified | https://docs.world-creator.com/reference/terrain/biome-layers.md |
| PBR materials, Substance, triplanar, stochastic, height-blend | Available and verified | https://docs.world-creator.com/reference/terrain/biome/materials.md |
| Splat/colour/normal/AO/roughness/metal maps | Available and verified | https://docs.world-creator.com/reference/export/conventional-export.md |
| Object scattering (glb/gltf/fbx/obj) | Available and verified (Indie tier restrictions: requires additional investigation) | https://docs.world-creator.com/reference/terrain/biome/objects.md |
| Real-world DEM import (GeoTIFF/HGT/DTED/ASC/VRT/XYZ) + MapTiler | Available and verified | https://docs.world-creator.com/release-notes/version-2026.x/world-creator-2026.3.md ; https://docs.world-creator.com/reference/terrain/shape-layers/maptiler.md |
| Heightmap export RAW/EXR 16/32, TIFF 8/16/32, PNG 8/16 | Available and verified | https://docs.world-creator.com/reference/export/conventional-export.md |
| Mesh export OBJ/FBX/glTF | Partially supported (docs confirm mesh; formats REPORTED) | https://www.cgchannel.com/2025/09/bitethebytes-releases-world-creator-2025-1/ |
| Tiled export | Available and verified | https://docs.world-creator.com/release-notes/version-2023.x/world-creator-2023.4.md |
| Unreal bridge (Landscape, layers, PCG, World Partition) | Available and verified | https://docs.world-creator.com/reference/export/bridge-tools/unreal-bridge.md |
| Unity bridge | Available and verified | https://docs.world-creator.com/reference/export/bridge-tools/unity-bridge.md |
| Blender / C4D / Houdini / Godot bridges | Available and verified | https://docs.world-creator.com/reference/export/bridge-tools.md |
| Live two-way link to engines | Unsupported (file-based sync only) | https://docs.world-creator.com/reference/export/bridge-tools.md |
| CLI / headless / scripting API | Unsupported | (absence across docs + press; https://docs.world-creator.com/llms.txt) |
| Batch processing / MCP server | Unsupported today; announced WC4 | https://digitalproduction.com/2026/10/13/world-creator-4-planetary-terrain/ |
| Path-traced viewport + denoiser | Available and verified | https://docs.world-creator.com/reference/render-settings.md |
| Atmosphere, volumetric clouds, water, HDRI | Available and verified | https://docs.world-creator.com/reference/scene.md |
| DLSS / hardware RT | Unsupported today; WC4 promise | https://www.cgchannel.com/2026/10/bitethebytes-unveils-world-creator-4/ |
| macOS / Linux | Unsupported (Windows only); WC4 planned | https://www.world-creator.com/en/buy.phtml |
| Max resolution 16k (32k untested) | Available and verified | https://docs.world-creator.com/reference/export/bridge-tools/unreal-bridge.md |
| Planetary / spherical terrain | Unsupported today (FAQ: "No, not directly"); WC4 | https://www.world-creator.com/en/features.phtml |
| Free tier | Available and verified (Community Edition, no export) | https://www.world-creator.com/en/index.phtml |
| Output ownership by user | Available and verified | https://www.world-creator.com/en/eula.phtml |
| Project file format spec | Requires additional investigation (undocumented) | https://docs.world-creator.com/walkthrough/save-and-export/save-as-package.md |
| LOD / impostors for scattered objects | Requires additional investigation | — |
| Multiple water bodies at different levels | Unsupported (REPORTED single water layer) | https://www.microbion.co.uk/html/blog_10_10_25_worldcreator_2025.php |

---

## 13. What Jenga Dunia should learn / do differently

### Learn (copy the good ideas)
1. **Multi-resolution pyramid as the core scheduler.** WC's level-step approach is the reason a 4k erosion feels interactive: run expensive simulations at 256–1024 px, upsample, and run only "detail" passes at full res. In Rust/wgpu: represent every stage as a compute pass over a mip level; store a per-stage `Vec<f32>` level-strength curve; expose it but default to sane curves. Keep a "preview resolution" (WC: 4k, 8k in High VRAM) separate from "export resolution" and compute the remainder off-screen (WC's "Gen Full Res in Background").
2. **Separate stacks for value, mask, and mask-effects.** The trio (Filter stack → Distribution stack → Effect stack) with uniform combine ops (Multiply/Add/Subtract/Min/Max + Opacity) is simple to teach and maps cleanly to a tiny IR. Make masks first-class textures that can be exported as heat maps.
3. **Terrain-feature distributions are the moat.** Slope, curvature, cavity, flow (droplet trail), occlusion, sun-direction, sea-level, "filter change" (delta masks), object-area. Ship these early; they are what makes texturing/scattering automatic.
4. **Staged recompute toggles.** Separate "Generate Filters / Materials / Objects" switches, hover-preview of a filter before adding it, a debounce ("generator start delay"), and a visible generator progress bar. Cheap to build, high perceived speed.
5. **Bridges as a dumb, documented file contract.** WC's `bridge.xml + heightmap_x_y.raw(u16) + splatmap_x_y.tga` is low-tech but works across six DCCs. Jenga Dunia should publish an equally simple, versioned manifest (prefer JSON over XML; prefer 32-bit float EXR/R32 plus uint16 RAW for engines) and let community plugins consume it.
6. **Engine mapping details that matter**: Unreal wants uint16 heightmaps, Landscape section/component sizing, World Partition grid, weightmaps per layer, PCG point clouds for objects; Unity wants tiles ("split threshold"), terrain layers, Mask Map (R metal/G AO/A smoothness); Godot via Terrain3D; Blender via Geometry Nodes instancing; Houdini as heightfield with mask layers. Design the export schema so each of these is a thin projection.
7. **Export bit-depth guidance in the UI** (8/16/32 step counts; warn when VRAM/size exceed limits; report tile count before export).
8. **Presets everywhere + hover preview + drag-to-slot.** Per-filter presets with thumbnails, biome presets, export presets. Low engineering cost, big onboarding win.
9. **Licensing/positioning gap to exploit**: WC is Windows-only, online-licensed, Indie-tier-restricted (8k export, historically no scattering), and its free tier cannot export. An open-source cross-platform (macOS/Linux/Windows via wgpu) tool with unrestricted export is a direct wedge; WC4 is about to close the platform gap (Nov 2026), so timing matters.

### Do differently (avoid the documented pain)
1. **Node graph from day one, with a "stack" view as a UI projection.** WC is bolting nodes on in v4 after a decade of layers. Jenga Dunia should keep a DAG as the single source of truth and render it as a linear stack when the graph is linear — gets WC's approachability without WC's ceiling.
2. **Direct heightfield sculpting at native res, not level-step-coupled.** WC4 is abandoning level steps for sculpting because artists found per-level behaviour confusing. Keep the pyramid as an *optimisation* (scheduler detail), never as a user-facing parameter unless explicitly opted in.
3. **Headless/CLI/API first.** WC has none; batch + MCP arrive only in WC4 as "experimental". A Rust core crate + CLI (`jenga render --graph x.json --res 4096 --out exr`) + Tauri UI on top gives CI pipelines, procedural variation runs, and AI-agent hooks for free. Make the graph file a documented, stable JSON schema (WC's project format is undocumented and reverse engineering is forbidden by EULA).
4. **Determinism as a contract.** Specify fixed-seed, hardware-independent results (avoid atomics-order-dependent reductions; use deterministic droplet scheduling or grid-based erosion). WC had AMD/NVIDIA output divergence bugs.
5. **Multiple water bodies and real lakes**: WC allows one water level; implement per-basin water levels (flood-fill/priority-flood) so lake + stream coexist.
6. **Tiling and streaming as first-class**: WC generates one terrain and only tiles at export; seams were a recurring bug. Build world-space tiles with overlap/borders into the core (also unlocks >16k worlds and out-of-core generation within modest VRAM).
7. **Export-folder and path semantics explicit** (project-relative by default, shown in UI) — a recurring WC support topic.
8. **Crash-resilience**: autosave + crash reporter early (WC added automatic crash reporting only in 2026.2 after years of crash fixes). GPU validation layers in debug; graceful fallback when VRAM is exceeded (WC added VRAM-scaled object counts in 2026.4).
9. **Full-screen / off-screen render sizes**: WC couldn't render larger than the window (REPORTED). Render to off-screen targets at arbitrary size from the start.
10. **Windows-GPU assumptions**: WC's min spec is a GTX 1080-class card. Target a lower floor (integrated/Apple Silicon via wgpu Metal) with the pyramid scheduler and f16 preview storage; make "High VRAM mode" an automatic decision based on adapter limits.
11. **Keep scope honest**: WC's rendering extras (path tracer, clouds, DOF, flythroughs) are marketing-visible but explicitly non-exportable. Jenga Dunia can ship a simple PBR rasterized preview with sky/atmosphere and spend the effort on export fidelity, which is what engine users actually consume.

---

## 14. Gaps / things not found

- No public spec of project file extension/structure; no internal height precision statement (f32 assumed, UNVERIFIED).
- Unreal Z-scale formula and supported UE version range are not documented on the bridge page (only bridge binaries per UE version in news).
- Whether Indie tier currently excludes object scattering (2025.1 did; buy-page matrix unreadable).
- No LOD/impostor documentation for scattered objects.
- No Reddit/Discord discussions surfaced through search; complaints come from Steam (WC1), Unity Asset Store (legacy plugin), OTOY/Unreal forums, and one 2025 blog.
- WC4 specifics (node editor semantics, graphics API on macOS, pricing/upgrade path, early access) are all REPORTED from press of 2026-10-12/13; no official WC4 page exists on world-creator.com/news as of 2026-10-19.
- Pricing figures diverge between the buy page, docs purchase page, and April 2026 press; buy page treated as authoritative.

## 15. Source list (all accessed 2026-10-19)

Official
- https://www.world-creator.com/en/index.phtml
- https://www.world-creator.com/en/buy.phtml
- https://www.world-creator.com/en/features.phtml
- https://www.world-creator.com/en/news.phtml
- https://www.world-creator.com/en/eula.phtml
- https://docs.world-creator.com/llms.txt
- https://docs.world-creator.com/world-creator-story.md
- https://docs.world-creator.com/faq/general.md ; …/faq/application.md ; …/faq/purchase-and-license.md
- https://docs.world-creator.com/getting-started/purchase.md
- https://docs.world-creator.com/walkthrough/introduction/{workspace,main-panel,toolbar,key-bindings-and-shortcuts}.md
- https://docs.world-creator.com/walkthrough/terrain-setup/{understanding-terrains,terrain-size-and-detail,level-step-system}.md
- https://docs.world-creator.com/walkthrough/save-and-export/save-as-package.md
- https://docs.world-creator.com/walkthrough/final-touches/add-volumetric-clouds.md
- https://docs.world-creator.com/reference/terrain/{noises,distributions,effects,biome-layers,mask-layers,hole-layers,shape-layers,simulation-layers}.md
- https://docs.world-creator.com/reference/terrain/shape-layers/{rivers,path,maptiler,2d-stamp,landscape}.md
- https://docs.world-creator.com/reference/terrain/biome/{filters,materials,objects}.md
- https://docs.world-creator.com/reference/terrain/biome/filters/basic-erosion/hydraulic.md ; …/advanced-erosion/hydraulic-sediment.md
- https://docs.world-creator.com/reference/terrain/simulation-layers/fluid.md
- https://docs.world-creator.com/reference/terrain/distributions/terrain/flow.md
- https://docs.world-creator.com/reference/{scene,render-settings,presets,export}.md ; …/scene/cameras.md
- https://docs.world-creator.com/reference/options/{generator,graphics,application}.md
- https://docs.world-creator.com/reference/export/conventional-export.md
- https://docs.world-creator.com/reference/export/bridge-tools.md and …/bridge-tools/{unreal,unity,blender,houdini,cinema-4d,godot}-bridge.md
- Release notes: https://docs.world-creator.com/release-notes/version-2026.x/world-creator-2026.{1,2,3,4}.md ; …/version-2025.x/world-creator-2025.{1,2,3,4,5,6}.md ; …/version-2024.x/world-creator-2024.{1,2,3}.md ; …/version-2023.x/world-creator-2023.{2,3,4}.md

Third party
- https://www.cgchannel.com/2026/10/bitethebytes-unveils-world-creator-4/
- https://digitalproduction.com/2026/10/13/world-creator-4-planetary-terrain/
- https://renderu.com/en/news/post/28675
- https://www.cgchannel.com/2026/04/world-creator-2026-4-is-out-with-a-new-free-community-edition/
- https://www.cgchannel.com/2025/09/bitethebytes-releases-world-creator-2025-1/
- https://www.cgchannel.com/2022/04/bitethebytes-ships-world-creator-3/
- https://www.digitalproduction.com/2022/03/08/neueste-version-des-world-creators-wird-veroeffentlicht/
- https://cgpress.org/archives/world-creator-3-released.html
- https://80.lv/articles/world-creator-2026-1-released
- https://www.microbion.co.uk/html/blog_10_10_25_worldcreator_2025.php
- https://forums.unrealengine.com/t/regarding-the-issue-with-the-world-creator-bridge-tool/2522426
- https://render.otoy.com/forum/viewtopic.php?t=82909
- https://assetstore.unity.com/packages/tools/terrain/world-creator-professional-55073/reviews
- https://steamcommunity.com/app/314730/reviews/
- https://discussions.unity.com/t/world-creator-terrain-and-landscape-generator/618636?page=7
