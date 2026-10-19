# Desktop app (Phase 2, early)

Status: first working build. One window, native macOS title bar, dark theme (Host Grotesk, Lucide icons). The layout follows World Creator's left-to-right workflow — project on the left, viewport in the centre, terrain/noise controls and export on the right, operations log at the bottom — and will grow into the full workspace of `docs/architecture/12-ui-workspace.md` (node editor, sculpting, materials, environment, jobs).

![Jenga Dunia desktop app, empty project state](images/desktop-empty.png)

*Real screenshot of the running app on macOS (2026-10-19). The header reports the viewport backend: here **WebGPU** inside the system WebView.*

![Jenga Dunia with a generated 2048² terrain in the viewport](images/desktop-generated.png)

*After **Terrain → Generate Terrain** (⌘G) and **File → Export Heightmap…** (⌘E), driven through the native menu bar: 2048² OpenSimplex2 fBm, 2.09 M viewport triangles; `exports/height.exr` validated by macOS ImageIO as 2048×2048, 32 bits/sample. Release build on Apple Silicon: generation 58–63 ms, 2 MB preview to the viewport in 6 ms (the screenshot above was taken from a debug build, hence its slower readout).*

![Native macOS menu bar](images/desktop-menubar.png)

## Native integration

- Menu bar: **Jenga Dunia** (About, Services, Hide, Quit) · **File** (New ⌘N, Open ⌘O, Save ⌘S, Export Heightmap… ⌘E, Close Window ⌘W) · **Edit** (Undo/Redo/Cut/Copy/Paste/Select All) · **Terrain** (Generate ⌘G) · **Window** (Minimize, Zoom, Full Screen). Menu actions run the same operations as the buttons.
- Launch with a project: `jd-app ~/Worlds/Hills.jdproj` (or double-click a `.jdproj` once the bundle's file association is registered) opens it before the window appears.
- Dark window theme, native title bar, Host Grotesk typography.

## Run from source

```sh
cargo build -p jd-mcp            # not needed by the app; used by the UI end-to-end tests
cd apps/desktop && pnpm install && pnpm tauri dev
```

## Tutorial: first terrain in four steps

![Step-by-step: create a project, generate, export, read the log](images/tutorial-01-create-project.png)

1. **Path** — type the project directory (it is created on disk as a `.jdproj` folder).
2. **New (⌘N)** — or **Open (⌘O)** for an existing project; the header shows `name — resolution² · extent m`.
3. **Generate (⌘G)** — after choosing lattice, fractal, seed, feature size, octaves, lacunarity, gain and height range.
4. **Export heightmap (⌘E)** — `.exr` (exact), `.png`/`.r16` (16-bit, warns about quantisation), `.r32` (exact).
5. **Log** — every operation with warnings and errors (stable code, why, what to do next).

![After generating: viewport readout, layer stats, export](images/tutorial-02-generated.png)

1. Viewport readout: backend (WebGPU here), triangle count, preview size and transfer time.
2. Layer statistics: min/max/mean height, generation time and the content hash — the same seed and parameters always give the same hash.
3. Export writes `exports/<file>` plus a `.jd-height.json` sidecar (size, range, flip, byte order).

*Annotations are generated from the real screenshots by `apps/desktop/scripts/annotate.mjs` from the `.annot.json` specs next to the images, so they can be regenerated whenever the UI changes.*

## Workflow

1. **Project** — type a directory path (e.g. `~/Worlds/Hills.jdproj`), set extent (m) and resolution (px), click **New** (or **Open** for an existing one). The header shows `name — resolution² · extent m`; the panel shows the project id, dirty flag and pending journal entries.
2. **Terrain · Noise** — choose lattice and fractal, seed, feature size, octaves, lacunarity, gain, height range; click **Generate**. Stats (min/max/mean, time, content hash) appear under the button and the viewport shows the terrain. Drag to orbit, wheel to zoom.
3. **Export** — type a file name (`height.exr`, `h.png`, `h.r16`, `h.r32`) and click **Export heightmap**; the file lands in the project's `exports/` with a `.jd-height.json` sidecar. Quantised formats log a `W_EXPORT_LOSS` warning.
4. **Save** — writes `project.json`/`graph.json`/`scene.json` atomically and clears the journal.

Every button calls the same operation the CLI and MCP server expose (`jd schema --list`), so anything you do here can be scripted.

## How the UI is tested

| Level | What | Where |
|---|---|---|
| Component (jsdom) | the real `App` with `invoke` mocked at the Tauri boundary: create → generate → stats → preview → export warning; error display | `apps/desktop/src/App.test.tsx` (`pnpm test`) |
| End-to-end (Chromium) | the real frontend driven by Playwright with `invoke` bridged to the **real Rust core** via `jd-mcp` (MCP over stdio): project files appear on disk, the viewport reports its backend and triangle count, a 16-bit PNG is written | `apps/desktop/e2e/terrain.spec.ts` (`pnpm test:e2e`) |
| Native window | screenshots of the real Tauri app on macOS; interaction recording requires Accessibility permission for the automating process | this page |

Known limits of this build: no file dialogs yet (type paths), no node editor, no sculpting, one layer. Figma-annotated tutorial images and screen recordings follow once the workflow panels exist.
