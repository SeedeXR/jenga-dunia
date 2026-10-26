# Plan — cave follow-ups and gate hardening (owner: "go, use the local loop until all done", 2026-10-26)

Scope: every actionable item of the "what's remaining" list. Each item: unit test and / or spec,
full gates, a commit (Opus 5.5 trailer), push; hosted CI checked after the last push.

| # | Item | Status | Verification |
|---|------|--------|--------------|
| 1 | E2E wait hardening: the 5 s waits on viewport / log text that flake at load 20–35 get explicit, sized timeouts | done (expect ceiling 15 s; two full runs 38/38 at load 23–25) | full suite twice at local load without a timeout flake |
| 2 | Speleothem density from cover: an optional `cover` input (0..1 soil / vegetation) scales drip sites per site; bare rock ≈ none (Kaufmann 2003) | done | unit: same cave, cover 0 vs 1 |
| 3 | Maze caves: a `maze` kind — dense multi-storey fissure grid on two fracture sets (gypsum / flood mazes), passage density near Klimchouk's 198 km/km² | todo | unit: density, storeys, connectivity |
| 4 | Cenotes (collapse shafts into flooded passages, karst) and a `flank_margin` kind (oval, wide, low chambers just inland of the coast at sea level + 2–7 m, interconnected, blind pockets) | todo | unit per feature |
| 5 | Lava tubes: stacked upper levels and braids (side tubes that split and rejoin within ~140 m) | todo | unit |
| 6 | Sea caves: blowholes (a shaft from the inland end to the surface) and raised caves at old sea levels | todo | unit |
| 7 | Network check: karst skeleton statistics (average vertex degree 1.8–2.6, orientation entropy 0.8–1.0, branch length 8–331 m — Paris 2021 Table 2) as a test | done (needed side passages: branch length was 351 m) | unit over several seeds |
| 8 | Acceptance item 19: caves (karst node, holes open the terrain, mesh + JSON export, Unity profile placement when a Unity project is set) | todo | acceptance run VERIFIED |
| 9 | Cave walls: project surface-nets vertices onto the isosurface (Newton steps along the gradient) so man-made edges lose the staircase ridges | done (root cause was a zero seam in the arch SDF) | unit: vertex distance to the surface; render viewed |

Out of scope (owner / hardware / decided): translations review, Blender Extensions submission,
Windows / Linux runs, real-photo depth check, distance LOD, triplanar / stochastic, path tracer.
Known limit kept: on a coarse terrain grid a mouth is at least one cell wide.

## Decisions
- E2E: one global `expect.timeout` of 15 s instead of per-call timeouts; assertions still return on success.
- The "ridges" on drifts were not voxel aliasing: the arch profile cut its roof circle with a half-plane,
  leaving the distance exactly 0 along the springing line inside the air; a voxel row on it grew a
  false sheet. Fixed by an uncut circle (or ellipse) roof; Newton projection onto the surface added.
- Network statistics failed honestly (branch length 351 m > 331 m): the generator now adds blind side
  passages every 80–160 m; five seeds give k 1.99–2.00, l 110–139 m, H 0.94–0.99.
- Cover thinning keeps a speleothem with probability = cover at its (u, v); stalactite and stalagmite
  share one draw (same site).

## Log
- start: plan written.
