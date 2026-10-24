> Research note (2026-10-24) behind `docs/user/reading-a-landscape.md`, the MCP prompt
> `terrain_from_reference`, the resources `jd://guide/*`, `media.metadata`, `terrain.measure` and
> `jd.analysis.geomorphon@1`. **Changes after review:** the PTRM score is *not* shipped — Eq. 1 is
> verified, but its inputs are the paper's "normalized coverage" of each geomorphon, which cannot be
> plain fractions (they would make every score negative) and is defined only in the supplement;
> geomorphons classify by the sign of zenith + nadir elevation (WhiteboxTools' |nadir| vs |zenith|
> leaves an exactly straight line of sight flat; a perfect cone came out "shoulder"). Exemplar
> fingerprints were measured (table in the user guide).

# Reference photo → believable terrain: how humans do it, what research offers, what Jenga Dunia's MCP brain should automate

2026-10-24. Tags: **VERIFIED** (I read the source), **REPORTED** (secondary: search abstract, vendor page, someone else's summary), **INFERRED** (my own reasoning).

## 0. Bottom line

1. **A photo is a set of constraints, not a heightfield.** The parts you can rely on are the **skyline and silhouettes**, the **depth order** of ridges, the **landform type and the process that made it**, the materials, and the light. The parts you can't rely on are **absolute scale**, the back side of everything, and anything in plan view. The repo's own test shows this. MoGe-2 read a kilometre-wide valley as 5–19 m, and DA3 read a synthetic scene "an order of magnitude too close". Neither "recovers a landscape's scale from one picture" (`docs/research/research-log/2026-10-22-metric-depth.md`, **VERIFIED**). So the brain should **constrain what is visible and simulate the rest** with the existing erosion and climate nodes, steered by real DEMs of the same landform. (INFERRED)
2. **What we have today:** `vision.depth` writes an image-space relief, "row 0 = the top of the photograph … near = high" (`crates/jd-core/src/ops.rs`, `crates/jd-vision/src/lib.rs`, **VERIFIED**). That is a 2.5D picture, not a top-down terrain. Several things from `docs/architecture/13-geo-vision.md` have not been built: `photo_to_terrain`, `analyze_lighting`, `describe_scene` and `compare_render_to_reference`. The MCP server registers **tools only**: `ServerCapabilities::builder().enable_tools()`, with no prompts or resources. Its `with_instructions` text still only lists the project ops (`apps/mcp/src/main.rs`, **VERIFIED**). Prompts and resources were planned in `docs/architecture/07-agent-interfaces.md` (**VERIFIED**).
3. **Cheapest big wins:**
   - (a) An MCP prompt and resource that teaches the agent to read a photo the way a geomorphologist would, with a mapping from landform type to node chain.
   - (b) A `terrain.measure` op that returns a landform fingerprint: hypsometric integral, slope histogram, geomorphon fractions and the **PTRM** realism score.
   - (c) An exemplar library of real DEMs per landform type, fetched with the existing `geo.fetch_dem` and used as the realism target.

   None of these needs an ML model.

## 1. How human artists turn a reference into terrain

Honest caveat: I found **no primary source that documents a complete "geology-first, photo → terrain" workflow**. What practitioners publish is scattered across tool docs, talks and breakdowns. The checklist after the table is my synthesis (INFERRED), built on the sources in the table.

| Practice | Evidence |
|---|---|
| Start from **real elevation data** where you can, then art-direct. One desert breakdown "chose real world lidar data … instead of procedurally generating", using OpenTopography/USGS, Global Mapper → heightmap + colour map | 80.lv "Desert gas station environment production breakdown", https://80.lv/articles/desert-gas-station-environment-production-breakdown/ — **REPORTED** (search abstract) |
| Use the topography as a guide, break it up with brushes, then let erosion add realism ("much more visually appealing"); drive colour from height, slope and flow ("all done through simulation, not artistic talent") | Kilroy's Kartography, https://kilroys-kartography.com/realistic-fantasy-maps-an-overview/ — **VERIFIED** |
| Set erosion to the **real feature scale**: Gaea's Feature Scale is "the lateral size of the largest erosion features in meters: width of largest valleys and ridges" (default 2000 m); Real Scale ties erosion physics to the terrain's extents | https://docs.quadspinner.com/Reference/Erosion/Erosion.html — **VERIFIED** |
| AAA open worlds: hand-sculpted landforms plus procedural rules for biomes, cliffs, water and placement. Far Cry 5: Houdini tools for "biomes, … freshwater networks, … cliff rocks" | https://blog.playstation.com/2018/03/22/the-procedural-world-generation-of-far-cry-5/ ; GDC Vault 1025557 — **REPORTED** |
| Ghost of Tsushima: "Samurai Landscapes: Building and Rendering Tsushima Island on PS4" (Matthew Pohlmann, GDC 2021): in-engine tools and rule language for texture/placement; terrain artists hand-sculpted cliffs and rivers | https://gdcvault.com/play/1027352 (vault was under maintenance); ArtStation tsmith3d — **REPORTED** |
| Matte painting distance cues: haze grows, contrast and saturation fall and colour cools with distance; check scale against objects of known size (trees, buildings) | creativebloq / Adobe matte-painting articles — **REPORTED** (weak sources). Physically, this is aerial perspective (Koschmieder law) — INFERRED |

**The reading checklist (INFERRED, but every item maps to an existing node):**

1. **Landform and process.** Name the type (alpine/glacial, fluvial badlands, mesa/butte, dune field, karst, volcanic, coastal cliff) and the process behind it. This picks the erosion family: `erosion.stream_power`/`hydraulic`, `aeolian`, `yardang`, `karst`, `coastal`, `thermal`, plus `terrain.strata` and `filter.terrace` for caprock.
2. **Geology.** Look for bedding visible as terraces, and estimate its dip and dip direction (`strata.dip_deg/dip_azimuth_deg`). Hard caps over soft beds give cliffs and talus aprons. Read the talus angle from scree slopes (`thermal`).
3. **Scale.** Use trees (typically 10–40 m), people, roads and buildings. Estimate haze depth. Use focal length and sensor size from EXIF. The real answer comes from GPS plus a DEM.
4. **Climate and biome.** Find the tree line and snow line, read the vegetation density, and note any aridity signs. These set the parameters of `climate.snow`, `climate.rain` and `biome.classify`.
5. **Light.** Note the sun side, shadow length and time of day (`sky.bake`).
6. **What the photo cannot show:** the back sides, the true plan extent, terrain beyond the skyline, and the sub-surface. Artists fill these with a real DEM of a similar place, or with simulation that follows the same process.

## 2. Academic approaches

| Approach | Key work | What it gives us |
|---|---|---|
| Example-based patch synthesis guided by a sketched ridge/valley map | Zhou, Sun, Turk, Rehg, "Terrain synthesis from digital elevation models", IEEE TVCG 13(4) 2007. Extracts ridges/valleys with "Profile recognition and Polygon breaking", joins patches with graph cut + Poisson, ordered by a feature tree. https://faculty.cc.gatech.edu/~turk/my_papers/terrain_synth_tvcg.pdf — **VERIFIED** (PDF text) | Real-DEM patches carry realism; the ridges come from the photo |
| Sketch → terrain with conditional GANs (pix2pix); separate erosion and inverse (terrain→sketch) synthesisers | Guérin, Digne, Galin, Peytavie, Wolf, Benes, Martinez, ACM TOG 36(6) Art. 228, SIGGRAPH Asia 2017. https://www.cs.purdue.edu/homes/bbenes/papers/Guerin17ToG.pdf — **REPORTED** (abstract/listing; the PDF download was truncated) | Shows sketch-conditioned learning works; needs training data and weights we don't have |
| Diffusion terrain from sketches / text | Lochner et al. CGF 2023; Terrain Diffusion Network AAAI 2024; TerraFusion arXiv:2505.04050 (red/green/blue = valley/ridge/cliff sketches); Earthbender MIG 2025; MESA arXiv:2504.07210 (Copernicus) — **REPORTED** | Possible future generator; weights' licences unknown |
| VAE+cGAN terrain latent space | Naik et al. arXiv:2201.02369 (2022) — **VERIFIED** (abstract) | Same |
| Perspective (first-person) sketch → terrain edit, matching sketched silhouettes to terrain silhouettes and ridges | Tasse, Emilien, Cani, Hahmann, Dodgson, GI 2014, https://hal.archives-ouvertes.fr/hal-00976689; Gain/Marais "Terrain sketching" I3D 2009 — **REPORTED** | **The closest published analogue to "make the terrain's skyline match the photo's"** |
| Photo ↔ DEM alignment by silhouette-edge matching (needs viewpoint + FOV estimate; robust to clouds, snow, forest) | Baboud, Čadík, Eisemann, Seidel, CVPR 2011, DOI 10.1109/CVPR.2011.5995727 — **REPORTED** (abstract) | Recovers camera pose and true scale when GPS is present |
| Geo-localising mountain photos without GPS: "contour words" from DEM skylines, 40,000 km² of Switzerland, >200 queries → >1000 in IJCV 2016 | Baatz, Saurer, Köser, Pollefeys, ECCV 2012 LNCS 7573:517–530; Saurer et al. IJCV 116(3) 2016 — **REPORTED** | Too heavy for us; confirms skylines are the strongest signal |
| Desert geo-localisation: user traces the skyline, matched against concavity features of DEM-rendered skylines; 44 images, often ~100 m | Tzeng, Zhai, Clements, Townshend, Zakhor, CVPRW 2013 — **REPORTED** | Relevant to the desert mandate: tracing the skyline by hand is a valid fallback |
| Benchmark: 3K+ mountain photos with GPS, orientation, FOV, depth, normals, semantic labels | Brejcha & Čadík, GeoPose3K, Image Vis. Comput. 2017, https://cphoto.fit.vutbr.cz/geoPose3K/ — **VERIFIED** (manuscript; the manuscript itself is CC-BY-NC-ND, dataset terms not checked) | Internal evaluation set only; do not ship it |
| Landform classification from a DEM: 498 ternary patterns → 10 forms (flat, peak, ridge, shoulder, spur, slope, hollow, footslope, valley, pit); parameters: search distance, skip, flatness angle | Jasiewicz & Stepinski, Geomorphology 182:147–156 (2013); Esri docs https://pro.arcgis.com/en/pro-app/3.5/tool-reference/spatial-analyst/how-geomorphon-landforms-works.htm — **VERIFIED** (Esri page + PTRM paper text) | A landform fingerprint for both the target DEM and our output |
| **Perceived realism of a DEM from geomorphon fractions** (linear regression on a user study, 72 % of variance explained, SRTM at ~200 m/px; real Arizona = 0.76, thermal-eroded synthetic = 0.51) | Rajasekaran, Kang, Čadík, Galin, Guérin, Peytavie, Slavík, Beneš, "PTRM", ACM TAP 19(2) Art. 6, 2022, **CC BY 4.0**, https://www.cs.purdue.edu/cgvlab/www/resources/papers/Rajasekaran-2022-TAP-Perceived_Terrain_Realism_Metric.pdf — **VERIFIED**. Eq. 1: `PTRM = (−38.02 + 3.55·G_depr + 1.75·G_peak + 25.12·G_flat + 9.61·G_valley + 7.59·G_ridge + 6.71·G_hollow + 9.02·G_spur + 7.31·G_shoulder + 28.95·G_slope + 7.63·G_footslope)/69.96` | **A ready-made, citable realism check** |
| Survey of the field | Galin et al., "A Review of Digital Terrain Modeling", CGF 38(2) 2019 (EG STAR), https://www.cs.purdue.edu/homes/bbenes/papers/Galin19CGF.pdf — **REPORTED** | Background reading |
| Terrain super-resolution from DEM + orthophoto (FCN) | Argudo, Chica, Andújar, CGF 37(2) 2018 — **REPORTED** | Not single-photo; I found no Argudo "single-image orography" paper |
| Generative unbounded landscapes from photo collections (terrain feature grid + skydome) | Chai et al., "Persistent Nature", CVPR 2023 — **REPORTED** | Research only; the output is a radiance field, not an editable DEM |
| Sun and illumination from one outdoor photo (sky, shadows, vertical surfaces, ground, plus a data prior) | Lalonde, Efros, Narasimhan, ICCV 2009 / IJCV 2012 — **REPORTED**; Hold-Geoffroy CVPR 2017 (already in geo-vision.md) | The "light" part of the reading |

**Limits of monocular depth at landscape scale:**
- Sky is masked: MoGe and DA3 both output a mask.
- Distant ridges have almost no parallax, and the metric heads were trained on scenes with scale cues (repo log, **VERIFIED**).
- Haze carries depth (transmission t = e^(−βd)). The dark-channel prior (He, Sun, Tang, CVPR 2009, **REPORTED**) gives *relative* depth exactly where the networks fail: far ridges. (INFERRED)

## 3. What to automate and what needs a human

| Task | Automate? | How |
|---|---|---|
| Landform, process, biome, season, materials, scale objects | **Yes**, through the MCP client's own vision model | Structured "reading sheet" (§4 item 1). Repo policy already says do not bundle CLIP (`geo-vision.md` B2/B5) |
| Skyline / horizon extraction | **Yes** | Top edge of the MoGe/DA3 sky mask, per column; ridge silhouettes from depth discontinuities |
| Scale | **With GPS: yes** (DEM alignment). Without: **estimate plus confirm** | EXIF focal + sensor → FOV (the parallel agent covers tags); known-size objects; haze. Ask the user when the uncertainty exceeds ~2× |
| Camera pose and DEM retrieval when GPS is present | **Yes** | `geo.fetch_dem` around the GPS point; search yaw/pitch/roll/FOV to match the skyline (Baboud-style). A 360° panorama makes the yaw search unambiguous (INFERRED) |
| Geo-locating with no GPS | **No** (research-grade, Baatz/Saurer) | Ask the user for a place name, then `geo.fetch_dem` |
| Sun direction | **Yes** with lat/lon/time (solar position); else shading/shadow heuristics plus the VLM | Feeds `sky.bake` and `climate.snow.sun_azimuth_deg` |
| Snowline and treeline height | **Yes** from latitude + climate | Lapse rate ~6.5 K/km; treeline at a growing-season mean air temperature of ≈ 6.4 °C (Paulsen & Körner 2014, revising Körner & Paulsen 2004's 6.7 °C soil temperature, J. Biogeogr. 31:713 — **REPORTED**); Köppen class (Beck et al. 2018, Sci. Data 5:180214, gloh2o.org/koppen — **REPORTED**, licence to confirm) |
| Picking erosion presets from the landform type | **Yes** | Lookup table (resource) plus fingerprint comparison |
| Render from the photo's camera → compare → adjust (analysis by synthesis) | **Yes**, within budgeted iterations | Needs `terrain.render_view` + `vision.compare_view` (§4) |
| The back side, terrain beyond the skyline, creative intent ("more dramatic") | **Human** | The agent presents 2–3 variants scored by the metrics |

## 4. Proposal for Jenga Dunia, ranked

Every new op should return `provenance[]` and a `confidence`, per the existing design.

**1. MCP prompt `terrain_from_reference` + resource `jd://guide/reading-a-landscape`. CHEAP WIN.**
- *What:* Enable `prompts` and `resources` in `apps/mcp/src/main.rs` (rmcp supports both; the capability flags are INFERRED from rmcp's builder). The prompt walks the agent through the steps:
  1. Fill a **reading sheet** in JSON: `landform`, `process[]`, `rock{type, strata_dip_deg, dip_azimuth_deg, talus_deg}`, `climate{koppen_guess, aridity}`, `biome`, `treeline_visible`, `snowline_visible`, `scale_objects[{kind, px_height}]`, `haze`, `sun{side, elevation_guess}`, `season`, `unknowns[]`, each with a confidence.
  2. Get metadata (the parallel agent's work).
  3. If GPS is known, fetch the DEM; otherwise pick an exemplar.
  4. Build the node chain from the resource's table.
  5. Measure, render and compare.
  6. Report what was assumed.

  The resource holds the §1 checklist and a landform → node-chain table. Two example rows:
  - mesa → `noise.fbm` → `terrain.strata`(dip ≈ 0) → `filter.terrace` → `erosion.thermal`(talus from the sheet) → `erosion.stream_power`
  - barchan field → `erosion.aeolian`(single wind, low sand supply) → `climate.*` dry

  Also fix the stale server instructions.
- *Model:* none; the client's VLM does the reading. *Effort:* S (1–2 days).
- *Verify:* 20 labelled photos covering each landform family (deserts too). The sheet must validate against its JSON schema, and the landform class must agree with a human label at ≥ 80 %.

**2. `terrain.measure` (or extend `terrain.stats`). CHEAP WIN.**
- *Outputs:*
  - Hypsometric integral. Computable today from `HeightfieldStats{min, max, mean}` as (mean − min)/(max − min), the elevation-relief ratio (Pike & Wilson 1971, **REPORTED**).
  - Slope histogram (Horn 1981, already used).
  - Relief.
  - Drainage density from `analysis.flow`.
  - **Geomorphon fractions**, implemented from the paper. GRASS/SAGA are GPL, so study them only. WhiteboxTools (MIT) can serve as a test oracle.
  - **PTRM score**: resample to ~200 m/px, then apply Eq. 1. The PTRM paper is CC BY, so cite it.
- *Effort:* S–M.
- *Verify:*
  - Geomorphon class maps match WhiteboxTools on a Copernicus tile at ≥ 95 % of cells.
  - PTRM ranks real `geo.fetch_dem` tiles above raw `noise.fbm`, and erosion raises the score. Expect, but don't hard-code, values around 0.5 for synthetic and around 0.7 for real terrain (the paper's ranges).

**3. Landform exemplar library: `jd://exemplars` resource + `geo.fetch_dem`. CHEAP WIN.**
- *What:* A curated list of 2–3 public DEM boxes per landform type, for example:
  - Namib barchans, Rub' al Khali linear dunes
  - Monument Valley mesas
  - Guilin tower karst
  - Bernese Alps
  - Badlands SD
  - Atacama playas
  - Iceland shield volcanoes

  Copernicus GLO-30 through the existing ADR-0014 fetch. The agent fetches the exemplar that matches the reading sheet, measures it, and uses that fingerprint as the target. The exemplar can also be the base for a "real DEM base + art-direct" workflow, as artists do.
- *Effort:* S (a list plus attribution).
- *Verify:* After tuning, the 1-D Wasserstein distance of the slope histogram, the L1 distance of geomorphon fractions and |ΔHI| between output and exemplar fall under set thresholds. Run in CI on a small fixture.

**4. `geo.climate_priors(lat, lon, elevation_range)`.**
- *Outputs:* Köppen class, mean temperature, lapse rate, treeline_m, snowline_m, dominant wind, aridity. These map straight onto `climate.snow.snow_line_m`, `climate.rain.temperature_c/lapse_c_per_km`, `aeolian.wind_azimuth_deg` and `biome.classify` rules.
- *Data:* Beck 2018 Köppen raster (check its licence before bundling a downsampled copy) or a coarse rule table. Treeline from the 6.4 °C rule plus the lapse rate.
- *Effort:* S–M.
- *Verify:* Predicted treeline within ±200 m of published values for 5 ranges (Alps ~2000–2300 m, Rockies, Andes, Kilimanjaro, Scandes). INFERRED tolerance.

**5. `vision.photo_to_terrain`: the top-down reprojection already designed in geo-vision.md B1.**
- *What:* Points from `vision.depth` (MoGe gives camera points) → RANSAC ground plane → orthographic rasterise → heightfield plus a confidence/visibility mask, with range capped (~200 m). Add **`vision.skyline`**: the per-column sky boundary as an angle profile, plus ridge silhouette layers from depth discontinuities. Beyond the depth cap, use haze ordering (dark-channel transmission) to rank far ridges into distance bands.
- *Chains:* `jd.input.heightmap` → its mask drives a blend with an exemplar or erosion result, so the model fills only what it saw.
- *Effort:* M.
- *Verify:* The repo already renders a known-FOV Blender scene (`fovcheck/render.py` in the metric-depth log). Render scenes from known heightfields and measure height RMSE inside the visible mask and skyline angular RMS (< 0.5°).

**6. `terrain.render_view` + `vision.compare_view`: the analysis-by-synthesis loop.**
- *What:* Ray-march the heightfield from a camera (pos, yaw, pitch, roll, fov) → depth image, skyline profile and silhouette edges, all deterministic on the CPU. Compare with the photo and return:
  - **skyline IoU** (sky masks)
  - **skyline RMS angle error**
  - **depth-ordinal agreement** (Kendall τ over sampled pixel pairs; scale-free, so robust to monocular scale error)
  - SSIM and ΔE of the existing `terrain.textures` render against the photo (later)
- The agent then adjusts `graph.set_param` (ridge height, strata dip, erosion strength) or adds `sculpt.strokes` / `terrain.spline` along a mismatched ridge, and re-renders. This is Tasse 2014's idea driven by an agent. A `graph.fit` op that runs a CMA-ES / coordinate search over 2–6 chosen parameters can come later. Skip it until hand-driven loops prove too slow.
- *Effort:* M.
- *Verify:* Fit a perturbed known terrain back to its own render; skyline IoU should climb from < 0.8 to > 0.95 within N iterations.

**7. `geo.align_photo` (needs GPS).**
- *What:* Fetch the DEM around the GPS point (`geo.fetch_dem`). Render a 360° skyline from that point with item 6's renderer. Find yaw by circular cross-correlation with the photo's skyline, then refine pitch/roll/FOV with a small grid search (Baboud 2011 idea, simplified). Output the camera pose plus the **true metric scale**. Then rescale `vision.depth`, or replace it with DEM depth. GPano panoramas give the whole 360° profile and skip the FOV search (INFERRED).
- *Effort:* M.
- *Verify:* GeoPose3K as an internal test set only (licence NC): median yaw error < 2°, skyline RMS < 1°, against the paper's ground truth.

**8. Optional on-device classifier for clients without vision.**
- SigLIP 2 (`google/siglip2-base-patch16-224`, Apache-2.0, **VERIFIED** via the HF API) through `ort` for zero-shot landform/biome labels.
- OWLv2 or Grounding-DINO-tiny (Apache-2.0, **VERIFIED** via the HF API) to find scale objects.
- SAM 2 (Apache-2.0, **VERIFIED** via the HF API) to segment rock, sand, vegetation and snow masks for material matching.
- *Effort:* M each. *Priority:* low, because the MCP client already has vision and repo policy avoids bundling.

**9. Defer: learned sketch/photo → DEM generators** (Guérin 2017 cGAN, the diffusion work).
- *Reasons:* No permissive weights. Training needs a curated DEM corpus. Results are hard to make deterministic (ADR-0007).
- *Revisit when:* Items 2, 3 and 6 show that procedural chains can't reach the exemplar fingerprints.

### Order and effort

| Rank | Item | Effort | Model / licence | Cheap win |
|---|---|---|---|---|
| 1 | Prompt + reading-sheet resource | S | none | yes |
| 2 | `terrain.measure` (HI, slope histogram, geomorphons, PTRM) | S–M | paper-derived (CC BY paper) | yes |
| 3 | Exemplar library | S | Copernicus data (existing ADR) | yes |
| 4 | `geo.climate_priors` | S–M | Köppen raster (licence to confirm) | partly |
| 5 | `photo_to_terrain` + `skyline` | M | MoGe-2 MIT / DA3 Apache (existing) | |
| 6 | `render_view` + `compare_view` | M | none | |
| 7 | `geo.align_photo` | M | none | |
| 8 | Optional local classifiers | M | SigLIP2 / OWLv2 / SAM2 Apache-2.0 | |
| 9 | Learned generators | L | unknown | defer |

All of the code paths above are permissive or plain algorithms from papers. Nothing here pulls GPL (GRASS, SAGA, RichDEM) or NC weights into `deny.toml`'s allow-list.

## 5. Realism checks the agent should run every time

These come from items 2, 3 and 6:

- (i) skyline IoU and RMS angle against the photo
- (ii) depth-ordinal τ against `vision.depth`
- (iii) slope-histogram Wasserstein distance and geomorphon L1 distance against the exemplar DEM of the same landform
- (iv) |ΔHI| against the exemplar
- (v) a PTRM floor (for example ≥ 0.6, INFERRED; calibrate on exemplars)
- (vi) climate consistency: no forest above the computed treeline, no snow below the snowline except in shaded north faces

The agent should report every assumption from the sheet's `unknowns[]` back to the user.

## Sources

- Repo: `docs/research/geo-vision.md`, `docs/research/materials-geomorphology.md`, `docs/research/research-log/2026-10-22-metric-depth.md`, `docs/architecture/07-agent-interfaces.md`, `docs/architecture/13-geo-vision.md`, `apps/mcp/src/main.rs`, `crates/jd-vision/src/lib.rs`, `crates/jd-core/src/ops.rs`, `schemas/v1/graph/node-kinds.json`, `deny.toml`
- PTRM: https://www.cs.purdue.edu/cgvlab/www/resources/papers/Rajasekaran-2022-TAP-Perceived_Terrain_Realism_Metric.pdf
- Zhou 2007: https://faculty.cc.gatech.edu/~turk/my_papers/terrain_synth_tvcg.pdf
- Guérin 2017: https://www.cs.purdue.edu/homes/bbenes/papers/Guerin17ToG.pdf ; https://hal-udl.archives-ouvertes.fr/hal-01583706/
- Galin 2019: https://www.cs.purdue.edu/homes/bbenes/papers/Galin19CGF.pdf
- Geomorphons: https://pro.arcgis.com/en/pro-app/3.5/tool-reference/spatial-analyst/how-geomorphon-landforms-works.htm
- Baboud 2011: https://resources.mpi-inf.mpg.de/photo-to-terrain ; https://mlanthology.org/cvpr/2011/baboud2011cvpr-automatic
- Baatz 2012: https://mlanthology.org/eccv/2012/baatz2012eccv-large ; Saurer 2016 DOI 10.1007/s11263-015-0830-0
- Tzeng 2013: https://openaccess.thecvf.com/content_cvpr_workshops_2013/W07/html/Tzeng_User-Driven_Geolocation_of_2013_CVPR_paper.html
- GeoPose3K: https://cphoto.fit.vutbr.cz/geoPose3K/
- Tasse 2014: https://hal.archives-ouvertes.fr/hal-00976689
- Diffusion terrain: https://arxiv.org/pdf/2505.04050 ; https://ojs.aaai.org/index.php/AAAI/article/view/29150 ; https://cgvr.cs.uni-bremen.de/papers/mig2025/earthbender-paper.pdf ; https://arxiv.org/abs/2201.02369
- Argudo 2018: https://diglib.eg.org/handle/10.1111/cgf13345
- Persistent Nature: https://openaccess.thecvf.com/content/CVPR2023/html/Chai_Persistent_Nature_A_Generative_Model_of_Unbounded_3D_Worlds_CVPR_2023_paper.html
- Lalonde 2009: https://ri.cmu.edu/publications/estimating-natural-illumination-from-a-single-outdoor-image
- Treeline: https://edoc.unibas.ch/8594 ; Köppen: https://pmc.ncbi.nlm.nih.gov/articles/PMC6207062
- Gaea erosion: https://docs.quadspinner.com/Reference/Erosion/Erosion.html ; Kilroy: https://kilroys-kartography.com/realistic-fantasy-maps-an-overview/ ; 80.lv: https://80.lv/articles/desert-gas-station-environment-production-breakdown/
- Far Cry 5: https://blog.playstation.com/2018/03/22/the-procedural-world-generation-of-far-cry-5/ ; Tsushima: https://gdcvault.com/play/1027352/Samurai-Landscapes-Building-and-Rendering
- HF licences (API): google/siglip2-base-patch16-224, google/owlv2-base-patch16-ensemble, IDEA-Research/grounding-dino-tiny, facebook/sam2-hiera-small
