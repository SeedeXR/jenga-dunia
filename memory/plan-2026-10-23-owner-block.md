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
| 0 | Stale markers (todo lines 68/69, handover 6(c)(d)) | done (f54fe87) | process slip: pushed before reading the gates line; gates had failed on a CleanMyMac node_modules wipe; reinstalled, re-run green |
| 1 | jd-geo HTTP-fetch policy: ADR + sources/attribution + cache + bandwidth-adaptive client + `geo.fetch_dem` (and OSM/contours as the sources allow) | done (ADR-0014; verified CLI/MCP/UI/Unity/Unreal/Blender) | contours deferred (nothing asks); OpenTopography wired but untested (no key); Unreal Interchange reports 1 026 tris for the 524 k GLB — bounds match, investigate |
| 2 | DA3 intrinsics: fov from `Heliosoph/da3-base-onnx` (Apache-2.0, `intrinsics` [1,1,3,3] at the 504² grid; the metric model has no camera head) feeding the metric model | next | second session, `camera_model` param; depth × mean(fx,fy)/300 |
| 3 | Texture tiling / detail maps: tileable detail albedo+normal per material, tiling scale; GLB (KHR_texture_transform?), Unity TerrainLayer tiling, Unreal, Blender, viewport | scouting | |
| 4 | Viewport: high-detail option beyond 1024² (setting + memory guard); panel docking | scouting | |
| 5 | In-process C++ FBX SDK adapter (feature-gated `jd-fbxsdk` crate, cc shim, macOS; skip on CI) + round-trip verification vs our writer/reader | planned | conflicts with ADR-0009 (out-of-process): owner asked for in-process → ADR-0015 supersedes the process boundary for the test oracle; licence notice 1.1.5; SDK libs universal arm64 at /Applications/Autodesk/FBX SDK/2020.3.9 |

## Decisions
- 2026-10-23: fetch client = ureq 3 (rustls), no tokio in domain crates; providers Terrarium (default), Copernicus (AWS COGs), OpenTopography (BYOK); Overpass for OSM; cache under `cache/geo`; attribution automatic. ADR-0014.
- 2026-10-23: FBX SDK adapter goes in-process per the owner, as a test oracle only, off by default (ADR-0015 to write).
- 2026-10-23: DA3 fov from the da3-base export's intrinsics (the metric export has no camera head).
