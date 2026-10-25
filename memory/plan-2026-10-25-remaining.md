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
| 7 | Blend-op readout in the layer stack | in gate | e2e `stack.spec` asserts the row text |
| 8 | Workflow conveniences: hover thumbnails, expressions in fields | todo | e2e for both |
| 9 | Rendering: HDRI import, post effects, flythrough + stills, path tracer (scoped) | todo | e2e per feature; path tracer: feasibility decided in-loop |
| 10 | Tile-level cache blobs + SQLite index | todo | unit test: a memo survives a restart |
| 11 | Diff / archive paging for deeper undo | todo | unit test: undo past the in-memory window |
| 12 | DA3 / metric-depth refinements in jd-vision | todo | the documented 10× scale error measured before / after |
| 13 | Acceptance report regeneration | todo | `scripts/acceptance.mjs --out /tmp/jd-accept` report attached to the handover |

## Order
6 → 7 → 13 → 10 → 11 → 8 → 9 → 12 → 4 (4 last: it needs screen-recording permission the OS may
ask for interactively; if a dialog blocks, report it rather than stall).

## Decisions
- (filled in as the loop runs)

## Log
- 11:05 plan written; starting 6.
- 12:50 6 done: `terrain.export_splines` (D8-traced river reaches + spline nodes → GeoJSON, local_m / world / wgs84), Blender `jd.import_splines`; 7 drafted, stack.spec green alone.
