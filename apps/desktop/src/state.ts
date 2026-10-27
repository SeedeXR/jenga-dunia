// Frontend store: a mirror of what the core reports, never the source of truth (docs/architecture/12-ui-workspace.md).
// The Editor (ADR-0017) shows the graph and the layer stack; Studio owns the viewport and the
// panels, so nothing here fetches previews, maps or meshes.
import { create } from "zustand";
import type { Edge, GraphEvaluateResult, GraphListResult, HeightmapFormat, JdOps, JobInfo as JobInfoGen, NodeDefInfo, NodeView, NoiseParams, OpId, ProjectOpenResult, ProjectSaveResult } from "./lib/ops.gen";
import { op, type JdError, type Response, type Warning } from "./lib/api";

export type ProjectSummary = ProjectOpenResult;

/** The noise node's generator parameters (Generate writes them into the graph's noise node). */
export type NoiseSettings = Required<Omit<NoiseParams, "offset_m" | "warp" | "octave_weights" | "strength">>;

export interface LogEntry {
  ts: number;
  level: "info" | "warn" | "error";
  text: string;
  /** 1, 2, 3… across the session: still rises once the log is full (e2e `logCount` reads it). */
  seq: number;
}

/** The log with `entry` appended, numbered, keeping the last 200. */
export function appendLog(log: LogEntry[], entry: Omit<LogEntry, "seq">): LogEntry[] {
  return [...log.slice(-199), { ...entry, seq: (log.at(-1)?.seq ?? 0) + 1 }];
}

export type GraphNode = NodeView;

export type GraphEdge = Edge;

export type GraphKind = NodeDefInfo;

export type JobInfo = JobInfoGen;

export type EvalReport = GraphEvaluateResult;

interface State {
  project: ProjectSummary | null;
  /** The layer the last evaluation published first (exports use the full-resolution `height`). */
  layer: string | null;
  noise: NoiseSettings;
  busy: boolean;
  log: LogEntry[];
  graph: { nodes: GraphNode[]; edges: GraphEdge[]; undo_depth?: number; redo_depth?: number; time?: number };
  /** Move the timeline (journaled `graph.set_time`); keyed parameters resolve there. Coalesces a drag. */
  setTime: (time: number) => void;
  graphKinds: GraphKind[];
  lastEval: EvalReport | null;
  /** Re-evaluate at preview LOD after every graph change (World Creator-style live preview). */
  live: boolean;
  setLive: (on: boolean) => void;
  /** Jobs running in the core (polled while an operation is in flight). */
  jobs: JobInfo[];
  cancelJob: (id: number) => Promise<void>;
  /** Node selected in the graph editor (Studio's selection follows over `ui.select`). */
  selectedNode: string | null;
  setSelectedNode: (id: string | null) => void;
  /** Record a crash the UI caught (diag.crash_report) and note it in the log. */
  reportCrash: (message: string, stack: string, source: string) => Promise<void>;
  /** Show an evaluation another window ran (Studio's `evaluated` event): as after one's own. */
  adoptEvaluation: (report: EvalReport) => Promise<void>;
  /** Re-read the graph; `evaluate: false` for another window's edit (that window evaluates it). */
  loadGraph: (evaluate?: boolean) => Promise<void>;
  addNode: (kind: string) => Promise<void>;
  removeNode: (id: string) => Promise<void>;
  moveNode: (id: string, position: [number, number]) => Promise<void>;
  undoGraph: () => Promise<void>;
  redoGraph: () => Promise<void>;
  connectNodes: (from: string, fromPort: string, to: string, toPort: string) => Promise<void>;
  disconnectNodes: (from: string, fromPort: string, to: string, toPort: string) => Promise<void>;
  setParam: (id: string, key: string, value: unknown) => Promise<void>;
  /** `graph.set_bypass`: pass a node's input through unchanged (the layer stack's eye). */
  setBypass: (id: string, bypass: boolean) => Promise<void>;
  /** Splice `kind` in front of the height output (the layer stack's +). */
  insertLayer: (kind: string, params?: Record<string, unknown> | null) => Promise<void>;
  /** Remove a chain node and reconnect what fed it to what it fed. */
  unsplice: (id: string) => Promise<void>;
  /** Move a chain node one step later (`up`, nearer the output) or earlier (`down`). */
  swapInChain: (id: string, dir: "up" | "down") => Promise<void>;
  evaluateGraph: (lod?: number) => Promise<void>;
  /** Evaluate (and live-preview) only this output layer and what feeds it — the gate pattern; null = everything. */
  evalOutput: string | null;
  setEvalOutput: (layer: string | null) => void;
  /** Adopt a project the backend already has open (launch argument / file association). */
  probeProject: (isCancelled?: () => boolean) => Promise<void>;
  /** Another window (Studio) opened or created a project: show it without evaluating it (that
   *  window evaluates; its report arrives as `evaluated`). */
  followProject: () => Promise<void>;
  newProject: (path: string, name: string, extent: number, resolution: number) => Promise<void>;
  openProject: (path: string) => Promise<void>;
  /** The noise settings into the graph's noise node, then a full evaluation (menu ▸ Generate). */
  generate: () => Promise<void>;
  exportHeightmap: (path: string, format: HeightmapFormat) => Promise<void>;
  /** Copy a heightmap or mesh file into assets/ and add an input node that reads it. */
  importAsset: (path: string) => Promise<void>;
  save: () => Promise<void>;
}

const defaultNoise: NoiseSettings = {
  kind: "open_simplex2",
  fractal: "fbm",
  seed: 0,
  feature_size_m: 1024,
  octaves: 6,
  lacunarity: 2,
  gain: 0.5,
  height_range_m: [0, 500],
};

function describe(r: Response): { level: LogEntry["level"]; text: string } {
  if (r.status === "error" && r.error) {
    const e: JdError = r.error;
    return { level: "error", text: `${r.op}: ${e.code} ${e.message}${e.next ? ` — ${e.next}` : ""}` };
  }
  const warns = (r.warnings ?? []).map((w: Warning) => `${w.code}: ${w.message}`).join("; ");
  return { level: warns ? "warn" : "info", text: `${r.op}: ${r.status}${warns ? ` (${warns})` : ""}` };
}

export const useStore = create<State>((set, get) => {
  const log = (r: Response) => set((s) => ({ log: appendLog(s.log, { ts: Date.now(), ...describe(r) }) }));
  // `invoke` rejects (transport/deserialisation failures, or commands returning Err) never escape
  // as unhandled rejections: they become error log entries.
  const logFailure = (what: string, e: unknown) => {
    const err = e as Partial<JdError> | string | undefined;
    const text = typeof err === "string" ? err : err && typeof err === "object" && "code" in err ? `${err.code} ${err.message ?? ""}${err.next ? ` — ${err.next}` : ""}` : String(e);
    set((s) => ({ log: appendLog(s.log, { ts: Date.now(), level: "error", text: `${what}: ${text}` }) }));
  };
  // While an op is in flight, poll the core's job table so long evaluations show progress and
  // can be cancelled; the poll uses `op` directly so it never appears in the log.
  let pollers = 0;
  let generation = 0; // bumped on stop so a late jobs.list response cannot resurrect a finished job
  let pollTimer: ReturnType<typeof setInterval> | undefined;
  const startPolling = () => {
    if (pollers++ > 0) return;
    const gen = generation;
    pollTimer = setInterval(() => {
      void op<JobInfo[]>("jobs.list")
        .then((r) => {
          if (gen === generation && pollers > 0) set({ jobs: r.status === "ok" && r.result ? r.result : [] });
        })
        .catch(() => undefined);
    }, 250);
  };
  const stopPolling = () => {
    if (--pollers > 0) return;
    clearInterval(pollTimer);
    generation++;
    set({ jobs: [] });
  };
  // Typed by the generated op map: request shape and result type follow the op id.
  const run = async <K extends OpId>(id: K, params?: JdOps[K]["request"]): Promise<Response<JdOps[K]["result"]>> => {
    set({ busy: true });
    startPolling();
    try {
      const r = await op<JdOps[K]["result"]>(id, (params ?? {}) as Record<string, unknown>);
      log(r);
      return r;
    } catch (e) {
      logFailure(id, e);
      return { op: id, schema_version: 1, request_id: "", status: "error", error: { code: "E_INTERNAL", message: String(e) } };
    } finally {
      stopPolling();
      set({ busy: false });
    }
  };
  // After an evaluation: remember what it published (a filtered `only <output>` evaluation
  // republishes one chain; the others stay published) and keep the layer in view by name.
  const adopt = async (report: EvalReport) => {
    const known = get().lastEval?.layers ?? {};
    set({ lastEval: { ...report, layers: { ...known, ...report.layers } } });
    const names = Object.keys(report.layers);
    if (names.length === 0) return;
    const current = get().layer?.split("#")[0];
    set({ layer: names.find((n) => n.split("#")[0] === current) ?? names[0]! });
  };
  // Preview LOD keeps live evaluations ≤ 512² so edits feel immediate; Evaluate/export use lod 0.
  const previewLod = () => {
    const res = get().project?.world.resolution_px ?? [512, 512];
    return Math.max(0, Math.ceil(Math.log2(Math.max(res[0], res[1]) / 512)));
  };
  let liveTimer: ReturnType<typeof setTimeout> | undefined;
  let timeTimer: ReturnType<typeof setTimeout> | undefined;
  /** The live evaluation in flight, if any: Generate and Evaluate wait for it (see `settle`). */
  let liveInFlight: Promise<void> | null = null;
  const liveEvaluate = () => {
    clearTimeout(liveTimer);
    liveTimer = setTimeout(() => {
      const { project, live } = get();
      if (!project || !live) return;
      liveInFlight = (async () => {
        startPolling(); // live evaluations show up in the header and can be cancelled too
        const gen = graphGen;
        try {
          const only = get().evalOutput;
          const r = await op<EvalReport>("graph.evaluate", only ? { lod: previewLod(), outputs: [only] } : { lod: previewLod() });
          // The graph moved on while this ran (a quick second edit): the result is of an older
          // graph and would overwrite the newer layer — drop it, the next one is queued.
          if (gen !== graphGen) return;
          if (r.status === "ok" && r.result) await adopt(r.result);
          else if (r.error?.code === "E_GRAPH_MISSING_INPUT") {
            // Half-wired graphs are normal while editing: say what is missing, without an error entry.
            set((s) => ({ log: appendLog(s.log, { ts: Date.now(), level: "info", text: `live preview waiting: ${r.error?.message ?? ""}` }) }));
          } else log(r);
        } catch (e) {
          logFailure("graph.evaluate (live)", e);
        } finally {
          stopPolling();
          liveInFlight = null;
        }
      })();
    }, 150);
  };
  // Before a full evaluation: cancel a queued preview and wait for a running one. Two evaluations
  // of one project overlapping in the core each snapshot the graph at their start, and the slower
  // one publishes a stale `height` last (seen under the parallel e2e gate).
  const settle = async () => {
    clearTimeout(liveTimer);
    if (liveInFlight) await liveInFlight;
  };
  // Splice a new node of `kind` between whatever feeds the output publishing `layer` and that
  // output (the core's `graph.splice`, shared with Studio).
  const spliceBefore = async (layer: string, kind: string, params: Record<string, unknown> | null): Promise<string | null> => {
    graphGen++;
    const r = await run("graph.splice", { layer, kind, ...(params ? { params } : {}) });
    if (r.status !== "ok" || !r.result) return null;
    await loadGraph();
    return r.result.id;
  };
  type GraphView = GraphListResult;
  // Autosave: a debounced project.save after any journaled edit, so `dirty` / `journal` return to
  // false / 0 on their own (the WAL already survives crashes; this is about never losing work on
  // quit). The delay is the project's `settings.autosave_interval_s` (project.json; 0 disables);
  // `?autosave=<ms>` in the URL overrides it (tests that inspect the journal pass 0).
  const autosaveOverride = new URLSearchParams(location.search).get("autosave");
  const autosaveMs = () => (autosaveOverride !== null ? Number(autosaveOverride) : (get().project?.settings.autosave_interval_s ?? 0) * 1000);
  let autosaveTimer: ReturnType<typeof setTimeout> | undefined;
  const scheduleAutosave = () => {
    const ms = autosaveMs();
    if (!(ms > 0)) return;
    clearTimeout(autosaveTimer);
    autosaveTimer = setTimeout(() => void autosave(), ms);
  };
  const autosave = async () => {
    if (get().busy || get().jobs.length > 0 || !get().project) return scheduleAutosave();
    const r = await op<ProjectSaveResult>("project.save");
    log(r);
    if (r.status === "ok" && r.result) set({ project: r.result });
  };
  // The noise settings mirror the graph's noise node: whenever the node's params change (a project
  // opened, an inspector edit, undo, Studio) they adopt them, so Generate — which writes them into
  // the node — never reverts an edit made elsewhere. Keyed (timeline) values fall back to defaults.
  let syncedNoise = "";
  const syncNoise = (g: GraphView) => {
    const node = g.nodes.find((n) => n.kind.startsWith("jd.noise."));
    if (!node) return;
    const json = JSON.stringify(node.params ?? {});
    if (json === syncedNoise) return;
    syncedNoise = json;
    const patch: Partial<NoiseSettings> = {};
    for (const key of Object.keys(defaultNoise) as (keyof NoiseSettings)[]) {
      const v = node.params?.[key];
      const d = defaultNoise[key];
      const plain = Array.isArray(d) ? Array.isArray(v) && v.length === 2 && v.every((x) => typeof x === "number") : typeof v === typeof d;
      (patch as Record<string, unknown>)[key] = plain ? v : d;
    }
    set((s) => ({ noise: { ...s.noise, ...patch } }));
  };
  const applyGraph = (r: Response<GraphView>, evaluate = true) => {
    if (r.status === "ok" && r.result) {
      set({ graph: r.result });
      syncNoise(r.result);
      if (evaluate) liveEvaluate();
      scheduleAutosave();
    }
  };
  // Every mutation bumps this before its request goes out; a graph.list that started earlier may
  // describe an older graph and is dropped (seen as e2e flakes: a late list hid a just-added node).
  let graphGen = 0;
  const loadGraph = async (evaluate = true) => {
    if (get().graphKinds.length === 0) {
      const k = await run("graph.kinds");
      if (k.status === "ok" && k.result) set({ graphKinds: k.result });
    }
    const gen = graphGen;
    const r = await run("graph.list");
    if (gen === graphGen) applyGraph(r, evaluate);
  };
  const mutate = async <K extends OpId>(id: K, params?: JdOps[K]["request"]) => {
    graphGen++;
    applyGraph((await run(id, params)) as Response<GraphView>);
  };
  return {
    project: null,
    layer: null,
    noise: defaultNoise,
    busy: false,
    log: [],
    graph: { nodes: [], edges: [] },
    graphKinds: [],
    lastEval: null,
    live: true,
    jobs: [],
    cancelJob: async (id) => {
      await op("jobs.cancel", { id }).catch((e) => logFailure("jobs.cancel", e));
    },
    selectedNode: null,
    setSelectedNode: (id) => {
      if (get().selectedNode === id) return;
      set({ selectedNode: id });
      // Studio highlights it too (ADR-0017: the selection crosses as `ui.select` → `selection`).
      void op("ui.select", { nodes: id ? [id] : [] }).catch(() => undefined);
    },
    reportCrash: async (message, stack, source) => {
      const r = await op<{ path: string }>("diag.crash_report", { message: message.slice(0, 4000), stack: stack.slice(0, 16000), source }).catch(() => null);
      set((st) => ({ log: appendLog(st.log, { ts: Date.now(), level: "error", text: `crash report${r?.status === "ok" && r.result ? `: ${r.result.path}` : " (not written)"} — ${message}` }) }));
    },
    adoptEvaluation: (report) => adopt(report),
    setLive: (on) => {
      set({ live: on });
      if (on) liveEvaluate();
    },
    loadGraph,
    importAsset: async (path) => {
      const r = await run("project.import_asset", { path });
      if (r.status !== "ok" || !r.result) return;
      const right = Math.max(-320, ...get().graph.nodes.map((n) => n.ui?.position?.[0] ?? 0));
      graphGen++;
      const add = await run("graph.add_node", { kind: "jd.input.heightmap@1", params: { path: r.result.name }, position: [right + 320, 0] });
      if (add.status === "ok") await loadGraph();
    },
    addNode: async (kind) => {
      // New nodes go to the right of the current graph so they are visible and easy to wire in.
      const right = Math.max(-320, ...get().graph.nodes.map((n) => n.ui?.position?.[0] ?? 0));
      graphGen++;
      const r = await run("graph.add_node", { kind, position: [right + 320, 0] });
      if (r.status === "ok") await loadGraph();
    },
    removeNode: async (id) => mutate("graph.remove_node", { id }),
    moveNode: async (id, position) => mutate("graph.move_node", { id, position }),
    undoGraph: async () => mutate("graph.undo"),
    redoGraph: async () => mutate("graph.redo"),
    connectNodes: async (from, fromPort, to, toPort) => mutate("graph.connect", { from, from_port: fromPort, to, to_port: toPort }),
    disconnectNodes: async (from, fromPort, to, toPort) => mutate("graph.disconnect", { from, from_port: fromPort, to, to_port: toPort }),
    setParam: async (id, key, value) => mutate("graph.set_param", { id, key, value }),
    setBypass: async (id, bypass) => mutate("graph.set_bypass", { id, bypass }),
    insertLayer: async (kind, params) => {
      const id = await spliceBefore("height", kind, params ?? null);
      if (id) get().setSelectedNode(id);
    },
    unsplice: async (id) => {
      // The core re-points whatever the node fed at what fed it, then removes it (shared with Studio).
      await mutate("graph.unsplice", { id });
      if (get().selectedNode === id) set({ selectedNode: null });
    },
    swapInChain: async (id, dir) => {
      // P → A → B → N becomes P → B → A → N (the core's graph.swap_in_chain, shared with Studio).
      await mutate("graph.swap_in_chain", { id, dir });
    },
    setTime: (time) => {
      set((s) => ({ graph: { ...s.graph, time } })); // the slider follows the pointer; the journal gets the settled value
      clearTimeout(timeTimer);
      timeTimer = setTimeout(() => void mutate("graph.set_time", { time }), 200);
    },
    evalOutput: null,
    setEvalOutput: (layer) => set({ evalOutput: layer }),
    evaluateGraph: async (lod = 0) => {
      await settle();
      const only = get().evalOutput;
      const r = await run("graph.evaluate", only ? { lod, outputs: [only] } : { lod });
      if (r.status === "ok" && r.result) await adopt(r.result);
    },
    probeProject: async (isCancelled) => {
      try {
        const r = await op<ProjectSummary>("project.info");
        if (isCancelled?.()) return;
        if (r.status === "ok" && r.result) {
          set({ project: r.result });
          log(r);
          await loadGraph();
        }
      } catch {
        // not inside Tauri (browser/tests)
      }
    },
    followProject: async () => {
      // Synchronously, before any await: the other window's evaluation report follows this event.
      set({ project: null, layer: null, lastEval: null });
      const r = await op<ProjectSummary>("project.info");
      if (r.status !== "ok" || !r.result) return log(r);
      await loadGraph(false);
      set({ project: r.result });
    },
    newProject: async (path, name, extent, resolution) => {
      const r = await run("project.create", {
        path,
        name,
        world: { extent_m: [extent, extent], resolution_px: [resolution, resolution], tile_size: 512, halo: 4, origin: [0, 0], seed: 42 },
      });
      if (r.status === "ok" && r.result) {
        set({ project: null, layer: null, lastEval: null }); // a new project publishes its own layers
        await loadGraph();
        set({ project: r.result });
        liveEvaluate();
      }
    },
    openProject: async (path) => {
      const r = await run("project.open", { path });
      if (r.status === "ok" && r.result) {
        // Opening the project already shown keeps it mounted: unmounting would drop an edit in progress.
        if (get().project?.project_id !== r.result.project_id) set({ project: null, layer: null, lastEval: null });
        await loadGraph();
        set({ project: r.result });
        liveEvaluate();
      }
    },
    generate: async () => {
      // One source of truth: the noise settings go into the graph's noise node and the graph is
      // evaluated. Without a noise node (a graph built from an imported heightmap) the standalone
      // op fills `height`.
      const noise = get().noise;
      const node = get().graph.nodes.find((n) => n.kind.startsWith("jd.noise."));
      if (node) {
        for (const [key, value] of Object.entries(noise)) {
          if (JSON.stringify(node.params?.[key]) !== JSON.stringify(value)) await mutate("graph.set_param", { id: node.id, key, value });
        }
        await settle(); // the full evaluation below supersedes the preview those edits queued
        const r = await run("graph.evaluate", { lod: 0 });
        if (r.status === "ok" && r.result) await adopt(r.result);
        return;
      }
      const r = await run("terrain.generate", { noise, layer: "height" });
      if (r.status === "ok" && r.result) set({ layer: r.result.layer });
    },
    exportHeightmap: async (path, format) => {
      // A live preview only published `height#lod<N>`; exports always use the full-resolution layer.
      if (get().layer?.includes("#lod")) {
        const r = await run("graph.evaluate", { lod: 0 });
        if (r.status !== "ok") return;
      }
      await run("terrain.export_heightmap", { layer: "height", path, format });
    },
    save: async () => {
      const r = await run("project.save");
      if (r.status === "ok" && r.result) set({ project: r.result });
    },
  };
});
