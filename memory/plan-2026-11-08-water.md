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
| W1a | `jd-terrain::falls`: find waterfalls on a river path (lip→base run of steps ≥ min_gradient, drop ≥ min_drop), carve plunge pools (depth 0.33 × drop, radius 0.57 × drop), falls mask; unit tests | todo |
| W1b | node `jd.hydrology.waterfalls@1` (in, rivers? → out, falls); op `hydrology.waterfalls {id}` listing them; jd-graph + jd-core tests; schemas | todo |
| W1c | Studio: falling-water sheets drawn at each fall (viewport), `+ falls` button; window test | todo |
| W2a | `jd-terrain::wetland`: probability from wetness (MFD) × flatness, cut 0.5; marsh core (standing water) / swamp fringe; standing water depth ≤ water_depth_m in micro-depressions; unit tests | todo |
| W2b | node `jd.hydrology.wetland@1` (in, wetness? → wetland, marsh, swamp, water); `+ wetland` button publishes `wetland_water` + masks; biome docs; tests | todo |
| W3a | rivers node: `water` output — depth ∝ Q^0.4 scaled so the largest river reaches `water_depth_m`, over the channel; tests (exponent, monotonic downstream) | todo |
| W3b | Studio draws every water layer (lakes + `*_water`), strip's rivers adds `river_water`; window test | todo |
| W4 | Studio Rivers section: rivers nodes listed, **Place springs** arms click-to-place (as cave entrances), springs listed with ×; window test | todo |
| W5 | engine export: `water` in export_profile + `water.export` op; Unity importer + check, Unreal check, Blender bridge + test; run the headless engine checks | todo |
| W6 | docs (guide sections, figures via guide.rs, CLI quickstart), acceptance item?, gates, CI, push | todo |

## Log
