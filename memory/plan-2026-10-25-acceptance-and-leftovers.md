# Plan — acceptance partials and leftovers (owner: "go, use the local loop until all done", 2026-10-25)

Scope: the actionable items of the "what's remaining" list. Every item: unit test and / or spec,
full gates, a commit (Opus 5.5 trailer), push; the acceptance report regenerated at the end
(`JD_UNITY_PROJECT=/tmp/jd-unity node scripts/acceptance.mjs --out /tmp/jd-accept`), hosted CI
checked after the last push.

| # | Item | Status | Verification |
|---|------|--------|--------------|
| 1 | Acceptance 4: the scenario uses an image texture set on a material layer | done `e2ab81a` (VERIFIED) | acceptance item 4 VERIFIED |
| 2 | Acceptance 5: fog and rain as weather (viewport + bakes) | done `e2ab81a` (VERIFIED) | unit / spec; acceptance item 5 VERIFIED |
| 3 | Acceptance 18: a per-project log file | done `42bde4c` (VERIFIED) | unit; acceptance item 18 VERIFIED |
| 8 | Camera paths: smooth (Catmull-Rom) flythrough + a key list you can edit | done `e2ab81a` | vitest + spec |
| 7 | Localisation scaffold (UI strings, a language select) + a local crash reporter | done | vitest + spec |
| 4 | Contour lines export (GeoJSON iso-height lines) | done `e2ab81a` | unit |

Not taken, with the reason:
- 5 distance-based viewport LOD — measured not needed (2048² frames fine); the todo says "only if needed".
- 6 triplanar / stochastic blending — engine-side; we export the texture sets the engines blend.
- 9 path tracer — no WebGPU path tracer in three 0.186 (decision recorded earlier).
- 10 Blender Extensions submission — needs the owner's repository and account.

## Decisions
- Fog is judged by horizon saturation, not the zenith/horizon brightness ratio: fog greys the
  horizon toward the row mean, which can brighten a single horizon pixel (ratio 0.56 → 0.42
  while the haze was correct). Saturation 0.34 → 0.12, zenith unchanged.
- Kiswahili and French strings are first drafts for the owner to correct; unknown strings fall
  back to English.
- A panicking op answers E_INTERNAL with a crash report path (catch_unwind in Registry::invoke);
  UI crashes go through `diag.crash_report`. Reports sit beside logs/ops.log, else the temp dir.

## Log
- start: plan written.
- 3 committed `42bde4c`; 1, 2, 4, 8 + acceptance (full run 18 VERIFIED) committed `e2ab81a`; 7 gated and committed after.
