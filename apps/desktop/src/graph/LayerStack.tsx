import { ArrowDown, ArrowUp, ChevronDown, ChevronRight, Eye, EyeOff, Plus, Search, Trash2 } from "lucide-react";
import { useState } from "react";
import { type GraphNode, useStore } from "../state";

/** World Creator's layer stack over the node graph: the chain that feeds the height output,
 *  top = last applied, with the side branches (masks, climate, classification) listed under it.
 *  Every row is the node it names: the eye is `graph.set_bypass`, the arrows re-wire the chain,
 *  the bin un-splices, "+" splices a new node in front of the output, a click selects the node
 *  for the inspector. */
export type LayerCategory = "shape" | "filter" | "simulation" | "mask" | "output";

export function categoryOf(kind: string): LayerCategory {
  const ns = kind.split(".")[1] ?? "";
  if (ns === "noise" || ns === "input" || ns === "shape") return "shape";
  if (ns === "erosion" || ns === "climate" || ns === "volcanic" || ns === "hydrology" || ns === "ocean") return "simulation";
  if (ns === "analysis" || ns === "mask" || ns === "biome") return "mask";
  if (ns === "output") return "output";
  return "filter";
}

/** The nodes feeding `output` through their `in` ports, nearest first (the chain), and the rest. */
export function stackOf(nodes: GraphNode[], edges: { from: { node: string; port: string }; to: { node: string; port: string } }[], layer = "height"): { chain: GraphNode[]; others: GraphNode[] } {
  const output = nodes.find((n) => n.kind.startsWith("jd.output.height@") && String(n.params?.layer ?? "height") === layer);
  const chain: GraphNode[] = [];
  const seen = new Set<string>();
  let cur = output;
  while (cur) {
    seen.add(cur.id);
    const feed = edges.find((e) => e.to.node === cur!.id && e.to.port === "in");
    const prev = feed ? nodes.find((n) => n.id === feed.from.node) : undefined;
    if (!prev || seen.has(prev.id)) break;
    chain.push(prev);
    cur = prev;
  }
  const others = nodes.filter((n) => !seen.has(n.id) && !n.kind.startsWith("jd.output."));
  return { chain, others };
}

const BADGE: Record<LayerCategory, string> = { shape: "bg-emerald-900/60 text-emerald-200", filter: "bg-sky-900/60 text-sky-200", simulation: "bg-amber-900/60 text-amber-200", mask: "bg-fuchsia-900/60 text-fuchsia-200", output: "bg-zinc-800 text-zinc-300" };

export default function LayerStack() {
  const s = useStore();
  const { chain, others } = stackOf(s.graph.nodes, s.graph.edges);
  const [kind, setKind] = useState("jd.erosion.thermal@1");
  // World Creator's layer tree: a search box over every row, and the side branches grouped by
  // category under collapsible headings with their counts.
  const [query, setQuery] = useState("");
  const [closed, setClosed] = useState<Record<string, boolean>>({});
  const q = query.trim().toLowerCase();
  const matches = (n: GraphNode) => !q || n.kind.toLowerCase().includes(q) || (n.ui?.label ?? "").toLowerCase().includes(q);
  const short = (n: GraphNode) => n.ui?.label ?? (n.kind.split(".").slice(1).join(".").replace(/@\d+$/, ""));
  // Blend readout: how a layer combines with the chain — the blend nodes' op and opacity with the
  // layer wired into `b`, a placed shape's `op` × opacity — so the stack reads like a layer list.
  const readout = (n: GraphNode): string | null => {
    const blend = /^jd\.math\.(min|max|lerp)@/.exec(n.kind);
    const pct = (v: unknown) => `${Math.round(Number(v ?? 1) * 100)} %`;
    if (blend) {
      const feed = s.graph.edges.find((e) => e.to.node === n.id && e.to.port === "b");
      const src = feed ? s.graph.nodes.find((x) => x.id === feed.from.node) : undefined;
      return `${blend[1]} · ${pct(n.params?.opacity)}${src ? ` · ${short(src)}` : " · b unwired"}`;
    }
    if (n.kind.startsWith("jd.shape.")) return `${String(n.params?.op ?? (n.kind.includes("polygon") ? "lerp" : "add"))} · ${pct(n.params?.opacity)}`;
    return null;
  };
  const row = (n: GraphNode, i: number, inChain: boolean) => {
    const cat = categoryOf(n.kind);
    const selected = s.selectedNode === n.id;
    return (
      <li key={n.id} className={`flex items-center gap-1 rounded px-1 py-0.5 text-xs ${selected ? "bg-jd-bg-2" : ""} ${n.bypass ? "opacity-50" : ""}`} data-testid={`layer-row-${n.id}`}>
        <button className="btn px-1 py-0" title={n.bypass ? "Bypassed: click to apply this layer again" : "Bypass: pass the input through unchanged (graph.set_bypass)"} onClick={() => void s.setBypass(n.id, !n.bypass)} aria-pressed={!n.bypass} data-testid={`layer-eye-${n.id}`}>
          {n.bypass ? <EyeOff size={12} /> : <Eye size={12} />}
        </button>
        <button className="min-w-0 flex-1 truncate text-left" title={n.kind} onClick={() => s.setSelectedNode(n.id)}>
          {short(n)}
          {readout(n) && (
            <span className="ml-1 text-[10px] text-jd-fg-2" data-testid={`layer-op-${n.id}`}>
              {readout(n)}
            </span>
          )}
        </button>
        <span className={`rounded px-1 text-[10px] ${BADGE[cat]}`}>{cat}</span>
        {inChain && (
          <>
            <button className="btn px-1 py-0" title="Apply later (move up the stack)" onClick={() => void s.swapInChain(n.id, "up")} disabled={i === 0} data-testid={`layer-up-${n.id}`}>
              <ArrowUp size={12} />
            </button>
            <button className="btn px-1 py-0" title="Apply earlier (move down the stack)" onClick={() => void s.swapInChain(n.id, "down")} disabled={i === chain.length - 1} data-testid={`layer-down-${n.id}`}>
              <ArrowDown size={12} />
            </button>
            <button className="btn px-1 py-0 text-jd-error" title="Remove this layer and close the gap" onClick={() => void s.unsplice(n.id)} disabled={n.kind.startsWith("jd.noise.") || n.kind.startsWith("jd.input.")} data-testid={`layer-remove-${n.id}`}>
              <Trash2 size={12} />
            </button>
          </>
        )}
      </li>
    );
  };
  return (
    <section data-testid="layer-stack">
      <h2 className="mt-4 mb-1 jd-section">Layers</h2>
      <div className="mb-1 flex gap-1">
        <select className="min-w-0 flex-1 rounded border border-jd-line bg-jd-bg-2 px-1 text-xs text-jd-fg-0" value={kind} onChange={(e) => setKind(e.target.value)} title="Node kind to add on top of the stack (in front of the height output)" data-testid="layer-add-kind">
          {s.graphKinds
            // Chain kinds that need no settings to be added (a shape, an action, a pattern's kind… have their own tools).
            .filter((k) => k.inputs.some((p) => p.name === "in") && k.outputs.some((p) => p.name === "out") && !((k.param_schema as { required?: string[] } | null)?.required?.length))
            .map((k) => (
              <option key={k.kind} value={k.kind}>
                {k.kind.split(".").slice(1).join(".").replace(/@\d+$/, "")}
              </option>
            ))}
        </select>
        <button className="btn px-1.5" onClick={() => void s.insertLayer(kind)} disabled={!s.project || s.busy} title="Splice the chosen node in front of the height output" data-testid="layer-add">
          <Plus size={12} />
        </button>
      </div>
      <label className="mb-1 flex items-center gap-1 rounded-md border border-white/5 bg-jd-bg-2 px-1.5 py-0.5 text-xs text-jd-fg-2 focus-within:border-jd-accent">
        <Search size={12} />
        <input className="min-w-0 flex-1 bg-transparent text-jd-fg-0 outline-none placeholder:text-jd-fg-2" placeholder="Search layers" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search layers" data-testid="layer-search" />
      </label>
      {(() => {
        const head = (key: string, label: string, count: number) => (
          <button className="flex w-full items-center gap-1 py-0.5 text-left text-[11px] font-semibold text-jd-fg-1" onClick={() => setClosed((c) => ({ ...c, [key]: !c[key] }))} aria-expanded={!closed[key]} data-testid={`layer-group-${key}`}>
            {closed[key] ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
            {label}
            <span className="ml-auto rounded bg-jd-bg-3 px-1 text-[10px] font-normal text-jd-fg-2">{count}</span>
          </button>
        );
        const groups: [LayerCategory, string][] = [["shape", "Shape layers"], ["filter", "Filters"], ["simulation", "Simulation layers"], ["mask", "Mask layers"]];
        return (
          <>
            {head("stack", "Height stack", chain.length)}
            {!closed.stack && (
              <ol className="mb-1" data-testid="layer-chain">
                {chain.map((n, i) => (matches(n) ? row(n, i, true) : null))}
              </ol>
            )}
            {others.length > 0 && (
              <ul data-testid="layer-others">
                {groups.map(([cat, label]) => {
                  const rows = others.filter((n) => categoryOf(n.kind) === cat);
                  if (rows.length === 0) return null;
                  return (
                    <li key={cat}>
                      {head(cat, label, rows.length)}
                      {!closed[cat] && <ul className="ml-2">{rows.map((n, i) => (matches(n) ? row(n, i, false) : null))}</ul>}
                    </li>
                  );
                })}
              </ul>
            )}
          </>
        );
      })()}
    </section>
  );
}
