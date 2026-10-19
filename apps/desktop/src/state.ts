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

interface State {
  project: ProjectSummary | null;
  layer: LayerInfo | null;
  preview: Preview | null;
  previewStats: { bytes: number; ms: number; lod: number } | null;
  noise: NoiseSettings;
  busy: boolean;
  log: LogEntry[];
  setNoise: (patch: Partial<NoiseSettings>) => void;
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
  return {
    project: null,
    layer: null,
    preview: null,
    previewStats: null,
    noise: defaultNoise,
    busy: false,
    log: [],
    setNoise: (patch) => set((s) => ({ noise: { ...s.noise, ...patch } })),
    probeProject: async (isCancelled) => {
      try {
        const r = await op<ProjectSummary>("project.info");
        if (isCancelled?.()) return;
        if (r.status === "ok" && r.result) {
          set({ project: r.result });
          log(r);
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
      if (r.status === "ok" && r.result) set({ project: r.result, layer: null, preview: null });
    },
    openProject: async (path) => {
      const r = await run<ProjectSummary>("project.open", { path });
      if (r.status === "ok" && r.result) set({ project: r.result, layer: null, preview: null });
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
