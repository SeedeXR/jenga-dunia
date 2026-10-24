# Reading a landscape: from a reference image to a believable world

This guide is for anyone, person or AI agent, building a world from a reference photo, a 360° panorama or a video. The MCP server serves it as the resource `jd://guide/reading-a-landscape`, and its `terrain_from_reference` prompt follows it step by step.

## What a picture can and cannot tell you

A photo is evidence, not a height map. Treat it the way an environment artist does: match what it shows and build the rest from how such landscapes form.

| A photo is good evidence of | A photo is weak or silent on |
|---|---|
| The skyline and the order in which ridges stand back | Absolute scale: a 50 m dune and a 300 m dune can look alike |
| The landform type and the process that made it (dunes, mesas, glacial valleys, volcanoes) | Everything behind the first ridge, and the far side of every hill |
| Rock layering, talus and scree angles, where snow lies | Anything only visible from above: drainage layout, basin shapes |
| Materials, vegetation and the snow line | Heights: monocular depth read a valley about a kilometre wide as 5–19 m deep in our own test |
| Light: sun side, time of day, haze | Units: depth models guess, and DA3 was an order of magnitude too close on a synthetic scene |

So the method is: read the facts the file states, read the landform from the picture, start from real elevation data where you can, simulate the processes the picture shows, then measure the result against a real terrain of the same kind.

## The steps

1. **Read the file's metadata.** `media.metadata {path}` returns the GPS position, the UTC capture time, the compass heading, the field of view and the projection of 360° images. It also returns the sun's azimuth and elevation at that place and time. Its `suggest` block holds ready params for the next ops, and its `notes` say how far to trust each fact.
2. **Look at the image and fill the reading sheet** below. Mark every field you are unsure of in `unknowns`.
3. **Ground it in real data when the place is known.** Use `geo.fetch_dem` with `suggest.geo_fetch_dem`, set the world's `crs` to the UTM zone and its `origin`, and import with `jd.input.heightmap@1 {georeference: true}`. The real DEM is the base, and the simulations add detail at the scale the DEM cannot resolve.
4. **Otherwise start from the landform recipe** in the table below, sized by the scale cues on the sheet.
5. **Use the picture for what it is good at.** For the visible relief run `vision.depth` with `suggest.vision_depth.fov_x_deg`, then shape the foreground with `jd.sculpt.strokes@1` or blend the relief in with `jd.math.*`. A 360° panorama needs perspective crops first; do not feed the equirectangular image to the depth model.
6. **Set the sky.** Use `suggest.sky` for `sky.preview` and `sky.bake`. The same sun azimuth goes to `jd.climate.snow@1` and `jd.climate.snowpack@1` as `sun_azimuth_deg`.
7. **Set the climate.** Use the world wind from the sheet, then derive `temperature_c` from the snow line: the snow line sits near `temperature_c ÷ lapse_c_per_km` km up.
8. **Measure.** Run `terrain.measure` on your result and on an exemplar DEM of the same landform at the same `search_m`. Compare the slope distribution, the geomorphon shares and the hypsometric integral, and adjust until they are close.
9. **Report.** Tell the user every assumption you made: each `unknowns` entry, the assumed scale, and any magnetic or compass heading.

## The reading sheet

Fill this from the image, one field at a time. Use `null` and add an `unknowns` entry rather than guess.

```json
{
  "landform": "mesa | butte | canyon | transverse dunes | star dunes | alpine peaks | glacial valley | stratovolcano | shield volcano | karst towers | badlands | playa | coast cliffs | rolling hills | …",
  "process": ["fluvial", "aeolian", "glacial", "volcanic", "karst", "coastal", "mass wasting"],
  "rock": { "layered": true, "dip_deg": 5, "dip_toward": "south", "caprock": true },
  "talus_deg": 35,
  "scale": { "cues": ["pine trees ~25 m", "road", "haze on the third ridge"], "relief_m": 400, "width_m": 3000 },
  "climate": { "aridity": "arid | semi-arid | temperate | humid | polar", "snowline_m": null, "treeline_m": null, "vegetation": "none | sparse | grass | forest" },
  "light": { "sun_side": "left", "time_of_day": "morning", "haze": "light" },
  "water": { "rivers": false, "lakes": false, "sea": false },
  "camera": { "height_above_ground_m": 2, "looking": "north" },
  "unknowns": ["the far side of the main butte", "absolute scale"],
  "confidence": { "landform": 0.9, "scale": 0.4 }
}
```

## Landform recipes

Each row is a starting graph built from node kinds this build has; `noise.fbm` stands for the kind `jd.noise.fbm@1`, and so on. Parameters are first guesses to tune against the exemplar.

| Landform | Graph | Key parameters |
|---|---|---|
| Mesas, buttes, stepped canyons | `noise.fbm` → `filter.terrace` → `terrain.strata` (hardness) → `erosion.thermal` and `erosion.stream_power` with that hardness | terrace `step_m` = bed thickness; strata `layer_m` same; thermal `talus_deg` 34 |
| Talus aprons and boulder fields | … → `erosion.debris` → `terrain.scatter {mask: "debris", rock: {}}` | `source_slope_deg` 45, `reach_deg` 30 |
| Transverse dunes | a gentle sheet (`noise.fbm` with 5–10 m of range) → `erosion.aeolian` | one wind; `iterations` 200+ |
| Linear and star dunes | the same with `wind2_azimuth_deg` | `wind2_share` 0.5 |
| Yardangs | `noise.fbm` (ridged) → `erosion.yardang` (+ strata hardness) | `wind_azimuth_deg` along the keels |
| Alpine peaks | `noise.fbm` (ridged) → `erosion.stream_power` → `erosion.thermal` → `climate.snowpack` | thermal `talus_deg` 38; snowpack `avalanche_slope_deg` 38 |
| River valleys | `hydrology.network` (rivers first) or `noise.fbm` → `erosion.hydraulic` → `hydrology.rivers` | hydraulic `iterations` 200+, `rain` from `climate.rain` |
| Rain-shadow ranges | … → `climate.rain` → hydraulic `rain` input and biome rules | `recovery_km` 40, world wind across the range |
| Karst plains and dolines | low `noise.fbm` → `erosion.karst` → `sinkholes` → `hydrology.fill` / `hydrology.rivers` `drains` | `density_per_km2`, `max_slope_deg` 5 |
| Karst towers | ridged noise inverted → `erosion.karst` → `erosion.thermal` (high talus) | steep `talus_deg` 60+ with strata hardness |
| Playas and wadis | low plain → `hydrology.fill` (`lakes` = salt pan) and `hydrology.rivers` (`method: dinf`) | rivers `threshold` 0.4, `depth_m` 3 |
| Coasts | … → `hydrology.sea_level` → `erosion.coastal` → `biome.seabed` → `ocean.bathymetry` | `level_m`, wave exposure |
| Volcanoes | a cone (a real DEM, `sculpt.strokes` or noise) → `volcanic.lava` | `vents`, `eruption_s`, `cooling` 0.3 |
| Landform masks for materials | any height → `analysis.geomorphon` → `mask.class {id}` | ridge 3, footslope 8, valley 9 |

## Exemplars: real terrains to measure against

Fetch one with `geo.fetch_dem {center: [lon, lat], radius_m: 7500, res_m: 40}` into a 10 km, 256² world on its UTM zone, import it, and run `terrain.measure {search_m: 800}`. These are our measurements of Terrarium data on 2026-10-24, at 39 m cells.

| Exemplar | lon, lat | Relief m | HI | Slope mean / p90 ° | Main geomorphons |
|---|---|---|---|---|---|
| Namib dunes (Sossusvlei) | 15.35, −24.75 | 342 | 0.30 | 10.7 / 23.8 | slope 0.31, valley 0.17, hollow 0.14, ridge 0.11, spur 0.11 |
| Monument Valley | −110.11, 36.98 | 485 | 0.26 | 6.7 / 17.5 | slope 0.54, hollow 0.15, spur 0.10, valley 0.08, ridge 0.05 |
| Grand Canyon (Bright Angel) | −112.11, 36.10 | 1505 | 0.39 | 29.2 / 53.2 | slope 0.33, spur 0.21, hollow 0.16, ridge 0.14, valley 0.12 |
| Guilin karst (Yangshuo) | 110.49, 24.78 | 485 | 0.24 | 16.7 / 36.7 | valley 0.25, slope 0.24, spur 0.14, hollow 0.13, ridge 0.10, pit 0.07 |
| Matterhorn | 7.658, 45.976 | 2499 | 0.40 | 24.5 / 46.0 | slope 0.53, hollow 0.20, spur 0.14, valley 0.07 |
| Badlands (South Dakota) | −101.95, 43.75 | 180 | 0.30 | 3.4 / 8.9 | slope 0.21, flat 0.15, valley 0.14, footslope 0.13, ridge 0.11 |
| Salar de Uyuni | −67.49, −20.13 | 14 | 0.54 | 1.2 / 2.2 | flat 0.46, footslope 0.16, shoulder 0.16, ridge 0.07, valley 0.07 |
| Mauna Loa (shield) | −155.608, 19.475 | 1183 | 0.68 | 7.4 / 11.5 | slope 0.75, spur 0.10, hollow 0.05 |
| Etna (stratovolcano) | 14.993, 37.751 | 1790 | 0.38 | 15.7 / 27.0 | slope 0.73, spur 0.15, hollow 0.10 |
| Raw fBm noise, for contrast | — | 1220 | 0.49 | 31.7 / 48.7 | slope 0.33, hollow 0.19, spur 0.19, ridge 0.12, valley 0.12 |
| The same after stream power + thermal | — | 1200 | 0.50 | 30.3 / 38.9 | slope 0.40, hollow 0.19, spur 0.18, ridge 0.10, valley 0.10 |

What the numbers say:

- **Real mountains are gentler on average than raw noise.** The Matterhorn's 24.5° mean against noise's 31.7° is typical. Erosion brings the steep tail down first (p90 48.7° → 38.9°).
- **Pits are rare in eroded country.** Water drains them. Noise and karst have pits (0.03 and 0.07); the Alps and volcanoes have none.
- **Flat ground marks deposition or a pan:** Uyuni 0.46, the Badlands' tables 0.15; mountains and noise have none.
- **The hypsometric integral separates young from old.** Mauna Loa's broad summit gives 0.68; dissected plateaus and dunes sit near 0.3.

The numbers depend on cell size, `search_m` and `flat_deg` (1° here: a direction whose up and down lines of sight cancel within a degree counts as flat, the paper's rule), so compare at the same settings. The Perceived Terrain Realism Metric (Rajasekaran et al. 2022) maps geomorphon shares to a realism score. We have not shipped it, because its input normalisation is defined only in the paper's supplementary data, which we could not read.

## Realism checks before you finish

- The slope distribution and geomorphon shares are close to the exemplar's.
- No forest above the treeline and no snow below the snow line, except on shaded faces.
- Talus slopes sit near their angle of repose: scree 35–40°, sand 30–34°.
- Water runs downhill to an outlet. Basins that hold water are lakes or playas on purpose.
- The sun in `sky` matches the light in the photo.
- Every `unknowns` entry is reported back to the user.

## Sources

`docs/research/image-reference-world-design.md` has the research behind this guide: artist workflows, the academic work on terrain from images and sketches, and what can be automated. `docs/research/media-metadata.md` covers the metadata tags and the sun model.
