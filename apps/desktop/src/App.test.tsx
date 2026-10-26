// UI-driven test: the real App component with the Tauri IPC mocked at the `invoke` boundary.
// The viewport is replaced (jsdom has no WebGPU/WebGL); everything else is the shipped UI.
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const calls: { cmd: string; args: unknown }[] = [];
let sculptAdded = false;
let noiseNodeParams: Record<string, unknown> = {};

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args: { request?: { op: string; params: Record<string, unknown> } }) => {
    calls.push({ cmd, args });
    if (cmd === "layer_preview") {
      const buf = new ArrayBuffer(16 + 4 * 2);
      const v = new DataView(buf);
      v.setUint32(0, 2, true);
      v.setUint32(4, 2, true);
      v.setFloat32(8, 0, true);
      v.setFloat32(12, 100, true);
      return buf;
    }
    const req = args.request;
    if (!req) throw new Error("unexpected command");
    const base = { op: req.op, schema_version: 1, request_id: "01TEST" };
    switch (req.op) {
      case "graph.kinds":
        return { ...base, status: "ok", result: [{ kind: "jd.noise.fbm@1", description: "", inputs: [], outputs: [{ name: "out", ty: "heightfield", required: false }], param_schema: {} }] };
      case "graph.list": {
        const nodes = [
          { id: "01N", kind: "jd.noise.fbm@1", params: noiseNodeParams, bypass: false, ui: { position: [0, 0], label: "Noise" } },
          { id: "01O", kind: "jd.output.height@1", params: { layer: "height" }, bypass: false, ui: { position: [320, 0], label: "Height" } },
        ];
        if (sculptAdded) nodes.push({ id: "01SCULPT00000000000000000A", kind: "jd.sculpt.strokes@1", params: {}, bypass: false, ui: { position: [320, 140], label: "Strokes" } });
        return { ...base, status: "ok", result: { nodes, edges: [{ from: { node: "01N", port: "out" }, to: { node: "01O", port: "in" } }], undo_depth: 0, redo_depth: 0 } };
      }
      case "graph.evaluate":
        return { ...base, status: "ok", result: { lod: 0, duration_ms: 3, nodes: [], layers: { height: "abc", water: "def" } } };
      case "terrain.stats": {
        const layer = String(req.params.layer ?? "height");
        return { ...base, status: "ok", result: { layer, size_px: [2048, 2048], stats: { min: layer === "water" ? 0 : 1.5, max: 498.2, mean: 250.1, non_finite: 0 }, content_hash: layer === "water" ? "def" : "abc", duration_ms: 0 } };
      }
      case "project.info":
        return { ...base, status: "error", error: { code: "E_INPUT_INVALID", message: "no project is open" } };
      case "project.create":
        if (req.params.path === "/bad") {
          return { ...base, status: "error", error: { code: "E_INPUT_INVALID", message: "target directory exists and is not empty", next: "Choose an empty or new directory." } };
        }
        return {
          ...base,
          status: "ok",
          result: { project_id: "01PROJ", name: req.params.name, path: req.params.path, world: { extent_m: [4096, 4096], resolution_px: [2048, 2048], tile_size: 512, halo: 4, seed: 42 }, schema_version: 1, dirty: false, pending_journal: 0, settings: { autosave_interval_s: 120, cache_limit_bytes: 2147483648, preview_lod: 2, memory_budget_bytes: 4294967296 } },
        };
      case "jobs.list":
        return { ...base, status: "ok", result: [] };
      case "graph.add_node":
      case "graph.splice": // the core wires the spliced node itself (ADR-0017 workflow ops)
      case "graph.sculpt_target": // …and finds or splices the brush's strokes node
        sculptAdded = true;
        return { ...base, status: "ok", result: { id: "01SCULPT00000000000000000A", published: [] } };
      case "graph.connect":
      case "graph.set_param":
      case "graph.append_param":
        return { ...base, status: "ok", result: { nodes: [{ id: "01SCULPT00000000000000000A", kind: "jd.sculpt.strokes@1", params: {}, bypass: false }], edges: [], undo_depth: 1, redo_depth: 0 } };
      case "terrain.generate":
        return { ...base, status: "ok", result: { layer: "height", size_px: [2048, 2048], stats: { min: 1.5, max: 498.2, mean: 250.1, non_finite: 0 }, content_hash: "abcdef0123456789abcdef", duration_ms: 42 } };
      case "terrain.export_heightmap":
        return { ...base, status: "ok", result: { path: "x.png" }, warnings: [{ code: "W_EXPORT_LOSS", message: "Png16 quantises heights" }] };
      default:
        return { ...base, status: "error", error: { code: "E_INPUT_INVALID", message: `unknown op ${req.op}` } };
    }
  }),
}));

// Native dialogs: resolved paths are scripted per test.
const dialog = { open: null as string | null, save: null as string | null };
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => dialog.open),
  save: vi.fn(async () => dialog.save),
}));

const menuListeners: ((e: { payload: string }) => void)[] = [];
const engineListeners: ((e: { payload: { topic: string; data: Record<string, unknown> } }) => void)[] = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    if (name === "menu") menuListeners.push(cb as (typeof menuListeners)[number]); // project-opened is not exercised here
    if (name === "engine-event") engineListeners.push(cb as (typeof engineListeners)[number]);
    return () => undefined;
  }),
}));

// React Flow needs layout APIs jsdom lacks; the panel is covered by the Playwright e2e.
vi.mock("./graph/GraphPanel", () => {
  function GraphPanelMock() {
    const s = useStore();
    return (
      <div data-testid="graph-panel">
        {s.graph.nodes.map((n) => n.kind).join(",")}
        <button data-testid="graph-evaluate" onClick={() => void s.evaluateGraph()}>
          eval
        </button>
      </div>
    );
  }
  return { default: GraphPanelMock };
});

vi.mock("./viewport/TerrainView", () => ({
  default: ({ preview }: { preview: { width: number } | null }) => <div data-testid="viewport">{preview ? `preview ${preview.width}` : "no preview"}</div>,
}));

import App from "./App";
import { useStore } from "./state";

describe("App", () => {
  beforeEach(() => {
    calls.length = 0;
    sculptAdded = false;
    noiseNodeParams = {};
    dialog.open = null;
    dialog.save = null;
    // The store is module-global; reset the mirrored state between tests.
    useStore.setState({ project: null, layer: null, preview: null, previewStats: null, log: [], busy: false, graph: { nodes: [], edges: [] }, graphKinds: [], lastEval: null, live: true, selectedNode: null, sculpting: false, jobs: [] });
  });

  it("creates a project, generates terrain, shows stats and preview, exports with a loss warning", async () => {
    render(<App />);
    expect(screen.getByTestId("app-version")).toHaveTextContent(/^v\d+\.\d+\.\d+$/);
    expect(screen.getByTestId("generate")).toBeDisabled();
    fireEvent.change(screen.getByTestId("project-path"), { target: { value: "/tmp/Hills.jdproj" } });
    fireEvent.click(screen.getByTestId("new-project"));
    await waitFor(() => expect(screen.getByText(/Hills\.jdproj|World — 2048²/)).toBeInTheDocument());
    const create = calls.find((c) => (c.args as { request?: { op: string } }).request?.op === "project.create");
    expect(create).toMatchObject({ cmd: "op", args: { request: { op: "project.create", schema_version: 1, params: { path: "/tmp/Hills.jdproj", name: "World" } } } });

    fireEvent.change(screen.getByTestId("seed"), { target: { value: "7" } });
    fireEvent.click(screen.getByTestId("generate"));
    await waitFor(() => expect(screen.getByTestId("layer-stats")).toBeInTheDocument());
    // Generate edits the graph's noise node (every differing setting, journaled) and evaluates the graph.
    const seedEdit = calls.find((c) => {
      const r = (c.args as { request?: { op: string; params: { key?: string } } }).request;
      return r?.op === "graph.set_param" && r.params.key === "seed";
    });
    expect(seedEdit).toMatchObject({ args: { request: { params: { id: "01N", key: "seed", value: 7 } } } });
    expect(calls.some((c) => (c.args as { request?: { op: string; params: { lod?: number } } }).request?.op === "graph.evaluate")).toBe(true);
    expect(calls.some((c) => (c.args as { request?: { op: string } }).request?.op === "terrain.generate")).toBe(false);
    expect(screen.getByTestId("layer-stats")).toHaveTextContent("1.5 / 498.2 m");
    expect(screen.getByTestId("viewport")).toHaveTextContent("preview 2");
    expect(calls.some((c) => c.cmd === "layer_preview")).toBe(true);

    fireEvent.change(screen.getByTestId("export-path"), { target: { value: "h.png" } });
    fireEvent.click(screen.getByTestId("export"));
    await waitFor(() => expect(screen.getByTestId("log")).toHaveTextContent("W_EXPORT_LOSS"));
    const exp = calls.find((c) => (c.args as { request?: { op: string } }).request?.op === "terrain.export_heightmap");
    expect(exp).toMatchObject({ args: { request: { params: { path: "h.png", format: "png16" } } } });
  });

  it("native menu events trigger the same operations as the buttons", async () => {
    render(<App />);
    fireEvent.change(screen.getByTestId("project-path"), { target: { value: "/tmp/Menu.jdproj" } });
    fireEvent.click(screen.getByTestId("new-project"));
    await waitFor(() => expect(screen.getByText(/World — 2048²/)).toBeInTheDocument());
    expect(menuListeners.length).toBeGreaterThan(0);
    menuListeners.forEach((cb) => cb({ payload: "generate" }));
    await waitFor(() => expect(screen.getByTestId("layer-stats")).toBeInTheDocument());
    // File → Export… asks the native save dialog, then exports to the chosen path.
    dialog.save = "/tmp/out/hills.png";
    menuListeners.forEach((cb) => cb({ payload: "export" }));
    await waitFor(() => expect(screen.getByTestId("log")).toHaveTextContent("terrain.export_heightmap"));
    const exp = calls.find((c) => (c.args as { request?: { op: string } }).request?.op === "terrain.export_heightmap");
    expect(exp).toMatchObject({ args: { request: { params: { path: "/tmp/out/hills.png", format: "png16" } } } });
    // File → Open… picks a directory and opens it; a cancelled dialog does nothing.
    dialog.open = "/tmp/Other.jdproj";
    menuListeners.forEach((cb) => cb({ payload: "open" }));
    await waitFor(() => expect(calls.some((c) => (c.args as { request?: { op: string; params: { path?: string } } }).request?.params?.path === "/tmp/Other.jdproj")).toBe(true));
    // Let the open sequence (kinds, list, stats, preview, textures…) finish before counting, or a
    // slow runner's trailing calls land inside the window and look like the cancelled dialog's.
    const settle = async () => {
      let n = -1;
      while (n !== calls.length) {
        n = calls.length;
        await new Promise((r) => setTimeout(r, 40));
      }
    };
    await settle();
    // What a cancelled dialog must not do is open a project: count project.open calls, not
    // every call — stats/preview/texture fetches of the previous open may still trickle in
    // on a slow runner (seen twice on CI).
    const opens = () => calls.filter((c) => (c.args as { request?: { op: string } }).request?.op === "project.open").length;
    const before = opens();
    dialog.open = null;
    menuListeners.forEach((cb) => cb({ payload: "open" }));
    await new Promise((r) => setTimeout(r, 40));
    expect(opens()).toBe(before);
    menuListeners.forEach((cb) => cb({ payload: "evaluate" }));
    await waitFor(() => expect(screen.getByTestId("log")).toHaveTextContent("graph.evaluate"));
    menuListeners.forEach((cb) => cb({ payload: "undo-graph" }));
    await waitFor(() => expect(screen.getByTestId("log")).toHaveTextContent("graph.undo"));
  });

  it("loads the project graph and evaluating it refreshes stats and preview", async () => {
    render(<App />);
    fireEvent.change(screen.getByTestId("project-path"), { target: { value: "/tmp/G.jdproj" } });
    fireEvent.click(screen.getByTestId("new-project"));
    await waitFor(() => expect(screen.getByTestId("graph-panel")).toHaveTextContent("jd.noise.fbm@1"));
    fireEvent.click(screen.getByTestId("graph-evaluate"));
    await waitFor(() => expect(screen.getByTestId("layer-stats")).toBeInTheDocument());
    const ops = calls.map((c) => (c.args as { request?: { op: string } }).request?.op).filter(Boolean);
    expect(ops).toEqual(expect.arrayContaining(["graph.kinds", "graph.list", "graph.evaluate", "terrain.stats"]));
    // Two published layers → a picker appears; choosing "water" fetches that layer's stats and preview.
    const pick = screen.getByTestId("layer-pick") as HTMLSelectElement;
    expect(Array.from(pick.options).map((o) => o.value)).toEqual(["height", "water"]);
    fireEvent.change(pick, { target: { value: "water" } });
    await waitFor(() => expect(screen.getByTestId("layer-stats")).toHaveTextContent("def"));
    const waterStats = calls.filter((c) => (c.args as { request?: { op: string; params: { layer?: string } } }).request?.op === "terrain.stats" && (c.args as { request: { params: { layer?: string } } }).request.params.layer === "water");
    expect(waterStats.length).toBeGreaterThan(0);
    expect(screen.getByTestId("viewport")).toHaveTextContent("preview 2");
  });

  it("the Noise panel mirrors the graph's noise node, so Generate never reverts an edit made in the graph", async () => {
    noiseNodeParams = { seed: 5, feature_size_m: 900, height_range_m: [0, 160], octaves: { keys: [[0, 3], [1, 6]] } };
    render(<App />);
    fireEvent.change(screen.getByTestId("project-path"), { target: { value: "/tmp/N.jdproj" } });
    fireEvent.click(screen.getByTestId("new-project"));
    await waitFor(() => expect(screen.getByTestId("graph-panel")).toHaveTextContent("jd.noise.fbm@1"));
    // The panel shows the node's values; a keyed (timeline) octaves stays at the panel default.
    expect(screen.getByTestId("seed")).toHaveValue(5);
    expect(screen.getByLabelText("Feature size (m)")).toHaveValue(900);
    expect(screen.getByLabelText("Octaves")).toHaveValue(6);
    fireEvent.click(screen.getByTestId("generate"));
    await waitFor(() => expect(screen.getByTestId("layer-stats")).toBeInTheDocument());
    // Generate wrote only what the panel changed relative to the node — nothing for the mirrored keys.
    const written = calls.map((c) => (c.args as { request?: { op: string; params: { key?: string } } }).request).filter((r) => r?.op === "graph.set_param").map((r) => r?.params.key);
    expect(written).not.toContain("seed");
    expect(written).not.toContain("height_range_m");
    expect(written).not.toContain("feature_size_m");
  });

  it("Sculpt wires a stroke node in front of the output and enters sculpt mode", async () => {
    render(<App />);
    fireEvent.change(screen.getByTestId("project-path"), { target: { value: "/tmp/S.jdproj" } });
    fireEvent.click(screen.getByTestId("new-project"));
    await waitFor(() => expect(screen.getByTestId("graph-panel")).toHaveTextContent("jd.noise.fbm@1"));
    fireEvent.click(screen.getByTestId("sculpt-toggle"));
    await waitFor(() => expect(screen.getByTestId("sculpt-toggle")).toHaveTextContent("Sculpting"));
    // One core call finds or puts the strokes node between the noise and the height output.
    const target = calls.find((c) => (c.args as { request?: { op: string } }).request?.op === "graph.sculpt_target");
    expect(target).toMatchObject({ args: { request: { params: { layer: "height", kind: "strokes" } } } });
    expect(screen.getByTestId("brush-strength")).toBeInTheDocument();
    // Mode select swaps strength for amount, and strokes carry mode + amount to the core.
    fireEvent.change(screen.getByTestId("brush-mode"), { target: { value: "flatten" } });
    expect(screen.getByTestId("brush-amount")).toBeInTheDocument();
    expect(screen.queryByTestId("brush-strength")).toBeNull();
    await useStore.getState().addStroke([
      [10, 20],
      [30, 40],
    ]);
    const stroke = calls.find((c) => (c.args as { request?: { op: string; params: { key?: string } } }).request?.op === "graph.append_param" && (c.args as { request: { params: { key?: string } } }).request.params.key === "strokes");
    expect(stroke).toMatchObject({ args: { request: { params: { value: { points: [[10, 20], [30, 40]], radius_m: 150, mode: "flatten", amount: 0.5 } } } } });
  });

  it("connected to Studio: its graph edits reload the graph, selections cross both ways", async () => {
    render(<App />);
    await waitFor(() => expect(engineListeners.length).toBeGreaterThan(0));
    const emit = (topic: string, data: Record<string, unknown>) => engineListeners.forEach((cb) => cb({ payload: { topic, data } }));
    // Studio selected a node (a layer row): the Editor follows, and does not echo it back.
    calls.length = 0;
    emit("selection", { nodes: ["01SCULPT00000000000000000A"], source: "gui-studio" });
    expect(useStore.getState().selectedNode).toBe("01SCULPT00000000000000000A");
    // The Editor's own selection goes out as ui.select.
    useStore.getState().setSelectedNode("01NOISE");
    await waitFor(() => expect(calls.some((c) => (c.args as { request?: { op: string; params: { nodes: string[] } } }).request?.op === "ui.select")).toBe(true));
    // A graph edit by Studio reloads the graph; the Editor's own is already applied.
    calls.length = 0;
    emit("project_changed", { op: "graph.splice", client: "gui-studio" });
    await waitFor(() => expect(calls.some((c) => (c.args as { request?: { op: string } }).request?.op === "graph.list")).toBe(true));
    calls.length = 0;
    emit("project_changed", { op: "graph.splice", client: "gui-editor" });
    await new Promise((r) => setTimeout(r, 20));
    expect(calls.some((c) => (c.args as { request?: { op: string } }).request?.op === "graph.list")).toBe(false);
  });

  it("a world preset fills resolution and extent (Unreal 2049²)", async () => {
    render(<App />);
    fireEvent.change(screen.getByTestId("world-preset"), { target: { value: "Unity / Unreal 2049² · 8 km" } });
    expect(screen.getByLabelText("Resolution (px)")).toHaveValue(2049);
    expect(screen.getByLabelText("Extent (m)")).toHaveValue(8192);
    fireEvent.change(screen.getByTestId("project-path"), { target: { value: "/tmp/U.jdproj" } });
    fireEvent.click(screen.getByTestId("new-project"));
    await waitFor(() => expect(calls.some((c) => (c.args as { request?: { op: string } }).request?.op === "project.create")).toBe(true));
    const create = calls.find((c) => (c.args as { request?: { op: string } }).request?.op === "project.create");
    expect(create).toMatchObject({ args: { request: { params: { world: { resolution_px: [2049, 2049], extent_m: [8192, 8192] } } } } });
  });

  it("surfaces core errors with code and next step in the log", async () => {
    render(<App />);
    fireEvent.change(screen.getByTestId("project-path"), { target: { value: "/bad" } });
    fireEvent.click(screen.getByTestId("new-project"));
    await waitFor(() => expect(screen.getByTestId("log")).toHaveTextContent("E_INPUT_INVALID"));
    expect(screen.getByTestId("log")).toHaveTextContent("Choose an empty or new directory.");
    expect(screen.getByTestId("generate")).toBeDisabled();
  });
});
