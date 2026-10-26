# 2026-10-26 — a Lightroom-style slider for both UIs

Owner: "the slides should be sleek and cool like the ones in light room" (Option B, ADR-0017: the
Tauri Editor and the egui Studio must look the same, so one slider design for both).

Lightroom's controls are built on Adobe's Spectrum design system. What Spectrum specifies:

| Part | Medium size | Source |
|---|---|---|
| Track | 2 px thick (`--spectrum-slider-track-thickness: 2px`), corner radius 2 px | VERIFIED, `@spectrum-css/tokens`, `@spectrum-css/slider` (unpkg, 2026-10-26) |
| Fill | the same 2 px (`--spectrum-slider-track-fill-thickness: var(--spectrum-slider-track-thickness)`), from `fill-start` to the value | VERIFIED, same; Spectrum Web Components: filled "from a starting point to the current value" |
| Handle | 16 px (`--spectrum-slider-handle-size-medium: 16px`), a ring: background transparent, border 2 px (`--spectrum-border-width-200`) | VERIFIED, same |
| Handle pressed | the ring thickens (`--spectrum-slider-handle-border-width-down-medium`: 6–8 px across the two scales) | VERIFIED, same |
| Track gap at the handle | 4 px (`--spectrum-slider-handle-gap`) | VERIFIED, same |
| Label | on top (default) or side; the value label at the end of the label row; "sliders should always have a label" | VERIFIED, spectrum.adobe.com/page/slider (search summary), Spectrum Design Data `labelPosition: top` |
| Reset | double-click the handle, or Escape while focused, returns to `default-value` | VERIFIED, opensource.adobe.com/spectrum-web-components/components/slider |
| Keys | arrow keys step | VERIFIED, same |
| Editable value | a number field paired with the slider | VERIFIED, same |
| Lightroom Classic | double-click a slider's **name** resets it; double-clicking a section name resets the group; Alt changes the group label to Reset; `.` moves between sliders, `+`/`−` step | REPORTED, jkost.com, lightroomkillertips.com, petapixel.com |

## Our slider (both UIs)

Label left and value right on one row (value double-click → type a number; unit shown); a 2 px
track below in `bg-3` with the fill in `fg-1` from `fill_start` (the default for bipolar ranges,
which also get a 1 px centre tick); a 14 px ring handle (2 px `fg-0` border, transparent centre,
4 px track gap) that thickens to 5 px while dragged; relative drag (no jump on press), Shift for
10× finer steps; double-click the label or the handle, or Escape, resets to the default; arrow keys
step; the scroll wheel does not change values (panels scroll without accidents); an optional
gradient track (sun elevation, temperature). Accessible as a slider with name, value, min, max.
Sizes are scaled from Spectrum's medium to our 13 px UI (Spectrum's body is 14 px).
