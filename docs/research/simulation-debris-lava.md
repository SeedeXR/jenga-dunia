# Debris and lava simulation (2026-10-24)

Why: World Creator lists Debris and a Fluid layer whose viscosity goes "water → lava" among its
simulation layers (`world-creator.md` §3.8), and the owner asked for both. This note records the
models `jd.erosion.debris@1` and `jd.volcanic.lava@1` follow, where each number comes from, and
what was measured. Tags: **VERIFIED** (read in the source), **REPORTED** (secondary source or
abstract), **MEASURED** (our run), **OURS** (a choice of ours, with the reason).

## Lava: MAGFLOW / GPUFLOW

- **Model family.** MAGFLOW is a cellular automaton whose transition rule is "a steady state
  solution of Navier-Stokes equations … for isothermal laminar pressure-driven Bingham fluid",
  with "radiative heat loss only from the surface of the flow" and temperature mixing between cells
  (EGU 2008 abstract, https://meetings.copernicus.org/www.cosis.net/abstracts/EGU2008/02644/EGU2008-A-02644.pdf). REPORTED.
- **Equations.** GPUFLOW (Cappello, Bilotta & Ganci 2022, *Appl. Sci.* 12 4395, open access via
  https://www.earth-prints.org/server/api/core/bitstreams/0f64a484-effb-40a3-91af-419e1994ab8b/content),
  §2.1, VERIFIED from the PDF text:
  - eq. 1, critical thickness: `h_cr ≅ S_y √(Δz² + Δx²) / (ρ g (Δz + Δh))`, Δz the ground
    difference "including any contribution from solidified fluid", Δh the liquid difference;
  - eq. 2, flux when the difference exceeds `h_cr`: `q = S_y h_cr² Δx / (3η) (a³ − 3/2 a² + 1/2)`,
    `a = h / h_cr`;
  - eq. 3, emissivity of 'a'ā lava: `ε(T) ≈ 0.976716 + 0.0000408881 T − 1.95062·10⁻⁷ T²`, fitted over
    773–1373 K;
  - Moore neighbourhood with a diagonal correction factor, optimum "α = 2⁻⁶" for a circular
    spread on a flat plane (§2.1.1; the exponent's sign is from extracted PDF text);
  - per-cell maximum time step, reduced to the global minimum each iteration (§2.2).
- **Etna parameters.** "density (2600 kg m⁻³); specific heat capacity (1150 J kg⁻¹ K⁻¹); emissivity
  (0.9); solidification temperature (1173 K) extrusion temperature (1360 K)" (Del Negro et al. 2013,
  *Sci. Rep.* 3 3493, Europe PMC PMC3861846). GPUFLOW uses 1360 K and 1143 K. VERIFIED. Our defaults:
  2600, 1150, 1360 K, 1143 K.
- **Rheology vs temperature.** MAGFLOW uses empirical viscosity laws (Giordano & Dingwell 2003 for
  hydrous Etna basalt) and a temperature-dependent yield strength (Ishihara et al. 1990). REPORTED:
  we could not open either paper, so we do **not** ship their coefficients. OURS: viscosity and
  yield strength rise log-linearly from the vent value to the solidus value (`viscosity_decades`,
  `yield_decades`), defaults 10⁴ Pa·s → 10⁷ and 10³ Pa → 10⁵.

### What we changed, and why (MEASURED)

| Finding | Evidence | Decision |
|---|---|---|
| `a³ − 3/2 a² + 1/2` cancels catastrophically for `a` just above 1 and went negative in f32, giving a negative step and 200 000 wasted steps | lava unit test stalled at 3 929 of 14 400 s | evaluate as `(a − 1)² (a + 1/2)`, the exact factorisation |
| GPUFLOW's 2⁻⁶ diagonal factor makes a diamond with our face widths | flat-plane spread, covered area / πr²: 0.71 at 2⁻⁶, 0.86 at ¼, 0.98 at ½, **1.01 at 1** | no factor: the flux crosses a face `cell area / Δx` wide, which already narrows diagonals |
| Frozen lava added to a 3 000 m ground in f32 loses thin films | Etna run: 2.508·10⁶ m³ on the ground for 2.592·10⁶ erupted (−3 %) | keep the frozen thickness in its own array: now exact (2.592·10⁶ = 2.592·10⁶) |
| Radiating at the bulk temperature (MAGFLOW's rule) freezes the front into a tower | Etna DEM, 30 m³/s for 24 h, 14.8 m cells: max 70 m thick, 0.30 km² | `cooling` = 0.3 by default (a crust cooler than the core radiates less): max 25 m, 0.69 km², molten cells 12 h after the eruption |
| Run-out depends on cell size | same eruption: ~1.1 km at 25 m cells, 2.9 km (map units) at 12 m | documented: evaluate lava at the final resolution, not a coarse preview |
| A spent step budget used to end the run silently | runny lava (10² Pa·s) used 200 000 steps for 9 of 36 h | the node now fails with the simulated time reached and what to change |

Etna figure (`docs/user/images/cli-lava.png`): Terrarium DEM around 37.738 N 15.030 E, radius 5 km,
1024² (12.35 m map units; EPSG:3857, so ground distances are ×0.79), vent at 2 940 m on the
south-east flank, 30 m³/s for 24 h: 68 s in a debug build, a 2.9 km (map) / 2.3 km (ground) tongue,
at most 11 m thick, still molten at the end of the eruption; 12 h later frozen, at most 12 m.

## Debris: rockfall run-out

- **Energy line (Fahrböschung).** Heim (1932): the angle from the top of the detachment zone to the
  furthest block, "not smaller than 30°"; Evans & Hungr (1993, *Can. Geotech. J.* 30 620–636): the
  shadow angle measured from the talus apex, "usually … not smaller than 27°"; reported field values
  for φp span 22°–37° (EGU 2005 abstract https://meetings.copernicus.org/www.cosis.net/abstracts/EGU05/01783/EGU05-J-01783.pdf;
  GRASS `r.droka` manual https://grass.osgeo.org/grass84/manuals/addons/r.droka.html). REPORTED (secondary).
  Default `reach_deg` 30°.
- **Angle of repose of scree** 35° (the thermal node's existing default, "scree ≈ 35–40").
- **Fall sorting** — bigger blocks travel further down a scree slope — is the textbook expectation
  (Kirkby & Statham 1975, *J. Geol.* 83, "Surface stone movement and scree formation"); we could not
  read the paper, so the `sorting` parameter is OURS: it moves each block's stopping angle from
  `reach_deg` (largest) towards the repose angle (smallest).
- **Model (OURS).** Sources steeper than `source_slope_deg` (45°) release `release_m`; blocks of
  volume ∝ size³ share it; a block steps downhill (probability ∝ slope) while the ground ahead is
  steeper than the repose angle or the energy line back to its source is steeper than its reach angle;
  it deposits where it stops. Mass is conserved exactly (unit test; mesa run: mean height unchanged to
  1 mm). Not modelled: bouncing, fragmentation, a ballistic first leg.

Mesa figure (`docs/user/images/cli-debris.png`): 1 024 m at 2 m cells, noise 0–160 m terraced into
40 m steps, defaults: 278 ms, 29 748 cells hold blocks, deposits up to 6.8 m, cliffs lowered 0.5 m;
`terrain.scatter {mask: "debris", rock: {}, spacing_m: 3}` placed 5 005 boulders.
