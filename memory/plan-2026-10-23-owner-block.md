# Plan — owner block of 2026-10-23 (loop)

Owner (2026-10-23): "clean up the stale markers and commit; work on: texture tiling and detail
maps; in-process C++ FBX SDK adapter; DA3 intrinsics head; panel docking; a high-detail viewport
option beyond 1024²; an HTTP-fetch policy for jd-geo (Copernicus, OpenTopography, OSM;
attribution; offline cache) that is robust and adapts to internet speed. Use the loop, work till
everything is completed, zero hallucination, everything verified from CLI to MCP to app UI and
against Unreal, Unity, Blender."

Rules for every stage: ground in the code first (scout reports under the task outputs), implement
the smallest thing that works, test at every layer the feature touches (unit → CLI → MCP → UI e2e →
engine/Blender headless), annotated capture or real output in docs/user, gates (`scripts/pre-push.sh`
in one command), commit, push, CI watch. Record decisions here as they are made.

| # | Stage | Status | Notes |
|---|-------|--------|-------|
| 0 | Stale markers (todo lines 68/69, handover 6(c)(d)) | done, committing | |
| 1 | jd-geo HTTP-fetch policy: ADR + sources/attribution + cache + bandwidth-adaptive client + `geo.fetch_dem` (and OSM/contours as the sources allow) | scouting | needs: which crate (rustls only), licence/attribution text from geo-vision.md, cache dir, adaptive concurrency/chunking, offline mode, resume |
| 2 | DA3 intrinsics: fov from an intrinsics-bearing export (or ported ray→intrinsics) feeding the metric model | scouting | research agent on HF exports + DA3 source |
| 3 | Texture tiling / detail maps: tileable detail albedo+normal per material, tiling scale; GLB (KHR_texture_transform?), Unity TerrainLayer tiling, Unreal, Blender, viewport | scouting | |
| 4 | Viewport: high-detail option beyond 1024² (setting + memory guard); panel docking | scouting | |
| 5 | In-process C++ FBX SDK adapter (feature-gated, cc shim, macOS; skip on CI) + round-trip verification vs our writer/reader | scouting | SDK at /Applications/Autodesk/FBX SDK/2020.3.9 |

## Decisions
- (none yet)
