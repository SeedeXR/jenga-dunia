// A real-world terrain through the MCP server (ADR-0014): usage
//   node scripts/geo-mcp.mjs <workspace dir> [lon lat radius_m] [provider]
// after `cargo build --release -p jd-mcp`. Fetches the DEM (default: Nairobi, 2 km, Terrarium)
// through `geo.fetch_dem` under `--scope network:geo`, places the world in the DEM's CRS,
// wires `jd.input.heightmap@1` to the height output, evaluates, prints the stats and the
// attribution, and exports the Unity and Unreal profiles so the engine checks can read them:
//   scripts/unity-import-check.sh <ws>/Real.jdproj/exports/unity
//   scripts/unreal-import-check.sh <ws>/Real.jdproj/exports/unreal
import fs from "node:fs";
import path from "node:path";
const root = new URL("..", import.meta.url).pathname.replace(/\/$/, "");
const { Client } = await import(path.join(root, "apps/desktop/node_modules/@modelcontextprotocol/sdk/dist/esm/client/index.js"));
const { StdioClientTransport } = await import(path.join(root, "apps/desktop/node_modules/@modelcontextprotocol/sdk/dist/esm/client/stdio.js"));
const ws = path.resolve(process.argv[2] ?? "/tmp/jd-scratch/geo-mcp");
const [lon, lat, radius] = [Number(process.argv[3] ?? 36.82), Number(process.argv[4] ?? -1.29), Number(process.argv[5] ?? 2000)];
const provider = process.argv[6] ?? "terrarium";
fs.mkdirSync(ws, { recursive: true });
const proj = path.join(ws, "Real.jdproj");
fs.rmSync(proj, { recursive: true, force: true });
const bin = ["target/release/jd-mcp", "target/debug/jd-mcp"].map((p) => path.join(root, p)).filter((p) => fs.existsSync(p)).sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs)[0];
const c = new Client({ name: "jd-geo", version: "0.0.1" });
await c.connect(new StdioClientTransport({ command: bin, args: ["--scope", "project:ro", "--scope", "project:rw", "--scope", "fs:write", "--scope", "export", "--scope", "network:geo", "--workspace", ws], env: process.env }));
const call = async (name, args = {}) => {
  const r = (await c.callTool({ name, arguments: args })).structuredContent;
  if (r.status !== "ok") throw new Error(`${name}: ${JSON.stringify(r.error)}`);
  return r.result;
};
const tools = (await c.listTools()).tools.map((t) => t.name);
console.log("geo tools under network:geo:", tools.filter((t) => t.startsWith("geo.")).join(", "));
await call("project.create", { path: proj, name: "Real", world: { extent_m: [2 * radius, 2 * radius], resolution_px: [513, 513], tile_size: 512, halo: 4, origin: [0, 0], seed: 42 } });
const t0 = Date.now();
const dem = await call("geo.fetch_dem", { center: [lon, lat], radius_m: radius, provider, res_m: 30 });
console.log(`geo.fetch_dem ${provider}: ${dem.px[0]}×${dem.px[1]} at ${dem.spacing[0].toFixed(2)} ${dem.crs === "EPSG:4326" ? "°" : "m"}, ${dem.height_m[0].toFixed(0)}–${dem.height_m[1].toFixed(0)} m, voids ${(dem.void_share * 100).toFixed(1)} %`);
console.log(`  fetch: ${dem.fetch.requests} requests, ${dem.fetch.cache_hits} cache hits, ${(dem.fetch.bytes / 1024).toFixed(0)} KB, ${dem.fetch.retries} retries, ${dem.fetch.connections} connections, ${(dem.fetch.bytes_per_s / 1024).toFixed(0)} KB/s, ${dem.fetch.elapsed_ms} ms (wall ${Date.now() - t0} ms)`);
await call("project.set_settings", { crs: dem.crs, origin: dem.origin });
const NOISE = "01JDGRAPH000000000000N01SE", HEIGHT = "01JDGRAPH0000000000HE1GHT0";
const node = (await call("graph.add_node", { kind: "jd.input.heightmap@1", params: { path: dem.asset, georeference: true } })).id;
await call("graph.disconnect", { from: NOISE, from_port: "out", to: HEIGHT, to_port: "in" }).catch(() => {});
await call("graph.connect", { from: node, from_port: "out", to: HEIGHT, to_port: "in" });
const ev = await call("graph.evaluate", {});
console.log(`graph.evaluate: ${ev.duration_ms} ms (${ev.nodes.map((n) => `${n.kind} ${n.duration_ms} ms`).join(", ")})`);
const st = await call("terrain.stats", { layer: "height" });
console.log(`height: ${st.stats.min.toFixed(1)}–${st.stats.max.toFixed(1)} m, mean ${st.stats.mean.toFixed(1)} m`);
const attr = await call("geo.attributions", {});
console.log(`attribution: ${attr.attribution.map((a) => a.source).join("; ")}`);
for (const p of ["unity", "unreal"]) {
  const r = await call("terrain.export_profile", { profile: p, dir: p, layer: "height" });
  console.log(`export_profile ${p}: ${r.files?.length ?? "?"} files in ${r.dir}${fs.existsSync(path.join(r.dir, "ATTRIBUTION.txt")) ? " + ATTRIBUTION.txt" : " (no ATTRIBUTION.txt!)"}`);
}
const mesh = await call("terrain.export_mesh", { layer: "height", path: path.join(proj, "exports", "real.glb") });
console.log(`export_mesh: ${mesh.vertices} vertices, ${mesh.triangles} triangles → exports/real.glb`);
await call("project.save", {});
await c.close();
console.log(`project: ${proj}`);
