# ADR-0005 — f32 heightfield tiles with halos as the unit of work

Status: Accepted · 2026-10-19

## Context
The 4 GB RAM ceiling (brief §14) versus 4k–16k heightfields: a single 8k² f32 plane is 256 MB and pipes erosion needs ~8 planes (2 GB un-tiled) (`algorithms.md` (b)). World Creator generates one terrain and tiles only at export, with a history of seam bugs; Gaea and World Machine both admit tile-aware simulation is hard (`gaea-world-machine.md` D2.17). Professionals expect 32-bit float pipelines (both tools).

## Decision
- Heightfields, masks and simulation planes are **f32**, stored as **tiles** (runtime parameter, default 512 GPU / 1024 CPU, power of two) with an **apron/halo** whose width is derived from each node's declared stencil radius.
- Nodes declare `Local(radius)` or `Global`; global nodes (priority-flood, whole-field reductions) run on the full field (possibly at reduced LOD) and are documented as not tile-safe.
- Tile data is content-hashed (blake3) for cache keys and region-based recomputation.
- f16 only for preview/export planes; f64 only for world origins (origin rebasing).

## Alternatives rejected
- u16 heightfields internally — quantisation artefacts under erosion; u16 is an export encoding.
- f64 everywhere — doubles memory for no visible benefit at terrain scales.
- Whole-field evaluation with paging to disk — simpler but breaks the memory ceiling and interactivity; WM's paging crash reports illustrate the failure mode.

## Consequences
- Every kernel is written tile-aware from day one; halo exchange is a core primitive.
- Some algorithms require a "gate" pattern (bake upstream at global resolution, upsample per tile) — the design explicitly allows it.
- Memory budget becomes computable per job (`06-jobs.md`).

## Evidence
`docs/research/algorithms.md` §(b) data layout; `docs/research/gaea-world-machine.md` D2.17, D5.31; `docs/research/world-creator.md` §4, §13.
