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
import { Play, Plus, Redo2, Trash2, Undo2, Zap, ZapOff } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { type GraphNode, useStore } from "../state";

type JdNodeData = { kind: string; label: string; params: Record<string, unknown>; inputs: string[]; outputs: string[]; selected?: boolean };

function JdNode({ data }: NodeProps<FlowNode<JdNodeData>>) {
  const summary = Object.entries(data.params)
    .slice(0, 3)
    .map(([k, v]) => `${k}=${Array.isArray(v) ? v.join("…") : String(v)}`)
    .join(" · ");
  return (
    <div className="min-w-40 rounded border border-jd-line bg-jd-bg-2 text-xs text-jd-fg-0 shadow">
      <div className="border-b border-jd-line px-2 py-1 font-semibold">{data.label}</div>
      <div className="px-2 py-1 text-[11px] text-jd-fg-2">{data.kind}</div>
      {summary && <div className="truncate px-2 pb-1 text-[11px] text-jd-fg-1">{summary}</div>}
      {data.inputs.map((p, i) => (
        <Handle key={p} id={p} type="target" position={Position.Left} style={{ top: 28 + i * 14 }} title={p} />
      ))}
      {data.outputs.map((p, i) => (
        <Handle key={p} id={p} type="source" position={Position.Right} style={{ top: 28 + i * 14 }} title={p} />
      ))}
    </div>
  );
}

const nodeTypes = { jd: JdNode };

type Prop = { type?: string; enum?: string[]; description?: string; default?: unknown };
type Schema = { properties?: Record<string, Prop & { $ref?: string }>; $defs?: Record<string, { enum?: string[]; oneOf?: { const?: unknown }[] }> };

/** Flatten schemars output for the inspector: follow `$ref`s into `$defs` and turn `oneOf` consts into an enum. */
function resolveProps(schema: unknown): Record<string, Prop> {
  const sc = (schema ?? {}) as Schema;
  const out: Record<string, Prop> = {};
  for (const [key, prop] of Object.entries(sc.properties ?? {})) {
    const def = prop.$ref ? sc.$defs?.[prop.$ref.split("/").pop() ?? ""] : undefined;
    const en = prop.enum ?? def?.enum ?? def?.oneOf?.map((o) => String(o.const));
    out[key] = { type: prop.type, description: prop.description, default: prop.default, ...(en ? { enum: en } : {}) };
  }
  return out;
}

function toFlow(nodes: GraphNode[], kinds: Map<string, { inputs: string[]; outputs: string[] }>): FlowNode<JdNodeData>[] {
  return nodes.map((n) => ({
    id: n.id,
    type: "jd",
    position: { x: n.ui?.position?.[0] ?? 0, y: n.ui?.position?.[1] ?? 0 },
    data: {
      kind: n.kind,
      label: n.ui?.label ?? n.kind.split("@")[0]?.split(".").slice(-1)[0] ?? n.kind,
      params: n.params,
      inputs: kinds.get(n.kind)?.inputs ?? [],
      outputs: kinds.get(n.kind)?.outputs ?? ["out"],
    },
  }));
}

function Inner() {
  const s = useStore();
  const kindMap = useMemo(() => new Map(s.graphKinds.map((k) => [k.kind, { inputs: k.inputs.map((p) => p.name), outputs: k.outputs.map((p) => p.name) }])), [s.graphKinds]);
  const [nodes, setNodes, onNodesChange] = useNodesState<FlowNode<JdNodeData>>([]);
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
    <div className="grid h-full grid-cols-[1fr_240px]" data-testid="graph-panel">
      <div className="relative">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          onNodeClick={(_, n) => setSelected(n.id)}
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
          colorMode="dark"
          proOptions={{ hideAttribution: true }}
          deleteKeyCode={["Backspace", "Delete"]}
        >
          <Background gap={16} color="#2a2a2a" />
          <Controls showInteractive={false} />
        </ReactFlow>
        <div className="absolute top-2 left-2 flex items-center gap-1">
          <select className="rounded border border-jd-line bg-jd-bg-2 px-1 py-0.5 text-xs" value={newKind} onChange={(e) => setNewKind(e.target.value)} data-testid="graph-kind">
            {s.graphKinds.map((k) => (
              <option key={k.kind} value={k.kind}>
                {k.kind}
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
          <button className="btn" title="Evaluate at full resolution" onClick={() => void s.evaluateGraph()} disabled={!s.project || s.busy} data-testid="graph-evaluate">
            <Play size={12} /> Evaluate
          </button>
        </div>
      </div>
      <aside className="overflow-y-auto border-l border-jd-line bg-jd-bg-1 p-2 text-xs" data-testid="graph-inspector">
        {!sel && <div className="text-jd-fg-2">Select a node to edit its parameters. Drag from an output handle to an input to connect.</div>}
        {sel && (
          <>
            <div className="mb-1 font-semibold">{sel.ui?.label ?? sel.kind}</div>
            <div className="mb-2 font-mono text-[11px] text-jd-fg-2">{sel.kind}</div>
            {Object.entries(schemaProps).map(([key, prop]) => {
              const current = sel.params[key];
              const isNum = prop.type === "number" || prop.type === "integer";
              return (
                <label key={key} className="mb-1 flex items-center justify-between gap-2" title={prop.description}>
                  <span className="text-jd-fg-1">{key}</span>
                  {prop.enum ? (
                    <select className="w-28 rounded border border-jd-line bg-jd-bg-2 px-1 text-xs" value={String(current ?? prop.default ?? prop.enum[0])} onChange={(e) => void s.setParam(sel.id, key, e.target.value)}>
                      {prop.enum.map((v) => (
                        <option key={v}>{v}</option>
                      ))}
                    </select>
                  ) : (
                    <input
                      className="w-28 rounded border border-jd-line bg-jd-bg-2 px-1 text-right text-xs"
                      defaultValue={current === undefined ? "" : Array.isArray(current) ? current.join(",") : String(current)}
                      placeholder={prop.default === undefined ? "" : Array.isArray(prop.default) ? prop.default.join(",") : String(prop.default)}
                      onBlur={(e) => {
                        const raw = e.target.value.trim();
                        if (raw === "") return void s.setParam(sel.id, key, null);
                        const value: unknown = raw.includes(",") ? raw.split(",").map(Number) : isNum ? Number(raw) : raw === "true" ? true : raw === "false" ? false : raw;
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

export default function GraphPanel() {
  return (
    <ReactFlowProvider>
      <Inner />
    </ReactFlowProvider>
  );
}
