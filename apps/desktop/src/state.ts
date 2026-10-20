// Frontend store: a mirror of what the core reports, never the source of truth (docs/architecture/12-ui-workspace.md).
import { create } from "zustand";
import type { Edge, GraphEvaluateResult, GraphListResult, HeightmapFormat, JdOps, JobInfo as JobInfoGen, NodeDefInfo, NodeView, NoiseParams, OpId, ProjectOpenResult, ProjectSaveResult, TerrainStatsResult } from "./lib/ops.gen";
import { layerPreview, op, type JdError, type Preview, type Response, type Warning } from "./lib/api";

export type ProjectSummary = ProjectOpenResult;

export type LayerInfo = TerrainStatsResult;

/** The noise inspector's fields: every generator parameter except the sampling offset. */
export type NoiseSettings = Required<Omit<NoiseParams, "offset_m">>;

export interface LogEntry {
  ts: number;
  level: "info" | "warn" | "error";
  text: string;
}

export type GraphNode = NodeView;

export type GraphEdge = Edge;

export type GraphKind = NodeDefInfo;

export type JobInfo = JobInfoGen;

export type EvalReport = GraphEvaluateResult;

interface State {
  project: ProjectSummary | null;
  layer: LayerInfo | null;
  preview: Preview | null;
  previewStats: { bytes: number; ms: number; lod: number } | null;
  noise: NoiseSettings;
  busy: boolean;
  log: LogEntry[];
  graph: { nodes: GraphNode[]; edges: GraphEdge[]; undo_depth?: number; redo_depth?: number };
  graphKinds: GraphKind[];
  lastEval: EvalReport | null;
  /** Re-evaluate at preview LOD after every graph change (World Creator-style live preview). */
  live: boolean;
  setLive: (on: boolean) => void;
  /** Show a published layer (height, height#lod2, water, sediment, …) in the viewport. */
  viewLayer: (name: string) => Promise<void>;
  /** Jobs running in the core (polled while an operation is in flight). */
  jobs: JobInfo[];
  cancelJob: (id: number) => Promise<void>;
  /** Node selected in the graph editor (also the sculpt target when it is a sculpt node). */
  selectedNode: string | null;
  setSelectedNode: (id: string | null) => void;
  /** Sculpt mode: viewport drags paint strokes into the selected sculpt node. */
  sculpting: boolean;
  setSculpting: (on: boolean) => void;
  brush: { radius_m: number; strength_m: number; mode: "raise" | "smooth" | "flatten"; amount: number };
  setBrush: (patch: Partial<{ radius_m: number; strength_m: number; mode: "raise" | "smooth" | "flatten"; amount: number }>) => void;
  /** Insert a sculpt node in front of the first output (or select the existing one) and enter sculpt mode. */
  startSculpting: () => Promise<void>;
  /** Append a stroke to the selected sculpt node (journaled: undo removes the whole stroke). */
  addStroke: (points: [number, number][]) => Promise<void>;
  setNoise: (patch: Partial<NoiseSettings>) => void;
  loadGraph: () => Promise<void>;
  addNode: (kind: string) => Promise<void>;
  removeNode: (id: string) => Promise<void>;
  moveNode: (id: string, position: [number, number]) => Promise<void>;
  undoGraph: () => Promise<void>;
  redoGraph: () => Promise<void>;
  connectNodes: (from: string, fromPort: string, to: string, toPort: string) => Promise<void>;
  disconnectNodes: (from: string, fromPort: string, to: string, toPort: string) => Promise<void>;
  setParam: (id: string, key: string, value: unknown) => Promise<void>;
  evaluateGraph: (lod?: number) => Promise<void>;
  /** Adopt a project the backend already has open (launch argument / file association). */
  probeProject: (isCancelled?: () => boolean) => Promise<void>;
  newProject: (path: string, name: string, extent: number, resolution: number) => Promise<void>;
  openProject: (path: string) => Promise<void>;
  generate: () => Promise<void>;
  exportHeightmap: (path: string, format: HeightmapFormat) => Promise<void>;
  save: () => Promise<void>;
  /** Bumped when a settings edit is refused so the inputs snap back to the stored values. */
  settingsRev: number;
  /** Change project settings (journaled); fields left out keep their value. */
  setSettings: (patch: JdOps["project.set_settings"]["request"]) => Promise<void>;
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
  const log = (r: Response) => set((s) => ({ log: [...s.log.slice(-199), { ts: Date.now(), ...describe(r) }] }));
  // `invoke` rejects (transport/deserialisation failures, or commands returning Err) never escape
  // as unhandled rejections: they become error log entries.
  const logFailure = (what: string, e: unknown) => {
    const err = e as Partial<JdError> | string | undefined;
    const text = typeof err === "string" ? err : err && typeof err === "object" && "code" in err ? `${err.code} ${err.message ?? ""}${err.next ? ` — ${err.next}` : ""}` : String(e);
    set((s) => ({ log: [...s.log.slice(-199), { ts: Date.now(), level: "error", text: `${what}: ${text}` }] }));
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
  const refreshPreview = async () => {
    const { layer } = get();
    if (!layer) return;
    // Preview LOD keeps the buffer ≤ ~1 M samples regardless of world size.
    const lod = Math.max(0, Math.ceil(Math.log2(Math.max(layer.size_px[0], layer.size_px[1]) / 1024)));
    try {
      const { preview, bytes, ms } = await layerPreview(layer.layer, lod);
      set({ preview, previewStats: { bytes, ms, lod } });
    } catch (e) {
      logFailure("layer_preview", e);
    }
  };
  // After an evaluation: adopt the first published layer (stats + viewport preview).
  // Stats + preview refresh are background work: they must not flip `busy`, or every toolbar
  // button flashes disabled for a moment after each live preview and a click landing right then
  // is lost (seen as e2e flakes: a redo / add click with no op in the log).
  const showLayer = async (layerName: string) => {
    const st = await op<TerrainStatsResult>("terrain.stats", { layer: layerName });
    log(st);
    if (st.status === "ok" && st.result) {
      set({ layer: st.result });
      await refreshPreview();
    }
  };
  // After an evaluation: keep showing the layer the user is looking at (same name, or its
  // full-res/preview twin) when the graph still publishes it; otherwise the first one.
  const adopt = async (report: EvalReport) => {
    set({ lastEval: report });
    const names = Object.keys(report.layers);
    if (names.length === 0) return;
    const current = get().layer?.layer.split("#")[0];
    const keep = names.find((n) => n.split("#")[0] === current);
    await showLayer(keep ?? names[0]!);
  };
  // Preview LOD keeps live evaluations ≤ 512² so edits feel immediate; Evaluate/export use lod 0.
  const previewLod = () => {
    const res = get().project?.world.resolution_px ?? [512, 512];
    return Math.max(0, Math.ceil(Math.log2(Math.max(res[0], res[1]) / 512)));
  };
  let liveTimer: ReturnType<typeof setTimeout> | undefined;
  const liveEvaluate = () => {
    clearTimeout(liveTimer);
    liveTimer = setTimeout(async () => {
      const { project, live } = get();
      if (!project || !live) return;
      startPolling(); // live evaluations show up in the header and can be cancelled too
      try {
        const r = await op<EvalReport>("graph.evaluate", { lod: previewLod() });
        if (r.status === "ok" && r.result) await adopt(r.result);
        else if (r.error?.code === "E_GRAPH_MISSING_INPUT") {
          // Half-wired graphs are normal while editing: say what is missing, without an error entry.
          set((s) => ({ log: [...s.log.slice(-199), { ts: Date.now(), level: "info", text: `live preview waiting: ${r.error?.message ?? ""}` }] }));
        } else log(r);
      } catch (e) {
        logFailure("graph.evaluate (live)", e);
      } finally {
        stopPolling();
      }
    }, 150);
  };
  type GraphView = GraphListResult;
  // Autosave: a debounced project.save after any journaled edit, so `dirty` / `journal` return to
  // false / 0 on their own (the WAL already survives crashes; this is about never losing work on
  // quit). The delay is the project's `settings.autosave_interval_s` (project.json; 0 disables);
  // `?autosave=<ms>` in the URL overrides it (tests that inspect the journal pass 0).
  // ponytail: edit the interval in project.json; a settings panel comes with the preferences UI.
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
  const applyGraph = (r: Response<GraphView>) => {
    if (r.status === "ok" && r.result) {
      set({ graph: r.result });
      liveEvaluate();
      scheduleAutosave();
    }
  };
  // Every mutation bumps this before its request goes out; a graph.list that started earlier may
  // describe an older graph and is dropped (seen as e2e flakes: a late list hid a just-added node).
  let graphGen = 0;
  const loadGraph = async () => {
    if (get().graphKinds.length === 0) {
      const k = await run("graph.kinds");
      if (k.status === "ok" && k.result) set({ graphKinds: k.result });
    }
    const gen = graphGen;
    const r = await run("graph.list");
    if (gen === graphGen) applyGraph(r);
  };
  const mutate = async <K extends OpId>(id: K, params?: JdOps[K]["request"]) => {
    graphGen++;
    applyGraph((await run(id, params)) as Response<GraphView>);
  };
  return {
    project: null,
    layer: null,
    preview: null,
    previewStats: null,
    noise: defaultNoise,
    busy: false,
    log: [],
    graph: { nodes: [], edges: [] },
    graphKinds: [],
    lastEval: null,
    live: true,
    viewLayer: showLayer,
    jobs: [],
    cancelJob: async (id) => {
      await op("jobs.cancel", { id }).catch((e) => logFailure("jobs.cancel", e));
    },
    selectedNode: null,
    setSelectedNode: (id) => set({ selectedNode: id }),
    sculpting: false,
    setSculpting: (on) => set({ sculpting: on }),
    brush: { radius_m: 150, strength_m: 40, mode: "raise", amount: 0.5 },
    setBrush: (patch) => set((s) => ({ brush: { ...s.brush, ...patch } })),
    startSculpting: async () => {
      const g = get().graph;
      let node = g.nodes.find((n) => n.kind.startsWith("jd.sculpt.strokes@"));
      if (!node) {
        const output = g.nodes.find((n) => n.kind.startsWith("jd.output.height@"));
        const feed = output ? g.edges.find((e) => e.to.node === output.id && e.to.port === "in") : undefined;
        const pos = output?.ui?.position ?? [0, 0];
        graphGen++;
        const r = await run("graph.add_node", { kind: "jd.sculpt.strokes@1", position: [pos[0], pos[1] + 140] });
        if (r.status !== "ok" || !r.result) return;
        const id = r.result.id;
        if (feed) await get().connectNodes(feed.from.node, feed.from.port, id, "in");
        if (output) await get().connectNodes(id, "out", output.id, "in");
        await loadGraph();
        node = get().graph.nodes.find((n) => n.id === id);
      }
      if (node) set({ selectedNode: node.id, sculpting: true });
    },
    addStroke: async (points) => {
      const { selectedNode, graph, brush } = get();
      const node = graph.nodes.find((n) => n.id === selectedNode && n.kind.startsWith("jd.sculpt.strokes@"));
      if (!node || points.length === 0) return;
      const strokes = Array.isArray(node.params?.strokes) ? (node.params.strokes as unknown[]) : [];
      await get().setParam(node.id, "strokes", [...strokes, { points: points.map(([u, v]) => [Math.round(u * 10) / 10, Math.round(v * 10) / 10]), radius_m: brush.radius_m, strength_m: brush.strength_m, mode: brush.mode, amount: brush.amount }]);
    },
    setLive: (on) => {
      set({ live: on });
      if (on) liveEvaluate();
    },
    setNoise: (patch) => set((s) => ({ noise: { ...s.noise, ...patch } })),
    loadGraph,
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
    evaluateGraph: async (lod = 0) => {
      const r = await run("graph.evaluate", { lod });
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
    newProject: async (path, name, extent, resolution) => {
      const r = await run("project.create", {
        path,
        name,
        world: { extent_m: [extent, extent], resolution_px: [resolution, resolution], tile_size: 512, halo: 4, origin: [0, 0], seed: 42 },
      });
      if (r.status === "ok" && r.result) {
        set({ project: r.result, layer: null, preview: null });
        await loadGraph();
      }
    },
    openProject: async (path) => {
      const r = await run("project.open", { path });
      if (r.status === "ok" && r.result) {
        set({ project: r.result, layer: null, preview: null });
        await loadGraph();
      }
    },
    generate: async () => {
      const r = await run("terrain.generate", { noise: get().noise, layer: "height" });
      if (r.status === "ok" && r.result) {
        set({ layer: r.result });
        await refreshPreview();
      }
    },
    exportHeightmap: async (path, format) => {
      // A live preview only published `height#lod<N>`; exports always use the full-resolution layer.
      if (get().layer?.layer.includes("#lod")) {
        const r = await run("graph.evaluate", { lod: 0 });
        if (r.status !== "ok") return;
      }
      await run("terrain.export_heightmap", { layer: "height", path, format });
    },
    settingsRev: 0,
    setSettings: async (patch) => {
      const r = await run("project.set_settings", patch);
      if (r.status === "ok" && r.result) set({ project: r.result });
      else set((s) => ({ settingsRev: s.settingsRev + 1 }));
    },
    save: async () => {
      const r = await run("project.save");
      if (r.status === "ok" && r.result) set({ project: r.result });
    },
  };
});
