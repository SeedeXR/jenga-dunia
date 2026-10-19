# Performance Targets and Benchmark Scenes

Status: Design. **No numbers below are measured yet.** Targets become binding only after baselines are measured on reference hardware (ADR-0008 records the methodology). Until then this document defines *what* is measured and the ceiling the brief imposes (≤ 4 GB RAM).

## Reference hardware

| Tier | Machine | Role |
|---|---|---|
| A (primary) | Apple Silicon laptop (M-series, 16 GB unified) | all targets must pass |
| B | Windows desktop, discrete GPU (mid-range), 16–32 GB | validation |
| C | Linux desktop/CI runner, no GPU | CPU-only path must pass with relaxed time |

## Benchmark scenes (`benches/scenes/`)

| Id | Scene | Exercises |
|---|---|---|
| `S1-small-interactive` | 1024² terrain, fBm + ridged + thermal, 2 materials | interactive preview latency |
| `S2-medium-detailed` | 4096² mountains + hydraulic (grid) 200k iters + hydrology + 4 materials | typical artist session |
| `S3-large-tiled` | 16384² in 512² tiles, noise + thermal + export tiles | tiling, streaming, cache |
| `S4-erosion-hires` | 8192² hydraulic (particle + grid) | kernel throughput, memory |
| `S5-complex-graph` | 150-node graph, many masks/blends at 2048² | graph overhead, cache hit rate |
| `S6-material-bake` | 4096² bake of 6 maps | baking, disk write |
| `S7-bulk-export` | S2 exported to GLB + USDZ + OBJ + R16 + weightmaps, 4 LODs | exporters, decimation |
| `S8-concurrent` | S1 preview while S4 builds | scheduler fairness |

## Metrics (captured by `jd bench`)

Processing time (wall, per stage), **peak RSS**, GPU memory peak, CPU utilisation, disk footprint (cache + exports), cache hit rate, viewport frame time p50/p99, import/export durations.

## Ceilings (binding now)

| Metric | Ceiling |
|---|---|
| Peak RSS, any benchmark scene | **≤ 4.0 GB** (hard); aim ≤ 2.5 GB |
| Preview latency (S1 param change → viewport update) | ≤ 100 ms p50 on Tier A (target, to be validated) |
| Viewport | ≥ 60 fps at 1440p on Tier A for S2 (target) |
| UI thread blocking | 0 ops > 16 ms on the UI/IPC thread (enforced by design: everything is a job) |

## Determinism tolerances (binding)

| Path | Tolerance |
|---|---|
| CPU, same build, any thread count | bit-identical |
| CPU across platforms (macOS/Linux/Windows, same arch) | bit-identical for integer/PRNG paths; ≤ 1e-6 relative for float kernels (no FMA contraction differences allowed: build with consistent flags) |
| GPU vs CPU | per node documented, default ≤ 1e-4 relative on heightfields; erosion ≤ 1e-3 |

## Measured baselines

| Date | Machine | Scene / command | Wall | Peak RSS | Notes |
|---|---|---|---|---|---|
| 2026-10-19 | Apple Silicon laptop, 10 cores, 16 GB (Darwin 27.0.1) | `jd terrain generate` 4096² OpenSimplex2 ridged 6 octaves + EXR export (release build) | generate 157 ms; whole process 0.42 s real / 3.06 s user | 235 MB | `/usr/bin/time -l`; EXR validated by macOS ImageIO (4096×4096, 32 bps) |
| 2026-10-19 | same | Desktop app (release, `tauri/custom-protocol`): `terrain.generate` 2048² fBm via menu; 1024² u16 preview (2.00 MB) to the WebGPU viewport | generate 58–63 ms (3 runs); preview delivered + decoded in 6 ms | — | header readout + `JD_LOG=info` op spans; viewport 2.09 M tris |
| 2026-10-20 | `jd.erosion.thermal@1`, 2048² × 50 steps, after fBm noise | release `jd op graph.evaluate` (CPU, rayon, flat two-pass Jacobi) | thermal 887 ms, noise 64 ms, total 952 ms; process 0.98 s wall, 130 MB peak RSS; 512² × 50 steps 62–68 ms | `crates/jd-terrain/src/sim.rs`; graph `noise → thermal → output`; `memory/handover_session.md` 02:10 |
| 2026-10-20 | same | `jd.erosion.hydraulic@1` (virtual pipes, 100 steps, closed boundary) after fBm noise, release `jd op graph.evaluate` | 512²: erosion 171 ms (total 214 ms); 2048²: erosion 2.10 s (total 2.16 s, 2.19 s wall) | 369 MB at 2048² (≈ 40 B/cell scratch) | `crates/jd-terrain/src/sim.rs`; terrain + suspended sediment conserved (mean 254.2 → 254.2 at 512²) |

## Regression policy

CI runs S1 and S5 (fast) on every PR and compares with the previous `main` baseline; > 10 % regression in time or > 5 % in peak RSS fails the check unless the PR carries a `perf-accepted` label with justification. Full suite runs nightly on Tier A self-hosted runner (when available) and on hosted runners for CPU-only.
