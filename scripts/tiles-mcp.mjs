// Tile-level re-evaluation through the long-lived MCP server (`docs/user/cli-quickstart.md`,
// "Sculpt and re-evaluate"): usage `node scripts/tiles-mcp.mjs <project.jdproj>` after
// `cargo build --release -p jd-mcp` and `jd new <project> --resolution 4096`; needs the MCP SDK
// from apps/desktop/node_modules. Prints the per-node report of a cold run, two strokes and an
// unchanged run, then compares the thermal layer's hash with a cold server's.
// Tile-level re-evaluation through the long-lived MCP server: cold run, then strokes.
import path from "node:path";
const root = new URL("..", import.meta.url).pathname.replace(/\/$/, "");
const { Client } = await import(path.join(root, "apps/desktop/node_modules/@modelcontextprotocol/sdk/dist/esm/client/index.js"));
const { StdioClientTransport } = await import(path.join(root, "apps/desktop/node_modules/@modelcontextprotocol/sdk/dist/esm/client/stdio.js"));
const proj = process.argv[2];
const c = new Client({ name: "jd-tiles", version: "0.0.1" });
await c.connect(new StdioClientTransport({ command: path.join(root, "target/release/jd-mcp"), args: ["--scope", "all", "--workspace", path.dirname(proj)], env: { ...process.env, JD_LOG: "error" } }));
const call = async (name, args = {}) => { const r = (await c.callTool({ name, arguments: args })).structuredContent; if (r.status !== "ok") throw new Error(name + ": " + JSON.stringify(r.error ?? r)); return r.result; };
await call("project.open", { path: proj });
const NOISE = "01JDGRAPH000000000000N01SE", HEIGHT = "01JDGRAPH0000000000HE1GHT0";
const add = async (kind, params) => (await call("graph.add_node", { kind, params })).id;
const con = (from, from_port, to, to_port) => call("graph.connect", { from, from_port, to, to_port });
const sc = await add("jd.sculpt.strokes@1", { strokes: [] });
const bl = await add("jd.filter.blur@1", { radius: 2 });
const sl = await add("jd.analysis.curvature@1", { radius_px: 8 });
const o2 = await add("jd.output.height@1", { layer: "curv" });
const th = await add("jd.erosion.thermal@1", { iterations: 30 });
const o3 = await add("jd.output.height@1", { layer: "thermal" });
await call("graph.disconnect", { from: NOISE, from_port: "out", to: HEIGHT, to_port: "in" }).catch(() => {});
await con(NOISE, "out", sc, "in"); await con(sc, "out", bl, "in"); await con(bl, "out", HEIGHT, "in"); await con(sc, "out", sl, "in"); await con(sl, "out", o2, "in"); await con(sc, "out", th, "in"); await con(th, "out", o3, "in");
const report = async (label) => {
  const r = await call("graph.evaluate", {});
  console.log(`== ${label}: total ${r.duration_ms} ms`);
  for (const n of r.nodes.filter((n) => !n.kind.startsWith("jd.output"))) console.log(`  ${n.kind.padEnd(26)} ${String(n.duration_ms).padStart(5)} ms cached=${n.cached} partial=${n.partial_tiles ?? "-"}`);
  return r;
};
await report("cold");
await call("graph.append_param", { id: sc, key: "strokes", value: { points: [[20480, 20480]], radius_m: 120, strength_m: 40 } });
await report("one 120 m stroke in the middle");
await call("graph.append_param", { id: sc, key: "strokes", value: { points: [[6000, 6000], [6800, 6800]], radius_m: 150, strength_m: -30 } });
await report("a second stroke elsewhere");
await report("nothing changed");
// Prove the spliced slope equals a cold full evaluation: compare the layer hash with a fresh server.
const stats = await call("terrain.stats", { layer: "thermal" });
console.log("thermal content hash (warm server):", stats.content_hash);
await call("project.save", {});
await c.close();
const c2 = new Client({ name: "jd-tiles-cold", version: "0.0.1" });
await c2.connect(new StdioClientTransport({ command: path.join(root, "target/release/jd-mcp"), args: ["--scope", "all", "--workspace", path.dirname(proj)], env: { ...process.env, JD_LOG: "error" } }));
const call2 = async (name, args = {}) => { const r = (await c2.callTool({ name, arguments: args })).structuredContent; if (r.status !== "ok") throw new Error(name + ": " + JSON.stringify(r.error ?? r)); return r.result; };
await call2("project.open", { path: proj });
const r2 = await call2("graph.evaluate", {});
console.log("cold server: thermal", r2.nodes.find((n) => n.kind.startsWith("jd.erosion.thermal")).duration_ms, "ms");
const stats2 = await call2("terrain.stats", { layer: "thermal" });
console.log("thermal content hash (cold server):", stats2.content_hash, stats2.content_hash === stats.content_hash ? "IDENTICAL" : "DIFFERENT");
await c2.close();
