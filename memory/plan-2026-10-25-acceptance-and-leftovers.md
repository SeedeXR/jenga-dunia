# Plan — acceptance partials and leftovers (owner: "go, use the local loop until all done", 2026-10-25)

Scope: the actionable items of the "what's remaining" list. Every item: unit test and / or spec,
full gates, a commit (Opus 5.5 trailer), push; the acceptance report regenerated at the end
(`JD_UNITY_PROJECT=/tmp/jd-unity node scripts/acceptance.mjs --out /tmp/jd-accept`), hosted CI
checked after the last push.

| # | Item | Status | Verification |
|---|------|--------|--------------|
| 1 | Acceptance 4: the scenario uses an image texture set on a material layer | todo | acceptance item 4 VERIFIED |
| 2 | Acceptance 5: fog and rain as weather (viewport + bakes) | todo | unit / spec; acceptance item 5 VERIFIED |
| 3 | Acceptance 18: a per-project log file | todo | unit; acceptance item 18 VERIFIED |
| 8 | Camera paths: smooth (Catmull-Rom) flythrough + a key list you can edit | todo | vitest + spec |
| 7 | Localisation scaffold (UI strings, a language select) + a local crash reporter | todo | vitest + spec |
| 4 | Contour lines export (GeoJSON iso-height lines) | todo | unit |

Not taken, with the reason:
- 5 distance-based viewport LOD — measured not needed (2048² frames fine); the todo says "only if needed".
- 6 triplanar / stochastic blending — engine-side; we export the texture sets the engines blend.
- 9 path tracer — no WebGPU path tracer in three 0.186 (decision recorded earlier).
- 10 Blender Extensions submission — needs the owner's repository and account.

## Decisions
- (filled in as the loop runs)

## Log
- start: plan written.
