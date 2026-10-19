# ADR-0012 — Format strategy: glTF + USD pure Rust first, FBX via adapter

Status: Proposed (accept after Phase 1 heightfield + glTF exporters pass validators) · 2026-10-19

## Context
Brief §9 requires glTF/GLB, USD(A/C/Z), FBX, OBJ, STL, PLY, EXR, PNG/TIFF, RAW, GeoTIFF, OpenVDB "where justified", with verified round trips and no silent loss. Research (`formats-sdks.md`) finds: `gltf` crate is read-only (write via `gltf-json` + small GLB packer); `openusd` 0.8 (MIT) reads and writes usda/usdc/usdz; Autodesk FBX SDK is proprietary (binaries distributable with notice, headers/source not) and `ufbx` (MIT) reads FBX; `exr`/`image`/`tiff`/`tobj`/`stl_io`/`ply-rs` are mature pure Rust; `vdb-rs` is read-only; Blender extensions must be GPL-3.0-or-later.

## Decision
- **Phase 1 (pure Rust):** heightfields (EXR f32, PNG16, TIFF 16/32f, RAW r16/r32 + JSON sidecar, flip/endian toggles, Unity/Unreal size presets), weightmaps, glTF/GLB writer, OBJ/STL/PLY writers, USDA text writer (UsdGeomMesh + UsdPreviewSurface, `upAxis`, `metersPerUnit`); imports via `gltf`, `tobj`, `stl_io`, `ply-rs`.
- **Phase 2:** `openusd` for usdc/usdz/composition and PointInstancer; `EXT_meshopt_compression` via `meshopt`; FBX import via `ufbx`; **Blender bridge** (separate GPL repo, subprocess) for FBX write and Blender-native setup; GeoTIFF read via `tiff` GeoKeys.
- **Phase 3:** KTX2/Basis (external `toktx`/`gltf-transform` or FFI feature), optional Draco, MaterialX `.mtlx` writer (OpenPBR + UsdPreviewSurface + gltf_pbr).
- **Phase 4:** opt-in Autodesk FBX SDK out-of-process adapter (user-installed SDK), `proj` reprojection, COG, optional 3D Tiles; OpenVDB only if a non-heightfield mode appears.
- Canonical interchange: glTF for meshes, USD for scenes/DCC, raw heightfields for engine terrain; FBX is compatibility-only and **never described as an open standard**.
- No `gdal` bundling (system lib, LGPL transitive).

## Alternatives rejected
- OpenUSD C++ via FFI — build complexity; `openusd` Rust crate covers needs (verify usdz alignment + `usdchecker --arkit`).
- Assimp for everything — BSD but large C++ surface, FBX write quality poor.
- Bundling FBX SDK — licence prohibits source/header redistribution; platform/legal risk.

## Consequences
- Validators are optional external tools detected at runtime; CI installs `gltf-validator` and uses macOS `usdchecker` where available.
- Interop matrix rows start as `planned` and flip to `verified` only with validator output attached.

## Evidence
`docs/research/formats-sdks.md` §1–§16, (a), (b), (c), gaps list.
