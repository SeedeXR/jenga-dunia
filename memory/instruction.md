# Instruction — Implementation Guidance

Complete implementation guidance for agents moving from docs to code. Architecture detail is in `docs/architecture/`; this file is the operational "how".

## 1. Directory structure (target)

See `docs/architecture/01-system-architecture.md` → *Workspace layout*. Create crates in dependency order: `jd-types` → `jd-diag` → `jd-cache` → `jd-jobs` → `jd-terrain` → `jd-graph` → `jd-core` → `jd-cli` → `jd-mcp` → `jd-app`. Add other domain crates when their phase starts; do not scaffold empty crates "for later".

## 2. Module conventions

- Each crate: `lib.rs` with a one-paragraph doc comment stating responsibility and what it must not depend on; `error.rs` with a `thiserror` enum mapping to `jd_types::ErrorCode`; `tests/` for integration; `benches/` where there is a kernel.
- Public API surface is small and documented; internals `pub(crate)`.
- Kernels take `&Tile`/`&mut Tile` slices plus a `KernelCtx { seed, lod, cancel: &CancellationToken, progress: &dyn Fn(f32) }`.
- Node implementations live beside their kernel (`jd-terrain/src/nodes/fbm.rs` registers `jd.noise.fbm@1`).

## 3. Process flows to implement first (Phase 1→2)

1. `project.create` → directory layout from `docs/spec/project-format.md`, lock, journal.
2. `graph.add_node` / `graph.connect` / `graph.set_param` → validation → dirty set → journal append.
3. `graph.evaluate {lod}` → job → tiles → cache → progress channel → preview tiles.
4. `io.export {format: exr|r16|png16}` → heightfield → file + report.
5. `jobs.status` / `jobs.cancel`.
6. `diag.env`.
Each must work identically in GUI, CLI and MCP before adding the next.

## 4. Data and compression

- Tiles: f32 little-endian row-major, `(size+2*halo)²`, blake3 content hash; cache blobs zstd level 3 (benchmark against raw; keep raw if decompression dominates).
- Strokes: packed `f32` samples, zstd.
- Documents: pretty JSON with sorted keys.
- Exports: EXR (32-bit float, PIZ/ZIP), PNG16, TIFF (16/32-bit float), RAW r16/r32 (document byte order and row origin per target profile).

## 5. Development standards (enforced)

`cargo fmt`, `clippy -D warnings`, `cargo deny` (licences: MIT/Apache-2.0/BSD/ISC/Zlib/MPL-2.0-only-as-dylib; advisories), `cargo nextest`, `insta` snapshots committed, `criterion` baselines committed under `benches/results/`. No `unwrap()` outside tests; errors carry codes. No `println!` — `tracing` only. Property tests for numeric kernels (finite outputs, bounds, symmetry where expected). Fuzz every parser.

## 6. GPU path rules

- Implement CPU first, GPU second; a GPU kernel ships with a parity test against the CPU kernel on fixtures with the tolerance from `docs/spec/performance-targets.md`.
- No float atomics in reductions; use tiled two-pass reductions.
- Readbacks batched per job, never per tile.
- Feature detection at startup; every GPU feature has a CPU fallback or a clear `E_GPU_DEVICE` with `next`.

## 7. Technical approach per subsystem (pointers)

| Subsystem | Start with | Reference |
|---|---|---|
| Noise | OpenSimplex2/Perlin via `noise`/`fastnoise-lite` wrapper, fBm/ridged/warp in-house, per-tile seeds | `docs/research/algorithms.md` §1 |
| Hydraulic erosion | particle droplet (fast, artist-friendly), then virtual-pipes grid (deterministic, GPU), then stream-power (large-scale) | §2 |
| Thermal | Musgrave/Olsen talus | §3 |
| Hydrology | priority-flood (Barnes) + D8/D∞ accumulation | §6 |
| Meshing | grid per tile + skirts; RTIN/TIN for export LODs | §10 |
| Decimation | QEM with attribute weights; compare with `meshopt` simplifier | §11 |
| Tangents | MikkTSpace (port or binding) | §11 |
| Materials | height/slope/curvature masks → layer blend; bake to textures; glTF PBR first | §12 |
| Ocean | Gerstner (viewport, Phase 3), FFT (later) | §13 |
| Sky | Hillaire 2020 or Hošek-Wilkie; sun position via NOAA algorithm | §14 |
| Scatter | Poisson-disk (Bridson) with density mask | §15 |
| glTF | `gltf` crate write + meshopt + KTX2 (later) | `docs/research/formats-sdks.md` |
| USD | USDA text writer in Rust first; USDZ packaging rules; usdchecker in CI when available | same |
| FBX | import via ufbx; export via optional adapter | same |

## 8. Porting/execution notes

- macOS: Metal via wgpu; notarisation in release workflow; `usdcat`/`usdchecker` from Apple USD Tools or OpenUSD builds detected by `jd diag env`.
- Windows: DX12 primary, Vulkan fallback; long paths; MSVC toolchain in CI.
- Linux: Vulkan; webkit2gtk-4.1 for Tauri; AppImage.

## 9. Definition of done for any feature

Code + tests + bench (if kernel) + docs (architecture/spec/matrix) + CLI + MCP exposure + error codes + handover entry + todo update.
