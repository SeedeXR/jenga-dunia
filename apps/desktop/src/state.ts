// Frontend store: a mirror of what the core reports, never the source of truth (docs/architecture/12-ui-workspace.md).
import { create } from "zustand";
import type { Edge, GraphEvaluateResult, GraphListResult, HeightmapFormat, JdOps, JobInfo as JobInfoGen, NodeDefInfo, NodeView, NoiseParams, OceanWavesRequest, OceanWavesResult, OpId, ProjectOpenResult, ProjectSaveResult, ScatterPoint, TerrainStatsResult } from "./lib/ops.gen";
import { horizonPreview, layerPreview, layerTextures, op, skyPreview, type JdError, type LayerMaps, type Preview, type Response, type Warning } from "./lib/api";
import { adaptLodBias } from "./lib/previewLod";
import type { MaterialSpec } from "./lib/ops.gen";

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

function readPreviewMaxPx(): number {
  try {
    const v = Number(localStorage.getItem("jd.previewMaxPx") ?? 1024);
    return v === 2048 ? 2048 : 1024;
  } catch {
    return 1024;
  }
}
function readPreviewTarget(): number {
  try {
    return Math.max(0, Number(localStorage.getItem("jd.previewTargetMs") ?? 0)) || 0;
  } catch {
    return 0;
  }
}

interface State {
  project: ProjectSummary | null;
  layer: LayerInfo | null;
  preview: Preview | null;
  /** Layer whose values colour the terrain instead of the height ramp (null = ramp). */
  colorLayer: string | null;
  colorPreview: Preview | null;
  setColorLayer: (name: string | null) => Promise<void>;
  previewStats: { bytes: number; ms: number; lod: number } | null;
  /** Baked material maps of the shown height layer (full-field albedo/normal/roughness), or null. */
  maps: LayerMaps | null;
  mapsMs: number;
  /** Texture the viewport with the baked maps (off = height ramp vertex colours). */
  pbr: boolean;
  /** The material layer stack the viewport bakes with (mask layer + colour + roughness, in order). */
  materials: MaterialSpec[];
  setMaterials: (materials: MaterialSpec[]) => void;
  /** Wind sea of the sea-level node (`ocean.waves`), drawn by the viewport; null without a sea. */
  waves: OceanWavesResult | null;
  loadWaves: (wind: OceanWavesRequest | null) => Promise<void>;
  setPbr: (on: boolean) => void;
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
  /** Show a published layer (height, height#lod2, water, sediment, …) in the viewport. */
  viewLayer: (name: string) => Promise<void>;
  /** Jobs running in the core (polled while an operation is in flight). */
  jobs: JobInfo[];
  cancelJob: (id: number) => Promise<void>;
  /** Last diagnostics results shown in the project panel. */
  diag: { env?: JdOps["diag.env"]["result"]; gc?: JdOps["project.cache_gc"]["result"]; bench?: JdOps["diag.bench"]["result"] };
  runDiag: (what: "env" | "gc" | "bench") => Promise<void>;
  /** Node selected in the graph editor (also the sculpt target when it is a sculpt node). */
  selectedNode: string | null;
  setSelectedNode: (id: string | null) => void;
  /** Sculpt mode: viewport drags paint strokes into the selected sculpt node. */
  sculpting: boolean;
  setSculpting: (on: boolean) => void;
  brush: { radius_m: number; strength_m: number; mode: "raise" | "smooth" | "flatten"; amount: number };
  /** Viewport sun direction (not part of the project). */
  sun: { azimuth_deg: number; elevation_deg: number; turbidity: number; clouds: number; fog: number };
  setSun: (patch: Partial<{ azimuth_deg: number; elevation_deg: number; turbidity: number; clouds: number; fog: number }>) => void;
  /** The noise continued beyond the border (`terrain.horizon`), drawn as a coarse skirt under the terrain. */
  horizon: { preview: Preview; scale: number } | null;
  showHorizon: boolean;
  setShowHorizon: (on: boolean) => void;
  refreshHorizon: () => Promise<void>;
  /** The physical sky (`sky.preview`) for the viewport dome, re-baked when the sun moves. */
  sky: { bitmap: ImageBitmap; px: [number, number] } | null;
  refreshSky: () => Promise<void>;
  /** Scatter preview: instances placed from a 0..1 mask layer and drawn in the viewport; export writes a GLB. */
  scatter: { mask: string | null; spacing_m: number; seed: number; grow: boolean; points: ScatterPoint[] | null };
  setScatter: (patch: Partial<{ mask: string | null; spacing_m: number; seed: number; grow: boolean }>) => void;
  placeScatter: () => Promise<void>;
  exportScatter: () => Promise<void>;
  setBrush: (patch: Partial<{ radius_m: number; strength_m: number; mode: "raise" | "smooth" | "flatten"; amount: number }>) => void;
  /** Insert a sculpt node in front of the first output (or select the existing one) and enter sculpt mode. */
  startSculpting: () => Promise<void>;
  /** Paint the scatter mask: a clamped (0..1) stroke node in front of the mask's output, brush strength 0.5, sculpt mode on. */
  paintDensity: () => Promise<void>;
  /** What the next viewport drag draws: a road (a spline node in front of the height output), or nothing (brush strokes). */
  drawing: "road" | null;
  /** Arm the next drag to lay a road; the road's width in metres comes from `roadWidthM`. */
  startRoad: () => void;
  roadWidthM: number;
  setRoadWidth: (m: number) => void;
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
  /** Evaluate (and live-preview) only this output layer and what feeds it — the gate pattern; null = everything. */
  evalOutput: string | null;
  setEvalOutput: (layer: string | null) => void;
  /** Adopt a project the backend already has open (launch argument / file association). */
  probeProject: (isCancelled?: () => boolean) => Promise<void>;
  newProject: (path: string, name: string, extent: number, resolution: number) => Promise<void>;
  openProject: (path: string) => Promise<void>;
  generate: () => Promise<void>;
  exportHeightmap: (path: string, format: HeightmapFormat) => Promise<void>;
  /** Write exports/water/: depth, colour, foam and the wave normal tile for the graph's sea-level node (`ocean.bake`). */
  exportWater: () => Promise<void>;
  /** Write exports/frames/: one 16-bit heightmap per frame across the timeline (`timeline.export`). */
  exportFrames: (frames: number) => Promise<void>;
  /** Copy a heightmap or mesh file into assets/ and add an input node that reads it. */
  importAsset: (path: string) => Promise<void>;
  /** Fetch a real-world DEM around a lon/lat, place the world in its CRS and wire it to the height output. */
  fetchDem: (center: [number, number], radiusM: number, provider: "terrarium" | "copernicus" | "opentopography") => Promise<void>;
  save: () => Promise<void>;
  /** Bumped when a settings edit is refused so the inputs snap back to the stored values. */
  settingsRev: number;
  /** Change project settings (journaled); fields left out keep their value. */
  setSettings: (patch: JdOps["project.set_settings"]["request"]) => Promise<void>;
  /** Live-preview target per evaluation, ms (0 = off): the preview LOD adapts to stay under it. */
  previewTargetMs: number;
  /** Longest side of the viewport preview in samples (1024 default, 2048 = high detail); browser store. */
  previewMaxPx: number;
  setPreviewMaxPx: (px: number) => void;
  /** Extra LOD steps the adaptive policy currently adds to the project's preview LOD. */
  previewLodBias: number;
  setPreviewTargetMs: (ms: number) => void;
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
    // Preview LOD keeps the buffer at ≤ previewMaxPx² samples (1024² ≈ 1 M by default; 2048² is the
    // high-detail option: 4× the samples, 8 MB per transfer, ~8 M triangles).
    const lod = Math.max(0, Math.ceil(Math.log2(Math.max(layer.size_px[0], layer.size_px[1]) / get().previewMaxPx)));
    try {
      const { preview, bytes, ms } = await layerPreview(layer.layer, lod);
      set({ preview, previewStats: { bytes, ms, lod } });
      await refreshColor(lod);
      await refreshMaps();
    } catch (e) {
      logFailure("layer_preview", e);
    }
  };
  // Material maps are baked from the full field — only for height-like layers (masks have no
  // relief to shade) and only when PBR texturing is on; a failure just leaves the ramp colours.
  // Live previews (lod > 0) and sculpt strokes keep the previous maps: a full-field bake per
  // brush stroke would cost more than the stroke itself on large worlds.
  const dropMaps = () => {
    const old = get().maps;
    if (old) for (const b of [old.albedo, old.normal, old.roughness, old.detailWeights, ...old.details.flatMap((d) => [d.albedo, d.normal])]) b?.close();
  };
  const refreshMaps = async () => {
    const { layer, pbr, lastEval, sculpting } = get();
    if (!layer || !pbr || !layer.layer.split("#")[0]?.startsWith("height")) {
      dropMaps();
      return set({ maps: null });
    }
    if ((lastEval?.lod ?? 0) > 0 || sculpting) return;
    try {
      const r = await layerTextures(layer.layer, get().previewMaxPx, get().materials);
      // Toggled off or moved to another layer while baking: this result is stale.
      if (!get().pbr || get().layer?.layer !== layer.layer) {
        if (r) for (const b of [r.maps.albedo, r.maps.normal, r.maps.roughness, r.maps.detailWeights, ...r.maps.details.flatMap((d) => [d.albedo, d.normal])]) b?.close();
        return;
      }
      dropMaps();
      set({ maps: r?.maps ?? null, mapsMs: r?.ms ?? 0 });
      if (r) {
        const n = get().materials.length;
        set((st) => ({ log: [...st.log.slice(-199), { ts: Date.now(), level: "info", text: `terrain.textures: ok ${r.maps.px[0]}×${r.maps.px[1]}${n ? ` with ${n} material layer${n === 1 ? "" : "s"}` : ""} in ${r.ms.toFixed(0)} ms` }] }));
      }
    } catch (e) {
      logFailure("terrain.textures", e);
      dropMaps();
      set({ maps: null });
    }
  };
  // The colour layer is fetched at the same LOD as the shown layer; a layer that vanished from
  // the last evaluation drops back to the height ramp.
  const refreshColor = async (lod: number) => {
    const { colorLayer, lastEval } = get();
    if (!colorLayer) return;
    if (lastEval && !(colorLayer in lastEval.layers)) return set({ colorLayer: null, colorPreview: null });
    try {
      const { preview } = await layerPreview(colorLayer, lod);
      set({ colorPreview: preview });
    } catch (e) {
      logFailure("layer_preview (colour)", e);
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
  // Scatter request from the panel settings; heights come from the `height` layer when published.
  const scatterReq = () => {
    const { scatter, lastEval } = get();
    const names = Object.keys(lastEval?.layers ?? {});
    // grow: the population follows the timeline (opt-in — at t = 0 nobody is born yet); scrubbing re-places it at the new time.
    return { mask: scatter.mask ?? "", height: names.includes("height") ? "height" : (names[0] ?? "height"), spacing_m: scatter.spacing_m, seed: scatter.seed, follow_time: scatter.grow };
  };
  // Scatter instances follow the evaluation: re-placed while the mask is still published, dropped otherwise.
  const refreshScatter = async () => {
    const { scatter, lastEval } = get();
    if (!scatter.mask) return;
    if (lastEval && !(scatter.mask in lastEval.layers)) return set({ scatter: { ...scatter, mask: null, points: null } });
    await get().placeScatter();
  };
  // After an evaluation: keep showing the layer the user is looking at (same name, or its
  // full-res/preview twin) when the graph still publishes it; otherwise the first one.
  const adopt = async (report: EvalReport) => {
    // A filtered evaluation (`only <output>`) republishes one chain; the other layers are still
    // published, so the header keeps listing them.
    const known = get().lastEval?.layers ?? {};
    set({ lastEval: { ...report, layers: { ...known, ...report.layers } } });
    const names = Object.keys(report.layers);
    if (names.length === 0) return;
    const current = get().layer?.layer.split("#")[0];
    const keep = names.find((n) => n.split("#")[0] === current);
    await showLayer(keep ?? names[0]!);
    await refreshScatter();
    await get().refreshHorizon();
  };
  // Preview LOD keeps live evaluations ≤ 512² so edits feel immediate; Evaluate/export use lod 0.
  const previewLod = () => {
    const res = get().project?.world.resolution_px ?? [512, 512];
    return Math.max(0, Math.ceil(Math.log2(Math.max(res[0], res[1]) / 512))) + get().previewLodBias;
  };
  let liveTimer: ReturnType<typeof setTimeout> | undefined;
  let timeTimer: ReturnType<typeof setTimeout> | undefined;
  let skyTimer: ReturnType<typeof setTimeout> | undefined;
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
          // The graph moved on while this ran (Generate's edits, a quick second edit): the result
          // is of an older graph and would overwrite the newer layer — drop it, the next one is queued.
          if (gen !== graphGen) return;
          if (r.status === "ok" && r.result) {
            const { previewTargetMs, previewLodBias } = get();
            const bias = adaptLodBias(previewLodBias, r.result.duration_ms, previewTargetMs);
            if (bias !== previewLodBias) set({ previewLodBias: bias });
          }
          if (r.status === "ok" && r.result) await adopt(r.result);
          else if (r.error?.code === "E_GRAPH_MISSING_INPUT") {
            // Half-wired graphs are normal while editing: say what is missing, without an error entry.
            set((s) => ({ log: [...s.log.slice(-199), { ts: Date.now(), level: "info", text: `live preview waiting: ${r.error?.message ?? ""}` }] }));
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
  // The stroke node feeding the output that publishes `layer` — spliced in when there is none
  // (clamped for masks). Returns its id, or null when nothing publishes that layer.
  // Splice a new node of `kind` between whatever feeds the output publishing `layer` and that output.
  const spliceBefore = async (layer: string, kind: string, params: Record<string, unknown> | null): Promise<string | null> => {
    const g = get().graph;
    const output = g.nodes.find((n) => n.kind.startsWith("jd.output.height@") && String(n.params?.layer ?? "height") === layer);
    if (!output) return null;
    const feed = g.edges.find((e) => e.to.node === output.id && e.to.port === "in");
    const pos = output.ui?.position ?? [0, 0];
    const below = g.nodes.filter((n) => Math.abs((n.ui?.position?.[0] ?? 0) - pos[0]) < 1).length; // stack spliced nodes under the output
    graphGen++;
    const r = await run("graph.add_node", { kind, position: [pos[0], pos[1] + 140 * below], ...(params ? { params } : {}) });
    if (r.status !== "ok" || !r.result) return null;
    const id = r.result.id;
    if (feed) await get().connectNodes(feed.from.node, feed.from.port, id, "in");
    await get().connectNodes(id, "out", output.id, "in");
    await loadGraph();
    return id;
  };
  const strokesNodeFor = async (layer: string, clamp: [number, number] | null): Promise<string | null> => {
    const g = get().graph;
    const output = g.nodes.find((n) => n.kind.startsWith("jd.output.height@") && String(n.params?.layer ?? "height") === layer);
    if (!output) return null;
    const feed = g.edges.find((e) => e.to.node === output.id && e.to.port === "in");
    const existing = feed ? g.nodes.find((n) => n.id === feed.from.node && n.kind.startsWith("jd.sculpt.strokes@")) : undefined;
    if (existing) return existing.id;
    return spliceBefore(layer, "jd.sculpt.strokes@1", clamp ? { clamp } : null);
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
    maps: null,
    mapsMs: 0,
    waves: null,
    loadWaves: async (wind) => {
      if (!wind) return set({ waves: null });
      const r = await run("ocean.waves", wind);
      set({ waves: r.status === "ok" ? (r.result ?? null) : null });
    },
    materials: [],
    setMaterials: (materials) => {
      set({ materials });
      void refreshMaps();
    },
    pbr: true,
    setPbr: (on) => {
      set({ pbr: on });
      void refreshMaps();
    },
    colorLayer: null,
    colorPreview: null,
    setColorLayer: async (name) => {
      set({ colorLayer: name, colorPreview: null });
      if (name) await refreshColor(get().previewStats?.lod ?? 0);
    },
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
    diag: {},
    runDiag: async (what) => {
      if (what === "env") {
        const r = await run("diag.env");
        if (r.status === "ok" && r.result) set((s) => ({ diag: { ...s.diag, env: r.result } }));
      } else if (what === "gc") {
        const r = await run("project.cache_gc");
        if (r.status === "ok" && r.result) set((s) => ({ diag: { ...s.diag, gc: r.result } }));
      } else {
        // Small scene so it stays interactive; the CLI's `jd diag bench s1` is the real benchmark.
        const r = await run("diag.bench", { scene: "s1", size_px: 256 });
        if (r.status === "ok" && r.result) set((s) => ({ diag: { ...s.diag, bench: r.result } }));
      }
    },
    cancelJob: async (id) => {
      await op("jobs.cancel", { id }).catch((e) => logFailure("jobs.cancel", e));
    },
    selectedNode: null,
    setSelectedNode: (id) => set({ selectedNode: id }),
    sculpting: false,
    setSculpting: (on) => set({ sculpting: on, ...(on ? {} : { drawing: null }) }),
    brush: { radius_m: 150, strength_m: 40, mode: "raise", amount: 0.5 },
    sun: { azimuth_deg: 140, elevation_deg: 35, turbidity: 3, clouds: 0, fog: 0 },
    setSun: (patch) => {
      set((s) => ({ sun: { ...s.sun, ...patch } }));
      clearTimeout(skyTimer);
      skyTimer = setTimeout(() => void get().refreshSky(), 150); // a slider drag re-bakes once it settles
    },
    horizon: null,
    showHorizon: false,
    setShowHorizon: (on) => {
      set({ showHorizon: on, ...(on ? {} : { horizon: null }) });
      if (on) void get().refreshHorizon();
    },
    refreshHorizon: async () => {
      if (!get().showHorizon || !get().project) return;
      try {
        const preview = await horizonPreview(3, 256);
        set({ horizon: preview ? { preview, scale: 3 } : null });
      } catch (e) {
        logFailure("terrain.horizon", e);
      }
    },
    sky: null,
    refreshSky: async () => {
      const { azimuth_deg, elevation_deg, turbidity, clouds } = get().sun;
      try {
        const r = await skyPreview({ sun_azimuth_deg: azimuth_deg, sun_elevation_deg: Math.max(1, elevation_deg), turbidity, cloud_cover: clouds });
        const old = get().sky;
        set({ sky: r });
        old?.bitmap.close();
        if (r) set((st) => ({ log: [...st.log.slice(-199), { ts: Date.now(), level: "info", text: `sky.preview: ok ${r.px[0]}×${r.px[1]} sun ${elevation_deg}°/${azimuth_deg}° turbidity ${turbidity} clouds ${Math.round(clouds * 100)} %` }] }));
      } catch (e) {
        logFailure("sky.preview", e);
      }
    },
    scatter: { mask: null, spacing_m: 40, seed: 1, grow: false, points: null },
    setScatter: (patch) => set((s) => ({ scatter: { ...s.scatter, ...patch, points: null } })),
    placeScatter: async () => {
      if (!get().scatter.mask) return;
      const r = await op<JdOps["terrain.scatter"]["result"]>("terrain.scatter", scatterReq());
      log(r);
      if (r.status === "ok" && r.result) set((s) => ({ scatter: { ...s.scatter, points: r.result?.points ?? [] } }));
    },
    exportScatter: async () => {
      const { mask } = get().scatter;
      if (!mask) return;
      log(await op("terrain.scatter", { ...scatterReq(), path: `${mask}.glb` }));
    },
    setBrush: (patch) => set((s) => ({ brush: { ...s.brush, ...patch } })),
    startSculpting: async () => {
      const id = await strokesNodeFor("height", null);
      if (id) set({ selectedNode: id, sculpting: true });
    },
    paintDensity: async () => {
      const mask = get().scatter.mask;
      if (!mask) return;
      const id = await strokesNodeFor(mask, [0, 1]);
      if (id) set((s) => ({ selectedNode: id, sculpting: true, brush: { ...s.brush, mode: "raise", strength_m: 0.5 } }));
    },
    drawing: null,
    roadWidthM: 12,
    setRoadWidth: (m) => set({ roadWidthM: m }),
    startRoad: () => set({ drawing: "road", sculpting: true }),
    addStroke: async (points) => {
      if (get().drawing === "road") {
        // One drag = one road: a spline node spliced in front of the height output; back to orbiting after.
        set({ drawing: null, sculpting: false });
        if (points.length < 2) return;
        const rounded = points.map(([u, v]) => [Math.round(u * 10) / 10, Math.round(v * 10) / 10]);
        const id = await spliceBefore("height", "jd.terrain.spline@1", { points: rounded, width_m: get().roadWidthM });
        if (id) set({ selectedNode: id });
        return;
      }
      const { selectedNode, graph, brush } = get();
      const node = graph.nodes.find((n) => n.id === selectedNode && n.kind.startsWith("jd.sculpt.strokes@"));
      if (!node || points.length === 0) return;
      // One journal entry per stroke (graph.append_param), not the whole array each time.
      await mutate("graph.append_param", { id: node.id, key: "strokes", value: { points: points.map(([u, v]) => [Math.round(u * 10) / 10, Math.round(v * 10) / 10]), radius_m: brush.radius_m, strength_m: brush.strength_m, mode: brush.mode, amount: brush.amount } });
    },
    setLive: (on) => {
      set({ live: on });
      if (on) liveEvaluate();
    },
    setNoise: (patch) => set((s) => ({ noise: { ...s.noise, ...patch } })),
    loadGraph,
    importAsset: async (path) => {
      const r = await run("project.import_asset", { path });
      if (r.status !== "ok" || !r.result) return;
      const right = Math.max(-320, ...get().graph.nodes.map((n) => n.ui?.position?.[0] ?? 0));
      graphGen++;
      const add = await run("graph.add_node", { kind: "jd.input.heightmap@1", params: { path: r.result.name }, position: [right + 320, 0] });
      if (add.status === "ok") await loadGraph();
    },
    fetchDem: async (center, radiusM, provider) => {
      const r = await run("geo.fetch_dem", { center, radius_m: radiusM, provider, res_m: 30 });
      if (r.status !== "ok" || !r.result) return;
      const dem = r.result;
      await get().setSettings({ crs: dem.crs, origin: dem.origin });
      const right = Math.max(-320, ...get().graph.nodes.map((n) => n.ui?.position?.[0] ?? 0));
      graphGen++;
      const add = await run("graph.add_node", { kind: "jd.input.heightmap@1", params: { path: dem.asset, georeference: true }, position: [right + 320, 0] });
      if (add.status !== "ok" || !add.result) return;
      // The starter noise feeds the height output; the DEM takes its place.
      const out = get().graph.nodes.find((n) => n.kind.startsWith("jd.output.height"));
      if (out) {
        for (const e of get().graph.edges.filter((e) => e.to.node === out.id && e.to.port === "in")) await run("graph.disconnect", { from: e.from.node, from_port: e.from.port, to: e.to.node, to_port: e.to.port });
        await run("graph.connect", { from: add.result.id, from_port: "out", to: out.id, to_port: "in" });
      }
      await loadGraph();
      await get().evaluateGraph(0);
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
    newProject: async (path, name, extent, resolution) => {
      const r = await run("project.create", {
        path,
        name,
        world: { extent_m: [extent, extent], resolution_px: [resolution, resolution], tile_size: 512, halo: 4, origin: [0, 0], seed: 42 },
      });
      if (r.status === "ok" && r.result) {
        set({ project: r.result, layer: null, preview: null, lastEval: null }); // a new project publishes its own layers
        await loadGraph();
      }
    },
    openProject: async (path) => {
      const r = await run("project.open", { path });
      if (r.status === "ok" && r.result) {
        set({ project: r.result, layer: null, preview: null, lastEval: null }); // a new project publishes its own layers
        await loadGraph();
      }
    },
    generate: async () => {
      // One source of truth: the panel's settings go into the graph's noise node and the graph is
      // evaluated — so Generate, Evaluate and the live preview all show the same terrain. Without
      // a noise node (a graph built from an imported heightmap) the standalone op fills `height`.
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
      if (r.status === "ok" && r.result) {
        set({ layer: r.result });
        await refreshPreview();
      }
    },
    exportWater: async () => {
      const sea = get().graph.nodes.find((n) => n.kind.startsWith("jd.hydrology.sea_level@"));
      if (!sea) return;
      const p = sea.params ?? {};
      const tint = Array.isArray(p.tint) && p.tint.length === 3 ? (p.tint.map(Number) as [number, number, number]) : undefined;
      await run("ocean.bake", {
        level_m: Number(p.level_m ?? 0),
        visibility_m: Number(p.visibility_m ?? 15),
        ...(tint ? { tint } : {}),
        wind_speed_mps: Number(p.wind_speed_mps ?? get().project?.world.wind?.speed_mps ?? 8),
        fetch_km: p.fetch_km == null ? null : Number(p.fetch_km),
        wind_azimuth_deg: Number(p.wind_azimuth_deg ?? get().project?.world.wind?.azimuth_deg ?? 270),
      });
    },
    exportFrames: async (frames) => {
      await run("timeline.export", { frames, dir: "frames", format: "png16" });
    },
    exportHeightmap: async (path, format) => {
      // A live preview only published `height#lod<N>`; exports always use the full-resolution layer.
      if (get().layer?.layer.includes("#lod")) {
        const r = await run("graph.evaluate", { lod: 0 });
        if (r.status !== "ok") return;
      }
      await run("terrain.export_heightmap", { layer: "height", path, format });
    },
    previewMaxPx: readPreviewMaxPx(),
    setPreviewMaxPx: (px) => {
      const v = px === 2048 ? 2048 : 1024;
      set({ previewMaxPx: v });
      try {
        localStorage.setItem("jd.previewMaxPx", String(v));
      } catch {
        /* no storage */
      }
      void refreshPreview();
    },
    previewTargetMs: readPreviewTarget(),
    previewLodBias: 0,
    setPreviewTargetMs: (ms) => {
      set({ previewTargetMs: Math.max(0, ms), previewLodBias: 0 });
      try {
        localStorage.setItem("jd.previewTargetMs", String(Math.max(0, ms)));
      } catch {
        /* a private window or blocked storage: the setting lasts the session */
      }
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
