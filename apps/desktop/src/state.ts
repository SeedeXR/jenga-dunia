// Frontend store: a mirror of what the core reports, never the source of truth (docs/architecture/12-ui-workspace.md).
import { create } from "zustand";
import { layerPreview, op, type JdError, type Preview, type Response, type Warning } from "./lib/api";

export interface ProjectSummary {
  project_id: string;
  name: string;
  path: string;
  world: { extent_m: [number, number]; resolution_px: [number, number]; tile_size: number; halo: number; seed: number };
  schema_version: number;
  dirty: boolean;
  pending_journal: number;
}

export interface LayerInfo {
  layer: string;
  size_px: [number, number];
  stats: { min: number; max: number; mean: number; non_finite: number };
  content_hash: string;
  duration_ms: number;
}

export interface NoiseSettings {
  kind: string;
  fractal: string;
  seed: number;
  feature_size_m: number;
  octaves: number;
  lacunarity: number;
  gain: number;
  height_range_m: [number, number];
}

export interface LogEntry {
  ts: number;
  level: "info" | "warn" | "error";
  text: string;
}

export interface GraphNode {
  id: string;
  kind: string;
  params: Record<string, unknown>;
  bypass: boolean;
  ui?: { position?: [number, number]; label?: string | null };
}

export interface GraphEdge {
  from: { node: string; port: string };
  to: { node: string; port: string };
}

export interface GraphKind {
  kind: string;
  description: string;
  inputs: { name: string; ty: string; required: boolean }[];
  outputs: { name: string; ty: string; required: boolean }[];
  param_schema: unknown;
}

export interface EvalReport {
  lod: number;
  duration_ms: number;
  nodes: { node: string; kind: string; hash: string; cached: boolean; duration_ms: number }[];
  layers: Record<string, string>;
}

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
  exportHeightmap: (path: string, format: string) => Promise<void>;
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
  const log = (r: Response) => set((s) => ({ log: [...s.log.slice(-199), { ts: Date.now(), ...describe(r) }] }));
  // `invoke` rejects (transport/deserialisation failures, or commands returning Err) never escape
  // as unhandled rejections: they become error log entries.
  const logFailure = (what: string, e: unknown) => {
    const err = e as Partial<JdError> | string | undefined;
    const text = typeof err === "string" ? err : err && typeof err === "object" && "code" in err ? `${err.code} ${err.message ?? ""}${err.next ? ` — ${err.next}` : ""}` : String(e);
    set((s) => ({ log: [...s.log.slice(-199), { ts: Date.now(), level: "error", text: `${what}: ${text}` }] }));
  };
  const run = async <T,>(id: string, params: Record<string, unknown> = {}): Promise<Response<T>> => {
    set({ busy: true });
    try {
      const r = await op<T>(id, params);
      log(r);
      return r;
    } catch (e) {
      logFailure(id, e);
      return { op: id, schema_version: 1, request_id: "", status: "error", error: { code: "E_INTERNAL", message: String(e) } };
    } finally {
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
  type GraphView = { nodes: GraphNode[]; edges: GraphEdge[]; undo_depth: number; redo_depth: number };
  const applyGraph = (r: Response<GraphView>) => {
    if (r.status === "ok" && r.result) set({ graph: r.result });
  };
  const loadGraph = async () => {
    if (get().graphKinds.length === 0) {
      const k = await run<GraphKind[]>("graph.kinds");
      if (k.status === "ok" && k.result) set({ graphKinds: k.result });
    }
    applyGraph(await run<GraphView>("graph.list"));
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
    setNoise: (patch) => set((s) => ({ noise: { ...s.noise, ...patch } })),
    loadGraph,
    addNode: async (kind) => {
      // New nodes go to the right of the current graph so they are visible and easy to wire in.
      const right = Math.max(-320, ...get().graph.nodes.map((n) => n.ui?.position?.[0] ?? 0));
      const r = await run<{ id: string }>("graph.add_node", { kind, position: [right + 320, 0] });
      if (r.status === "ok") await loadGraph();
    },
    removeNode: async (id) => applyGraph(await run<GraphView>("graph.remove_node", { id })),
    moveNode: async (id, position) => applyGraph(await run<GraphView>("graph.move_node", { id, position })),
    undoGraph: async () => applyGraph(await run<GraphView>("graph.undo")),
    redoGraph: async () => applyGraph(await run<GraphView>("graph.redo")),
    connectNodes: async (from, fromPort, to, toPort) => applyGraph(await run<GraphView>("graph.connect", { from, from_port: fromPort, to, to_port: toPort })),
    disconnectNodes: async (from, fromPort, to, toPort) => applyGraph(await run<GraphView>("graph.disconnect", { from, from_port: fromPort, to, to_port: toPort })),
    setParam: async (id, key, value) => applyGraph(await run<GraphView>("graph.set_param", { id, key, value })),
    evaluateGraph: async (lod = 0) => {
      const r = await run<EvalReport>("graph.evaluate", { lod });
      if (r.status === "ok" && r.result) {
        set({ lastEval: r.result });
        const layerName = Object.keys(r.result.layers)[0];
        if (layerName) {
          const st = await run<LayerInfo>("terrain.stats", { layer: layerName });
          if (st.status === "ok" && st.result) {
            set({ layer: st.result });
            await refreshPreview();
          }
        }
      }
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
      const r = await run<ProjectSummary>("project.create", {
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
      const r = await run<ProjectSummary>("project.open", { path });
      if (r.status === "ok" && r.result) {
        set({ project: r.result, layer: null, preview: null });
        await loadGraph();
      }
    },
    generate: async () => {
      const r = await run<LayerInfo>("terrain.generate", { noise: get().noise, layer: "height" });
      if (r.status === "ok" && r.result) {
        set({ layer: r.result });
        await refreshPreview();
      }
    },
    exportHeightmap: async (path, format) => {
      await run("terrain.export_heightmap", { layer: "height", path, format });
    },
    save: async () => {
      const r = await run<ProjectSummary>("project.save");
      if (r.status === "ok" && r.result) set({ project: r.result });
    },
  };
});
