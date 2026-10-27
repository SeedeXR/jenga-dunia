// UI-driven test: the real App component (the Editor: graph + layer stack, ADR-0017) with the
// Tauri IPC mocked at the `invoke` boundary.
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const calls: { cmd: string; args: unknown }[] = [];
let sculptAdded = false;
let noiseNodeParams: Record<string, unknown> = {};

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args: { request?: { op: string; params: Record<string, unknown> } }) => {
    calls.push({ cmd, args });
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
        if (String(req.params.path).startsWith("/bad")) {
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
  function GraphPanelMock({ inspector = true }: { inspector?: boolean }) {
    const s = useStore();
    return (
      <div data-testid="graph-panel" data-inspector={String(inspector)}>
        {s.graph.nodes.map((n) => n.kind).join(",")}
        <button data-testid="graph-evaluate" onClick={() => void s.evaluateGraph()}>
          eval
        </button>
      </div>
    );
  }
  return { default: GraphPanelMock };
});

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
    useStore.setState({ project: null, layer: null, log: [], busy: false, graph: { nodes: [], edges: [] }, graphKinds: [], lastEval: null, live: true, selectedNode: null, jobs: [] });
  });
  const opsCalled = () => calls.map((c) => (c.args as { request?: { op: string } }).request?.op).filter(Boolean);
  const menu = (payload: string) => menuListeners.forEach((cb) => cb({ payload }));
  // File ▸ New with the native save dialog answering `path` (Studio's Project section has the fields).
  const create = async (path: string) => {
    render(<App />);
    await waitFor(() => expect(menuListeners.length).toBeGreaterThan(0));
    dialog.save = path;
    menu("new");
  };

  it("without a project it points to Studio; File ▸ New opens one with the graph and the layer stack", async () => {
    render(<App />);
    expect(screen.getByTestId("no-project")).toHaveTextContent("Studio");
    await waitFor(() => expect(menuListeners.length).toBeGreaterThan(0));
    dialog.save = "/tmp/Hills";
    menu("new");
    await waitFor(() => expect(screen.getByTestId("project-title")).toHaveTextContent("World — 2048² · 4096 m"));
    const made = calls.find((c) => (c.args as { request?: { op: string } }).request?.op === "project.create");
    expect(made).toMatchObject({ args: { request: { params: { path: "/tmp/Hills.jdproj" } } } }); // a project is a directory
    await waitFor(() => expect(screen.getByTestId("graph-panel")).toHaveTextContent("jd.noise.fbm@1"));
    expect(screen.getByTestId("panel-layers")).toBeInTheDocument();
  });

  it("native menu events run the operations: generate, export, open, evaluate, undo", async () => {
    await create("/tmp/Menu.jdproj");
    await waitFor(() => expect(screen.getByTestId("project-title")).toHaveTextContent("World"));
    menu("generate");
    await waitFor(() => expect(opsCalled()).toContain("graph.evaluate"));
    await waitFor(() => expect(useStore.getState().layer).toBe("height"));
    // File → Export… asks the native save dialog, then exports to the chosen path.
    dialog.save = "/tmp/out/hills.png";
    menu("export");
    await waitFor(() => expect(screen.getByTestId("log")).toHaveTextContent("W_EXPORT_LOSS"));
    const exp = calls.find((c) => (c.args as { request?: { op: string } }).request?.op === "terrain.export_heightmap");
    expect(exp).toMatchObject({ args: { request: { params: { path: "/tmp/out/hills.png", format: "png16" } } } });
    // File → Open… picks a directory and opens it; a cancelled dialog does nothing.
    dialog.open = "/tmp/Other.jdproj";
    menu("open");
    await waitFor(() => expect(calls.some((c) => (c.args as { request?: { op: string; params: { path?: string } } }).request?.params?.path === "/tmp/Other.jdproj")).toBe(true));
    const opens = () => opsCalled().filter((o) => o === "project.open").length;
    const before = opens();
    dialog.open = null;
    menu("open");
    await new Promise((r) => setTimeout(r, 40));
    expect(opens()).toBe(before);
    menu("evaluate");
    await waitFor(() => expect(screen.getByTestId("log")).toHaveTextContent("graph.evaluate"));
    menu("undo-graph");
    await waitFor(() => expect(screen.getByTestId("log")).toHaveTextContent("graph.undo"));
  });

  it("the noise settings mirror the graph's noise node, so Generate never reverts an edit made in the graph", async () => {
    noiseNodeParams = { seed: 5, feature_size_m: 900, height_range_m: [0, 160], octaves: { keys: [[0, 3], [1, 6]] } };
    await create("/tmp/N.jdproj");
    await waitFor(() => expect(screen.getByTestId("graph-panel")).toHaveTextContent("jd.noise.fbm@1"));
    expect(useStore.getState().noise).toMatchObject({ seed: 5, feature_size_m: 900, height_range_m: [0, 160], octaves: 6 });
    menu("generate");
    await waitFor(() => expect(opsCalled()).toContain("graph.evaluate"));
    // Generate wrote only what differs from the node — nothing for the mirrored keys.
    const written = calls.map((c) => (c.args as { request?: { op: string; params: { key?: string } } }).request).filter((r) => r?.op === "graph.set_param").map((r) => r?.params.key);
    expect(written).not.toContain("seed");
    expect(written).not.toContain("height_range_m");
    expect(written).not.toContain("feature_size_m");
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
    await waitFor(() => expect(opsCalled()).toContain("ui.select"));
    // A graph edit by Studio reloads the graph; the Editor's own is already applied. (A project
    // is open, so live evaluation is armed: its first evaluation runs before the edit.)
    dialog.save = "/tmp/Sync.jdproj";
    menu("new");
    await waitFor(() => expect(opsCalled()).toContain("graph.evaluate"));
    calls.length = 0;
    emit("project_changed", { op: "graph.splice", client: "gui-studio" });
    await waitFor(() => expect(opsCalled()).toContain("graph.list"));
    // … without evaluating it again: Studio evaluates its own edits (its report arrives below).
    await new Promise((r) => setTimeout(r, 250));
    expect(opsCalled()).not.toContain("graph.evaluate");
    calls.length = 0;
    emit("project_changed", { op: "graph.splice", client: "gui-editor" });
    await new Promise((r) => setTimeout(r, 20));
    expect(opsCalled()).not.toContain("graph.list");
    // A project Studio opened opens here, without a second evaluation.
    calls.length = 0;
    emit("project_changed", { op: "project.open", client: "gui-studio" });
    await waitFor(() => expect(opsCalled()).toContain("project.info"));
    await new Promise((r) => setTimeout(r, 250));
    expect(opsCalled()).not.toContain("graph.evaluate");
    // An evaluation Studio ran is adopted here: its layers become the Editor's.
    emit("evaluated", { report: { lod: 1, duration_ms: 5, nodes: [], layers: { height: {} } }, client: "gui-studio" });
    await waitFor(() => expect(useStore.getState().layer).toBe("height"));
  });

  it("surfaces core errors with code and next step in the status bar and the log", async () => {
    await create("/bad.jdproj");
    await waitFor(() => expect(screen.getByTestId("status-line")).toHaveTextContent("E_INPUT_INVALID"));
    expect(screen.getByTestId("log")).toHaveTextContent("Choose an empty or new directory.");
    // The log opens from the status bar.
    expect(screen.getByTestId("log")).toHaveClass("hidden");
    fireEvent.click(screen.getByTestId("toggle-log"));
    expect(screen.getByTestId("log")).not.toHaveClass("hidden");
  });

  it("the status bar's dock toggles hide and bring back the layer stack, the inspector and the log", async () => {
    localStorage.removeItem("jd-docks");
    await create("/tmp/Docks");
    await waitFor(() => expect(screen.getByTestId("graph-panel")).toHaveTextContent("jd.noise.fbm@1"));
    fireEvent.click(screen.getByTestId("dock-stack"));
    expect(screen.queryByTestId("panel-layers")).toBeNull();
    expect(screen.getByTestId("dock-stack")).toHaveAttribute("aria-label", "Layer stack: show");
    fireEvent.click(screen.getByTestId("dock-inspector"));
    expect(screen.getByTestId("graph-panel")).toHaveAttribute("data-inspector", "false");
    expect(JSON.parse(localStorage.getItem("jd-docks") ?? "{}")).toEqual({ stack: false, inspector: false }); // remembered
    fireEvent.click(screen.getByTestId("dock-stack"));
    fireEvent.click(screen.getByTestId("dock-inspector"));
    expect(screen.getByTestId("panel-layers")).toBeInTheDocument();
    expect(screen.getByTestId("graph-panel")).toHaveAttribute("data-inspector", "true");
    fireEvent.click(screen.getByTestId("dock-log"));
    expect(screen.getByTestId("log")).not.toHaveClass("hidden");
    localStorage.removeItem("jd-docks");
  });
});
