# ADR-0015 — The Autodesk FBX SDK in process, as a verification oracle

**Status:** Accepted · 2026-10-23 (owner decision: "in-process C++ FBX SDK adapter") · supersedes the FBX part of ADR-0009's process boundary

## Context

ADR-0009 keeps proprietary SDKs out of the MIT binary by running adapters out of process. Since then the product grew its **own** FBX writer and reader (`jd-terrain::fbx`, binary 7.4/7.5, zlib arrays, verified through Blender, Unity, Maya's FBX plugin, Unreal's Interchange and Optima). What was missing was an exact, array-level check against the reference implementation: the existing engine checks count meshes and triangles but never compare positions, triangulation, units or axes with what the Autodesk SDK itself reads, and no fixture in the repository was ever written by the SDK (its default compressed arrays exercise our reader's zlib path only by proxy). The owner asked for the adapter in process.

## Decision

`crates/jd-fbxsdk` links the Autodesk FBX SDK **found on the developer's machine** (`FBXSDK_DIR`, or the default install) through a small C shim (`cpp/shim.cpp`, exceptions caught at the boundary, caller-freed buffers) built by `cc` at compile time. It exposes `read` (the first mesh, triangulated by the SDK, with units, axis, version, material and texture) and `write` (a triangle mesh as a binary 7400 or 7500 file with the SDK's compressed arrays), and `available()`. Without the SDK the crate builds to a stub whose calls return `E_DEPENDENCY_MISSING`, so CI and every other machine compile unchanged. It is the one crate that allows `unsafe` (its own `[lints]`); the workspace keeps `unsafe_code = "forbid"`.

Its role is a **test oracle and a diagnostic**: `jd-terrain`'s tests read our exports through the SDK and compare positions, triangle count, units and axis exactly, and read SDK-written 7500 files through our reader; the `fbx.sdk_check` op reports the SDK's view of any FBX for the CLI, the MCP server and the desktop. It is **not** an export path: `terrain.export_mesh` keeps writing FBX with our own writer (ADR-0012), and the SDK's `write` exists to make fixtures.

## Licence and distribution

The SDK licence (`License.rtf`, 2020.3.9) grants linking into "Developed Software" (1.1.1) and distribution of the developed software with the library "as is" (1.1.3), requires the notice "This software contains Autodesk® FBX® code developed by Autodesk, Inc. Copyright 2019 Autodesk, Inc. All rights, reserved. Such code is provided 'as is' and Autodesk, Inc. disclaims any and all warranties, whether express or implied…" in the EULA or About box and the documentation (1.1.5), and forbids redistributing the SDK's headers, libraries and API information (2.1.1(e), §3). Consequences: the SDK is never vendored, copied into the repository, the cache or an artefact; a binary built on a machine with the SDK contains Autodesk code and must carry the 1.1.5 notice (`docs/user/cli-quickstart.md`, the desktop About) — the release pipeline builds without the SDK, so shipped binaries are the MIT stub; the repository stays MIT.

## Alternatives rejected

- A CLI helper binary called with `Command` (the ADR-0009 shape): the same oracle at arm's length, but the owner asked for in process, and the array-level comparison is simpler without a protocol.
- A feature flag (`--features fbxsdk`): the crate would never be compiled by the gates and would rot; build-time detection keeps the stub compiled everywhere and the real shim compiled wherever the SDK is.
- Vendoring the SDK for CI: forbidden by its licence.

## Consequences

- Tests that need the SDK skip with "no FBX SDK: skipping" where it is absent (CI), like the usdzip tests.
- macOS only today (the shim's link line); Windows and Linux need the equivalent `lib/vs20xx` or `lib/gcc` lines in `build.rs`.
- ADR-0009 stands for Blender, OpenVDB and future node packs; only the FBX verification adapter moved in process.

## Evidence

Standalone link and run on 2026-10-23: `fixtures/tiny-grid.fbx` read by SDK 2020.3.9 as version 7400, 25 control points, 16 quads → 32 triangles, normals and UVs present, unit scale 100 (metres), Y up; the SDK's own 7500 export of the same mesh read back. `crates/jd-terrain/src/fbx.rs` oracle tests; `docs/research/formats-sdks.md` (licence clauses, cited by section here).
