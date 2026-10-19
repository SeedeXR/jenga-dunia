# ADR-0008 — Measured performance: benches, peak-RSS gate, 4 GB ceiling

Status: Accepted · 2026-10-19

## Context
Brief §14 ("performance must be measured, not assumed"; ≤ 4 GB RAM; profile Apple Silicon first). Incumbents gate resolution by RAM and repeatedly ship paging/backpressure fixes (`gaea-world-machine.md` D5.31).

## Decision
- `criterion`/`divan` micro-benches per kernel with committed baselines; `jd bench <scene>` macro-benches over scenes S1–S8 capturing wall time, peak RSS, GPU memory, cache hit rate (`docs/spec/performance-targets.md`).
- CI gate: S1 + S5 on every PR; > 10 % time or > 5 % peak-RSS regression fails unless labelled `perf-accepted` with justification. Peak RSS > 4 GB on any scene is a hard failure.
- A `MemoryBudget` in `jd-jobs` admits jobs by estimate and falls back to mmap/tiling; over-budget → `E_RESOURCE_MEMORY` with a suggestion.
- No optimisation PR without a bench diff; no "faster" claim in docs without a number and a machine fingerprint.
- Allocator choice (`mimalloc` vs system) and SIMD dispatch (runtime, not compile-time AVX2 — Gaea's AVX2 requirement locks out Apple Silicon even under emulation) decided by measurement in Phase 2.

## Alternatives rejected
- Targets set now from guesses — the brief forbids it; targets are set after baselines.
- Hardware-feature requirements at compile time — excludes platforms.

## Consequences
- Benchmark fixtures and a self-hosted Apple Silicon runner (nightly) are Phase 1 backlog items.
- Regression tolerance bands may need tuning for hosted-runner noise.

## Evidence
`docs/research/gaea-world-machine.md` D5.30–31; brief §14.
