// Node graph editor (React Flow). The canvas mirrors `graph.json`; every edit is an op round-trip
// (graph.connect / graph.add_node / graph.set_param / graph.remove_node / graph.move_node) so GUI, CLI and MCP agree.
import {
  Background,
  type Connection,
  Controls,
  type Edge as FlowEdge,
  Handle,
  type Node as FlowNode,
  type NodeProps,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useEdgesState,
  useNodesInitialized,
  useNodesState,
  useReactFlow,
} from "@xyflow/react";
import { Clock, Play, Plus, Redo2, Trash2, Undo2, Zap, ZapOff } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { nodePreview, type Preview } from "../lib/api";
import { num } from "../lib/expr";
import { type GraphNode, useStore } from "../state";
import { Range } from "../components/Range";

type JdNodeData = { kind: string; label: string; params: Record<string, unknown>; inputs: string[]; outputs: string[]; selected?: boolean };

/** The keyframes of a parameter written as `{ keys: [[t, v], …] }`, or null for a plain value. */
function keysOf(v: unknown): [number, unknown][] | null {
  if (!v || typeof v !== "object" || Array.isArray(v)) return null;
  const o = v as { keys?: unknown };
  return Array.isArray(o.keys) && Object.keys(o).length === 1 ? (o.keys as [number, unknown][]) : null;
}

/** Keyframes as the inspector shows and accepts them: `t:v, t:v` (`0:1, 1:300`). */
function keysText(keys: [number, unknown][]): string {
  return keys.map(([t, v]) => `${t}:${typeof v === "number" ? String(Number(v.toPrecision(7))) : String(v)}`).join(", ");
}
function parseKeys(raw: string, isNum: boolean): { keys: [number, unknown][] } | null {
  if (!raw.includes(":")) return null;
  const keys = raw.split(",").map((part) => {
    const [t, v] = part.split(":").map((x) => x.trim());
    return [Number(t), isNum ? num(String(v)) : v] as [number, unknown];
  });
  return keys.every(([t, v]) => Number.isFinite(t) && (!isNum || Number.isFinite(v as number))) ? { keys } : null;
}

/** A kind as the layer stack names it: `jd.filter.blur@1` → `filter.blur`. */
const shortKind = (kind: string) => kind.split(".").slice(1).join(".").replace(/@\d+$/, "");

function JdNode({ data, selected }: NodeProps<FlowNode<JdNodeData>>) {
  const summary = Object.entries(data.params)
    .slice(0, 3)
    .map(([k, v]) => `${k}=${Array.isArray(v) ? (v.some((x) => typeof x === "object" && x !== null) ? `${v.length} recorded` : v.join("…")) : keysOf(v) ? `⏱${keysOf(v)!.length} keys` : String(v)}`)
    .join(" · ");
  return (
    <div className={`min-w-40 rounded-lg border ${selected ? "border-jd-accent" : "border-white/5"} bg-jd-bg-2 text-xs text-jd-fg-0 shadow-lg shadow-black/40`}>
      <div className="rounded-tl-[3px] rounded-tr-lg border-b border-white/5 border-l-2 border-l-jd-primary bg-jd-bg-3/60 px-2 py-1 font-semibold">{data.label}</div>
      <div className="px-2 py-1 text-[11px] text-jd-fg-2">{data.kind}</div>
      {summary && <div className="truncate px-2 pb-1 text-[11px] text-jd-fg-1">{summary}</div>}
      {/* One row per port pair, its handles on the row's edges; a side with several ports names
          them (rivers: out, rivers, …), a lone `in` / `out` stays a bare handle. */}
      <div className="pb-1">
        {Array.from({ length: Math.max(data.inputs.length, data.outputs.length) }, (_, i) => {
          const [inp, out] = [data.inputs[i], data.outputs[i]];
          return (
            <div key={i} className="relative flex h-4 items-center justify-between px-2 text-[10px] text-jd-fg-2">
              <span>{inp && data.inputs.length > 1 ? inp : ""}</span>
              <span>{out && data.outputs.length > 1 ? out : ""}</span>
              {inp && <Handle id={inp} type="target" position={Position.Left} title={inp} />}
              {out && <Handle id={out} type="source" position={Position.Right} title={out} />}
            </div>
          );
        })}
      </div>
    </div>
  );
}

const nodeTypes = { jd: JdNode };

/** A column of a structured (array-of-objects) parameter: scalar, or a 2-number range, possibly nullable. */
type ItemProp = { kind: "string" | "number" | "boolean" | "range2"; nullable: boolean; description?: string };
type Prop = { type?: string; enum?: string[]; description?: string; default?: unknown; structured?: boolean; nested?: boolean; items?: Record<string, ItemProp> };
type RawProp = { type?: string | string[]; description?: string; items?: { type?: string }; minItems?: number; maxItems?: number };
type Schema = {
  properties?: Record<string, Prop & { $ref?: string; items?: { type?: string; $ref?: string } }>;
  $defs?: Record<string, { enum?: string[]; oneOf?: { const?: unknown }[]; properties?: Record<string, RawProp> }>;
};

/** Item columns when every field of the item object is a scalar or a 2-number range (e.g. biome rules). */
function itemColumns(def: { properties?: Record<string, RawProp> } | undefined): Record<string, ItemProp> | undefined {
  if (!def?.properties) return undefined;
  const out: Record<string, ItemProp> = {};
  for (const [k, p] of Object.entries(def.properties)) {
    const types = Array.isArray(p.type) ? p.type : p.type ? [p.type] : [];
    const nullable = types.includes("null");
    const main = types.filter((t) => t !== "null");
    if (main.length !== 1) return undefined;
    const t = main[0];
    if (t === "string" || t === "boolean") out[k] = { kind: t, nullable, description: p.description };
    else if (t === "number" || t === "integer") out[k] = { kind: "number", nullable, description: p.description };
    else if (t === "array" && p.minItems === 2 && p.maxItems === 2 && (p.items?.type === "number" || p.items?.type === "integer")) out[k] = { kind: "range2", nullable, description: p.description };
    else return undefined;
  }
  return out;
}

/** Flatten schemars output for the inspector: follow `$ref`s into `$defs` and turn `oneOf` consts into an enum. */
function resolveProps(schema: unknown): Record<string, Prop> {
  const sc = (schema ?? {}) as Schema;
  const out: Record<string, Prop> = {};
  for (const [key, prop] of Object.entries(sc.properties ?? {})) {
    const def = prop.$ref ? sc.$defs?.[prop.$ref.split("/").pop() ?? ""] : undefined;
    const en = prop.enum ?? def?.enum ?? def?.oneOf?.map((o) => String(o.const));
    // Arrays of objects: editable as a table when the item is flat (biome rules); otherwise shown
    // as a count (recorded strokes are data, not something to type into a field).
    const structured = prop.type === "array" && (prop.items?.$ref !== undefined || prop.items?.type === "object");
    const items = structured && prop.items?.$ref ? itemColumns(sc.$defs?.[prop.items.$ref.split("/").pop() ?? ""]) : undefined;
    // Arrays of arrays (a spline's points) are geometry, drawn with their own tools: shown, not typed.
    const nested = prop.type === "array" && prop.items?.type === "array";
    out[key] = { type: prop.type, description: prop.description, default: prop.default, structured, nested, ...(items ? { items } : {}), ...(en ? { enum: en } : {}) };
  }
  return out;
}

/** Editor for an array of flat objects: one block of fields per element, blur applies, elements can be added and removed. */
function StructuredEditor({ columns, rows, testId, onChange }: { columns: Record<string, ItemProp>; rows: Record<string, unknown>[]; testId: string; onChange: (rows: Record<string, unknown>[]) => void }) {
  const fmt = (v: unknown, c: ItemProp): string => (v === null || v === undefined ? "" : c.kind === "range2" && Array.isArray(v) ? v.map((x) => String(Number(Number(x).toPrecision(7)))).join(",") : String(v));
  const parse = (raw: string, c: ItemProp): unknown => {
    const t = raw.trim();
    if (t === "") return c.nullable ? null : c.kind === "string" ? "" : undefined;
    if (c.kind === "number") return num(t);
    if (c.kind === "boolean") return t === "true";
    if (c.kind === "range2") return t.split(",").map(num);
    return t;
  };
  const set = (i: number, k: string, raw: string, c: ItemProp) => {
    const next = rows.map((r, j) => (j === i ? { ...r, [k]: parse(raw, c) } : r));
    const v = next[i]?.[k];
    if (v === undefined) delete next[i]?.[k];
    onChange(next);
  };
  const blank = (): Record<string, unknown> => Object.fromEntries(Object.entries(columns).map(([k, c]) => [k, c.nullable ? null : c.kind === "string" ? "" : c.kind === "boolean" ? false : c.kind === "range2" ? [0, 1] : 0]));
  return (
    <div className="w-full" data-testid={testId}>
      {rows.length === 0 && <div className="text-jd-fg-2">none (defaults apply)</div>}
      {/* One block per record, its fields stacked as the inspector's own (a table's five columns
          overflowed the inspector's width). */}
      {rows.map((row, i) => (
        <div key={i} className="mb-1 rounded border border-jd-line px-1.5 py-1" data-testid={`${testId}-row-${i}`}>
          <div className="flex items-center justify-between text-jd-fg-2">
            <span>
              {i + 1}
              {typeof row.name === "string" && row.name ? ` · ${row.name}` : ""}
            </span>
            <button className="text-jd-error" title="Remove row" onClick={() => onChange(rows.filter((_, j) => j !== i))} data-testid={`${testId}-remove-${i}`}>
              <Trash2 size={11} />
            </button>
          </div>
          {Object.entries(columns).map(([k, c]) => (
            <label key={k} className="flex items-center justify-between gap-2 py-0.5" title={c.description}>
              <span className="text-jd-fg-1">{k}</span>
              <input className="w-28 jd-field" defaultValue={fmt(row[k], c)} placeholder={c.kind === "range2" ? "min,max" : ""} onBlur={(e) => { if (e.target.value !== fmt(row[k], c)) set(i, k, e.target.value, c); }} data-testid={`${testId}-${i}-${k}`} />
            </label>
          ))}
        </div>
      ))}
      <button className="btn mt-1 px-1.5 py-0" onClick={() => onChange([...rows, blank()])} data-testid={`${testId}-add`}>
        <Plus size={11} /> row
      </button>
    </div>
  );
}

function toFlow(nodes: GraphNode[], kinds: Map<string, { inputs: string[]; outputs: string[] }>): FlowNode<JdNodeData>[] {
  return nodes.map((n) => ({
    id: n.id,
    type: "jd",
    position: { x: n.ui?.position?.[0] ?? 0, y: n.ui?.position?.[1] ?? 0 },
    data: {
      kind: n.kind,
      label: n.ui?.label ?? n.kind.split("@")[0]?.split(".").slice(-1)[0] ?? n.kind,
      params: n.params ?? {},
      inputs: kinds.get(n.kind)?.inputs ?? [],
      outputs: kinds.get(n.kind)?.outputs ?? ["out"],
    },
  }));
}

/** Per-node presets (World Creator's per-filter presets): a named set of parameters per kind,
 *  applied as ordinary journaled set_param calls, so undo removes them one by one. */
export const PRESETS: Record<string, { name: string; params: Record<string, unknown> }[]> = {
  "jd.erosion.thermal@1": [
    { name: "dry sand", params: { talus_deg: 32, iterations: 60 } },
    { name: "scree", params: { talus_deg: 38, iterations: 100 } },
    { name: "soft slump", params: { talus_deg: 28, iterations: 200, rate: 0.2 } },
  ],
  "jd.erosion.hydraulic@1": [
    { name: "gentle rain", params: { iterations: 100, rain_rate: 0.006 } },
    { name: "monsoon", params: { iterations: 300, rain_rate: 0.03 } },
    { name: "hard rock", params: { iterations: 200, dissolve_k: 0.1 } },
  ],
  "jd.erosion.stream_power@1": [
    { name: "young uplands", params: { k: 0.0004, iterations: 20 } },
    { name: "old lowlands", params: { k: 0.002, iterations: 60 } },
  ],
  "jd.erosion.aeolian@1": [
    { name: "transverse dunes", params: { iterations: 200, wind2_azimuth_deg: null } },
    { name: "star dunes", params: { iterations: 300, wind2_azimuth_deg: 0, wind2_share: 0.5 } },
  ],
  "jd.climate.snowpack@1": [
    { name: "deep winter", params: { temperature_c: -8, steps: 60 } },
    { name: "spring thaw", params: { temperature_c: 6, steps: 60 } },
    { name: "alpine summer", params: { temperature_c: 14, steps: 40 } },
  ],
  "jd.climate.rain@1": [
    { name: "temperate westerlies", params: { temperature_c: 12, recovery_km: 40 } },
    { name: "arid shadow", params: { temperature_c: 28, base_per_km: 0.002, recovery_km: 120 } },
  ],
  "jd.erosion.debris@1": [
    { name: "cliff talus", params: { source_slope_deg: 45, reach_deg: 30, sorting: 0.5 } },
    { name: "boulder field", params: { source_slope_deg: 40, reach_deg: 27, size_min_m: 1, size_max_m: 6, sorting: 1 } },
  ],
  "jd.volcanic.lava@1": [
    { name: "short a'ā tongue", params: { eruption_s: 3600, duration_s: 7200, cooling: 0.6 } },
    { name: "long pāhoehoe sheet", params: { eruption_s: 43200, duration_s: 86400, viscosity_pa_s: 1000, cooling: 0.2 } },
  ],
  "jd.filter.terrace@1": [
    { name: "rice paddies", params: { step_m: 5, softness: 0.1 } },
    { name: "mesa caps", params: { step_m: 40, softness: 0.05 } },
  ],
  "jd.shape.primitive@1": [
    { name: "table plateau", params: { shape: "plateau", ratio: 0.85, falloff_m: 20, op: "max", opacity: 1 } },
    { name: "mesa (steep sides)", params: { shape: "plateau", ratio: 0.6, falloff_m: 5, op: "max", opacity: 1 } },
    { name: "volcano cone", params: { shape: "crater", ratio: 0.3, floor: -0.4, op: "max", opacity: 1 } },
  ],
  "jd.shape.polygon@1": [
    { name: "building pad (flatten)", params: { op: "lerp", falloff_m: 8, opacity: 1 } },
    { name: "raised field", params: { op: "add", falloff_m: 15, opacity: 1 } },
  ],
  "jd.erosion.coastal@1": [
    { name: "sandy beach", params: { band_m: 6, iterations: 120, rate_m: 0.5, drift: 0.3 } },
    { name: "cliff coast", params: { band_m: 25, iterations: 60, rate_m: 1.5, drift: 0.1 } },
  ],
  "jd.erosion.karst@1": [
    { name: "doline plain", params: { density_per_km2: 3, profile: "bowl" } },
    { name: "collapse field", params: { density_per_km2: 6, profile: "collapse", depth_m: [6, 25] } },
  ],
};

function Inner({ inspector }: { inspector: boolean }) {
  const s = useStore();
  const kindMap = useMemo(() => new Map(s.graphKinds.map((k) => [k.kind, { inputs: k.inputs.map((p) => p.name), outputs: k.outputs.map((p) => p.name) }])), [s.graphKinds]);
  const [nodes, setNodes, onNodesChange] = useNodesState<FlowNode<JdNodeData>>([]);
  // Hover thumbnail: a node's output at a coarse lod, fetched 250 ms after the pointer settles
  // (a cache hit after the first look) and drawn as a 96 px grey-scale square by the pointer.
  const [thumb, setThumb] = useState<{ id: string; x: number; y: number; preview: Preview | null } | null>(null);
  const thumbTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const thumbCanvas = useRef<HTMLCanvasElement | null>(null);
  const hoverNode = (id: string, x: number, y: number) => {
    clearTimeout(thumbTimer.current);
    thumbTimer.current = setTimeout(() => {
      setThumb({ id, x, y, preview: null });
      void nodePreview(id).then((preview) => setThumb((t) => (t && t.id === id ? { ...t, preview } : t)));
    }, 250);
  };
  const leaveNode = () => {
    clearTimeout(thumbTimer.current);
    setThumb(null);
  };
  useEffect(() => {
    const c = thumbCanvas.current;
    const p = thumb?.preview;
    if (!c || !p) return;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    const img = ctx.createImageData(p.width, p.height);
    const range = Math.max(1, p.max - p.min) / 65535;
    for (let y = 0; y < p.height; y++) {
      for (let x = 0; x < p.width; x++) {
        const v = (p.data[(p.height - 1 - y) * p.width + x] ?? 0) / 65535; // row 0 = south: flip for the screen
        const g = Math.round(40 + 215 * v);
        const o = (y * p.width + x) * 4;
        img.data[o] = g;
        img.data[o + 1] = g;
        img.data[o + 2] = g;
        img.data[o + 3] = 255;
      }
    }
    c.width = p.width;
    c.height = p.height;
    ctx.putImageData(img, 0, 0);
    void range;
  }, [thumb]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<FlowEdge>([]);
  const selected = s.selectedNode;
  const setSelected = (v: string | null | ((cur: string | null) => string | null)) => s.setSelectedNode(typeof v === "function" ? v(s.selectedNode) : v);
  const [newKind, setNewKind] = useState("jd.filter.blur@1");

  useEffect(() => {
    setNodes(toFlow(s.graph.nodes, kindMap));
    setEdges(
      s.graph.edges.map((e) => ({
        id: `${e.from.node}.${e.from.port}->${e.to.node}.${e.to.port}`,
        source: e.from.node,
        sourceHandle: e.from.port,
        target: e.to.node,
        targetHandle: e.to.port,
        animated: false,
      })),
    );
  }, [s.graph, kindMap, setNodes, setEdges]);

  const onConnect = useCallback(
    (c: Connection) => {
      if (c.source && c.target) void s.connectNodes(c.source, c.sourceHandle ?? "out", c.target, c.targetHandle ?? "in");
    },
    [s],
  );

  // Keep every node in view when the graph gains or loses nodes (new nodes land off-screen otherwise).
  const rf = useReactFlow();
  const measured = useNodesInitialized(); // false until every node has its size, so the fit is exact
  const nodeCount = s.graph.nodes.length;
  useEffect(() => {
    if (measured) void rf.fitView({ padding: 0.2, maxZoom: 1 });
  }, [measured, nodeCount, rf]);

  const sel = s.graph.nodes.find((n) => n.id === selected) ?? null;
  const selDef = sel ? s.graphKinds.find((k) => k.kind === sel.kind) : null;
  const schemaProps = useMemo(() => resolveProps(selDef?.param_schema), [selDef]);

  return (
    <div className={`grid h-full ${inspector ? "grid-cols-[1fr_240px]" : "grid-cols-1"}`} data-testid="graph-panel">
      <div className="relative">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          onNodeClick={(_, n) => setSelected(n.id)}
          onNodeMouseEnter={(e, n) => hoverNode(n.id, e.clientX, e.clientY)}
          onNodeMouseLeave={leaveNode}
          onNodeDragStart={leaveNode}
          onPaneClick={() => setSelected(null)}
          onDelete={({ nodes: dn, edges: de }) =>
            void (async () => {
              // Sequential: every response carries the whole graph, so concurrent ops could race to a stale view.
              // graph.remove_node drops a node's edges itself; only free-standing edge deletions need graph.disconnect.
              const gone = new Set(dn.map((n) => n.id));
              for (const e of de) if (!gone.has(e.source) && !gone.has(e.target)) await s.disconnectNodes(e.source, e.sourceHandle ?? "out", e.target, e.targetHandle ?? "in");
              for (const n of dn) await s.removeNode(n.id);
              setSelected((cur) => (cur && gone.has(cur) ? null : cur));
            })()
          }
          onNodeDragStop={(_, _n, dragged) =>
            void (async () => {
              for (const n of dragged) await s.moveNode(n.id, [Math.round(n.position.x), Math.round(n.position.y)]);
            })()
          }
          fitView
          minZoom={0.1}
          colorMode="dark"
          proOptions={{ hideAttribution: true }}
          deleteKeyCode={["Backspace", "Delete"]}
        >
          <Background gap={16} color="#2a2a2a" />
          <Controls showInteractive={false} />
        </ReactFlow>
        {thumb && (
          <div className="pointer-events-none fixed z-50 rounded border border-jd-line bg-jd-bg-1 p-1 shadow" style={{ left: thumb.x + 16, top: thumb.y + 16 }} data-testid="node-thumb">
            <canvas ref={thumbCanvas} className="block h-24 w-24 bg-jd-bg-2" style={{ imageRendering: "pixelated" }} data-testid="node-thumb-canvas" data-ready={thumb.preview ? "1" : "0"} />
            <div className="mt-0.5 text-center text-[10px] text-jd-fg-2">{thumb.preview ? `${thumb.preview.min.toFixed(0)} … ${thumb.preview.max.toFixed(0)} m` : "…"}</div>
          </div>
        )}
        <div className="absolute top-2 left-2 flex items-center gap-1">
          <select className="jd-field" value={newKind} onChange={(e) => setNewKind(e.target.value)} data-testid="graph-kind">
            {s.graphKinds.map((k) => (
              <option key={k.kind} value={k.kind} title={k.kind}>
                {shortKind(k.kind)}
              </option>
            ))}
          </select>
          <button className="btn" onClick={() => void s.addNode(newKind)} disabled={!s.project || s.busy} data-testid="graph-add">
            <Plus size={12} /> Add
          </button>
          <button className="btn px-1.5" title="Undo graph edit (⌥⌘Z)" onClick={() => void s.undoGraph()} disabled={!s.graph.undo_depth || s.busy} data-testid="graph-undo">
            <Undo2 size={12} />
          </button>
          <button className="btn px-1.5" title="Redo graph edit (⇧⌥⌘Z)" onClick={() => void s.redoGraph()} disabled={!s.graph.redo_depth || s.busy} data-testid="graph-redo">
            <Redo2 size={12} />
          </button>
          <button className={`btn px-1.5 ${s.live ? "text-jd-accent" : ""}`} title={s.live ? "Live preview on: every edit re-evaluates at preview resolution" : "Live preview off"} onClick={() => s.setLive(!s.live)} aria-pressed={s.live} data-testid="graph-live">
            {s.live ? <Zap size={12} /> : <ZapOff size={12} />}
          </button>
          <select
            className="jd-field"
            title="Evaluate only this output and what feeds it (the gate pattern: erode a coarse chain at the preview lod, then read it back with jd.input.layer in the fine chain)"
            value={s.evalOutput ?? ""}
            onChange={(e) => s.setEvalOutput(e.target.value || null)}
            disabled={!s.project}
            data-testid="graph-only"
          >
            <option value="">all outputs</option>
            {Object.values(s.graph.nodes)
              .filter((n) => n.kind.startsWith("jd.output.height@"))
              .map((n) => String((n.params as { layer?: unknown }).layer ?? "height"))
              .filter((v, i, a) => a.indexOf(v) === i)
              .map((layer) => (
                <option key={layer} value={layer}>
                  only {layer}
                </option>
              ))}
          </select>
          <button className="btn btn-primary" title="Evaluate at full resolution" onClick={() => void s.evaluateGraph()} disabled={!s.project || s.busy} data-testid="graph-evaluate">
            <Play size={12} /> Evaluate
          </button>
          <label className="ml-2 flex items-center gap-1 text-xs text-jd-fg-1" title="Timeline: keyframed parameters (t:v, t:v in the inspector) resolve here; journaled, undoable">
            <Clock size={12} />
            <Range min={0} max={1} step={0.01} value={s.graph.time ?? 0} onChange={(e) => s.setTime(Number(e.target.value))} disabled={!s.project} className="w-24" onReset={() => s.setTime(0)} data-testid="graph-time" />
            <span className="w-8 font-mono" data-testid="graph-time-value">{(s.graph.time ?? 0).toFixed(2)}</span>
          </label>
        </div>
      </div>
      <aside className={`overflow-y-auto border-l border-jd-line bg-jd-bg-1 p-2 text-xs ${inspector ? "" : "hidden"}`} data-testid="graph-inspector">
        {!sel && <div className="text-jd-fg-2">Select a node to edit its parameters. Drag from an output handle to an input to connect.</div>}
        {sel && (
          <>
            <div className="text-[12px] font-semibold text-jd-fg-0">{sel.ui?.label ?? shortKind(sel.kind)}</div>
            <div className="mb-2 font-mono text-[11px] text-jd-fg-2">{sel.kind}</div>
            {PRESETS[sel.kind] && (
              <label className="flex min-h-7 items-center justify-between gap-2" title="A named set of parameters for this kind; each value is one journaled edit">
                <span className="text-jd-fg-1">preset</span>
                <select
                  className="w-28 jd-field"
                  value=""
                  onChange={(e) => {
                    const pr = PRESETS[sel.kind]?.find((x) => x.name === e.target.value);
                    if (!pr) return;
                    void (async () => {
                      for (const [k, v] of Object.entries(pr.params)) await s.setParam(sel.id, k, v);
                    })();
                  }}
                  data-testid="node-preset"
                >
                  <option value="">choose…</option>
                  {PRESETS[sel.kind]?.map((pr) => (
                    <option key={pr.name} value={pr.name}>
                      {pr.name}
                    </option>
                  ))}
                </select>
              </label>
            )}
            {Object.entries(schemaProps).map(([key, prop]) => {
              const current = sel.params?.[key];
              const isNum = prop.type === "number" || prop.type === "integer";
              // f32 params arrive as f64 (0.55 → 0.550000011920929); show 7 significant digits.
              const show = (v: unknown): string => (Array.isArray(v) ? v.map(show).join(",") : typeof v === "number" ? String(Number(v.toPrecision(7))) : String(v));
              return (
                <label key={key} className={`flex min-h-7 gap-2 ${prop.structured && prop.items ? "flex-col items-stretch py-1" : "items-center justify-between"}`} title={prop.description}>
                  <span className="text-jd-fg-1">{key}</span>
                  {prop.structured && prop.items ? (
                    <StructuredEditor
                      columns={prop.items}
                      rows={Array.isArray(current) ? (current as Record<string, unknown>[]) : []}
                      testId={`param-${key}`}
                      onChange={(rows) => void s.setParam(sel.id, key, rows.length === 0 ? null : rows)}
                    />
                  ) : prop.structured || prop.nested ? (
                    <span className="text-jd-fg-2" data-testid={`param-${key}-count`}>
                      {Array.isArray(current) ? `${current.length} ${prop.nested ? "points" : "recorded"}` : "none"}
                    </span>
                  ) : prop.enum ? (
                    <select className="w-28 jd-field" value={String(current ?? prop.default ?? prop.enum[0])} onChange={(e) => void s.setParam(sel.id, key, e.target.value)}>
                      {prop.enum.map((v) => (
                        <option key={v}>{v}</option>
                      ))}
                    </select>
                  ) : (
                    <input
                      className={`w-28 jd-field text-right ${keysOf(current) ? "text-jd-accent" : ""}`}
                      defaultValue={current === undefined ? "" : keysOf(current) ? keysText(keysOf(current)!) : show(current)}
                      placeholder={prop.default === undefined ? "" : show(prop.default)}
                      title={`${prop.description ?? ""}\nKeyframes: t:v, t:v (0:1, 1:300) resolve at the timeline position`.trim()}
                      onBlur={(e) => {
                        const raw = e.target.value.trim();
                        const keyed = raw === "" ? null : parseKeys(raw, isNum);
                        const value: unknown = raw === "" ? null : (keyed ?? (raw.includes(",") ? raw.split(",").map(num) : isNum ? num(raw) : raw === "true" ? true : raw === "false" ? false : raw));
                        // Leaving a field is not an edit: an unchanged value writes nothing (no undo step, no re-evaluation).
                        if (JSON.stringify(value) === JSON.stringify(current ?? null)) return;
                        void s.setParam(sel.id, key, value);
                      }}
                    />
                  )}
                </label>
              );
            })}
            <button className="btn mt-2 text-jd-error" onClick={() => void s.removeNode(sel.id).then(() => setSelected(null))} data-testid="graph-remove">
              <Trash2 size={12} /> Remove node
            </button>
          </>
        )}
      </aside>
    </div>
  );
}

/** The graph and, unless `inspector` is false (its dock hidden), the selected node's inspector. */
export default function GraphPanel({ inspector = true }: { inspector?: boolean }) {
  return (
    <ReactFlowProvider>
      <Inner inspector={inspector} />
    </ReactFlowProvider>
  );
}
