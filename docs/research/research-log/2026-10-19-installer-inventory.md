# Research log — installer-surface inventory of reference tools

Date: 2026-10-19 · Operator: Claude (agent) directed by the project owner · Policy: ADR-0010 (REA / clean-room)

## Scope and legal basis

Targets are installers the owner downloaded for personal use (`installables/`, git-ignored): **World Machine Basic 4067** (macOS `.pkg`) and **Gaea 2.3.1.0** (Windows `.exe`). Both are proprietary. World Machine's EULA §5(a) (read from the package, see below) prohibits reverse engineering, disassembly and decompilation, with the EU Directive 2009/24/EC interoperability exception. QuadSpinner's Gaea EULA was not readable from the installer (see Gaea section) and is assumed to contain equivalent terms.

Accordingly this session performed **inventory-level inspection only**: package metadata, file lists, bundled licence/privacy/release-note texts that the vendor ships for users to read, linked system/third-party libraries (`otool -L`), resource/preset/example file names and types, and file-type magic of data files. **No** decompilation, disassembly, debugging, tracing, licence-check inspection, or format reverse engineering was performed. REA (`rea-agents` 6.7.0, MIT) was **not** installed or run: it was not present on the machine, the authorised scope is fully covered by native tools (`pkgutil`, `otool`, `file`, `7z -tPE`, `innoextract`), and ADR-0010 requires REA to run in an isolated VM. Nothing in this log is a source for implementation; it refines the competitive matrix and the memory/CPU budgeting design only.

## Commands (reproducible)

```
pkgutil --expand-full installables/WorldMachineBasic-4067.pkg <scratch>/wm/expanded
plutil -p ".../World Machine Basic.app/Contents/Info.plist"
otool -L ".../Contents/MacOS/World Machine Basic"
file / xxd -l 64 on startup.tmd and example .tmd files (magic only)
sed/grep on "WM License.txt", "WM Privacy.txt", licenses.txt, WhatsNew.txt, world.ini
7z l -tPE installables/Gaea-2.3.1.0.exe ; strings | grep installer signatures
innoextract -l installables/Gaea-2.3.1.0.exe   (1.9: unsupported loader; HEAD build attempted)
```

## World Machine Basic 4067 (macOS) — findings (all VERIFIED by direct inspection)

| Item | Observation |
|---|---|
| Package | `com.world-machine.worldmachine-basic` v4067 (CFBundleShortVersionString 4.0.67), 901 files, ~238 MB installed, install location `/Applications`, built with InstallCmds-864.12 on macOS 25G83 |
| Binary | Universal (x86_64 + arm64) Mach-O, 32 MB main executable; signed (`_CodeSignature`) |
| UI toolkit | **Qt 6.11.1** (QtCore/Gui/Widgets/OpenGL/OpenGLWidgets/Network/Svg/DBus) — licence notice: LGPL |
| Graphics | Links **OpenGL.framework and Metal.framework**; bundles **`libslang-compiler.0.2026.7.1.dylib`** (Slang shader compiler 2026.7.1, 56 MB) — consistent with the Code device compiling Slang to Metal on macOS |
| Other system frameworks | Security, IOKit, DiskArbitration, CoreWLAN (licence activation / machine identification per the privacy policy), ImageIO, UniformTypeIdentifiers |
| Third-party notices (`licenses.txt`) | Qt (LGPL), OpenEXR, LibTIFF, LibPNG, zlib, Freepik icons |
| Plugin architecture | `Contents/Resources/plugins/devices/DevicesStandard.bundle` and `DevicesCustomGUI.bundle` — devices ship as loadable plugin bundles |
| Presets | 34 folders under `Resources/presets/`: 4-character device codes `AO01 APRL BIGA BLUR CELL CLMP CMB2 COER COG2 COGR CRVG CRVP cTNT CURV ERD2 EWTH EXPA FILI LCRP LCRV P_cm PRLM RGEN SNOW SSEL STRA TRCE`, plus `ViewportLighting` and 5 GUID-named custom-device presets |
| Examples | 54 `.tmd` worlds + 4 `.tms` files + images, in categories Beginner Tutorials, Scenes (Simple/Intermediate/Advanced), Device Examples (e.g. Flow Restructure, River, Terraced River, Height Matching Chooser, MultiChooser, Mimic Device, Roughness Selector), Techniques, Utility Examples, Materials |
| Project format | `.tmd` = UTI `com.world-machine.tmd` "World Machine World"; **binary, chunked** (magic `TMDFile2`, length-prefixed chunk names such as `DEVICEWORLD`, `WorldCommonData`). Not JSON/XML. No further inspection. |
| Resources | `assets.db`, `colors.tga/txt`, `startup.tmd`, `world.ini`, `WhatsNew.txt`, `WM License.txt`, `WM Privacy.txt`, icon packs `.rcc` |
| Memory/build defaults (`world.ini`) | `Memory_Percentage=85`, `Memory_Percentage_History=35`, `Memory_Paging=1`, `Paging_build=1`, `Paging_history=1`, `Paging_history_archiveGB=32`, `Paging_minRes=2048`, `Preview_Size=128`, `Preview_Limit=2048`, `Preview_TargetTime=57`, `Viewport_3Dlimit_default=2048`, `Viewport_3Dlimit_large=8192`, `Allow Device Threading=1`, `Build_autothrottle=1`, `Workview_multisample=8` |
| Release notes (4067) | Meshify device 4–20× faster with better triangle quality near shorelines; background previewer is memory-aware (stops refining or pages when memory is high); int-grid support in tiled builds; EXR single-channel input; cancellation improvements across devices; custom device versioning/library sync; macOS fixes (pinch zoom, file association) |
| EULA (`WM License.txt`) | Editions: Community (personal, **non-commercial**, Output too), Time-Limited Evaluation (30 days), Indie/Professional per-seat (2 machines, one in use), Site. §5(a): no reverse engineering/decompiling/disassembly except the EU interoperability exception; §5(b): no service-bureau / **terrain-generation-as-a-service** without consent. "Bundled Content" (devices, macros, presets, examples) is part of the Software |
| Privacy (`WM Privacy.txt`) | Creative work never leaves the machine; licence activation sends licence key + machine identifier + IP; telemetry pseudonymous (random installation ID); crash reports opt-in |

### What this changes in Jenga Dunia's design

1. **Memory budgeting pattern confirmed from a shipping tool:** percentage-of-RAM budget with separate history budget, paging thresholds by resolution, disk archive cap, auto-throttle and memory-aware preview refinement → mirrors `06-jobs.md` (MemoryBudget, priorities, cooperative cancellation); add "history/undo paging with archive cap" and "preview target frame time" to the Phase 2 backlog.
2. **Device-as-plugin bundles** and **versioned custom-device library** validate ADR-0009/`09-plugins.md` and the macro/library item in the graph backlog.
3. **Slang in a shipping terrain tool** is a data point for the "user-programmable node"; Jenga Dunia's plan (WGSL compute node) stands — Slang would add a 56 MB compiler dependency.
4. **Binary chunked `.tmd`** reinforces ADR-0011 (open JSON project format as a differentiator).
5. **Qt 6 + OpenGL/Metal** on macOS: World Machine's cross-platform port went native-toolkit; Jenga Dunia's Tauri/wgpu route is different but equally viable; no change.
6. **EULA language** (terrain-generation-as-a-service prohibited; Community output non-commercial) sharpens the open-source positioning: Jenga Dunia must state plainly that outputs are unrestricted.

## Gaea 2.3.1.0 (Windows) — findings

| Item | Observation (VERIFIED) |
|---|---|
| Installer | 32-bit PE bootstrapper, **Inno Setup 7.0.0.3** (messages 6.5.0), linker 2.25, built 2026-08-10; version resource: CompanyName QuadSpinner, ProductName Gaea, "Copyright © 2008-2024 QuadSpinner", ProductVersion 2.0; 68 MB payload after the PE image |
| Extraction | Neither `innoextract 1.9` (Homebrew) nor `innoextract 1.10-dev+6e9e34e` (HEAD build) supports the Inno Setup 7 loader ("Unexpected setup loader revision: 2 … Could not determine setup data version"). Without extraction, no file inventory, EULA text or third-party notices could be read from the installer on macOS. **Follow-up (owner's call):** install Gaea Community on a Windows machine/VM under its own EULA and inventory the installed folder (file list, third-party notices, bundled EULA) the same way — still inventory-only. |
| Note | The research report `gaea-world-machine.md` already documents Gaea's edition terms from the public purchase/EULA pages. |

## Limits

- No runtime observation (neither app was launched); no network capture; no inspection of licence enforcement.
- World Machine Basic is the Community edition: the EULA restricts Output of this edition to non-commercial use, which is irrelevant here because no Output was produced.
- Findings are competitive context only and must not be cited as implementation sources (ADR-0010).
