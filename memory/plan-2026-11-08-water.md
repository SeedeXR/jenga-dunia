# Plan 2026-11-08 — Waterfalls, wetlands, river water, springs

Owner, 2026-11-08: "do all four in that order, as usual ground yourself, with research facts,
realism, properties and ensure works fine with unity and unreal engine and blender, proper
tests, regression, integration and unit tests; invoke local loop and don't stop until you
finish everything." Research and defaults: `docs/research/research-log/2026-11-08-water-features.md`.

Loop prompt: `/loop Water features per memory/plan-2026-11-08-water.md (waterfalls, wetlands, river water, springs) until all done`

## Shared design

- **Water depth layers** (convention): a published layer named `lakes` or ending in `_water`
  (`river_water`, `wetland_water`) is a water depth in metres. Studio draws them all as one water
  surface (per-cell max depth), coloured by depth; the engine export meshes them.
- **Engine export** (as caves): `terrain.export_profile { water: true }` writes `water.fbx`
  (Unity) / `water.glb` (Unreal) in the terrain frame — the water surfaces of every water layer
  plus each waterfall's sheet — and `water.json` (layers, waterfalls: lip/base u,v,z, drop,
  width, pool). Unity importer instantiates `water.fbx`; Unreal check imports `water.glb`;
  Blender bridge calls `water.export` and imports the glb. Masks (`falls`, `wetland`, `marsh`,
  `swamp`, `rivers`) already travel as weightmaps via `masks`.
- **Studio one-click**: Simulation layers row gains `+ falls` and `+ wetland` (as `+ snow`):
  the node goes in, its outputs are published, water layers drawn.

## Stages

| # | Stage | Status |
|---|---|---|
| W1a | `jd-terrain::falls`: find waterfalls on a river path (lip→base run of steps ≥ min_gradient, drop ≥ min_drop), carve plunge pools (depth 0.33 × drop, radius 0.57 × drop), falls mask; unit tests | done (d97574c) |
| W1b | node `jd.hydrology.waterfalls@1` (in, rivers? → out, falls); op `hydrology.waterfalls {id}` listing them; jd-graph + jd-core tests; schemas | done (d97574c) |
| W1c | Studio: falling-water sheets drawn at each fall (viewport), `+ falls` button; window test | done (d97574c) |
| W2a | `jd-terrain::wetland`: probability from wetness (MFD) × flatness, cut 0.5; marsh core (standing water) / swamp fringe; standing water depth ≤ water_depth_m in micro-depressions; unit tests | done |
| W2b | node `jd.hydrology.wetland@1` (in, wetness? → wetland, marsh, swamp, water); `+ wetland` button publishes `wetland_water` + masks; biome docs; tests | done |
| W3a | rivers node: `water` output — depth ∝ Q^0.4 scaled so the largest river reaches `water_depth_m`, over the channel; tests (exponent, monotonic downstream) | done |
| W3b | Studio draws every water layer (lakes + `*_water`), strip's rivers adds `river_water`; window test | done |
| W4 | Studio Rivers section: rivers nodes listed, **Place springs** arms click-to-place (as cave entrances), springs listed with ×; window test | done |
| W5 | engine export: `water` in export_profile + `water.export` op; Unity importer + check, Unreal check, Blender bridge + test; run the headless engine checks | done |
| W6 | docs (guide sections, figures via guide.rs, CLI quickstart), acceptance item?, gates, CI, push | done (docs, figures; acceptance list unchanged) |

## Follow-ups (owner, 2026-11-08: "pick this up and proceed")

| # | Follow-up | Status |
|---|---|---|
| W7a | a fall as wide as its river: the falls node reads the river's water and spans its wet width at the lip | done |
| W7b | measure `hydrology.waterfalls` (build_falls re-runs the search per Studio evaluation); memo it if it costs | done: 0.26 s wall on 1024² incl. process start — no memo |
| W7c | near-black cliff faces in the viewport: find why (albedo, AO, shading) and fix at the cause | done |
| W7f | the GLB normal map (same root cause): written against the exported mesh, bitangent south, w −1; Blender-verified | done |
| W7d | Studio orbit pan target, so the camera can visit a feature off the world's centre | done: middle / Shift drag, keys interpolate it, Fit resets |
| W7e | Unreal check: triangles None when the NaniteTriangles tag is unreadable | done: the registry tags exist only after a save — saved first (29 066 reported) |

## Log
- 2026-11-08 W1 done (d97574c). Changes from the plan: the pools hold their own water (the node's
  `water` output, published `falls_water`) instead of a fill node, which lakes every hollow (on
  terraces it flooded 207 k cells); W3b's "draw every `*_water` layer" moved forward for it.
  Ribbons follow a jet's free-fall arc at 1.5 m/s with impact foam. Found: on high relief the
  orbit centre (sea level clamped to the terrain's range = its minimum) puts a zoomed-in camera
  underground — a Studio follow-up (orbit the relief's middle, or a pan target), noted for W6.
  The guide figure for falls needs a view near a fall; the orbit only circles the world centre.
- 2026-11-08 W2 done. Changed from the plan: the wetness is HAND (height above the nearest
  drainage, Nobre et al. 2011: ≤ 5.3 m waterlogged) not a TWI cut — on gentle relief the TWI at
  mean + 1σ picked drainage lines (dendritic speckle), HAND gives the broad valley floors. TWI or a
  painted mask can still be wired to `wetness`. Open: wetland water draws with the lakes' clear
  tint; a murkier look for `wetland_water` belongs with W3b's water drawing.
- 2026-11-08 W3 done. The rivers node's `water` output (params `water_depth_m` 3, `water_width_m`
  40) uses each cell's nearest cross-section; ground below that section's bed stays dry (a perched
  reach on fBm otherwise showed 12 m of water). `carve_rivers_area` returns the raw area the mask
  is built from (same log scale). `Simulation::Rivers` (strip's rivers button) publishes `rivers` +
  `river_water`. Wetland water draws tea-brown with a CDOM spectrum (research log). Fixed in
  passing: the wetland simulation test compared `layers.height.hash` (always Null — the layers map
  is name → hash), so its "terrain unchanged" check was vacuous; both tests now read the hash.
- 2026-11-08 W4 done. Rivers card after Caves: Add rivers (= the strip's button), one box per rivers
  node ("Rivers n · k springs", click selects), Place arms `Drawing::Spring` (hint "Click — place a
  spring"), springs listed with ×. The caves card's Place button and point rows became shared
  helpers (`place_button`, `point_rows`, `points`). No viewport marker for springs (the river
  appearing marks it) — follow-up if wanted.
- 2026-11-08 W5 done. `jd-terrain::water`: `water_surface` (the terrain grid lifted by the merged
  depth, triangles touching a wet cell; dry corners on the bank) and `fall_sheets` (the jet-arc
  ribbon, now shared with Studio; two-sided as geometry — a back copy, flipped normals). One mesh
  `water` (surfaces + sheets): `water.export` (.glb/.fbx/.json) and `export_profile { water }`
  (water.fbx / water.glb + water.json: meshes with bounds, layers, falls, probe = the deepest
  water's surface). Studio's engine export sets `water` when water layers or falls exist.
  Checked headlessly on a terraced 1024 m / 512² scene (15 falls, river + wetland + pools water):
  Unity 6000.4 places water.fbx (29 066 tris), probe 0.0 m from the mesh; Unreal 5.8 imports
  water.glb, 0 m bound error (the square bounds can't tell Y from X — the caves check measured
  the mapping); Blender bridge brings it in, probe 0.0 m. CI fix pushed with W2–W4 (80044eb):
  yanked async-executor, and a 3-min job-wait ceiling for a slow ubuntu runner.
- 2026-11-08 W6. Guide: desktop-app.md "Water: rivers, waterfalls, wetlands and springs" (5 figures,
  viewed), CLI quickstart "Water … and to engines" (measured on the terraced 1 km world), blender.md,
  the Unity/Unreal READMEs. The waterfalls figure needed its own scene: falls are metres wide on a
  km world and the orbit only circles the centre, so the scene is a 400 m ramp with a 40 m terrace
  cliff across the middle and a spring-fed river over it. Open follow-ups: an orbit pan target in
  Studio (and the Editor) so a camera can visit a feature off-centre; a fall's width is its lip
  cells × cell size (a D8 river is 1–2 cells), narrower than the river water drawn above it; steep
  cliff faces render near-black (vertical faces in the heightfield shading).
- 2026-11-08 Reviews (/code-review medium + /ponytail-review over 017c9d5..HEAD; W2–W4 had been
  pushed before them — a slip against the review-before-push rule). Fixed: plunge-pool water now
  stands at the dug bowl's spill level (fill of the dug terrain, the lake connected to each pool),
  not at the foot over a lower gorge; a run is fed only by its largest donor and runs sharing a foot
  merge (a steep tributary no longer takes the main lip); pool_depth/pool_radius 0 = no pool;
  falls' log accumulation derived from the raw one; wetland fills once; the wetland node's
  description (still the abandoned TWI σ model) rewritten; unevaluated water layers are left out of
  the export with a note; the Blender bridge survives "no water"; one failed water preview no longer
  hides the rest; Studio maps cell-centred falls onto its corner grid. Ponytail: GridMesh Default +
  bounds(), shared water_layer / node_frame, the dead cycle guard. Follow-up: build_falls re-runs
  the graph and the falls search on every Studio refresh — the node could publish its falls list.
  No dedicated test for the tributary rule (hard to build a D8 junction by hand).
- 2026-11-08 W7a–c, f. Fall width = the river water's wet width across the flow at the lip
  (the node's new `water` input, wired by + falls). build_falls: 0.26 s on 1024² — left alone.
  Black cliffs: the macro normal map is the terrain's normal against flat ground (slope included)
  and the viewport applied it in the tilted face's frame, counting the slope twice (cliffs tipped
  past horizontal). The viewport now decodes it in the flat frame. Same root in the GLB: the map
  was tangent-space against flat ground with the bitangent north (w +1); Blender (oracle: Cycles
  bake of object-space shading normals, sine bumps step 8 vs step 1) showed median 99° error, a
  26.6° ramp shaded flat. Now baked against the exported mesh's own interpolated normals
  (rasterized; grid, decimated, LODs share LOD 0's), bitangent south (glTF image-up = v 0 = our
  south), w −1: median 6.8° (decimated 5.3°); the control (bitangent north) is back at 99°.
  scripts/blender/tangent_check.py had the sign rule inverted (glTF w = −Blender sign) and had
  passed the broken map; corrected. Old textured GLBs should be re-exported (quickstart says so).
- 2026-11-08 W7 review (/code-review medium db5e06a..bcd4c2d, 8 findings; ponytail: bounds shrink).
  Fixed: a fall's sheet is centred on the river's wet span (`shift_m`, applied to lip and foot by
  build_falls); grid GLB normals come straight from the heights (`Against::Grid`, no second mesh —
  an 8k² export would have held 2 GB more); each LOD level's map is baked against its own triangles
  (Blender oracle: LOD 2 median 2.0°, decimated 2.0°, grid 6.8°); terrain.textures says its normal
  map is against flat ground; pan clamps each axis to its own half-extent; flythroughs stay within
  their keys' centres. Documented, not capped: a lake layer wired into the falls' `water` widens the
  fall to the lake. Dismissed: "middle-drag breaks UI parity" — the Editor has no 3-D viewport
  since Option B. falls_water reaches 33 m on the terraced world: 39 cells of pools in closed
  hollows, which hold water to their rim (1 130 wet cells, under the falls mask's 2 107).
