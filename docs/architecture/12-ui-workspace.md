# 12 — UI Workspace and Design Language

## Design language

- Dark-first. Palette: black `#0A0A0A`, charcoal `#141414`, graphite `#1F1F1F`/`#2A2A2A`, greys `#3A3A3A` → `#9A9A9A`, white `#F2F2F2`; one accent for selection/progress (muted, high-contrast; exact value chosen with contrast tests ≥ 4.5:1 for text).
- Typography: Host Grotesk (verify licence/availability in `docs/research/stack.md`); monospace fallback for values/logs.
- Icons: Lucide. Spacing on a 4 px grid. Tailwind utility classes with a small token layer (`--jd-bg-0..3`, `--jd-fg-0..2`, `--jd-accent`).
- Minimal chrome, progressive disclosure (advanced node params behind "More"), clear technical language, tooltips with units.

## Workspace layout

```mermaid
flowchart TB
  subgraph window["Main window"]
    TB["Top bar: project · command palette (⌘K) · build/preview toggle · quality · job indicator"]
    subgraph center["Center (dockable, resizable)"]
      VP["3D Viewport<br/>(three.js WebGPU/WebGL 2 canvas — ADR-0003)"]
      NE["Node editor (graph)"]
    end
    subgraph left["Left dock"]
      SH["Scene hierarchy"]
      AB["Asset browser"]
    end
    subgraph right["Right dock"]
      PI["Property inspector"]
      ME["Material editor"]
      EW["Environment & weather"]
    end
    subgraph bottom["Bottom dock"]
      TL["Simulation timeline"]
      TM["Task monitor"]
      PF["Profiler"]
      DC["Diagnostics console"]
      EX["Export manager"]
    end
    TB --- center
    left --- center --- right
    center --- bottom
  end
```

Panels map to the brief §12 workspace list; sculpt/brush/mask controls appear as a toolbar overlay on the viewport when a stroke node is active.

## Frontend architecture

```mermaid
flowchart LR
  subgraph fe["Frontend (TypeScript)"]
    ST["State store (document mirror, job list, selection)"]
    NEU["Node editor (xyflow/React Flow — ADR-0004)"]
    INS["Inspectors (generated from node param JSON schemas)"]
    VPH["Viewport host (surface or canvas)"]
    JOB["Job monitor (subscribes to progress channel)"]
  end
  subgraph ipc["Tauri IPC"]
    INV["invoke(op, envelope)"]
    CH["channels: progress, previews (binary)"]
    EV["events: document changed, job state"]
  end
  NEU & INS & JOB --> ST
  ST <--> INV
  CH --> ST & VPH
  EV --> ST
```

- The frontend holds a **mirror** of the document, never the source of truth; every mutation is an op round-trip (fast, local) and the store updates from the result/event, which keeps GUI, CLI and MCP consistent.
- Inspector forms are generated from the node parameter JSON schemas (`schemas/`), so adding a node in Rust adds its UI without frontend code.
- Non-blocking: every op that can exceed ~50 ms is a job; the UI shows progress and a cancel control.
- Keyboard shortcuts and a searchable command palette are backed by the operation registry.
- Accessibility: focus order, ARIA roles on custom widgets, contrast ≥ 4.5:1, reduced-motion respect.

## Interaction principles (brief §12)

Clear navigation, progressive disclosure, contextual guidance, shortcuts, searchable commands, undo/redo, presets, tooltips, consistent feedback, non-blocking ops, accessible errors (`message / why / next` from `JdError`), progress and cancellation.
