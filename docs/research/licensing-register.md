# Dependency and Licensing Register

Rule (brief §16.1): no dependency enters the workspace without a row here, licence verified from the source (crates.io/GitHub LICENSE), linking implications assessed. Core must stay MIT-compatible: MIT / Apache-2.0 / BSD / ISC / Zlib / CC0 / Unlicense OK; MPL-2.0 only as unmodified files; LGPL only via process boundary or dynamic linking with care; GPL/AGPL and proprietary → **outside the core** (plugin, bridge, user-installed). Versions as verified 2026-10-19 (`stack.md`, `formats-sdks.md`); re-verify at adoption.

## Rust — core
| Crate | Version | SPDX | Purpose | Risk / note |
|---|---|---|---|---|
| tauri + official plugins | 2.12.4 | Apache-2.0 OR MIT | shell | Tauri 3/CEF alpha — watch |
| wgpu, naga | 30.0.1 | MIT OR Apache-2.0 | GPU | breaking major ~quarterly; isolate in `jd-gpu` |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib | surface | — |
| glam, bytemuck, half | 0.34.1 / 1.25.2 / 2.7.1 | MIT OR Apache-2.0 (bytemuck + Zlib) | math, layout, f16 | — |
| rayon | 1.12.0 | MIT OR Apache-2.0 | parallelism | — |
| fastnoise-lite / libnoise / opensimplex2 | 1.1.1 / 1.2.0 / — | MIT / MIT / CC0 | CPU noise | fastnoise2 (MIT, C++) optional feature |
| image, png, tiff, exr | 0.25.10 / 0.18.1 / 0.11.3 / 1.74.2 | MIT OR Apache-2.0 / MIT / BSD-3-Clause | raster I/O | exr needs notice |
| meshopt | 0.7.0 | MIT OR Apache-2.0 (C++ MIT) | decimation/LOD | C++ toolchain |
| memmap2 | 0.9.11 | MIT OR Apache-2.0 | cache mmap | unsafe API |
| serde, serde_json, postcard | 1.0.229 / 1.0.154 / 1.1.3 | MIT OR Apache-2.0 | docs, cache blobs | **avoid bincode 3 (tombstone)** |
| schemars, jsonschema | 1.2.3 / 0.59.0 | MIT | schemas | — |
| clap | 4.6.7 | MIT OR Apache-2.0 | CLI | — |
| tracing{,-subscriber,-appender} | 0.1.44 / 0.3.23 / 0.2.5 | MIT | logging | — |
| thiserror, anyhow, miette | 2.0.21 / 1.0.104 / 7.6.0 | MIT OR Apache-2.0 / Apache-2.0 | errors | miette CLI only |
| tokio, tokio-util | 1.53.3 / 0.7.20 | MIT | async, cancellation | — |
| blake3 | 1.8.7 | CC0-1.0 OR Apache-2.0 | hashing | — |
| redb | 4.4.0 | MIT OR Apache-2.0 | cache index | format changes across majors (or `rusqlite` bundled, MIT) |
| zstd, lz4_flex | 0.14.3 / 0.14.0 | BSD-3-Clause / MIT | compression | — |
| rmcp | 3.5.1 | Apache-2.0 | MCP | spec churn (2026-07-28) |
| mimalloc, sysinfo, peak_alloc, notify | 0.1.52 / 0.39.6 / 0.3.0 / 8.2.0 | MIT / MIT / MIT / CC0-1.0 | allocator, monitoring, hot reload | — |
| gltf, gltf-json | 1.4.1 | MIT OR Apache-2.0 | glTF read / JSON write | read-only crate; own GLB packer |
| openusd, openusd-schemas | 0.8.0 | MIT; schemas TOST-1.0 (Apache-2.0 variant, LicenseRef) | USD | keep NOTICE |
| zip | 9.0.5 | MIT | usdz | — |
| ufbx | 0.11.5 | MIT OR Unlicense | FBX import | C |
| fbxcel | 0.9.0 | MIT OR Apache-2.0 | FBX binary writer (opt) | — |
| tobj, stl_io, ply-rs | 4.0.5 / 0.11.0 / 0.1.3 | MIT | OBJ/STL/PLY | ply-rs unmaintained |
| contour | — | MIT/Apache-2.0 | contours | **`contour-isobands` is AGPL — banned** |
| proj4rs / proj | — / 0.31.0 | MIT / MIT (PROJ MIT) | reprojection | `proj` heavy, optional |
| spa | 0.5.1 | — (verify) | sun position | — |
| quick-xml | — | MIT | .mtlx | — |
| proptest, insta, criterion, divan (dev) | 1.12.0 / 1.49.0 / 0.8.2 / 0.1.21 | MIT/Apache | tests, benches | — |
| ort (optional) | 2.0.0-rc.15 | MIT OR Apache-2.0 | ONNX inference | RC churn; native lib; `onnx` feature only |

Banned / avoided: `bincode` 3 (tombstone), `rend3`, `sled`, `simdnoise` (dormant), `gdal` crate (system lib + LGPL GEOS), `contour-isobands` (AGPL), `mcp-sdk`/`mcpr` (dormant).

## Frontend
| Package | Version | Licence |
|---|---|---|
| react | 19.3.1 | MIT |
| @xyflow/react | 12.12.0 | MIT |
| three | 0.186.1 | MIT |
| tailwindcss | 4.3.3 | MIT |
| vite / typescript / @biomejs/biome | 8.3.4 / 7.0.2 / 2.5.15 | MIT / Apache-2.0 / MIT OR Apache-2.0 |
| lucide-react | 1.59.0 | **ISC** |
| @fontsource-variable/host-grotesk | 5.3.0 | **OFL-1.1** (bundling allowed; keep OFL text) |
| radix-ui / shadcn | 1.8.1 / 4.22.1 | MIT |
| zustand, vitest, playwright | 5.0.15 / 5.0.5 / 1.64.1 | MIT / MIT / Apache-2.0 |

## External tools (process boundary, never linked)
| Tool | Licence | Use |
|---|---|---|
| Blender 5.2 LTS | GPL-3.0-or-later | subprocess for bridge/round-trip; our add-on `integrations/blender/jenga_dunia_bridge` is GPL-3.0-or-later (own `LICENSE`, talks to the core only via the `jd` CLI); move to a separate repo before an Extensions Platform submission |
| Apple USD Tools (`usdchecker`, `usdzip`, …) 0.25.11 | Apple/TOST | validation on macOS |
| gltf-validator / @gltf-transform/cli | Apache-2.0 / MIT | validation |
| KTX-Software `toktx` 4.4.2 | Apache-2.0 | KTX2 encoding |
| Autodesk FBX SDK 2020.3.9 | proprietary EULA | opt-in out-of-process adapter; binaries OK with notice; headers/source not redistributable |
| Maya 2027 / Maya USD 0.37 | proprietary / Apache-2.0 | user-side scripts only |
| Unity / Unreal | Unity Companion / Epic EULA | user-side |
| REA (morluto/rea) | MIT | dev-time only; policy ADR-0010 |

## Data sources (see `geo-vision.md` A5 for full table)
Allowed by default: Copernicus DEM (mandatory notice), SRTM/NASADEM/3DEP (public), AWS Terrain Tiles, GEBCO, ESA WorldCover / Copernicus GLC (CC BY 4.0), OSM (ODbL, attribution + derivative DB handling). BYOK: OpenTopography. Excluded: Google Elevation/Earth Engine, Cesium World Terrain, MapTiler Terrain-RGB, Mapbox Terrain-DEM.

## Models (see `geo-vision.md` B5)
Default: MoGe-2 (MIT — verify weight LICENSE file before GA), DA3 small/base/metric/mono (Apache-2.0), DA-V2 Small (Apache-2.0). Non-commercial (off by default, tagged): DA-V2 Base/Large/Giant, DA3 large/giant/nested. Excluded: UniDepth v2, DUSt3R, MASt3R.

## Algorithms/code reuse (see `opensource-rea.md` §6)
Port with attribution: TerraForge3D shaders (MIT), SebLague erosion (MIT), ProceduralTerrains core (MIT). Study only, never copy: weigert repos (README-only MIT / no LICENSE), ANT Landscape (GPL), kosmos (GPL), soillib (LGPL), Unity Terrain Tools (Unity Companion), Unreal source, Houdini.
