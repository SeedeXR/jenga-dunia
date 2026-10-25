import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Box, Brush, Camera, Circle, CircleDot, Download, FileInput, FilePlus, FolderOpen, FolderPlus, FolderSearch, Gem, Globe, Grid2x2, Grid3x3, Grip, Home, Maximize2, Mountain, PanelBottom, PanelLeftClose, PanelLeftOpen, PanelRight, PanelRightClose, PanelRightOpen, Play, Redo2, Route, Save, Sparkles, Undo2, Upload, Wand2, WavesVertical, Workflow } from "lucide-react";
import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import GraphPanel from "./graph/GraphPanel";
import LayerStack from "./graph/LayerStack";
import TerrainView, { type ViewportInfo } from "./viewport/TerrainView";
import { type Brush as BrushTool, type NoiseSettings, useStore } from "./state";
import type { FractalKind, HeightmapFormat, NoiseKind } from "./lib/ops.gen";
import type { Preview } from "./lib/api";

/** A resizable panel's size, kept in the browser store (`jd.layout.<key>`) and restored on launch: the
 *  inline style sets the saved px, a ResizeObserver writes every change (try/catch: storage may be off). */
function usePersistedSize(key: string, axis: "width" | "height"): { ref: React.RefCallback<HTMLElement>; style: React.CSSProperties } {
  const saved = useRef<number | null>(null);
  if (saved.current === null) {
    try {
      const v = Number(localStorage.getItem(`jd.layout.${key}`));
      saved.current = Number.isFinite(v) && v > 0 ? v : 0;
    } catch {
      saved.current = 0;
    }
  }
  const observer = useRef<ResizeObserver | null>(null);
  const ref = useCallback(
    (el: HTMLElement | null) => {
      observer.current?.disconnect();
      observer.current = null;
      if (!el || typeof ResizeObserver === "undefined") return;
      observer.current = new ResizeObserver(() => {
        const px = Math.round(axis === "width" ? el.getBoundingClientRect().width : el.getBoundingClientRect().height);
        if (px <= 0) return;
        try {
          localStorage.setItem(`jd.layout.${key}`, String(px));
        } catch {
          /* no storage: the size lasts the session */
        }
      });
      observer.current.observe(el);
    },
    [key, axis],
  );
  return { ref, style: saved.current ? { [axis]: `${saved.current}px` } : {} };
}

type Dock = { left: boolean; right: boolean; graph: "bottom" | "right" };
function readDock(): Dock {
  try {
    const d = JSON.parse(localStorage.getItem("jd.layout.dock") ?? "{}") as Partial<Dock>;
    return { left: d.left === true, right: d.right === true, graph: d.graph === "right" ? "right" : "bottom" };
  } catch {
    return { left: false, right: false, graph: "bottom" };
  }
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex items-center justify-between gap-2 py-1 text-jd-fg-1">
      <span className="text-xs">{label}</span>
      {children}
    </label>
  );
}

const inputCls = "w-28 rounded-md border border-white/5 bg-jd-bg-2 px-2 py-0.5 text-right text-xs text-jd-fg-0 transition-colors hover:border-white/10 focus:border-jd-accent focus:outline-none";

/** World size presets; Unity/Unreal landscapes want 2ⁿ+1 samples so quads line up with their patch grids. */
export const PRESETS: { label: string; resolution: number; extent: number }[] = [
  { label: "Preview 512² · 2 km", resolution: 512, extent: 2048 },
  { label: "Square 2048² · 8 km", resolution: 2048, extent: 8192 },
  { label: "Square 4096² · 16 km", resolution: 4096, extent: 16384 },
  { label: "Unity 1025² · 2 km", resolution: 1025, extent: 2048 },
  { label: "Unity / Unreal 2049² · 8 km", resolution: 2049, extent: 8192 },
  { label: "Unreal 4033² · 16 km", resolution: 4033, extent: 16128 },
  { label: "Unity 4097² · 16 km", resolution: 4097, extent: 16384 },
];

/** Terrain presets: patches on the noise settings (seed and lattice stay as set). */
export const NOISE_PRESETS: { label: string; patch: Partial<NoiseSettings> }[] = [
  { label: "Mountains 0..500 m", patch: { fractal: "fbm", feature_size_m: 1024, octaves: 6, gain: 0.5, height_range_m: [0, 500] } },
  { label: "Ridged alpine 0..1200 m", patch: { fractal: "ridged", feature_size_m: 1400, octaves: 7, gain: 0.5, height_range_m: [0, 1200] } },
  { label: "Rolling hills 0..150 m", patch: { fractal: "fbm", feature_size_m: 800, octaves: 5, gain: 0.45, height_range_m: [0, 150] } },
  { label: "Desert plain 0..40 m", patch: { fractal: "fbm", feature_size_m: 1200, octaves: 3, gain: 0.5, height_range_m: [0, 40] } },
];

/** sRGB 0..1 triple ↔ `#rrggbb` for the colour input. */
export function hexOf(rgb: [number, number, number]): string {
  return `#${rgb.map((c) => Math.round(Math.min(1, Math.max(0, c)) * 255).toString(16).padStart(2, "0")).join("")}`;
}
export function rgbOf(hex: string): [number, number, number] {
  const n = Number.parseInt(hex.slice(1), 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

export function formatFor(path: string): HeightmapFormat {
  const ext = path.split(".").pop()?.toLowerCase();
  return ext === "png" ? "png16" : ext === "r16" ? "raw_r16" : ext === "r32" ? "raw_r32" : ext === "tif" || ext === "tiff" ? "tiff32" : "exr";
}

/** Nice tick step for a span in metres: 1 / 2 / 5 × 10ⁿ giving 4–8 ticks. */
function tickStep(span: number): number {
  const raw = span / 6;
  const pow = Math.pow(10, Math.floor(Math.log10(Math.max(raw, 1e-6))));
  const m = raw / pow;
  return (m < 1.5 ? 1 : m < 3.5 ? 2 : m < 7.5 ? 5 : 10) * pow;
}

const HINTS: Record<"orbit" | "sculpt" | "grid" | "road", [string, string][]> = {
  orbit: [["Drag", "orbit"], ["Wheel", "zoom"]],
  sculpt: [["Left drag", "stroke"], ["Right drag", "orbit"], ["Ctrl + wheel", "radius"], ["Shift + wheel", "strength"], ["Sculpting…", "back to orbit"]],
  grid: [["Click", "toggle a cell"], ["Left drag", "add cells"], ["Alt + drag", "remove cells"], ["Right drag", "orbit"], ["Grid panel", "level / raise / lower"]],
  road: [["Left drag", "lay the road"], ["Right drag", "orbit"], ["Drawing road…", "cancel"]],
};

/** World Creator-style viewport chrome: a compass strip, a height ruler, per-tool shortcut hints and fit / still / fullscreen buttons. */
function ViewportChrome({ heading, distanceM, preview, seaLevelM, mode, onFit, onStill, canStill, frameOn, onFrame }: { heading: number; distanceM: number; preview: Preview | null; seaLevelM: number | null; mode: "orbit" | "sculpt" | "grid" | "road"; onFit: () => void; onStill: () => void; canStill: boolean; frameOn: boolean; onFrame: () => void }) {
  // Compass: 180° of bearings across a 240 px strip, the camera's heading at the centre mark.
  const W = 240;
  const px = (deg: number) => W / 2 + ((((deg - heading + 540) % 360) - 180) * W) / 180;
  const ticks = Array.from({ length: 24 }, (_, i) => i * 15);
  const names: Record<number, string> = { 0: "N", 90: "E", 180: "S", 270: "W" };
  // Ruler: the preview's height range, zero and the sea level marked, negatives below the datum.
  const lo = preview?.min ?? 0;
  const hi = preview?.max ?? 1;
  const span = Math.max(hi - lo, 1e-6);
  const step = tickStep(span);
  const first = Math.ceil(lo / step) * step;
  const rulerTicks: number[] = [];
  for (let v = first; v <= hi + 1e-6 && rulerTicks.length < 40; v += step) rulerTicks.push(Number(v.toFixed(6)));
  const y = (v: number) => 100 - ((v - lo) / span) * 100;
  const fmt = (v: number) => (Math.abs(v) >= 1000 ? `${(v / 1000).toFixed(v % 1000 === 0 ? 0 : 1)} km` : `${Math.round(v)} m`);
  const fullscreen = () => {
    const el = document.querySelector('[data-testid="viewport"]') as HTMLElement | null;
    if (!el) return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else void el.requestFullscreen?.();
  };
  return (
    <>
      <div className="pointer-events-none absolute top-11 left-1/2 flex -translate-x-1/2 flex-col items-center" data-testid="compass" aria-label={`heading ${Math.round(heading)}°`}>
        <svg width={W} height={26} className="overflow-visible">
          <rect x={0} y={0} width={W} height={26} rx={4} className="fill-jd-bg-1/80" />
          {ticks.map((t) => {
            const x = px(t);
            if (x < 2 || x > W - 2) return null;
            const major = t % 90 === 0;
            return (
              <g key={t}>
                <line x1={x} y1={major ? 15 : 19} x2={x} y2={26} className="stroke-jd-fg-2" strokeWidth={1} />
                {major && (
                  <text x={x} y={12} textAnchor="middle" className={`text-[10px] font-semibold ${t === 0 ? "fill-jd-accent" : "fill-jd-fg-0"}`}>
                    {names[t]}
                  </text>
                )}
              </g>
            );
          })}
          <polygon points={`${W / 2 - 4},26 ${W / 2 + 4},26 ${W / 2},19`} className="fill-jd-accent" />
        </svg>
        <span className="mt-0.5 rounded bg-jd-bg-1/70 px-1 text-[10px] text-jd-fg-1" data-testid="compass-heading">
          {Math.round(heading)}° · {distanceM >= 1000 ? `${(distanceM / 1000).toFixed(1)} km` : `${Math.round(distanceM)} m`}
        </span>
      </div>
      {preview && (
        // The ruler in HTML: ticks placed by percentage of the height range, text at its real size.
        <div className="pointer-events-none absolute top-14 bottom-16 left-1 w-16" data-testid="height-ruler" aria-label={`heights ${fmt(lo)} to ${fmt(hi)}`}>
          <div className="absolute top-0 right-1 bottom-0 w-px bg-jd-fg-2/60" />
          {rulerTicks.map((v) => (
            <div key={v} className="absolute right-1 flex -translate-y-1/2 items-center gap-1" style={{ top: `${y(v)}%` }}>
              <span className={`whitespace-nowrap text-[10px] tabular-nums ${v < 0 ? "text-jd-warn" : v === 0 ? "text-jd-fg-0" : "text-jd-fg-2"}`}>{fmt(v)}</span>
              <span className={`h-px ${v === 0 ? "w-3 bg-jd-fg-0" : "w-2 bg-jd-fg-2"}`} />
            </div>
          ))}
          {seaLevelM !== null && seaLevelM >= lo && seaLevelM <= hi && (
            <div className="absolute right-0 flex -translate-y-1/2 items-center gap-1" style={{ top: `${y(seaLevelM)}%` }} data-testid="ruler-sea">
              <span className="text-[10px] text-sky-300">sea</span>
              <span className="h-0.5 w-4 bg-sky-400" />
            </div>
          )}
          <div className="absolute -top-5 left-0 whitespace-nowrap text-[10px] text-jd-fg-1" data-testid="ruler-max">
            Max {fmt(hi)}
          </div>
          <div className="absolute -bottom-5 left-0 whitespace-nowrap text-[10px] text-jd-fg-1" data-testid="ruler-min">
            Min {fmt(lo)}
          </div>
        </div>
      )}
      <dl className="pointer-events-none absolute bottom-2 left-1/2 grid -translate-x-1/2 grid-cols-[auto_auto] gap-x-3 gap-y-0 rounded bg-jd-bg-1/80 px-3 py-1 text-[10px] text-jd-fg-2" data-testid="viewport-hints" data-mode={mode}>
        {HINTS[mode].map(([k, v]) => (
          <Fragment key={k}>
            <dt className="text-jd-fg-1">{k}</dt>
            <dd>{v}</dd>
          </Fragment>
        ))}
      </dl>
      <span className="absolute top-2 left-2 flex gap-1">
        <button className="btn px-1.5" title="Fit: back to the home view" onClick={onFit} data-testid="view-fit">
          <Home size={14} />
        </button>
        <button className="btn px-1.5" title="Still: save this frame as exports/stills/view_<n>.png" onClick={onStill} disabled={!canStill} data-testid="view-still">
          <Camera size={14} />
        </button>
        <button className="btn px-1.5" title="Fullscreen viewport (Esc leaves)" onClick={fullscreen} data-testid="view-fullscreen">
          <Maximize2 size={14} />
        </button>
        <button className={`btn px-1.5 ${frameOn ? "text-jd-accent" : ""}`} title="Frame: draw the world as a block with side walls and a rim" onClick={onFrame} aria-pressed={frameOn} data-testid="view-frame">
          <Box size={14} />
        </button>
      </span>
    </>
  );
}

export default function App() {
  const s = useStore();
  const [projPath, setProjPath] = useState("");
  const [importPath, setImportPath] = useState("");
  const [geo, setGeo] = useState({ lon: 36.82, lat: -1.29, radius: 2000, provider: "terrarium" as "terrarium" | "copernicus" | "opentopography" });
  const [frames, setFrames] = useState(24);
  const leftSize = usePersistedSize("left", "width");
  const rightSize = usePersistedSize("right", "width");
  const graphSize = usePersistedSize("graph", "height");
  const graphWidth = usePersistedSize("graphw", "width");
  // Docking: side panels collapse to a strip, the graph docks under or beside the viewport; kept in the browser store.
  const [dock, setDockState] = useState<Dock>(readDock);
  const setDock = useCallback((patch: Partial<Dock>) => {
    setDockState((d) => {
      const next = { ...d, ...patch };
      try {
        localStorage.setItem("jd.layout.dock", JSON.stringify(next));
      } catch {
        /* no storage */
      }
      return next;
    });
  }, []);
  const [projName, setProjName] = useState("World");
  const [extent, setExtent] = useState(4096);
  const [resolution, setResolution] = useState(2048);
  const [exportPath, setExportPath] = useState("height.exr");
  const [engine, setEngine] = useState<"unity" | "unreal">("unity");
  const [meshPath, setMeshPath] = useState("terrain.glb");
  const [tiles, setTiles] = useState(1); // 1 = one folder; 2..16 = tiles × tiles folders with tiles.json
  const [scatterModel, setScatterModel] = useState(""); // an FBX in assets/ to instance; empty = the proxy
  const [holesLayer, setHolesLayer] = useState("");
  const [hdriPath, setHdriPath] = useState("sky.exr");
  const [flyFrames, setFlyFrames] = useState(48);
  const [frameOn, setFrameOn] = useState(true); // the world drawn as a framed block // an EXR in assets/ (or an absolute path) for the dome // a 0..1 layer whose cells ≥ 0.5 are holes in the exports; "" = none
  const [skyPath, setSkyPath] = useState("sky.exr");
  const [vp, setVp] = useState<ViewportInfo>({ backend: "none", triangles: 0, heading: 326, distanceM: 5200 });
  const [showGraph, setShowGraph] = useState(true);
  const onInfo = useCallback((i: ViewportInfo) => setVp(i), []);
  const pathInput = useRef<HTMLInputElement>(null);
  const sculptNode = s.graph.nodes.find((n) => n.id === s.selectedNode && n.kind.startsWith("jd.sculpt.strokes@"));
  // The viewport draws the sea where the graph's sea-level node puts it (only over the height layer).
  const seaNode = s.graph.nodes.find((n) => n.kind.startsWith("jd.hydrology.sea_level@"));
  const noiseNode = s.graph.nodes.find((n) => n.kind.startsWith("jd.noise."));
  const seaLevelM = seaNode && (s.layer === null || s.layer.layer.startsWith("height")) ? Number(seaNode.params?.level_m ?? 0) : null;
  const seaTint = seaNode?.params?.tint;
  const water = seaNode ? { visibilityM: Number(seaNode.params?.visibility_m ?? 15), tint: (Array.isArray(seaTint) && seaTint.length === 3 ? seaTint.map(Number) : [0.1, 0.35, 0.4]) as [number, number, number], breakingDepthM: s.waves?.breaking_depth_m ?? 0 } : null;
  // The wind sea is the core's call (`ocean.waves`): re-fetched when the node's wind changes.
  // A sea node without its own wind follows the world's shared wind (Settings ▸ Wind).
  const worldWind = s.project?.world.wind ?? { azimuth_deg: 270, speed_mps: 8 };
  const worldOrigin = s.project?.world.origin ?? [0, 0];
  const worldCrs = s.project?.world.crs ?? "";
  const windSpeed = seaNode ? Number(seaNode.params?.wind_speed_mps ?? worldWind.speed_mps) : null;
  const fetchKm = seaNode?.params?.fetch_km == null ? null : Number(seaNode.params.fetch_km);
  const windAzimuth = seaNode ? Number(seaNode.params?.wind_azimuth_deg ?? worldWind.azimuth_deg) : null;
  const refreshSky = s.refreshSky;
  useEffect(() => {
    void refreshSky(); // the dome shows the physical sky from the first frame; later sun edits re-bake it
  }, [refreshSky]);
  const loadWaves = s.loadWaves;
  useEffect(() => {
    void loadWaves(windSpeed === null ? null : { wind_speed_mps: windSpeed, fetch_km: fetchKm, wind_azimuth_deg: windAzimuth ?? 270 });
  }, [loadWaves, windSpeed, fetchKm, windAzimuth]);

  // Native file dialogs (tauri-plugin-dialog). Outside Tauri (browser, tests) they reject and the
  // typed paths keep working, so the dialogs are a convenience layer over the same operations.
  const pickProjectToOpen = useCallback(async () => {
    const p = await openDialog({ directory: true, title: "Open Jenga Dunia project" }).catch(() => null);
    if (typeof p !== "string") return;
    setProjPath(p);
    await useStore.getState().openProject(p);
  }, []);
  const pickProjectToCreate = useCallback(async () => {
    const chosen = await saveDialog({ title: "New Jenga Dunia project", defaultPath: `${projName}.jdproj` }).catch(() => null);
    if (!chosen) return;
    const p = chosen.endsWith(".jdproj") ? chosen : `${chosen}.jdproj`; // a project is a directory
    setProjPath(p);
    await useStore.getState().newProject(p, projName, extent, resolution);
  }, [projName, extent, resolution]);
  const pickAssetToImport = useCallback(async () => {
    const p = await openDialog({
      title: "Import heightmap or mesh",
      filters: [
        { name: "Heightmaps and meshes", extensions: ["png", "exr", "tif", "tiff", "r16", "r32", "fbx"] },
      ],
    }).catch(() => null);
    if (typeof p !== "string") return;
    setImportPath(p);
    await useStore.getState().importAsset(p);
  }, []);
  const [refPath, setRefPath] = useState("");
  const pickReference = useCallback(async () => {
    const p = await openDialog({
      title: "Reference image or video",
      filters: [{ name: "Images and videos", extensions: ["jpg", "jpeg", "png", "tif", "tiff", "heic", "webp", "mp4", "mov"] }],
    }).catch(() => null);
    if (typeof p !== "string") return;
    setRefPath(p);
    await useStore.getState().readReference(p);
  }, []);
  const pickExportPath = useCallback(async () => {
    const p = await saveDialog({
      title: "Export heightmap",
      defaultPath: exportPath,
      filters: [
        { name: "OpenEXR (32-bit float)", extensions: ["exr"] },
        { name: "PNG (16-bit)", extensions: ["png"] },
        { name: "RAW 16-bit", extensions: ["r16"] },
        { name: "RAW 32-bit float", extensions: ["r32"] },
        { name: "TIFF (32-bit float)", extensions: ["tif", "tiff"] },
      ],
    }).catch(() => null);
    if (!p) return;
    setExportPath(p);
    await useStore.getState().exportHeightmap(p, formatFor(p));
  }, [exportPath]);

  // A project opened at launch (file association / `jd-app path.jdproj`) is adopted on mount.
  useEffect(() => {
    let cancelled = false;
    void useStore.getState().probeProject(() => cancelled);
    return () => {
      cancelled = true;
    };
  }, []);

  // macOS hands double-clicked .jdproj documents to a running app: the core opened it, re-read.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<string>("project-opened", () => void useStore.getState().probeProject(() => cancelled))
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Native menu bar → the same actions as the buttons (ids from src-tauri build_menu).
  // Esc aborts whatever is running (World Creator's "Generating (ESC to abort)"); inside a text
  // field Esc only leaves the field, so typing is never interrupted.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable)) return;
      const st = useStore.getState();
      for (const j of st.jobs) if (!j.cancelling) void st.cancelJob(j.id);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false; // StrictMode runs effects twice; never leave a second listener behind
    listen<string>("menu", (e) => {
      const st = useStore.getState();
      switch (e.payload) {
        case "new":
          void pickProjectToCreate();
          break;
        case "open":
          void pickProjectToOpen();
          break;
        case "save":
          if (st.project) void st.save();
          break;
        case "generate":
          if (st.project) void st.generate();
          break;
        case "evaluate":
          if (st.project) void st.evaluateGraph();
          break;
        case "undo-graph":
          if (st.project) void st.undoGraph();
          break;
        case "redo-graph":
          if (st.project) void st.redoGraph();
          break;
        case "export":
          if (st.layer) void pickExportPath();
          break;
        case "import":
          if (st.project) void pickAssetToImport();
          break;
      }
    })
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      })
      .catch(() => undefined); // not running inside Tauri (tests, browser): no menu
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [pickProjectToCreate, pickProjectToOpen, pickExportPath, pickAssetToImport]);

  return (
    <div className="grid h-full grid-rows-[40px_1fr_160px] bg-jd-bg-0 text-jd-fg-0">
      {/* Top bar */}
      <header className="flex items-center gap-3 border-b border-white/5 bg-jd-bg-1 px-3">
        <img src="/jd-mark.svg" alt="Jenga Dunia" className="h-5 w-5" data-testid="brand-mark" />
        <span className="font-semibold tracking-wide">Jenga Dunia</span>
        <span className="text-xs text-jd-fg-2" data-testid="app-version">v{__JD_VERSION__}</span>
        <span className="text-xs text-jd-fg-2">{s.project ? `${s.project.name} — ${s.project.world.resolution_px[0]}² · ${s.project.world.extent_m[0]} m` : "no project"}</span>
        <nav className="flex items-center gap-0.5 border-l border-jd-line pl-2" aria-label="Project and view actions" data-testid="toolbar">
          {(
            [
              ["tb-new", FilePlus, "New project… (⌘N)", () => void pickProjectToCreate(), false],
              ["tb-open", FolderOpen, "Open project… (⌘O)", () => void pickProjectToOpen(), false],
              ["tb-save", Save, "Save (⌘S)", () => void s.save(), !s.project],
              ["tb-import", FileInput, "Import a DEM, heightmap or FBX into assets/ (⌘I)", () => void pickAssetToImport(), !s.project],
              ["tb-export", Download, "Export the heightmap… (⌘E)", () => void pickExportPath(), !s.layer],
              null,
              ["tb-undo", Undo2, "Undo graph edit (⌥⌘Z)", () => void s.undoGraph(), !s.project || !s.graph.undo_depth],
              ["tb-redo", Redo2, "Redo graph edit (⇧⌥⌘Z)", () => void s.redoGraph(), !s.project || !s.graph.redo_depth],
              null,
              ["tb-generate", Wand2, "Generate terrain (⌘G)", () => void s.generate(), !s.project],
              ["tb-evaluate", Play, "Evaluate the graph", () => void s.evaluateGraph(), !s.project],
              null,
              ["tb-still", Camera, "Save the viewport as a still (exports/stills/)", () => void s.saveStill(), !s.project || !s.viewport],
              ["tb-fullscreen", Maximize2, "Fullscreen viewport (Esc leaves)", () => {
                const el = document.querySelector('[data-testid="viewport"]') as HTMLElement | null;
                if (document.fullscreenElement) void document.exitFullscreen();
                else void el?.requestFullscreen?.();
              }, false],
            ] as ([string, typeof Save, string, () => void, boolean] | null)[]
          ).map((b, i) =>
            b === null ? (
              <span key={`sep-${i}`} className="mx-1 h-4 w-px bg-jd-line" />
            ) : (
              <button key={b[0]} className="rounded-md p-1.5 text-jd-fg-1 transition-colors hover:bg-jd-bg-3 hover:text-jd-fg-0 disabled:opacity-30" title={b[2]} aria-label={b[2]} onClick={b[3]} disabled={b[4] || s.busy} data-testid={b[0]}>
                {(() => {
                  const Icon = b[1];
                  return <Icon size={15} />;
                })()}
              </button>
            ),
          )}
        </nav>
        {s.lastEval && Object.keys(s.lastEval.layers).length > 1 && (
          <label className="ml-auto flex items-center gap-1 text-xs text-jd-fg-2">
            layer
            <select className="rounded border border-jd-line bg-jd-bg-2 px-1 text-xs text-jd-fg-0" value={s.layer?.layer ?? ""} onChange={(e) => void s.viewLayer(e.target.value)} data-testid="layer-pick">
              {Object.keys(s.lastEval.layers).map((n) => (
                <option key={n}>{n}</option>
              ))}
            </select>
            colour
            <select className="rounded border border-jd-line bg-jd-bg-2 px-1 text-xs text-jd-fg-0" value={s.colorLayer ?? ""} onChange={(e) => void s.setColorLayer(e.target.value || null)} title="Colour the terrain by another layer (masks and class maps) instead of the height ramp" data-testid="color-pick">
              <option value="">height ramp</option>
              {Object.keys(s.lastEval.layers).map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
        )}
        <span className={`${s.lastEval && Object.keys(s.lastEval.layers).length > 1 ? "" : "ml-auto "}text-xs text-jd-fg-2`} data-testid="viewport-info">
          viewport: {vp.backend} · {vp.triangles.toLocaleString()} tris{vp.seaM !== undefined ? ` · sea ${vp.seaM} m` : ""}{vp.waves ? ` · waves ${vp.waves.heightM.toFixed(1)} m / ${Math.round(vp.waves.lengthM)} m` : ""}{vp.fog ? ` · fog ${Math.round(vp.fog * 100)} %` : ""}{vp.sun ? ` · sun ${vp.sun.elevationDeg}°/${vp.sun.azimuthDeg}°` : ""}{s.colorLayer && s.colorPreview ? ` · colour ${s.colorLayer}` : ""}{vp.instances ? ` · ${vp.instances.toLocaleString()} instances` : ""}{vp.lakes ? ` · lakes ${vp.lakes.toLocaleString()} cells` : ""}{vp.pbrPx ? ` · pbr ${vp.pbrPx[0]}×${vp.pbrPx[1]}` : ""}{vp.skyPx ? ` · sky ${vp.skyPx[0]}×${vp.skyPx[1]}` : ""}{vp.hdri ? " · hdri" : ""}{vp.frame ? " · box" : ""}{vp.shoreSegments ? ` · shore ${vp.shoreSegments.toLocaleString()}` : ""}{vp.post ? ` · post ${vp.post}` : ""}{vp.horizonPx ? ` · horizon ${vp.horizonPx}²` : ""}
          {s.previewStats ? ` · preview lod ${s.previewStats.lod}, ${(s.previewStats.bytes / 1048576).toFixed(2)} MB in ${s.previewStats.ms.toFixed(0)} ms` : ""}
        </span>
        {(s.busy || s.jobs.length > 0) &&
          (s.jobs.length > 0 ? (
            <span className="flex items-center gap-2 text-xs text-jd-accent" data-testid="job-progress">
              {s.jobs[0]!.op}
              {s.jobs[0]!.total > 0 ? ` · node ${Math.min(s.jobs[0]!.done + 1, s.jobs[0]!.total)}/${s.jobs[0]!.total} ${s.jobs[0]!.current.split("@")[0]?.split(".").pop() ?? ""}` : ""}
              {` · ${(s.jobs[0]!.elapsed_ms / 1000).toFixed(1)} s`}
              <button className="btn px-1.5 py-0" onClick={() => void s.cancelJob(s.jobs[0]!.id)} disabled={s.jobs[0]!.cancelling} title="Cancel the running job (Esc)" data-testid="job-cancel">
                {s.jobs[0]!.cancelling ? "cancelling…" : "Cancel"}
              </button>
              <span className="text-jd-fg-2">Esc to abort</span>
            </span>
          ) : (
            <span className="text-xs text-jd-accent">working…</span>
          ))}
      </header>

      <div className="grid grid-cols-[auto_1fr_auto] overflow-hidden">
        {/* Left: project */}
        {dock.left ? (
          <aside className="flex w-6 flex-col items-center border-r border-jd-line bg-jd-bg-1 py-2" data-testid="panel-left-collapsed">
            <button className="btn px-1 py-0" title="Show the project panel" onClick={() => setDock({ left: false })} data-testid="expand-left">
              <PanelLeftOpen size={14} />
            </button>
          </aside>
        ) : (
        <aside ref={leftSize.ref} style={leftSize.style} className="jd-panel relative m-2 mr-1 w-[260px] min-w-[200px] max-w-[520px] resize-x overflow-y-auto p-3" title="Drag the bottom-right corner to resize this panel" data-testid="panel-left">
          <button className="btn absolute top-2 right-2 px-1 py-0" title="Collapse the project panel to the edge" onClick={() => setDock({ left: true })} data-testid="collapse-left">
            <PanelLeftClose size={14} />
          </button>
          <h2 className="mb-2 jd-section">Project</h2>
          <Field label="Path">
            <span className="flex gap-1">
              <input ref={pathInput} className={`${inputCls} w-28`} value={projPath} onChange={(e) => setProjPath(e.target.value)} placeholder="~/Worlds/Hills.jdproj" data-testid="project-path" />
              <button className="btn px-1.5" title="Browse… (opens an existing project)" onClick={() => void pickProjectToOpen()} disabled={s.busy} data-testid="browse-project">
                <FolderSearch size={14} />
              </button>
            </span>
          </Field>
          <Field label="Name">
            <input className={`${inputCls} w-36`} value={projName} onChange={(e) => setProjName(e.target.value)} />
          </Field>
          <Field label="Extent (m)">
            <input className={inputCls} type="number" value={extent} onChange={(e) => setExtent(Number(e.target.value))} />
          </Field>
          <Field label="Resolution (px)">
            <input className={inputCls} type="number" value={resolution} onChange={(e) => setResolution(Number(e.target.value))} />
          </Field>
          <Field label="Preset">
            <select
              className={`${inputCls} w-36`}
              value=""
              onChange={(e) => {
                const p = PRESETS.find((x) => x.label === e.target.value);
                if (p) {
                  setResolution(p.resolution);
                  setExtent(p.extent);
                }
              }}
              title="Engine-friendly sizes: Unity and Unreal heightmaps are 2ⁿ+1 samples"
              data-testid="world-preset"
            >
              <option value="">choose…</option>
              {PRESETS.map((p) => (
                <option key={p.label}>{p.label}</option>
              ))}
            </select>
          </Field>
          <div className="mt-2 flex gap-2">
            <button className="btn btn-primary" title="Create at the path above (⌘N opens a dialog)" onClick={() => s.newProject(projPath, projName, extent, resolution)} disabled={!projPath || s.busy} data-testid="new-project">
              <FolderPlus size={14} /> New
            </button>
            <button className="btn" onClick={() => s.openProject(projPath)} disabled={!projPath || s.busy} data-testid="open-project">
              <FolderOpen size={14} /> Open
            </button>
            <button className="btn" onClick={() => s.save()} disabled={!s.project || s.busy} data-testid="save-project">
              <Save size={14} /> Save
            </button>
          </div>
          {s.project && (
            <Field label="Import">
              <span className="flex gap-1">
                <input className={`${inputCls} w-28`} value={importPath} onChange={(e) => setImportPath(e.target.value)} placeholder="dem.tif, sculpt.fbx…" title="Heightmap (PNG 16-bit, EXR, TIFF, RAW) or binary FBX mesh: copied into assets/ and read by a new jd.input.heightmap node" data-testid="import-path" />
                <button className="btn px-1.5" title="Import this file (⌘I opens a dialog)" onClick={() => void s.importAsset(importPath)} disabled={!importPath || s.busy} data-testid="import-asset">
                  <Upload size={14} />
                </button>
                <button className="btn px-1.5" title="Browse…" onClick={() => void pickAssetToImport()} disabled={s.busy} data-testid="browse-asset">
                  <FolderSearch size={14} />
                </button>
              </span>
            </Field>
          )}
          {s.project && (
            <Field label="Real world">
              <span className="flex flex-wrap gap-1" title="Fetch a DEM around a point (ADR-0014: Terrarium tiles without a key, Copernicus GLO-30, or OpenTopography with JD_OPENTOPOGRAPHY_KEY), place the world in its CRS and feed it to the height output; the provider's notice goes into every export">
                <input className={`${inputCls} w-16`} type="number" step={0.001} value={geo.lon} onChange={(e) => setGeo({ ...geo, lon: Number(e.target.value) })} aria-label="Longitude" data-testid="geo-lon" />
                <input className={`${inputCls} w-16`} type="number" step={0.001} value={geo.lat} onChange={(e) => setGeo({ ...geo, lat: Number(e.target.value) })} aria-label="Latitude" data-testid="geo-lat" />
                <input className={`${inputCls} w-16`} type="number" min={100} step={100} value={geo.radius} onChange={(e) => setGeo({ ...geo, radius: Number(e.target.value) })} aria-label="Radius (m)" data-testid="geo-radius" />
                <select className={`${inputCls} w-24`} value={geo.provider} onChange={(e) => setGeo({ ...geo, provider: e.target.value as typeof geo.provider })} aria-label="Provider" data-testid="geo-provider">
                  <option value="terrarium">Terrarium</option>
                  <option value="copernicus">Copernicus</option>
                  <option value="opentopography">OpenTopography</option>
                </select>
                <button className="btn px-1.5" title="Fetch the DEM and place the world on it" onClick={() => void s.fetchDem([geo.lon, geo.lat], geo.radius, geo.provider)} disabled={s.busy} data-testid="geo-fetch">
                  <Globe size={14} />
                </button>
              </span>
            </Field>
          )}
          {s.project && (
            <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-2 gap-y-1 text-xs text-jd-fg-2">
              <dt>id</dt>
              <dd className="truncate font-mono text-jd-fg-1">{s.project.project_id}</dd>
              <dt>dirty</dt>
              <dd data-testid="project-dirty">{String(s.project.dirty)}</dd>
              <dt>journal</dt>
              <dd data-testid="project-journal">{s.project.pending_journal}</dd>
            </dl>
          )}
          {s.project && (
            <>
              <h2 className="mt-4 mb-2 jd-section">Reference image</h2>
              <Field label="File">
                <span className="flex gap-1">
                  <input className={`${inputCls} w-24`} value={refPath} onChange={(e) => setRefPath(e.target.value)} placeholder="photo, 360°, video" title="A photo (JPEG, PNG, TIFF, HEIF, WebP), a 360° photo sphere or a video (MP4, MOV): media.metadata reads where, when and how it was taken, and the sun" data-testid="ref-path" />
                  <button className="btn px-1.5" title="Choose a reference image or video…" onClick={() => void pickReference()} disabled={s.busy} data-testid="ref-browse">
                    <FolderSearch size={14} />
                  </button>
                  <button className="btn px-1.5" onClick={() => void s.readReference(refPath)} disabled={!refPath || s.busy} data-testid="ref-read">
                    Read
                  </button>
                </span>
              </Field>
              {s.reference && (
                <div className="mb-2 text-xs" data-testid="ref-info">
                  <dl className="grid grid-cols-[auto_1fr] gap-x-2 text-jd-fg-2">
                    <dt>kind</dt>
                    <dd>{s.reference.kind}{s.reference.px ? ` ${s.reference.px[0]}×${s.reference.px[1]}` : ""}</dd>
                    <dt>when</dt>
                    <dd>{s.reference.captured_at_utc ?? "unknown"}{s.reference.captured_at_source ? ` (${s.reference.captured_at_source})` : ""}</dd>
                    <dt>where</dt>
                    <dd>{s.reference.gps ? `${s.reference.gps.lat.toFixed(4)}, ${s.reference.gps.lon.toFixed(4)}` : "no GPS"}</dd>
                    <dt>heading</dt>
                    <dd>{s.reference.heading_deg != null ? `${s.reference.heading_deg.toFixed(0)}° ${s.reference.heading_ref ?? ""}` : "unknown"}</dd>
                    <dt>field of view</dt>
                    <dd>{s.reference.fov_x_deg != null ? `${s.reference.fov_x_deg.toFixed(1)}°` : s.reference.projection ? s.reference.projection.type : "unknown"}</dd>
                    <dt>sun</dt>
                    <dd>{s.reference.sun ? (s.reference.sun.below_horizon ? "below the horizon" : `${s.reference.sun.azimuth_deg.toFixed(0)}° from north, ${s.reference.sun.elevation_deg.toFixed(0)}° up`) : "needs place and time"}</dd>
                  </dl>
                  <div className="mt-1 flex gap-1">
                    <button className="btn" onClick={() => s.useReferenceSun()} disabled={!s.reference.sun || s.reference.sun.below_horizon} title="Set the viewport sun to the reference's" data-testid="ref-use-sun">
                      Use sun
                    </button>
                    <button className="btn" onClick={() => void s.terrainFromReference()} disabled={!s.reference.gps || s.busy} title="Fetch the real terrain 5 km around the reference's position (Terrarium) and place the world on it" data-testid="ref-terrain">
                      Terrain here
                    </button>
                  </div>
                  {s.reference.notes.length > 0 && (
                    <ul className="mt-1 list-disc pl-4 text-jd-fg-2" data-testid="ref-notes">
                      {s.reference.notes.map((n) => (
                        <li key={n}>{n}</li>
                      ))}
                    </ul>
                  )}
                </div>
              )}
              {s.project && <LayerStack />}
              <h2 className="mt-4 mb-2 jd-section">Settings</h2>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="Seconds after the last edit before the project autosaves; 0 disables">
                Autosave (s)
                <input className={inputCls} type="number" min={0} max={86400} key={`as-${s.settingsRev}-${s.project.settings.autosave_interval_s}`} defaultValue={s.project.settings.autosave_interval_s} onBlur={(e) => { const v = Number(e.target.value); if (v !== s.project?.settings.autosave_interval_s) void s.setSettings({ autosave_interval_s: v }); }} data-testid="setting-autosave" />
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="Layer cache cap on disk; oldest layers are evicted beyond it">
                Cache limit (MB)
                <input className={inputCls} type="number" min={16} key={`cl-${s.settingsRev}-${s.project.settings.cache_limit_bytes}`} defaultValue={Math.round(s.project.settings.cache_limit_bytes / 1048576)} onBlur={(e) => { const v = Math.round(Number(e.target.value)) * 1048576; if (v !== s.project?.settings.cache_limit_bytes) void s.setSettings({ cache_limit_bytes: v }); }} data-testid="setting-cache" />
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="Preview level of detail hint (resolution ÷ 2^lod)">
                Preview LOD
                <input className={inputCls} type="number" min={0} max={8} key={`pl-${s.settingsRev}-${s.project.settings.preview_lod}`} defaultValue={s.project.settings.preview_lod} onBlur={(e) => { const v = Number(e.target.value); if (v !== s.project?.settings.preview_lod) void s.setSettings({ preview_lod: v }); }} data-testid="setting-lod" />
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="Evaluations whose memory estimate exceeds this are refused (E_RESOURCE_MEMORY)">
                Memory budget (GB)
                <input className={inputCls} type="number" min={0.25} step={0.25} key={`mb-${s.settingsRev}-${s.project?.settings.memory_budget_bytes ?? 0}`} defaultValue={(s.project?.settings.memory_budget_bytes ?? 4294967296) / 1073741824} onBlur={(e) => { const v = Math.round(Number(e.target.value) * 1073741824); if (v !== (s.project?.settings.memory_budget_bytes ?? 4294967296)) void s.setSettings({ memory_budget_bytes: v }); }} title="Evaluations whose memory estimate exceeds this are refused (E_RESOURCE_MEMORY)" data-testid="setting-memory" />
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="Viewport preview detail: the longest side of the mesh the viewport shows. 1024² ≈ 1 M samples (2 MB a transfer, 2 M triangles); 2048² is 4× that — 8 MB and 8 M triangles, for worlds above 1024² when the fine ridges matter more than the frame rate">
                Preview detail
                <select className={inputCls} value={s.previewMaxPx} onChange={(e) => s.setPreviewMaxPx(Number(e.target.value))} data-testid="setting-preview-px">
                  <option value={1024}>1024²</option>
                  <option value={2048}>2048² (high)</option>
                </select>
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="Live preview target per evaluation, ms (0 = off): the preview LOD steps coarser while evaluations run over it and finer again when they are well under; this app setting is not part of the project">
                Live target (ms)
                <input className={inputCls} type="number" min={0} step={50} value={s.previewTargetMs} onChange={(e) => s.setPreviewTargetMs(Number(e.target.value))} data-testid="preview-target" />
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="World origin, metres: where pixel (0, 0) sits in the CRS. Georeferenced exports carry it; generators sample world space, so moving it moves the window over the noise field (adjacent worlds stay seamless)">
                Origin x (m)
                <input className={inputCls} type="number" step={1} key={`ox-${s.settingsRev}-${worldOrigin[0]}`} defaultValue={worldOrigin[0]} onBlur={(e) => { const v = Number(e.target.value); if (Number.isFinite(v) && v !== worldOrigin[0]) void s.setSettings({ origin: [v, worldOrigin[1]] }); }} data-testid="origin-x" />
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="World origin y (north), metres">
                Origin y (m)
                <input className={inputCls} type="number" step={1} key={`oy-${s.settingsRev}-${worldOrigin[1]}`} defaultValue={worldOrigin[1]} onBlur={(e) => { const v = Number(e.target.value); if (Number.isFinite(v) && v !== worldOrigin[1]) void s.setSettings({ origin: [worldOrigin[0], v] }); }} data-testid="origin-y" />
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="Coordinate reference system of the origin, e.g. EPSG:32637 (UTM 37N); empty = none. GeoTIFF exports carry it">
                CRS
                <input className={inputCls} type="text" placeholder="EPSG:32637" key={`crs-${s.settingsRev}-${worldCrs}`} defaultValue={worldCrs} onBlur={(e) => { const v = e.target.value.trim(); if (v !== worldCrs) void s.setSettings({ crs: v }); }} data-testid="crs" />
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="Shared wind: dunes, yardangs, snow scour and the wind sea follow it unless a node sets its own wind">
                Wind from (°)
                <input className={inputCls} type="number" min={0} max={360} key={`wa-${s.settingsRev}-${worldWind.azimuth_deg}`} defaultValue={worldWind.azimuth_deg} onBlur={(e) => { const v = Number(e.target.value); if (v !== worldWind.azimuth_deg) void s.setSettings({ wind: { azimuth_deg: v, speed_mps: worldWind.speed_mps ?? 8 } }); }} data-testid="wind-azimuth" />
              </label>
              <label className="mb-1 flex items-center justify-between gap-2 text-xs text-jd-fg-1" title="Shared wind speed 10 m above the ground, m/s: 3 a breeze, 8 moderate, 20 a gale">
                Wind (m/s)
                <input className={inputCls} type="number" min={0} step={0.5} key={`ws-${s.settingsRev}-${worldWind.speed_mps}`} defaultValue={worldWind.speed_mps} onBlur={(e) => { const v = Number(e.target.value); if (v !== worldWind.speed_mps) void s.setSettings({ wind: { azimuth_deg: worldWind.azimuth_deg ?? 270, speed_mps: v } }); }} data-testid="wind-speed" />
              </label>
            </>
          )}
          <h2 className="mt-4 mb-2 jd-section">Tasks</h2>
          <ul className="text-xs text-jd-fg-1" data-testid="tasks">
            {s.jobs.length === 0 && <li className="text-jd-fg-2">no running jobs</li>}
            {s.jobs.map((j) => (
              <li key={j.id} className="mb-1 flex items-center justify-between gap-2" data-testid={`task-${j.id}`}>
                <span className="truncate">
                  #{j.id} {j.op}
                  {j.state === "queued" ? " · queued" : j.total > 0 ? ` · ${Math.min(j.done + 1, j.total)}/${j.total}` : ""}
                  {j.priority !== "normal" ? ` · ${j.priority}` : ""} · {(j.elapsed_ms / 1000).toFixed(1)} s
                </span>
                <button className="btn px-1.5 py-0" onClick={() => void s.cancelJob(j.id)} disabled={j.cancelling}>
                  {j.cancelling ? "…" : "Cancel"}
                </button>
              </li>
            ))}
          </ul>
          <h2 className="mt-4 mb-2 jd-section">Diagnostics</h2>
          <div className="mb-2 flex flex-wrap gap-1">
            <button className="btn px-1.5 py-0" onClick={() => void s.runDiag("env")} disabled={s.busy} data-testid="diag-env">Environment</button>
            <button className="btn px-1.5 py-0" onClick={() => void s.runDiag("gc")} disabled={!s.project || s.busy} data-testid="diag-gc">Cache GC</button>
            <button className="btn px-1.5 py-0" onClick={() => void s.runDiag("bench")} disabled={s.busy} title="S1 scene at 256² with the real kernels; use `jd diag bench s1` for the full benchmark" data-testid="diag-bench">Bench 256²</button>
          </div>
          <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 text-xs text-jd-fg-2" data-testid="diag-results">
            {s.diag.env && (
              <>
                <dt>host</dt>
                <dd className="text-jd-fg-1">{s.diag.env.os} · {s.diag.env.arch} · {s.diag.env.cpus} cpus · {(s.diag.env.memory_total_bytes / 1073741824).toFixed(0)} GB</dd>
              </>
            )}
            {s.diag.gc && (
              <>
                <dt>cache</dt>
                <dd className="text-jd-fg-1">{s.diag.gc.blobs} blobs · {(s.diag.gc.bytes / 1048576).toFixed(1)} MB · evicted {s.diag.gc.evicted_blobs}{s.diag.gc.limit_bytes != null ? ` · limit ${(s.diag.gc.limit_bytes / 1048576).toFixed(0)} MB` : ""}</dd>
              </>
            )}
            {s.diag.bench && (
              <>
                <dt>bench</dt>
                <dd className="text-jd-fg-1">S1 {s.diag.bench.size_px}² {s.diag.bench.profile}: total {s.diag.bench.total_ms.toFixed(0)} ms{s.diag.bench.peak_rss_bytes != null ? ` · ${(s.diag.bench.peak_rss_bytes / 1048576).toFixed(0)} MB` : ""} · {s.diag.bench.stages.map((st) => `${st.name.split(".").pop()} ${st.ms.toFixed(0)}`).join(", ")}</dd>
              </>
            )}
          </dl>
        </aside>
        )}

        {/* Center: viewport (top) + node graph (bottom, toggle) */}
        <main className={`relative my-2 grid min-w-0 overflow-hidden rounded-xl bg-jd-bg-0 ring-1 ring-white/5 ${showGraph ? (dock.graph === "right" ? "grid-cols-[1fr_auto]" : "grid-rows-[1fr_auto]") : "grid-rows-[1fr]"}`}>
          <div className="relative min-h-0 min-w-0">
            <TerrainView fog={s.sun.fog} preview={s.preview} extentM={s.project ? s.project.world.extent_m : null} onInfo={onInfo} sculpt={{ enabled: s.sculpting && (sculptNode !== undefined || s.drawing !== null), radiusM: s.drawing === "road" ? s.roadWidthM / 2 : s.brush.radius_m, onStroke: (pts) => void s.addStroke(pts), onAdjust: s.adjustBrush }} grid={s.grid.on ? { cells: s.grid.cells, selected: s.grid.selected, onSelect: s.gridSelect } : null} seaLevelM={seaLevelM} lakes={s.lakesPreview} water={water} waves={s.waves} sky={s.sky} hdri={s.hdri} post={s.post} onHandle={s.setViewportHandle} frame={frameOn} horizon={s.horizon} sun={{ azimuthDeg: s.sun.azimuth_deg, elevationDeg: s.sun.elevation_deg }} overlay={s.colorPreview} overlayName={s.colorLayer} instances={s.scatter.points ? { points: s.scatter.points, sizeM: Math.max(s.scatter.spacing_m * 0.35, (s.project?.world.extent_m[0] ?? 0) * 0.012), shape: /debris|rock|boulder|scree|talus/i.test(s.scatter.mask ?? "") ? "rock" : "cone" } : null} maps={s.maps} />
            <ViewportChrome
              heading={vp.heading}
              distanceM={vp.distanceM}
              preview={s.preview}
              seaLevelM={seaLevelM}
              mode={s.sculpting ? (s.drawing === "road" ? "road" : "sculpt") : s.grid.on ? "grid" : "orbit"}
              onFit={() => s.viewport?.resetView()}
              frameOn={frameOn}
              onFrame={() => setFrameOn((v) => !v)}
              onStill={() => void s.saveStill()}
              canStill={!!s.viewport && !!s.project}
            />
            <button
              className={`btn absolute top-2 right-28 ${s.sculpting ? "text-jd-accent" : ""}`}
              title={s.sculpting ? "Sculpting: drag on the terrain to paint a stroke (click to stop)" : "Sculpt: adds a stroke node before the output and paints with the brush"}
              onClick={() => (s.sculpting ? s.setSculpting(false) : void s.startSculpting())}
              disabled={!s.project || s.busy}
              aria-pressed={s.sculpting}
              data-testid="sculpt-toggle"
            >
              <Brush size={14} /> {s.sculpting ? "Sculpting…" : "Sculpt"}
            </button>
            <button
              className={`btn absolute top-2 right-52 ${s.drawing === "road" ? "text-jd-accent" : ""}`}
              title={s.drawing === "road" ? "Drag on the terrain to lay the road (one drag = one road node)" : "Road: the next drag on the terrain becomes a spline node that flattens a road bed along it"}
              onClick={() => (s.drawing === "road" ? s.setSculpting(false) : s.startRoad())}
              disabled={!s.project || s.busy}
              aria-pressed={s.drawing === "road"}
              data-testid="road-draw"
            >
              <Route size={14} /> {s.drawing === "road" ? "Drawing road…" : "Road"}
            </button>
            <button
              className={`btn absolute top-2 right-[23.5rem] ${s.grid.on ? "text-jd-accent" : ""}`}
              title={s.grid.on ? "Grid: drag over cells to select them (Alt removes, click toggles); the Grid panel levels, raises or lowers them" : "Grid tool: select cells of the world and set their height, World Creator style"}
              onClick={() => s.toggleGrid(!s.grid.on)}
              disabled={!s.project || s.busy}
              aria-pressed={s.grid.on}
              data-testid="grid-toggle"
            >
              <Grid3x3 size={14} /> Grid
            </button>
            <button
              className={`btn absolute top-2 right-72 ${s.showHorizon ? "text-jd-accent" : ""}`}
              title="Horizon: continue the noise beyond the world's border as a coarse skirt (same seed, so it matches at the edge; erosion is not continued)"
              onClick={() => s.setShowHorizon(!s.showHorizon)}
              disabled={!s.project || s.busy}
              aria-pressed={s.showHorizon}
              data-testid="horizon-toggle"
            >
              <Mountain size={14} /> Horizon
            </button>
            <button className="btn absolute top-2 right-2" onClick={() => setShowGraph((v) => !v)} data-testid="toggle-graph">
              <Workflow size={14} /> {showGraph ? "Hide graph" : "Graph"}
            </button>
            {showGraph && (
              <button className="btn absolute top-10 right-2 px-1.5" title={dock.graph === "right" ? "Dock the graph under the viewport" : "Dock the graph beside the viewport"} onClick={() => setDock({ graph: dock.graph === "right" ? "bottom" : "right" })} data-testid="dock-graph">
                {dock.graph === "right" ? <PanelBottom size={14} /> : <PanelRight size={14} />}
              </button>
            )}
          </div>
          {showGraph && dock.graph === "bottom" && (
            <div ref={graphSize.ref} style={graphSize.style} className="h-[300px] max-h-[70vh] min-h-[160px] resize-y overflow-hidden border-t border-jd-line" title="Drag the bottom-right corner to resize the graph" data-testid="panel-graph">
              <GraphPanel />
            </div>
          )}
          {showGraph && dock.graph === "right" && (
            <div ref={graphWidth.ref} style={graphWidth.style} className="w-[460px] min-w-[280px] max-w-[60vw] resize-x overflow-hidden border-l border-jd-line" title="Drag the bottom-right corner to resize the graph" data-testid="panel-graph">
              <GraphPanel />
            </div>
          )}
          {!s.preview && (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-jd-fg-2">
              {s.project ? "Generate terrain to see it here" : "Create or open a project"}
            </div>
          )}
        </main>

        {/* Right: terrain inspector */}
        {dock.right ? (
          <aside className="flex w-6 flex-col items-center border-l border-jd-line bg-jd-bg-1 py-2" data-testid="panel-right-collapsed">
            <button className="btn px-1 py-0" title="Show the inspector" onClick={() => setDock({ right: false })} data-testid="expand-right">
              <PanelRightOpen size={14} />
            </button>
          </aside>
        ) : (
        <aside ref={rightSize.ref} style={rightSize.style} className="jd-panel relative m-2 ml-1 w-[280px] min-w-[220px] max-w-[560px] resize-x overflow-y-auto p-3" title="Drag the bottom-right corner to resize this panel" data-testid="panel-right">
          <button className="btn absolute top-2 right-2 px-1 py-0" title="Collapse the inspector to the edge" onClick={() => setDock({ right: true })} data-testid="collapse-right">
            <PanelRightClose size={14} />
          </button>
          {s.sculpting && (
            <>
              <h2 className="mb-2 jd-section">Brush</h2>
              <Field label="Mode">
                <select className={`${inputCls} w-36`} value={s.brush.mode} onChange={(e) => s.setBrush({ mode: e.target.value as BrushTool["mode"] })} data-testid="brush-mode">
                  <option value="raise">raise</option>
                  <option value="lower">lower</option>
                  <option value="smooth">smooth</option>
                  <option value="flatten">flatten</option>
                  <option value="rotate">rotate</option>
                  <option value="scale">scale</option>
                  <option value="tilt">tilt</option>
                  <option value="push">push / pull</option>
                  <option value="align">align</option>
                </select>
              </Field>
              {s.brush.mode === "rotate" && (
                <Field label="Angle (°)">
                  <input className={inputCls} type="number" step={5} value={s.brush.angle_deg} onChange={(e) => s.setBrush({ angle_deg: Number(e.target.value) })} title="Click (or drag) on the terrain: the footprint turns this much counter-clockwise about where you pressed" data-testid="brush-angle" />
                </Field>
              )}
              {s.brush.mode === "scale" && (
                <Field label="Factor">
                  <input className={inputCls} type="number" step={0.1} min={0} max={10} value={s.brush.factor} onChange={(e) => s.setBrush({ factor: Number(e.target.value) })} title="Relief about the footprint's mean: 0 levels it, 2 doubles it" data-testid="brush-factor" />
                </Field>
              )}
              {s.brush.mode === "tilt" && (
                <Field label="Tilt (°)">
                  <input className={inputCls} type="number" step={1} value={s.brush.tilt_deg} onChange={(e) => s.setBrush({ tilt_deg: Number(e.target.value) })} title="Drag on the terrain: the footprint tilts this much, rising in the drag's direction" data-testid="brush-tilt" />
                </Field>
              )}
              {(s.brush.mode === "rotate" || s.brush.mode === "scale" || s.brush.mode === "tilt" || s.brush.mode === "push" || s.brush.mode === "align") && (
                <p className="mb-2 text-[11px] text-jd-fg-2" data-testid="brush-xform-hint">
                  {s.brush.mode === "push" ? "Drag: the terrain under the brush slides with the drag (grab)." : s.brush.mode === "align" ? "Click: the footprint is levelled onto its own best-fit plane." : "Press where the footprint is centred; each drag is one transform in a jd.sculpt.transforms node."}
                </p>
              )}
              <div className="mb-2 grid grid-cols-7 gap-1" role="radiogroup" aria-label="Brush alpha" data-testid="brush-alphas">
                {(
                  [
                    ["plain", Circle, "Plain: the smooth round falloff"],
                    ["noise", Sparkles, "Noise: fractal value noise, 0.25..1 of the stroke"],
                    ["crater", CircleDot, "Crater: a raised rim round a sunken bowl"],
                    ["ridges", WavesVertical, "Ridges: parallel bands across the stroke"],
                    ["rocks", Gem, "Rocks: rounded rock bumps"],
                    ["dots", Grip, "Dots: scattered round dots"],
                    ["checker", Grid2x2, "Checker: full and quarter strength squares"],
                  ] as const
                ).map(([k, Icon, title]) => (
                  <button key={k} className={`flex aspect-square items-center justify-center rounded-md border ${s.brush.alpha === k ? "border-jd-accent bg-jd-bg-3 text-jd-fg-0" : "border-white/5 bg-jd-bg-2 text-jd-fg-2 hover:text-jd-fg-0"}`} title={title} aria-label={title} role="radio" aria-checked={s.brush.alpha === k} onClick={() => s.setBrush({ alpha: k })} data-testid={`alpha-${k}`}>
                    <Icon size={14} />
                  </button>
                ))}
              </div>
              <Field label="Radius (m)">
                <input className={inputCls} type="number" value={s.brush.radius_m} onChange={(e) => s.setBrush({ radius_m: Number(e.target.value) })} data-testid="brush-radius" />
              </Field>
              {s.brush.mode === "raise" || s.brush.mode === "lower" ? (
                <Field label="Strength (m)">
                  <input className={inputCls} type="number" value={s.brush.strength_m} onChange={(e) => s.setBrush({ strength_m: Number(e.target.value) })} data-testid="brush-strength" />
                </Field>
              ) : (
                <Field label="Amount (0–1)">
                  <input className={inputCls} type="number" step="0.1" min="0" max="1" value={s.brush.amount} onChange={(e) => s.setBrush({ amount: Number(e.target.value) })} data-testid="brush-amount" />
                </Field>
              )}
              {s.brush.mode === "flatten" && (
                <Field label="Height (m)">
                  <input className={inputCls} type="number" value={s.brush.target_m ?? ""} placeholder="under 1st point" onChange={(e) => s.setBrush({ target_m: e.target.value === "" ? null : Number(e.target.value) })} title="Flatten to this height; empty = the height under the first point of the stroke" data-testid="brush-target" />
                </Field>
              )}
              <Field label={`Hardness ${s.brush.hardness.toFixed(2)}`}>
                <input type="range" min={0} max={1} step={0.05} value={s.brush.hardness} onChange={(e) => s.setBrush({ hardness: Number(e.target.value) })} className="w-28" title="0 = soft falloff over the whole radius, 1 = hard edge" data-testid="brush-hardness" />
              </Field>
              <p className="mb-2 text-xs text-jd-fg-2">
                {s.brush.mode === "raise" ? "Drag on the terrain to raise it." : s.brush.mode === "lower" ? "Drag on the terrain to lower it." : s.brush.mode === "smooth" ? "Drag to blend heights towards their local mean." : "Drag to pull heights towards the target height."} Each stroke is one undo step.
              </p>
              {sculptNode && (
                <div className="mb-3 rounded-lg bg-jd-bg-2/60 p-2" data-testid="layer-settings">
                  <h3 className="jd-section mb-1">Layer settings</h3>
                  {(
                    [
                      ["height_scale", "Height scale", 1, 0.1, "Multiplies what the strokes changed (0 hides the layer)"],
                      ["height_offset_m", "Height offset (m)", 0, 1, "Added wherever the layer applies"],
                      ["blending", "Blending", 1, 0.05, "Opacity of the whole layer, 0..1"],
                      ["level_step_m", "Level step (m)", 0, 1, "Quantise the change into steps of this height (0 = off)"],
                    ] as const
                  ).map(([key, label, dflt, step, title]) => (
                    <Field key={key} label={label}>
                      <input className={inputCls} type="number" step={step} key={`${key}-${String(sculptNode.params?.[key] ?? dflt)}`} defaultValue={Number(sculptNode.params?.[key] ?? dflt)} title={title} onBlur={(e) => { const v = Number(e.target.value); if (v !== Number(sculptNode.params?.[key] ?? dflt)) void s.setParam(sculptNode.id, key, v); }} data-testid={`layer-${key}`} />
                    </Field>
                  ))}
                  <div className="mt-1 flex items-center justify-between text-[11px] text-jd-fg-2">
                    <span data-testid="layer-area">{sculptNode.params?.area ? "area: placed" : "area: whole terrain"}</span>
                    <button className="btn btn-primary" disabled={!sculptNode.params?.area} onClick={() => void s.setParam(sculptNode.id, "area", null)} title="Apply the layer over the whole terrain (clears the area; place one in the inspector)" data-testid="layer-fit">
                      Fit to terrain
                    </button>
                  </div>
                </div>
              )}
              <dl className="mb-4 grid grid-cols-[auto_1fr] gap-x-2 text-[11px] text-jd-fg-2" data-testid="brush-shortcuts">
                <dt>Left drag</dt><dd>stroke</dd>
                <dt>Right drag</dt><dd>orbit</dd>
                <dt>Ctrl + wheel</dt><dd>radius</dd>
                <dt>Shift + wheel</dt><dd>strength</dd>
              </dl>
            </>
          )}
          {s.grid.on && (
            <>
              <h2 className="mb-2 jd-section">Grid</h2>
              <Field label="Cells per side">
                <select className={`${inputCls} w-20`} value={s.grid.cells} onChange={(e) => { s.setGrid({ cells: Number(e.target.value) }); s.gridClear(); }} data-testid="grid-cells">
                  {[8, 16, 32, 64].map((n) => (
                    <option key={n} value={n}>{n}</option>
                  ))}
                </select>
              </Field>
              <Field label="Selected">
                <span className="text-xs" data-testid="grid-count">{s.grid.selected.length} cell{s.grid.selected.length === 1 ? "" : "s"}</span>
              </Field>
              <Field label={`Smoothness ${s.grid.smoothness.toFixed(2)}`}>
                <input type="range" min={0} max={1} step={0.05} value={s.grid.smoothness} onChange={(e) => s.setGrid({ smoothness: Number(e.target.value) })} className="w-28" title="How far the selection's edge blends into its surroundings (0 = hard cell edges)" data-testid="grid-smoothness" />
              </Field>
              <Field label="Height (m)">
                <input className={inputCls} type="number" value={s.grid.height_m} onChange={(e) => s.setGrid({ height_m: Number(e.target.value) })} data-testid="grid-height" />
              </Field>
              <div className="mb-2 flex gap-2">
                <button className="btn" onClick={() => void s.gridApply("set")} disabled={s.grid.selected.length === 0 || s.busy} title="Level the selected cells to the height above (one undoable stroke)" data-testid="grid-set">Set height</button>
                <button className="btn" onClick={() => s.gridClear()} disabled={s.grid.selected.length === 0} data-testid="grid-clear">Clear</button>
              </div>
              <Field label="Step (m)">
                <input className={inputCls} type="number" value={s.grid.step_m} onChange={(e) => s.setGrid({ step_m: Number(e.target.value) })} data-testid="grid-step" />
              </Field>
              <div className="mb-2 flex gap-2">
                <button className="btn" onClick={() => void s.gridApply("raise")} disabled={s.grid.selected.length === 0 || s.busy} data-testid="grid-raise">Raise</button>
                <button className="btn" onClick={() => void s.gridApply("lower")} disabled={s.grid.selected.length === 0 || s.busy} data-testid="grid-lower">Lower</button>
              </div>
              <h3 className="mb-1 text-[11px] font-semibold text-jd-fg-1">Transform selection</h3>
              <div className="mb-1 grid grid-cols-[auto_1fr_auto_1fr] items-center gap-1 text-[11px] text-jd-fg-2">
                <span>Angle</span>
                <input className={`${inputCls} w-full`} type="number" step={5} value={s.grid.angle_deg} onChange={(e) => s.setGrid({ angle_deg: Number(e.target.value) })} data-testid="grid-angle" />
                <span>Factor</span>
                <input className={`${inputCls} w-full`} type="number" step={0.1} value={s.grid.factor} onChange={(e) => s.setGrid({ factor: Number(e.target.value) })} data-testid="grid-factor" />
                <span>Tilt</span>
                <input className={`${inputCls} w-full`} type="number" step={1} value={s.grid.tilt_deg} onChange={(e) => s.setGrid({ tilt_deg: Number(e.target.value) })} data-testid="grid-tilt" />
                <span>Toward</span>
                <input className={`${inputCls} w-full`} type="number" step={15} value={s.grid.tilt_azimuth_deg} onChange={(e) => s.setGrid({ tilt_azimuth_deg: Number(e.target.value) })} title="Direction the tilt rises toward, degrees clockwise from north" data-testid="grid-tilt-azimuth" />
              </div>
              <div className="mb-2 flex flex-wrap gap-1" data-testid="grid-transforms">
                {(
                  [
                    ["duplicate", "Duplicate", "Copy the selected block one selection-width east"],
                    ["rotate", "Rotate", "Turn the selected block by Angle (counter-clockwise)"],
                    ["scale", "Scale", "Scale the block's relief about its mean by Factor"],
                    ["tilt", "Tilt", "Tilt the block by Tilt degrees, rising toward Toward"],
                    ["align", "Align", "Level the block onto its own best-fit plane"],
                  ] as const
                ).map(([k, label, title]) => (
                  <button key={k} className="btn" onClick={() => void s.gridTransform(k)} disabled={s.grid.selected.length === 0 || s.busy} title={`${title} — appended to a jd.sculpt.transforms node; Raise / Lower above are World Creator's Push / Pull`} data-testid={`grid-${k}`}>
                    {label}
                  </button>
                ))}
              </div>
              <dl className="mb-4 grid grid-cols-[auto_1fr] gap-x-2 text-[11px] text-jd-fg-2" data-testid="grid-shortcuts">
                <dt>Click</dt><dd>toggle a cell</dd>
                <dt>Left drag</dt><dd>add cells</dd>
                <dt>Alt + drag</dt><dd>remove cells</dd>
                <dt>Right drag</dt><dd>orbit</dd>
              </dl>
            </>
          )}
          <h2 className="mb-2 jd-section">Sun</h2>
          <Field label="Textured (PBR)">
            <input type="checkbox" checked={s.pbr} onChange={(e) => s.setPbr(e.target.checked)} title="Shade the terrain with baked albedo, normal and roughness maps (full-field relief at any preview LOD); off = height-ramp vertex colours" data-testid="pbr-toggle" />
          </Field>
          <Field label={`Azimuth ${s.sun.azimuth_deg}°`}>
            <input className="w-28" type="range" min={0} max={359} value={s.sun.azimuth_deg} onChange={(e) => s.setSun({ azimuth_deg: Number(e.target.value) })} title="Direction the light comes from, clockwise from north" data-testid="sun-azimuth" />
          </Field>
          <Field label={`Elevation ${s.sun.elevation_deg}°`}>
            <input className="w-28" type="range" min={3} max={90} value={s.sun.elevation_deg} onChange={(e) => s.setSun({ elevation_deg: Number(e.target.value) })} title="Height of the sun above the horizon; low sun is warm and long-shadowed" data-testid="sun-elevation" />
          </Field>
          <Field label={`Turbidity ${s.sun.turbidity}`}>
            <input className="w-28" type="range" min={2} max={10} step={0.5} value={s.sun.turbidity} onChange={(e) => s.setSun({ turbidity: Number(e.target.value) })} title="Atmospheric haze for the sky dome: 2 clear mountain air, 3 clear, 6 hazy, 10 thick haze (Preetham daylight model)" data-testid="sun-turbidity" />
          </Field>
          <Field label="HDRI">
            <span className="flex gap-1">
              <input className={`${inputCls} w-24`} value={hdriPath} onChange={(e) => setHdriPath(e.target.value)} title="An equirectangular EXR — sky.bake's output or any 2:1 HDRI — in the project's assets/ (import it with ⌘I) or an absolute path; drawn on the dome instead of the physical sky" data-testid="hdri-path" />
              <button className="btn px-1.5" disabled={!s.project || s.busy || !hdriPath.trim()} onClick={() => void s.loadHdri(hdriPath.trim())} title="Load the HDRI onto the dome (sky.load_hdri)" data-testid="hdri-load">
                Load
              </button>
              <button className="btn px-1.5" disabled={!s.hdri} onClick={() => s.setSun({ azimuth_deg: Math.round(s.hdri!.sun.azimuth_deg), elevation_deg: Math.max(3, Math.min(90, Math.round(s.hdri!.sun.elevation_deg))) })} title="Point the sun where the HDRI is brightest" data-testid="hdri-sun">
                Use sun
              </button>
              <button className="btn px-1.5" disabled={!s.hdri} onClick={() => s.clearHdri()} title="Back to the physical sky" data-testid="hdri-clear">
                ×
              </button>
            </span>
          </Field>
          <Field label={`Exposure ${s.post.exposure}`}>
            <input className="w-28" type="range" min={0.25} max={4} step={0.05} value={s.post.exposure} onChange={(e) => s.setPost({ exposure: Number(e.target.value) })} title="Tone-mapping exposure (ACES while any effect is on); 1 = the plain render" data-testid="post-exposure" />
          </Field>
          <Field label={`Bloom ${s.post.bloom}`}>
            <input className="w-28" type="range" min={0} max={1} step={0.05} value={s.post.bloom} onChange={(e) => s.setPost({ bloom: Number(e.target.value) })} title="Bloom strength on the bright parts (sun glints, lava glow); 0 = off" data-testid="post-bloom" />
          </Field>
          <Field label={`Vignette ${s.post.vignette}`}>
            <input className="w-28" type="range" min={0} max={1} step={0.05} value={s.post.vignette} onChange={(e) => s.setPost({ vignette: Number(e.target.value) })} title="Darken the frame's corners; 0 = off" data-testid="post-vignette" />
          </Field>
          <Field label="Camera">
            <span className="flex flex-wrap gap-1">
              <button className="btn px-1.5" disabled={!s.viewport} onClick={() => s.addFlyKey()} title="Add the current view as a flythrough key" data-testid="fly-add">
                + key
              </button>
              <span className="text-xs text-jd-fg-2" data-testid="fly-count">{s.flyKeys.length} keys</span>
              <button className="btn px-1.5" disabled={s.flyKeys.length === 0} onClick={() => s.clearFlyKeys()} title="Drop the flythrough keys" data-testid="fly-clear">
                ×
              </button>
              <input className={`${inputCls} w-14`} type="number" min={2} max={2400} value={flyFrames} onChange={(e) => setFlyFrames(Math.max(2, Number(e.target.value) || 2))} title="Frames to render along the keys" data-testid="fly-frames" />
              <button className="btn px-1.5" disabled={!s.viewport || s.flyKeys.length < 2 || s.busy} onClick={() => void s.recordFlythrough(flyFrames)} title="Render the frames along the keys into exports/flythrough/ (then: ffmpeg -framerate 24 -i frame_%04d.png flythrough.mp4)" data-testid="fly-record">
                Record
              </button>
              <button className="btn px-1.5" disabled={!s.viewport || !s.project} onClick={() => void s.saveStill()} title="Save the current frame as exports/stills/view_<n>.png" data-testid="still-save">
                Still
              </button>
            </span>
          </Field>
          <Field label={`Clouds ${Math.round(s.sun.clouds * 100)} %`}>
            <input className="w-28" type="range" min={0} max={1} step={0.05} value={s.sun.clouds} onChange={(e) => s.setSun({ clouds: Number(e.target.value) })} title="Cloud cover on the sky dome and in the HDRI bake: a seeded cumulus-like noise layer, brighter on the sun side" data-testid="sky-clouds" />
          </Field>
          <Field label={`Fog ${Math.round(s.sun.fog * 100)} %`}>
            <input className="w-28" type="range" min={0} max={1} step={0.05} value={s.sun.fog} onChange={(e) => s.setSun({ fog: Number(e.target.value) })} title="Viewport fog: 0 keeps the faint aerial perspective, 1 closes the view within the world's width (a look setting — bakes and exports are untouched)" data-testid="sky-fog" />
          </Field>
          <h2 className="mt-4 mb-2 jd-section">Scatter</h2>
          <Field label="Mask">
            <select className={inputCls} value={s.scatter.mask ?? ""} onChange={(e) => s.setScatter({ mask: e.target.value || null })} title="0..1 layer used as the instance density (a class mask, rivers, wetness…)" data-testid="scatter-mask">
              <option value="">none</option>
              {Object.keys(s.lastEval?.layers ?? {}).map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Spacing (m)">
            <input className={inputCls} type="number" min={1} value={s.scatter.spacing_m} onChange={(e) => s.setScatter({ spacing_m: Number(e.target.value) })} data-testid="scatter-spacing" />
          </Field>
          <Field label="Seed">
            <input className={inputCls} type="number" value={s.scatter.seed} onChange={(e) => s.setScatter({ seed: Number(e.target.value) })} data-testid="scatter-seed" />
          </Field>
          <Field label="Grow with timeline">
            <input type="checkbox" checked={s.scatter.grow} onChange={(e) => s.setScatter({ grow: e.target.checked })} title="The population follows the timeline: nobody at t = 0, saplings as it advances, full-grown by t = 1; off = always full-grown" data-testid="scatter-grow" />
          </Field>
          <div className="flex items-center gap-2 py-1">
            <button className="btn" onClick={() => void s.placeScatter()} disabled={!s.scatter.mask || s.busy} title="Place instances from the mask (one jittered candidate per spacing cell, kept with probability = mask value)" data-testid="scatter-place">
              Place
            </button>
            <button className="btn" onClick={() => void s.paintDensity()} disabled={!s.scatter.mask || s.busy} title="Paint the density by hand: a clamped stroke node goes in front of the mask's output; drag on the terrain to add density (strength 0.5), negative strength to thin it" data-testid="scatter-paint">
              Paint density
            </button>
            <input className={`${inputCls} w-24`} value={scatterModel} onChange={(e) => setScatterModel(e.target.value)} placeholder="pine.fbx" title="Your own model to instance: a binary FBX in the project's assets/ (metres, +Y up; import it with ⌘I). Empty = the 1 m proxy, or a generated rock for rock-named masks" data-testid="scatter-model" />
            <button className="btn" onClick={() => void s.exportScatter(scatterModel.trim() || undefined)} disabled={!s.scatter.points?.length || s.busy} title="Write exports/<mask>.glb: the model (or the 1 m proxy) instanced with EXT_mesh_gpu_instancing" data-testid="scatter-export">
              Export .glb
            </button>
            <span className="text-xs text-jd-fg-2" data-testid="scatter-count">{s.scatter.points ? `${s.scatter.points.length.toLocaleString()} instances` : ""}</span>
          </div>
          <h2 className="mt-4 mb-2 jd-section">Actions</h2>
          <div className="mb-1 flex flex-wrap gap-1" data-testid="terrain-actions">
            {(
              [
                ["flip_x", "Flip X", "Mirror the terrain east ↔ west"],
                ["flip_y", "Flip Y", "Mirror the terrain north ↔ south"],
                ["rotate_left", "Rot ⟲", "Turn the terrain 90° counter-clockwise (square worlds)"],
                ["rotate_right", "Rot ⟳", "Turn the terrain 90° clockwise (square worlds)"],
                ["flatten", "Flatten", "Flatten the terrain to its mean height (set height_m in the inspector)"],
              ] as const
            ).map(([k, label, title]) => (
              <button key={k} className="btn" onClick={() => void s.applyAction(k)} disabled={!s.project || s.busy} title={`${title} — a jd.filter.action node in front of the output; undo or bypass it in the layer stack`} data-testid={`action-${k}`}>
                {label}
              </button>
            ))}
          </div>
          <h2 className="mt-4 mb-2 jd-section">Simulation layers</h2>
          <div className="mb-1 flex flex-wrap gap-1">
            {(["snow", "debris", "sand", "lava"] as const).map((k) => (
              <button
                key={k}
                className="btn"
                onClick={() => void s.addSimulation(k)}
                disabled={!s.project || s.busy}
                title={{ snow: "A season of snow on the terrain (jd.climate.snowpack): the cover becomes the `snow` layer, a snow material and the viewport colour", debris: "Rockfall debris (jd.erosion.debris) in front of the height output: boulder aprons, the `debris` layer and a rock material", sand: "Wind-blown sand (jd.erosion.aeolian) in front of the height output: dunes along the world wind", lava: "A lava flow from the summit (jd.volcanic.lava) in front of the height output: the `heat` glow and `lava` thickness layers, a basalt material" }[k]}
                data-testid={`sim-${k}`}
              >
                + {k}
              </button>
            ))}
          </div>
          <p className="mb-2 text-xs text-jd-fg-2">One click adds the node, publishes its masks as layers and a material; tune it in the graph inspector.</p>
          <h2 className="mt-4 mb-2 jd-section">Materials</h2>
          {s.materials.map((m, i) => (
            <div className="mb-1 flex flex-wrap items-center gap-1" key={i} data-testid={`material-${i}`}>
              <select
                className={`${inputCls} w-24 text-left`}
                value={m.mask ?? ""}
                onChange={(e) => s.setMaterials(s.materials.map((x, k) => (k === i ? { ...x, mask: e.target.value || null } : x)))}
                title="0..1 mask layer that weights this material (everywhere when empty)"
                data-testid={`material-mask-${i}`}
              >
                <option value="">everywhere</option>
                {Object.keys(s.lastEval?.layers ?? {}).map((n) => (
                  <option key={n}>{n}</option>
                ))}
              </select>
              <input
                type="color"
                value={hexOf(m.albedo)}
                onChange={(e) => s.setMaterials(s.materials.map((x, k) => (k === i ? { ...x, albedo: rgbOf(e.target.value) } : x)))}
                title="Base colour"
                data-testid={`material-color-${i}`}
              />
              <input
                type="color"
                value={hexOf(m.emissive ?? [0, 0, 0])}
                onChange={(e) => {
                  const c = rgbOf(e.target.value);
                  s.setMaterials(s.materials.map((x, k) => (k === i ? { ...x, emissive: c.every((v) => v === 0) ? null : c } : x)));
                }}
                title="Emitted light (black = none): glows in the viewport and the GLB, listed for the engines"
                data-testid={`material-emissive-${i}`}
              />
              <input
                className={`${inputCls} w-14`}
                type="number"
                min={0}
                max={1}
                step={0.05}
                value={m.roughness ?? 0.9}
                onChange={(e) => s.setMaterials(s.materials.map((x, k) => (k === i ? { ...x, roughness: Number(e.target.value) } : x)))}
                title="Roughness 0..1"
                data-testid={`material-rough-${i}`}
              />
              <input
                type="checkbox"
                checked={!!m.detail}
                onChange={(e) => s.setMaterials(s.materials.map((x, k) => (k === i ? { ...x, detail: e.target.checked ? { tile_m: 8, bump: 0.5, contrast: 0.3, seed: i, px: 512 } : null } : x)))}
                title="Tileable detail: a seamless grain tile repeated every tile_m metres, blended by this layer's mask in the viewport, baked into exported maps by world position, written as tiles for Unity and Unreal"
                data-testid={`material-detail-${i}`}
              />
              {m.detail && (
                <input
                  className={`${inputCls} w-14`}
                  type="number"
                  min={0.1}
                  step={1}
                  key={`tile-${i}-${m.detail.tile_m}`}
                  defaultValue={m.detail.tile_m}
                  onBlur={(e) => {
                    const v = Number(e.target.value);
                    if (v > 0 && v !== m.detail?.tile_m) s.setMaterials(s.materials.map((x, k) => (k === i && x.detail ? { ...x, detail: { ...x.detail, tile_m: v } } : x)));
                  }}
                  title="Metres per repeat of the detail tile"
                  data-testid={`material-tile-${i}`}
                />
              )}
              <button className="btn px-1.5" onClick={() => s.setMaterials(s.materials.filter((_, k) => k !== i))} title="Remove this layer" data-testid={`material-remove-${i}`}>
                ×
              </button>
            </div>
          ))}
          <button
            className="btn"
            onClick={() => s.setMaterials([...s.materials, { mask: null, albedo: [0.5, 0.5, 0.5], roughness: 0.9 }])}
            disabled={!s.project}
            title="Add a material layer: the stack blends albedo and roughness over the base, each layer by its mask, in order (terrain.textures / export materials)"
            data-testid="material-add"
          >
            + layer
          </button>
          <h2 className="mt-4 mb-2 jd-section">Terrain · Noise</h2>
          <Field label="Preset">
            <select
              className={`${inputCls} w-36`}
              value=""
              onChange={(e) => {
                const p = NOISE_PRESETS.find((x) => x.label === e.target.value);
                if (p) s.setNoise(p.patch);
              }}
              title="Fills the noise fields below; a desert plain is the start for dunes (jd.erosion.aeolian@1) and sinkholes (jd.erosion.karst@1)"
              data-testid="noise-preset"
            >
              <option value="">choose…</option>
              {NOISE_PRESETS.map((p) => (
                <option key={p.label}>{p.label}</option>
              ))}
            </select>
          </Field>
          <Field label="Lattice">
            <select className={inputCls} value={s.noise.kind} onChange={(e) => s.setNoise({ kind: e.target.value as NoiseKind })}>
              {["open_simplex2", "open_simplex2s", "perlin", "value", "value_cubic", "cellular"].map((k) => (
                <option key={k}>{k}</option>
              ))}
            </select>
          </Field>
          <Field label="Fractal">
            <select className={inputCls} value={s.noise.fractal} onChange={(e) => s.setNoise({ fractal: e.target.value as FractalKind })}>
              {["none", "fbm", "ridged", "ping_pong"].map((k) => (
                <option key={k}>{k}</option>
              ))}
            </select>
          </Field>
          <Field label="Seed">
            <input className={inputCls} type="number" value={s.noise.seed} onChange={(e) => s.setNoise({ seed: Number(e.target.value) })} data-testid="seed" />
          </Field>
          <Field label="Feature size (m)">
            <input className={inputCls} type="number" value={s.noise.feature_size_m} onChange={(e) => s.setNoise({ feature_size_m: Number(e.target.value) })} />
          </Field>
          <Field label="Octaves">
            <input className={inputCls} type="number" min={1} max={16} value={s.noise.octaves} onChange={(e) => s.setNoise({ octaves: Number(e.target.value) })} />
          </Field>
          <Field label="Lacunarity">
            <input className={inputCls} type="number" step={0.1} value={s.noise.lacunarity} onChange={(e) => s.setNoise({ lacunarity: Number(e.target.value) })} />
          </Field>
          <Field label="Gain">
            <input className={inputCls} type="number" step={0.05} value={s.noise.gain} onChange={(e) => s.setNoise({ gain: Number(e.target.value) })} />
          </Field>
          {noiseNode && (
            <>
              <Field label={`Strength ${Number(noiseNode.params?.strength ?? 1).toFixed(2)}`}>
                <input className="w-28" type="range" min={0} max={2} step={0.05} defaultValue={Number(noiseNode.params?.strength ?? 1)} key={`str-${noiseNode.id}-${String(noiseNode.params?.strength ?? 1)}`} onPointerUp={(e) => void s.setParam(noiseNode.id, "strength", Number((e.target as HTMLInputElement).value))} onKeyUp={(e) => void s.setParam(noiseNode.id, "strength", Number((e.target as HTMLInputElement).value))} title="General strength: scales the relief around the middle of the height range (World Creator's General Strength)" data-testid="noise-strength" />
              </Field>
              <div className="mb-2" data-testid="octave-bars" title="Strength per octave, coarsest first (fbm): drag a bar and release to apply; double-click to reset all to 1">
                <div className="mb-1 flex items-center justify-between text-xs text-jd-fg-1">
                  <span>Octaves</span>
                  <button className="text-[10px] text-jd-fg-2 hover:text-jd-fg-0" onClick={() => void s.setParam(noiseNode.id, "octave_weights", null)} data-testid="octave-reset">
                    reset
                  </button>
                </div>
                <div className="flex h-20 items-end gap-1">
                  {Array.from({ length: s.noise.octaves }, (_, i) => {
                    const ws = Array.isArray(noiseNode.params?.octave_weights) ? (noiseNode.params!.octave_weights as number[]) : [];
                    const w = ws[i] ?? 1;
                    return (
                      <label key={i} className="flex h-full flex-1 flex-col items-center gap-0.5">
                        <input
                          type="range"
                          min={0}
                          max={2}
                          step={0.05}
                          defaultValue={w}
                          key={`oct-${i}-${w}`}
                          className="h-full w-3 [writing-mode:vertical-lr] [direction:rtl]"
                          disabled={s.noise.fractal !== "fbm"}
                          onPointerUp={(e) => {
                            const next = Array.from({ length: s.noise.octaves }, (_, k) => (k === i ? Number((e.target as HTMLInputElement).value) : (ws[k] ?? 1)));
                            void s.setParam(noiseNode.id, "octave_weights", next);
                          }}
                          aria-label={`octave ${i + 1} strength`}
                          data-testid={`octave-${i}`}
                        />
                        <span className="text-[9px] text-jd-fg-2">{i + 1}</span>
                      </label>
                    );
                  })}
                </div>
              </div>
            </>
          )}
          <Field label="Height min / max (m)">
            <span className="flex gap-1">
              <input className={`${inputCls} w-14`} type="number" value={s.noise.height_range_m[0]} onChange={(e) => s.setNoise({ height_range_m: [Number(e.target.value), s.noise.height_range_m[1]] })} />
              <input className={`${inputCls} w-14`} type="number" value={s.noise.height_range_m[1]} onChange={(e) => s.setNoise({ height_range_m: [s.noise.height_range_m[0], Number(e.target.value)] })} />
            </span>
          </Field>
          <button className="btn btn-primary mt-2 w-full justify-center" onClick={() => s.generate()} disabled={!s.project || s.busy} data-testid="generate">
            <Mountain size={14} /> Generate
          </button>
          {s.layer && (
            <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-2 gap-y-1 text-xs text-jd-fg-2" data-testid="layer-stats">
              <dt>min / max</dt>
              <dd>
                {s.layer.stats.min.toFixed(1)} / {s.layer.stats.max.toFixed(1)} m
              </dd>
              <dt>mean</dt>
              <dd>{s.layer.stats.mean.toFixed(1)} m</dd>
              <dt>time</dt>
              <dd>{s.layer.duration_ms} ms</dd>
              <dt>hash</dt>
              <dd className="truncate font-mono">{s.layer.content_hash.slice(0, 16)}…</dd>
            </dl>
          )}
          {s.layer && (
            <div className="mt-2">
              <button className="btn" onClick={() => void s.measureLayer()} disabled={s.busy} title="terrain.measure: relief, hypsometric integral, slope statistics and geomorphon shares — compare with a real DEM of the same landform (Reading a landscape)" data-testid="measure">
                Measure landform
              </button>
              {s.measure && (
                <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-2 gap-y-1 text-xs text-jd-fg-2" data-testid="measure-readout">
                  <dt>relief</dt>
                  <dd>{s.measure.relief_m.toFixed(0)} m</dd>
                  <dt>hypsometric</dt>
                  <dd>{s.measure.hypsometric_integral.toFixed(2)} {s.measure.hypsometric_integral < 0.45 ? "(dissected, old)" : s.measure.hypsometric_integral > 0.55 ? "(plateau-like, young)" : ""}</dd>
                  <dt>slope</dt>
                  <dd>mean {s.measure.slope.mean_deg.toFixed(1)}°, median {s.measure.slope.p50_deg.toFixed(1)}°, p90 {s.measure.slope.p90_deg.toFixed(1)}°</dd>
                  <dt>landforms</dt>
                  <dd>
                    {Object.entries(s.measure.geomorphons)
                      .sort((a, b) => b[1] - a[1])
                      .slice(0, 5)
                      .map(([k, v]) => `${k} ${(v * 100).toFixed(0)} %`)
                      .join(", ")}
                  </dd>
                </dl>
              )}
            </div>
          )}
          <h2 className="mt-5 mb-2 jd-section">Export</h2>
          <Field label="File">
            <span className="flex gap-1">
              <input className={`${inputCls} w-28`} title="Relative paths go to the project's exports/ directory" value={exportPath} onChange={(e) => setExportPath(e.target.value)} data-testid="export-path" />
              <button className="btn px-1.5" title="Choose where to save… (⌘E)" onClick={() => void pickExportPath()} disabled={!s.layer || s.busy} data-testid="browse-export">
                <FolderSearch size={14} />
              </button>
            </span>
          </Field>
          <button
            className="btn mt-1 w-full justify-center"
            disabled={!s.layer || s.busy}
            onClick={() => s.exportHeightmap(exportPath, formatFor(exportPath))}
            data-testid="export"
          >
            <Upload size={14} /> Export heightmap
          </button>
          <Field label="Holes">
            <select className={inputCls} value={holesLayer} onChange={(e) => setHolesLayer(e.target.value)} title="A 0..1 layer (a painted mask, a class mask) whose cells ≥ 0.5 are holes: left out of the mesh exports, written as holes.png for the engines (Unity SetHoles, Unreal Visibility)" data-testid="export-holes">
              <option value="">none</option>
              {Object.keys(s.lastEval?.layers ?? {})
                .filter((n) => !n.includes("#"))
                .map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
            </select>
          </Field>
          <Field label="Engine">
            <span className="flex gap-1">
              <select className={`${inputCls} w-20`} value={engine} onChange={(e) => setEngine(e.target.value as "unity" | "unreal")} title="Unity Terrain (RAW 16-bit + weightmaps) or Unreal Landscape (PNG16 + weightmaps), with the material stack as materials.json" data-testid="export-engine">
                <option value="unity">Unity</option>
                <option value="unreal">Unreal</option>
              </select>
              <input className={`${inputCls} w-12`} type="number" min={1} max={16} value={tiles} onChange={(e) => setTiles(Math.max(1, Math.min(16, Number(e.target.value) || 1)))} title="Tiles per side: 1 writes one folder; 2–16 cut the world into tiles × tiles engine folders (tile_<x>_<y>/) sharing one height range, with tiles.json — the resolution must divide" data-testid="export-tiles" />
              <button className="btn" disabled={!s.layer || s.busy} onClick={() => void s.exportProfile(engine, engine, tiles, holesLayer || null)} title="Write exports/<engine>/: heightmap, one weightmap per material mask, RGBA splat maps, README with the import numbers, materials.json" data-testid="export-profile">
                <Upload size={14} /> Export
              </button>
            </span>
          </Field>
          <Field label="Mesh">
            <span className="flex gap-1">
              <input className={`${inputCls} w-24`} value={meshPath} onChange={(e) => setMeshPath(e.target.value)} title="exports/<file>: .glb (materials baked into its textures), .fbx, .obj, .stl, .ply, .usda, .usdz" data-testid="export-mesh-path" />
              <button className="btn" disabled={!s.layer || s.busy} onClick={() => void s.exportMesh(meshPath, null, null, holesLayer || null)} data-testid="export-mesh">
                <Upload size={14} /> Export
              </button>
            </span>
          </Field>
          <Field label="HDRI">
            <span className="flex gap-1">
              <input className={`${inputCls} w-24`} value={skyPath} onChange={(e) => setSkyPath(e.target.value)} title="exports/<file>.exr: the viewport's sun, turbidity and clouds as an equirectangular HDRI (a .png preview beside it)" data-testid="export-sky-path" />
              <button className="btn" disabled={!s.project || s.busy} onClick={() => void s.exportSky(skyPath)} data-testid="export-sky">
                <Upload size={14} /> Bake
              </button>
            </span>
          </Field>
          <button
            className="btn mt-1 w-full justify-center"
            disabled={!seaNode || s.busy}
            onClick={() => void s.exportWater()}
            title="Write exports/water/: depth, colour, foam and a tileable wave normal map for the sea-level node's clarity and wind"
            data-testid="water-export"
          >
            <Upload size={14} /> Export water maps
          </button>
          <div className="mt-1 flex gap-1">
            <input className={`${inputCls} w-16`} type="number" min={2} max={10000} value={frames} onChange={(e) => setFrames(Number(e.target.value))} title="Frames across the timeline (t = 0 … 1)" data-testid="frames-count" />
            <button
              className="btn flex-1 justify-center"
              disabled={!s.project || s.busy}
              onClick={() => void s.exportFrames(frames)}
              title="Write exports/frames/: one 16-bit heightmap per frame from t = 0 to 1, keyframed parameters animating (timeline.export)"
              data-testid="frames-export"
            >
              <Upload size={14} /> Export frames
            </button>
          </div>
        </aside>
        )}
      </div>

      {/* Bottom: log */}
      <section className="jd-panel mx-2 mb-2 overflow-y-auto px-3 py-2 font-mono text-xs" data-testid="log">
        {s.log.length === 0 && <span className="text-jd-fg-2">Operations and warnings appear here.</span>}
        {s.log.map((e, i) => (
          <div key={i} className={e.level === "error" ? "text-jd-error" : e.level === "warn" ? "text-jd-warn" : "text-jd-fg-1"}>
            {new Date(e.ts).toLocaleTimeString()} {e.text}
          </div>
        ))}
      </section>
    </div>
  );
}
