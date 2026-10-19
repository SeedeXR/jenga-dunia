# Competitive Research: QuadSpinner Gaea 2.x and World Machine (Dragontail Peak)

Reference report for Jenga Dunia (open-source Rust/Tauri procedural terrain tool).
All sources accessed 2026-10-19. Evidence tags:

- **VERIFIED** = official vendor site, official docs, official blog/forum post by the vendor.
- **REPORTED** = third-party press (CG Channel, 80.lv, Digital Production), community forum users, directories.
- **UNVERIFIED** = could not confirm from any source; stated as a gap, not a fact.

A recurring caveat: both vendors' documentation is partially stale. Many QuadSpinner doc pages at `docs.quadspinner.com` carry a "being updated for version 1.3" banner and describe Gaea 1.x behavior; the Gaea 2 docs at `docs.gaea.app` have index pages but several sub-pages 404ed during this research. World Machine's `help.world-machine.com` device pages are a mix of 2019-era and 2026-era content. Where a Gaea 1.x doc page is the only source for a Gaea 2 node, that is flagged.

---

## Part A: QuadSpinner Gaea 2.x

### A1. Version, cadence, pricing, licensing

**Current version (VERIFIED/REPORTED):**
- Gaea 2.3 is the current stable release. Official blog post "Gaea 2.3 now available" dated 2026-05-20 (https://blog.quadspinner.com/tag/releases/). CG Channel coverage 2026-06-15 (https://www.cgchannel.com/?p=176494) confirms it shipped in May 2026.
- Last 2.2-line build was 2.2.7.0, 2025-12-24 ("Christmas Release", https://blog.quadspinner.com/gaea-2-2-7-christmas-release/).
- The quadspinner.com/Download/Changelog page still showed 2.2.4.0 as newest at time of search (REPORTED via search snippet); treat as stale.
- Gaea 3.0 was unveiled 2025-12-09 (https://www.cgchannel.com/2025/12/quadspinner-unveils-gaea-3-0/). Per the June 2026 article, 3.0 early access was re-scheduled to July 2026 with a stable release "around September"; as of this research no stable 3.0 announcement was found (UNVERIFIED whether EA actually began). Gaea 2.4 ("God mode" for very large worlds, distributed network builds) was pushed back; no new date.
- Gaea 3 is backwards-compatible but not forwards-compatible: Gaea 3 files cannot be opened in 2.x (REPORTED, CG Channel Dec 2025).

**Release cadence (VERIFIED from blog release list, https://blog.quadspinner.com/tag/releases/):**
2.0 (2024-08-01), 2.0.4 (2024-09-11), 2.0.6 (2024-12-31), 2.1 preview (2025-02-11), 2.2 (2025-07-14), 2.2.7 (2025-12-24), 2.3 (2026-05-20). Roughly two feature releases per year plus point releases. Gaea 1.0 shipped 2019.

**Platform:** Windows 10+ only (VERIFIED, https://help.quadspinner.com/article/9-what-are-the-system-requirements-for-gaea). See A7.

**Editions and pricing (VERIFIED, https://quadspinner.com/Order, section "#Editions"):**

| Edition | Price (USD, perpetual, one-time) | Intended user | License type | Revenue cap | Commercial use |
|---|---|---|---|---|---|
| Community | Free | Learning/evaluation | n/a | n/a | Not allowed |
| Indie | $99 | Independent artists | Node-locked | < $100K | Allowed |
| Professional | $199 | Production artists | Node-locked | < $1M | Allowed |
| Enterprise | $299 | Studios | Movable (transferable within company) | > $1M | Allowed |

Feature gates by edition (VERIFIED, same page):
- Single-build resolution: Community 1024x1024; Indie 8192x8192; Professional/Enterprise 16,384x16,384 ("experimental", "may require very large amounts of RAM").
- Split builds into tiles: Indie, Pro, Enterprise (not Community).
- Tiled world builds: Pro/Enterprise only, "up to 262,144 x 262,144".
- Regions: Pro/Enterprise only.
- Macros: Community none; Indie "Use Only"; Pro/Enterprise "Create + Use".
- Variables, expressions, automation, batch workflows: Pro/Enterprise only.
- Unreal Engine integration: Indie, Pro, Enterprise.
- Houdini integration: Pro/Enterprise only.
- Adaptive meshes: Pro/Enterprise only.
- Gaea SDK and native USD: Pro/Enterprise, "Coming in Gaea 3".
- Use on 2 PCs: Community, Indie, Pro (one named user). Enterprise = one per user, transferable.
- Floating licenses and offline licenses: optional paid add-ons for any edition (floating = shared pool on same network, concurrency limited by seat count).
- Minor updates free; major upgrades paid, but "Gaea 3 is a free upgrade for Gaea 2 purchasers". Gaea 1 users get a 30% loyalty discount.
- Edition upgrades: pay the difference.

Homepage headline (VERIFIED, https://quadspinner.com/): "$199. Pay once. Own forever." Also links to Floating Servers, Volume/Resellers, Education purchasing.

Note the Community edition change: Gaea 1.x Community allowed commercial use; Gaea 2 Community is non-commercial and 1K-capped (REPORTED, CG Channel Dec 2025 and Aug 2024).

**Output usage rights:** No published restriction found on ownership of exported heightmaps beyond the per-edition commercial-use and revenue gates above. The homepage links to "License Agreements" and "Terms of Use" (/Legal) which were not fetched. UNVERIFIED beyond the table.

### A2. Node graph architecture

**Node families (VERIFIED, https://docs.gaea.app/reference):**
1. Primitive (shapes, gradients, patterns, noise)
2. Terrain (large terrain primitives, landform starters)
3. Modify (filtering, remapping, warping, shaping, cleanup)
4. Surface (breakup, stratification, rocks, sand, terraces)
5. Simulate (erosion, snow, water, vegetation, deposition)
6. Derive (masks and data maps from terrain, color, flow, slope, structure)
7. Colorize (color, palettes, channel ops)
8. Output (rendering, baking, export, handoff)
9. Utility (graph organization, control flow, routing, masking, helper logic)
10. Macro (installed custom macros shown as nodes)
Plus an "Integration Reference" for nodes exposed through Gaea2Houdini.

The marketing feature page (VERIFIED, https://quadspinner.com/Gaea/Features) groups differently and names nodes: Terrains/Primitives, Surface, Transpose, Transform, Draw, Mask, Object; Simulations: Erosion_2, Erosion_1 ("Classic Erosion"), Snow, Water, WaterColor, Debris, Vegetation (Trees chain), Accumulator, Anastomosis, Scree, Stones, Crumble; Color: ColorErosion, Mixer, SatMaps ("over 1400 color maps"), Synth, Weathering, LightX, Cartography; Texturing/Output: Data Maps, Geological maps, TextureBaker; Flow and logic: Math, Switch, Route, LoopBegin/LoopEnd. The Gaea2Houdini post says "180+ nodes" (VERIFIED, blog 2025-08-06).

**Graph model (VERIFIED, Gaea 1.x-era docs at https://docs.quadspinner.com/Guide/Graph/Graph.html; still describes the Gaea 2 "Infinity Graph"):**
- Each node is "a heightfield processor that performs a single task"; nodes can have multiple inputs and outputs. "The graph flows one way, from left to right, never the opposite."
- Loop nodes (LoopBegin/LoopEnd) were added in 2.1 (VERIFIED, https://blog.quadspinner.com/gaea-2-1-preview/) so cycles are expressed as explicit loop constructs, not graph cycles.

**Typed ports:** Not documented as a formal type system in any fetched page. Observable: heightfield outputs, mask outputs (e.g., Lakes node's Lakes/Depth/Shore masks), color outputs (Colorize family), and "hidden ports" accessed by dropping a connection on a node's center (VERIFIED for Lakes, https://docs.quadspinner.com/Reference/Water/Lakes.html). Variables are typed: Float, Integer, Choice, Bool, Color, String (VERIFIED, https://blog.quadspinner.com/advanced-features-variables-compute-and-expressions/). Whether port-type mismatches are refused at connect time: UNVERIFIED.

**Data precision (VERIFIED):** "Gaea stores and processes its heightfields in 32-bit floating points" (https://docs.quadspinner.com/Guide/Using-Gaea/FileFormats.html).

**Evaluation model: preview vs build (VERIFIED):**
- Preview resolution selectable 512 to 4096 (https://docs.quadspinner.com/Guide/Using-Gaea/Resolutions.html and https://docs.gaea.app/ui). "Higher resolutions, however, can take 4x longer to process the image."
- Build/export re-evaluates the whole graph at the final resolution: builds "use the preview's graph settings as a template and regenerate the terrain at full export resolution."
- Resolution dependence is acknowledged: "there is always a chance that certain shapes and effects may change across several nodes when seen at different resolutions."
- Properties panel: "Changes apply immediately to the viewport" (docs.gaea.app/ui). Heavy nodes have an Apply/Lock button when Auto Apply is off (Gaea 1.x interface doc, https://docs.quadspinner.com/Guide/Interface/Getting-around.html).
- 2.2.7: "Export/Build commands are now handled asynchronously so the UI stays responsive."

This is a push-style, dirty-propagating, preview-then-rebuild model: edits re-evaluate downstream nodes at preview resolution; final builds are a separate full-resolution pass, delegated to the Build Swarm process. No official document describes the internal scheduler (push vs pull, topological ordering) in those terms; this is an inference from documented behavior.

**Caching, baking, freezing (VERIFIED, mixed 1.x/2.x sources):**
- Baking: "building a node to a specific high resolution and then storing it in a locked or 'baked' form." Baking a node bakes all its ancestors. Max bake resolution was 8192 in 1.x docs (https://docs.quadspinner.com/Guide/Using-Gaea/Cache.html); raised to 16K in Gaea 2.3 (REPORTED, CG Channel June 2026: "maximum resolution for baking maps from terrain has been raised to 16K").
- Baked caches live in `\Caches\Hard\<file>_<hash>` beside the project; session ("soft") caches in a Central Cache Repository (`\Caches\Soft\`), opt-in via "Cache graph on close".
- "Changing the resolution AFTER nodes have been baked will invalidate existing bakes."
- "Linchpin nodes": a baked node connected to unbaked nodes; ancestors of linchpins are not loaded, saving memory.
- Build Swarm reuses baked results: "the Build Swarm will use that cache for those nodes instead of building them." Swarm honors `--ignorecache` (2.2 notes).
- Gaea 2.2: "Cache/Bake/Freeze – Major rewrite for reliability and speed"; "Smart Cache can offload unused cache to disk" (https://blog.quadspinner.com/gaea-2-2-released/).
- Gaea 2.2.7: cache is shared by Baking and Idle Cache ("Freezing"); "Cache is no longer destroyed upon loading; it is overwritten on Save and deleted if unchecked."

**Determinism and seeds (VERIFIED):**
- Erosion_2 "delivers deterministic results with up to 10x faster performance, even on the CPU" (https://docs.gaea.app/using/using-gaea/understanding-erosion/erosion_2/index.html). Gaea 2.2 notes: "Improved determinism on GPU nodes like Erosion2."
- Classic Erosion/Wizard have a "Deterministic (Slow)" option that "will force Gaea to use a single processor core"; otherwise, due to parallel processing, "results can vary slightly between runs" (https://docs.quadspinner.com/Guide/Using-Gaea/Erosion.html, 1.x doc).
- Nodes expose a Seed; CLI `--mutate##` "Repeats the build with randomized node seeds for each run" (https://docs.quadspinner.com/Guide/Build/Swarm.html).
- Hurricane-Ridge-style "same result every build" is therefore guaranteed only for Erosion_2 and for Deterministic mode on classic nodes.

**Portals, groups, chokepoints, tabs (VERIFIED, 1.x docs https://docs.quadspinner.com/Guide/Graph/Portals.html; feature retained in 2.x "Infinity Graph"):**
- "Any output port in a node can be converted to a portal"; connect via a searchable list; portal name appears as a tab on the target input.
- Chokepoints: routing stubs "used exclusively for routing connections."
- Infinity Graph tabs: moving nodes to a new tab auto-converts crossing connections into portals.
- Groups: color-coded, collapsible.
- Gaea 2.0.4 added a "Simplified View" that draws zoomed-out nodes cheaply (https://blog.quadspinner.com/gaea-204/).

**Macros (VERIFIED/REPORTED):** Gaea 2.3's headline feature. "package entire node graphs into a single node"; stored in a central library so editing one updates all instances; author chooses which parameters to expose (REPORTED, CG Channel June 2026; VERIFIED teaser on blog). Indie can use, Pro/Enterprise can create.

**Automation nodes (VERIFIED):** Math (up to 3 inputs a,b,c; per-pixel x,y), MathX (2.1, "custom math-based node processing"), Var (2.2), Switch, Route, LoopBegin/LoopEnd, AB Compare (2.1). Variables bind to any property; Expressions are modifiers on a property or variable (example: multiply height by 0.3). "Exposed properties" are a byproduct of Variables, editable from the Variable Editor panel and from CLI.

**Build types (VERIFIED, https://docs.quadspinner.com/Guide/Build/Build-Types.html and /Tiled.html, 1.x docs; plus https://quadspinner.com/Gaea/Features for 2.x):**
- Normal build: single pass, max 8192x8192 (1.x doc; 2.x Pro raises single build to 16K per Order page).
- Split build: normal build then cut into a grid; same cap.
- Distributed (Tiled) build: terrain split into buckets, each built separately, then blended (default blending 25%; "increasing the blending amount will directly increase the memory and build time"). Buckets "does not interact with neighboring buckets."
- TileGate node: everything upstream is baked in screen-space before the tiled build; each tile's portion is upscaled with a detail-preserving algorithm, then downstream nodes run per tile. Non-tiling (screen-space) nodes are marked with a diamond in the Toolbox in Tiled mode.
- Hybrid builds (2.x, VERIFIED Features page): "a bucket build system that works within your RAM limits," "up to 16K terrains," "like a tiled build but does not suffer from the same 'small tile issues'".
- Regions (2.1+): "Isolate sections of your terrain and build them at higher resolutions"; Build Scope and Build Profiles (https://blog.quadspinner.com/gaea-2-1-preview/). Example on Features page: upscale "1/6th of a 8192 x 8192 terrain".
- Tiled world cap: 262,144 x 262,144 (Order page). Features page phrases this as "support for up to 65 billion pixels" (262,144 squared is 68.7e9; the "65 billion" wording is QuadSpinner's).
- Distributed network builds: "Coming Soon in Gaea 2.4" (Features page).

### A3. Erosion and simulation nodes

Gaea 2 node names (VERIFIED unless noted). Algorithm descriptions are limited; QuadSpinner does not publish solver internals.

| Node | What it does | Algorithm / hardware notes | Source |
|---|---|---|---|
| Erosion_2 | Primary hydraulic erosion in Gaea 2. Params: Duration, Downcutting, Erosion Scale; sediment classes Suspended Load / Bed Load / Coarse Sediments each with own Discharge Angle; Shape, Shape Sharpness, Shape Detail Scale; Orographic influence (Directional Precipitation, Direction, Rain Shadow, Altitude/Slope range, Reverse); Direction Bias (2.2); "Extra Deposits" and "No Erosion" modes (2.3, REPORTED). | "can utilize either GPU or CPU"; VRAM-overflow fallback to CPU; "deterministic results with up to 10x faster performance, even on the CPU"; "low-level thermal erosion wrapped within the hydraulic erosion simulation". Press (REPORTED, Digital Production 2025-07-18) says it "runs entirely on the GPU"; official doc says GPU or CPU. Grid vs particle: UNVERIFIED (not published). | https://docs.gaea.app/using/using-gaea/understanding-erosion/erosion_2/index.html |
| Erosion (Erosion_1, "Classic") | Gaea 1 hydraulic erosion, full control, Selective Processing via Area mask. Outputs Wear, Deposits, Flow. Deterministic option = single core. | Multi-core CPU; nondeterministic unless Deterministic on. | https://docs.quadspinner.com/Guide/Using-Gaea/Erosion.html (1.x doc) |
| Wizard | "a lightweight wrapper for the Erosion node" with Phase 1 presets (Fast Erosion, Gaea Classic, Craggy, Dessicated, Swept Away, Low Sediments, Two Pass, Four Pass) and Phase 2 recipes (Mild Erosion, Channels, Low Deposits, Pitted Surface, Hawaii Furrows, Collapsed); Seed; Deterministic (Slow). | Same engine as Erosion. | https://docs.quadspinner.com/Reference/Erosion/Wizard.html (1.x doc; node still exists in 2.x per Tiled docs) |
| Thermal_2 | Thermal erosion: Duration, Strength, Anisotropy; Talus Angle ("maximum angle of the slopes before materials start to move downward"), Sediment Removal; Feature Scale in meters. | Not stated. | https://docs.gaea.app/using/using-gaea/understanding-erosion/thermal_2.html |
| Thermal (classic) | "sister to Hydraulic erosion"; Thermal Stress Anisotropy. | Not stated. | Erosion overview doc |
| ThermalShaper | "Rewritten for faster GPU execution" in 2.2; minor result variance vs 2.1. | GPU. | 2.2 release post |
| Fluvial | Gaea 1 node (formerly JErosion). Presence in Gaea 2: UNVERIFIED (not found in 2.x sources). | | search only |
| Sediment(s) | Generic sediment deposition; "can make sand or snow"; Drift mode for snow buildup/glaciers; 2.2 adds "Fine" mode. | Not stated. | https://docs.quadspinner.com/Reference/Erosion/Sediment.html (1.x) + 2.2 notes |
| Snow | "Simulate convincing snow accumulation across slopes, crevices, and terrain curvature"; 2.2 adds Model Scale. | Not stated. | Features page |
| Snowfield (2.2) | Snow for large environments, "simulates snow in cascades", new and old snow layers. | Not stated. | 2.2 post |
| Glacier (2.2) | Converts Snow or any heightfield into ice sheets with directional grooves and "fluvial breakage". | Not stated. | 2.2 post |
| IceFloe (2.2) | Scatters frozen chunks along lakes and sea shores. | Not stated. | 2.2 post |
| Lakes / Lake | "simulates rainfall on the terrain and creates physically accurate lakes" using the erosion engine; Precipitation, Small Lakes, Flood Control, Water Floor, Shore Size/Altitude Bias/Size Bias; optional Rain input; outputs main heightfield with lake surface embedded, Lakes mask, Depth mask, Shore mask, hidden Surface port. 2.3 adds "Simple mode" (position + depth) (REPORTED). | Precipitation/flow accumulation on grid (inferred from description). | https://docs.quadspinner.com/Reference/Water/Lakes.html (1.x doc) |
| Rivers | Builds river networks; "will intelligently adapt the terrain to allow river formations while still preserving the overall shape" (1.3 blog). | Not stated. | https://blog.quadspinner.com/gaea-1-3-now-available/ (REPORTED via search) |
| Sea | Adds water surface from edges without touching inland; provides coastal erosion; "replaces Coast". | Not stated. | 1.3 blog (REPORTED via search) |
| Water (2.x umbrella) | "Simulate water bodies such lakes, rivers, and entire coasts. Export water bodies as separate heightfield or meshes." | | Features page |
| WaterColor (2.1) | Colorizes water bodies. | | 2.1 post |
| Debris | "generates millions of physical rock fragments using a physics-based engine"; 2.2 Scale control. | Physics-based particle scattering (per vendor wording). | Features page, 2.2 post |
| Scree, Stones, Crumble, Anastomosis, Accumulator | Named simulation/LookDev nodes; Anastomosis/Shear/Canyonizer/Stacks described as LookDev alternatives that "don't need to be reprocessed with Erosion to be believable". | | Features page, Erosion overview |
| Vegetation: Trees, Shrubs (2.2) | Chained tree populations reacting to previous populations. | | Features page, 2.2 post |
| ColorErosion, Weathering | Color-domain erosion/weathering. | | Features page |
| Uplift, Lichtenberg (2.1) | Added in 2.1. | | 2.1 post |

Data outputs from erosion (VERIFIED, Erosion overview): Wear ("where sediment is removed"), Deposits ("where it settles"), Flow ("its path between the two"). Erosion_2 outputs Flow/Wear/Deposits are confirmed only via a third-party MCP tool listing (REPORTED, glama.ai gaea-mcp); the official Erosion_2 page does not enumerate ports.

Gaea 3 (announced, not shipped): sand/dune solver, spline-guided meandering river model, advanced snow tools, "2.7D displacement" (REPORTED, CG Channel Dec 2025).

### A4. Data outputs and integrations

**Heightmap/bitmap formats (VERIFIED):**
- Features page: "32-bit, 16-bit, and 8-bit bitmaps including OpenEXR, TIFF, PNG, and RAW".
- 1.x FileFormats doc: OpenEXR 32-bit; TIFF 16/32-bit; PNG 8/16/32-bit; RAW 16/32-bit; PSD 16-bit; "exporting as either OpenEXR or TIFF will give you the best results".
- Import: .exr, .tif/.tiff, .webp, .jpeg, .psd, .hdr, .pfm, .r32, .raw, .bmp.
- Scale modes at export: Natural Elevation (`--unclamped`), Full Range (`--forcescale`), Custom range (`--range-min`/`--range-max`), Mapped scale for File nodes (Swarm doc).

**Meshes and point clouds (VERIFIED):** Features page: "OBJ, FBX, DAE, and GLTF"; point clouds "PLY or XYZ". Adaptive meshes Pro/Enterprise. Mesher node existed in 2.0 (crash fix note). TextureBaker replaces MeshWarp (2.2). 2.3 raises bake-map resolution to 16K (REPORTED).

**Masks / data maps (VERIFIED):** Flow, Wear, Deposits from erosion; Lakes/Depth/Shore masks; "Data Maps" and "Geological maps" nodes; Unreal weightmaps; "ColorThreshold" node for masks from color (2.3, REPORTED).

**Water export (VERIFIED):** "Export water bodies as separate heightfield or meshes."

**Tiled export:** Yes, Pro/Enterprise (see A2). Tile naming convention: UNVERIFIED (1.x doc does not list it).

**Unreal (VERIFIED):** Gaea2Unreal plugin, open source under "a very permissive modified MIT License" since 2025-03-11 (https://blog.quadspinner.com/gaea2unreal-now-open-source/). Brings terrain via Gaea's Unreal export node; re-sync after changes; standard and World Partition landscapes; weightmaps; UE 5.3+ (branches for 5.3/5.4/5.5; 2.2 notes add UE 5.6). Build Manager has a Resample option to match Unreal/Unity dimensions (https://docs.quadspinner.com/Guide/Build/Manager.html). A separate early-access "Gaea Unreal Tools" importer exists ("we do not recommend using them in production", https://eadocs.quadspinner.com/gaea-unreal-tools). Available to Indie and up.

**Houdini (VERIFIED):** Gaea2Houdini ("The all-new Gaea2Houdini now available!", 2025-08-06): Houdini runs Gaea as a heightfield backend; "Gaea TOR Processor SOP" bundles a Gaea graph into a SOP; Gaea variables appear in Houdini UI; color workflows incl. ColorErosion; powered by a "background-service Build System" and the Variables/Expressions system. Pro/Enterprise only. SideFX Labs also ships a legacy "Labs Gaea Tor Processor" SOP (Gaea 1.2+). Houdini Engine compatibility: UNVERIFIED (only 2020 forum workarounds found).

**Unity:** No Gaea 2 Unity plugin exists today (searched; none found). A Unity plugin is announced for Gaea 3 alongside 3ds Max, Blender, Maya (REPORTED, CG Channel Dec 2025). Workaround: heightmap export + Resample.

**Blender:** No official 2.x plugin; Gaea 3 announced one (REPORTED).

**Unigine:** Unigine publishes its own Gaea 2 heightmap import guide recommending 32-bit (REPORTED, developer.unigine.com).

**USD:** "Native USD" Pro/Enterprise, "Coming in Gaea 3" (VERIFIED Order page).

### A5. Automation

**Project file (VERIFIED):** Gaea 2 stores worlds in `.terrain` files, plain-text JSON: "Gaea stores its world in a .terrain file using JSON. It uses a common-sense naming system to describe all world properties and expose the entire node graph... compatible with Source Control tools as well File Diff" (https://quadspinner.com/Gaea/Advanced via search snippet; page 404ed on direct fetch, so VERIFIED-by-snippet). Gaea 1 used `.tor`; "Gaea 1.0 .tor files are not compatible with Gaea 2.0 .terrain files because the engines are completely different" (https://help.quadspinner.com/article/42-are-my-gaea-1-files-compatible-with-gaea-2). No published JSON schema (UNVERIFIED structure).

**CLI (VERIFIED, https://docs.quadspinner.com/Guide/Build/Swarm.html; page is 1.x-era but 2.x notes confirm continued existence):**
- Executable: `Gaea.Build.exe` ("Build Swarm") in install folder; Gaea launches it itself for large builds. Gaea 2 developer index also names `Gaea.exe` and `Gaea.Swarm.exe` commands (https://docs.gaea.app/developers).
- Syntax: double-dash switches; quote paths. `Gaea.Build.exe "C:\...\MyFile.tor"`.
- Modes: `--silent` (no console output, for unattended batch), `--automata`/`--automation` (parseable status lines).
- Management: `--savetor`, `--buildlog` (tab-separated `buildlog.txt` with STARTLOG/GAEA/START/END/TOTAL/RESOLUTION/BUCKET SIZE/NODE/OUTPUT/ENDLOG markers, per-node timing), `--open`, `--close`, `--beep`.
- Build modification: `--unclamped`, `--forcescale`, `--mutate##` (1-99 seed-randomized repeats), `--nodemap` (XML map of exposed properties: type, name, friendly name, limits, default), `--resolution####` (512/1024/2048/4096/8192), `--cpulimit##`, `--range-min:###`/`--range-max:###` (must be paired), then `var:value` pairs for exposed properties (must come last).
- 2.2 additions (VERIFIED, 2.2 post): resolution and license-server settings, `buildpath`, `--ignorecache`; "SwarmHost" middleware for custom bridges; Swarm UI overhaul and CLI help restored in 2.1; Batch Build and Mutations in Build Menu; Build Profiles.
- StdOut can be monitored remotely; vendor warns to use `--silent` when capturing output or the build slows.
- Exit codes: UNVERIFIED (not documented).
- Headless: the Swarm is a separate console process, so headless builds are possible on a Windows box; no Linux/Docker story.
- Automation/variables/batch are Pro/Enterprise features (Order page).

**Bridges (VERIFIED):** docs cover "Building Bridges" and "Building a Bridge with the CLI" (developer index); SwarmHost is the 2.2 middleware. Gaea2Houdini and Gaea2Unreal are the first-party bridges.

**SDK:** "TOR Platform SDK" page exists in developer docs with the caution to treat it "as planning context unless the page says the feature is currently available"; Order page says SDK is "Coming in Gaea 3".

**Third-party:** an unofficial "gaea-mcp" MCP server exists (REPORTED, glama.ai) that drives the node graph; indicates the JSON format is hackable.

### A6. Performance and system requirements

**Official requirements (VERIFIED, https://help.quadspinner.com/article/9-what-are-the-system-requirements-for-gaea):**
- Minimum: Windows 10 1803; i5-4200H / FX-8370; 16 GB RAM; GTX 1080 / RX Vega 64; 10 GB disk.
- Recommended: Windows 11 23H2; i7-4770K / Ryzen 5 1500X; 32 GB RAM ("64-128GB for complex, high-res projects"); RTX 3070 Ti / RX 6800 XT or better; 50 GB disk.
- CPU must support AVX2. GPU compute: CUDA 12.5+ or HIP 6.0+. "Gaea can be used without GPU compute with CPU only."
- 16K: "experimental capacity, and may require very large amounts of RAM."

**Engine (VERIFIED, Features page):** "TOR 2.0 harnesses both CPU and GPU in perfect concert" using "modern AVX instruction sets". GPU nodes include Erosion_2, ThermalShaper, Selective Precipitation ("GPU-friendly algorithm"). Erosion_2 falls back to CPU on VRAM exhaustion. Gaea 2.0.1-2.0.3 fixed a "CUDA fallback behavior that would make GPU processing slower than your CPU" (https://blog.quadspinner.com/gaea-204/).

**Resolution limits per tier:** see A1 (1K / 8K / 16K single; 256K tiled Pro+). Preview max 4K. Bake max 8K (1.x) / 16K (2.3).

**Build-time claims:** Erosion_2 "up to 10x faster" than Classic "even on the CPU", "several magnitudes" faster overall (VERIFIED doc wording). No absolute benchmark numbers published. Build logs record per-node times.

**Memory behavior:** Smart Cache offloads to disk; linchpin nodes unload ancestors; Hybrid builds bucket within RAM; "orphan nodes... add to memory waste" (2.0.4 post). Memory docs page referenced but not fetched.

### A7. Platform support

- Windows 10+ only (VERIFIED). "Currently, Gaea is Windows-only"; Linux "being considered, but no timeline" (https://help.quadspinner.com/article/11-can-gaea-run-on-mac-or-linux).
- Parallels on Mac explicitly unsupported: "Gaea 2 requires the AVX2 instruction set which does not exist in the MacOS Parallels environment. Users must utilize Windows natively in order to use Gaea 2." (VERIFIED, https://help.quadspinner.com/article/68-...). Note: Apple Silicon cannot expose AVX2 to a VM at all, so this is a hard block on ARM Macs.
- Wine/Linux (REPORTED): WineHQ forum user got it installed with winetricks `dotnet48 vcrun2019 corefonts d3dcompiler_47` but "the 3D viewport (Unity engine based) flickers severely and the mouse interaction with it, simply doesn't work" (https://forum.winehq.org/viewtopic.php?p=130700). CachyOS user ran it in Bottles but "cannot access my GPU for processing, just my CPU" and could not display terrain in the viewport (https://discuss.cachyos.org/t/has-anyone-gotten-gaea-to-work-on-linux/27050). CodeWeavers CrossOver rates Gaea "Installs, Will Not Run" on Mac (https://www.codeweavers.com/compatibility/crossover/gaea).
- Note the viewport is reported to be Unity-engine based and the app is .NET (dotnet48 needed) (REPORTED).
- Gaea 3 plans: no Mac/Linux announced (REPORTED).

### A8. Known limitations and complaints

- **Stability of 2.0 (VERIFIED, vendor admission):** "stability was a big issue holding back many users from putting Gaea 2.0 into production use"; "the last few builds were... subpar in stability"; causes split between "GPU driver issues" and "a cluster of bugs deep in Gaea's core" (https://blog.quadspinner.com/gaea-204/). 2.2 claims 150+ fixes; 2.2.7 claims "a MAJOR increment in Gaea's stability". QuadSpinner says Gaea 3 will not ship "until community consensus is that it is production-ready" because of the 2.0 experience (REPORTED, CG Channel Dec 2025).
- **Schedule slips (REPORTED):** 2.4 and 3.0 dates pushed back (CG Channel June 2026); 2.0 itself had a "new release date" post.
- **Docs fragmentation (VERIFIED by this research):** three doc hosts (docs.quadspinner.com 1.x, eadocs.quadspinner.com EA, docs.gaea.app 2.x) with 404s and "being updated for 1.3" banners; changelog page stale.
- **Windows-only / AVX2 hard requirement** (see A7).
- **Breaking node changes between minor versions (VERIFIED):** 2.2 says Trees, Cellular3D, Debris, ThermalShaper "may not be fully backward compatible with Gaea 2.1 or earlier"; "back up projects before migrating".
- **Non-determinism of classic nodes** unless single-core (VERIFIED).
- **Tiled-build fragility (VERIFIED 1.x doc):** screen-space nodes break across tiles; needs TileGate discipline; buckets do not interact.
- **Resolution-dependent results (VERIFIED)** acknowledged in docs.
- **Community-edition commercial rights removed in 2.x** (REPORTED).
- **Community sentiment (REPORTED):** Polycount 2021: Gaea "is fast because it's all GPU but that means it has limitations and... lacks a number of features compared to world machine"; artists "prefer to work in Gaea because of the speed and more streamlined UI but do fall back to world machine for some tasks" (https://polycount.com/discussion/228295/...; page 403ed on direct fetch, quoted from search snippet). WM forum Oct 2023: "Original Gaea still falls short of WM"; concern about "a lack of decomposable data types" in Gaea being "a deal breaker" (https://forum.world-machine.com/t/gaea-2-vs-worldmachine/7183).
- Reddit-sourced complaints: none surfaced in searches (gap).

### A9. UX structure

(VERIFIED, https://docs.gaea.app/ui and 1.x https://docs.quadspinner.com/Guide/Interface/Getting-around.html)
- **Infinity Graph** (center): node workspace with tabs; Toolbox inside the graph (drag-drop nodes; Tab-key search popup); Toolbox position Full/Top/Bottom.
- **Viewport**: 3D with optional 2D orthographic view; 2D doubles as editor surface for Draw/Mask/Island nodes; embedded or floating ("Floating Top Most"); Viewport Toolbar sets preview resolution (512-4096), lighting, render styles (realistic + skybox, or AO-based "day" mode), measuring tools; colorized mask previews (2.2).
- **Property Editor** (right): selected node's parameters, immediate apply; toolbar with replace/revert/reset/save state and gizmo access; **Modifier Stack** (lightweight post-process on node output; the 1.x "Post Process Stack").
- **Data View** (right of graph): project structure tabs, Terrain node tree, filters.
- **Build Manager**: on a Build tab reached from the graph area; Terrain Definition controls; Resample for Unreal/Unity; Build Profiles/Scopes (2.1); warnings for non-tiling nodes/unbaked TileGates; Tiled Build (Preview) readout of final tileset resolution.
- **Build Swarm**: separate console process with its own UI (overhauled 2.1).
- **Main Menu** and **Lazy Menu (F1)**: keyboard-first command hub. Gaea 3 adds a "Command Bar" and HUD mode (REPORTED).
- **Examples Browser** (2.2): 60 examples from Start dialog.
- **Variable Editor** panel for variables/exposed properties.

---

## Part B: World Machine (Dragontail Peak series)

### B1. Version, cadence, pricing, licensing

**Current version (VERIFIED):**
- Release series: "Dragontail Peak" (Build 4059 onward, "released from May 2026"; "a release series, not a single build", http://www.world-machine.com/version/dragontail).
- Builds in the series per official releases index (https://help.world-machine.com/topics/releases): 4059, 4065, 4066, 4067. 4059 was Windows-only (forum replies 2026-05-25/27). 4065 (announced 2026-07-22, https://forum.world-machine.com/t/build-4065-dragontail-peak/8510) added native macOS and Linux. 4066 = small bugfix (quick-export, IBL sun toggle, fixed CLI scripting crash and AVX-512 crash on Linux) (REPORTED via search of blog). 4067 shipped first half of September 2026 ("build system quality-of-life", background previewer memory backpressure); 4068 planned to reintegrate ridge-breaking in Flow Restructure (VERIFIED, blog "September Notes / Whats Next" 2026-09-24 and "Wrapping August" 2026-08-26, https://www.world-machine.com/blog/category/world-machine-development-news/dragontail-peak/).
- Prior series: Hurricane Ridge (4041-4051; described on the homepage as "2025 LTS"), Artist Point (4031), Mt Rainier (4015/4020, the "LTE"/Long Term Evolution line that moved the UI to Qt, 2021), Alpine Lakes (3025-3028).
- Homepage history: WM 1 (2005), WM 2 (2008), "45 releases between 2008 and 2025" (VERIFIED, https://www.world-machine.com/).

**Cadence:** named series roughly every 1-2 years, with monthly-ish point builds inside a series and a developer blog each month. Single-developer company (Stephen Schmitt), which users have noted as a risk ("almost no signs of life from Stephen", WM forum Oct 2023, REPORTED).

**Editions and pricing (VERIFIED, https://www.world-machine.com/purchase.php and /purchase.php?page=pro; feature page https://www.world-machine.com/features.php):**

| Edition | Price (USD, one-time) | Resolution | CPU threads | SIMD | Tiled builds | Automation | Installs |
|---|---|---|---|---|---|---|---|
| Basic | Free | 1K max (press: 1,025x1,025 px) | 4 | SSE 128-bit | No | No | Personal, non-commercial |
| Indie | $119 | "limited only by RAM" | 4 (purchase page; CG Channel May 2026 said "two CPU cores" - conflicting REPORTED) | AVX2 256-bit | No | No | 2 workstations |
| Professional | $299 | "Unlimited (tiled)" | All cores | AVX-512 | Yes | Yes (XML scripts) | 2 workstations |
| Studio Site | $1,999 | as Pro | as Pro | as Pro | Yes | Yes | Unlimited at one physical address |

- Indie: "Export assets for your commercial projects", "Perpetual license with 1 year of updates".
- Pro adds: "Turbocharged builds using all available CPU", "Multimonitor workstation support", "Tiled terrains for massive world sizes", "Automation to script builds", "Ability to lock macros for resale".
- "All editions include all terrain tools" (features page); Basic "is free for personal, noncommercial work, including the erosion and water tools" (homepage).
- Updates: paid editions get 1 year of updates; renewal available afterwards (forum user recalled ~$115 Pro / ~$45 Standard renewal; UNVERIFIED). "Existing license holders should check their update window. Builds within that window remain downloadable" (VERIFIED, Dragontail page). A current license was required to download Dragontail (REPORTED forum).
- No revenue caps published. Floating licenses: not mentioned (UNVERIFIED/absent).
- "Community editions" (free Basic) for Mac/Linux: "will be out next week" per developer reply 2026-07-23 (VERIFIED forum); shipped status not confirmed.

### B2. Node graph architecture

**Terminology:** nodes are "devices"; the graph editor is the "Workview"; a project is a "world"; saved as `.tmd`.

**Device categories (VERIFIED, https://help.world-machine.com/topics/alldevices/ sidebar):** Generator, Filter, Combiner, Selector, Natural, Output, Parameter, Texture, Utility; plus Macros, Code devices, and (2026) VDM devices. "100+ devices" (homepage). Named devices seen in official pages: Code Device, Instance Tiling, Layers, Material, Crystallize, Crop & Transform, Meshify, Scene Output, Tap, Tint, Strata, Adjust Color, Levels, Checkpoint, Reach Character, File Output, Flow Restructure, Pull-up, Switch, Create Normals, Equalizer, Flipper, Probability, Thermal Erosion, Erosion (Legacy), Chooser, Radial Gradient, Constant, Bitmap Output, Color Generator, Voronoi, Perlin Noise, Height/Slope/Convexity Selectors, Lightmap, Material map/Splatmap (Weightmap), Compare Maps, Control Device, Report Presence/Spatial, Automation Scalar/Parameter, Tiled File Input, Height Output, Mesh Output, Blur, Colorizer, Instance Scattering, Scalar Generator, Compare Numbers, Snow, Coastal Erosion, Create Water, River, Thermal Weathering, Select by ID, ID Convert, Type filter.

**Typed ports (VERIFIED):** WM has explicit data types: Heightfield, Bitmap (RGB), Water (a distinct datatype: "Water in world machine has a velocity as well as a depth", River device page), Scalar/parameter, Mesh/VDM, and as of Dragontail "Byte and int grids are first-class data types" with an int inspector (http://www.world-machine.com/version/dragontail). Erosion (Legacy) accepts "heightfield or Water datatype" on its Water Channel input. Devices can "decompose" data (the WM forum praised "very simple, basic devices" and "decomposable data types" as WM's strength vs Gaea, REPORTED). Parameter ports: Dragontail adds "a rail of parameter ports on the panel edge lets you wire a device directly to a parameter."

**Evaluation model (VERIFIED, mix of LTE-era posts and current docs):**
- Classic model: "interactive preview" at a base resolution (default 128 px; "A value of 64 to 128 is appropriate for most computer systems", https://help.world-machine.com/topic/program-configuration/) plus explicit "Build" at final resolution.
- LTE (4015+) multi-resolution model: "LTE builds maintain results starting at the interactive res" up to "the smaller of the final resolution and a limit you specify" (typically 1K-4K); "There's still a build world command for creating the final output" when final res exceeds the limit; goal is "to drastically reduce the number of builds you need" (https://forum.world-machine.com/t/lte-feature-2/5070).
- Two preview engines: an interactive preview that updates while scrubbing, and a "refining preview" that builds in the background after each committed change; high-res build results are downsampled to replace lower-res results so previews match final output (REPORTED-via-vendor-blog snippets, https://forum.world-machine.com/t/feb-updates/5921, https://www.world-machine.com/blog/?p=938).
- Background previews favor on-screen devices; "it no longer blocks the UI."
- Dragontail: "Narrow-focus preview renders only the selected device at full detail to save memory"; "the background previewer now receives backpressure from memory usage" (4067); "Large builds and previews can page to disk when memory runs low"; "Each device shows its size and build time."
- Memory: by default WM "keeps the results for every device in memory at once"; "Memory Conservation" discards results as the network grows (help ?p=411, VERIFIED via snippet). "Maximum build at once" preference trades RAM for speed (REPORTED forum).
- Checkpoint device exists (listed) - presumably a cache point; behavior UNVERIFIED.
- Determinism: Hurricane Ridge: "Erosion output now matches between builds" (VERIFIED, https://help.world-machine.com/topic/build-4041-hurricane-ridge/). Older worlds "should build identically" after upgrade; devices keep old behavior unless upgraded individually (VERIFIED Dragontail page). Seeds exist per generator; Dragontail adds "Randomize flag support" in Code/Macro devices.
- Non-destructive with "full edit history" (homepage).

**Precision:** Heightfields exported as 32-bit float (EXR/RAW-FP32); "RGB packets use float16, which... cuts memory use by 50%" (Hurricane Ridge). Internal heightfield precision not explicitly stated but float32 output and "Natural/Full Range/Specify" elevation mapping imply float internal (VERIFIED outputs; internal = inferred).

**Groups, macros, blueprints, tabs (VERIFIED, features page + Dragontail page):** groups; macros ("wrap a subgraph into one reusable device"), versioned macros and Code devices updatable in place; blueprints; Layout View for drawn shapes (Bezier, auto-smooth paths, boolean compositing, Interpolating Surface); Workview "handles graphs with hundreds to thousands of nodes"; Wire Slice, Organize, Reveal in Workview tools. Portals: not found by that name (UNVERIFIED; WM uses groups/macros rather than portals).

**Automation devices (VERIFIED):** Automation Scalar (binds an XML `<var>` to a parameter), Automation Parameter (now accepts string/filename params), Control Device ("great for macros"), Scalar Generator, Compare Numbers, Switch, Chooser, Probability, Report Presence/Placement/Spatial, Type filter.

**Code device (VERIFIED):** Lua host language; "Slang compute shaders via Vulkan and Metal" (HLSL-compatible); sandboxed plugins; byte/int grid support; VDM support; in-app script editor with reference and examples; errors report real line numbers. Marked "development preview" in Hurricane Ridge.

**Tiled builds (VERIFIED, https://help.world-machine.com/topic/world-machine-professional-edition-addendum/ and features page):**
- Pro only. Parameters: tile resolution, tiles per side, blending percentage; final res = tile res x tiles per side; tile subset export; "Share Edge Vertices"; "Merge Output into single file" (falls back if memory exhausted); "Flip Y-axis"; naming template default `_x%x_y%y` with `%x %y %res`, zero-padding, 0/1 start.
- Each output needs "Participate when building tiled worlds".
- Known issue: "Erosion and Snow can give different results in tiled mode because they can't see beyond the tile"; WM "builds extra border area and blends during a 'Merging' phase."
- Hurricane Ridge: "Tiled builds gain recovery from failures and incremental tile merging."
- Tiled File Input: maps a whole tileset into world space with a multi-resolution cache so you can "edit a world far larger than memory"; infers naming from two sample files; elevation mapping Natural/Full Range/meters; RGB interpretation.
- Create Water: "may not work as intended when used in a tiled build" (VERIFIED device page).

**Render extents / world space:** WM works in km-based world extents (`<location>x0 y0 x1 y1</location>` in km), multiple named extents, which is the feature WM users compared to Gaea's planned "God Mode" (REPORTED).

### B3. Erosion and simulation devices

| Device | What it does | Algorithm / hardware notes | Source |
|---|---|---|---|
| Erosion (current, Hurricane Ridge model) | Hydraulic erosion; "up to 100 times faster at high resolutions"; handles "soil-mantled hills"; feature-size limits, soil evolution from an initial distribution, map control over every parameter; re-parameterized in 4044; outputs flow/wear/deposition masks; deterministic between builds. | CPU, AVX2/SSE vectorized (Pro: all cores, AVX-512). Grid vs particle: not published; a forum thread calls it "secret sauce" (REPORTED). | https://help.world-machine.com/topic/build-4041-hurricane-ridge/, features page |
| Erosion (Legacy) | Older model: Duration, Rock hardness, Sediment carry amount, Filter type/strength, Geological-time intensifier, Reconstruction type, Uplift, Channeled Erosion (depth, post-channel %), Preserve map borders. Inputs: Hardness Mask, Water Channel (heightfield or Water). Outputs: Flow, Wear, Deposition masks. "Pre-existing water is assumed to have an infinite capacity to carry sediment away." | CPU. "one of the slowest devices". | https://help.world-machine.com/topic/device-erosion/ |
| Thermal Weathering (Hurricane Ridge) | "simulates the fracturing of exposed rock into talus"; talus can "completely bury vulnerable ridgelines"; mass balance option (bedrock wear vs talus deposition); mass-movement-only mode ("treats the entire input as granular material"); crack-like rock textures. | CPU. | 4041 notes, features page |
| Thermal Erosion (classic) | Talus Production, Talus Repose Angle (real talus ~30-40 deg), Fracture Size, Talus Size, Simulation Length, Intensity; inputs Talus Production Mask, Talus Removal Mask (accepts Water); outputs Talus Mask, Talus Depth ("Subtracting this from the primary output provides the actual post-erosion bedrock surface"). | Angle-of-repose model. | https://help.world-machine.com/topic/device-thermalerosion/ |
| Snow | Accumulation from dusting to glacier-like; Intensity, evaporation balance, snow amount, snow line; melt separated from accumulation (3022 "Mt. Daniel", 2018); "the wind drives it into drifts"; depth mask output; "hundreds to thousands" times faster than the pre-2018 device. | CPU. | features page; https://www.world-machine.com/blog/2018/05/the-new-snow-device/ (REPORTED via snippet) |
| Coastal Erosion | "a fast, non-simulated approximation of water's effect on land"; sets sea level and reshapes near-shore into beaches and bluffs, fills ocean; pairs with Create Water as drainage target. | Non-simulated heuristic. Known bug reports: water level mismatch after 4044 (REPORTED forum). | features page; 2007 dev blog (REPORTED) |
| Create Water | "Automatically place rivers and lakes"; rivers start where upslope contributing area crosses Channel Head Area (km^2); lakes form in depressions; inputs Water, Precipitation; Drain To; Headwater Locations auto/specified; Channel Model none/single-pixel/variable width-depth-speed by discharge; Minimum Water Depth; Discharge Scalar; Flow Speed; Pin current headwaters (resolution-stable). | Flow-accumulation / drainage-area model on grid (per description). Not tile-safe. | https://help.world-machine.com/topic/device-create-water/ |
| Flow Restructure | "removes local minima so river networks appear"; can force water toward map edges or drainage targets; experimental synthesis mode "can generate eroded terrain without running a simulation"; ridge-breaking mode returning in 4068. | Depression-removal / drainage enforcement. | Build 3028 notes (REPORTED snippet), Sept 2026 blog |
| River (device/tool) | Hand-drawn reaches with spline interpolation; tributaries; GCS presets drive meanders ("meanders will grow as the bankfull width increases"), width/depth/structure variation; riffle-pool sequences (features page); Bankfull Width/Depth, Channel Type Curved/Trapezoidal, Flow Speed; valley: floodplain height/width, meander belt flattening, valley width/height/wall shape/breakup; "will always prohibit the river from flowing uphill"; water output with velocity. | Procedural geometric carving. | https://help.world-machine.com/topic/device-river/ |
| Strata, Reach Character | Natural-category shaping devices. | | natural-devices index |
| VDM Erosion / Thermal Weathering / Snow (Dragontail) | Run on Vector Displacement Map terrain ("first true 3D erosion simulations"), experimental. | | Dragontail page |

Water export: "Water is exported as a 3D mesh with a flow vectormap (direction and speed)" (VERIFIED features page).

### B4. Data outputs and integrations

**Heightfield formats (VERIFIED, https://help.world-machine.com/topic/device-fileoutput/):**
Low precision: TGA, BMP, PNG 8-bit, RAW 8-bit headerless. High precision: Terragen (.ter), Leveller, PNG 16-bit, Povray-TGA (16-bit split R/G), RAW16 ("standard PC byte-endian"), RAW-FP32 ("32bit Floating Point RAW"), BT (Virtual Terrain Project), PGM ASCII 16-bit, TIFF 16-bit, HFZ compressed, OpenEXR ("highest-precision 32bit float output"). Dragontail adds "RGB EXR variant" of height export "for Godot" (RGB-packed). Files get `.r16`/`.r32` extensions for RAW (REPORTED forum). Elevation mapping modes: Natural, Full Range, Specify range in meters (VERIFIED on Tiled File Input; same options on outputs per forum). 32-bit TIFF: not listed (only 16-bit).

**"LDR/HDR":** WM's own terminology is "Low Precision" vs "High Precision" outputs, not LDR/HDR. The Dragontail viewport uses "HDR panorama" lighting; that is a rendering feature, not an output format.

**Bitmaps/masks (VERIFIED):** Bitmap Output for color; simulation masks for "talus, wear, flow, sedimentation, and snow"; Weightmap (Splatmap) device: processes N masks with sum-to-one normalization, "typically 4, although sometimes 8 or more"; modes Packed (RGBA per 4 channels), Individual, Material ID (dominant channel); exported via Bitmap Output; Dragontail adds byte-based Material ID output, Select by ID, ID Convert, ID-grid File Input/Output. Limitation: "You cannot currently view the result of distributing high resolution detailed materials" inside WM.

**PBR textures (VERIFIED features page):** diffuse/albedo, normal, height, roughness, metalness, AO; "Create Normals" device.

**Meshes (VERIFIED):** Mesh Output: OBJ, glTF (.gltf/.glb), GLB direct from Mesh Output (Dragontail); Meshify device (volume and quad export restored in 4065); VDM Output; "VDM -> Heightfield+Cliffs" hybrid export; Scene Output. FBX: not listed (UNVERIFIED/absent). Point clouds: not listed.

**Tiled export (VERIFIED):** Pro; any output type can be tiled; naming template; subset re-export; "written out as engine-ready tiles"; resume after failure.

**Engines (VERIFIED features page):** "Unreal Engine, Unity, and Godot natively" - meaning export presets/guides, not live plugins. Official guides: Unreal uses Height Output RAW16 `.r16` imported via Landscape mode (https://help.world-machine.com/?p=1106); Unity uses RAW16 with a Flipper device for Y-flip (https://help.world-machine.com/topic/export-to-unity/); Godot via RGB EXR + Terrain3D (forum Q&A). No World-Partition-aware plugin or live sync equivalent to Gaea2Unreal was found (UNVERIFIED/absent).

**Houdini/Blender:** No official bridges found (absent). "Export Terrain Assets" dialog with editable filenames and one-click selective export (Dragontail).

**Import (VERIFIED):** File Input (heightfield/bitmap/ID grid), Tiled File Input (Pro), scene objects; planned: mesh-to-VDM / mesh-to-heightfield import (Dragontail "Planned").

### B5. Automation

**Project file:** `.tmd` ("World Machine worlds only describe a set of operations to create the terrain rather than the terrain itself", REPORTED fileinfo/forum). Binary vs XML: UNVERIFIED (no published spec). Dragontail: atomic saves (temp file + swap), session tracking for concurrent instances, "project versioning" (Hurricane Ridge).

**XML automation scripts (VERIFIED, Pro only, https://help.world-machine.com/topic/world-machine-professional-edition-addendum/):**
- Run via File > "Run Automation Script..." or by passing the script path as the executable's argument / drag-drop onto the exe.
- Root `<automation version="WMP2">`; tags run in order.
- `<section name enable>` (failures abort only the section), `<echo text>`, `<load file>` (required), `<save file>`, `<build mode="normal|tiled">`, `<output/>` (export all enabled outputs), `<modify>` containing `<world res use_extents>`, `<extent index><location>x0 y0 x1 y1</location><resolution/><name/></extent>`, `<tile res number use_extents>`, `<enable device|group>`, `<disable device|group>` (case-sensitive names), `<variables><var name value/></variables>` bound via Automation Scalar devices; `<tileset>` tag to select tiles (neighbors still built for blending, REPORTED forum).
- Files are written only if the File Output's "Save file every build" is set or `<output/>` is used.
- Dragontail additions: in-app script editor, `<var:name>` template tag for output filenames (prevents batch overwrites), automatic export of build commands, built-in Console (~) that "runs most actions from a command prompt and supports scripted actions"; Automation Parameter accepts strings. 4066 fixed a "CLI scripting crash".
- Headless: the GUI executable processes the script; no documented separate headless binary; exit codes UNVERIFIED. Linux AppImage + script argument should allow CI use but is undocumented (UNVERIFIED).

**Macros (VERIFIED):** user-defined devices from subgraphs; versioned; Pro can "lock macros for resale"; official Macro Library (https://www.world-machine.com/library/).

**Plugins:** Code device (Lua + Slang) is the in-app plugin system; sandboxed; shareable. Older native plugin SDK existed historically (REPORTED forum mentions; current status UNVERIFIED).

### B6. Performance and system requirements

- **CPU-only simulation.** Erosion, thermal, snow, water run on CPU with SSE/AVX2/AVX-512 vectorization gated by edition (VERIFIED purchase page). GPU is used only for the viewport (OpenGL; "OpenGL Core profile" on Mac) and for user-written Code-device compute shaders (Slang on Vulkan/Metal). A "GPU acceleration of compute-heavy nodes" request sits at "Under consideration" on the official feature board (VERIFIED, https://worldmachine.featureupvote.com/suggestions/272437/...).
- **Thread limits:** Basic/Indie 4 threads (purchase page); Pro "all cores" with "Maximum worker threads" and "Allow devices to use workers" preferences (Pro only). Pro addendum (old) said "up to 16 simultaneous processors".
- **Speed claims:** Hurricane Ridge: erosion "up to 100 times faster at high resolutions" (VERIFIED, 4041 notes; homepage repeats "up to 100x"); Mt Rainier "up to 60% faster building typical worlds" (REPORTED); 2018 Snow "hundreds to thousands" times faster. No absolute times published.
- **Memory:** "A new memory system... supports builds handling gigabytes of data"; float16 RGB; async paging; build throttling in low memory; Dragontail page-to-disk and previewer backpressure. Users report "WM eats up memory like crazy" and paging crashes on huge networks (REPORTED forum).
- **Resolution limits:** Basic 1K; Indie RAM-limited single build; Pro unlimited via tiles. Pro addendum notes a single heightfield "practically tops out around 8192x8192" (older doc); forum users mention 8K-16K builds.
- **System requirements:** No official current page found. Third-party wiki: Windows 7+/10 64-bit, quad-core 2 GHz, 8+ GB RAM (REPORTED, Brown VR wiki, dated). Linux: glibc 2.34+ (Ubuntu 22.04 / RHEL 9+), x64, AppImage (VERIFIED). macOS: universal binary .pkg (VERIFIED). Official FAQ: "if you have enough system RAM, you can create as large a world as you want" (REPORTED snippet).

### B7. Platform support

- **Windows, macOS, Linux native** as of Build 4065 (2026-07-22) (VERIFIED). macOS: "Native universal binary for Apple Silicon and Intel... No Rosetta or VM required." Linux: AppImage, "glibc 2.34 or newer". Press notes Linux "focuses on Ubuntu-based distributions" and to "expect some maturity gaps" (REPORTED, windowsforum/cgchannel).
- Technical path (VERIFIED, https://www.world-machine.com/blog/2026/03/mac-world-machine/): Qt port done "5 years ago" for LTE; made GCC/Clang-clean; replaced Win32 threading; fixed `long` size assumptions; Mac needs OpenGL Core profile; OpenGL "officially deprecated", OpenCL "really really deprecated" (hence Slang on Vulkan/Metal for Code devices). "the core code is all platform-agnostic; the main catch has always been the GUI."
- Pre-port: official FAQ said no Mac build but WM "works just fine under Parallels and certainly Bootcamp" unofficially; Wine on Arch "workable" with occasional crashes (REPORTED forum).
- Updater is now built-in on all three platforms with optional auto-install (VERIFIED).

### B8. Known limitations and complaints

- **No GPU simulation** (VERIFIED feature board + forum: "iterating is now very slow, especially with more complex projects"). Slow high-res recompute cited by third-party roundups (REPORTED, low authority).
- **Dated UI** (REPORTED; vendor itself calls Dragontail "a long-overdue design redo" and the device-UI redesign is "Planned" on the feature board).
- **Tiled erosion inconsistency** (VERIFIED addendum: Erosion/Snow differ in tiled mode; Create Water not tile-safe).
- **Coastal/river interactions** (REPORTED forum bugs: coast water level mismatch after 4044; rivers "heavily resolution dependent, and sometimes do not reach the sea").
- **Erosion masks not normalized** across resolutions (VERIFIED LTE post).
- **Memory hunger and paging crashes** on large networks (REPORTED forum; vendor shipped paging hardening in 4065/4067).
- **Single-developer bus factor and slow cadence** (REPORTED forum 2023).
- **Update-window licensing**: must be in a paid update window to get Dragontail; renewal pricing not public (REPORTED).
- **VDM limitations** (VERIFIED): no holes ("you can't create a cave mouth" per press); heightfield+cliff seam quality still being improved; many VDM devices experimental; Coastal/Water/River/AO lack VDM support yet.
- **Thread cap on Indie** (4 threads) is a frequent forum topic ("Unable to use more than 4 threads", REPORTED).
- **No FBX mesh export** found in docs (UNVERIFIED/absent); Photoshop/other-app compatibility quirks with `.r16/.r32` (REPORTED).
- **Splatmap preview** not viewable inside WM (VERIFIED).
- Reddit-sourced complaints: none surfaced (gap).

### B9. UX structure

(VERIFIED, Dragontail page + 4059/4065 notes + program-configuration page)
- **Workview**: node graph (devices, wires, groups, macros, blueprints); Wire Slice, Organize, Reveal in Workview; velocity-sensitive grid snapping; favorites toolbar tab; Devices menu with macros as first-class entries; Library search by version/author/tag; each device shows size and build time; parameter-port rail on device panels.
- **3D View** (merged with former 2D view; standalone 2D removed in 4059): layers list, HDR panorama lighting with rotatable environment, exponential height fog, exposure/tone mapping, soft penumbra shadows (Lightmap), A:B side-by-side and wipe comparison, slope overlay, contour lines, terrain grid, camera slots 1-8, orthographic camera, backface toggle, tile overlay for tiled builds, "Scene Water" toggle, R/F vertical move, Explorer view with LRU cache.
- **Layout View**: vector shapes (Bezier/auto-smooth), river reaches, headwater editing, extents placement for Tiled File Input.
- **Parameter pane** (right side since Hurricane Ridge): combined slider/edit box (Shift fine, Ctrl coarse, double-click exact), draggable numeric fields, pinnable dialogs, "parameter editor that follows selection" under consideration.
- **Build**: single "Build & Export" button; Build Status panel color-coded, sortable, timestamped logs; Project Settings tab for tiled parameters; Export Terrain Assets dialog; quick export from device right-click.
- **Docking/workspaces**: savable workspaces, second-monitor layouts persist (Pro multimonitor).
- **Console (~)** for commands and scripted actions; Script editor; Code Viewer with host/Slang API panes.
- **Messages** as dismissible cards (non-modal); EULA/license as modal card on first run; opt-out pseudonymous analytics (Program Settings > Misc > Privacy).
- Themes, icons, selectable UI font, standard/dense sizes.

---

## Part C: Comparative capability matrix

Status values: **Available and verified** / **Partially supported** / **Unsupported** / **Requires additional investigation**.

| Capability | Gaea 2.3 | World Machine Dragontail (4067) | Gaea status | WM status |
|---|---|---|---|---|
| Free edition | Community, 1K, non-commercial | Basic, 1K, non-commercial, 4 threads | Available and verified | Available and verified |
| Entry paid tier | Indie $99, 8K, <$100K revenue | Indie $119, RAM-limited res, 4 threads, 1 yr updates | Available and verified | Available and verified |
| Pro tier | $199, 16K single (experimental), 256K tiled, <$1M | $299, unlimited tiled, all cores, AVX-512 | Available and verified | Available and verified |
| Perpetual license | Yes, minor updates free, major paid (Gaea 3 free for 2.x buyers) | Yes, 1-year update window then renewal | Available and verified | Available and verified |
| Floating licenses | Paid add-on | Not offered (site license instead) | Available and verified | Unsupported |
| Windows | 10+ | 10+ | Available and verified | Available and verified |
| macOS native | No; Parallels blocked by AVX2 | Universal binary (4065+) | Unsupported | Available and verified |
| Linux native | No; Wine partial, no viewport/GPU | AppImage, glibc 2.34+ | Unsupported | Available and verified |
| GPU-accelerated erosion | Erosion_2, ThermalShaper (CUDA 12.5 / HIP 6.0), CPU fallback | None (CPU SIMD); GPU only via user Code device | Available and verified | Unsupported |
| Deterministic erosion | Erosion_2 yes; classic nodes only in single-core mode | "Erosion output now matches between builds" (HR) | Partially supported | Available and verified |
| 32-bit float pipeline | Stated explicitly | float32 outputs; float16 RGB internal | Available and verified | Partially supported (internal precision not stated) |
| Typed ports / decomposable data types | Heightfield/mask/color; hidden ports; no published type system | Heightfield, Bitmap, Water (depth+velocity), Scalar, Mesh/VDM, Byte/Int grid | Requires additional investigation | Available and verified |
| Preview vs build resolution | 512-4096 preview; full rebuild at export | Interactive res + multi-res cache up to user limit; build when exceeded | Available and verified | Available and verified |
| Node caching / baking | Bake to 16K, hard/soft caches, linchpins, Smart Cache to disk | Multi-res result cache, memory conservation, disk paging, Checkpoint device | Available and verified | Partially supported (Checkpoint undocumented) |
| Portals / tabs | Portals, chokepoints, Infinity Graph tabs | Groups/macros; no portal concept found | Available and verified | Requires additional investigation |
| Macros | Create (Pro+), use (Indie+), central library | All editions; versioned; lockable for resale (Pro) | Available and verified | Available and verified |
| Loops / math nodes | LoopBegin/End, Math, MathX, Var, Switch, Route | Switch, Chooser, Probability, Scalar/Compare devices, Code device (Lua) | Available and verified | Available and verified |
| Expressions / variables | Typed variables, expressions, exposed properties (Pro+) | Automation Scalar/Parameter bound to XML vars | Available and verified | Partially supported |
| User-programmable nodes | MathX only; SDK "Gaea 3" | Code device: Lua + Slang compute (Vulkan/Metal) | Partially supported | Available and verified |
| Hydraulic erosion | Erosion_2, Erosion, Wizard | Erosion (HR), Erosion (Legacy) | Available and verified | Available and verified |
| Thermal erosion | Thermal_2, Thermal, ThermalShaper | Thermal Weathering, Thermal Erosion | Available and verified | Available and verified |
| Snow / ice | Snow, Snowfield, Glacier, IceFloe, Sediments drift | Snow (accumulate + melt) | Available and verified | Available and verified |
| Lakes | Lakes (precip sim), Simple mode | Create Water (depressions) | Available and verified | Available and verified |
| Rivers (procedural) | Rivers | Create Water (drainage area) + Flow Restructure | Available and verified | Available and verified |
| Rivers (hand-drawn spline) | Draw node + Rivers (Gaea 3 adds spline meanders) | River device with GCS meanders, valley carving | Partially supported | Available and verified |
| Coast / sea | Sea (coastal erosion) | Coastal Erosion (non-simulated) | Available and verified | Available and verified |
| Flow/wear/deposit masks | Yes | Yes (flow, wear, deposition, talus, snow) | Available and verified | Available and verified |
| Water as mesh + flow vectors | Water bodies as heightfield or mesh | Water mesh + flow vectormap (dir + speed) | Partially supported | Available and verified |
| Debris / rocks / vegetation | Debris, Scree, Stones, Trees, Shrubs | Instance Scattering/Tiling, Rock and Soil / Vegetation macros | Available and verified | Partially supported |
| Overhangs / 3D terrain | 2.7D displacement (Gaea 3, unshipped) | VDM terrain + VDM erosion (experimental) | Unsupported | Partially supported |
| Heightmap export: EXR 32 / TIFF / PNG 16 / RAW 16/32 | EXR32, TIFF 16/32, PNG 8/16/32, RAW 16/32, PSD | EXR32, TIFF16, PNG 8/16, RAW 8/16/FP32, TER, BT, HFZ, PGM, Leveller | Available and verified | Available and verified |
| Mesh export | OBJ, FBX, DAE, glTF; adaptive (Pro) | OBJ, glTF/GLB, VDM; no FBX found | Available and verified | Partially supported |
| Point cloud export | PLY, XYZ | Not found | Available and verified | Unsupported |
| Splat / weightmaps | Unreal weightmaps, Data Maps | Weightmap device (packed RGBA, material ID) | Available and verified | Available and verified |
| PBR texture baking | TextureBaker, SatMaps, LightX | Albedo/normal/roughness/metal/AO, Lightmap | Available and verified | Available and verified |
| Tiled builds | Pro+, 256K, TileGate, blending, hybrid buckets | Pro, unlimited, blending %, naming template, resume, tiled input | Available and verified | Available and verified |
| Regions / partial rebuild | Build Regions (Pro+) | Render extents; tile subset export | Available and verified | Partially supported |
| Unreal plugin | Gaea2Unreal (MIT-ish, UE 5.3-5.6, World Partition, resync) | Export guide only (RAW16) | Available and verified | Partially supported |
| Unity | None (Gaea 3 planned); Resample helper | Export guide (RAW16 + flip) | Partially supported | Partially supported |
| Godot | None | RGB EXR height export | Unsupported | Partially supported |
| Houdini | Gaea2Houdini TOR Processor SOP (Pro+) | None | Available and verified | Unsupported |
| Blender | None (Gaea 3 planned) | None | Unsupported | Unsupported |
| USD | Gaea 3 (Pro+) | None | Unsupported | Unsupported |
| CLI / headless build | Gaea.Build.exe / Gaea.Swarm.exe with switches, vars, mutate, logs (Pro+) | XML scripts via exe argument; Console; `<var:name>` (Pro) | Available and verified | Available and verified |
| Machine-parseable build status | `--automata`, tab-separated buildlog | Build Status logs; stdout behavior undocumented | Available and verified | Requires additional investigation |
| Exit codes documented | No | No | Requires additional investigation | Requires additional investigation |
| Distributed / network builds | "Coming in 2.4" | None | Unsupported | Unsupported |
| Project file format | JSON `.terrain`, diff-friendly | `.tmd`, format unpublished | Available and verified | Requires additional investigation |
| Open-source components | Gaea2Unreal plugin | None | Partially supported | Unsupported |
| Published system requirements | Yes (AVX2, CUDA/HIP, 16-128 GB) | No current page | Available and verified | Requires additional investigation |
| Stability track record | 2.0 instability admitted; 2.2.7/2.3 stabilization | Dragontail VDM experimental; paging crash reports | Partially supported | Partially supported |

---

## Part D: Design lessons for Jenga Dunia

Concrete architectural implications derived from the evidence above. Each item cites which observed fact motivates it.

### D1. Node-graph evaluation model

1. **Adopt a DAG with explicit loop constructs, not graph cycles.** Both tools are strictly one-directional ("flows one way, from left to right, never the opposite"); Gaea expresses iteration as LoopBegin/LoopEnd. A DAG keeps dirty propagation and caching simple.
2. **Two-tier evaluation: cheap interactive preview plus explicit full-res build.** Both tools do this (Gaea 512-4K preview then Swarm build; WM interactive res plus multi-res cache plus Build). WM's LTE approach (retain results at every resolution from interactive up to a user cap, downsample high-res results back into the cache so preview matches final) is the more refined model and directly addresses the "erosion looks different at build res" complaint both vendors acknowledge. Recommendation: evaluation requests carry a target resolution; cache keyed by (node, params hash, upstream hash, resolution).
3. **Background, non-blocking refinement with memory backpressure.** WM's "refining preview" that prioritizes on-screen devices and "receives backpressure from memory usage" (4067), plus Gaea's async build commands (2.2.7), are responses to real UI-freeze complaints. In Rust: a worker pool with a cancellable job queue; UI thread never computes.
4. **Content-addressed, disk-spillable node cache with "bake/freeze" semantics.** Gaea's hard (baked) vs soft (session) caches, linchpin nodes that skip loading ancestors, invalidation on resolution change, and `--ignorecache` are the expected feature set. WM's "memory conservation" and page-to-disk are the equivalent. Store caches beside the project with a hash suffix as Gaea does; make them optional and safe to delete.
5. **Determinism as a first-class guarantee, with a documented cost.** Gaea's classic nodes are nondeterministic in multi-core mode and offer a "Deterministic (Slow)" single-core switch; Erosion_2 and WM Hurricane Ridge advertise deterministic output as a headline fix. Design simulations so that parallel execution is deterministic by construction (fixed tile decomposition, fixed reduction order, per-cell seeds hashed from (seed, x, y)); expose a global seed plus per-node seeds; support a "mutate" batch mode (Gaea `--mutate##`).
6. **Resolution-independent parameters in world units.** Both tools moved toward meters (Thermal_2 "Feature Scale in meters", WM km extents, "Channel Head Area" in km^2, "Talus Size 0m disables"). WM's unnormalized erosion masks across resolutions are a known pain point. Define every spatial parameter in meters and normalize mask outputs.
7. **32-bit float heightfields end to end**, with f16 for color/RGB packets as a memory optimization (WM does this, halving RGB memory).
8. **Typed ports with a small, decomposable type set.** WM users explicitly cite "decomposable data types" and "very simple, basic devices" as why they stay; Gaea's lack of a visible type system was called "a deal breaker." Recommend: Heightfield(f32), Mask(f32 0..1), Color(RGBA f16/f32), Water(depth + velocity vec2), IdGrid(u8/u32), Mesh/VDM, Scalar/Int/Bool/String/Color parameters with parameter ports that can be wired (WM's parameter-port rail). Refuse mismatched connections at connect time and show the type on the port.
9. **Graph ergonomics expected by pros:** portals/chokepoints and graph tabs (Gaea), groups, notes, bypass, pin, A:B compare node, "reveal in graph", wire slice, organize, simplified view when zoomed out (Gaea 2.0.4 did this for GPU cost), per-node build time and memory readout (WM Dragontail). Handle "hundreds to thousands of nodes."
10. **Macros as a central library with instance updating, versioning, and exposed parameters** (Gaea 2.3; WM versioned macros). Store as separate files so they are shareable and diff-able.
11. **A user-programmable node** is a differentiator. WM's Code device (Lua + Slang compute on Vulkan/Metal) ships today; Gaea only has MathX with SDK deferred to 3.0. For Jenga Dunia, a WGSL compute node (wgpu) plus a small expression language on parameters (Gaea-style expressions: "multiply height by 0.3", per-pixel x,y) would cover both.

### D2. Simulation design

12. **GPU first, CPU fallback, both deterministic.** Gaea's GPU erosion is its selling point ("fast because it's all GPU") and WM's lack of GPU is its top feature request. Gaea's lesson: GPU paths must gracefully fall back on VRAM exhaustion and must not be slower than CPU after fallback (2.0.x bug). wgpu gives Jenga Dunia Vulkan/Metal/DX12 portability that neither competitor has (Gaea is CUDA/HIP + AVX2, Windows-only).
13. **Expose the erosion data maps professionals rely on:** Flow, Wear, Deposition (both tools), Talus mask and Talus depth (WM), Snow depth, Lake/Depth/Shore masks (Gaea Lakes), water surface heightfield, water mesh with flow vectors (WM). These are consumed for texturing/splat generation; they are not optional.
14. **Sediment classes and orographic precipitation are current state-of-the-art knobs** (Erosion_2: suspended/bed/coarse loads with discharge angles; rain shadow by direction, altitude, slope, or mask). Mask/precipitation inputs on every simulation node are expected (Gaea Lakes Rain input, WM Precipitation and Hardness mask inputs).
15. **Thermal erosion = angle-of-repose talus with mass balance;** WM's "mass-movement-only" mode and "Talus Depth" output are cheap to add and useful.
16. **Hydrology stack: depression removal (Flow Restructure) -> drainage-area rivers (Create Water / Rivers) -> lakes -> coast.** Both tools converge on this pipeline; the ordering and the "drain to edge or existing water" option matter.
17. **Tile-aware simulation is hard and both vendors admit it** (Gaea TileGate/screen-space nodes, WM "can't see beyond the tile", Create Water not tile-safe). Design the simulation API with a halo/overlap parameter from day one, mark nodes as tile-safe or screen-space, and provide a Gaea-style gate node that bakes upstream at global resolution and upsamples per tile.
18. **Overhangs are the next frontier** (WM VDM shipped experimentally; Gaea 3 "2.7D"). Not a v1 requirement, but keep the heightfield type abstract enough that a VDM/mesh-displacement type can be added without rewriting the graph core.

### D3. Outputs professionals expect

19. **Heightmaps:** EXR 32-bit float (both call it the best option), TIFF 16/32, PNG 16, RAW r16 (little-endian, headerless, for Unreal/Unity) and RAW r32 float; explicit elevation mapping modes (Natural / Full Range / Custom min-max in meters) with the chosen range written to a sidecar; optional Y-flip (WM Flipper, Unity gotcha); RGB-packed EXR for Godot Terrain3D.
20. **Masks/splats:** per-mask 8/16-bit PNG, packed RGBA weightmaps with sum-to-one normalization (WM Weightmap), material-ID byte grids (WM Dragontail), Unreal weightmap layers.
21. **Meshes:** OBJ and glTF/GLB are the common denominator; FBX is a Gaea-only extra; adaptive/decimated meshes and point clouds are Pro features worth noting.
22. **Tiled export** with templated filenames (`%x %y %res`, zero-padding, 0/1 origin), shared-edge-vertex option, subset re-export, resume after failure, and incremental merge (WM); hybrid "bucket" builds within a RAM budget (Gaea).
23. **Engine bridges beat raw files.** Gaea2Unreal (open source, World Partition, re-sync) is the benchmark; WM only has guides. A Jenga Dunia Unreal/Unity/Godot bridge could be a thin importer that reads a manifest (JSON listing heightmap, scale, weightmaps, tiles) and is itself open source.
24. **Build logs as machine-readable data** (Gaea's tab-separated buildlog with per-node timings; WM's timestamped, sortable Build Status).

### D4. Automation APIs that matter

25. **A real headless CLI binary, separate from the GUI**, with: silent and machine-parseable (`--automata`) output modes, resolution override, cache ignore, output path override, seed mutation count, CPU limit, variable overrides as `name:value`, and a command to dump the graph's exposed parameters as JSON/XML (Gaea `--nodemap`). Document exit codes; neither competitor does.
26. **Declarative batch scripts** (WM's XML: load, modify extents/resolution/tiles, enable/disable groups, set vars, build normal|tiled, output; sections that fail independently) map naturally to a JSON/TOML job file for Jenga Dunia.
27. **Plain-text, diff-friendly project format with a published schema.** Gaea's JSON `.terrain` is explicitly marketed for source control; WM's `.tmd` is opaque. Publish the schema and keep it stable across minor versions; version node definitions so old projects "build identically" with opt-in upgrades (WM's per-device upgrade model).
28. **Variables and exposed parameters** are the bridge between graph and pipeline (Gaea2Houdini surfaces Gaea variables as SOP params; WM Automation Scalar binds XML vars). Make every parameter addressable by a stable path.
29. **Bridge/middleware story** (Gaea SwarmHost, background build service for Houdini). A local HTTP/IPC build service that DCCs can call would let Jenga Dunia be embedded in Houdini/Blender without a plugin per host.

### D5. Platform, performance, and licensing lessons

30. **Cross-platform is now table stakes.** WM just spent a release series on Mac/Linux; Gaea is locked to Windows plus AVX2 and is unusable on Apple Silicon even in Parallels. Rust/Tauri/wgpu gives Jenga Dunia native Windows/macOS/Linux at low cost; avoid hard CPU-feature requirements (runtime SIMD dispatch, not compile-time AVX2).
31. **Memory is the real limit, not resolution.** Both tools gate resolution by RAM and ship paging/backpressure fixes repeatedly. Budget memory explicitly: per-node size readouts, a global RAM cap, spill-to-disk caches, narrow-focus preview of one node.
32. **Stability over features.** Gaea 2.0's instability "held back many users from putting Gaea 2.0 into production"; WM's "LTS"/"Final" naming signals that users value stable lines. Keep a stable channel and an experimental channel.
33. **Licensing reference points for any future commercial tier:** free 1K non-commercial editions are the market norm; paid tiers at $99-$299 perpetual; feature gates on tiled builds, automation, and macro creation, not on simulation quality ("All editions include all terrain tools"). An open-source tool that ships tiled builds, automation, and cross-platform for free undercuts both on exactly the gated features.
34. **Documentation debt is visible to users.** Both vendors have stale/404 doc pages; Gaea has three doc hosts. Generate node reference docs from the node definitions themselves.

---

## Appendix: Source index (all accessed 2026-10-19)

Gaea (official):
- https://quadspinner.com/ (homepage, "$199. Pay once. Own forever.")
- https://quadspinner.com/Order (editions table)
- https://quadspinner.com/Gaea/Features (features, formats, 65 billion pixels, hybrid builds)
- https://docs.gaea.app/ , /reference , /ui , /using , /developers , /history
- https://docs.gaea.app/using/using-gaea/understanding-erosion/erosion_2/index.html
- https://docs.gaea.app/using/using-gaea/understanding-erosion/thermal_2.html
- https://docs.quadspinner.com/Guide/Build/Swarm.html (CLI)
- https://docs.quadspinner.com/Guide/Build/Build-Types.html , /Tiled.html , /Manager.html
- https://docs.quadspinner.com/Guide/Using-Gaea/Resolutions.html , /Cache.html , /FileFormats.html , /Erosion.html
- https://docs.quadspinner.com/Guide/Graph/Portals.html , /Graph.html
- https://docs.quadspinner.com/Guide/Interface/Getting-around.html
- https://docs.quadspinner.com/Reference/Erosion/Wizard.html , /Sediment.html
- https://docs.quadspinner.com/Reference/Water/Lakes.html
- https://help.quadspinner.com/article/9-what-are-the-system-requirements-for-gaea
- https://help.quadspinner.com/article/11-can-gaea-run-on-mac-or-linux
- https://help.quadspinner.com/article/68-why-does-gaea-2-require-a-native-windows-installation-to-operate
- https://help.quadspinner.com/article/42-are-my-gaea-1-files-compatible-with-gaea-2
- https://blog.quadspinner.com/tag/releases/ (release list)
- https://blog.quadspinner.com/gaea-2-2-released/ , /gaea-2-2-7-christmas-release/ , /gaea-2-1-preview/ , /gaea-204/ , /gaea2unreal-now-open-source/ , /advanced-features-variables-compute-and-expressions/
- https://eadocs.quadspinner.com/gaea-unreal-tools
- Pages that 404ed on fetch (content taken from search snippets only): quadspinner.com/pricing, /Gaea/Simulations, /Gaea/Build, /Gaea/Unreal, /Gaea/Houdini, /Gaea/Advanced, /Roadmap, /Download/Changelog, docs.gaea.app/reference/simulate, /reference/output, /reference/utility, /developers/automation/command-line-interface.

Gaea (third party):
- https://www.cgchannel.com/?p=176494 (Gaea 2.3, 2026-06-15)
- https://www.cgchannel.com/2025/12/quadspinner-unveils-gaea-3-0/
- https://www.cgchannel.com/2025/07/quadspinner-releases-gaea-2-2/
- https://digitalproduction.com/2025/07/18/gaea-2-2-unveils-frosty-nodes-and-selective-erosion/
- https://forum.winehq.org/viewtopic.php?p=130700 ; https://discuss.cachyos.org/t/has-anyone-gotten-gaea-to-work-on-linux/27050 ; https://www.codeweavers.com/compatibility/crossover/gaea
- https://polycount.com/discussion/228295/gaea-vs-world-machine-vs-world-creator-vs-instant-terra (403 on fetch; snippet)
- https://forum.world-machine.com/t/gaea-2-vs-worldmachine/7183
- https://www.sidefx.com/ja/docs/houdini/nodes/sop/labs--gaea_tor_processor.html
- https://glama.ai/mcp/servers/nghia-peirth/gaea-mcp/tools/connect_nodes (unofficial)

World Machine (official):
- https://www.world-machine.com/ ; /purchase.php ; /purchase.php?page=pro ; /features.php
- http://www.world-machine.com/version/dragontail
- https://www.world-machine.com/blog/2026/03/mac-world-machine/
- https://www.world-machine.com/blog/category/world-machine-development-news/dragontail-peak/
- https://help.world-machine.com/topics/releases ; /topics/alldevices/ ; /topics/natural-devices/
- https://help.world-machine.com/topic/build-4059-dragontail-peak/ ; /build-4065-dragontail-peak/ ; /build-4041-hurricane-ridge/
- https://help.world-machine.com/topic/world-machine-professional-edition-addendum/ (XML scripting, tiled builds)
- https://help.world-machine.com/topic/device-fileoutput/ ; /device-erosion/ ; /device-thermalerosion/ ; /device-create-water/ ; /device-river/ ; /device-splatmap/ ; /program-configuration/
- https://help.world-machine.com/topic/export-to-unity/ ; https://help.world-machine.com/?p=1106 (Unreal)
- https://forum.world-machine.com/t/build-4065-dragontail-peak/8510 ; /t/lte-feature-2/5070 ; /t/feb-updates/5921
- https://worldmachine.featureupvote.com/ (feature board; GPU acceleration "Under consideration")
- Pages not fetched/404: version-hurricane-ridge, Coastal Erosion device page, Snow device page, Checkpoint device page.

World Machine (third party):
- https://www.cgchannel.com/2026/07/world-machine-software-ships-world-machine-dragontail-peak/
- https://www.cgchannel.com/2026/05/world-machine-dragontail-peak-preview/
- https://www.cgchannel.com/2021/01/world-machine-build-4015-mt-rainier-ships/
- https://cgpress.org/archives/world-machine-for-mac-and-linux-announced.html
- https://windowsforum.com/news/world-machine-build-4065-adds-native-macos-linux-and-vdm-terrain.440477/
- https://forum.world-machine.com/t/gpgpu-enhancments-multiscreen/3177 ; /t/cellular-automata-for-hydraulic-erosion/3525 ; /t/unable-to-use-more-than-4-threads/6391 ; /t/q-a-world-machine-godot-terrain3d/8099 ; /t/4044-coast-erosion-water-level-bug/7797
- https://www.vrwiki.cs.brown.edu/vr-development-software/development-tutorials/world-machine (dated requirements)
