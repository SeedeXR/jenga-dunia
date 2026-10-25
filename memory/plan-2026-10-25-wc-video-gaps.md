# Plan — World Creator video gaps (owner: "go, use the local loop until all done", 2026-10-25)

Source: frames of `~/Downloads/EverythingIsRealtime01.mp4` (todo item "World Creator video gaps").
Every item ships through `scripts/pre-push.sh` with its own test (unit and/or e2e), a commit with the
Opus 5.5 trailer, and a push; hosted CI checked after the last push. Engines: Unreal, Unity, Blender
only. Engine importer scripts are not gated — run their headless checks if touched.

| # | Item | Status | Verification |
|---|------|--------|--------------|
| 10 | Esc aborts a running generation | done | e2e: Esc during an evaluation logs E_CANCELLED |
| 3 | Whole-terrain actions: Flip X / Flip Y, Rotate left / right 90°, Flatten | done | unit (node), e2e (buttons) |
| 9 | World-border box with side walls + white shoreline contour | done | e2e header flag + capture viewed |
| 6 | Per-octave strength on the base noise + a general strength | done | unit: octave weights change the field as specified |
| 2 | Grid transforms on a selection: Duplicate, Rotate, Scale, Tilt, Push / Pull, Align | done | unit (kernel), e2e |
| 1 | Brush transforms: Rotate, Scale, Tilt, Push / Pull, Align | done | unit (kernel), e2e |
| 11 | Premium look like World Creator (owner, mid-loop): palette, panel cards, value-bar sliders, section headings, primary buttons, icon buttons | done | captures viewed before/after; all specs green |
| 4 | Alpha brushes: a palette of brush shapes beyond round / square | todo | unit, e2e |
| 5 | Sculpt-layer settings: height offset / scale / blending, steps, placeable area + Fit to terrain | todo | unit, e2e |
| 7 | Layer tree with search box and per-row eye in the right panel | todo | e2e |
| 8 | Vertical icon strip for adding layer types | todo | e2e |

## Decisions
- 2/1: one kernel (`jd_terrain::transform`) and one node (`jd.sculpt.transforms@1`, ops appended like strokes) serve both the grid and the brush. Grid Push / Pull = the existing Raise / Lower; brush Push / Pull = a grab (the terrain slides with the drag). The lookup walks the whole chain feeding the output, so a strokes node spliced later does not make a second transforms node (the e2e caught that).

## Log
- start: plan written.
- 10 + 3 done: global Esc cancels running jobs outside text fields (jobs.spec, two E_CANCELLED); `jd.filter.action@1` (flip_x / flip_y / rotate_left / rotate_right on square worlds / flatten) + desktop Actions row splicing it before the output (actions.spec).
- 9 + 6 done: framed block (walls to below the lowest ground, teal rim) + marching-squares shoreline, Frame toggle, header `box` / `shore N` (vitest + chrome.spec, capture viewed); `octave_weights` + `strength` on the noise (weighted FBm reproduces the plain fractal at weights 1; the unit test caught `mid` using the scaled half), Strength slider + octave bars (octaves.spec).
- 2 + 1 done: transform kernel (rotate / scale relief / tilt / push-grab / align to best-fit plane / duplicate, round or square footprint, soft edge; 3 unit tests), Grid ▸ Transform selection, brush modes rotate / scale / tilt / push / align (transforms.spec).
- 11 done: navy-charcoal tokens + vivid blue, floating `.jd-panel` cards (panels, viewport, log), `.jd-section` headings with a chevron, `.btn-primary` (New, Generate, Evaluate), accent slider fills, thin scrollbars, rounded graph nodes with an accent stripe; before/after captures compared; full suite green.
