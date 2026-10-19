import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, FolderPlus, FolderSearch, Mountain, Save, Upload, Workflow } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import GraphPanel from "./graph/GraphPanel";
import TerrainView, { type ViewportInfo } from "./viewport/TerrainView";
import { useStore } from "./state";

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex items-center justify-between gap-2 py-1 text-jd-fg-1">
      <span className="text-xs">{label}</span>
      {children}
    </label>
  );
}

const inputCls = "w-28 rounded border border-jd-line bg-jd-bg-2 px-2 py-0.5 text-right text-xs text-jd-fg-0 focus:border-jd-accent focus:outline-none";

export function formatFor(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase();
  return ext === "png" ? "png16" : ext === "r16" ? "raw_r16" : ext === "r32" ? "raw_r32" : "exr";
}

export default function App() {
  const s = useStore();
  const [projPath, setProjPath] = useState("");
  const [projName, setProjName] = useState("World");
  const [extent, setExtent] = useState(4096);
  const [resolution, setResolution] = useState(2048);
  const [exportPath, setExportPath] = useState("height.exr");
  const [vp, setVp] = useState<ViewportInfo>({ backend: "none", triangles: 0 });
  const [showGraph, setShowGraph] = useState(true);
  const onInfo = useCallback((i: ViewportInfo) => setVp(i), []);
  const pathInput = useRef<HTMLInputElement>(null);

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
  const pickExportPath = useCallback(async () => {
    const p = await saveDialog({
      title: "Export heightmap",
      defaultPath: exportPath,
      filters: [
        { name: "OpenEXR (32-bit float)", extensions: ["exr"] },
        { name: "PNG (16-bit)", extensions: ["png"] },
        { name: "RAW 16-bit", extensions: ["r16"] },
        { name: "RAW 32-bit float", extensions: ["r32"] },
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

  // Native menu bar → the same actions as the buttons (ids from src-tauri build_menu).
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
  }, [pickProjectToCreate, pickProjectToOpen, pickExportPath]);

  return (
    <div className="grid h-full grid-rows-[40px_1fr_160px] bg-jd-bg-0 text-jd-fg-0">
      {/* Top bar */}
      <header className="flex items-center gap-3 border-b border-jd-line bg-jd-bg-1 px-3">
        <Mountain size={16} className="text-jd-accent" />
        <span className="font-semibold tracking-wide">Jenga Dunia</span>
        <span className="text-xs text-jd-fg-2" data-testid="app-version">v{__JD_VERSION__}</span>
        <span className="text-xs text-jd-fg-2">{s.project ? `${s.project.name} — ${s.project.world.resolution_px[0]}² · ${s.project.world.extent_m[0]} m` : "no project"}</span>
        {s.lastEval && Object.keys(s.lastEval.layers).length > 1 && (
          <label className="ml-auto flex items-center gap-1 text-xs text-jd-fg-2">
            layer
            <select className="rounded border border-jd-line bg-jd-bg-2 px-1 text-xs text-jd-fg-0" value={s.layer?.layer ?? ""} onChange={(e) => void s.viewLayer(e.target.value)} data-testid="layer-pick">
              {Object.keys(s.lastEval.layers).map((n) => (
                <option key={n}>{n}</option>
              ))}
            </select>
          </label>
        )}
        <span className={`${s.lastEval && Object.keys(s.lastEval.layers).length > 1 ? "" : "ml-auto "}text-xs text-jd-fg-2`} data-testid="viewport-info">
          viewport: {vp.backend} · {vp.triangles.toLocaleString()} tris
          {s.previewStats ? ` · preview lod ${s.previewStats.lod}, ${(s.previewStats.bytes / 1048576).toFixed(2)} MB in ${s.previewStats.ms.toFixed(0)} ms` : ""}
        </span>
        {s.busy && <span className="text-xs text-jd-accent">working…</span>}
      </header>

      <div className="grid grid-cols-[260px_1fr_280px] overflow-hidden">
        {/* Left: project */}
        <aside className="overflow-y-auto border-r border-jd-line bg-jd-bg-1 p-3">
          <h2 className="mb-2 text-xs font-semibold uppercase tracking-wider text-jd-fg-2">Project</h2>
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
          <div className="mt-2 flex gap-2">
            <button className="btn" title="Create at the path above (⌘N opens a dialog)" onClick={() => s.newProject(projPath, projName, extent, resolution)} disabled={!projPath || s.busy} data-testid="new-project">
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
            <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-2 gap-y-1 text-xs text-jd-fg-2">
              <dt>id</dt>
              <dd className="truncate font-mono text-jd-fg-1">{s.project.project_id}</dd>
              <dt>dirty</dt>
              <dd>{String(s.project.dirty)}</dd>
              <dt>journal</dt>
              <dd>{s.project.pending_journal}</dd>
            </dl>
          )}
        </aside>

        {/* Center: viewport (top) + node graph (bottom, toggle) */}
        <main className={`relative grid bg-jd-bg-0 ${showGraph ? "grid-rows-[1fr_300px]" : "grid-rows-[1fr]"}`}>
          <div className="relative min-h-0">
            <TerrainView preview={s.preview} extentM={s.project ? s.project.world.extent_m : null} onInfo={onInfo} />
            <button className="btn absolute top-2 right-2" onClick={() => setShowGraph((v) => !v)} data-testid="toggle-graph">
              <Workflow size={14} /> {showGraph ? "Hide graph" : "Graph"}
            </button>
          </div>
          {showGraph && (
            <div className="min-h-0 border-t border-jd-line">
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
        <aside className="overflow-y-auto border-l border-jd-line bg-jd-bg-1 p-3">
          <h2 className="mb-2 text-xs font-semibold uppercase tracking-wider text-jd-fg-2">Terrain · Noise</h2>
          <Field label="Lattice">
            <select className={inputCls} value={s.noise.kind} onChange={(e) => s.setNoise({ kind: e.target.value })}>
              {["open_simplex2", "open_simplex2s", "perlin", "value", "value_cubic", "cellular"].map((k) => (
                <option key={k}>{k}</option>
              ))}
            </select>
          </Field>
          <Field label="Fractal">
            <select className={inputCls} value={s.noise.fractal} onChange={(e) => s.setNoise({ fractal: e.target.value })}>
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
          <Field label="Height min / max (m)">
            <span className="flex gap-1">
              <input className={`${inputCls} w-14`} type="number" value={s.noise.height_range_m[0]} onChange={(e) => s.setNoise({ height_range_m: [Number(e.target.value), s.noise.height_range_m[1]] })} />
              <input className={`${inputCls} w-14`} type="number" value={s.noise.height_range_m[1]} onChange={(e) => s.setNoise({ height_range_m: [s.noise.height_range_m[0], Number(e.target.value)] })} />
            </span>
          </Field>
          <button className="btn mt-2 w-full justify-center" onClick={() => s.generate()} disabled={!s.project || s.busy} data-testid="generate">
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
          <h2 className="mt-5 mb-2 text-xs font-semibold uppercase tracking-wider text-jd-fg-2">Export</h2>
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
        </aside>
      </div>

      {/* Bottom: log */}
      <section className="overflow-y-auto border-t border-jd-line bg-jd-bg-1 px-3 py-2 font-mono text-xs" data-testid="log">
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
