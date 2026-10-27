import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ChevronDown, ChevronUp } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import GraphPanel from "./graph/GraphPanel";
import LayerStack from "./graph/LayerStack";
import { type EvalReport, useStore } from "./state";
import type { HeightmapFormat } from "./lib/ops.gen";

// The Editor (ADR-0017): the node graph and the layer stack, beside Studio — which owns the 3-D
// viewport and every panel. A status bar carries the last log line; the log opens above it.

function formatFor(path: string): HeightmapFormat {
  const ext = path.split(".").pop()?.toLowerCase();
  return ext === "png" ? "png16" : ext === "r16" ? "raw_r16" : ext === "r32" ? "raw_r32" : ext === "tif" || ext === "tiff" ? "tiff32" : "exr";
}

/** Defaults for a project created from the Editor's File ▸ New (Studio's Project section has the fields). */
const NEW_PROJECT = { name: "World", extentM: 4096, resolutionPx: 2048 };

export default function App() {
  const s = useStore();
  const [logOpen, setLogOpen] = useState(false);

  // Native file dialogs (tauri-plugin-dialog) for the menu; outside Tauri (browser, tests) they reject.
  const pickProjectToOpen = useCallback(async () => {
    const p = await openDialog({ directory: true, title: "Open Jenga Dunia project" }).catch(() => null);
    if (typeof p === "string") await useStore.getState().openProject(p);
  }, []);
  const pickProjectToCreate = useCallback(async () => {
    const chosen = await saveDialog({ title: "New Jenga Dunia project", defaultPath: `${NEW_PROJECT.name}.jdproj` }).catch(() => null);
    if (!chosen) return;
    const p = chosen.endsWith(".jdproj") ? chosen : `${chosen}.jdproj`; // a project is a directory
    await useStore.getState().newProject(p, NEW_PROJECT.name, NEW_PROJECT.extentM, NEW_PROJECT.resolutionPx);
  }, []);
  const pickAssetToImport = useCallback(async () => {
    const p = await openDialog({
      title: "Import heightmap or mesh",
      filters: [{ name: "Heightmaps and meshes", extensions: ["png", "exr", "tif", "tiff", "r16", "r32", "fbx"] }],
    }).catch(() => null);
    if (typeof p === "string") await useStore.getState().importAsset(p);
  }, []);
  const pickExportPath = useCallback(async () => {
    const p = await saveDialog({
      title: "Export heightmap",
      defaultPath: "height.exr",
      filters: [
        { name: "OpenEXR (32-bit float)", extensions: ["exr"] },
        { name: "PNG (16-bit)", extensions: ["png"] },
        { name: "RAW 16-bit", extensions: ["r16"] },
        { name: "RAW 32-bit float", extensions: ["r32"] },
        { name: "TIFF (32-bit float)", extensions: ["tif", "tiff"] },
      ],
    }).catch(() => null);
    if (p) await useStore.getState().exportHeightmap(p, formatFor(p));
  }, []);

  // A project opened at launch (file association / `jd-app path.jdproj`) is adopted on mount.
  useEffect(() => {
    let cancelled = false;
    void useStore.getState().probeProject(() => cancelled);
    return () => {
      cancelled = true;
    };
  }, []);

  // Connected to Studio: a project opened there opens here, a graph edit made there reloads the
  // graph here, an evaluation run there shows here, and Studio's selection selects the node here.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    type EngineEvent = { topic: string; data: { op?: string; client?: string | null; nodes?: string[]; source?: string | null; report?: EvalReport } };
    listen<EngineEvent>("engine-event", ({ payload: { topic, data } }) => {
      const st = useStore.getState();
      const other = data.client !== "gui-editor";
      if (topic === "project_changed" && other && data.op?.startsWith("graph.") && data.op !== "graph.evaluate") void st.loadGraph(false);
      if (topic === "project_changed" && other && (data.op === "project.open" || data.op === "project.create")) void st.followProject();
      if (topic === "evaluated" && other && data.report) void st.adoptEvaluation(data.report);
      if (topic === "selection" && data.source !== "gui-editor") st.setSelectedNode(data.nodes?.[0] ?? null);
    })
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

  // Crash reporter: an uncaught error or rejected promise anywhere in the UI is written as a report
  // (diag.crash_report) and shown in the log; the app keeps running.
  useEffect(() => {
    const onError = (e: ErrorEvent) => void useStore.getState().reportCrash(String(e.message || e.error), String((e.error as Error | undefined)?.stack ?? ""), "window");
    const onRejection = (e: PromiseRejectionEvent) => {
      const r = e.reason as Error | undefined;
      void useStore.getState().reportCrash(String(r?.message ?? r), String(r?.stack ?? ""), "promise");
    };
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  }, []);

  // Esc aborts whatever is running; inside a text field Esc only leaves the field.
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

  // Native menu bar → the same operations (ids from src-tauri build_menu).
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

  const last = s.log[s.log.length - 1];
  const job = s.jobs[0];
  return (
    <div className="flex h-full flex-col bg-jd-bg-0 text-jd-fg-0">
      <header className="flex h-9 shrink-0 items-center gap-3 border-b border-jd-line bg-jd-bg-1 px-3">
        <img src="/jd-mark.svg" alt="Jenga Dunia" className="h-4 w-4" data-testid="brand-mark" />
        <span className="text-xs font-semibold tracking-wide">Jenga Dunia</span>
        <span className="text-xs text-jd-fg-2" data-testid="app-version">
          v{__JD_VERSION__}
        </span>
        <span className="text-xs text-jd-fg-1" data-testid="project-title">
          {s.project ? `${s.project.name} — ${s.project.world.resolution_px[0]}² · ${s.project.world.extent_m[0]} m` : "no project"}
        </span>
        {job && (
          <span className="ml-auto flex items-center gap-2 text-xs text-jd-accent" data-testid="job-progress">
            {job.op}
            {job.total > 0 ? ` · node ${Math.min(job.done + 1, job.total)}/${job.total} ${job.current.split("@")[0]?.split(".").pop() ?? ""}` : ""}
            {` · ${(job.elapsed_ms / 1000).toFixed(1)} s`}
            <button className="btn px-1.5 py-0" onClick={() => void s.cancelJob(job.id)} disabled={job.cancelling} title="Cancel the running job (Esc)" data-testid="job-cancel">
              {job.cancelling ? "cancelling…" : "Cancel"}
            </button>
            <span className="text-jd-fg-2">Esc to abort</span>
          </span>
        )}
      </header>

      {s.project ? (
        <div className="grid min-h-0 flex-1 grid-cols-[260px_1fr]">
          <aside className="overflow-y-auto border-r border-jd-line bg-jd-bg-1 p-3" data-testid="panel-layers">
            <LayerStack />
          </aside>
          <main className="min-h-0 min-w-0" data-testid="panel-graph">
            <GraphPanel />
          </main>
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center text-sm text-jd-fg-2" data-testid="no-project">
          Create or open a project in Studio — the graph appears here.
        </div>
      )}

      <section className={`shrink-0 overflow-y-auto border-t border-jd-line bg-jd-bg-1 px-3 py-2 font-mono text-xs ${logOpen ? "h-40" : "hidden"}`} data-testid="log">
        {s.log.length === 0 && <span className="text-jd-fg-2">Operations and warnings appear here.</span>}
        {s.log.map((e) => (
          <div key={e.seq} data-seq={e.seq} className={e.level === "error" ? "text-jd-error" : e.level === "warn" ? "text-jd-warn" : "text-jd-fg-1"}>
            {new Date(e.ts).toLocaleTimeString()} {e.text}
          </div>
        ))}
      </section>

      <footer className="flex h-6 shrink-0 items-center gap-2 border-t border-jd-line bg-jd-bg-1 px-3 text-xs" data-testid="status-bar">
        <button className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => setLogOpen((v) => !v)} title={logOpen ? "Hide the log" : "Show the log"} aria-expanded={logOpen} data-testid="toggle-log">
          {logOpen ? <ChevronDown size={12} /> : <ChevronUp size={12} />}
          <span className={`truncate ${last?.level === "error" ? "text-jd-error" : last?.level === "warn" ? "text-jd-warn" : "text-jd-fg-2"}`} data-testid="status-line">
            {last ? last.text : "Ready"}
          </span>
        </button>
      </footer>
    </div>
  );
}
