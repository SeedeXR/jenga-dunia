# Plan — World Creator video gaps (owner: "go, use the local loop until all done", 2026-10-25)

Source: frames of `~/Downloads/EverythingIsRealtime01.mp4` (todo item "World Creator video gaps").
Every item ships through `scripts/pre-push.sh` with its own test (unit and/or e2e), a commit with the
Opus 5.5 trailer, and a push; hosted CI checked after the last push. Engines: Unreal, Unity, Blender
only. Engine importer scripts are not gated — run their headless checks if touched.

| # | Item | Status | Verification |
|---|------|--------|--------------|
| 10 | Esc aborts a running generation | done | e2e: Esc during an evaluation logs E_CANCELLED |
| 3 | Whole-terrain actions: Flip X / Flip Y, Rotate left / right 90°, Flatten | done | unit (node), e2e (buttons) |
| 9 | World-border box with side walls + white shoreline contour | todo | e2e header flag + capture viewed |
| 6 | Per-octave strength on the base noise + a general strength | todo | unit: octave weights change the field as specified |
| 2 | Grid transforms on a selection: Duplicate, Rotate, Scale, Tilt, Push / Pull, Align | todo | unit (kernel), e2e |
| 1 | Brush transforms: Rotate, Scale, Tilt, Push / Pull, Align | todo | unit (kernel), e2e |
| 4 | Alpha brushes: a palette of brush shapes beyond round / square | todo | unit, e2e |
| 5 | Sculpt-layer settings: height offset / scale / blending, steps, placeable area + Fit to terrain | todo | unit, e2e |
| 7 | Layer tree with search box and per-row eye in the right panel | todo | e2e |
| 8 | Vertical icon strip for adding layer types | todo | e2e |

## Decisions
- (filled in as the loop runs)

## Log
- start: plan written.
- 10 + 3 done: global Esc cancels running jobs outside text fields (jobs.spec, two E_CANCELLED); `jd.filter.action@1` (flip_x / flip_y / rotate_left / rotate_right on square worlds / flatten) + desktop Actions row splicing it before the output (actions.spec).
