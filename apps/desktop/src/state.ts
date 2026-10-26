// Frontend store: a mirror of what the core reports, never the source of truth (docs/architecture/12-ui-workspace.md).
import { create } from "zustand";
import type { Edge, GraphEvaluateResult, GraphListResult, HeightmapFormat, JdOps, JobInfo as JobInfoGen, NodeDefInfo, NodeView, NoiseParams, OceanWavesRequest, OceanWavesResult, OpId, ProjectOpenResult, ProjectSaveResult, ScatterPoint, TerrainStatsResult } from "./lib/ops.gen";
import { horizonPreview, layerPreview, layerTextures, op, skyPreview, type JdError, type LayerMaps, type Preview, type Response, type Warning, hdriPreview, writeExport } from "./lib/api";
import { orbitAt } from "./lib/camera";
import type { CavesMeshResult, Entrance } from "./lib/ops.gen";

/** Cave kinds of `jd.cave.system@1`. */
export type CaveKindName = "karst" | "lava_tube" | "sea_cave" | "mountain" | "mine" | "tunnel" | "catacombs" | "maze" | "flank_margin";
/** Default reach per kind, metres (scaled down on small worlds). */
interface MeshData {
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
}
/** A cave as the viewport draws it (terrain frame: x east, y up, z south, centred). */
export interface CaveView {
  id: string;
  kind: string;
  walls: MeshData;
  spel: MeshData;
  waterLevelM: number | null;
  entrances: Entrance[];
  lengthM: number;
  notes: string[];
  voxelM: number;
  speleothems: number;
  error?: string;
}
const bytesOf = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
const f32Of = (b64: string) => {
  const b = bytesOf(b64);
  return new Float32Array(b.buffer, b.byteOffset, b.byteLength / 4);
};
const u32Of = (b64: string) => {
  const b = bytesOf(b64);
  return new Uint32Array(b.buffer, b.byteOffset, b.byteLength / 4);
};
const meshOf = (m: { positions_base64: string; normals_base64: string; indices_base64: string } | undefined): MeshData =>
  m ? { positions: f32Of(m.positions_base64), normals: f32Of(m.normals_base64), indices: u32Of(m.indices_base64) } : { positions: new Float32Array(), normals: new Float32Array(), indices: new Uint32Array() };
function caveView(id: string, kind: string, r: CavesMeshResult | null): CaveView {
  return {
    id,
    kind,
    walls: meshOf(r?.walls),
    spel: meshOf(r?.speleothems),
    waterLevelM: r?.water_level_m ?? null,
    entrances: r?.entrances ?? [],
    lengthM: r?.length_m ?? 0,
    notes: r?.notes ?? [],
    voxelM: r?.voxel_m ?? 0,
    speleothems: r?.speleothem_count ?? 0,
  };
}
import { storedLang } from "./lib/i18n";
import { adaptLodBias } from "./lib/previewLod";
import type { MaterialSpec, MediaMetadataResult, TerrainMeasureResult } from "./lib/ops.gen";

export type ProjectSummary = ProjectOpenResult;

export type LayerInfo = TerrainStatsResult;

/** The noise inspector's fields: every generator parameter except the sampling offset. */
export type NoiseSettings = Required<Omit<NoiseParams, "offset_m" | "warp" | "octave_weights" | "strength">>;

/** The viewport brush. `lower` is `raise` with a negative strength; `target_m` is flatten's height (null = under the first point). */
export interface Brush {
  radius_m: number;
  strength_m: number;
  mode: "raise" | "lower" | "smooth" | "flatten" | "rotate" | "scale" | "tilt" | "push" | "align";
  amount: number;
  /** Rotate: degrees per drag (counter-clockwise). */
  angle_deg: number;
  /** Scale: relief factor about the footprint's mean. */
  factor: number;
  /** Tilt: slope angle, degrees; the drag direction sets which way it rises. */
  tilt_deg: number;
  /** Brush alpha: the pattern inside the footprint. */
  alpha: "plain" | "noise" | "crater" | "ridges" | "rocks" | "dots" | "checker";
  /** Edge hardness 0..1 (0 = soft falloff over the whole radius, 1 = hard edge). */
  hardness: number;
  target_m: number | null;
}

export type SimulationKind = "snow" | "debris" | "sand" | "lava";

/** The grid tool's state: `selected` holds `"ix,iy"` keys; `smoothness` 0..1 softens the selection's edge. */
export interface GridTool {
  on: boolean;
  /** Cells per side of the world. */
  cells: number;
  selected: string[];
  smoothness: number;
  height_m: number;
  step_m: number;
  /** Transform settings for the selection: rotation (degrees), relief factor, tilt angle and its direction. */
  angle_deg: number;
  factor: number;
  tilt_deg: number;
  tilt_azimuth_deg: number;
}

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
  /** Depth preview (metres) of a published `lakes` layer (`jd.hydrology.fill@1`), drawn as standing water; null when none. */
  lakesPreview: Preview | null;
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
  brush: Brush;
  /** Multiply the brush radius or strength by `factor` (Ctrl/Shift + wheel in the viewport). */
  adjustBrush: (what: "radius" | "strength", factor: number) => void;
  /** Grid tool: cells of the world selected in the viewport and levelled, raised or lowered as one square-stamp stroke. */
  grid: GridTool;
  setGrid: (patch: Partial<Omit<GridTool, "on" | "selected">>) => void;
  /** Turn the grid tool on (leaves sculpt mode) or off. */
  toggleGrid: (on: boolean) => void;
  /** Add, remove or toggle cells (`[ix, iy]` from the south-west corner) in the selection. */
  gridSelect: (cells: [number, number][], op: "add" | "remove" | "toggle") => void;
  gridClear: () => void;
  /** One journaled stroke over the selected cells: `set` flattens them to `height_m`, `raise` / `lower` move them by `step_m`. */
  gridApply: (kind: "set" | "raise" | "lower") => Promise<void>;
  /** A transform of the selection (its bounding square) appended to the transforms node. */
  gridTransform: (kind: "duplicate" | "rotate" | "scale" | "tilt" | "align") => Promise<void>;
  /** Viewport sun direction (not part of the project). */
  sun: { azimuth_deg: number; elevation_deg: number; turbidity: number; clouds: number; fog: number; /** Rain in the viewport 0..1 (look only, not saved). */ rain: number };
  setSun: (patch: Partial<State["sun"]>) => void;
  /** UI language (persisted in the browser store). */
  lang: import("./lib/i18n").Lang;
  setLang: (lang: import("./lib/i18n").Lang) => void;
  /** Record a crash the UI caught (diag.crash_report) and note it in the log. */
  reportCrash: (message: string, stack: string, source: string) => Promise<void>;
  /** The noise continued beyond the border (`terrain.horizon`), drawn as a coarse skirt under the terrain. */
  horizon: { preview: Preview; scale: number } | null;
  showHorizon: boolean;
  setShowHorizon: (on: boolean) => void;
  refreshHorizon: () => Promise<void>;
  /** The physical sky (`sky.preview`) for the viewport dome, re-baked when the sun moves. */
  sky: { bitmap: ImageBitmap; px: [number, number] } | null;
  /** An imported HDRI on the dome (wins over the physical sky until cleared) and the sun it implies. */
  hdri: { path: string; bitmap: ImageBitmap; px: [number, number]; sun: { azimuth_deg: number; elevation_deg: number } } | null;
  loadHdri: (path: string) => Promise<void>;
  clearHdri: () => void;
  /** Viewport post effects (viewport only; exports untouched). */
  post: { exposure: number; bloom: number; vignette: number };
  setPost: (patch: Partial<{ exposure: number; bloom: number; vignette: number }>) => void;
  /** The viewport's capture handle (set by the viewport when mounted). */
  viewport: import("./viewport/TerrainView").ViewportHandle | null;
  setViewportHandle: (h: import("./viewport/TerrainView").ViewportHandle | null) => void;
  /** Flythrough: orbit keys in order; frames interpolate between them. */
  flyKeys: import("./viewport/TerrainView").Orbit[];
  addFlyKey: () => void;
  clearFlyKeys: () => void;
  /** Drop one key; move the camera to one. */
  removeFlyKey: (i: number) => void;
  goToFlyKey: (i: number) => void;
  /** Play the path in the viewport (no frames written), over `seconds`. */
  previewFlythrough: (seconds: number) => Promise<void>;
  /** Save the current frame as exports/stills/view_<n>.png. */
  saveStill: () => Promise<void>;
  /** Render `frames` along the keys into exports/flythrough/frame_<k>.png. */
  recordFlythrough: (frames: number) => Promise<void>;
  refreshSky: () => Promise<void>;
  /** Scatter preview: instances placed from a 0..1 mask layer and drawn in the viewport; export writes a GLB. */
  scatter: { mask: string | null; spacing_m: number; seed: number; grow: boolean; points: ScatterPoint[] | null };
  setScatter: (patch: Partial<{ mask: string | null; spacing_m: number; seed: number; grow: boolean }>) => void;
  placeScatter: () => Promise<void>;
  /** `model`: a binary FBX in the project's assets/ to instance instead of the proxy cone. */
  exportScatter: (model?: string) => Promise<void>;
  setBrush: (patch: Partial<Brush>) => void;
  /** Insert a sculpt node in front of the first output (or select the existing one) and enter sculpt mode. */
  startSculpting: () => Promise<void>;
  /** One click, one simulation layer (World Creator's Snow / Debris / Sand / Fluid): the node goes in front of
   *  the height output (or beside it for snow, which does not move the terrain), its masks are published
   *  as layers, a material row takes the first mask, and the viewport colours by it. */
  addSimulation: (which: SimulationKind) => Promise<void>;
  /** A whole-terrain action (`jd.filter.action@1`) spliced in front of the height output. */
  applyAction: (action: "flip_x" | "flip_y" | "rotate_left" | "rotate_right" | "flatten") => Promise<void>;
  /** Paint the scatter mask: a clamped (0..1) stroke node in front of the mask's output, brush strength 0.5, sculpt mode on. */
  paintDensity: () => Promise<void>;
  /** What the next viewport drag draws: a road (a spline node in front of the height output), or nothing (brush strokes). */
  drawing: "road" | "cave" | null;
  /** Arm the next drag to lay a road; the road's width in metres comes from `roadWidthM`. */
  startRoad: () => void;
  roadWidthM: number;
  setRoadWidth: (m: number) => void;
  /** Cave systems (`jd.cave.system@1` nodes) as viewport meshes, refreshed after each evaluation. */
  caves: CaveView[];
  showCaves: boolean;
  setShowCaves: (on: boolean) => void;
  /** Terrain see-through 0..1 so the caves under it show (0 = solid). */
  xray: number;
  setXray: (x: number) => void;
  /** The cave whose entrances the next viewport clicks place. */
  caveTarget: string | null;
  /** Splice a cave node of `kind` in front of the height output and publish its holes and plan. */
  addCave: (kind: CaveKindName) => Promise<void>;
  /** Clicks on the terrain now add entrances to cave `id` (until the tool is switched off). */
  startCaveEntrance: (id: string) => void;
  removeCaveEntrance: (id: string, i: number) => Promise<void>;
  /** `caves.export` of one cave (.glb / .fbx / .json under exports/). */
  exportCave: (id: string, path: string) => Promise<void>;
  /** Re-mesh the caves; with an evaluation report, caves whose node hash and lod did not change are kept. */
  refreshCaves: (lod?: number, report?: EvalReport) => Promise<void>;
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
  /** `graph.set_bypass`: pass a node's input through unchanged (the layer stack's eye). */
  setBypass: (id: string, bypass: boolean) => Promise<void>;
  /** Splice `kind` in front of the height output (the layer stack's +). */
  insertLayer: (kind: string, params?: Record<string, unknown> | null) => Promise<void>;
  /** Remove a chain node and reconnect what fed it to what it fed. */
  unsplice: (id: string) => Promise<void>;
  /** Move a chain node one step later (`up`, nearer the output) or earlier (`down`) by re-wiring its neighbours. */
  swapInChain: (id: string, dir: "up" | "down") => Promise<void>;
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
  /** `terrain.export_profile`: the height, every material mask as a weightmap and the material stack, for Unity or Unreal. */
  /** `tiles` > 1 cuts the export into tiles × tiles engine folders sharing one height range. */
  exportProfile: (profile: "unity" | "unreal", dir: string, tiles?: number, holes?: string | null) => Promise<void>;
  /** `terrain.export_mesh` with the material stack baked in; the extension picks the format. */
  /** `holes`: a 0..1 layer whose cells ≥ 0.5 are left out of the mesh. */
  exportMesh: (path: string, lods: number | null, targetTriangles: number | null, holes?: string | null) => Promise<void>;
  /** `sky.bake`: the viewport's sun, turbidity and clouds as an equirectangular HDRI. */
  exportSky: (path: string) => Promise<void>;
  /** Write exports/water/: depth, colour, foam and the wave normal tile for the graph's sea-level node (`ocean.bake`). */
  exportWater: () => Promise<void>;
  /** Write exports/frames/: one 16-bit heightmap per frame across the timeline (`timeline.export`). */
  exportFrames: (frames: number) => Promise<void>;
  /** Copy a heightmap or mesh file into assets/ and add an input node that reads it. */
  importAsset: (path: string) => Promise<void>;
  /** Fetch a real-world DEM around a lon/lat, place the world in its CRS and wire it to the height output. */
  fetchDem: (center: [number, number], radiusM: number, provider: "terrarium" | "copernicus" | "opentopography") => Promise<void>;
  /** `terrain.measure` of the shown layer: relief, hypsometric integral, slope statistics and geomorphon shares, to compare with a real landform. */
  measure: TerrainMeasureResult | null;
  measureLayer: () => Promise<void>;
  /** What a reference photo, panorama or video states (`media.metadata`): place, time, heading, field of view, the sun. */
  reference: MediaMetadataResult | null;
  readReference: (path: string) => Promise<void>;
  /** Put the reference's sun on the viewport sky (when it was above the horizon). */
  useReferenceSun: () => void;
  /** Fetch the real terrain around the reference's position (Terrarium, 5 km radius) and place the world on it. */
  terrainFromReference: () => Promise<void>;
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
  const refreshPreview = async () => {
    const { layer } = get();
    if (!layer) return;
    // Preview LOD keeps the buffer at ≤ previewMaxPx² samples (1024² ≈ 1 M by default; 2048² is the
    // high-detail option: 4× the samples, 8 MB per transfer, ~8 M triangles).
    const lod = Math.max(0, Math.ceil(Math.log2(Math.max(layer.size_px[0], layer.size_px[1]) / get().previewMaxPx)));
    try {
      const { preview, bytes, ms } = await layerPreview(layer.layer, lod);
      set({ preview, previewStats: { bytes, ms, lod } });
      // Standing water: a published layer named `lakes` (or `lakes#lod<N>` from a live preview) is
      // the fill node's depth map; shown as water lying on the terrain wherever it is above zero.
      const names = Object.keys(get().lastEval?.layers ?? {});
      const lakes = names.find((n) => n.split("#")[0] === "lakes" && n.includes("#") === layer.layer.includes("#")) ?? names.find((n) => n.split("#")[0] === "lakes");
      if (lakes) {
        try {
          set({ lakesPreview: (await layerPreview(lakes, lod)).preview });
        } catch (e) {
          logFailure("layer_preview (lakes)", e);
        }
      } else if (get().lakesPreview) set({ lakesPreview: null });
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
        set((st) => ({ log: appendLog(st.log, { ts: Date.now(), level: "info", text: `terrain.textures: ok ${r.maps.px[0]}×${r.maps.px[1]}${n ? ` with ${n} material layer${n === 1 ? "" : "s"}` : ""} in ${r.ms.toFixed(0)} ms` }) }));
      }
    } catch (e) {
      logFailure("terrain.textures", e);
      dropMaps();
      set({ maps: null });
    }
  };
  // The colour layer is fetched at the same LOD as the shown layer; a layer that vanished from
  // the last evaluation drops back to the height ramp.
  // A colour layer chosen before it exists (a simulation layer just added) waits for the
  // evaluation that publishes it instead of being dropped.
  let pendingColor: string | null = null;
  const refreshColor = async (lod: number) => {
    const { lastEval } = get();
    if (pendingColor && lastEval && pendingColor in lastEval.layers) {
      set({ colorLayer: pendingColor });
      pendingColor = null;
    }
    const { colorLayer } = get();
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
    set({ lastEval: { ...report, layers: { ...known, ...report.layers } }, measure: null });
    const names = Object.keys(report.layers);
    if (names.length === 0) return;
    const current = get().layer?.layer.split("#")[0];
    const keep = names.find((n) => n.split("#")[0] === current);
    await showLayer(keep ?? names[0]!);
    await refreshScatter();
    await get().refreshHorizon();
    void get().refreshCaves(report.lod, report);
  };
  let caveGen = 0;
  const caveHashes = new Map<string, string>();
  // Publish a cave's `holes` / `plan` as a layer; a second cave joins the first through a max node.
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
  // The stroke node feeding the output that publishes `layer` — spliced in when there is none
  // (clamped for masks). Returns its id, or null when nothing publishes that layer.
  // Splice a new node of `kind` between whatever feeds the output publishing `layer` and that output.
  // One implementation for the desktop and Studio (ADR-0017): the core's `graph.splice` puts the
  // node between the layer's output and its feed, stacked under the output on the canvas.
  const spliceBefore = async (layer: string, kind: string, params: Record<string, unknown> | null): Promise<string | null> => {
    graphGen++;
    const r = await run("graph.splice", { layer, kind, ...(params ? { params } : {}) });
    if (r.status !== "ok" || !r.result) return null;
    await loadGraph();
    return r.result.id;
  };
  // The sculpt node for a layer — strokes right in front of its output, transforms up its chain —
  // found or spliced in by the core (graph.sculpt_target, shared with Studio).
  const sculptTarget = async (layer: string, kind: "strokes" | "transforms", clamp: [number, number] | null): Promise<string | null> => {
    graphGen++;
    const r = await run("graph.sculpt_target", { layer, kind, ...(clamp ? { clamp } : {}) });
    if (r.status !== "ok" || !r.result) return null;
    await loadGraph();
    return r.result.id;
  };
  const strokesNodeFor = (layer: string, clamp: [number, number] | null) => sculptTarget(layer, "strokes", clamp);
  const transformsNodeFor = (layer: string) => sculptTarget(layer, "transforms", null);
  const r1 = (v: number) => Math.round(v * 10) / 10;
  type GraphView = GraphListResult;
  // The look (materials, scatter, sun) lives in scene.json: restored from the project summary on
  // open, written back (debounced, journaled) whenever a setter changes it.
  const lookState = (p: { look?: { materials?: MaterialSpec[]; scatter?: { mask?: string | null; spacing_m: number; seed?: number; grow?: boolean } | null; sun?: { azimuth_deg: number; elevation_deg: number; turbidity?: number; clouds?: number; fog?: number } | null } }) => {
    const look = p.look;
    const sc = look?.scatter;
    const su = look?.sun;
    return {
      materials: look?.materials ?? [],
      ...(sc ? { scatter: { mask: sc.mask ?? null, spacing_m: sc.spacing_m, seed: sc.seed ?? 0, grow: sc.grow ?? false, points: null } } : {}),
      ...(su ? { sun: { azimuth_deg: su.azimuth_deg, elevation_deg: su.elevation_deg, turbidity: su.turbidity ?? 3, clouds: su.clouds ?? 0, fog: su.fog ?? 0, rain: 0 } } : {}),
    };
  };
  let lookTimer: ReturnType<typeof setTimeout> | undefined;
  let lookPending = false;
  // A background write like autosave: `op` + `log`, never `run` — `run` flips `busy`, which
  // disables every gated button for the round trip, and a click landing in that window is lost
  // (seen as e2e flakes: Paint density ignored while the look was being written).
  const writeLook = async () => {
    lookPending = false;
    const { project, materials, scatter, sun } = get();
    if (!project) return;
    await op<JdOps["project.set_look"]["result"]>("project.set_look", { materials, scatter: { mask: scatter.mask, spacing_m: scatter.spacing_m, seed: scatter.seed, grow: scatter.grow }, sun: { azimuth_deg: sun.azimuth_deg, elevation_deg: sun.elevation_deg, turbidity: sun.turbidity, clouds: sun.clouds, fog: sun.fog } }).then(log, (e) => logFailure("project.set_look", e));
  };
  const pushLook = () => {
    clearTimeout(lookTimer);
    lookPending = true;
    lookTimer = setTimeout(() => void writeLook(), 400);
  };
  // A save must carry the look as it is now, not as it was 400 ms ago: flush the pending write
  // first (seen as an e2e race: Save right after a spacing edit stored the previous spacing).
  const flushLook = async () => {
    if (!lookPending) return;
    clearTimeout(lookTimer);
    await writeLook();
  };
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
    await flushLook();
    const r = await op<ProjectSaveResult>("project.save");
    log(r);
    if (r.status === "ok" && r.result) set({ project: r.result });
  };
  // The Noise panel mirrors the graph's noise node: whenever the node's params change (a project
  // opened, an inspector edit, undo) the panel adopts them, so Generate — which writes the panel
  // into the node — never reverts an edit made elsewhere. Keyed (timeline) values stay as they are.
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
  const applyGraph = (r: Response<GraphView>) => {
    if (r.status === "ok" && r.result) {
      set({ graph: r.result });
      syncNoise(r.result);
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
    lakesPreview: null,
    maps: null,
    mapsMs: 0,
    waves: null,
    loadWaves: async (wind) => {
      if (!wind) return set({ waves: null });
      const r = await op<JdOps["ocean.waves"]["result"]>("ocean.waves", wind).catch((e: unknown) => (logFailure("ocean.waves", e), null)); // background (an effect on wind changes): no `busy`
      if (r) log(r);
      set({ waves: r?.status === "ok" ? (r.result ?? null) : null });
    },
    materials: [],
    setMaterials: (materials) => {
      set({ materials });
      pushLook();
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
    brush: { radius_m: 150, strength_m: 40, mode: "raise", amount: 0.5, hardness: 0, target_m: null, angle_deg: 30, factor: 1.5, tilt_deg: 15, alpha: "plain" },
    adjustBrush: (what, factor) =>
      set((s) => ({
        brush: what === "radius" ? { ...s.brush, radius_m: Math.max(1, Math.round(s.brush.radius_m * factor)) } : { ...s.brush, strength_m: Math.round(s.brush.strength_m * factor * 10) / 10 },
      })),
    grid: { on: false, cells: 32, selected: [], smoothness: 0.25, height_m: 0, step_m: 10, angle_deg: 90, factor: 1.5, tilt_deg: 10, tilt_azimuth_deg: 0 },
    setGrid: (patch) => set((s) => ({ grid: { ...s.grid, ...patch } })),
    toggleGrid: (on) => set((s) => ({ grid: { ...s.grid, on }, ...(on ? { sculpting: false, drawing: null } : {}) })),
    gridSelect: (cells, op) =>
      set((s) => {
        const sel = new Set(s.grid.selected);
        for (const [ix, iy] of cells) {
          const k = `${ix},${iy}`;
          if (op === "add" || (op === "toggle" && !sel.has(k))) sel.add(k);
          else sel.delete(k);
        }
        return { grid: { ...s.grid, selected: [...sel] } };
      }),
    gridClear: () => set((s) => ({ grid: { ...s.grid, selected: [] } })),
    gridApply: async (kind) => {
      const { grid, project } = get();
      if (!project || grid.selected.length === 0) return;
      const id = await strokesNodeFor("height", null);
      if (!id) return;
      // One square stamp per cell, hard-edged at smoothness 0; one stroke, so one undo step.
      const ext = project.world.extent_m;
      const [cw, ch] = [ext[0] / grid.cells, ext[1] / grid.cells];
      const points = grid.selected.map((k) => k.split(",").map(Number)).map(([ix = 0, iy = 0]) => [Math.round((ix + 0.5) * cw * 10) / 10, Math.round((iy + 0.5) * ch * 10) / 10]);
      const base = { points, radius_m: Math.round((Math.max(cw, ch) / 2) * 10) / 10, shape: "square", hardness: Math.round((1 - grid.smoothness) * 100) / 100, amount: 1 };
      const value = kind === "set" ? { ...base, mode: "flatten", target_m: grid.height_m } : { ...base, mode: "raise", strength_m: kind === "raise" ? grid.step_m : -grid.step_m };
      set({ selectedNode: id });
      await mutate("graph.append_param", { id, key: "strokes", value });
    },
    gridTransform: async (kind) => {
      const { grid, project } = get();
      if (!project || grid.selected.length === 0) return;
      const ext = project.world.extent_m;
      const [cw, ch] = [ext[0] / grid.cells, ext[1] / grid.cells];
      const cells = grid.selected.map((k) => k.split(",").map(Number) as [number, number]);
      const xs = cells.map(([x]) => x);
      const ys = cells.map(([, y]) => y);
      // The selection's bounding square: World Creator transforms the selected block as one piece.
      const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs) + 1, Math.min(...ys), Math.max(...ys) + 1];
      const centre = [r1(((x0 + x1) / 2) * cw), r1(((y0 + y1) / 2) * ch)];
      const half = r1((Math.max((x1 - x0) * cw, (y1 - y0) * ch)) / 2);
      const id = await transformsNodeFor("height");
      if (!id) return;
      set({ selectedNode: id });
      await mutate("graph.append_param", {
        id,
        key: "ops",
        value: {
          kind,
          center_m: centre,
          radius_m: half,
          shape: "square",
          falloff: Math.round(grid.smoothness * 100) / 100,
          angle_deg: kind === "tilt" ? grid.tilt_deg : grid.angle_deg,
          azimuth_deg: grid.tilt_azimuth_deg,
          factor: grid.factor,
          // Duplicate: the copy lands one selection-width east.
          vector_m: kind === "duplicate" ? [r1((x1 - x0) * cw), 0] : [0, 0],
          strength: 1,
        },
      });
    },
    lang: storedLang(),
    setLang: (lang) => {
      set({ lang });
      try {
        localStorage.setItem("jd.lang", lang);
      } catch {
        /* no storage: the choice lasts the session */
      }
    },
    reportCrash: async (message, stack, source) => {
      const r = await op<{ path: string }>("diag.crash_report", { message: message.slice(0, 4000), stack: stack.slice(0, 16000), source }).catch(() => null);
      set((st) => ({ log: appendLog(st.log, { ts: Date.now(), level: "error", text: `crash report${r?.status === "ok" && r.result ? `: ${r.result.path}` : " (not written)"} — ${message}` }) }));
    },
    sun: { azimuth_deg: 140, elevation_deg: 35, turbidity: 3, clouds: 0, fog: 0, rain: 0 },
    setSun: (patch) => {
      set((s) => ({ sun: { ...s.sun, ...patch } }));
      pushLook();
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
    post: { exposure: 1, bloom: 0, vignette: 0 },
    setPost: (patch) => set((st) => ({ post: { ...st.post, ...patch } })),
    viewport: null,
    setViewportHandle: (h) => set({ viewport: h }),
    flyKeys: [],
    addFlyKey: () => {
      const o = get().viewport?.getOrbit();
      if (o) set((st) => ({ flyKeys: [...st.flyKeys, o] }));
    },
    clearFlyKeys: () => set({ flyKeys: [] }),
    removeFlyKey: (i) => set((st) => ({ flyKeys: st.flyKeys.filter((_, k) => k !== i) })),
    goToFlyKey: (i) => {
      const key = get().flyKeys[i];
      if (key) get().viewport?.setOrbit(key);
    },
    previewFlythrough: async (seconds) => {
      const v = get().viewport;
      const keys = get().flyKeys;
      if (!v || keys.length < 2) return;
      const t0 = performance.now();
      for (;;) {
        const t = (performance.now() - t0) / (seconds * 1000);
        v.setOrbit(orbitAt(keys, Math.min(t, 1)));
        if (t >= 1) break;
        await new Promise((r) => requestAnimationFrame(() => r(null)));
      }
    },
    saveStill: async () => {
      const v = get().viewport;
      if (!v) return;
      const blob = await v.snapshot();
      if (!blob) return logFailure("still", "the canvas gave no image");
      const n = Date.now() % 100000;
      const path = await writeExport(`stills/view_${n}.png`, blob);
      set((st) => ({ log: appendLog(st.log, { ts: Date.now(), level: path ? "info" : "error", text: path ? `still: ${path} (${(blob.size / 1024).toFixed(0)} KB)` : "still: write failed" }) }));
    },
    recordFlythrough: async (frames) => {
      const v = get().viewport;
      const keys = get().flyKeys;
      if (!v || keys.length < 2 || !(frames >= 2)) return;
      const start = v.getOrbit();
      const at = (t: number) => orbitAt(keys, t); // Catmull-Rom through every key
      let written = 0;
      for (let k = 0; k < frames; k++) {
        v.setOrbit(at(k / (frames - 1)));
        await new Promise((r) => requestAnimationFrame(() => r(null)));
        const blob = await v.snapshot();
        if (blob && (await writeExport(`flythrough/frame_${String(k).padStart(4, "0")}.png`, blob))) written++;
      }
      v.setOrbit(start);
      set((st) => ({ log: appendLog(st.log, { ts: Date.now(), level: written === frames ? "info" : "warn", text: `flythrough: ${written}/${frames} frames → exports/flythrough/ (ffmpeg -framerate 24 -i frame_%04d.png …)` }) }));
    },
    hdri: null,
    loadHdri: async (path) => {
      const r = await hdriPreview(path).catch((e: unknown) => (logFailure("sky.load_hdri", e), null));
      const old = get().hdri;
      if (!r) return log({ op: "sky.load_hdri", schema_version: 1, request_id: "", status: "error", error: { code: "E_INPUT_INVALID", message: `could not read ${path}` } });
      set({ hdri: { path, bitmap: r.bitmap, px: r.px, sun: r.sun } });
      old?.bitmap.close();
      set((st) => ({ log: appendLog(st.log, { ts: Date.now(), level: "info", text: `sky.load_hdri: ok ${r.px[0]}×${r.px[1]} sun ${Math.round(r.sun.elevation_deg)}°/${Math.round(r.sun.azimuth_deg)}° luminance ${r.luminance.toFixed(2)}` }) }));
    },
    clearHdri: () => {
      const old = get().hdri;
      set({ hdri: null });
      old?.bitmap.close();
    },
    refreshSky: async () => {
      const { azimuth_deg, elevation_deg, turbidity, clouds, fog } = get().sun;
      try {
        const r = await skyPreview({ sun_azimuth_deg: azimuth_deg, sun_elevation_deg: Math.max(1, elevation_deg), turbidity, cloud_cover: clouds, fog });
        const old = get().sky;
        set({ sky: r });
        old?.bitmap.close();
        if (r) set((st) => ({ log: appendLog(st.log, { ts: Date.now(), level: "info", text: `sky.preview: ok ${r.px[0]}×${r.px[1]} sun ${elevation_deg}°/${azimuth_deg}° turbidity ${turbidity} clouds ${Math.round(clouds * 100)} %` }) }));
      } catch (e) {
        logFailure("sky.preview", e);
      }
    },
    scatter: { mask: null, spacing_m: 40, seed: 1, grow: false, points: null },
    setScatter: (patch) => {
      set((s) => ({ scatter: { ...s.scatter, ...patch, points: null } }));
      pushLook();
    },
    placeScatter: async () => {
      if (!get().scatter.mask) return;
      const r = await op<JdOps["terrain.scatter"]["result"]>("terrain.scatter", scatterReq());
      log(r);
      if (r.status === "ok" && r.result) set((s) => ({ scatter: { ...s.scatter, points: r.result?.points ?? [] } }));
    },
    exportScatter: async (model) => {
      const { mask } = get().scatter;
      if (!mask) return;
      log(await op("terrain.scatter", { ...scatterReq(), path: `${mask}.glb`, ...(model ? { model } : {}) }));
    },
    setBrush: (patch) => set((s) => ({ brush: { ...s.brush, ...patch } })),
    startSculpting: async () => {
      const id = await strokesNodeFor("height", null);
      if (id) set((s) => ({ selectedNode: id, sculpting: true, grid: { ...s.grid, on: false } }));
    },
    addSimulation: async (which) => {
      // The core places the node (in front of the height, or beside it for snow), publishes its
      // layers and adds its material rows (`graph.add_simulation`, shared with Studio).
      graphGen++;
      const r = await run("graph.add_simulation", { which, max_height_m: get().layer?.stats.max ?? 500 });
      if (r.status !== "ok" || !r.result) return;
      await loadGraph();
      if (r.result.materials) get().setMaterials(r.result.materials);
      // Colour by the first mask once the (live or explicit) evaluation has published it.
      pendingColor = r.result.published?.[0] ?? null;
      set({ selectedNode: r.result.id });
    },
    applyAction: async (action) => {
      const id = await spliceBefore("height", "jd.filter.action@1", { action });
      if (id) set({ selectedNode: id });
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
    caves: [],
    showCaves: true,
    setShowCaves: (on) => {
      set({ showCaves: on });
      if (on) void get().refreshCaves();
      else set({ caves: [] });
    },
    xray: 0,
    setXray: (x) => set({ xray: Math.min(0.95, Math.max(0, x)) }),
    caveTarget: null,
    addCave: async (kind) => {
      // Seed, reach and the published holes / plan layers come from the core (`graph.add_cave`).
      graphGen++;
      const r = await run("graph.add_cave", { kind });
      if (r.status !== "ok" || !r.result) return;
      await loadGraph();
      const id = r.result.id;
      // A tunnel needs its two portals: start placing right away.
      set({ selectedNode: id, caveTarget: id, ...(kind === "tunnel" ? { drawing: "cave" as const, sculpting: true } : {}) });
    },
    startCaveEntrance: (id) => set({ drawing: "cave", sculpting: true, caveTarget: id, selectedNode: id }),
    removeCaveEntrance: async (id, i) => {
      const node = get().graph.nodes.find((x) => x.id === id);
      const list = ((node?.params?.entrances as [number, number][] | undefined) ?? []).filter((_, k) => k !== i);
      await mutate("graph.set_param", { id, key: "entrances", value: list });
    },
    exportCave: async (id, path) => {
      await run("caves.export", { id, path });
    },
    refreshCaves: async (lod, report) => {
      const gen = ++caveGen;
      const nodes = get().graph.nodes.filter((x) => x.kind.startsWith("jd.cave.system@"));
      if (!get().showCaves || nodes.length === 0 || !get().project) {
        if (get().caves.length) set({ caves: [] });
        caveHashes.clear();
        return;
      }
      const at = lod ?? previewLod();
      const hashOf = (id: string) => report?.nodes.find((x) => x.node === id)?.hash;
      const out: CaveView[] = [];
      for (const n of nodes) {
        // Unchanged cave (same node hash = same params and inputs, same lod): keep its meshes.
        const h = hashOf(n.id);
        const prev = get().caves.find((c) => c.id === n.id);
        if (h && prev && !prev.error && caveHashes.get(n.id) === `${h}@${at}`) {
          out.push(prev);
          continue;
        }
        const r = await op<CavesMeshResult>("caves.mesh", { id: n.id, lod: at }).catch(() => null);
        if (gen !== caveGen) return; // a newer refresh is running
        const kind = String(n.params?.kind ?? "karst");
        if (r?.status === "ok" && r.result) {
          out.push(caveView(n.id, kind, r.result));
          if (h) caveHashes.set(n.id, `${h}@${at}`);
        }
        else out.push({ ...caveView(n.id, kind, null), error: r?.error?.message ?? "caves.mesh failed" });
      }
      set({ caves: out });
    },
    addStroke: async (points) => {
      if (get().drawing === "cave") {
        // Each click on the terrain places one entrance of the target cave; the tool stays on.
        const id = get().caveTarget;
        const p = points[points.length - 1];
        if (!id || !p) return;
        await mutate("graph.append_param", { id, key: "entrances", value: [Math.round(p[0] * 10) / 10, Math.round(p[1] * 10) / 10] });
        return;
      }
      if (get().drawing === "road") {
        // One drag = one road: a spline node spliced in front of the height output; back to orbiting after.
        set({ drawing: null, sculpting: false });
        if (points.length < 2) return;
        const rounded = points.map(([u, v]) => [Math.round(u * 10) / 10, Math.round(v * 10) / 10]);
        const id = await spliceBefore("height", "jd.terrain.spline@1", { points: rounded, width_m: get().roadWidthM });
        if (id) set({ selectedNode: id });
        return;
      }
      const xform = (["rotate", "scale", "tilt", "push", "align"] as const).find((m) => m === get().brush.mode);
      if (xform) {
        // A transform brush: the drag's start is the centre, its end gives the push vector and the tilt direction.
        if (points.length === 0) return;
        const b = get().brush;
        const [p0, p1] = [points[0]!, points[points.length - 1]!];
        const d = [p1[0] - p0[0], p1[1] - p0[1]];
        const id = await transformsNodeFor("height");
        if (!id) return;
        set({ selectedNode: id });
        await mutate("graph.append_param", {
          id,
          key: "ops",
          value: {
            kind: xform,
            center_m: [r1(p0[0]), r1(p0[1])],
            radius_m: b.radius_m,
            shape: "round",
            falloff: Math.round((1 - b.hardness) * 100) / 100,
            angle_deg: xform === "tilt" ? b.tilt_deg : b.angle_deg,
            azimuth_deg: Math.round(((Math.atan2(d[0]!, d[1]!) * 180) / Math.PI + 360) % 360),
            factor: b.factor,
            vector_m: [r1(d[0]!), r1(d[1]!)],
            strength: b.amount,
          },
        });
        return;
      }
      const { selectedNode, graph, brush } = get();
      const node = graph.nodes.find((n) => n.id === selectedNode && n.kind.startsWith("jd.sculpt.strokes@"));
      if (!node || points.length === 0) return;
      // One journal entry per stroke (graph.append_param), not the whole array each time.
      const lower = brush.mode === "lower";
      await mutate("graph.append_param", {
        id: node.id,
        key: "strokes",
        value: {
          points: points.map(([u, v]) => [Math.round(u * 10) / 10, Math.round(v * 10) / 10]),
          radius_m: brush.radius_m,
          strength_m: lower ? -Math.abs(brush.strength_m) : brush.strength_m,
          mode: lower ? "raise" : brush.mode,
          amount: brush.amount,
          hardness: brush.hardness,
          ...(brush.mode === "flatten" && brush.target_m != null ? { target_m: brush.target_m } : {}),
          ...(brush.alpha !== "plain" ? { alpha: brush.alpha } : {}),
        },
      });
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
    measure: null,
    measureLayer: async () => {
      const layer = get().layer?.layer.split("#")[0] ?? "height";
      const r = await run("terrain.measure", { layer });
      set({ measure: r.status === "ok" && r.result ? r.result : null });
    },
    reference: null,
    readReference: async (path) => {
      const r = await run("media.metadata", { path });
      set({ reference: r.status === "ok" && r.result ? r.result : null });
    },
    useReferenceSun: () => {
      const sun = get().reference?.sun;
      if (!sun || sun.below_horizon) return;
      get().setSun({ azimuth_deg: Math.round(sun.azimuth_deg) % 360, elevation_deg: Math.min(90, Math.max(3, Math.round(sun.elevation_deg))) });
    },
    terrainFromReference: async () => {
      const g = get().reference?.gps;
      if (!g) return;
      await get().fetchDem([g.lon, g.lat], 5000, "terrarium");
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
    setBypass: async (id, bypass) => mutate("graph.set_bypass", { id, bypass }),
    insertLayer: async (kind, params) => {
      const id = await spliceBefore("height", kind, params ?? null);
      if (id) set({ selectedNode: id });
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
    newProject: async (path, name, extent, resolution) => {
      const r = await run("project.create", {
        path,
        name,
        world: { extent_m: [extent, extent], resolution_px: [resolution, resolution], tile_size: 512, halo: 4, origin: [0, 0], seed: 42 },
      });
      if (r.status === "ok" && r.result) {
        // The graph first, then the project (as on open): the Noise panel mirrors the starter graph
        // before anything is editable, so a preset chosen right away is never overwritten by the load.
        set({ project: null, layer: null, preview: null, lastEval: null }); // a new project publishes its own layers
        await loadGraph();
        set({ project: r.result, ...lookState(r.result) });
        liveEvaluate();
      }
    },
    openProject: async (path) => {
      const r = await run("project.open", { path });
      if (r.status === "ok" && r.result) {
        // The graph first, then the project: the Noise panel shows the opened graph's values
        // before anything is editable, so a quick edit cannot be overwritten by the load. Opening
        // the project already shown (the start-up probe found it, then Open on the same path) keeps
        // it mounted: unmounting would drop an edit in progress (seen as an e2e flake: a settings
        // value typed before the Open round trip came back was wiped by the remount).
        const same = get().project?.project_id === r.result.project_id;
        if (!same) set({ project: null, layer: null, preview: null, lastEval: null });
        await loadGraph();
        set({ project: r.result, ...lookState(r.result) });
        void refreshMaps();
        liveEvaluate();
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
    exportProfile: async (profile, dir, tiles, holes) => {
      const materials = get().materials;
      const masks = [...new Set(materials.map((m) => m.mask).filter((m): m is string => !!m))];
      // The project's caves go along: their meshes in the engine's format, their mouths merged into holes.png.
      const caves = get().graph.nodes.some((n) => n.kind.startsWith("jd.cave.system@"));
      await run("terrain.export_profile", { profile, dir, layer: "height", masks, materials, ...(tiles && tiles > 1 ? { tiles } : {}), ...(holes ? { holes } : {}), ...(caves ? { caves: true } : {}) });
    },
    exportMesh: async (path, lods, targetTriangles, holes) => {
      // Without a holes layer chosen, the caves' mouths open the mesh (cave_holes, when published).
      const open = holes ?? (get().lastEval?.layers?.cave_holes ? "cave_holes" : null);
      await run("terrain.export_mesh", { layer: "height", path, materials: get().materials, ...(lods ? { lods } : {}), ...(targetTriangles ? { target_triangles: targetTriangles } : {}), ...(open ? { holes: open } : {}) });
    },
    exportSky: async (path) => {
      const { sun } = get();
      await run("sky.bake", { sun_azimuth_deg: sun.azimuth_deg, sun_elevation_deg: sun.elevation_deg, turbidity: sun.turbidity, cloud_cover: sun.clouds, fog: sun.fog, path });
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
      await flushLook();
      const r = await run("project.save");
      if (r.status === "ok" && r.result) set({ project: r.result });
    },
  };
});
