# Jenga Dunia — Algorithm Reference for Procedural Terrain & Environment

Research analyst report. Accessed/verified 2026-10-19.
Target: open-source Rust tool, CPU-first (rayon) with optional wgpu compute, 4k–8k heightfields under ~4 GB RAM via tiling.

## Citation legend

- **[V]** — bibliographic record resolved online on 2026-10-19 (ACM DL / Wiley / EG DigLib / arXiv / HAL / author page / crates.io).
- **[M]** — standard citation; details from background knowledge, DOI not independently resolved today. Treat DOI as probable, confirm before publishing in docs.
- **UNVERIFIED — could not locate** — searched; no record found. Do not cite as fact.
- Complexity notation: N = number of cells (W×H), T = iterations, P = particles, K = octaves.

Phases referenced throughout: **P2** core terrain, **P3** simulation, **P4** interop/optimization, **P5** scale.

---

## 1. Noise & base synthesis

### 1.1 Established algorithms

| Algorithm | Canonical source | Complexity / sample | GPU | Determinism | Pitfalls |
|---|---|---|---|---|---|
| Perlin gradient noise | Perlin, K. 1985. "An Image Synthesizer." SIGGRAPH '85 / Computer Graphics 19(3). DOI 10.1145/325165.325247 [M] | O(2^d) corner lookups (4 in 2D, 8 in 3D) | Excellent (pure ALU, permutation table) | Fully deterministic given permutation seed | Axis-aligned artifacts (zeros on the integer lattice; visible "grid" at low octave counts), original cubic fade has C1 discontinuity |
| Improved Perlin | Perlin, K. 2002. "Improving Noise." SIGGRAPH 2002 / ACM TOG 21(3): 681–682. DOI 10.1145/566654.566636 [M] | Same | Excellent | Same | Quintic fade (6t^5-15t^4+10t^3) fixes C2; 12-gradient set reduces directional bias but grid alignment remains |
| Simplex noise | Perlin, K. 2001/2002 (SIGGRAPH 2001 course notes "Real-Time Shading"); clearest exposition: Gustavson, S. 2005. "Simplex noise demystified" (Linköping report, https://weber.itn.liu.se/~stegu/simplexnoise/simplexnoise.pdf) [M]. Patent: US 6,867,776 B2 "Standard for Perlin noise", Google Patents status **"Expired – Lifetime"**, anticipated expiration **2022-01-08** [V] (https://patents.google.com/patent/US6867776B2/en) | O(d+1) corners (3 in 2D, 4 in 3D) — scales O(d²) vs O(2^d) | Excellent | Deterministic | Patent concern is gone (expired Jan 2022; patent covered only 3D+ anyway). Still shows mild 60° lattice artefacts; the original 2D simplex has visible directional streaking at some frequencies |
| OpenSimplex2 / OpenSimplex2S | KdotJPG, https://github.com/KdotJPG/OpenSimplex2 (public domain / CC0). Rust: crate `opensimplex2` v1.1.0, co-owned by KdotJPG [V] (https://docs.rs/opensimplex2); also `fastnoise-lite` port (Perlin, OpenSimplex2/2S, cellular, value, domain warp, fractal) [V] (https://docs.rs/fastnoise-lite) | Same order as simplex; 2S slightly slower (smoother kernel) | Excellent | Deterministic | Rotated lattice ("ImproveXY") reduces visible lattice; no patent history |
| Value noise / cubic | folklore (Peachey, in *Texturing & Modeling*) | Cheapest | Excellent | Deterministic | Blobby, low-quality; use only for cheap masks |
| fBm (fractional Brownian motion) | Mandelbrot & Van Ness 1968 (math); for graphics: Musgrave, Kolb, Mace 1989 (below); *Texturing & Modeling: A Procedural Approach* (Ebert, Musgrave, Peachey, Perlin, Worley), 3rd ed. 2003, Morgan Kaufmann [M] | O(K) base evaluations | Excellent | Deterministic | Lacunarity 2.0 exactly with integer-lattice noise produces octave-aligned artefacts; use 1.9–2.1 or add per-octave rotation/offset. Amplitude normalization: sum of gains → normalise to [-1,1] |
| Ridged multifractal / hybrid multifractal / heterogeneous terrain | Musgrave, F.K., Kolb, C.E., Mace, R.S. 1989. "The synthesis and rendering of eroded fractal terrains." SIGGRAPH '89 / Computer Graphics 23(3): 41–50. DOI 10.1145/74334.74337 [M]. Fuller treatment in *Texturing & Modeling* ch. 16 ("Procedural Fractal Terrains") [M] | O(K) | Excellent | Deterministic | Ridged = 1-|noise| then square; exponent (H) and offset parameters are sensitive; multifractal weighting must be clamped [0,1] or it blows up |
| Domain warping | Quilez, I. "Domain Warping." https://iquilezles.org/articles/warp/ (2002/2012, web article) [M]. Formal cousin: Perlin & Hoffert 1989 "Hypertexture" (DOI 10.1145/74334.74359) [M] | O(K·(1+warp levels)) | Excellent | Deterministic | Stacked warps create "swirly" plasticine look if amplitude too high; derivatives of warped noise need chain rule (needed for analytic normals/slope) |
| Voronoi / Worley (cellular) | Worley, S. 1996. "A cellular texture basis function." SIGGRAPH '96, 291–294. DOI 10.1145/237170.237267 [M] | O(3^d) cell scan per sample | Excellent | Deterministic (hash per cell) | F1/F2 distance variants; Manhattan/Chebyshev metrics give different looks; jitter<1 for regularity. Gives plateau/terrace masks, cracked mud, dry lakebeds |
| Spectral synthesis (1/f^β, Fourier filtering) | Saupe, D. 1988. "Algorithms for random fractals." In Peitgen & Saupe (eds.), *The Science of Fractal Images*, Springer, 71–136 [M]. Also Voss 1985 | O(N log N) FFT | Good (FFT on GPU) | Deterministic given seeded phases | Inherently periodic (seamless tiles for free); global Gaussian statistics, no local control; β≈2H+1 relates to Hurst exponent H; ringing if spectrum truncated sharply |
| Diamond-square (midpoint displacement) | Fournier, A., Fussell, D., Carpenter, L. 1982. "Computer rendering of stochastic models." CACM 25(6): 371–384. DOI 10.1145/358523.358553 [M]. Diamond-square refinement: Miller, G. 1986. "The definition and rendering of terrain maps." SIGGRAPH '86 [M] | O(N) | Awkward (sequential level dependency) | Deterministic given seed order | **Creasing / axis-aligned artefacts** (Miller 1986 documented them), square-grid "ridges" along subdivision lines, non-stationary statistics; mostly historical — prefer fBm or spectral |
| Gabor noise (sparse convolution) | Lagae, A., Lefebvre, S., Drettakis, G., Dutré, P. 2009. "Procedural noise using sparse Gabor convolution." ACM TOG 28(3), art. 54. DOI 10.1145/1531326.1531360 [V] | O(impulses per cell · 3^d) — ~10× costlier than Perlin | Good | Deterministic (PRNG per cell) | Full spectral control (anisotropy, band-limited); overkill for base relief, excellent for anisotropic dune/stria detail |
| Phasor noise | Tricard et al. 2019. "Procedural phasor noise." ACM TOG 38(4). DOI 10.1145/3306346.3322990 [V] | Like Gabor | Good | Deterministic | Sharp oscillating profiles (strata, ripples) without blur |
| "Uber noise" | Hello Games / Sean Murray, GDC 2017 talk "Building Worlds in No Man's Sky Using Math(s)" (GDC Vault) [M]. Informal: a single fBm-style loop combining ridged, billowed, domain-warped, and slope-erosion-like terms with per-octave parameters | O(K) | Excellent | Deterministic | No paper; a design pattern, not an algorithm. Treat as a node preset built from the primitives above |
| Non-periodic tiling of noise | Heitz & Neyret style; "Non-periodic tiling of procedural noise functions" (Wyvill-style), PACMCGIT 2018, DOI 10.1145/3233306 [V] | — | — | — | Useful if noise must be baked to texture tiles |

### 1.2 Tiling / seamless / periodic variants
- Lattice noises are made periodic by wrapping lattice coordinates modulo period before hashing (`fastnoise-lite` and `noise` crate expose this). Period must be integer in lattice units; fBm needs each octave period = base × lacunarity^k → use lacunarity 2 exactly for tileable output (trade-off with artefact note above).
- Spectral synthesis is periodic by construction.
- Alternative: sample 2D noise on a 4D torus embedding (two circles) — exact seamlessness, double the cost.
- For planets (Area 9) use 3D noise sampled on the sphere; no seams by construction.

### 1.3 Analytic derivatives
Compute dnoise/dx, dnoise/dy alongside value (Quilez, "Value noise derivatives"/"Gradient noise derivatives", iquilezles.org [M]). Enables: slope-damped fBm ("erosion-like" look, popularized in Quilez's terrain shaders and Hello Games' uber noise), analytic normals, cheap flow direction prior for erosion seeding.

### 1.4 Recommendation (P2)
- **Primary base synthesis: OpenSimplex2S (or Improved Perlin for compatibility) + fBm/ridged/hybrid multifractal + domain warp**, all with analytic derivatives, f32.
- Use `fastnoise-lite` (ports of KdotJPG's reference; MIT) or `opensimplex2` (CC0) crate for the kernel, wrap in our own fBm/warp nodes so the GPU WGSL port is a straight transliteration.
- **Spectral synthesis** as a secondary node (P3/P5) for large-scale seamless base relief and for planet-scale 1/f^β.
- **Worley** as a mask generator (P2).
- **Gabor/phasor**: defer (P5), only for aeolian ripple detail.
- **Diamond-square**: do not implement (artefacts, poor parallelism); document why.
- Expose: seed (u64), frequency, octaves, lacunarity, gain, H/exponent, offset (ridged), warp amplitude/frequency, period (optional), derivative output.
- Determinism: integer hashing (e.g. PCG/xxhash-style on cell coordinates) rather than a 256-entry permutation table is preferable on GPU (no table upload, no cache miss), and gives identical CPU/GPU results as long as the hash is integer-only and float math stays within IEEE-754 basic ops (see §16).

---

## 2. Hydraulic erosion

### 2.1 Families and canonical papers

**(a) Cellular, heuristic (Musgrave-style)**
- Musgrave, Kolb, Mace 1989 (DOI 10.1145/74334.74337) [M] — first hydraulic + thermal erosion on heightfields; per-cell water/sediment passed to lower neighbors. O(N·T). Simple, deterministic, GPU-friendly (Jacobi-style). Looks crude; no inertia; over-smooths.
- Olsen, J. 2004. "Realtime Procedural Terrain Generation: Realtime Synthesis of Eroded Fractal Terrain for Use in Computer Games." Tech report, IMADA, Univ. of Southern Denmark, 31 Oct 2004. https://web.mit.edu/cesium/Public/terrain.pdf [V] — simplified Musgrave thermal + hydraulic for games, speed-optimised variants, discusses the "ridge-preserving" trick.

**(b) Layered material representation**
- Beneš, B., Forsbach, R. 2001. "Layered data representation for visual simulation of terrain erosion." SCCG 2001, 80–86. DOI 10.1109/SCCG.2001.945341 [V] — stratified horizontal layers per cell (material + thickness), trade-off between heightfield and voxels.
- Beneš, B., Forsbach, R. 2002. "Visual simulation of hydraulic erosion." Journal of WSCG 10(1–3): 79–86 (page range varies by source) [V] — four-step model (rain, erosion, transport, evaporation) on the layered structure.

**(c) Grid shallow-water / "virtual pipes" (Eulerian)**
- Mei, X., Decaudin, P., Hu, B.-G. 2007. "Fast Hydraulic Erosion Simulation and Visualization on GPU." Pacific Graphics 2007, 47–56. DOI 10.1109/PG.2007.15. Project page: http://www-evasion.inrialpes.fr/Publications/2007/MDH07/ [V] — shallow-water via virtual pipe model (O'Brien & Hodgins 1995 pipes), velocity-field-driven sediment capacity, semi-Lagrangian sediment advection, all GPU. *The* reference for grid erosion. [V: PDF obtained 2026-10-21 via the Wayback Machine, equations checked against the implementation — see research-log/2026-10-21-follow-ups.md]
- Šťava, O., Beneš, B., Brisbin, M., Křivánek, J. 2008. "Interactive terrain modeling using hydraulic erosion." SCA 2008, 201–210. DOI 10.2312/SCA/SCA08/201-210 [V] — extends Mei: multi-layer materials, force-based erosion, dissolution + sediment slippage, GPU, ≥20 fps at 2048×1024 with 4 layers. Includes thermal/talus for each layer.
- Krištof, P., Beneš, B., Křivánek, J., Šťava, O. 2009. "Hydraulic Erosion Using Smoothed Particle Hydrodynamics." CGF 28(2): 219–228 (EG 2009). DOI 10.1111/j.1467-8659.2009.01361.x [V] — SPH fluid + donor-acceptor erosion; handles 3D/overhang; expensive.

**(d) Particle / droplet (Lagrangian)**
- Chiba, N., Muraoka, K., Fujita, K. 1998. "An erosion model based on velocity fields for the visual simulation of mountain scenery." J. Visualization & Computer Animation 9(4): 185–194 [M; not located in today's search, cited widely as the first particle-based graphics erosion] .
- Beyer, H.T. 2015. "Implementation of a method for hydraulic erosion." Bachelor thesis, Technische Universität München (Informatics: Games Engineering), 15 Nov 2015. Author page https://www.firespark.de/?id=project&project=HydraulicErosion [V] — the droplet model everyone copies: inertia, capacity ∝ slope·speed·water, deposit/erode fraction, evaporation, radius-weighted erosion brush, min slope.
- Lague, S. 2019. "Hydraulic Erosion" (Coding Adventure), https://github.com/SebLague/Hydraulic-Erosion — direct implementation of Beyer's thesis in C#/compute shader [M; repo not resolved in today's search].
- McDonald, N. (weigert). "SimpleHydrology" https://github.com/weigert/SimpleHydrology [V] — particle erosion extended with stream/pool tracking, momentum transfer, "stream trees". Blog: nickmcd.me.
- McDonald, N., Cordonnier, G. 2026. "Stochastic Geomorphological Transport for Terrain Erosion Simulation." ACM TOG 45(4), art. 78 (SIGGRAPH 2026, Honorable Mention). DOI 10.1145/3811336. Project: https://erosiv.studio/publications/stochastic-geomorphological-transport [V] — formalises the particle approach as stochastic transport consistent with geomorphological laws; the most current justification for particle methods.

**(e) Stream-power / landscape-evolution (geological scale)**
- Cordonnier, G., Braun, J., Cani, M.-P., Beneš, B., Galin, E., Peytavie, A., Guérin, E. 2016. "Large Scale Terrain Generation from Tectonic Uplift and Fluvial Erosion." CGF 35(2): 165–175 (EG 2016). DOI 10.1111/cgf.12820. HAL hal-01262376 [V] — stream power law dh/dt = U − K·A^m·S^n solved implicitly on a graph (Braun & Willett 2013 O(N) ordering), uplift map as the control.
- Cordonnier, G., Galin, E., Gain, J., Beneš, B., Guérin, E., Peytavie, A., Cani, M.-P. 2017. "Authoring landscapes by combining ecosystem and terrain erosion simulation." ACM TOG 36(4), art. 134. DOI 10.1145/3072959.3073667 [V] — couples erosion with vegetation (vegetation modulates erodibility).
- Schott, H., Paris, A., Fournier, L., Guérin, E., Galin, E. 2023. "Large-scale Terrain Authoring through Interactive Erosion Simulation." ACM TOG 42(5), art. 162. DOI 10.1145/3592787. Code: https://github.com/H-Schott/StreamPowerErosion [V] — GPU stream-power with uplift inversion from an input DEM, hydrologically consistent patch blending. **Limitation stated by authors: water always exits the domain → no lakes.**
- Schott, H., Galin, E., Guérin, E., Paris, A., Peytavie, A. 2024. "Terrain Amplification using Multi Scale Erosion." ACM TOG 43(4), art. 145. DOI 10.1145/3658200. Code: https://github.com/H-Schott/MultiScaleErosion [V] — stream-power at coarse scale, local erosion at fine scale, for amplification.
- Related 2024: "Efficient Debris-flow Simulation for Steep Terrain Erosion." ACM TOG 43(4), DOI 10.1145/3658213 [V].
- Guérin, E., Digne, J., Galin, E., Peytavie, A., Wolf, C., Beneš, B., Martinez, B. 2017. "Interactive example-based terrain authoring with conditional generative adversarial networks." ACM TOG 36(6), art. 228. DOI 10.1145/3130800.3130804 [V] — **note only**: learned "erosion" from sketches; not a simulation; out of scope for a deterministic tool.

### 2.2 Comparison

| Criterion | Droplet / particle | Grid virtual pipes (SWE) | Stream power (LEM) |
|---|---|---|---|
| Cost | O(P · path length), typically 50k–500k drops for 1k² ; embarrassingly parallel over particles but **scatter writes collide** | O(N·T); T≈500–5000 substeps for 1k² | O(N log N) per step (priority ordering) or O(N) with Braun–Willett; T≈50–500 |
| Memory | Height + optional hardness; tiny | h, water, 4 flux, 2 velocity, sediment (≈8 f32/cell = 32 B/cell; 8k² = 2 GB — **tiling mandatory**) | h, A (drainage), receiver index, order list (≈4 fields) |
| GPU | Possible (one thread per drop) but non-deterministic unless atomics are ordered or writes are binned | **Ideal** — pure stencil Jacobi passes | Partial: flow routing is a serial dependency (parallel variants exist: Schott 2023 GPU) |
| Determinism | Depends on drop order & float scatter order; fixed seed + sequential CPU = deterministic; parallel = must bin particles by tile and apply in fixed order | Deterministic (gather stencil, no atomics) | Deterministic given a stable total ordering of cells (tie-break by index) |
| Stability | Unconditionally "stable" but drops can oscillate in pits (inertia) → min-slope/iteration caps | **CFL**: Δt ≤ Δx / (√(g·h_max)·2) roughly; pipe model needs flux clamping so outflow ≤ stored water; also "evaporation" must not drive water < 0 | Implicit scheme unconditionally stable (Braun & Willett); explicit variant needs K·A^m·Δt/Δx small |
| Scale realism | Gullies, small valleys, alluvial fans; good at 1–10 m/cell | Rivers, sediment deposition, floodplains; good at 1–30 m/cell | Drainage networks, ridgelines, Hack's law; good at 50 m – 1 km/cell |
| Artefacts | Noisy pitting, "worm" channels on flats, boundary pile-up | Checkerboarding if pipe lengths mis-set; diffusion from semi-Lagrangian advection; mass loss at tile borders | Over-regular dendritic look; needs uplift noise & hardness; no lakes unless depression handling added |

### 2.3 Recommendation & order
1. **P3a — Grid virtual-pipes (Mei 2007 + Šťava 2008 layers)**: implement first. Deterministic, a direct stencil → identical rayon and wgpu kernels, tiles cleanly with a halo of 1–2 cells (flux exchange across apron). Add thermal step (Area 3) inside the same loop as Šťava.
2. **P3b — Droplet erosion (Beyer 2015)**: cheap, popular, gives the "Lague look". Implement CPU-first with a fixed-seed PCG per drop index; parallelise by assigning drops to tiles and processing each tile's drops sequentially (deterministic). Later GPU variant with per-tile binning.
3. **P5 — Stream power (Cordonnier 2016 / Schott 2023)**: for 8k+ and planetary scale; builds on the flow-routing infrastructure of Area 6 (priority-flood + receiver graph). Add lake handling via depression filling to fix Schott's stated limitation.
4. SPH (Krištof 2009): do not implement; cost/benefit poor for heightfields.

Parameters to expose (grid): rain rate / rain map, pipe cross-section & length, gravity, Δt, evaporation, sediment capacity constant Kc, dissolving Ks, deposition Kd, min-tilt (capacity floor), hardness map (multiplies Ks), talus angle & thermal rate, iterations, boundary mode (open/closed/periodic).
Parameters (droplet): drops, max lifetime, inertia, capacity, min slope, erode speed, deposit speed, evaporate speed, gravity, erosion radius, start-water, start-speed, hardness map, seed.
Parameters (stream power): uplift map, K (erodibility map), m, n, Δt, iterations, hillslope diffusion D, base level.

---

## 3. Thermal erosion / talus

- Musgrave, Kolb, Mace 1989 [M] — thermal weathering: if slope to neighbor exceeds talus angle, move fraction of the excess downslope. O(N·T), 4- or 8-neighbour stencil.
- Olsen 2004 [V] — speed-optimised thermal (only move to the lowest neighbour vs. all exceeding neighbours), and an inverted "ridge sharpening" variant.
- Šťava et al. 2008 [V] — per-layer talus angles (rock ~ 90°, sand ~ 30–35°), material slippage into the next layer.
- Beneš & Forsbach 2001 [V] — layered representation that makes multi-material thermal tractable.

Pitfalls: Jacobi vs Gauss–Seidel ordering changes results — use Jacobi (gather) for determinism and GPU parity; distribute moved material proportionally to slope excess to avoid oscillation; clamp moved volume ≤ excess/2 per step for stability; talus angle must be in height-units/cell-size consistent units (tan θ · Δx).

Recommendation (P3, with hydraulic): one `thermal` node with parameters talus angle (per material), rate, iterations, neighbourhood (4/8), and a layered-material mode once layers exist. Share the stencil infrastructure with the pipes solver.

---

## 4. Wind / aeolian erosion & dunes

- Werner, B.T. 1995. "Eolian dunes: Computer simulations and attractor interpretation." Geology 23(12): 1107–1110. DOI 10.1130/0091-7613(1995)023<1107:EDCSAA>2.3.CO;2 [V] — slab-based cellular automaton: pick a slab, hop it downwind L cells, deposit with probability p (higher on sand than bedrock), shadow zone behind crests, avalanche at angle of repose. Produces barchan, transverse, linear, star dunes. O(slabs·hop) per step; stochastic; deterministic given seed and sequential order. Reference implementations: https://github.com/ebgoldstein/wDune95, https://github.com/markstock/wernerdunes [V].
- Narteau, C., Zhang, D., Rozier, O., Claudin, P. 2009. "Setting the length and time scales of a cellular automaton dune model from the analysis of superimposed bed forms." JGR 114, F03006. DOI 10.1029/2008JF001127 [V] — 3D lattice-gas CA (ReSCAL) with physically calibrated scales; heavier than Werner.
- Paris, A., Peytavie, A., Guérin, E., Argudo, O., Galin, E. 2019. "Desertscapes Simulation." CGF 38(7): 47–55 (Pacific Graphics 2019). DOI 10.1111/cgf.13815. Code: https://github.com/aparis69/Desertscapes-Simulation [V] — Werner-style CA with layered bedrock/sand, vegetation, wind shadowing, user-controlled wind fields; interactive. The graphics reference.

Pitfalls: Werner CA is inherently sequential-random (slab picks); a parallel version must process slabs in fixed tile-order with a per-tile PRNG stream; shadow test needs upwind scan (precompute a shadow mask per wind direction per step); results are only meaningful in sand-depth layers (needs the layered representation).

Recommendation: P5. Implement Paris 2019 (which subsumes Werner) as a layered-terrain node after layered materials exist. Expose wind direction/strength (map or constant), hop length, deposition probabilities (sand/bedrock), shadow angle, angle of repose, vegetation mask, steps, seed.

---

## 5. Coastal, glacial, snow, weathering

**Coastal**
- Searched ACM/EG venues for a dedicated coastal-erosion paper by the Lyon group (Galin/Peytavie/Paris/Cortial): **UNVERIFIED — could not locate** such a paper. Cortial's planet work (Area 9) and Paris 2019 "Terrain Amplification with Implicit 3D Features" (sea-cliff/arch examples as implicit features, not an erosion model) are the nearest.
- Outside graphics: Brown, I. 2006. "Dynamic simulation and visualisation of coastal erosion." Computers, Environment and Urban Systems 30(6). https://www.sciencedirect.com/science/article/abs/pii/S0198971505000591 [V] — cliff-recession model coupled to DEM editing.
- Practical approach for Jenga Dunia: shoreline = sea-level isocontour; wave-exposure = fetch (distance scan against wind direction over water); cliff retreat = thermal-like erosion rate ∝ exposure within a vertical band around sea level; beach deposition from eroded sediment into low-slope cells adjacent to water. Document as heuristic, not a published model.

**Glacial**
- Argudo, O., Galin, E., Peytavie, A., Paris, A., Guérin, E. 2020. "Simulation, modeling and authoring of glaciers." ACM TOG 39(6), art. 177 (SIGGRAPH Asia 2020). DOI 10.1145/3414685.3417855 [V] — shallow-ice approximation (SIA) ice-flow simulation + procedural crevasses/moraines/ogives.
- 2023 follow-up: "Forming Terrains by Glacial Erosion." ACM TOG (SIGGRAPH 2023). DOI 10.1145/3592422 [V] — glacial erosion of bedrock (U-valleys, cirques).
- SIA = nonlinear diffusion of ice thickness: ∂H/∂t = ∇·(D ∇(h+H)) + mass balance, D ∝ H^(n+2)|∇s|^(n−1), Glen n=3. Explicit stencil, stability Δt ∝ Δx²/D_max (very stiff) → implicit or adaptive Δt. Fits our stencil infrastructure.

**Snow**
- Festenberg, N. v., Gumhold, S. 2011. "Diffusion-Based Snow Cover Generation." CGF 30(6): 1837–1849. DOI 10.1111/j.1467-8659.2011.01904.x [V] — snow height as diffusion from occlusion/accumulation, granular-deposition theory, handles bridges/overhangs (mesh-based). For heightfields: snow mask = f(altitude, slope < angle of repose, sky visibility/AO, aspect-weighted insolation); then one diffusion pass.
- Related: "Real-time accumulation of occlusion-based snow" (Visual Computer 2015, DOI 10.1007/s00371-014-0995-5) [V] and the 2024 CGF STAR "Snow and Ice Animation Methods in Computer Graphics" (DOI 10.1111/cgf.15059) [V] for a survey.

**Tectonic/sub-surface weathering (interactive)**
- Cordonnier, G., Cani, M.-P., Beneš, B., Braun, J., Galin, E. 2018. "Sculpting Mountains: Interactive Terrain Modeling Based on Subsurface Geology." IEEE TVCG 24(5): 1756–1769. DOI 10.1109/TVCG.2017.2689022. https://www.cs.purdue.edu/cgvlab/www/publications/cordonnier2018sculpting/ [V] — user moves plates; crust-folding model produces a volumetric uplift map; joint uplift+erosion; folded strata rendering.

Recommendation: P5 all. Order: snow mask (cheap, big visual payoff, P4 even) → glacial SIA (reuses diffusion stencil) → coastal heuristic → geology-aware sculpting only if an interactive editor is in scope.

---

## 6. Hydrology (flow routing, depressions, rivers)

| Problem | Algorithm | Source | Complexity | Notes |
|---|---|---|---|---|
| Single flow direction | D8 | O'Callaghan & Mark 1984, CVGIP 28(3) (DOI 10.1016/S0734-189X(84)80011-0) [M] | O(N) | 45° quantisation → parallel-line artefacts on planar slopes |
| Multiple flow direction | MFD (FD8) | Quinn, Beven, Chevallier, Planchon 1991, Hydrological Processes 5(1) [M]; Freeman 1991, Computers & Geosciences 17(3) [M] | O(N) after topological sort | Diffuses flow on hillslopes; exponent p (Freeman 1.1) controls dispersion; Holmgren 1994 adaptive exponent |
| D-infinity | Tarboton, D.G. 1997. "A new method for the determination of flow directions and upslope areas in grid digital elevation models." WRR 33(2): 309–319. DOI 10.1029/96WR03137 [V] | O(N) | Flow angle from steepest of 8 triangular facets, split between 2 neighbours. Best general choice |
| Depression filling / watershed labelling | **Priority-Flood** | Barnes, R., Lehman, C., Mulla, D. 2014. "Priority-flood: An optimal depression-filling and watershed-labeling algorithm for digital elevation models." Computers & Geosciences 62: 117–127. DOI 10.1016/j.cageo.2013.04.024. arXiv 1511.04463. Code: https://github.com/r-barnes/Barnes2013-Depressions; production: RichDEM [V] | O(N log N) float, O(N) integer | Flood inward from edges with a priority queue; also yields lake extents (filled − original) and labels. Add epsilon-gradient variant for drainable flats. Serial; for tiles use Barnes 2016 parallel priority-flood (arXiv 1606.06204) [M] |
| Flow accumulation | Topological order over receivers (or recursion with memo) | Braun & Willett 2013 (Geomorphology 180–181, DOI 10.1016/j.geomorph.2012.10.008) [M] stack ordering is O(N) and reusable for stream power | O(N) | Order cells by descending height (sort O(N log N)) or by the Braun–Willett donor stack; accumulate rain-weighted area |
| Stream order | Strahler 1957 (Trans. AGU 38(6)) [M]; Shreve 1966 | O(N) on tree | Threshold accumulation → channel mask → order on tree |
| Drainage basins | Labels from priority-flood or from receiver-tree root | O(N) | |
| River carving / hydrology-first generation | Génevaux, J.-D., Galin, E., Guérin, E., Peytavie, A., Beneš, B. 2013. "Terrain generation using procedural models based on hydrology." ACM TOG 32(4), art. 143. DOI 10.1145/2461912.2461996. HAL hal-01339224 [V] | Graph growth + primitives | Grow river network as a tree from the coast (Horton/Strahler-consistent), then build terrain from river primitives + blending/carving. Also "Procedural Riverscapes" (Peytavie et al. 2019, CGF 38(7)) [M] |
| Lake detection | Priority-flood lake volumes; or Schott 2023's limitation workaround | O(N log N) | lake = filled − original > ε; water level = spill elevation |
| Wetness index | TWI = ln(a / tan β) | Beven & Kirkby 1979, Hydrological Sciences Bulletin 24(1) (DOI 10.1080/02626667909491834) [M] | O(N) | a = specific catchment area (accumulation × Δx / contour width); needs MFD/D∞ for sensible hillslope values; clamp tan β ≥ 0.001 |

GPU: flow routing is a tree dependency; GPU-friendly alternatives are iterative "push" accumulation (T ≈ longest flow path) or a hybrid: priority-flood + receivers on CPU (one-off), accumulation on GPU. Priority-flood on 8k² (64 M cells) with a binary heap: ~10–30 s single-thread, 64 M × (4 B h + 4 B label + 1 B visited) ≈ 600 MB — fits the budget without tiling; tile only for >8k.

Recommendation (P2 for D8/accumulation/TWI as analysis masks; P3 for priority-flood & D∞ as erosion prerequisites; P5 for Génevaux-style generation).

---

## 7. Terrain analysis

- Slope / aspect: Horn 1981 (3×3 Sobel-like finite difference; Proc. IEEE 69(1), DOI 10.1109/PROC.1981.11918) [M] — standard in GIS. Zevenbergen & Thorne second-order fit gives slope from the quadratic too.
- Curvature: Zevenbergen, L.W., Thorne, C.R. 1987. "Quantitative analysis of land surface topography." Earth Surface Processes and Landforms 12(1): 47–56. DOI 10.1002/esp.3290120107 [V] — 9-term polynomial through 3×3; profile, plan (planform), total curvature. Evans–Young: Evans 1979/1980 (Durham report; Young 1978) 6-term least-squares quadratic [M] — smoother, less noise-sensitive; SAGA implements both [V]. Use Evans–Young for masks (robust), Z–T when exact-fit matters.
- Multi-scale: compute curvature at several Gaussian pyramid levels; "cavity/peaks" = sign-split curvature at scale s; "exposure" = height − mean(height, radius r) (a.k.a. TPI, Weiss 2001 poster) — all O(N) per scale with separable blurs.
- Ambient occlusion / horizon maps for heightfields: Max, N.L. 1988. "Horizon mapping: shadows for bump-mapped surfaces." The Visual Computer 4(2): 109–117 [V] — per-cell horizon angle per azimuth. Scalable exact version: Timonen, V., Westerholm, J. 2010. "Scalable Height Field Self-Shadowing." CGF 29(2): 723–731. DOI 10.1111/j.1467-8659.2009.01642.x [V] — convex-hull sweep per azimuth, O(N·directions), 23 fps at 1024² × 64 dirs on GTX 280. Heightfield AO = 1 − mean over azimuths of (1 − cos horizon) with cosine weighting; sky-view factor likewise.
- Flow maps: from D∞ angle (Area 6) → 2D unit vector per cell, smoothed; encode RG8.
- Rock/soil masks: slope threshold ⊗ curvature ⊗ TWI; standard practice (no single canonical paper; e.g. Gaia/World Machine node docs).

All O(N) or O(N·directions); trivially parallel; identical on CPU/GPU if the finite-difference stencil and blur kernels are fixed. Phase P2 (slope/aspect/curvature/TPI), P3 (AO/horizon, flow maps).

---

## 8. Non-heightfield terrain (overhangs, caves, arches)

- Marching Cubes: Lorensen, W.E., Cline, H.E. 1987. SIGGRAPH '87, Computer Graphics 21(4): 163–169. DOI 10.1145/37401.37422 [M]. O(voxels); 256-case table; ambiguous cases → use MC33 (Chernyaev 1995) or the asymptotic decider. GPU trivial. Patent expired 2005.
- Transvoxel (LOD stitching for MC): Lengyel, E.S. 2010. "Voxel-Based Terrain for Real-Time Virtual Simulations." PhD dissertation, UC Davis. PDF: https://www.terathon.com/lengyel/Lengyel-VoxelTerrain.pdf; https://transvoxel.org [V citation / M URL] — transition cells between LOD levels, no cracks.
- Dual Contouring: Ju, T., Losasso, F., Schaefer, S., Warren, J. 2002. "Dual contouring of hermite data." SIGGRAPH 2002 / ACM TOG 21(3): 339–346. DOI 10.1145/566654.566586 [V venue, M DOI] — vertex per cell via QEF on Hermite data; preserves sharp features; needs normals; QEF solve can place vertices outside cell (clamp). Manifold variant: Schaefer, Ju, Warren 2007 "Manifold dual contouring" TVCG.
- Surface Nets: Gibson 1998 (MICCAI, "Constrained elastic surface nets") [M]; "naive surface nets" (Mikola Lysenko 2012, web) — cheaper DC without Hermite data; smoother; no sharp features.
- Layered heightfields / implicit amplification: Paris, A., Galin, E., Peytavie, A., Guérin, E., Gain, J. 2019. "Terrain Amplification with Implicit 3D Features." ACM TOG 38(5), art. 147 (SIGGRAPH Asia 2019). DOI 10.1145/3342765. Code (partial): https://github.com/aparis69/Implicit-Volumetric-Terrains [V] — heightfield + construction tree of implicit primitives (caves, arches, overhangs, hoodoos, karst) placed by Poisson sampling, shaped by erosion-inspired grammars; polygonised with MC.
- Peytavie, A., Galin, E., Grosjean, J., Mérillou, S. 2009. "Arches: a Framework for Modeling Complex Terrains." CGF 28(2): 457–467 (EG 2009). DOI 10.1111/j.1467-8659.2009.01385.x [V] — layered material stack per column (bedrock/sand/rocks/water) + implicit "Arches" blending; stability/relaxation of granular layers.
- Becher, M., Krone, M., Reina, G., Ertl, T. 2019. "Feature-Based Volumetric Terrain Generation and Decoration." IEEE TVCG 25(2): 1283–1296. DOI 10.1109/TVCG.2017.2762304 [V] — diffusion-based volumetric terrain from sparse curve features.

Memory: dense SDF at 8k × 8k × 1k is impossible (64 G voxels); use sparse brick/octree chunks (e.g. 32³ bricks, f16) only where features exist, with the heightfield as the base implicit (h(x,y) − z).

Recommendation: heightfield primary (P2). **SDF/voxel secondary module (P5)**: sparse bricks of f16 SDF; mesh via naive surface nets (simplest, robust) with Transvoxel-style stitching deferred; Paris 2019 implicit primitives as the authoring model. Dual contouring only if sharp cliff edges are required.

---

## 9. Large-scale / tectonic / planetary

- Cordonnier 2016 (uplift + stream power) [V] — see §2.
- Cortial, Y., Peytavie, A., Galin, E., Guérin, E. 2019. "Procedural Tectonic Planets." CGF 38(2): 1–11 (EG 2019). DOI 10.1111/cgf.13614. PDF: https://perso.liris.cnrs.fr/eric.galin/Articles/2019-planets.pdf [V] — procedural (not physical) plate model on a sphere: subduction, collision, rifting via crust-type rules; user steers plates; output elevation/crust maps for later amplification. Follow-up: "Real-time hyper-amplification of planets," The Visual Computer 2020, DOI 10.1007/s00371-020-01923-4 [V].
- Cordonnier 2018 Sculpting Mountains [V] — interactive, local-scale alternative.
- Planet parametrisation: cube-sphere (6 faces, quadtree per face, tangent-adjusted "COBE"/arctan warp to equalise cell size) — standard practice (e.g. Kooima, Cesium quantized-mesh, Outerra). No single canonical paper; cite Cesium's "World Terrain" docs or Nvidia/AMD planet demos [M].
- Floating origin: Thorne, C. 2005. "Using a floating origin to improve fidelity and performance of large, distributed virtual worlds." CW 2005 (DOI 10.1109/CW.2005.94) [M]. Use f64 for world positions, f32 relative to a camera-centred origin on GPU; recentre when |offset| > threshold. Alternative: fixed-point i64 metres×1e3 for authoritative coordinates.
- Geodesy if real-world data: WGS84 ellipsoid, ECEF ↔ geodetic (standard; e.g. `nav-types`/`geodesy` crates).

Recommendation: P5. Planet = cube-sphere quadtree of the same tile type as the flat world; 3D noise on the sphere for base; Cortial 2019 for continents; stream power on the sphere graph (Cordonnier-style) for drainage. f64 world coords, f32 tiles.

---

## 10. Terrain LOD & meshing

| Technique | Source | Characteristics |
|---|---|---|
| ROAM | Duchaineau, M. et al. 1997. "ROAMing terrain: real-time optimally adapting meshes." IEEE Visualization '97, 81–88. DOI 10.1109/VISUAL.1997.663860 [M] | Binary-triangle-tree split/merge with priority queues; CPU-bound; historical, poor fit for modern GPUs |
| Chunked LOD | Ulrich, T. 2002. "Rendering massive terrains using chunked level of detail control." SIGGRAPH 2002 course notes. http://tulrich.com/geekstuff/chunklod.html [V] | Quadtree of pre-simplified chunks, per-chunk geometric error → screen-space error, **skirts** hide cracks, per-chunk vertex morphing |
| Geometry clipmaps | Losasso, F., Hoppe, H. 2004. "Geometry clipmaps: terrain rendering using nested regular grids." ACM TOG 23(3) (SIGGRAPH 2004). DOI 10.1145/1015706.1015799 [M DOI; V project page https://hhoppe.com/proj/geomclipmap/]. GPU version: Asirvatham & Hoppe 2005, GPU Gems 2 ch. 2 [V] | Nested regular grids around viewer, heights from texture (VTF), toroidal updates, blends between levels; ideal for streaming big heightfields |
| CDLOD | Strugar, F. 2009/2010. "Continuous Distance-Dependent Level of Detail for Rendering Heightmaps." JGT 14(4). DOI 10.1080/2151237X.2009.10129287. Code/paper: https://github.com/fstrugar/CDLOD [V] | Quadtree of fixed grid meshes, per-vertex morphing by 3D distance, no stitching; simplest modern choice |
| Quadtree + stitching/skirts | folklore; skirts per Ulrich 2002 | Stitching = index-buffer variants per edge LOD combination (16 variants) |
| GPU tessellation / mesh shaders | e.g. Cantlay 2011 "DirectX 11 Terrain Tessellation" (NVIDIA whitepaper) [M] | Hardware tessellation; wgpu lacks tessellation → not applicable; use CDLOD |
| RTIN | Evans, W., Kirkpatrick, D., Townsend, G. 2001. "Right-Triangulated Irregular Networks." Algorithmica 30(2): 264–286. DOI 10.1007/s00453-001-0006-x [V]. Implementation: Mapbox MARTINI (https://github.com/mapbox/martini, JS; Rust ports exist) [M] | Error-bounded mesh from a heightfield in O(N); grid must be 2^k+1; hierarchical error propagation; great for quick, deterministic export meshes |
| Greedy TIN insertion | Garland, M., Heckbert, P.S. 1995. "Fast polygonal approximation of terrains and height fields." Tech. Report CMU-CS-95-181, Carnegie Mellon [M]. Implementations: hmm (https://github.com/fogleman/hmm), Delatin [M] | Incremental Delaunay insertion of max-error point; O(N log N)–O(N²) worst; best triangle count for a given error |
| Delaunay TIN | Bowyer–Watson 1981; Rust: `delaunator`, `spade` crates [M] | For arbitrary point sets |

Screen-space error: ε_screen = (ε_world · viewport_h) / (2 · d · tan(fov/2)); select the coarsest LOD with ε_screen ≤ τ (typically 1–2 px). Store per-node max geometric error at build time (max |h_fine − h_coarse_interpolated|).

Recommendation: **CDLOD** for the viewport (P4; wgpu compute can build the quadtree selection), skirts optional; **RTIN** for fast error-bounded mesh export (P2 — simple, deterministic, O(N)); **greedy TIN (Garland–Heckbert)** as the "quality" export option (P4); geometry clipmaps only if streaming >8k from disk becomes a goal (P5).

---

## 11. Mesh decimation, remeshing & topology

- QEM: Garland, M., Heckbert, P.S. 1997. "Surface simplification using quadric error metrics." SIGGRAPH '97, 209–216. DOI 10.1145/258734.258849 [M]. O(n log n) with heap; edge collapse; preserves shape well; needs boundary-constraint quadrics for open meshes (terrain tiles!).
- Attribute-aware QEM: Hoppe, H. 1999. "New quadric metric for simplifying meshes with appearance attributes." IEEE Visualization 1999, 59–66. https://hhoppe.com/proj/newqem/ [V]; DOI 10.1109/VISUAL.1999.809869 [M]. Wedge structure for UV seams/creases. Patent US6362820B1 (Microsoft, 2002) — expired 2019–2020 by term [V existence; M expiry].
- Vertex clustering: Rossignac, J., Borrel, P. 1993. "Multi-resolution 3D approximations for rendering complex scenes." In *Modeling in Computer Graphics*, Springer, 455–465. DOI 10.1007/978-3-642-78114-8_29 [V]. O(n), GPU-friendly, poor quality; fine for distant LODs/imposters.
- meshoptimizer simplifier: Kapoulkine, A. https://github.com/zeux/meshoptimizer — QEM-based with attribute weights, border locking, `meshopt_simplifySloppy` (clustering), error bounds; Rust bindings `meshopt` crate [M]. Production-proven; recommended over rolling our own.
- Isotropic remeshing: Botsch, M., Kobbelt, L. 2004. "A Remeshing Approach to Multiresolution Modeling." SGP 2004, 185–192. DOI 10.2312/SGP/SGP04/189-196 [V] — split/collapse/flip/tangential-relax loop toward target edge length; good for SDF-extracted meshes before export.
- Manifold / degenerate checks: half-edge build, every edge ≤ 2 faces, consistent orientation, no zero-area triangles (|cross| < ε), no NaN. Rust: `baby_shark`, `tri-mesh`, or hand-rolled on index buffers [M].
- UV seams: lock seam vertices or use wedge quadrics (Hoppe 1999) — meshoptimizer does attribute-aware collapse.
- **Tangents: MikkTSpace**. Mikkelsen, M.S. 2008. "Simulation of Wrinkled Surfaces Revisited." Master's thesis, University of Copenhagen [M — thesis not located in today's search; cited by Blender/Unity/Unreal docs]. Reference C: https://github.com/mmikk/MikkTSpace; Rust: `mikktspace` crate (bevy fork `bevy_mikktspace`) [M]. Required for normal-map consistency with Blender, Unity, Unreal, glTF spec recommendation.

Recommendation (P4): wrap `meshopt` for decimation (attribute-aware, border-locked for tiles), Botsch–Kobbelt remesh only for the SDF module, mandatory manifold/degenerate validation before export, MikkTSpace tangents always.

---

## 12. Materials & texturing

- Splatting by slope/height/curvature/TWI: node outputs masks (Area 7) → N weight layers normalised to 1; standard (World Machine/Gaea convention; no canonical paper).
- Triplanar mapping: Geiss, R. 2007. "Generating Complex Procedural Terrains Using the GPU." GPU Gems 3, ch. 1 [M] — project on 3 axes weighted by |n|^k; removes stretching on cliffs; 3× texture cost.
- Stochastic texturing: Heitz, E., Neyret, F. 2018. "High-Performance By-Example Noise using a Histogram-Preserving Blending Operator." Proc. ACM CGIT 1(2), art. 31 (HPG 2018). DOI 10.1145/3233304. https://eheitzresearch.wordpress.com/722-2/ [V] — triangular lattice of 3 random tiles blended with variance-preserving weights in a Gaussianised colour space (precomputed T and T⁻¹ LUTs). Follow-up: Deliot & Heitz 2019 JCGT "Procedural Stochastic Textures by Tiling and Blending" [V]. ~20× faster than Gabor-style by-example noise.
- Texture bombing: Glanville, R.S. 2004. "Texture Bombing." GPU Gems ch. 20 [M].
- Detail / macro variation: standard multi-frequency blend; macro = low-frequency noise (Area 1) modulating albedo/roughness.
- Virtual texturing: Mittring, M. 2008. "Advanced Virtual Texture Topics." SIGGRAPH 2008 course "Advances in Real-Time Rendering" [M]; Barrett, S. 2008 "Sparse Virtual Textures" (GDC) [M]; Unreal Engine Runtime Virtual Texturing docs [M]. Needed only for >8k painted textures in-viewport — P5 at most.
- Normal from height: Sobel (3×3) or Scharr (better rotational symmetry); n = normalize(−dh/dx · s, −dh/dy · s, 1) with s = height_scale / cell_size. Tile aprons needed for the stencil at borders. Export: tangent-space normal map in +Y (OpenGL) convention by default, with a flip-green toggle for DirectX targets.
- AO baking: horizon method (Area 7, Timonen 2010) or ray-marched heightfield AO (cosine-weighted hemisphere, 16–64 rays, O(N · rays · steps)) — GPU-ideal.

Recommendation: P2 masks + normal/AO bake; P3 splat weight export; P4 Heitz–Neyret stochastic tiling and triplanar in the viewport; VT deferred.

---

## 13. Ocean / water

- FFT ocean: Tessendorf, J. 2001. "Simulating Ocean Water." SIGGRAPH 2001 course notes (updated 2004) [M]. Phillips/JONSWAP spectrum → inverse FFT height + choppy displacement; O(N log N) per frame; periodic; GPU via `wgpu` compute FFT; deterministic given seeded Gaussian spectrum (seed + fixed FFT order; CPU/GPU FFT ordering differs → tolerance, not bit-equality).
- Gerstner waves: Fournier, A., Reeves, W.T. 1986. "A simple model of ocean waves." SIGGRAPH '86, 75–84. DOI 10.1145/15922.15894 [M]. Sum of trochoidal waves; O(waves) per vertex; good for shoreline/lakes with few waves; cusps if steepness Q·k·A > 1.
- Shallow water (Saint-Venant): pipes model (Mei 2007) for interactive flooding; height-field SWE (Kass & Miller 1990 "Rapid, stable fluid dynamics for computer graphics", DOI 10.1145/97880.97884 [M]) for rivers/lakes with semi-implicit stability.
- Flow maps: Vlachos, A. 2010. "Water Flow in Portal 2." SIGGRAPH 2010 Advances in Real-Time Rendering course. https://www.advances.realtimerendering.com/s2010/Vlachos-Waterflow(SIGGRAPH%202010%20Advanced%20RealTime%20Rendering%20Course).pdf [V] — low-res 2D vector field warps tiled normal maps with two phase-offset layers; our flow map comes from D∞ (Area 6).
- Foam/shoreline: depth-based (scene depth − water depth) masks; standard.
- Underwater attenuation: Beer–Lambert T = exp(−σ·d), per-channel σ (e.g. Jerlov water types) [M].
- Reflection/refraction: planar reflection pass or screen-space reflections (McGuire & Mara 2014 JCGT "Efficient GPU Screen-Space Ray Tracing") [M]; refraction via screen-space UV offset by normal.

Recommendation: P4 Gerstner (cheap, deterministic) for the viewport; FFT ocean as an optional GPU node (P5); water surfaces from priority-flood lake levels + sea level; flow maps from hydrology.

---

## 14. Atmosphere, sky, weather visualisation

- Preetham, A.J., Shirley, P., Smits, B. 1999. "A practical analytic model for daylight." SIGGRAPH '99, 91–100. DOI 10.1145/311535.311545 [M] — Perez-model fit; cheap; known to be wrong near horizon and at high turbidity (Zotti et al. 2007 critique, WSCG) [V].
- Hošek, L., Wilkie, A. 2012. "An analytic model for full spectral sky-dome radiance." ACM TOG 31(4) (SIGGRAPH 2012). DOI 10.1145/2185520.2185591 [M DOI; V project https://cgg.mff.cuni.cz/projects/SkylightModelling/] — better fit, includes ground albedo; reference C code (BSD-ish licence); used in Blender's Nishita predecessor.
- Bruneton, E., Neyret, F. 2008. "Precomputed Atmospheric Scattering." CGF 27(4): 1079–1086 (EGSR 2008). DOI 10.1111/j.1467-8659.2008.01245.x [V]. Code: https://github.com/ebruneton/precomputed_atmospheric_scattering [M] — 4D LUTs, multiple scattering, aerial perspective; LUT bake is heavy.
- Hillaire, S. 2020. "A Scalable and Production Ready Sky and Atmosphere Rendering Technique." CGF 39(4): 13–22 (EGSR 2020). DOI 10.1111/cgf.14050 [V] — small 2D LUTs (transmittance, multi-scattering, sky-view) recomputed per frame; UE4/5 implementation; ideal for wgpu.
- Clouds: Schneider, A., Vos, N. 2015. "The real-time volumetric cloudscapes of Horizon: Zero Dawn." SIGGRAPH 2015 Advances in Real-Time Rendering course [M]; Hillaire, S. 2016. "Physically Based Sky, Atmosphere and Cloud Rendering in Frostbite." SIGGRAPH 2016 PBS course. https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/s2016_pbs_frostbite_sky_clouds.pdf [V]. Perlin–Worley density, ray-march with Beer–Powder, temporal reprojection.
- Height fog: exponential height fog (Wenzel 2006 "Real-time atmospheric effects in games", SIGGRAPH course) [M].
- Snow/wetness shading: snow mask (Area 5) → albedo/roughness blend; wetness darkens albedo & lowers roughness (Nakamae et al. 1990 SIGGRAPH "A lighting model aiming at drive simulators" for wet roads, DOI 10.1145/97879.97922) [M].
- Sun/moon position: NOAA Solar Calculator equations (based on Meeus, J. 1998. *Astronomical Algorithms*, 2nd ed., Willmann-Bell) [M]; NREL SPA (Reda & Andreas 2004, Solar Energy 76(5), DOI 10.1016/j.solener.2003.12.003) [M]. Rust crates: `spa` v0.5.1 — sunrise/sunset + azimuth/zenith from UTC time & lat/lon, no_std [V, https://docs.rs/spa]; `sun` v0.3.1 — port of suncalc.js, position + phases, also moon [V, https://docs.rs/sun]; `sunrise-sunset-calculator` (NREL SPA port) [V].

Recommendation: P4 — Hillaire 2020 sky in wgpu (small LUTs, scalable); Hošek–Wilkie as CPU fallback/preview; `spa` crate for sun; clouds (Schneider/Hillaire) P5.

---

## 15. Vegetation / scatter

- Poisson-disk: Bridson, R. 2007. "Fast Poisson disk sampling in arbitrary dimensions." SIGGRAPH 2007 Sketches. DOI 10.1145/1278780.1278807 [M DOI; V PDF https://www.cs.ubc.ca/~rbridson/docs/bridson-siggraph07-poissondisk.pdf] — O(n) with background grid (cell = r/√2), k=30 candidates. Variable-radius variant: weight by density map (scale r per candidate). Deterministic given seed and sequential processing — parallelise per tile with halo and a tile-indexed PRNG stream; tile-boundary samples processed in a fixed order.
- Blue-noise tiles / Wang tiles: Cohen, M.F. et al. 2003 "Wang Tiles for image and texture generation" SIGGRAPH (DOI 10.1145/882262.882265) [M]; Kopf, J. et al. 2006 "Recursive Wang tiles for real-time blue noise" ACM TOG 25(3) (DOI 10.1145/1141911.1141916) [M] — precomputed point sets for unbounded, density-controlled scatter; O(points) at runtime.
- Ecosystem simulation: Deussen, O., Hanrahan, P., Lintermann, B., Měch, R., Pharr, M., Prusinkiewicz, P. 1998. "Realistic modeling and rendering of plant ecosystems." SIGGRAPH '98, 275–286. DOI 10.1145/280814.280898. https://algorithmicbotany.org/papers/ecosys.sig98.html [V] — individual-based competition for resources (light, water), self-thinning, succession; O(plants · neighbours · years). Cordonnier 2017 (Area 2) couples this to erosion.
- Biome classification: Whittaker, R.H. 1975. *Communities and Ecosystems*, 2nd ed., Macmillan [M] — temperature × precipitation diagram; Holdridge, L.R. 1947. "Determination of World Plant Formations From Simple Climatic Data." Science 105(2727): 367–368. DOI 10.1126/science.105.2727.367 [M] — biotemperature × precipitation × PET ratio. Inputs: temperature = lapse rate (≈ −6.5 °C/km) × altitude + latitude term; precipitation = base + orographic (upwind slope · wind) − rain shadow; moisture proxy = TWI / rainfall accumulation.
- Rules: slope < max for species, altitude band, moisture band, aspect (insolation), exclusion near rivers/roads; density → scatter radius.
- Rendering: GPU instancing; LOD + impostors (octahedral impostors: Rotational/"Octahedral Impostors", e.g. Ryan Brucks UE4 2018; academic: Décoret et al. 2003 "Billboard clouds" SIGGRAPH, DOI 10.1145/882262.882326) [M].

Recommendation: P3 — climate maps (temperature/precipitation from altitude/latitude/wind), Whittaker lookup → biome id map, rule masks; Bridson scatter per tile with density; P5 — Deussen ecosystem years for succession; instancing/impostors in viewport P4.

---

## 16. Caching & determinism

- Content hashing: BLAKE3 (O'Connor, Aumasson, Neves, Wilcox-O'Hearn 2020, https://github.com/BLAKE3-team/BLAKE3) [M] — `blake3` crate, multi-threaded (rayon feature), ~GB/s. Hash = blake3(node_type ‖ version ‖ canonicalised params ‖ input hashes); params serialised with a stable binary encoding (e.g. `postcard`/bincode with fixed int widths; floats as raw bits so −0.0 ≠ 0.0 is intentional or canonicalised). Hash tiles individually (tile = node_hash ‖ tile_xy) for partial recompute.
- PRNG: PCG32 (O'Neill 2014, "PCG: A Family of Simple Fast Space-Efficient Statistically Good Algorithms for Random Number Generation," HMC tech report hmc-cs-2014-0905, https://www.pcg-random.org/paper.html) [M] — `rand_pcg`; xoshiro256** (Blackman & Vigna 2018/2021, ACM TOMS 47(4), DOI 10.1145/3460772) [M] — `rand_xoshiro`. Stream per (seed, node_hash, tile_id, particle_id): derive via a hash (SplitMix64 or blake3-keyed) rather than sequential jumps so every tile/particle is independently seeded and order-independent. On GPU: implement the same PCG32 step in WGSL (u32 ops only) → bit-identical streams.
- Deterministic parallel reductions: floating-point addition is not associative; `rayon` `sum()` order depends on work-stealing → run-to-run differences. Fix: reduce per fixed-size chunk, then sum chunk partials in index order (deterministic); or Kahan/pairwise with fixed tree; or accumulate in f64 / fixed-point i64 for statistics (min/max/mean/histograms). Same tree shape on GPU (workgroup reduce + fixed second pass).
- Scatter (erosion particles, splats): never use atomics on f32 for the authoritative result; bin to tiles and apply in fixed order, or convert to gather formulation.
- CPU vs GPU parity: expect differences from (a) FMA contraction (Rust does not contract by default; WGSL/Metal/Vulkan compilers may — disable via naga/driver flags where possible, or write `fma()` explicitly in both), (b) transcendental functions (`sin`, `exp`, `pow` differ by ULPs across vendors; the "fast math" GPU paths more so) — use our own polynomial approximations for anything inside hashed/thresholded logic, (c) denormal flushing on GPU, (d) reduction order. Define tolerance tiers: **bit-exact** for integer hashing, PRNG, and noise lattice selection; **≤ 1e-6 relative** for noise values & stencil ops; **≤ 1e-3 relative + bounded iteration drift** for simulations (compare statistics, not cells); unit-test with golden CPU tiles and a tolerance assert for GPU.
- Cache validity key must include: algorithm version, backend (cpu/gpu) if tolerance ≠ bit-exact, tile size & apron, f32/f16 precision mode.

---

## (a) Algorithm selection table

| Problem | Recommended algorithm(s) | Alternatives considered | Why | Phase |
|---|---|---|---|---|
| Base noise | OpenSimplex2S / Improved Perlin via `fastnoise-lite`/`opensimplex2`, with analytic derivatives | Simplex (patent now expired), value noise, Gabor | Fewest artefacts per cost; CC0/MIT; trivial WGSL port; integer hashing for parity | P2 |
| Fractal combinators | fBm, ridged & hybrid multifractal (Musgrave 1989), domain warp (Quilez) | "uber noise" (preset of the same) | Canonical, cheap, well understood | P2 |
| Cellular masks | Worley 1996 (F1/F2, metrics) | — | Terraces, cracks, lakebeds | P2 |
| Seamless/large base | Spectral 1/f^β (Saupe 1988) | Periodic lattice noise | Periodic by construction, O(N log N) | P3 |
| Hydraulic erosion (default) | Virtual pipes SWE (Mei 2007) + layers (Šťava 2008) | Droplet, SPH | Deterministic gather stencil → CPU/GPU parity, tiles with 1–2 cell halo | P3 |
| Hydraulic erosion (fast/stylised) | Droplet (Beyer 2015; McDonald/Cordonnier 2026 for rationale) | — | Cheap, popular look; tile-binned for determinism | P3 |
| Thermal | Musgrave 1989 / Olsen 2004 Jacobi talus, per-layer angles | Gauss–Seidel | Determinism | P3 |
| Large-scale erosion | Stream power (Cordonnier 2016; Schott 2023 GPU) + priority-flood lakes | Multi-scale (Schott 2024) | Geological plausibility at 8k+/planet | P5 |
| Aeolian | Paris 2019 Desertscapes (Werner CA) | Narteau ReSCAL | Interactive, graphics-tested; needs layers | P5 |
| Glacial | Argudo 2020 SIA + Forming Terrains 2023 | — | Reuses diffusion stencil | P5 |
| Snow | Occlusion/slope mask + diffusion (Festenberg 2011) | — | Cheap, big payoff | P4 |
| Coastal | Heuristic exposure/retreat (documented as such) | — | No graphics paper located | P5 |
| Flow routing | D∞ (Tarboton 1997) + D8 for channels; MFD optional | — | Standard; angles give flow maps | P2/P3 |
| Depressions/lakes | Priority-flood (Barnes 2014) | Planchon–Darboux | Optimal, labels basins, gives lake levels | P3 |
| Accumulation/order | Braun–Willett O(N) stack; Strahler | Recursive | Reusable for stream power | P3 |
| River-first generation | Génevaux 2013 | — | When rivers drive layout | P5 |
| Slope/aspect | Horn 1981 | Z–T | GIS standard | P2 |
| Curvature | Evans–Young (robust) & Zevenbergen–Thorne | — | Both standard; multi-scale via pyramid | P2 |
| AO / horizon | Timonen 2010 sweep; ray-march on GPU | Max 1988 | Exact, scalable | P3 |
| Overhangs/caves | Sparse f16 SDF bricks + Paris 2019 implicit primitives; surface nets | Dual contouring, Transvoxel | Simplest robust meshing; HF stays primary | P5 |
| Planet | Cube-sphere quadtree, 3D noise, Cortial 2019 plates, f64 origin | Icosphere | Reuses tile type | P5 |
| Viewport LOD | CDLOD (Strugar) | Geometry clipmaps, ROAM | No tessellation in wgpu; no stitching | P4 |
| Mesh export (fast) | RTIN (Evans 2001 / MARTINI) | Regular grid | O(N), error-bounded, deterministic | P2 |
| Mesh export (quality) | Greedy TIN (Garland–Heckbert 1995) | Delaunay | Fewest triangles per error | P4 |
| Decimation | meshoptimizer (QEM, attribute-aware, border lock) | Hoppe 1999 own impl, vertex clustering | Production-proven | P4 |
| Tangents | MikkTSpace | — | DCC/engine parity | P4 |
| Texturing | Slope/height/curvature/TWI splat; Scharr normals; Heitz–Neyret stochastic; triplanar | Texture bombing, VT | Standard pipeline | P2–P4 |
| Water surface | Gerstner (viewport), FFT ocean (optional GPU) | SWE render | Cheap/deterministic | P4/P5 |
| Sky | Hillaire 2020 (GPU), Hošek–Wilkie (CPU) | Bruneton 2008, Preetham | Small LUTs, production-proven | P4 |
| Sun position | `spa` crate (NREL/NOAA) | `sun` crate | Accuracy + no_std | P4 |
| Scatter | Bridson 2007 per tile, density-weighted | Wang tiles | Deterministic, simple | P3 |
| Biomes | Whittaker/Holdridge lookup on climate maps | — | Standard | P3 |
| Caching | blake3 content hashes per node/tile | SHA-256 | Fast, parallel | P2 |
| PRNG | PCG32 (`rand_pcg`) hashed streams | xoshiro | Tiny state, trivially portable to WGSL | P2 |

---

## (b) Rust implementation notes

### Data layout
- `Tile { origin: (i32,i32), size: u32 /* 512 or 1024 */, apron: u32 /* 2 default; 1 for pure 3×3 ops; 4 for 9×9 blurs */, data: Vec<f32> /* (size+2·apron)² row-major, y-major */ }`. Store channels as separate planes (SoA): height, water, sediment, flux[4], vel[2], hardness, layers[k]. Row-major f32 plane = 4 B/cell; 1024² + apron(2) = 4.2 MB.
- Full-field budget: 8k² = 64 M cells → 256 MB per f32 plane. Pipes erosion needs ≈8 planes = 2 GB resident if un-tiled → **process in tile sweeps**: load tile + apron from the backing store, run T_inner substeps (apron shrinks by 1 cell per stencil step, so re-fetch every `apron` substeps), write back interior only. Keep ≤ 2–3 planes resident globally (height, hardness, water), stream the rest per tile. Backing store: memory-mapped file of planes (`memmap2`) or `Vec<f32>` per plane with explicit budget accounting; 4 GB cap enforced by a `MemoryBudget` that refuses to allocate beyond it and falls back to mmap.
- f16 (`half` crate) for export/preview planes only; never for simulation state (sediment mass drift).
- Tile size: 1024 for CPU (fewer halo re-fetches), 512 when GPU-resident (buffer ≤ 1–4 MB per plane keeps dispatches balanced). Make it a runtime parameter; pick 2^k.

### Parallelism (rayon)
- Per-tile `par_iter` over a tile grid; inside a tile, `par_chunks_mut(row_len)` over rows for stencils. Avoid `par_iter().sum()` for anything persisted; use chunk partials summed in order.
- Halo exchange between tiles after each substep batch: double-buffer planes (`ping`/`pong`); copying halos is O(perimeter).
- Particle erosion: bucket drops by tile; each tile processes its drops sequentially with `Pcg32::seed_from_u64(hash(seed, node, tile, drop_idx))`; drops exiting a tile are re-queued to the neighbour in a fixed ordering pass (deterministic two-phase).

### GPU (wgpu compute)
- One WGSL kernel per stencil pass with identical arithmetic to the Rust kernel (share constants via `include_str!`-generated code or a tiny codegen so magic numbers live once). Workgroup 16×16 (or 8×8 for heavy kernels); read apron via storage buffer offsets, no shared memory tricks initially.
- Keep f32 only (no f16 storage until parity tests pass); avoid `pow`, `exp`, `sin` in decision paths; use integer hashing (PCG hash / xxhash32-style) for noise — same code CPU/GPU.
- Request `naga` to not emit fast-math; explicitly call `fma()` in both CPU (`f32::mul_add`) and WGSL where numerically important (pipe flux updates).
- Reductions: workgroup reduce → per-workgroup partials buffer → CPU sums partials in index order (deterministic, cheap).
- Fallback: every node has a CPU implementation; GPU is an accelerator chosen per node with a `Backend` enum; the cache key includes backend when tolerance tier isn't bit-exact.

### Keeping CPU and GPU paths matching
1. Shared test harness: golden tiles computed on CPU with fixed seed; GPU output compared with per-tier tolerance (§16); run in CI with `wgpu` software (lavapipe/SwiftShader) adapter.
2. Same traversal order for sequential algorithms (priority-flood stays CPU-only; its outputs are uploaded).
3. Same boundary policy enum (`Clamp`, `Mirror`, `Periodic`, `Open`) implemented once per backend and property-tested at tile seams.
4. Iteration count & Δt are node parameters, never derived from wall-clock or thread count.
5. Noise: integer lattice hashing + float gradient dot in both; no permutation table.

### Crate shortlist
`rayon`, `wgpu` + `naga`, `fastnoise-lite` or `opensimplex2`, `rand_pcg`/`rand_xoshiro`, `blake3`, `half`, `memmap2`, `meshopt` (meshoptimizer bindings), `mikktspace` (bevy fork), `delaunator`/`spade`, `spa`, `image`/`tiff`/`exr` for I/O, `gltf`/`obj` writers, `serde` + `postcard` for stable param hashing.

---

## Gaps & open verification items
1. Coastal erosion: no computer-graphics paper located (searched ACM/EG/CGF); only GIS work (Brown 2006). Treat our coastal node as a documented heuristic.
2. Chiba et al. 1998 (first particle erosion) and Mikkelsen 2008 thesis: widely cited, not resolved to a DOI/landing page today — cite with "[M]" or fetch Wiley/KU repositories before publication.
3. DOIs marked [M] (Perlin 1985/2002, Musgrave 1989, Lorensen 1987, Losasso–Hoppe 2004, Ju 2002, Hošek–Wilkie 2012, Preetham 1999, Bridson 2007, Hoppe 1999, Garland–Heckbert 1997, Fournier–Reeves 1986, Kass–Miller 1990) are standard and highly likely correct but were not resolved in this session.
4. Mei 2007's exact pipe formulation vs. generic SWE: abstract says "shallow-water fluid model"; the virtual-pipe details are in the PDF (INRIA page) — confirm before quoting equations.
5. Olsen 2004 is a tech report (no DOI); the MIT mirror PDF is the accessible copy.
6. Simplex patent: Google Patents "Expired – Lifetime", anticipated expiry 2022-01-08; no USPTO maintenance-fee record checked. Sufficient for an OSS project but not legal advice.
7. Lengyel 2010 dissertation and Transvoxel: citation confirmed via secondary sources; PDF at terathon.com not opened.

---

## Appendix A — Core equations and reference pseudo-code for the recommended algorithms

All pseudo-code is gather-form (each cell reads neighbours, writes itself) unless noted, so the same loop body is valid for a rayon row-chunk and a WGSL invocation.

### A.1 fBm family with analytic derivatives

```
fn fbm(p: vec2, K, lac, gain, H) -> (value, dvalue/dp):
    a = 1; f = 1; sum = 0; dsum = (0,0); norm = 0
    for k in 0..K:
        (n, dn) = noise_with_deriv(p * f + offset_k)      // offset_k = hash(seed,k) to break lattice alignment
        sum  += a * n;  dsum += a * f * dn;  norm += a
        a *= gain; f *= lac
    return (sum / norm, dsum / norm)

ridged:  n' = (1 - |n|)^2 ; weight_k = clamp(prev * ridge_gain, 0, 1)   // Musgrave ridged multifractal
hybrid:  weight_k = clamp(prev_signal, 0, 1); signal = (n + offset) * a; sum += signal * weight
slope-damped (Quilez / "uber"): a_k *= 1 / (1 + erosion_strength * |dsum|^2)
warp:    q = fbm(p); r = fbm(p + 4*q); h = fbm(p + 4*r)
```
Normalise by `norm` so output range is independent of K; derivative chain rule through warp: dh/dp = dfbm(p+4r)/dp · (I + 4·dr/dp).

### A.2 Virtual-pipes hydraulic erosion (Mei 2007; Šťava 2008 layers)

State per cell: terrain height `b`, water depth `d`, sediment `s`, outflow flux `f[L,R,T,B]`, velocity `v`.
Constants: `A` pipe cross-section, `l` pipe length (= Δx), `g`, `Δt`, `Kc` capacity, `Ks` dissolve, `Kd` deposit, `Ke` evaporation.

```
1 rain:        d1 = d + Δt * r(x,y)
2 flux:        for dir in {L,R,T,B}:
                 Δh   = (b + d1) - (b_n + d1_n)
                 f'   = max(0, f[dir] + Δt * A * g * Δh / l)
               K = min(1, d1 * Δx*Δy / ((Σ f') * Δt))        // scale so outflow ≤ stored water
               f = K * f'
3 water:       ΔV = Δt * (Σ inflow − Σ outflow);  d2 = d1 + ΔV / (Δx Δy)
4 velocity:    ΔWx = ½ (fin_L − fout_L + fout_R − fin_R); u = ΔWx / (Δy * (d1+d2)/2);  v likewise
5 capacity:    C = Kc * max(sin α, min_tilt) * |v|            // optionally * clamp(d, 0, 1) to limit deep-water erosion
6 erode/dep:   if C > s:  Δ = Ks * hardness * (C − s); b −= Δ; s += Δ
               else:      Δ = Kd * (s − C);            b += Δ; s −= Δ
7 advect s:    s(x) = s_old(x − v Δt)   // semi-Lagrangian, bilinear
8 evaporate:   d = d2 * (1 − Ke Δt)
9 thermal:     talus step (A.3) on b, per layer if layered
```
Stability: Δt·√(g·d_max)/Δx ≲ 0.5 (CFL); step 2's K-scaling is what keeps the pipe model from going negative; use Δt ≈ 0.02–0.05 with A=l=1, g=9.81, Δx=1. Boundary modes: open (flux leaves), closed (f=0 across edge), periodic. Tile halo: steps 2–4 read 1 neighbour; step 7 reads ⌈|v|Δt⌉ ≤ 1 cell if CFL holds; so apron=2 covers one full substep; re-exchange halos every `apron−1` substeps.

### A.3 Thermal (talus) step, Jacobi form

```
for each cell c:
    excess = 0; total = 0
    for n in N8:  dh = b[c] − b[n] − talus * dist(c,n);  if dh > 0: {excess_n = dh; total += dh}
    move = rate * Δt * min(total, max_excess) / 2            // ≤ half to avoid overshoot
    out[c] = b[c] − move
    for each n with excess_n > 0:  gather-form: each cell also adds Σ (moves directed at it from neighbours)
```
Implement as two passes (compute `move_n` per directed edge into a 8-plane or recompute symmetric terms in the gather) to stay atomic-free.

### A.4 Droplet erosion (Beyer 2015)

```
for i in 0..num_drops:                     // per tile: sequential; rng = Pcg32::seed(hash(seed,node,tile,i))
    pos = rng.uniform_in_tile(); dir = 0; speed = 1; water = 1; sediment = 0
    for life in 0..max_lifetime:
        (h, grad) = bilinear_height_and_gradient(pos)
        dir = normalize(dir * inertia − grad * (1 − inertia))
        if |dir| == 0: dir = rng.unit_vector()
        new_pos = pos + dir
        if out_of_bounds(new_pos): break           // or hand off to neighbour tile queue
        Δh = height(new_pos) − h
        capacity = max(−Δh, min_slope) * speed * water * capacity_factor
        if sediment > capacity or Δh > 0:
            amount = (Δh > 0) ? min(Δh, sediment) : (sediment − capacity) * deposit_speed
            deposit_bilinear(pos, amount); sediment −= amount
        else:
            amount = min((capacity − sediment) * erode_speed * hardness(pos), −Δh)
            erode_brush(pos, amount, radius)     // weights ∝ (radius − dist), normalised; multiply by hardness
            sediment += amount
        speed = sqrt(max(0, speed² + Δh * gravity)); water *= (1 − evaporate_speed); pos = new_pos
```
Determinism: a drop's writes depend on earlier drops; fixing per-tile sequential order + seeded rng makes results reproducible for a given tile size. Changing tile size changes results — put tile size in the cache key.

### A.5 Stream-power erosion (Cordonnier 2016; implicit Braun–Willett)

Receivers from D8 (after priority-flood to remove pits, or route through lakes). Drainage area `A` via donor stack. Update, implicit in h:
```
for cells in upstream-to-downstream order (stack reversed):
    r = receiver(c); if r == c: continue
    f = K * A^m * Δt / Δx^n            // n = 1 case
    h[c] = (h[c] + Δt*U[c] + f * h[r]) / (1 + f)
```
m≈0.5, n=1, K≈1e-5…1e-4 (units m^(1−2m)/yr), U uplift ≈ 1e-4…1e-3 m/yr, Δt ≈ 1e3–1e4 yr, Δx ≈ 100–1000 m. Add hillslope diffusion `h += D Δt ∇²h` (D≈0.01–1 m²/yr) for smooth ridges, and re-route flow every step. Order dependency → CPU (or GPU with level-set parallelism over the donor tree depth, as Schott 2023).

### A.6 Priority-flood (Barnes 2014) with epsilon drainage

```
open = BinaryHeap<(h, idx)>; push all border cells; mark closed
while let Some((h, c)) = open.pop_min():
    for n in N8(c) not closed:
        closed[n] = true
        if h[n] < h + ε: { lake[n] = h + ε − h[n]; h[n] = h + ε }   // ε > 0 ensures drainable flats
        open.push((h[n], n)); label[n] = label[c]
```
Tie-break on `idx` in the heap ordering for determinism. O(N log N). Memory: h (f32), label (u32), closed (bitset), heap ≤ N entries × 8 B.

### A.7 D∞ flow direction (Tarboton 1997)

For each of 8 triangular facets (centre, cardinal e1, diagonal e2): s1 = (h0 − h1)/Δx, s2 = (h1 − h2)/Δx; r = atan2(s2, s1); clamp r to [0, π/4] (if r<0 → r=0, s=s1; if r>π/4 → r=π/4, s=(h0−h2)/(√2Δx)). Pick facet with max slope; angle = facet_base ± r. Flow splits to the two neighbours with weights (π/4 − r)/(π/4) and r/(π/4). Accumulation then needs a topological order (sort by height desc, deterministic tie-break on index).

### A.8 Horizon / AO sweep (Timonen 2010, simplified)

For azimuth θ in D directions: for each scanline along θ (rasterise lines with fixed start offsets), maintain a convex hull of (distance, height) seen so far; horizon angle at cell = max over hull. AO = 1 − (1/D) Σ_θ sin(horizon_θ)… (cosine-weighted sky visibility uses ∫ cos over the unoccluded cap = 1 − sin²(horizon)/… choose one convention and document). D = 16 is adequate for masks, 64 for baking.

### A.9 Bridson Poisson-disk (per tile, density map)

Grid cell = r_min/√2; active list; k=30 candidates in annulus [r(p), 2r(p)] where r(p) = r_min / sqrt(density(p)); accept if no existing sample within r(candidate) (check neighbourhood of ⌈2·r_max/cell⌉ cells). Apron: include neighbour tiles' accepted samples within r_max of the border as read-only "existing" samples; process tiles in fixed order (row-major) so the dependency is well defined. Output: positions + rng-derived rotation/scale.

### A.10 CDLOD selection & morph

```
select(node, cam):
    if !sphere_intersects(node.bounds, range[node.lod]): return false   // whole node too far for this LOD
    if node.lod == 0 or !sphere_intersects(node.bounds, range[node.lod−1]): draw(node); return true
    for child: if !select(child): draw_partial(child)   // child at this node's LOD
vertex shader: morph = clamp((dist − range_lo) / (range_hi − range_lo), 0, 1); pos.xy += morph * (snap_to_coarser(pos.xy) − pos.xy); z = height_texture(pos.xy)
```
Ranges: range[l] = base_range · 2^l; grid mesh 32×32 or 64×64 per node; heights from a tiled height texture (R32F or R16 + scale).

### A.11 RTIN (Evans 2001 / MARTINI) build

Grid (2^k+1)². Recursively split the two root triangles; for each triangle store error = |h(midpoint of hypotenuse) − avg(h(hyp endpoints))|, then propagate `error[parent] = max(error[parent], error[children])` bottom-up. Mesh for tolerance τ: traverse, subdivide while error > τ. O(N) build, O(output) query; deterministic. Tile = one RTIN; neighbouring tiles share edge errors to avoid cracks (or add skirts).

### A.12 Heitz–Neyret histogram-preserving blend

Precompute: T (texture → Gaussian) via histogram sort; T⁻¹ as 1D LUT per channel (plus optional PCA decorrelation). Shade: triangle-grid lattice over UV; 3 vertices → 3 random offsets (hash); G = (w1·T(x+o1) + w2·T(x+o2) + w3·T(x+o3) − 0.5)/√(w1²+w2²+w3²) + 0.5; colour = T⁻¹(G). Works on albedo/normal/roughness alike (normals need unpack/repack).

---

## Appendix B — Default parameter ranges (starting points for node UIs)

| Node | Parameter | Default | Range | Notes |
|---|---|---|---|---|
| noise | frequency (cycles/km) | 1 | 0.05–50 | world-unit based so tile size doesn't change output |
| fbm | octaves | 8 | 1–16 | stop when 1/(f·lac^k) < cell size |
| fbm | lacunarity | 1.97 | 1.5–3 | non-integer avoids lattice alignment (unless periodic) |
| fbm | gain | 0.5 | 0.2–0.8 | H = −log2(gain) |
| ridged | offset | 1.0 | 0.5–1.5 | |
| warp | amplitude | 0.4 | 0–2 | in units of base wavelength |
| pipes | Δt | 0.03 | 0.005–0.1 | CFL-checked; UI shows max safe |
| pipes | rain | 0.01 | 0–0.1 | per cell per step (water depth units) |
| pipes | Kc / Ks / Kd | 1.0 / 0.5 / 1.0 | 0–10 / 0–1 / 0–1 | |
| pipes | evaporation | 0.015 | 0–0.1 | per step |
| pipes | min tilt | 0.02 | 0–0.2 | sin α floor |
| pipes | iterations | 1000 | 100–20000 | |
| thermal | talus angle | 35° (sand) / 45° (soil) / 75° (rock) | 10–85° | per material layer |
| thermal | rate | 0.5 | 0–1 | fraction of excess per step |
| droplet | drops | 2·N / 10 | — | ~200k for 1k² |
| droplet | lifetime | 30 | 10–100 | |
| droplet | inertia | 0.05 | 0–0.3 | |
| droplet | capacity | 4 | 1–16 | |
| droplet | min slope | 0.01 | 0.001–0.1 | |
| droplet | erode / deposit speed | 0.3 / 0.3 | 0–1 | |
| droplet | evaporate | 0.01 | 0–0.1 | |
| droplet | gravity | 4 | 1–10 | |
| droplet | radius | 3 | 1–8 cells | |
| stream power | K | 2e-5 | 1e-6–1e-3 | |
| stream power | m / n | 0.5 / 1 | 0.3–0.7 / 1–2 | n=1 keeps update linear |
| stream power | Δt | 2000 yr | 100–20000 | |
| stream power | uplift | 5e-4 m/yr | 0–5e-3 | map-driven |
| priority-flood | ε | 1e-4 · Δh_range | ≥ 1 ULP | |
| TWI | min tanβ | 0.001 | | |
| curvature | scale (cells) | 1, 4, 16 | | pyramid levels |
| AO | directions / max dist | 16 / 256 cells | 8–64 | |
| Bridson | r_min / k | species-defined / 30 | | |
| CDLOD | base range / grid | 2 × node size / 32 | | τ ≈ 1 px |
| RTIN | τ | 0.5 m | | in height units |
| decimate | target error / ratio | 0.1 % bbox / 0.25 | | lock borders for tiles |

---

## Appendix C — Consolidated bibliography (by area)

Noise
- Perlin 1985, "An Image Synthesizer," SIGGRAPH '85, DOI 10.1145/325165.325247 [M]
- Perlin 2002, "Improving Noise," SIGGRAPH 2002, DOI 10.1145/566654.566636 [M]
- Perlin 2001 simplex (course notes); Gustavson 2005 "Simplex noise demystified" [M]; US 6,867,776 B2 expired 2022-01-08 [V]
- KdotJPG, OpenSimplex2 (GitHub, CC0); `opensimplex2` crate v1.1.0; `fastnoise-lite` crate [V]
- Musgrave, Kolb, Mace 1989, SIGGRAPH '89, DOI 10.1145/74334.74337 [M]
- Ebert et al., *Texturing & Modeling: A Procedural Approach*, 3rd ed. 2003 [M]
- Quilez, "Domain Warping," iquilezles.org/articles/warp/ [M]
- Worley 1996, SIGGRAPH '96, DOI 10.1145/237170.237267 [M]
- Saupe 1988, in *The Science of Fractal Images* [M]
- Fournier, Fussell, Carpenter 1982, CACM 25(6), DOI 10.1145/358523.358553 [M]; Miller 1986 SIGGRAPH [M]
- Lagae et al. 2009, ACM TOG 28(3), DOI 10.1145/1531326.1531360 [V]
- Tricard et al. 2019, "Procedural phasor noise," ACM TOG 38(4), DOI 10.1145/3306346.3322990 [V]
- Hello Games / Murray, GDC 2017 "Building Worlds in No Man's Sky Using Math(s)" [M]

Hydraulic / thermal erosion
- Olsen 2004, "Realtime Procedural Terrain Generation," IMADA SDU tech report, https://web.mit.edu/cesium/Public/terrain.pdf [V]
- Beneš & Forsbach 2001, SCCG, DOI 10.1109/SCCG.2001.945341 [V]
- Beneš & Forsbach 2002, "Visual simulation of hydraulic erosion," J. WSCG 10 [V]
- Mei, Decaudin, Hu 2007, PG 2007, DOI 10.1109/PG.2007.15, http://www-evasion.inrialpes.fr/Publications/2007/MDH07/ [V]
- Šťava, Beneš, Brisbin, Křivánek 2008, SCA, DOI 10.2312/SCA/SCA08/201-210 [V]
- Krištof, Beneš, Křivánek, Šťava 2009, CGF 28(2), DOI 10.1111/j.1467-8659.2009.01361.x [V]
- Chiba, Muraoka, Fujita 1998, J. Vis. Comput. Anim. 9(4) [M — not located today]
- Beyer 2015, bachelor thesis TUM, https://www.firespark.de/?id=project&project=HydraulicErosion [V]
- Lague 2019, github.com/SebLague/Hydraulic-Erosion [M]
- McDonald, github.com/weigert/SimpleHydrology [V]
- McDonald & Cordonnier 2026, ACM TOG 45(4), DOI 10.1145/3811336 [V]
- Cordonnier et al. 2016, CGF 35(2), DOI 10.1111/cgf.12820 [V]
- Cordonnier et al. 2017, ACM TOG 36(4), DOI 10.1145/3072959.3073667 [V]
- Schott et al. 2023, ACM TOG 42(5), DOI 10.1145/3592787, github.com/H-Schott/StreamPowerErosion [V]
- Schott et al. 2024, ACM TOG 43(4), DOI 10.1145/3658200, github.com/H-Schott/MultiScaleErosion [V]
- "Efficient Debris-flow Simulation for Steep Terrain Erosion," ACM TOG 2024, DOI 10.1145/3658213 [V]
- Guérin et al. 2017, ACM TOG 36(6), DOI 10.1145/3130800.3130804 [V] (note only)
- Braun & Willett 2013, Geomorphology 180–181, DOI 10.1016/j.geomorph.2012.10.008 [M]

Aeolian / glacial / snow / geology
- Werner 1995, Geology 23(12), DOI 10.1130/0091-7613(1995)023<1107:EDCSAA>2.3.CO;2 [V]
- Narteau, Zhang, Rozier, Claudin 2009, JGR 114 F03006, DOI 10.1029/2008JF001127 [V]
- Paris, Peytavie, Guérin, Argudo, Galin 2019, "Desertscapes Simulation," CGF 38(7), DOI 10.1111/cgf.13815, github.com/aparis69/Desertscapes-Simulation [V]
- Argudo, Galin, Peytavie, Paris, Guérin 2020, ACM TOG 39(6), DOI 10.1145/3414685.3417855 [V]
- "Forming Terrains by Glacial Erosion," ACM TOG 2023, DOI 10.1145/3592422 [V]
- Festenberg & Gumhold 2011, CGF 30(6), DOI 10.1111/j.1467-8659.2011.01904.x [V]
- Goswami 2024, "Snow and Ice Animation Methods in Computer Graphics," CGF, DOI 10.1111/cgf.15059 [V]
- Cordonnier, Cani, Beneš, Braun, Galin 2018, IEEE TVCG 24(5), DOI 10.1109/TVCG.2017.2689022 [V]
- Brown 2006, Computers, Environment and Urban Systems 30(6) (coastal, GIS) [V]
- Coastal erosion in graphics venues: UNVERIFIED — could not locate

Hydrology / analysis
- O'Callaghan & Mark 1984, CVGIP 28(3) [M]
- Quinn et al. 1991, Hydrological Processes 5(1) [M]; Freeman 1991, Computers & Geosciences 17(3) [M]
- Tarboton 1997, WRR 33(2), DOI 10.1029/96WR03137 [V]
- Barnes, Lehman, Mulla 2014, Computers & Geosciences 62, DOI 10.1016/j.cageo.2013.04.024, arXiv 1511.04463 [V]
- Barnes 2016, parallel priority-flood, arXiv 1606.06204 [M]
- Strahler 1957, Trans. AGU 38(6) [M]; Beven & Kirkby 1979, Hydrol. Sci. Bull. 24(1) [M]
- Génevaux et al. 2013, ACM TOG 32(4), DOI 10.1145/2461912.2461996, HAL hal-01339224 [V]
- Horn 1981, Proc. IEEE 69(1) [M]
- Zevenbergen & Thorne 1987, ESPL 12(1), DOI 10.1002/esp.3290120107 [V]
- Evans 1979/1980 (Durham) [M]
- Max 1988, The Visual Computer 4(2) [V]
- Timonen & Westerholm 2010, CGF 29(2), DOI 10.1111/j.1467-8659.2009.01642.x [V]

Volumetric / planetary
- Lorensen & Cline 1987, SIGGRAPH '87, DOI 10.1145/37401.37422 [M]
- Lengyel 2010, PhD UC Davis, terathon.com/lengyel/Lengyel-VoxelTerrain.pdf [V citation]
- Ju, Losasso, Schaefer, Warren 2002, SIGGRAPH 2002 / ACM TOG 21(3) [V]; DOI 10.1145/566654.566586 [M]
- Peytavie, Galin, Grosjean, Mérillou 2009, CGF 28(2), DOI 10.1111/j.1467-8659.2009.01385.x [V]
- Paris, Galin, Peytavie, Guérin, Gain 2019, ACM TOG 38(5), DOI 10.1145/3342765, github.com/aparis69/Implicit-Volumetric-Terrains [V]
- Becher, Krone, Reina, Ertl 2019, IEEE TVCG 25(2), DOI 10.1109/TVCG.2017.2762304 [V]
- Cortial, Peytavie, Galin, Guérin 2019, CGF 38(2), DOI 10.1111/cgf.13614 [V]; 2020 Visual Computer DOI 10.1007/s00371-020-01923-4 [V]
- Thorne 2005, floating origin, CW 2005 [M]

LOD / meshing / decimation
- Duchaineau et al. 1997, ROAM, IEEE Vis '97 [M]
- Ulrich 2002, chunked LOD, SIGGRAPH 2002 course notes, tulrich.com/geekstuff/chunklod.html [V]
- Losasso & Hoppe 2004, ACM TOG 23(3), hhoppe.com/proj/geomclipmap/ [V]; Asirvatham & Hoppe 2005 GPU Gems 2 ch. 2 [V]
- Strugar 2009/2010, CDLOD, JGT, DOI 10.1080/2151237X.2009.10129287, github.com/fstrugar/CDLOD [V]
- Evans, Kirkpatrick, Townsend 2001, Algorithmica 30(2), DOI 10.1007/s00453-001-0006-x [V]; Mapbox MARTINI [M]
- Garland & Heckbert 1995, CMU-CS-95-181 [M]; Garland & Heckbert 1997, SIGGRAPH '97, DOI 10.1145/258734.258849 [M]
- Hoppe 1999, IEEE Vis '99, hhoppe.com/proj/newqem/ [V]
- Rossignac & Borrel 1993, Springer, DOI 10.1007/978-3-642-78114-8_29 [V]
- Botsch & Kobbelt 2004, SGP, DOI 10.2312/SGP/SGP04/189-196 [V]
- Kapoulkine, meshoptimizer, github.com/zeux/meshoptimizer [M]
- Mikkelsen 2008, MSc thesis Univ. Copenhagen; github.com/mmikk/MikkTSpace [M]

Texturing / water / sky / vegetation
- Heitz & Neyret 2018, PACMCGIT 1(2), DOI 10.1145/3233304 [V]; Deliot & Heitz 2019 JCGT [V]
- Geiss 2007 GPU Gems 3 ch. 1 [M]; Glanville 2004 GPU Gems ch. 20 [M]; Mittring 2008 SIGGRAPH course [M]
- Tessendorf 2001 SIGGRAPH course notes [M]; Fournier & Reeves 1986 SIGGRAPH, DOI 10.1145/15922.15894 [M]; Kass & Miller 1990 SIGGRAPH DOI 10.1145/97880.97884 [M]
- Vlachos 2010, SIGGRAPH Advances course, advances.realtimerendering.com [V]
- Preetham, Shirley, Smits 1999 SIGGRAPH [V venue]; Hošek & Wilkie 2012 ACM TOG 31(4), cgg.mff.cuni.cz/projects/SkylightModelling/ [V]
- Bruneton & Neyret 2008, CGF 27(4), DOI 10.1111/j.1467-8659.2008.01245.x [V]
- Hillaire 2020, CGF 39(4), DOI 10.1111/cgf.14050 [V]; Hillaire 2016 Frostbite PBS course PDF [V]; Schneider & Vos 2015 SIGGRAPH Advances [M]
- Meeus 1998 *Astronomical Algorithms* [M]; Reda & Andreas 2004 NREL SPA [M]; crates `spa` 0.5.1, `sun` 0.3.1 [V]
- Bridson 2007 SIGGRAPH Sketches, cs.ubc.ca/~rbridson/docs/bridson-siggraph07-poissondisk.pdf [V]
- Cohen et al. 2003 Wang tiles [M]; Kopf et al. 2006 recursive Wang tiles [M]
- Deussen et al. 1998 SIGGRAPH, DOI 10.1145/280814.280898 [V]
- Whittaker 1975 [M]; Holdridge 1947 Science 105, DOI 10.1126/science.105.2727.367 [M]

Determinism / infra
- BLAKE3 (O'Connor et al. 2020) [M]; O'Neill 2014 PCG [M]; Blackman & Vigna 2021 xoshiro, ACM TOMS 47(4) [M]
