# ADR-0009 — Out-of-process plugins for licence and crash isolation

Status: Accepted · 2026-10-19

## Context
Brief §16.2 requires that proprietary or licence-incompatible components (Autodesk FBX SDK, GPL Blender add-ons, MPL OpenVDB) are never silently bundled, and §15 of the plugin architecture asks for permissions. The core is MIT.

## Decision
Phase 4 plugins run **out of process** (JSON-RPC over stdio), declared by a manifest with explicit permission scopes and capability declarations (`09-plugins.md`). The FBX SDK exporter is a separately built adapter the user installs; the Blender bridge is a separate GPL-compatible add-on that talks the bridge manifest; OpenVDB (if ever) is a separate binary. In-process `cdylib` plugins are deferred until a measured need.

## Alternatives rejected
- Dynamic linking of proprietary SDKs into the MIT binary — licensing and distribution risk; platform restrictions.
- In-process Rust plugins now — requires ABI stability work (`abi_stable`/C ABI) before any plugin exists.
- WASM plugins — attractive sandbox, but no GPU/large-buffer story yet for terrain kernels; revisit for node packs.

## Consequences
- Serialisation overhead for adapter plugins (acceptable: export-time operations).
- Capability declarations feed the interoperability matrix automatically.
- Plugin manager must verify binaries by hash and enforce scopes.

## Evidence
Brief §9.2, §16.2; `docs/research/opensource-rea.md` §5 (Blender plugin GPL precedent in ProceduralTerrains).
