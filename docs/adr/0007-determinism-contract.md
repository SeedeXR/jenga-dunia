# ADR-0007 — Determinism contract and CPU/GPU tolerance tiers

Status: Accepted · 2026-10-19

## Context
Brief §3.3 and §6 require deterministic seeds and reproducible evaluation. World Creator had cross-GPU output divergence (`world-creator.md` §2.4); Gaea's classic nodes are non-deterministic in multi-core mode and ship a "Deterministic (Slow)" switch; World Machine advertises build-to-build consistency as a headline fix (`gaea-world-machine.md` D1.5).

## Decision
1. All randomness derives from explicit seeds: per-item streams are `PCG32::seed(hash(seed, node_id, tile, item))`; no permutation tables; the same integer hash is used in Rust and WGSL.
2. Parallel reductions are ordered (chunk partials summed in index order); no float atomics in persisted results.
3. Iteration counts and Δt are parameters, never derived from wall-clock or thread count.
4. Tolerance tiers (also in `docs/spec/performance-targets.md`):
   - CPU same build, any thread count: **bit-identical**.
   - CPU across platforms/same arch: bit-identical for integer/PRNG paths; ≤ 1e-6 relative for float kernels (explicit `fma`, consistent flags).
   - GPU vs CPU: per node, default ≤ 1e-4 relative; erosion ≤ 1e-3 (statistical tier for particle methods).
5. Cache keys include backend when a node is not in the bit-exact tier.
6. A `mutate` batch mode (seed variations) is a CLI feature, as in Gaea.

## Alternatives rejected
- "Best effort" determinism — produces the complaints documented for all three incumbents.
- Forcing single-threaded deterministic mode — unacceptable performance.

## Consequences
- Slight performance cost from ordered reductions; measured in benches.
- Every kernel ships a golden-tile test; CI runs GPU parity on a software adapter where available.

## Evidence
`docs/research/algorithms.md` §16 and §(b) "Keeping CPU and GPU paths matching".
