# 2026-11-08 — Waterfalls, wetlands, river water and springs (owner directive)

Owner, 2026-11-08: "do all four in that order" — waterfalls, wetlands (swamps, marshes), water
drawn in river channels, and springs placed by clicking — grounded in research, realistic,
working in Unity, Unreal and Blender, with unit, integration and regression tests. Brief §4.2
already requires "Riverbeds and drainage basins" and "Lakes and wetlands".

## Findings

### Waterfalls

- VERIFIED (Canadian Encyclopedia; Wikipedia "Waterfall"): a waterfall is where a river's water
  meets a vertical or near-vertical drop in its bed. No single minimum height is agreed; 20 ft
  (6.1 m) is a common convention. Cascades are a series of small falls; rapids stay in contact
  with a steep, turbulent bed.
- VERIFIED (Mabin 2000, via Wikipedia "Waterfall"): the horizontal distance from the lip to the
  plunge pool is at most about 25 % of the waterfall's height — a mean gradient of 4 (76°).
- VERIFIED (Hayakawa & Oguchi, Geomorphometry 2011; Open Geosciences 2017): DEM knickzone
  detection compares a reach's local gradient with its longer trend and thresholds the excess;
  the stream network comes first, from a flow-accumulation threshold. A gradient threshold alone
  also flags steep rapids, so a minimum drop is needed as well (caveat noted in the same work).
- VERIFIED (Scheingross & Lamb 2016, JGR Earth Surface 121, doi:10.1002/2015JF003620, Table S1:
  75 surveyed plunge pools, California and Hawai'i; computed here from the table):
  drop height 0.6–120 m (median 4 m); pool depth / drop median 0.20 (IQR 0.06–0.42), 0.33 for
  pools at least 0.5 m deep (many pools are part-filled by sediment); pool radius / drop median
  0.57 (IQR 0.27–1.04); pool radius / channel width median 0.53, i.e. the pool's diameter is
  about the channel's width; channel slope above the fall median 0.07.
- VERIFIED (same paper): pools self-adjust their depth to the sediment supply — more sediment,
  shallower pools — so depth is a property to expose, not a constant.

### Wetlands

- VERIFIED (USGS, Cowardin et al. 1979 "Classification of Wetlands and Deepwater Habitats"):
  inland wetlands end at about 2 m of water, the depth to which emergent plants grow.
- VERIFIED (state wetland descriptions, via the same search): shallow marshes hold more than
  ~15 cm (6 in) of water through the year; wooded swamps a few inches; soils saturated.
- VERIFIED (USACE "Recognizing wetlands"): a wetland needs wetland vegetation, hydric soil and
  wetland hydrology; they sit in low spots and floodplains where water stands at or above the
  surface in the growing season.
- VERIFIED (Illinois TWI study; HESS 2023 Halabisky et al., Wetland Intrinsic Potential): TWI
  thresholds are site-specific (3.1–7.3 across Illinois watersheds; one standard deviation above
  the mean was used); MFD routing fits field wetlands best; WIP models a per-cell wetland
  probability from slope, curvature, TWI and depth-to-water and cuts it at 0.5.
- VERIFIED (Nobre et al. 2011, J. Hydrology 404:13–29, doi:10.1016/j.jhydrol.2011.03.051; Rennó
  et al. 2008, Remote Sensing of Environment): HAND — height above the nearest drainage, each
  cell's height above where its flow path meets the stream network — correlates strongly with
  water-table depth; field-optimised classes over 18,000 km² of the lower Rio Negro: ≤ 5.3 m
  waterlogged, 5.3–15 m ecotone, above that slope and plateau (validated on 70 points).
- FOUND IN USE (2026-11-08): a TWI cut at mean + 1σ on gentle synthetic relief picks the
  drainage *lines* (dendritic speckle), not the flats a marsh fills; HAND gives the broad,
  waterlogged valley floors. So the wetness is HAND-based; TWI stays as an optional input.
- NOT FOUND: a published slope cut-off for wetlands. INFERRED: low slope is a proxy only, so it
  enters as a soft term with an exposed limit, not a hard rule.
- INFERRED (Cowardin's classes): persistently flooded cores grow emergent plants (marsh);
  seasonally flooded fringes grow trees (forested wetland, swamp).

### River water

- VERIFIED (Leopold & Maddock 1953, USGS Professional Paper 252): downstream, width ∝ Q^0.5,
  depth ∝ Q^0.4, velocity ∝ Q^0.1 (b + f + m = 1). Later work scatters around these averages
  (e.g. 0.43 / 0.40 / 0.18 for Victorian rivers, Geomorphology 2007).
- NOT FOUND: universal absolute coefficients (Moody & Troutman 2002 fit them per data set).
  INFERRED: use the exponents with a user scale — the largest river's depth and width — and
  discharge proportional to the upstream area, which our flow accumulation already is.

- VERIFIED (Marinho et al. 2024, Remote Sensing 16:613, Negro basin): CDOM absorption 1.9–20.1
  m⁻¹ (higher in the blackwater upper course). VERIFIED (Ocean Optics Web Book, "Absorption by
  oceanic constituents"): CDOM spectral slopes typically 0.012–0.022 nm⁻¹. VERIFIED (CSIRO, Mar.
  Freshw. Res. 45:1209): humic black-water and turbid lakes have Secchi depths 0.25–1.84 m.
  So wetland water is drawn with a(440) = 10 m⁻¹, slope 0.015 — absorption ≈ 0.83 / 1.97 / 8.6
  m⁻¹ at 650 / 550 / 450 nm with pure water's added: blue dies first, the water shows tea-brown.
- FOUND IN USE (2026-11-08): stamping a river section's level onto every cell within its
  half-width floods ground beside a perched reach (lower than the bed; 12 m "deep" on fBm noise).
  Each cell takes its nearest section's level and stays dry when it lies below that section's bed.

### Springs

- No research needed beyond what the rivers node does: a spring adds catchment at a point
  (`carve_rivers_sourced`). The work is placing them by clicking, as cave entrances are placed.

## Decisions (defaults, each exposed as a parameter)

| Property | Default | Grounding |
|---|---|---|
| waterfall minimum drop | 6.1 m | 20 ft convention |
| waterfall minimum gradient | 1.0 (45°) | knickzone steepness; Mabin's 4 needs metre cells |
| plunge pool depth / drop | 0.33 | Table S1, pools ≥ 0.5 m deep |
| plunge pool radius / drop | 0.57 | Table S1 median |
| wetland probability cut | 0.5 | WIP |
| waterlogged above drainage | 5.3 m (HAND) | Nobre et al. 2011 |
| drainage network | ≥ 0.1 km² upstream | INFERRED (channel heads ~0.1–1 km²) |
| wetland standing water | 0.3 m, ≤ 2 m | marsh > 0.15 m; Cowardin 2 m |
| river depth / width | ∝ Q^0.4 / Q^0.5; largest 3 m × 40 m | Leopold & Maddock; scale INFERRED |
| wetland water colour | CDOM a(440) 10 m⁻¹, slope 0.015 nm⁻¹ | Marinho 2024; Ocean Optics |
