# Plan — remaining items (owner decisions 2026-10-25 ~11:00)

Owner: "ignore and strikethrough 1 2 3, work on 4, ignore 5, work on 6, work on 7, work on 8,
work on 9, cover 10, work on 11, work on 12, work on 13, invoke the local loop and start working
on them until you have completed all of them and verified." Engine scope: Unreal, Unity, Blender
only. Every item ships through the usual gate (`scripts/pre-push.sh`) with its own test, then a
commit with the Fable 5.1 trailer and a push; CI verified after the last push.

| # | Item | Status | Verification |
|---|------|--------|--------------|
| ~~1~~ | ~~CleanMyMac exclusions~~ | struck by owner | — |
| ~~2~~ | ~~Signed macOS bundle~~ | struck by owner | — |
| ~~3~~ | ~~Windows run check~~ | struck by owner | — |
| 4 | Native captures: Edit-menu capture, native sculpt recording | todo | the PNG / MP4 under `docs/user/images` + `videos/`, viewed |
| ~~5~~ | ~~jd-vision model licences~~ | ignored by owner | — |
| 6 | Spline I/O: river network + `jd.terrain.spline@1` points → GeoJSON (+ Blender curves) | done (gates green; Blender 5.2 BRIDGE OK) | unit test on the writer; Blender bridge test imports the file |
| 7 | Blend-op readout in the layer stack | done (gates green) | e2e `stack.spec` asserts the row text |
| 8 | Workflow conveniences: hover thumbnails, expressions in fields | todo | e2e for both |
| 9 | Rendering: HDRI import, post effects, flythrough + stills, path tracer (scoped) | todo | e2e per feature; path tracer: feasibility decided in-loop |
| 10 | Node-output cache tier on disk (no SQLite: see Decisions) | applied, awaiting gate | unit test: a memo survives a restart |
| 11 | History paging to `<project>/history/` | applied, awaiting gate | unit test: undo past the in-memory window |
| 12 | DA3 / metric-depth refinements in jd-vision | todo | the documented 10× scale error measured before / after |
| 13 | Acceptance report regeneration | done: 15 VERIFIED / 3 PARTIAL (was 14 / 4) | `scripts/acceptance.mjs --out /tmp/jd-accept` report attached to the handover |

## Order
6 → 7 → 13 → 10 → 11 → 8 → 9 → 12 → 4 (4 last: it needs screen-recording permission the OS may
ask for interactively; if a dialog blocks, report it rather than stall).

## Decisions
- 10: no SQLite. Nothing in the workspace depends on rusqlite and the per-key index is a few hundred entries; the node tier reuses `jd-cache`'s content-addressed `.jdl` blobs, `index.json` and LRU GC under `cache/layers/` with a `node-ports.json` beside them. If a project ever holds tens of thousands of node results, an SQLite index is the upgrade (one crate, same adapter).
- 11: snapshots are paged whole (JSON per file), not as diffs — a 2 MiB stroke-heavy snapshot is the worst case seen and a session's archive is cleared on open; diffs would only matter for a persistent cross-session history, which nobody asked for.

## Log
- 11:05 plan written; starting 6.
- 12:50 6 done: `terrain.export_splines` (D8-traced river reaches + spline nodes → GeoJSON, local_m / world / wgs84), Blender `jd.import_splines`; 7 drafted, stack.spec green alone.
- 13:05 7 done (lerp/min/max and shape rows read op · opacity · b-layer); 13 started: `JD_UNITY_PROJECT=/tmp/jd-unity node scripts/acceptance.mjs --out /tmp/jd-accept`.
- 13:35 13: first run 13 VERIFIED / 3 PARTIAL / 2 FAILED — both FAILED are the Unity headless import: `6018a16`'s importer set the read-only `TerrainData.holesResolution` (compile error; the gates never run Unity). Fixed (the property follows heightmapResolution − 1); rule added to the push-gating memory; Unity check rerunning, then the report.
- 13:55 13 done: report regenerated — 15 VERIFIED / 3 PARTIAL, Unity 6000.4.8f1 headless import verified again; 10 and 11 applied (unit tests green, jd-core 43 passed); gates running.
