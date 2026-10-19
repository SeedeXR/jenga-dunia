# ADR-0006 — Erosion: virtual pipes first, droplet second, stream power later

Status: Accepted · 2026-10-19

## Context
Brief §5.1 asks for hydraulic, thermal, wind, coastal, snow erosion with evaluation of particle, grid and hybrid approaches and attention to determinism and tiling. Literature review (`algorithms.md` §2–§5) and incumbents (`gaea-world-machine.md` D2; `world-creator.md` §3.3) establish the options.

## Decision
Implement in this order:
1. **Thermal** (Musgrave 1989 / Olsen 2004 Jacobi talus) — simplest stencil, validates the tile/halo machinery. Phase 3. **Shipped 2026-10-20** as `jd.erosion.thermal@1` (`jd-terrain::sim::thermal_erode`): CPU gather formulation (each cell subtracts its own outflow and adds its neighbours' flow towards it), exactly mass-conserving, deterministic across thread counts; whole-field passes per iteration for now, tile-local passes + WGSL twin pending.
2. **Hydraulic — virtual pipes / shallow water** (Mei, Decaudin, Hu 2007; layered materials per Šťava et al. 2008) as the **default** hydraulic node: a gather stencil → deterministic, tiles with a 1–2 cell halo, identical CPU/WGSL kernels. Phase 3.
3. **Hydraulic — droplet/particle** (Beyer 2015; design cues from ProceduralTerrains' deterministic batching) as the fast, stylised alternative, tile-binned with per-drop PCG seeds. Phase 3.
4. **Stream power / large scale** (Cordonnier et al. 2016; Schott et al. 2023) plus priority-flood lakes (Barnes 2014) to address the known "cannot form lakes" limitation. Phase 5.
5. **Aeolian** (Paris et al. 2019 Desertscapes), **glacial** (Argudo et al. 2020), **snow** (occlusion/slope + diffusion), **coastal** (documented heuristic; no graphics paper located). Phase 4–5.
Every node exposes rainfall, hardness map, sediment capacity, deposition/evaporation, iterations, Δt, and declares fidelity (*physical* / *physically-inspired* / *visual*). Outputs include flow, wear, deposition, sediment, talus-depth masks — the maps professionals consume.

## Alternatives rejected
- Droplet-first — most popular open-source path (Lague) but inherently order-dependent; determinism requires extra machinery and tiles poorly as the default.
- SPH — expensive, no clear advantage for heightfields.
- Porting incumbents' behaviour — no public algorithm descriptions beyond parameter names; clean-room rules apply.

## Consequences
- Pipes kernel is the first GPU parity test case.
- CFL/stability constraints must be enforced and surfaced (`E_NUMERIC_UNSTABLE`).
- Mei 2007 equation details to be confirmed from the PDF before implementation (`algorithms.md` gaps #4).

## Evidence
`docs/research/algorithms.md` §2–§6, selection table; `docs/research/opensource-rea.md` §6 items 7–11.
