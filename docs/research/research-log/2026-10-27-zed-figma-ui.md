# 2026-10-27 — A clean UI after Zed and Figma (owner directive)

Owner, 2026-10-27: "make sure at the end we have a clean well designed ui like zed is or figma is".
The two UIs (Studio, egui; the Editor, Tauri) keep matching each other; both change together.

## Sources

- Figma, "Inside the redesigned Figma" (https://www.figma.com/blog/behind-our-redesign-ui3/) and
  "Our approach to designing UI3" (https://www.figma.com/blog/our-approach-to-designing-ui3/).
- Zed, "Zed's new panel system" (https://zed.dev/blog/new-panel-system) and the visual
  customization docs (https://zed.dev/docs/visual-customization).

## Findings

- VERIFIED (Figma UI3 blog): floating navigation and properties panels were tried and reversed;
  they cramped the canvas on smaller screens and "slowed people down". Panels are fixed and
  resizable; only toolbars float. Minimize UI collapses both side panels.
- VERIFIED (Figma UI3 blog): visible labels kept (a minimal, label-less panel hurt accessibility);
  "backgrounds on inputs, borders around dropdowns, rounded corners"; a checkbox beats a dropdown
  for a two-state control; hidden state behind icon buttons slowed people down.
- VERIFIED (Figma UI3 blog): one slim toolbar floats at the bottom of the canvas.
- VERIFIED (Zed blog): three docks (left, right, bottom), each toggled; panels share a dock and
  only one shows at a time.
- VERIFIED (Zed docs): the status bar carries the dock panels' toggle icons (project, outline, git,
  diagnostics, terminal), each individually hideable.
- INFERRED (from the two): neither puts chrome in floating cards with drop shadows, gutters or
  always-open logs; surfaces are flush, separated by 1 px lines, with one accent colour.
- INFERRED: a neutral grey surround keeps the terrain's colours true (Lightroom, Blender and
  Unreal frame the image in neutral grey); today's navy tint was taken from World Creator 2025.

## Spec (both UIs) — plan P6

1. Panels — owner, 2026-10-27: the floating icons "or panels", don't touch them. The floating
   panel cards stay (left, right, rounded, over the backdrop) with their resize and collapse;
   Figma's docking finding is set aside. Polish only: even gutters, one corner radius, a quieter
   shadow, responsive widths (a narrow window shrinks or collapses a panel instead of squeezing
   the viewport).
2. Status bar (24 px, bottom): left the last log line (error / warning coloured), a click toggles
   the log; right the job progress, viewport readout and the dock toggles (left, graph, right,
   log). The log becomes the bottom dock, closed by default.
3. Title bar (36 px): brand mark, project name (`fg-0`) and world size (`fg-2`), the toolbar; view
   pickers (layer, colour) at its end.
4. Sections: full width, separated by 1 px lines; 12 px semibold heading with a chevron icon;
   12 px padding; label rows 28 px, labels `fg-2`.
5. Controls: filled inputs (`bg-2`, no border, radius 4, focus ring accent), buttons radius 4
   without borders, one filled primary per group.
6. Palette: neutral greys, one blue accent; tokens keep their names (`--color-jd-*`, theme.rs).
7. Viewport chrome — owner, 2026-10-27: "the floating icons don't touch them but ensure there are
   well designed and interactive and responsive". The floating buttons (top left: fit / still /
   fullscreen / frame; top right: Grid / Horizon / Road / Sculpt / Hide graph; the layer rail)
   keep their places and set. Polish only: one size and corner radius, clear hover / pressed /
   active (armed tool) / disabled states, a tooltip with the shortcut on each, a keyboard focus
   ring; responsive — on a narrow viewport the labelled chips drop to icons and the rail scrolls
   instead of overlapping the compass or each other.

Order: after P5 retires the Editor's own viewport and panels (owner, 2026-10-27), so the pass
covers Studio (viewport and panels) and the Editor's graph window. Slices: S1 = 1–6 in both; S2 = 7;
S3 = the Editor's graph window (nodes, inspector); S4 = docs and the guide figures recaptured.
