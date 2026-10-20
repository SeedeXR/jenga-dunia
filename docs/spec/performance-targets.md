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

| 2026-10-20 | same | `jd.erosion.stream_power@1`, 2048² × 20 steps (D8, release `jd op graph.evaluate`) | 4.00 s node time (noise 56 ms); 4.17 s wall | 321 MB | `crates/jd-terrain/src/sim.rs::stream_power_erode`; implicit Braun–Willett solve + one flow-accumulation sort per step (~200 ms/step at 2048²) |
| 2026-10-21 | same | `jd.erosion.droplet@1`, 1024² × 1 500 000 droplets (release `jd op graph.evaluate`) | 4.60 s node time; 4.68 s wall. 512² × 400 000: 1.13 s | 48 MB | `crates/jd-terrain/src/droplet.rs`; single-threaded by nature (droplets interact through the field); ~3 µs per droplet |
| 2026-10-21 | same | Preview IPC through the e2e bridge (`JD_MEASURE=1 pnpm exec playwright test e2e/ipc.measure.spec.ts`: Playwright ↔ jd-mcp over stdio, base64 u16 preview + decode, WebGPU viewport) | 512² lod 0: 0.50 MB in 76 ms · 1024² lod 0: 2.00 MB in 302 ms · 2048² lod 1: 2.00 MB in 307 ms ≈ 7 MB/s | — | the harness path, not the product: the native Tauri raw-bytes IPC delivered + decoded the same 2.00 MB preview in 6 ms on 2026-10-19 (row above). Windows not measured (no machine) |
| 2026-10-21 | same | `jd diag bench s1` (release, after the shared-wind and scatter work) | fbm 27–35 ms, ridged 25–30 ms, add 1–2 ms, thermal 207–247 ms; total 260–315 ms (2 runs) | 45 MB | hash `859db903b963db8a` = the CI runners' hash: the S1 result is bit-identical across macOS, Linux, Windows and this machine |
| 2026-10-21 | same | `jd-gpu` thermal erosion, 2048² × 50 steps, `cargo test -p jd-gpu --release -- --ignored --nocapture bench` (Apple M4, Metal, headless wgpu 30) | GPU 146 ms incl. upload, pipeline build and readback; CPU 683 ms (same field, same process) — 4.7× | — | `crates/jd-gpu/src/thermal.wgsl` = line-for-line port of `sim.rs`; max abs diff 2.7e-4 m on a 0..460 m field (6e-7 relative), parity test 1.9e-7 at 96×80; not wired into the graph yet (`engine` param is the next step) |
| 2026-10-21 | same | `jd.erosion.hydraulic@1 { engine: gpu }` vs `cpu`, 1024² × 100 steps through `jd op graph.evaluate` (**debug** build, Apple M4 Metal) | GPU 578 ms vs CPU 6486 ms (11×; the debug CPU figure is ~4× the release one) | — | `crates/jd-gpu/src/hydraulic.wgsl`; parity test: 1 ulp (3e-5 m) after one step, 2.4e-3 m after 80 on a 280 m field (1e-5 relative), water 1.4e-4 m, sediment 4e-5 m; stats agree to 1e-7 relative on the 1024² world |
| 2026-10-21 | same | Whole-field path at 8k² and 16k² (release `jd op graph.evaluate` of the starter graph, then `jd export heightmap --format png16`; `/usr/bin/time -l` peak RSS) | 8k: evaluate 0.91 s, 16k: 3.7 s noise + 5.2 s wall | **before** 8k 1.12 / 1.24 GB, 16k 3.97 / 4.43 GB (evaluate / export) → **after** 8k 0.31 / 0.71 GB, 16k 1.21 / 2.78 GB | Three whole-field copies removed: `rebuild_halos` cloned the field, `LayerCache::put`/`get` buffered the blob twice (`to_vec` + bytes) and the evaluate op cloned every published layer out of the evaluator's `Arc`; png16/r16/r32 exports are now quantised row by row (`Heightfield::row`/`set_row`). A 16k² f32 layer is 1.07 GB with halos, so evaluate now costs the layer itself. The png16 export then still held the u16 plane plus the `image` encoder's copy (2.78 GB); written row by row through `png::StreamWriter` it is **1.21 GB** too (2.1 s wall, 463 MB file; r16 0.85 s, same peak) |
| 2026-10-21 | same | `terrain.preview {lod: 4}` of a 16k² layer from a cold process (release `jd op`, the layer only in the project's cache) | 1.61 s (was 0.28 s) | **22 MB** (was 1.22 GB) | `LayerCache::preview` streams the blob's rows and keeps only every 16th sample; bit-identical to the in-memory preview (test). The cost is one sequential 1 GiB read instead of a load; lod 0 on 16k² is still 2.3 GB because the preview itself is 268 M samples |
| 2026-10-20 | same | `jd diag bench s1` (S1-small-interactive: 1024² fBm + ridged + add + thermal×50, release) | fbm 30–47 ms, ridged 25–28 ms, add 1.6 ms, thermal 206–251 ms; total 283–308 ms (2 runs). 2048²: total 1.07 s | 43–44 MB (141 MB at 2048²) | `crates/jd-core/src/diag.rs::bench`; same content hash on both runs (deterministic); CI runs it on every push on the three OSes as a smoke with a 15 s / 1 GB budget |

## Regression policy

CI runs `jd diag bench s1` on every push (release build, three OSes) as a **smoke** — it fails above 15 s or 1 GB and prints the per-stage numbers in the job log — and compares it with `scripts/bench-baseline.json` (`scripts/check-bench.mjs`): the S1 **content hash must match the baseline exactly** (the kernels produce the same terrain on Linux, Windows and macOS — `859db903b963db8a` on all three runners and on the Apple Silicon laptop), the total may drift up to 1.5× the per-OS runner baseline silently, 1.5–3× raises a warning annotation (hosted runners vary that much between days), and above 3× the job fails. The baseline totals are the hosted runners' own (Linux 1 752 ms, Windows 1 679 ms, macOS 1 387 ms, run 39466828738, 2026-10-20); change the file only for an intended kernel change, in the same commit. The finer PR-against-main comparison described below (S1 + S5, 10 % / 5 %) remains the next step.
