# Jenga Dunia — 3D Interop Formats & SDK Research

**Scope:** file formats and SDKs for an MIT-licensed, commercially usable Rust/Tauri terrain tool that must read/write assets usable in Blender, Maya, Unreal, Unity and WebGL/WebGPU.
**Access date:** 2026-10-19. All crate versions/licences pulled from the crates.io API; repo activity from the GitHub API; local tool versions checked on the author's macOS 27 (Darwin 27.0.0) workstation.

**Evidence labels**
- **VERIFIED** — read directly from a primary source today (crates.io API, GitHub API/raw file, vendor docs, local binary/license file).
- **REPORTED** — stated by a secondary source (forum, news, package doc summary) or inferred from a primary source that did not state it outright.
- **UNVERIFIED** — could not confirm; treat as a hypothesis.

Local environment facts used below (VERIFIED by running the binaries):
- Blender 5.2.0 LTS (build 2026-07-14), io_scene_gltf2 5.2.39, io_scene_fbx 5.15.0
- Maya 2027 (API 20270200) + Maya USD 0.37.0, FBX SDK 2020.3.9 (universal x86_64+arm64 dylib)
- Unity 6000.4.8f1
- Apple USD Tools 0.25.11 in `/usr/bin`: `usdcat usdchecker usdcrush usdextract usdrecord usdtree usdzip`
- Unreal: only the Epic Games Launcher is installed (no engine build found)

---

## 1. glTF 2.0 / GLB

**Spec.** Khronos glTF 2.0 (royalty-free, open standard). Relevant extensions and their registry status (VERIFIED from the Khronos extensions README): all of `KHR_texture_transform`, `KHR_materials_emissive_strength`, `KHR_texture_basisu`, `KHR_draco_mesh_compression`, `EXT_meshopt_compression`, `EXT_mesh_gpu_instancing`, `KHR_mesh_quantization`, `KHR_materials_variants` are **ratified**.

**Rust crates (VERIFIED, crates.io 2026-10-19):**

| Crate | Version | Date | License | Read | Write | Notes |
|---|---|---|---|---|---|---|
| `gltf` | 1.4.1 | 2024-05-10 | MIT OR Apache-2.0 | yes | no (loader) | 10.6M downloads; repo last push 2026-05-11; 66 open issues; MSRV 1.61. Extensions parsed: KHR_lights_punctual, pbrSpecularGlossiness, unlit, texture_transform, materials_variants, volume, specular, transmission, ior, emissive_strength, EXT_texture_webp. **No** KHR_texture_basisu, Draco, meshopt or EXT_mesh_gpu_instancing typed support — these arrive as raw `extensions` JSON (`extras`/`names` features). |
| `gltf-json` | 1.4.1 | 2024-05-10 | MIT OR Apache-2.0 | yes | **yes (JSON only)** | Serde model of the glTF JSON; `gltf-export` example shows building a document. GLB container (12-byte header, JSON chunk + BIN chunk, 4-byte padding) must be assembled by hand — ~40 lines. |
| `meshopt` | 0.7.0 | 2026-10-17 | MIT OR Apache-2.0 | n/a | n/a | FFI to zeux/meshoptimizer (vendored submodule, C++ built via build.rs). Exposes `encode_vertex_buffer`, `encode_index_buffer`, decoders, remap/optimize/quantize/stripify. These are the primitives of `EXT_meshopt_compression`; the glTF-level bookkeeping (bufferView extension object, `mode`, `filter`) is yours to write. Upstream meshoptimizer v1.3 (2026-09-25, MIT). Filter encoders (octahedral/quaternion/exponential) exposure in 0.7: UNVERIFIED. |
| `draco-core` / `draco-gltf` / `draco-io` | 2.3.2 / 0.7.0 / 0.6.0 | 2026-10 | Apache-2.0 | yes | yes | **Pure Rust** Draco (Filyus/draco-rust). Claims bitstream compatibility checked against upstream fixtures; `draco-gltf` does document-preserving Draco compression. Only 5 GitHub stars — low adoption risk. |
| `draco-oxide` | 0.1.0-alpha.11 | 2026-08-31 | MIT OR Apache-2.0 | yes | yes | Pure Rust rewrite by reearth; bidirectional bitstream 2.2 compat claimed; alpha API. |
| `draco-rs` | 0.1.3 | 2025-04-28 | (not checked) | yes | yes | C++ FFI bindings; needs a C++ toolchain. |
| `ktx2` | 0.5.0 | 2026-04-19 | Apache-2.0 | yes | **no** | Parser only (BVE-Reborn). 5.8M downloads (Bevy dependency). |
| `basisu` | 0.1.0 | 2026-07-18 | Apache-2.0 | transcode | **no encode** | Pure-Rust Basis Universal transcoder (ETC1S/UASTC → BCn/ETC/ASTC). Single-commit repo, author-claimed conformance; unproven. |
| `basis-universal` | 0.3.1 | 2023-11-02 | MIT OR Apache-2.0 | transcode | **encode** | FFI to Binomial's C++ (Apache-2.0). Last crate release 2023-11, repo push 2024-04-28 — stale but "production-ready" per README. Only Rust path to *encode* ETC1S/UASTC in-process. |
| `ktx2-rw` | 0.2.4 | 2025-11-11 | MIT OR Apache-2.0 | yes | yes | Builds KTX-Software from source with CMake at compile time (5–10 min first build). Heavy. |

**Mesh compression choice — Draco vs EXT_meshopt_compression.**
- Draco: best ratio for static meshes, decoder is a ~300 KB WASM in browsers; Unity needs `com.unity.cloud.draco`, Blender exports it (`export_draco_mesh_compression_enable`, VERIFIED local), Unreal's Interchange glTF support for Draco is UNVERIFIED.
- meshopt: smaller decoder, GPU-ready quantized buffers, works with `KHR_mesh_quantization`; three.js (`setMeshoptDecoder`), Unity glTFast (`com.unity.meshopt.decompress`) and Blender 5.2 (`export_meshopt_extension` property present, VERIFIED local) all support it. The `meshopt` crate is actively maintained (released two days ago).
- **Recommendation:** meshopt first (actively maintained FFI crate, no C++ toolchain concerns beyond `cc`), keep Draco as an optional feature via `draco-core` once it earns trust. Both are optional extensions; always keep an uncompressed export path.

**Textures — KHR_texture_basisu / KTX2 (VERIFIED from the extension spec).** KTX2 with `supercompressionScheme` 1 (ETC1S/BasisLZ) or 0/2 (UASTC, optionally Zstd); width/height multiples of 4; full mip chain expected; BT.709+sRGB transfer for colour, UNSPECIFIED+LINEAR for data textures (normal, splat weights); `KTXorientation` must be `rd`. Encoding options in Rust are thin: `basis-universal` (stale FFI) or shelling out to `toktx` (KTX-Software v4.4.2, Apache-2.0) or `gltf-transform` CLI. Recommendation: Phase 1 exports PNG/JPEG/WebP; KTX2 is an "optimise" step via an external tool or the `basis-universal` feature.

**Instancing.** `EXT_mesh_gpu_instancing` is the right carrier for scatter (rocks, trees): per-node `TRANSLATION`/`ROTATION`/`SCALE` accessors. Supported by three.js (`EXTENSIONS` list VERIFIED), Unity glTFast import (no custom `_ID` attributes), Blender export (`export_gpu_instances`, VERIFIED). Unreal: UNVERIFIED. Writing it with `gltf-json` requires the raw `extensions` map (serde_json::Value) — straightforward.

**Validation tools (VERIFIED).**
- `gltf-validator` npm 2.0.0-dev.3.10 (2024-10-22, Apache-2.0) — library (`validateBytes`), **no `bin`**; CLI exists as the Dart app (`dart pub global activate gltf_validator`) — REPORTED. GitHub repo pushed 2026-10-06.
- `@gltf-transform/cli` 4.5.1 (2026-09-28, MIT) — has `bin`; `inspect`, `validate`, `optimize`, `draco`, `meshopt`, `etc1s`/`uastc` (needs KTX-Software `toktx` on PATH — REPORTED).
- Both are Node tools; Jenga Dunia can invoke them as optional dev-time validators, or embed `gltf-validator`'s JS in a Tauri webview for in-app checks.

**Platform notes.** `gltf`/`gltf-json` are pure Rust. `meshopt` compiles C++ via `cc` — works on macOS arm64/Windows MSVC/Linux. `basis-universal` likewise but stale; verify it still builds with current `cc` and Apple clang before adopting.

**Pitfalls.** 4-byte alignment of accessors/bufferViews; Y-up right-handed (section 16); 16-bit index limit if you use `UNSIGNED_SHORT`; `KHR_texture_transform` is the only portable way to tile terrain textures (scale UVs) — three.js/glTFast/Blender all support it; sRGB vs linear texture tagging is implicit (baseColor/emissive sRGB, everything else linear).

---

## 2. OpenUSD (USD / USDA / USDC / USDZ)

**Spec / version.** Pixar OpenUSD 26.08 (2026-07-20) is the latest release; 26.05 (2026-04-24), 26.03 (2026-02-24) (VERIFIED, GitHub releases + CHANGELOG). 26.08 builds with MaterialX 1.39.5; Python 3.9/3.10 support dropped in 26.08 (VERIFIED CHANGELOG). The AOUSD "Core Specification" exists and is what the Rust implementations test against (REPORTED via `openusd` README).

**License (VERIFIED from LICENSE.txt).** "Tomorrow Open Source Technology License 1.0" — the file states it differs from Apache-2.0 *only* in Section 6 (Trademarks). No SPDX identifier is printed in the file; SPDX has not (to my knowledge) registered it — label it `LicenseRef-TOST-1.0` in your register (UNVERIFIED whether SPDX lists it). For practical purposes it is Apache-2.0-equivalent and MIT-compatible for linking/vendoring with attribution.

**Build complexity.** The C++ reference needs TBB, Boost (optional in recent versions), Python (optional), MaterialX, OpenSubdiv, and `build_usd.py`; 30–60 min builds, large binaries, ABI-sensitive. Not something to vendor into a Tauri app. (REPORTED, common knowledge; not re-verified today.)

**Rust options (VERIFIED crates.io/GitHub):**

| Crate | Version | License | Read | Write | Status |
|---|---|---|---|---|---|
| `openusd` (mxpv) | 0.8.0 (2026-10-11) | MIT (+ TOST-1.0 for vendored schemas/generated code) | usda, usdc, usdz | **usda, usdc, usdz** | Pure Rust, no C++. Composition: sublayers, references, payloads, variants, inherits, specializes (LIVERPS), list-ops, instancing, value clips, time samples. `openusd-schemas` 0.8.0 (UsdGeom/UsdShade/UsdSkel/Lux/Physics…), `openusd-build` for custom schemas. Tested against AOUSD core-spec supplemental tests; CI on Linux/macOS/Windows. Pre-1.0 ("APIs may change"); MSRV 1.96; repo pushed 2026-10-19, 142 stars, 3 open issues. **Best candidate.** |
| `openusd-rs` (FloatyMonkey) | 0.1.0 (2025-07-24) | MIT OR Apache-2.0 | usdc, usda (partial) | no | Early WIP, 24 commits, last push 2026-03-08. Not recommended. |
| `rust-usd` | 0.0.5 (2026-06-10) | Apache-2.0 | via pxr C++ | some authoring | FFI to a prebuilt OpenUSD; GitHub repo returned 404 today — UNVERIFIED/abandoned risk. |
| `usd` | 0.0.9 (2020) | — | — | — | Dead placeholder. |
| `tinyusdz` crate | — | — | — | — | **Does not exist** on crates.io. |

**tinyusdz → LightUSD (VERIFIED raw README).** C++17, Apache-2.0 (some MIT helpers), reads *and writes* USDA/USDC/USDZ; v1.0.0 RC on `main`, release planned 26.09/26.10; last tag v0.9.1 (2025-11-04); pip `lightusd`; WASM/JS packages; no Rust/C binding listed. A viable FFI fallback via `cxx`, but with `openusd` 0.8 pure-Rust writing available, not needed for Phase 1.

**Writing USDA as text (pure Rust).** Trivially feasible: USDA is a line-oriented text format (`#usda 1.0`, `def Mesh "x" { ... }`); for terrain you need `UsdGeomMesh` (points, faceVertexCounts, faceVertexIndices, normals, `primvars:st`), `UsdGeomPointInstancer` (scatter), `UsdShadeMaterial`/`UsdPreviewSurface`, `UsdUVTexture`, `metersPerUnit`, `upAxis`. A hand-rolled writer is a few hundred lines and has zero deps; `openusd` gives you usdc/usdz and reading on top. Recommendation: use `openusd` for both; keep a text-writer fallback only if the crate's API churn bites.

**USDZ packaging rules (VERIFIED openusd.org spec).** Zip, **zero compression, unencrypted**; each file's data starts at a 64-byte multiple from package start; first file must be a USD layer (or `defaultLayer.usd` when several root layers); allowed payloads: usda/usdc/usd, png/jpeg/OpenEXR/AVIF, m4a/mp3/wav; read-only (unpack → edit → repack). The `zip` crate 9.0.5 (MIT, 2026-10-17) can write stored entries; 64-byte alignment needs the zip "extra field" padding trick — `usdzip` does it for you; `openusd` 0.8 claims usdz writing (alignment handling UNVERIFIED — validate output with `usdchecker`).

**Materials.** `UsdPreviewSurface` spec v2.5 page, 2.6 current head (VERIFIED openusd.org): inputs `diffuseColor(0.18)`, `emissiveColor`, `useSpecularWorkflow(0)`, `specularColor`, `metallic(0)`, `roughness(0.5)`, `clearcoat`, `clearcoatRoughness(0.01)`, `opacity(1)`, `opacityMode(transparent|presence)`, `opacityThreshold`, `ior(1.5)`, `normal`, `displacement`, `occlusion`. MaterialX-in-USD: OpenUSD 26.08 bundles MaterialX 1.39.5; Blender 5.2 `wm.usd_export` has `generate_materialx_network` (VERIFIED local); Unreal Interchange USD imports embedded MaterialX as UsdPreviewSurface in user reports (REPORTED, forum). Ship `UsdPreviewSurface` always; MaterialX network optional.

**Validation.** `usdchecker` (with `--arkit` for RealityKit constraints — note upstream deprecated `--arkit` in 26.03 and removed it in 26.05, replaced by the validation framework / `--includeKeywords`; the local Apple build 0.25.11 still has `--arkit`), `usdzip` (create/list/validate packages), `usdcat`, `usdtree`.

**Apple USD Tools 0.25.11 (VERIFIED locally; interpretation REPORTED).** macOS 27 ships Pixar-derived CLI tools in `/usr/bin` reporting "Apple USD Tools (0.25.11)": `usdcat`, `usdchecker`, `usdcrush` (mesh/texture compression; same engine as Preview's "compress"), `usdextract` (extract files from `.usdz`, `.glb` or `.gltf`), `usdrecord`, `usdtree`, `usdzip`. The version number tracks OpenUSD 25.11 (inference). WWDC26 session 285 introduced **USDKit** (Swift framework, `USDStage.open`, `exportPackage(... preferSmallMeshFiles)`) and 3D editing/conversion in Preview on macOS 27; OpenPBR is supported by RealityKit/Storm/raytracer renderers (VERIFIED from session transcript). The older "USDZ Tools" (`usdzconvert`, Python) are not installed here. Reality Composer / Quick Look constraints: `usdchecker --arkit` encodes them.

**Platform notes.** `openusd` crate: pure Rust, all three OSes. Apple tools: macOS only. OpenUSD C++: all platforms but heavy.

**Pitfalls.** `upAxis` default is Y but Blender writes Z unless `convert_orientation`; `metersPerUnit` must be set explicitly (default 1.0 only by convention); usdz requires relative asset paths inside the package; texture colour-space via `sourceColorSpace`; `UsdGeomPointInstancer` prototypes must be inside the stage; large meshes should use `usdc` not `usda`.

---

## 3. FBX

**Legal position.** FBX is a **proprietary Autodesk format** — not an open standard, no public spec. The binary format has been reverse-engineered (Blender's importer, ufbx, fbxcel are all clean-room).

**Autodesk FBX SDK (VERIFIED from the local `License.rtf`, 2020.3.9).**
- §1.1.1–1.1.3: you may build "Developed Software" that links the Library and **distribute Licensee Products (free or for a fee)** provided the SDK parts are sublicensed "as is" without warranty.
- §1.1.5: a mandatory notice string ("This software contains Autodesk® FBX® code developed by Autodesk, Inc. …") must appear in your EULA/About box and docs.
- §2.1.1(e),(j): no redistributing the SDK itself (headers/libs) except as set forth; no modifying/reverse-engineering it. §2.1.2: licensed as a single product.
- §2.1.5: using Open Source alongside must not "cause the Software to be subject to any licensing terms other than those set forth" — i.e. the SDK must stay outside your MIT grant.
- §49: SDK source/headers are confidential — **cannot be committed to a public repo.**
- Agreement is accepted per-installer; MIT downstream users would have to accept it themselves.
- **Conclusion:** an MIT project can offer an *optional, non-default* Cargo feature that links a user-supplied FBX SDK (like `ffmpeg-sys` with proprietary codecs), and can ship *binaries* that embed it with the notice, but cannot vendor it. Keep it out of the default build and CI.
- Platforms: 2020.3.9 macOS package is a **universal x86_64+arm64 dylib/static lib** (VERIFIED with `lipo`); Linux builds exist (AUR packages 2020.3.7 — REPORTED); Windows MSVC yes. Rust FFI would need a C shim (the SDK is C++ classes) — `cxx` or a hand-written C wrapper.

**Alternatives (VERIFIED):**

| Option | Version | License | Read | Write | Notes |
|---|---|---|---|---|---|
| `ufbx` crate | 0.11.5 (2026-09-25) | MIT OR Unlicense | binary + ASCII FBX ≥ v3000, OBJ | **no** | Bindings to ufbx C (v0.23.1, same license, single-file C99, builds with `cc`). Materials normalised to PBR, skinning, blend shapes, animation, embedded textures. Mature C core (1.4k stars, heavy fuzzing). Best read path. |
| `fbxcel` | 0.9.0 (2022-10-29) | MIT OR Apache-2.0 | binary 7.4/7.5 (pull parser) | **yes, binary 7.4/7.5 (`writer` feature)** | Low-level node tree only — you construct the FBX object graph (Definitions/Objects/Connections) yourself. No ASCII. `fbxcel-dom` "incomplete and unmaintained". Repo push 2026-04-11. |
| `fbx-dom`, `oxideav-fbx`, `fbx` | 0.1.2 / 0.0.3 / 0.2.0 | MIT/Apache | read | no | Small/young; not recommended. |
| Assimp | v6.0.5 (2026-04-30) | BSD-3-Clause (REPORTED; GitHub shows "Other") | yes | **FBX exporter exists**, quality poor | Open issues: per-face materials ignored, axis metadata ignored, blendshape double/float mismatch, crashes (REPORTED, search of issue titles). `russimp` 3.2.1 (2025-04-30) declares a non-standard license string — check before use. Not recommended for FBX write. |
| Blender bridge | 5.2 LTS | GPL-3.0 (process boundary) | yes (binary only) | yes (binary 7.4) | `blender -b -P export.py`: glTF/USD in → FBX out. Blender's importer does **not** read ASCII FBX (REPORTED, long-standing). Highest-quality free writer. |
| Write FBX ASCII yourself | — | — | — | feasible | Unity and Unreal use the Autodesk SDK and read ASCII FBX (REPORTED); Blender does not. Format is undocumented; static-mesh-only ASCII (Geometry + Model + Material + Connections) is ~500 lines of Rust and widely cloned. Risky for anything beyond static meshes. |

**Recommendation.** Read via `ufbx`. Write via the Blender bridge (Phase 2) and, if demanded, a minimal `fbxcel`-based binary 7.4 static-mesh writer (Phase 3/4). Treat the FBX SDK as an opt-in feature for users who accept Autodesk's terms. Communicate clearly: "FBX is proprietary; glTF/USD are the canonical interchange paths."

---

## 4. OBJ / STL / PLY

| Format | Crate | Version | License | Read | Write | Notes (VERIFIED crates.io) |
|---|---|---|---|---|---|---|
| OBJ | `tobj` | 4.0.5 (2026-08-02) | MIT | yes | no | tinyobjloader-style, triangulation, MTL; 3M downloads; maintained. |
| OBJ | `obj` (kvark) | 0.10.2 (2020-11-30) | Apache-2.0 | yes | yes (REPORTED) | Old. |
| OBJ | `wavefront_obj` | 11.0.0 (2025-01-08) | MIT | yes | partial | Piston. |
| OBJ | `obj-rs` | 0.7.4 (2024-09-28) | Apache-2.0 OR MIT | yes | no | |
| STL | `stl_io` | 0.11.0 (2026-03-15) | MIT | ascii+binary | ascii+binary | 5M downloads; maintained. |
| PLY | `ply-rs` | 0.1.3 (2020-08-27) | MIT | ascii+binary | ascii+binary | Unmaintained since 2020 but format is frozen; works. |
| all three | `draco-io` | 0.6.0 (2026-10-10) | Apache-2.0 | OBJ/PLY/STL/FBX | OBJ/PLY/STL | Part of draco-rust. |

**Triviality.** Writing OBJ (`v`/`vt`/`vn`/`f`, 1-based indices, `mtllib`/`usemtl`), binary STL (80-byte header, u32 count, 50-byte triangles) and binary PLY (header + packed records) is 50–150 lines each. Recommendation: **write your own exporters** (no deps, full control of axis/units/precision), use `tobj` + `stl_io` + `ply-rs` for import. OBJ has no units/up-axis; Blender's OBJ I/O defaults to `forward=-Z, up=Y` with `global_scale=1` (VERIFIED local), i.e. it assumes Y-up metres — match that.

---

## 5. EXR

`exr` 1.74.2 (2026-07-10), **BSD-3-Clause**, pure Rust, `#[forbid(unsafe_code)]` (VERIFIED). Read + write; f16/f32/u32 channels; compression: none, RLE, ZIP(S), PIZ, PXR24, B44/B44A, DWAA/DWAB, HTJ2K; multi-part/multi-layer; **deep data not supported**. 80M downloads (pulled in by `image`, which uses it decode-only). Use `exr` directly for a single-channel `R` f32 heightmap (ZIP compression, scanlines). This is the preferred lossless float heightfield exchange format for Blender (Displace/Image Texture, float), Houdini, Gaea, World Machine; Unity/Unreal do not import EXR heightmaps directly (convert to PNG16/RAW). Pitfall: `exr` ships OpenEXR 2.x features; nothing engine-specific.

---

## 6. PNG / TIFF 16-bit

- `image` 0.25.10 (2026-03-10), MIT OR Apache-2.0, MSRV 1.88 (VERIFIED). PNG encode/decode via `png` 0.18.1 (2026-02-14, MIT OR Apache-2.0); 16-bit grayscale (`ImageLuma16`/`Luma<u16>`) encodes to 16-bit PNG — VERIFIED by crate type system (`ImageBuffer<Luma<u16>,_>::save`), behaviour REPORTED. TIFF encode/decode supported; EXR and HDR decode only (VERIFIED docs.rs codec table).
- `tiff` 0.11.3 (2026-02-10), MIT, pure Rust (VERIFIED). Encoder colour types include `Gray8/16/32/64`, `GrayI8..I64`, `Gray32Float`, `Gray64Float`, `RGB*`, `RGBA*`, `*Float` (VERIFIED docs.rs). `DirectoryEncoder::write_tag` accepts the `Tag` enum, which includes `Unknown(u16)` **and named GeoTIFF tags** `ModelPixelScaleTag(33550)`, `ModelTiepointTag(33922)`, `ModelTransformationTag(34264)`, `GeoKeyDirectoryTag(34735)`, `GeoDoubleParamsTag(34736)`, `GeoAsciiParamsTag(34737)`, `GdalNodata(42113)` (VERIFIED docs.rs). So **32-bit float TIFF and basic GeoTIFF writing are feasible in pure Rust** with `tiff` alone.
- Pitfalls: PNG 16-bit is big-endian in-file (handled by `png`); Unity/Unreal want *unsigned* 16-bit grayscale PNG with no alpha; `image` normalises when converting between depths — write `Luma<u16>` directly, don't go through `DynamicImage::to_luma8`.

---

## 7. RAW heightfields & splatmaps

**Conventions (industry, REPORTED unless noted).**
- `.r16`: headerless, row-major, 16-bit unsigned, little-endian (Unreal's native; World Machine/Gaea default). `.r32`: 32-bit float LE. `.r8`: 8-bit.
- **Unity Terrain RAW (VERIFIED manual + script ref):** Import Raw / Export Raw take 8- or 16-bit, with **Byte Order = Windows (little-endian) or Mac (big-endian)** (the labels are VERIFIED; their endian meaning is REPORTED — GDAL output needed "Mac" in one report), and **Flip Vertically**. Heightmap resolution must be 2^n+1: `TerrainData.heightmapResolution ∈ {33, 65, 129, 257, 513, 1025, 2049, 4097}`, clamped (VERIFIED). Height is normalised: 0..65535 maps to 0..`terrainData.size.y` (Terrain Height, 1–10,000 units; width/length 1–100,000) (VERIFIED). Row 0 of the RAW is the *bottom* (south) row in Unity's `GetHeights` convention — hence the Flip option; test one import.
- **Unreal Landscape (VERIFIED Landscape Technical Guide + import docs):** preferred inputs 16-bit grayscale PNG, `.r16`, `.r8`; RAW needs a same-name `.json` sidecar with `width`, `height`, bit depth. Height is stored as 16-bit mapped to −256..+255.992 units then multiplied by Z scale: at Z=1 range is ±256 cm, default Z=100 gives ±256 m. **Z scale = (max height m) × 100 × 0.001953125 (=1/512).** Example: 4207 m → 821.68. Recommended overall resolutions (quads = 63 or 127/section, 1 or 4 sections/component): **8129, 4033, 2017, 1009, 505, 253, 127**, ≤1024 components. Mid-grey (32768) = height 0. X/Y scale 100 = 1 m per quad.
- **Unreal weightmaps (VERIFIED custom-layers doc):** one file per layer, **8-bit single-channel grayscale PNG or RAW**, resolution must equal the landscape Overall Resolution or import is refused (e.g. 1024² heightmap → 1072² layer; 2048² → 2160²; 4096² → 4336² when the heightmap is not already a recommended size). Layers are weight-blended (`LandscapeLayerBlend` LB_WeightBlend) or alpha-blended (`Layered`/`Additive` import choice).
- **Unity splatmaps (VERIFIED `TerrainData.SetAlphamaps`):** `float[y, x, layer]` ("y and x are not in the traditional order"), values 0..1, weights per texel should sum to 1 (REPORTED). Resolution = Control Texture Resolution (independent of heightmap). `TerrainLayer` assets hold diffuse/normal/mask + tiling. Unity Terrain Tools 5.3.3 (VERIFIED) adds Toolbox import/export of heightmaps and splatmaps.
- **Recommended sizes for Jenga Dunia:** author at 2^n+1 (1025/2049/4097) for Unity; resample to 1009/2017/4033/8129 for Unreal; export both `.r16` LE and PNG16, write a sidecar JSON with min/max height so the Unreal Z-scale and Unity Terrain Height can be computed.

---

## 8. GeoTIFF / geospatial

| Crate | Version | License | Notes (VERIFIED crates.io / README) |
|---|---|---|---|
| `geotiff` (georust) | 0.1.0 (2025-06-10) | MIT | Read-only, "works for very basic GeoTIFFs", per-pixel `get_value_at`; built on `tiff`; repo push 2026-07-31, 9 open issues. |
| `georaster` | 0.2.0 (2025-01-11) | MIT/Apache-2.0 | Thin GeoTIFF reader; small. |
| `cog3pio` | 0.1.0 (2026-05-25) | MIT OR Apache-2.0 | Cloud-Optimized GeoTIFF parallel reader; MSRV 1.91; young. |
| `gdal` | 0.19.0 (2025-12-23) | MIT | Bindings to **system libgdal** (GDAL 3.4–3.11 prebuilt bindings; `bindgen` feature otherwise); docs.rs build of 0.19 failed. GDAL itself is MIT-style (VERIFIED LICENSE.TXT) with many permissive sub-licences; but dynamic deps (PROJ, GEOS LGPL, libtiff, HDF, …) make bundling into a Tauri app heavy (≥100 MB, per-platform packaging). |
| `proj` | 0.31.0 (2025-08-29) | MIT OR Apache-2.0 | Needs libproj 9.6.x or `bundled_proj` (builds from source; needs SQLite3 + libtiff). PROJ 9.9.0 (2026-09-15) is MIT. |

**Recommendation.** Phase 1: read GeoTIFF DEMs in pure Rust using `tiff` directly (read `ModelPixelScale`, `ModelTiepoint`, `GeoKeyDirectory`, `GdalNodata`; support uncompressed/LZW/Deflate tiled+striped, i16/f32 samples) — `geotiff` 0.1 is too thin and `tiff` already exposes the tags. Write GeoTIFF the same way. CRS reprojection: defer; offer `proj` behind a `geo` feature (bundled build) for UTM↔WGS84, or just carry the CRS WKT/EPSG through untouched and let GDAL users pre-process. COG output = tiled TIFF + overviews + specific IFD ordering; `tiff` can write tiles (`ImageEncoder` tiled support UNVERIFIED for 0.11 — check) — defer to Phase 4.

---

## 9. OpenVDB

- License: **Apache-2.0** since OpenVDB 12.0 (relicensed from MPL-2.0; VERIFIED GitHub license detection + README mention of RE-LICENSE_NOTE.txt; the "12.0" trigger is REPORTED from Phoronix/openSUSE changelog). Latest v13.1.0 (2026-09-16).
- Rust: `vdb-rs` 0.6.0 (2024-01-04), MIT, **read-only**, pure Rust; missing: writing, multi-pass I/O (point grids), older file versions, delay loading (VERIFIED README). Repo push 2026-10-01. `openvdb`/`openvdb-sys` 0.1.0 are placeholders. NanoVDB: header-only C++ (Apache-2.0), no Rust crate found.
- **Defer.** Heightfield terrain has no need for sparse volumes; caves/overhangs would motivate it in Phase 4+. If needed, go via Blender (Volume object) or Houdini.

---

## 10. MaterialX & OpenPBR

- **MaterialX 1.39.5** (2026-05-22), Apache-2.0 (VERIFIED). 1.39.0 added OpenPBR Surface; 1.39.5 updates OpenPBR to v1.1.1 and UsdPreviewSurface to the latest Pixar spec; glTF PBR node graph consolidated; no 1.40 yet. OpenUSD 26.08 pins 1.39.5.
- **OpenPBR Surface v1.1.1** (2026-04-17), repo Apache-2.0 (spec text badge says CC BY-SA 4.0 — inconsistent, VERIFIED). Reference implementation is the MaterialX nodedef `open_pbr_surface` in `open_pbr_surface.mtlx` (REPORTED from filename).
- **Document structure.** `.mtlx` is XML: `<materialx version="1.39">` → `<nodegraph>` with `<image>`, `<tiledimage>`, `<normalmap>` nodes → `<open_pbr_surface>` / `<standard_surface>` / `<UsdPreviewSurface>` / `<gltf_pbr>` surface shader → `<surfacematerial>`. **A pure-Rust writer is trivial** (string templating or `quick-xml`); a reader for *your own* subset is easy; a general MaterialX graph evaluator is not (that's the C++ library's job). No usable Rust binding exists (`materialx` 0.0.0 placeholder; `shadewire-mtlx` 0.3.0 published today, 29 downloads — UNVERIFIED).
- **Consumption in 2026:** Blender 5.2 writes a MaterialX network into USD on export (`generate_materialx_network`, VERIFIED) and reads USD MaterialX networks via Hydra Storm/`usd` importer (REPORTED); Unreal 5.x has a MaterialX Interchange translator (standalone .mtlx import, REPORTED) and USD-embedded MaterialX currently lands as UsdPreviewSurface (REPORTED, forum); Unity has no MaterialX import (REPORTED); Maya has LookdevX (installed locally, VERIFIED directory `lookdevx`) which authors/reads MaterialX natively (REPORTED); Apple RealityKit renders OpenPBR (VERIFIED WWDC26). Net: ship **glTF PBR + UsdPreviewSurface** as the portable baseline; emit an OpenPBR `.mtlx` sidecar for DCCs.

**Channel equivalence table (terrain-relevant channels)**

| Concept | glTF 2.0 metallic-roughness | UsdPreviewSurface 2.6 | OpenPBR 1.1.1 | Loss notes |
|---|---|---|---|---|
| Albedo | `baseColorFactor` × `baseColorTexture.rgb` (sRGB) | `diffuseColor` (linear) | `base_color` × `base_weight` | Lossless (sRGB→linear) |
| Alpha / cutout | `baseColorTexture.a`, `alphaMode` OPAQUE/MASK/BLEND, `alphaCutoff` | `opacity`, `opacityMode` presence/transparent, `opacityThreshold` | `geometry_opacity` only | OpenPBR has no cutoff threshold; MASK → bake |
| Metalness | `metallicFactor` × `metallicRoughnessTexture.b` | `metallic` | `base_metalness` | Lossless (channel packing differs: glTF packs in B) |
| Roughness | `roughnessFactor` × `metallicRoughnessTexture.g` | `roughness` | `specular_roughness` | Lossless; glTF packs in G |
| Specular (dielectric F0) | `KHR_materials_specular` / `KHR_materials_ior` (default ior 1.5) | `ior` 1.5 (when `useSpecularWorkflow=0`), `specularColor` when =1 | `specular_weight`, `specular_color`, `specular_ior` 1.5 | glTF core has no spec control; extension needed |
| Normal | `normalTexture` (+ `scale`) tangent-space, OpenGL +Y | `normal` (tangent-space via UsdUVTexture, scale/bias to −1..1) | `geometry_normal` | Lossless; watch DirectX vs OpenGL green channel (Unreal expects DX −Y — flip on export) |
| Occlusion | `occlusionTexture.r` (+ `strength`) | `occlusion` | — (none) | **Lost in OpenPBR**; bake into base_color or drop |
| Emission | `emissiveFactor` × `emissiveTexture`, `KHR_materials_emissive_strength` | `emissiveColor` | `emission_color` × `emission_luminance` (cd/m²) | Units differ: glTF strength is unitless; OpenPBR is nits — document conversion |
| Clearcoat | `KHR_materials_clearcoat` | `clearcoat`, `clearcoatRoughness` | `coat_weight`, `coat_roughness`, `coat_ior`, `coat_color` | Fine for wet-rock look; coat_color lost in glTF/USD |
| Sheen / fuzz | `KHR_materials_sheen` | — | `fuzz_*` | Lost in UsdPreviewSurface |
| Transmission / SSS | `KHR_materials_transmission/volume` | — | `transmission_*`, `subsurface_*` | Lost in UsdPreviewSurface; irrelevant for terrain |
| Displacement / height | — (none) | `displacement` (float) | — (`displacement` lives on the MaterialX `displacementshader`) | **glTF cannot carry displacement**; send heightmap separately |
| UV tiling | `KHR_texture_transform` (offset/rotation/scale) | `UsdTransform2d` node (REPORTED) | `place2d`/`tiledimage` node | Lossless if the importer supports it |
| Anisotropy | `KHR_materials_anisotropy` | — | `specular_roughness_anisotropy`, `geometry_tangent` | Lost in UsdPreviewSurface |
| Thin-walled | `KHR_materials_volume` thickness 0 | — | `geometry_thin_walled` | — |
| Unlit | `KHR_materials_unlit` | (none; emulate with emissive) | — | Lossy |

Terrain layering (splat blending of N layers) has **no representation in any of the three** — each engine builds its own terrain shader. Export per-layer materials as separate glTF/USD materials plus the weightmaps as data textures; provide a baked single-material option for viewers.

---

## 11. Blender

- **Version:** Blender 5.2 LTS (2026-07-14); 5.1 (2026-03-17); 5.0 (2025-11-18); 4.5 LTS (2025-07-15); 4.2 LTS (2024-07-16) (VERIFIED blender.org). Local install: 5.2.0 LTS.
- **Background mode:** `blender -b [file.blend] -P script.py -- args` (standard; VERIFIED by running it today). Also `blender --command extension build|validate` for packaging (VERIFIED manual).
- **Key operator options (VERIFIED by introspecting bpy in Blender 5.2):**
  - `bpy.ops.export_scene.gltf`: `export_format` (GLB / GLTF_SEPARATE / GLTF_EMBEDDED — dynamic enum, REPORTED), `export_yup=True`, `export_apply`, `export_image_format` {AUTO, JPEG, WEBP, NONE} + `export_image_add_webp`/`export_image_webp_fallback`, `export_draco_mesh_compression_enable` (+ `_level`, quantization bits), **`export_meshopt_extension`**, `export_gpu_instances`, `export_materials` {EXPORT, PLACEHOLDER, VIEWPORT, NONE}, `export_lights`, `export_extras`, `export_hierarchy_flatten_objs`, `use_selection`, `collection`. **No KTX2/basisu export.** Add-on io_scene_gltf2 5.2.39.
  - `bpy.ops.import_scene.gltf`: `merge_vertices`, `import_shading`, `bone_heuristic`.
  - `bpy.ops.export_scene.fbx`: `axis_forward='-Z'`, `axis_up='Y'`, `global_scale=1.0`, `apply_unit_scale=True`, `apply_scale_options` {FBX_SCALE_NONE|UNITS|CUSTOM|ALL}, `bake_space_transform`, `use_mesh_modifiers`, `path_mode` {AUTO…COPY}, `use_selection`. Blender writes binary FBX 7.4 (REPORTED). Add-on io_scene_fbx 5.15.0.
  - `bpy.ops.import_scene.fbx`: same axis/scale options, `mtl_name_collision_mode`.
  - `bpy.ops.wm.usd_export`: `convert_orientation` (default False → Z-up), `export_global_forward_selection='NEGATIVE_Z'`, `export_global_up_selection='Y'`, `convert_scene_units` {METERS…CUSTOM} + `meters_per_unit`, `generate_preview_surface`, `generate_materialx_network`, `export_textures_mode` {KEEP, PRESERVE, NEW}, `usdz_downscale_size`, `use_instancing`, `export_subdivision` {IGNORE, TESSELLATE, BEST_MATCH}, `triangulate_meshes`, `root_prim_path='/root'`, `export_custom_properties`, `evaluation_mode` {RENDER, VIEWPORT}.
  - `bpy.ops.wm.usd_import`: `scale`, `import_usd_preview`, `import_textures_mode` {IMPORT_NONE, IMPORT_PACK, IMPORT_COPY}, `support_scene_instancing`, `validate_meshes`.
  - `bpy.ops.wm.obj_export/import`: `forward_axis='NEGATIVE_Z'`, `up_axis='Y'`, `global_scale`.
- **Extensions system (VERIFIED manual, 4.2+):** `blender_manifest.toml` with `schema_version="1.0.0"`, `id`, `name`, `version` (semver), `tagline` (≤64 chars), `maintainer`, `type="add-on"`, `blender_version_min` (≥4.2.0), `license=["SPDX:GPL-3.0-or-later"]`; optional `platforms`, `wheels`, `permissions`. Validate with `blender --command extension validate`.
- **Licensing (VERIFIED blender.org/about/license + extensions/licenses):** Blender is GPL-2.0-or-later (binaries GPL-3.0-or-later). Blender's position: scripts using `bpy` that are *published* must be under a GPL-compatible licence; the **Extensions Platform requires GPL-3.0-or-later for add-ons** (themes: any GPL-compatible; bundled assets CC0). **Implication for Jenga Dunia:** keep the Blender add-on in a **separate repository under GPL-3.0-or-later**; the Rust core stays MIT. The Rust app may *invoke* Blender as a subprocess (process boundary, no linking) and pass data via files/CLI without licence contamination — this is the same model as any CLI tool calling Blender. Do not embed `bpy` (the PyPI `bpy` wheel is GPL) in the MIT app.

---

## 12. Maya

- **Local:** Maya 2027 (API 20270200), Maya USD 0.37.0 (release 2026-07-23, VERIFIED GitHub), `mayapy` present, `maya -batch`, `-prompt`, `-command`, `-file`, `-script`, `-noAutoloadPlugins` flags (VERIFIED `maya -help`). Devkit is a separate download ("README_DEVKIT_MOVED.txt", VERIFIED).
- **Devkit / SDK licence:** governed by the Autodesk License and Services Agreement (REPORTED); plugins you write are yours (many MIT/Apache Maya plugins exist, e.g. appleseed-maya), but devkit headers/libs are not redistributable; building a C++ Maya plugin requires each builder to have the devkit. Python (`maya.cmds`, `OpenMaya` 2.0) needs no devkit — **prefer Python**.
- **Batch licensing:** running `mayapy`/`maya -batch` consumes a Maya licence per Autodesk forum answers; some say batch mode is covered — conflicting, UNVERIFIED. Assume a licensed seat is needed.
- **Formats:** FBX native (`fbxmaya` plugin, Autodesk SDK). USD via maya-usd (`mayaUSDImport`/`mayaUSDExport`, Apache-2.0, VERIFIED repo). OBJ native. **No native glTF import/export in Maya 2026/2027** (REPORTED: CG Channel July 2026; Khronos sponsored a 3ds Max glTF plugin, "no plans" for Maya); third-party: ProtoTech, SimLab, Babylon exporter (unmaintained).
- **Integration boundary recommendation:** do not write a Maya plugin. Deliver **USD (usdc/usda) + FBX** files, plus an optional ~100-line `mayapy` script (MIT, it only *calls* Maya; Autodesk doesn't claim scripts) that imports the USD stage, sets units (`currentUnit -l cm`) and assigns materials. Maya USD honours `metersPerUnit`/`upAxis` on import (REPORTED).

---

## 13. Unreal Engine

- **Version:** UE 5.8 released 2026-06-17 (VERIFIED 80.lv article date), described as the last planned major UE5 release before UE6 (REPORTED). Docs site serves "Unreal Engine 5.8 Documentation".
- **Landscape import (VERIFIED):** see §7 — 16-bit PNG / `.r16` / `.r8` / RAW+JSON; Z scale = max_height_m × 100 / 512; recommended overall resolutions 127/253/505/1009/2017/4033/8129; weightmaps 8-bit grayscale, one per layer, exact resolution match; "Flip Y Axis"; import into World Partition with grid/region sizes; Edit Layers; Resample/Expand/Subregion modes; export heightmaps as PNG16/r16, tiled when World Partition is on.
- **Landscape material layers:** `LandscapeLayerBlend` node with LB_WeightBlend / LB_AlphaBlend / LB_HeightBlend; layer info objects per layer (REPORTED, standard).
- **World Partition:** streaming grid cells; landscapes are split into proxies per region (REPORTED). Nanite on Landscape: supported, but non-Nanite data is still kept for RVT/water, roughly doubling streamed data (VERIFIED docs).
- **Mesh Terrain (5.8, experimental):** mesh-based terrain workflow built on PCG, meant for very large worlds (VERIFIED 80.lv summary). Relevant later for Jenga Dunia's mesh-terrain export path.
- **Interchange framework:** format-agnostic import pipeline. glTF (.gltf/.glb) is imported through Interchange (`UInterchangeGLTFTranslator`: textures, materials, static meshes, skeletal meshes, animation — VERIFIED API reference listing); supported glTF extension list could not be retrieved (page is JS-rendered) — UNVERIFIED (community reports KHR_texture_transform and materials extensions supported; instancing/meshopt/basisu UNVERIFIED). USD via Interchange: `InterchangeOpenUSD` plugin with `UInterchangeUSDTranslator`/`UInterchangeUsdPipeline` (options `bGeneratePrimvarCompatibleMaterials`, `SubdivisionLevel`) exists (VERIFIED API index); Epic is prioritising it over the older USD Stage actor (REPORTED forum). USD Stage actor/importer is **Beta**, supports usd/usda/usdc/usdz, UsdPreviewSurface + DisplayColor materials, HISM for point instancers, UsdLux lights, runtime loading with `FORCE_ANSI_ALLOCATOR=1` (VERIFIED docs; bundled USD version not stated on page). FBX: native (Autodesk SDK), Interchange FBX translator. Datasmith: for CAD/Archviz DCC bridges; not needed for terrain.
- **Material loss notes:** glTF → Unreal creates Material Instances off a Khronos-shipped master; emissive strength and texture transform map; occlusion maps to AO; no Landscape material is generated from any format — terrain layers must be reconstructed in-engine (consider shipping a small Python `unreal` editor script: `unreal.LandscapeEditorSubsystem` / `LandscapeImportLayerInfo` — UNVERIFIED API names).
- **Coordinate:** Z-up, left-handed, centimetres. glTF/USD importers convert automatically (Y-up → Z-up, m → cm); heightmap RAW has no axes — Flip Y if your row 0 is north.

---

## 14. Unity

- **Version:** Unity 6 generation (`6000.x`); local 6000.4.8f1 (VERIFIED). Manual pages exist through 7000.0 (VERIFIED search results).
- **Terrain heightmap (VERIFIED):** RAW 8/16-bit, Byte Order Windows|Mac, Flip Vertically, resolution 33–4097 (2^n+1), Terrain Height 1–10,000, width/length 1–100,000. PNG import of heightmaps is via Terrain Tools Toolbox (REPORTED) or script (`TerrainData.SetHeights(float[,] 0..1)`).
- **TerrainLayers / alphamaps:** `TerrainData.terrainLayers`, `SetAlphamaps(x,y,float[y,x,layer])`, Control Texture Resolution; Unity packs 4 layers per RGBA splat texture internally (REPORTED).
- **Terrain Tools package:** `com.unity.terrain-tools` 5.3.3 — Toolbox creates terrains from heightmaps, batch settings, import/export heightmaps & splatmaps, erosion brushes (VERIFIED).
- **glTF:** `com.unity.cloud.gltfast` 6.20.0, requires **Unity 6000.0+** (VERIFIED). Import: KHR_texture_transform ✓, KHR_texture_basisu ✓ (needs `com.unity.cloud.ktx`), KHR_draco_mesh_compression ✓ (needs `com.unity.cloud.draco`), EXT_meshopt_compression ✓ (needs `com.unity.meshopt.decompress`), EXT_mesh_gpu_instancing ✓ (no custom attributes), KHR_mesh_quantization ✓, KHR_materials_variants ✓, KHR_lights_punctual ✓; emissive_strength not marked. Export: texture_transform, Draco, lights; editor export "experimental". Runtime + editor import; URP/HDRP/BiRP all supported, but editor import of default Lit/Standard materials "not yet supported" in some paths (VERIFIED feature page/landing). UnityGLTF (Khronos/prefrontal cortex fork) remains an alternative — not re-verified today.
- **USD:** legacy `com.unity.formats.usd` deprecated 2024-11-15, no Apple Silicon (VERIFIED README). New: `com.unity.usd.core` 1.0.0-pre.1 (C# bindings to **OpenUSD 23.02**, Unity 2023.1+, Win/macOS/Linux), `com.unity.importer.usd` 1.0.0-pre.2 (meshes, transforms, textures, skeletal/blendshape animation, cameras, limited lights, **UsdPreviewSurface only**), `com.unity.exporter.usd` experimental; exporter docs list "OSX (Intel and Silicon)" (REPORTED). All still pre-release in 2026 — treat Unity USD as secondary.
- **FBX:** native importer (Autodesk SDK), plus `com.unity.formats.fbx` exporter/round-trip package (REPORTED).
- **Render pipelines:** material mapping differs — BiRP `Standard`, URP `Universal Render Pipeline/Lit`, HDRP `HDRP/Lit`; glTFast handles all three with its own shader graphs; terrain uses `Nature/Terrain/Standard` (BiRP), `Universal Render Pipeline/Terrain/Lit`, `HDRP/TerrainLit` with the same TerrainLayer data (REPORTED).
- **Mesh terrain vs Terrain object:** glTF/USD/FBX meshes become static `MeshRenderer`s (no Terrain tools, no detail/tree painting, Nanite-equivalent absent); the Unity `Terrain` object needs heightmap + layers + alphamaps via script. Ship both: RAW/PNG16 + splat PNGs + a C# editor script (`TerrainData` API) for the Terrain path; glTF for the mesh path.

---

## 15. Web viewers

- **three.js r186** (2026-09-24), MIT (VERIFIED). `GLTFLoader` `EXTENSIONS` (VERIFIED from source): KHR_draco_mesh_compression, KHR_texture_basisu, KHR_texture_transform, KHR_mesh_quantization, KHR_materials_emissive_strength, EXT_meshopt_compression (+ KHR_meshopt_compression alias), EXT_mesh_gpu_instancing, EXT_texture_webp/avif, materials clearcoat/sheen/transmission/volume/ior/specular/iridescence/anisotropy/dispersion/diffuse_roughness/unlit, lights_punctual. Wire `setDRACOLoader`, `setKTX2Loader` (needs `detectSupport(renderer)` and the basis transcoder WASM), `setMeshoptDecoder`. WebGPU renderer is production in r16x+ (REPORTED).
- **Babylon.js** 9.31.0 npm / 9.30.0 GH release (2026-10), Apache-2.0 (VERIFIED versions). glTF loader supports Draco, meshopt, KTX2/Basis, texture_transform, instancing (REPORTED).
- **`<model-viewer>`** 4.4.0 (2026-10-09), Apache-2.0 (VERIFIED); built on three.js, supports Draco/KTX2/meshopt out of the box (REPORTED); also loads USDZ for iOS AR Quick Look.
- **3D Tiles 1.1** (OGC community standard, 2023-06-05, VERIFIED tag; spec text CC-BY/Apache) — tiles are glTF; CesiumJS (Apache-2.0) renders them. Optional later path for geospatial streaming of large terrains; the `glTF + KTX2 + meshopt` stack above is a prerequisite anyway.
- **Pitfalls:** KTX2 in browsers needs the transcoder WASM served alongside; UASTC → BC7/ASTC availability varies, ETC1S is the safe default for albedo; three.js expects `KHR_texture_transform` offset/scale in glTF UV space (V flipped vs Blender); WebGPU path in three.js needs `three/webgpu` entry.

---

## 16. Coordinate conventions

| Target | Up | Handedness | Native unit | Forward (convention) | Transform from Jenga Dunia canonical (**Y-up, RH, metres, −Z forward = glTF**) |
|---|---|---|---|---|---|
| glTF 2.0 | +Y | RH | m | −Z (by convention; spec: +Z front) | identity |
| USD | configurable `upAxis` (Y default, Z allowed), `metersPerUnit` (default 1 but *set it*) | RH | arbitrary | — | write `upAxis="Y"`, `metersPerUnit=1` → identity. For Z-up consumers set `upAxis="Z"` and rotate +90° about X. |
| FBX | +Y default (axis system stored in file; Z-up possible) | RH | **cm** default (`UnitScaleFactor=1 cm`) | −Z | scale ×100 and set `UnitScaleFactor=1`; or write `UnitScaleFactor=100` (metres) and let importers convert. |
| OBJ | none (de facto Y-up) | RH (de facto) | none | — | identity; document "metres, Y-up". |
| Unreal | **+Z** | **LH** | **cm** | +X | rotate +90° about X (Y→Z), **mirror Y** (RH→LH; Unreal's importers do Y-negation), scale ×100. Importers handle this for glTF/USD/FBX; for raw meshes you write: `(x, y, z)_gltf → (x·100, −z·100, y·100)` (REPORTED; verify by round-trip test). |
| Unity | +Y | **LH** | m | +Z | mirror one axis: `(x, y, z) → (−x, y, z)` or `(x, y, −z)` (Unity's glTF/FBX importers flip X by convention — glTFast flips X, REPORTED) + wind triangle order. |
| Blender | **+Z** | RH | m | −Y | rotate +90° about X: `(x, y, z)_gltf → (x, −z, y)`. Blender's importers/exporters do this automatically (glTF `export_yup`, FBX/OBJ/USD axis options). |
| Maya | +Y default (Z-up option) | RH | **cm** default | −Z | scale ×100; identity rotation. |
| Houdini / Gaea / World Machine (heightmaps) | n/a | n/a | m | row 0 = north (top) in images; Unity RAW row 0 = south | Heightmap *row order* is the usual source of "flipped terrain" — expose a flip toggle on every heightfield export. |

Winding: glTF/USD/Blender/Maya are CCW front faces; after a mirror (Unity/Unreal) winding must be reversed to keep normals outward.

---

## (a) Import / Export feature matrix

Status key: **P1–P4** = recommended phase; ✓ = supported by the recommended path; ◐ = partial; ✗ = not in scope.

| Format | Import | Export | Round-trip | Geometry | Materials | Textures | Hierarchy | Instancing | Animation | Known losses | Dependencies (Rust) | License | Validation tool |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **glTF 2.0 / GLB** | ✓ P1 (`gltf`) | ✓ P1 (`gltf-json` + own GLB writer) | ✓ | tri meshes, u16/u32 idx, tangents | PBR MR + texture_transform, emissive_strength, clearcoat | PNG/JPEG/WebP P1; KTX2 P3 | nodes/scenes | EXT_mesh_gpu_instancing P2 | ✗ (terrain) | no displacement; no terrain layers; 2-UV practical limit | `gltf`, `gltf-json`, `meshopt` (P2), `draco-core` (P3, opt) | MIT/Apache-2.0 | gltf-validator (npm), gltf-transform |
| **USD (usda/usdc)** | ✓ P2 (`openusd`) | ✓ P1 usda (own writer) → P2 via `openusd` | ◐ | UsdGeomMesh | UsdPreviewSurface P1; MaterialX net P3 | relative paths; png/jpg/exr | prims, Xform, variants | UsdGeomPointInstancer P2 | ✗ | DCC-specific shader graphs; variants if hand-written | `openusd`, `openusd-schemas` | MIT + TOST-1.0 | usdchecker, usdcat, usdtree |
| **USDZ** | ✓ P2 | ✓ P2 (`zip` stored + 64-B align, or `openusd`) | ◐ | same | UsdPreviewSurface | png/jpg/exr/avif inside | same | same | ✗ | no compression; ARKit limits (≤ ~100 MB practical) | `openusd`, `zip` | MIT | usdchecker --arkit, usdzip |
| **FBX** | ✓ P2 (`ufbx`) | ◐ P2 via Blender bridge; P4 `fbxcel` binary static-mesh writer; opt-in Autodesk SDK feature | ✗ | meshes, normals, UVs | Phong/Lambert + `FbxSurfaceMaterial`; PBR via Stingray/Maya props only | embedded or referenced | nodes | none standard | ✗ | proprietary; materials lossy; ASCII not readable by Blender | `ufbx`; `fbxcel` (opt) | MIT/Unlicense; MIT/Apache | none open (Autodesk FBX Review app) |
| **OBJ/MTL** | ✓ P1 (`tobj`) | ✓ P1 (own) | ✓ | polys, normals, UVs | MTL (Kd/Ks/map_*), PBR ext non-standard | referenced | groups only | ✗ | no units/axis/hierarchy | `tobj` | MIT | none needed |
| **STL** | ✓ P1 (`stl_io`) | ✓ P1 (own/`stl_io`) | ✓ | tris only | ✗ | ✗ | ✗ | ✗ | everything but geometry | `stl_io` | MIT | — |
| **PLY** | ✓ P1 (`ply-rs`) | ✓ P1 (own) | ✓ | verts/faces/colours/custom props | ✗ | ✗ | ✗ | ✗ | — | `ply-rs` | MIT | — |
| **EXR (float heightmap)** | ✓ P1 (`exr`) | ✓ P1 | ✓ | f32 raster | — | — | layers | — | — | engines don't import EXR terrain | `exr` | BSD-3-Clause | `exrinfo` (OpenEXR) |
| **PNG16 / TIFF16/32f** | ✓ P1 (`image`, `tiff`) | ✓ P1 | ✓ | raster | — | — | — | — | — | PNG 16-bit only (no float) | `image`, `png`, `tiff` | MIT/Apache | ImageMagick `identify` |
| **RAW r16/r32 (+ json sidecar)** | ✓ P1 (own) | ✓ P1 | ✓ | raster | — | — | — | — | — | endian/flip ambiguity | none | — | Unity/Unreal import dialogs |
| **Splat / weightmaps** | ✓ P1 (PNG8/RGBA) | ✓ P1 (per-layer PNG8 for Unreal; RGBA packs / per-layer for Unity) | ✓ | raster | — | — | — | — | — | layer semantics engine-specific | `image` | MIT/Apache | — |
| **GeoTIFF** | ✓ P2 (`tiff` + own GeoKeys) | ◐ P3 | ◐ | raster + georef | — | — | — | — | — | no reprojection (P4 `proj`) | `tiff` (+`proj` opt) | MIT | `gdalinfo`, `listgeo` (external) |
| **COG** | ◐ P3 (`cog3pio`) | ✗/P4 | — | — | — | — | — | — | — | — | `cog3pio` | MIT/Apache | `rio cogeo validate` (external) |
| **OpenVDB** | ✗ (defer; `vdb-rs` read-only) | ✗ | ✗ | volumes | — | — | — | — | — | — | — | Apache-2.0 | `vdb_print` |
| **MaterialX (.mtlx)** | ✗ (P4 subset) | ✓ P3 (own XML writer, OpenPBR + gltf_pbr + UsdPreviewSurface nodedefs) | ✗ | — | ✓ | file refs | nodegraphs | — | — | no consumer in Unity | `quick-xml` | MIT | MaterialX `mxvalidate`/`mxtest` (external) |
| **3D Tiles** | ✗ | P4 optional | — | glTF tiles | — | — | tileset.json | — | — | — | own + glTF stack | OGC | `3d-tiles-validator` (npm) |

---

## (b) Recommended format strategy, Phase 1 → 4

**Phase 1 — pure Rust, zero native deps (MVP).**
1. Heightfield I/O: `.r16`/`.r32` (+ JSON sidecar with width/height/bit-depth/min/max/CRS), PNG16 (`image`/`png`), TIFF 16/32f (`tiff`), EXR f32 (`exr`). Flip/endian toggles. Unity (2^n+1, 33–4097) and Unreal (1009/2017/4033/8129, Z-scale) presets with on-screen formulas.
2. Splat/weight maps: per-layer PNG8 (Unreal) and packed RGBA/per-layer (Unity), normalised to sum 1.
3. Mesh export: glTF/GLB (`gltf-json` + own GLB packer) with `KHR_texture_transform`, `KHR_materials_emissive_strength`; OBJ/STL/PLY own writers; USDA text writer (UsdGeomMesh + UsdPreviewSurface, `upAxis`, `metersPerUnit`).
4. Mesh import (reference meshes/props): `gltf`, `tobj`, `stl_io`, `ply-rs`.
5. Validation hooks: shell out to `usdchecker` (macOS) and `gltf-transform validate`/`gltf-validator` if present; never required.

**Phase 2 — richer USD + compression + bridges.**
1. Switch USD to `openusd` 0.8 (usdc/usdz, read, composition); keep the usda writer as fallback. Add `UsdGeomPointInstancer` and glTF `EXT_mesh_gpu_instancing` for scatter.
2. `EXT_meshopt_compression` via `meshopt` crate (feature `meshopt`).
3. FBX read via `ufbx`. **Blender bridge** (separate GPL-3.0-or-later repo: add-on + headless scripts) for FBX write, Blender-native terrain setup, and any format Blender covers better. Detect Blender path; run `blender -b -P`.
4. GeoTIFF read with `tiff` GeoKeys; carry CRS as metadata.
5. Engine helper scripts (separate, permissive): Unity C# editor script (`TerrainData`), Unreal Python/editor script (landscape import), Maya `mayapy` USD import script.

**Phase 3 — optimise & materials.**
1. KTX2/Basis (`KHR_texture_basisu`): feature `ktx2-encode` using `basis-universal` FFI, or external `toktx`/`gltf-transform`; pure-Rust `basisu` for transcoding previews.
2. Draco as optional feature (`draco-core`), behind the same `--compress` switch as meshopt.
3. MaterialX `.mtlx` sidecar writer (OpenPBR + UsdPreviewSurface + gltf_pbr nodedefs) and MaterialX network inside USD.
4. GeoTIFF write; COG read (`cog3pio`).

**Phase 4 — long tail / plugins.**
1. Optional Autodesk FBX SDK feature (user-supplied SDK, not in CI), or `fbxcel` binary static-mesh writer.
2. `proj`-based reprojection (bundled build), COG write, 3D Tiles tileset output for Cesium.
3. OpenVDB only if a non-heightfield terrain mode appears (via Blender/Houdini, not in-process).
4. Unreal 5.8 Mesh Terrain / PCG and Unity Terrain Tools adapters as separate engine plugins.

**Rules of thumb:** glTF is the canonical mesh interchange (web + Unity + Blender + Unreal), USD is the canonical DCC/scene interchange (Blender, Maya, Unreal, Apple), raw heightfields are the canonical *terrain* interchange (engines rebuild Terrain/Landscape objects from them); FBX is a compatibility export only.

---

## (c) Licensing register (recommended dependencies)

| Dependency | Version (2026-10-19) | SPDX | Kind | Linking | MIT-compatible | Notes |
|---|---|---|---|---|---|---|
| `gltf`, `gltf-json` | 1.4.1 | MIT OR Apache-2.0 | Rust | static | ✓ | VERIFIED |
| `meshopt` (+ vendored meshoptimizer) | 0.7.0 (meshoptimizer v1.3) | MIT OR Apache-2.0 (crate); MIT (C++) | Rust+C++ | static | ✓ | VERIFIED; needs C++ toolchain |
| `draco-core`/`draco-gltf` | 2.3.2 / 0.7.0 | Apache-2.0 | Rust | static | ✓ (notice) | VERIFIED; optional |
| `basis-universal` (+ Binomial C++) | 0.3.1 | MIT OR Apache-2.0 (crate); Apache-2.0 (upstream) | Rust+C++ | static | ✓ | VERIFIED; stale, optional |
| `basisu` (pure transcoder) | 0.1.0 | Apache-2.0 | Rust | static | ✓ | VERIFIED; unproven |
| `ktx2` | 0.5.0 | Apache-2.0 | Rust | static | ✓ | VERIFIED; read-only |
| `openusd`, `openusd-schemas` | 0.8.0 | MIT; vendored schemas under TOST-1.0 (Apache-2.0 w/ trademark clause) | Rust | static | ✓ (keep NOTICE) | VERIFIED |
| `zip` | 9.0.5 | MIT | Rust | static | ✓ | VERIFIED |
| `ufbx` (+ ufbx C) | 0.11.5 (C v0.23.1) | MIT OR Unlicense | Rust+C | static | ✓ | VERIFIED |
| `fbxcel` | 0.9.0 | MIT OR Apache-2.0 | Rust | static | ✓ | VERIFIED; optional |
| `tobj` | 4.0.5 | MIT | Rust | static | ✓ | VERIFIED |
| `stl_io` | 0.11.0 | MIT | Rust | static | ✓ | VERIFIED |
| `ply-rs` | 0.1.3 | MIT | Rust | static | ✓ | VERIFIED; unmaintained |
| `exr` | 1.74.2 | BSD-3-Clause | Rust | static | ✓ (notice) | VERIFIED |
| `image` / `png` | 0.25.10 / 0.18.1 | MIT OR Apache-2.0 | Rust | static | ✓ | VERIFIED |
| `tiff` | 0.11.3 | MIT | Rust | static | ✓ | VERIFIED |
| `cog3pio` | 0.1.0 | MIT OR Apache-2.0 | Rust | static | ✓ | VERIFIED; optional |
| `proj` (+ PROJ 9.x) | 0.31.0 (PROJ 9.9.0) | MIT OR Apache-2.0 (crate); MIT (PROJ) | Rust+C | static/dyn | ✓ | VERIFIED; optional, heavy |
| `gdal` (+ libgdal) | 0.19.0 (GDAL 3.13.3) | MIT (crate); MIT-style + mixed permissive (GDAL); transitive GEOS LGPL-2.1 | Rust+C | **dynamic** | ✓ with care | VERIFIED; **not recommended** to bundle |
| `quick-xml` (for .mtlx) | (not pinned) | MIT | Rust | static | ✓ | REPORTED |
| OpenUSD C++ (if ever) | 26.08 | LicenseRef-TOST-1.0 (Apache-2.0 variant) | C++ | dyn | ✓ | VERIFIED license text |
| LightUSD/tinyusdz (if ever) | 1.0 RC | Apache-2.0 | C++ | static | ✓ | VERIFIED README |
| Autodesk FBX SDK | 2020.3.9 | proprietary EULA | C++ | static/dyn | **✗ for source**; binaries OK with notice, opt-in | VERIFIED local License.rtf |
| Blender (subprocess) | 5.2 LTS | GPL-3.0-or-later (binaries) | external | **process boundary** | ✓ (no linking) | VERIFIED |
| Blender add-on (our repo) | — | **GPL-3.0-or-later (required by Extensions Platform)** | Python | — | separate repo | VERIFIED |
| Maya / Maya USD | 2027 / 0.37.0 | proprietary / Apache-2.0 | external | process boundary | ✓ (scripts only) | VERIFIED |
| Unity glTFast / USD packages | 6.20.0 / 1.0.0-pre | Unity Companion License (REPORTED) | external | n/a | user-side | REPORTED |
| Unreal Engine | 5.8 | Epic EULA | external | n/a | user-side | REPORTED |
| three.js / Babylon.js / model-viewer | r186 / 9.31 / 4.4.0 | MIT / Apache-2.0 / Apache-2.0 | JS (webview) | bundled | ✓ | VERIFIED |
| KTX-Software `toktx` (external) | 4.4.2 | Apache-2.0 | C++ CLI | process | ✓ | VERIFIED |
| gltf-validator / @gltf-transform/cli (external) | 2.0.0-dev.3.10 / 4.5.1 | Apache-2.0 / MIT | JS | process | ✓ | VERIFIED |
| Apple USD Tools (`usdchecker`, `usdzip`, …) | 0.25.11 | Apple/TOST (bundled with macOS 27) | external | process | ✓ | VERIFIED presence |
| MaterialX / OpenPBR (spec + nodedefs) | 1.39.5 / 1.1.1 | Apache-2.0 | data | — | ✓ | VERIFIED |
| OpenVDB (if ever) | 13.1.0 | Apache-2.0 (was MPL-2.0 <12) | C++ | — | ✓ | VERIFIED |

---

## Open gaps / things to verify before committing

1. `openusd` 0.8 usdz writer: confirm 64-byte alignment and `usdchecker --arkit` pass on real output.
2. `meshopt` 0.7: confirm filter encoders (`encode_filter_oct/quat/exp`) are exposed; otherwise implement in Rust (small).
3. Unreal Interchange glTF: supported-extension list (JS-rendered docs; test with an actual 5.8 install — none installed locally).
4. Unity RAW "Windows/Mac" byte-order semantics — test one import each way.
5. `basis-universal` 0.3.1 still builds on macOS arm64 with current `cc`/Xcode.
6. `draco-core` bitstream compatibility with three.js/glTFast decoders — round-trip test before enabling.
7. Maya batch licensing for `mayapy` — confirm with Autodesk terms if you ship a Maya script that users run headless.
8. SPDX registration status of the TOST-1.0 license (use `LicenseRef-` until confirmed).
9. `tiff` 0.11 tiled-image *encoding* support (needed for COG write).
10. Whether `rust-usd` still exists (GitHub 404 today).

## Sources (primary, accessed 2026-10-19)
- crates.io API (`/api/v1/crates/<name>`, `/versions`) for every crate above; GitHub REST API for repo license/push dates/releases.
- Khronos: glTF extensions README; KHR_texture_basisu README; three.js `GLTFLoader.js` (dev branch); npm registry for `gltf-validator`, `@gltf-transform/cli`, `three`, `@babylonjs/core`, `@google/model-viewer`.
- Pixar OpenUSD: LICENSE.txt, CHANGELOG.md (26.03/26.05/26.08), openusd.org `spec_usdz.html`, `spec_usdpreviewsurface.html`; mxpv/openusd README; FloatyMonkey/openusd-rs README; lighttransport/tinyusdz README (dev).
- Autodesk: local `/Applications/Autodesk/FBX SDK/2020.3.9/License.rtf`; `lipo -info` on libfbxsdk.dylib; `maya -help`; Maya USD release tag.
- Epic: Landscape Technical Guide; Importing and Exporting Landscape Heightmaps; Creating and Using Custom Heightmaps and Layers; Using Nanite with Landscapes; Universal Scene Description in Unreal Engine; Interchange API index (InterchangeOpenUSD, UInterchangeGLTFTranslator); 80.lv UE 5.8 article.
- Unity: Manual terrain-Heightmaps, terrain-OtherSettings, ScriptReference TerrainData.heightmapResolution / SetAlphamaps; package docs com.unity.cloud.gltfast@6.20 (index, features), com.unity.terrain-tools@5.3, com.unity.usd.core@1.0, com.unity.importer.usd@1.0; usd-unity-sdk README.
- Blender: blender.org releases + license pages; manual Extensions "Getting Started" and "Licenses"; operator introspection in local Blender 5.2.0 LTS.
- ASWF: MaterialX releases; OpenPBR repo, CHANGELOG, spec site; OpenVDB license + release tag; Phoronix/openSUSE on OpenVDB relicensing.
- Apple: WWDC26 session 285 transcript; local `/usr/bin/usd*` tools (`--help`, `--version`).
- georust: geotiff README; docs.rs for `gdal`, `proj`, `tiff` (encoder colortype, DirectoryEncoder, Tag enum), `image` codecs; OSGeo GDAL LICENSE.TXT; PROJ release tag.
- Filyus/draco-rust, reearth/draco-oxide, marcogomez/basisu, AllenDang/ktx2-rw, BVE-Reborn/ktx2, aclysma/basis-universal-rs, gwihlidal/meshopt-rs, ufbx/ufbx, lo48576/fbxcel, Traverse-Research/vdb-rs, johannesvollmer/exrs READMEs.
- CG Channel (2026-07-16) and Autodesk forums on Maya glTF status; Unreal forums on Interchange USD/MaterialX; Unity forums on RAW byte order.

---

## Appendix A — Minimal USDA terrain template (pure-text writer target)

The Phase-1 text writer only needs to emit this shape. It is valid for `usdcat`, Blender `wm.usd_import`, Maya USD, Unreal USD Stage and Unity's importer (UsdPreviewSurface only).

```usda
#usda 1.0
(
    defaultPrim = "Terrain"
    upAxis = "Y"
    metersPerUnit = 1
    doc = "Jenga Dunia export"
)

def Xform "Terrain" (kind = "component")
{
    def Mesh "Tile_0_0"
    {
        int[] faceVertexCounts = [3, 3, ...]
        int[] faceVertexIndices = [0, 1, 2, ...]
        point3f[] points = [(0, 0, 0), (1, 0.2, 0), ...]
        normal3f[] normals = [(0, 1, 0), ...] (interpolation = "vertex")
        texCoord2f[] primvars:st = [(0, 0), ...] (interpolation = "vertex")
        uniform token subdivisionScheme = "none"
        rel material:binding = </Terrain/Materials/Ground>
        float3[] extent = [(0, -10, 0), (1000, 200, 1000)]
    }

    def Scope "Materials"
    {
        def Material "Ground"
        {
            token outputs:surface.connect = </Terrain/Materials/Ground/PBR.outputs:surface>

            def Shader "PBR"
            {
                uniform token info:id = "UsdPreviewSurface"
                color3f inputs:diffuseColor.connect = </Terrain/Materials/Ground/Albedo.outputs:rgb>
                float inputs:roughness = 0.9
                float inputs:metallic = 0
                normal3f inputs:normal.connect = </Terrain/Materials/Ground/Normal.outputs:rgb>
                token outputs:surface
            }
            def Shader "stReader"
            {
                uniform token info:id = "UsdPrimvarReader_float2"
                string inputs:varname = "st"
                float2 outputs:result
            }
            def Shader "Albedo"
            {
                uniform token info:id = "UsdUVTexture"
                asset inputs:file = @textures/ground_albedo.png@
                token inputs:sourceColorSpace = "sRGB"
                token inputs:wrapS = "repeat"
                token inputs:wrapT = "repeat"
                float2 inputs:st.connect = </Terrain/Materials/Ground/stReader.outputs:result>
                float3 outputs:rgb
            }
            def Shader "Normal"
            {
                uniform token info:id = "UsdUVTexture"
                asset inputs:file = @textures/ground_normal.png@
                token inputs:sourceColorSpace = "raw"
                float4 inputs:scale = (2, 2, 2, 1)
                float4 inputs:bias = (-1, -1, -1, 0)
                float2 inputs:st.connect = </Terrain/Materials/Ground/stReader.outputs:result>
                float3 outputs:rgb
            }
        }
    }

    def PointInstancer "Scatter_Rocks"
    {
        rel prototypes = [</Terrain/Prototypes/Rock_A>]
        int[] protoIndices = [0, 0, 0]
        point3f[] positions = [(10, 1.2, 10), ...]
        quath[] orientations = [(1, 0, 0, 0), ...]
        float3[] scales = [(1, 1, 1), ...]
    }

    def Scope "Prototypes"
    {
        def Xform "Rock_A" (
            prepend references = @props/rock_a.usdc@
        ) {}
    }
}
```

Notes: `normals` as a primvar is what Blender/Maya expect; `extent` is required by `usdchecker`; `sourceColorSpace="raw"` for data textures; `quath` (half) orientations are what the spec prescribes for PointInstancer.

## Appendix B — GLB container (what you write around `gltf-json` output)

```
offset  size  field
0       4     magic  = 0x46546C67 ("glTF")
4       4     version = 2
8       4     total length (bytes)
12      4     chunk0 length (JSON, padded to 4 with 0x20 spaces)
16      4     chunk0 type = 0x4E4F534A ("JSON")
20      n     JSON bytes + space padding
..      4     chunk1 length (BIN, padded to 4 with 0x00)
..      4     chunk1 type = 0x004E4942 ("BIN\0")
..      m     binary buffer (buffers[0], uri omitted)
```
Accessor/bufferView `byteOffset` must be a multiple of the component size (4 for float/u32, 2 for u16). Interleave or pad each attribute block to 4 bytes. `gltf-json` serialises with `serde_json`; set `asset.generator = "jenga-dunia x.y"`.

## Appendix C — `EXT_meshopt_compression` bookkeeping (on top of the `meshopt` crate)

Per compressed bufferView:
```json
{
  "buffer": 1, "byteOffset": 0, "byteLength": 123456, "byteStride": 16,
  "extensions": { "EXT_meshopt_compression": {
    "buffer": 0, "byteOffset": 0, "byteLength": 45678,
    "byteStride": 16, "count": 7716,
    "mode": "ATTRIBUTES" | "TRIANGLES" | "INDICES",
    "filter": "NONE" | "OCTAHEDRAL" | "QUATERNION" | "EXPONENTIAL"
  }}
}
```
Rules: the fallback `buffer` (index 1 above) must exist in `buffers[]` with `extensions.EXT_meshopt_compression.fallback = true` and may be omitted from the GLB if the extension is in `extensionsRequired`; vertex buffers go through `encode_vertex_buffer` with `byteStride` = packed vertex size; index buffers through `encode_index_buffer` (TRIANGLES mode, 3-aligned count); quantise positions to i16/u16 with `KHR_mesh_quantization` first — meshopt compresses quantised data far better than f32. Filters OCTAHEDRAL (normals/tangents as snorm8/16) and EXPONENTIAL (positions/uvs) need `meshopt_encodeFilter*` — confirm exposure in `meshopt` 0.7, else port (~100 lines, MIT).

## Appendix D — Heightfield export sidecar (proposed `*.jd-height.json`)

```json
{
  "schema": "jenga-dunia/heightfield/1",
  "width": 4097, "height": 4097,
  "format": "r16le" | "r16be" | "r32f" | "png16" | "exr32f" | "tiff32f",
  "rowOrder": "north-first" | "south-first",
  "minHeightM": -12.5, "maxHeightM": 842.0,
  "cellSizeM": 1.0,
  "origin": { "x": 0, "y": 0 },
  "crs": "EPSG:32637" | null,
  "engines": {
    "unity":  { "terrainHeight": 854.5, "byteOrder": "Windows", "flipVertically": false },
    "unreal": { "zScale": 166.9, "overallResolution": 4033, "resampled": true, "flipY": false }
  },
  "layers": [ { "name": "grass", "file": "weights/grass.png", "bitDepth": 8 }, ... ]
}
```
Unreal `zScale = (maxHeightM − minHeightM) × 100 / 512`; Unity `terrainHeight = maxHeightM − minHeightM` with the RAW normalised so that `minHeightM → 0`. Keep the sidecar even for PNG16 — engines strip all metadata.

## Appendix E — Engine helper scripts (separate, permissive, ship as examples)

- **Unity (C#, Editor):** `TerrainData td = new TerrainData(); td.heightmapResolution = 4097; td.size = new Vector3(w, hRange, l); td.SetHeights(0,0,floats01); td.terrainLayers = layers; td.SetAlphamaps(0,0,weights[y,x,i]); Terrain.CreateTerrainGameObject(td);` — reads the sidecar to fill `size.y`. Avoids the RAW byte-order dialog entirely.
- **Unreal (Python, Editor):** import via `unreal.LandscapeEditorSubsystem` / `LandscapeImportHelper` (API names UNVERIFIED — check 5.8 Python stubs) or use the Landscape Mode import UI with the sidecar's `zScale`. Create `LandscapeLayerInfoObject` per weight layer.
- **Blender add-on (GPL-3.0-or-later repo):** operator that reads the sidecar + heightmap, builds a grid mesh with a Displace modifier (EXR f32 preferred), assigns a node material mirroring the glTF PBR, and sets scene units to metres.
- **Maya (`mayapy`):** `cmds.loadPlugin('mayaUsdPlugin'); cmds.mayaUSDImport(file=..., primPath='/')`; `cmds.currentUnit(linear='cm')`; the stage's `metersPerUnit` handles the scale.

## Appendix F — Proposed Cargo feature plan

| Feature | Default | Pulls in | Phase |
|---|---|---|---|
| `formats-core` | on | `gltf`, `gltf-json`, `image`, `png`, `tiff`, `exr`, `tobj`, `stl_io`, `ply-rs`, `zip` | 1 |
| `usd` | on | `openusd`, `openusd-schemas` | 2 |
| `fbx-read` | on | `ufbx` (C via `cc`) | 2 |
| `meshopt` | on | `meshopt` (C++ via `cc`) | 2 |
| `geotiff` | on | (none extra; `tiff` tags) | 2 |
| `draco` | off | `draco-core`, `draco-gltf` | 3 |
| `ktx2-encode` | off | `basis-universal` (C++) | 3 |
| `materialx` | off | `quick-xml` | 3 |
| `fbx-write-fbxcel` | off | `fbxcel` | 4 |
| `fbx-sdk` | off, not in CI | user-supplied Autodesk SDK via `cxx` | 4 |
| `proj` | off | `proj` with `bundled_proj` | 4 |
| `cog` | off | `cog3pio` | 3/4 |

Everything in the default set is pure Rust or single-file C built by `cc` — no CMake, no system libraries, which keeps Tauri bundles for macOS arm64 / Windows / Linux reproducible.

## Appendix G — Per-format pitfall checklist (pre-release QA)

- [ ] glTF: run `gltf-transform validate` + `gltf-validator`; open in three.js, glTFast (Unity 6), Blender 5.2, Unreal 5.8 Interchange.
- [ ] glTF: UV V-flip check against Blender round-trip; `KHR_texture_transform` tiling renders in all four.
- [ ] USD: `usdchecker` (and `--arkit` for usdz); Blender import with default settings is Y-up/metres correct; Maya USD import scale; Unreal USD Stage shows UsdPreviewSurface.
- [ ] USDZ: zip entries stored, 64-byte aligned; Quick Look opens on macOS/iOS; size ≤ 100 MB.
- [ ] FBX (via Blender bridge): opens in Maya 2027 and Unity/Unreal with correct scale (cm) and Y-up.
- [ ] Heightfields: Unity 4097 RAW imports without spikes (byte order), Unreal 4033 PNG16 imports at the documented Z scale; row order toggles verified against a known asymmetric test terrain.
- [ ] Weightmaps: Unity alphamaps sum to 1; Unreal layer PNGs exactly match Overall Resolution.
- [ ] EXR/TIFF32f: round-trip through Blender's Image Texture (float, non-colour) without 8-bit clamping.
