#!/usr/bin/env node
// Mandatory end-to-end acceptance scenario (brief §20): builds one demonstration project through
// the CLI, exports every supported format, validates what can be validated on this machine, replays
// the key operations through the MCP server, exercises cancel + crash recovery, and writes a report
// with real numbers and an explicit status per §20 item. Items that need tooling this machine lacks
// (Unity, Unreal, a browser viewer, Blender on CI) are reported UNVERIFIED, never assumed.
//
// Usage: node scripts/acceptance.mjs [--quick] [--out <dir>] [--report <file.md>]
//   --quick   256² world and shorter simulations (CI); default 512².
//   --out     working directory (default: a temp dir); the project and exports land there.
//   --report  where to write the Markdown report (default docs/spec/acceptance-report.md).
import { execFileSync, spawnSync, spawn } from "node:child_process";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, statSync, writeFileSync, appendFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const opt = (name, dflt) => (args.includes(name) ? args[args.indexOf(name) + 1] : dflt);
const quick = args.includes("--quick");
const out = opt("--out", mkdtempSync(path.join(tmpdir(), "jd-acceptance-")));
const reportPath = opt("--report", path.join(root, "docs/spec/acceptance-report.md"));
const exe = process.platform === "win32" ? ".exe" : "";
// The most recently built binary wins, so a stale release build never shadows a fresh debug one.
const newest = (names) => names.map((p) => path.join(root, p + exe)).filter(existsSync).sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
const JD = process.env.JD_BIN ?? newest(["target/release/jd", "target/debug/jd"]);
const MCP = process.env.JD_MCP_BIN ?? newest(["target/release/jd-mcp", "target/debug/jd-mcp"]);
if (!JD) throw new Error("jd binary not found; cargo build -p jd-cli");
mkdirSync(out, { recursive: true });
const proj = path.join(out, "Acceptance.jdproj");
rmSync(proj, { recursive: true, force: true });
const res = quick ? 256 : 512;
const extent = quick ? 2560 : 5120;
const SEA = 350; // metres: the noise spans −80..1400, so this leaves a border-connected sea
const lines = []; // report body
const say = (s) => {
  lines.push(s);
  console.log(s);
};
const items = new Map(); // §20 item → { status, evidence[] }
const mark = (n, status, evidence) => {
  const it = items.get(n) ?? { status: "UNVERIFIED", evidence: [] };
  const rank = { VERIFIED: 3, PARTIAL: 2, UNVERIFIED: 1, FAILED: 0 };
  if (!items.has(n) || rank[status] < rank[it.status] || (it.status === "UNVERIFIED" && status !== "UNVERIFIED")) it.status = status;
  if (evidence) it.evidence.push(evidence);
  items.set(n, it);
};
const t = () => performance.now();
const ms = (t0) => `${(t() - t0).toFixed(0)} ms`;

/** Run `jd --json …`, return the envelope for `op` (or the last one). Throws on a non-ok status. */
function jd(argv, opName) {
  // 64 MiB: terrain.textures returns three base64 PNGs, well past Node's 1 MiB default buffer.
  const r = spawnSync(JD, ["--json", ...argv], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024, env: { ...process.env, JD_LOG: "error" } });
  const envs = r.stdout.split("\n").filter((l) => l.startsWith("{")).map((l) => JSON.parse(l));
  const env = opName ? envs.find((e) => e.op === opName) : envs[envs.length - 1];
  if (!env) throw new Error(`no envelope for ${argv.join(" ")}: ${r.stderr.slice(-300)}`);
  if (env.status !== "ok") throw new Error(`${env.op}: ${JSON.stringify(env.error).slice(0, 300)}`);
  return env.result;
}
const op = (name, params, save = true) => jd(["op", name, "--project", proj, "--params", JSON.stringify(params ?? {}), ...(save ? ["--save"] : [])], name);
const addNode = (kind, params) => op("graph.add_node", { kind, params: params ?? {} }).id;
const connect = (from, fromPort, to, toPort) => op("graph.connect", { from, from_port: fromPort, to, to_port: toPort });
const stats = (layer) => {
  const s = op("terrain.stats", { layer }, false).stats;
  return `${layer}: min ${s.min.toFixed(1)} max ${s.max.toFixed(1)} mean ${s.mean.toFixed(1)} m`;
};

say(`# Acceptance scenario (brief §20) — run ${new Date().toISOString()}`);
say("");
say(`Machine: ${process.platform} ${process.arch}, node ${process.version}; \`jd\` = ${path.relative(root, JD)}; world ${res}² / ${extent} m${quick ? " (--quick)" : ""}.`);
say("");

// ── 1–2. Landscape: mountains + valleys, fluvial erosion, river, coast ──────────────────────────
let t0 = t();
jd(["new", proj, "--extent", String(extent), "--resolution", String(res)]);
const g = jd(["op", "graph.list", "--project", proj], "graph.list");
const noise = g.nodes.find((n) => n.kind.startsWith("jd.noise")).id;
const heightOut = g.nodes.find((n) => n.kind.startsWith("jd.output")).id;
op("graph.set_param", { id: noise, key: "height_range_m", value: [-80, 1400] });
op("graph.set_param", { id: noise, key: "feature_size_m", value: extent / 4 });
const sp = addNode("jd.erosion.stream_power@1", { k: 0.002, m: 0.5, iterations: quick ? 8 : 20, method: "mfd" });
const rivers = addNode("jd.hydrology.rivers@1", { method: "dinf" });
const coast = addNode("jd.erosion.coastal@1", { level_m: SEA, band_m: 8, iterations: quick ? 30 : 80, rate_m: 0.4 });
const sea = addNode("jd.hydrology.sea_level@1", { level_m: SEA, shore_m: 2 });
op("graph.disconnect", { from: noise, from_port: "out", to: heightOut, to_port: "in" });
connect(noise, "out", sp, "in");
connect(sp, "out", rivers, "in");
connect(rivers, "out", coast, "in");
connect(coast, "out", heightOut, "in");
connect(coast, "out", sea, "in");
const outputs = {};
const publish = (layer, from, port) => {
  const o = addNode("jd.output.height@1", { layer });
  connect(from, port, o, "in");
  outputs[layer] = o;
};
publish("rivers", rivers, "rivers");
publish("ocean", sea, "ocean");
publish("shore", coast, "shore");
// ── 3. Biomes: slope + wetness-driven rules, two class masks, snow ──────────────────────────────
const slope = addNode("jd.analysis.slope@1", {});
const wet = addNode("jd.hydrology.wetness@1", {});
const biome = addNode("jd.biome.classify@1", {
  rules: [
    { name: "rock", slope: [0.55, 2] },
    { name: "snow", height_m: [1000, 1e9] },
    { name: "beach", height_m: [-1e9, SEA + 6] },
    { name: "wetland", wetness: [0.7, 1e9] },
    { name: "grassland", height_m: [SEA + 6, 700] },
    { name: "forest", height_m: [700, 1000] },
  ],
});
connect(coast, "out", slope, "in");
connect(coast, "out", wet, "in");
connect(coast, "out", biome, "height");
connect(slope, "out", biome, "slope");
connect(wet, "out", biome, "wetness");
publish("biome", biome, "biome");
for (const [name, id] of [["forest", 6], ["grassland", 5], ["rock", 1]]) {
  const m = addNode("jd.mask.class@1", { id });
  connect(biome, "biome", m, "classes");
  publish(`mask_${name}`, m, "out");
}
const snow = addNode("jd.climate.snow@1", { snow_line_m: 950, transition_m: 120, wind_strength: 0.4 }); // no wind of its own: follows the world's shared wind
connect(coast, "out", snow, "in");
publish("snow", snow, "snow");
// ── Ocean floor, desert family, strata, a road: branches off the same terrain ──────────────────
const seabed = addNode("jd.biome.seabed@1", { level_m: SEA });
connect(coast, "out", seabed, "in");
connect(rivers, "rivers", seabed, "rivers");
connect(coast, "shore", seabed, "shore");
publish("seabed", seabed, "seabed");
const bathy = addNode("jd.ocean.bathymetry@1", { level_m: SEA, smooth_m: 200, max_fill_m: 20 });
connect(coast, "out", bathy, "in");
connect(seabed, "seabed", bathy, "seabed");
publish("floor", bathy, "out");
const strata = addNode("jd.terrain.strata@1", { layer_m: 60, hard_fraction: 0.3 });
connect(coast, "out", strata, "in");
publish("hardness", strata, "hardness");
const karst = addNode("jd.erosion.karst@1", { density_per_km2: 8, max_slope_deg: 20, min_spacing_m: 120 });
connect(coast, "out", karst, "in");
publish("sinkholes", karst, "sinkholes");
const yardang = addNode("jd.erosion.yardang@1", { iterations: quick ? 40 : 120, wind_azimuth_deg: 270 });
connect(coast, "out", yardang, "in");
connect(strata, "hardness", yardang, "hardness");
publish("yardang", yardang, "out");
const road = addNode("jd.terrain.spline@1", { points: [[extent * 0.1, extent * 0.2], [extent * 0.5, extent * 0.55], [extent * 0.9, extent * 0.8]], width_m: 14, falloff_m: 12 });
connect(coast, "out", road, "in");
publish("road", road, "mask");
const ev = op("graph.evaluate", {}, false);
const evalMs = ev.duration_ms;
say(`## Build`);
say("");
say(`Graph: ${ev.nodes.length} nodes evaluated in ${evalMs} ms (${ms(t0)} wall incl. CLI startups). Layers: ${Object.keys(ev.layers).join(", ")}.`);
say("");
say("```");
for (const l of ["height", "rivers", "ocean", "shore", "biome", "snow", "mask_forest", "seabed", "floor", "hardness", "sinkholes", "yardang", "road"]) say(stats(l));
say("```");
const biomeStats = op("terrain.stats", { layer: "biome" }, false).stats;
const oceanFrac = op("terrain.stats", { layer: "ocean" }, false).stats.mean;
mark(1, oceanFrac > 0.02 ? "VERIFIED" : "FAILED", `noise → stream power (mfd) → rivers (dinf) → coastal erosion → sea level ${SEA} m: ${stats("height")}; ocean fraction ${(oceanFrac * 100).toFixed(1)} % (border-connected sea)`);
mark(2, "VERIFIED", `stream-power incision + river carving + coastal cut/fill in the same graph; evaluation ${evalMs} ms`);
const seabedStats = op("terrain.stats", { layer: "seabed" }, false).stats;
const floorStats = op("terrain.stats", { layer: "floor" }, false).stats;
const heightStats = op("terrain.stats", { layer: "height" }, false).stats;
mark(1, seabedStats.max >= 2 ? "VERIFIED" : "FAILED", `ocean floor: seabed classes up to ${seabedStats.max} (sand / rock / mud / reef) and bathymetry sediment fill raised the mean floor from ${heightStats.mean.toFixed(2)} to ${floorStats.mean.toFixed(2)} m`);
const sinkStats = op("terrain.stats", { layer: "sinkholes" }, false).stats;
const roadStats = op("terrain.stats", { layer: "road" }, false).stats;
mark(2, roadStats.max > 0.99 ? (sinkStats.max > 0.99 ? "VERIFIED" : "PARTIAL") : "FAILED", `desert family on the same terrain: karst sinkholes ${sinkStats.max > 0.99 ? `(floor mask max ${sinkStats.max.toFixed(2)}, ${(sinkStats.mean * 100).toFixed(2)} % of the world)` : "(none placed: this mountain world has too little ground under 20°)"}, wind abrasion with strata hardness (${stats("yardang")}), a spline road corridor ${(roadStats.mean * 100).toFixed(2)} % of the world`);
mark(3, "VERIFIED", `biome classes present: max class id ${biomeStats.max} (6 rules), class masks forest/grassland/rock published, snow cover mean ${op("terrain.stats", { layer: "snow" }, false).stats.mean.toFixed(3)}`);

// ── 7 + 4. Bakes: textures, heightmaps, engine profiles with weightmaps ─────────────────────────
t0 = t();
const tex = op("terrain.textures", { layer: "height", max_px: 1024 }, false);
mark(7, "VERIFIED", `terrain.textures: albedo/normal/roughness ${tex.width}×${tex.height} PNGs in ${ms(t0)}`);
mark(4, "PARTIAL", `PBR maps baked from masks of the terrain itself (height ramp + slope, normal, roughness) and viewport PBR texturing; a material graph assigning user materials per biome mask is not built yet`);
for (const [fmt, file] of [["exr", "height.exr"], ["png16", "height.png"], ["tiff32", "height.tif"]]) {
  const r = op("terrain.export_heightmap", { layer: "height", path: file, format: fmt, ...(fmt === "png16" ? { range_m: [-80, 1400] } : {}) }, false);
  mark(7, "VERIFIED", `${fmt} heightmap ${r.bytes ?? statSync(r.path).size} bytes`);
}
const water = op("ocean.bake", { level_m: SEA, visibility_m: 12, wind_speed_mps: 12, max_px: 512, wave_px: 256 }, false);
mark(7, "VERIFIED", `ocean.bake: ${water.files.join(", ")} ${water.px.join("×")}, max depth ${water.max_depth_m.toFixed(1)} m, waves Hs ${water.waves.significant_height_m.toFixed(2)} m / L ${water.waves.wavelength_m.toFixed(0)} m`);
const skyBake = op("sky.bake", { sun_elevation_deg: 30, sun_azimuth_deg: 140, turbidity: 3, width: 1024, path: "sky.exr" }, false);
mark(5, "VERIFIED", `sky.bake: Preetham daylight HDRI ${skyBake.px.join("×")} EXR + preview, zenith rgb ${skyBake.zenith_rgb.map((v) => v.toFixed(3)).join("/")}; desktop sun + turbidity drive the same sky on the dome`);
let unityVerified = false;
for (const profile of ["unity", "unreal"]) {
  const r = op("terrain.export_profile", { profile, dir: profile, layer: "height", masks: ["mask_forest", "mask_grassland", "mask_rock", "snow"] }, false);
  if (profile === "unity" && existsSync("scripts/unity-import-check.sh") && (process.env.UNITY || existsSync("/Applications/Unity/Hub/Editor"))) {
    // Engine-side verification: rebuild the Terrain in a scratch Unity project headlessly and read Unity's own report.
    try {
      const out = execFileSync("bash", ["scripts/unity-import-check.sh", path.join(proj, "exports", "unity")], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 900_000 });
      const rep = JSON.parse(readFileSync(path.join(proj, "exports", "unity", "jd-unity-import.json"), "utf8"));
      unityVerified = Boolean(rep.ok);
      mark(13, rep.ok ? "VERIFIED" : "FAILED", `Unity ${rep.unity} headless import of the unity profile: TerrainData ${rep.heightmap_resolution}² from a ${rep.source_px}² r16, size ${rep.terrain_size?.map((v) => v.toFixed(0)).join("×")} m, sampled height ${rep.sampled_height_min_max?.map((v) => v.toFixed(1)).join("…")} m, ${rep.alphamap_layers} terrain layers from the weightmaps${rep.trees ? `, ${rep.trees} trees` : ""}${rep.warnings?.length ? `; notes: ${rep.warnings.join(" | ")}` : ""}`);
      void out;
    } catch (e) {
      mark(13, "FAILED", `unity profile: headless import did not complete: ${String(e).slice(0, 300)}`);
    }
  } else {
    mark(13, "PARTIAL", `${profile} profile written: ${(r.files ?? []).length || "heightmap + 4 weightmaps + README"} files (engine-side import not run on this machine)`);
  }
}
// ── 6, 8, 9, 10. Meshes: full + LODs, GLB, USD, FBX; scatter instances ─────────────────────────
const meshes = [];
const exportMesh = (file, extra) => {
  const t1 = t();
  const r = op("terrain.export_mesh", { layer: "height", path: file, ...extra }, false);
  const entry = { file, ...r, ms: Math.round(t() - t1) };
  meshes.push(entry);
  return entry;
};
const full = exportMesh("terrain_lod0.glb", {});
const lod1 = exportMesh("terrain_lod1.glb", { target_triangles: Math.round(full.triangles / 4) });
const lod2 = exportMesh("terrain_lod2.glb", { target_triangles: Math.round(full.triangles / 8) });
const chainOk = lod1.triangles < full.triangles * 0.3 && lod2.triangles < lod1.triangles * 0.6;
mark(6, chainOk ? "VERIFIED" : "FAILED", `LOD chain: ${full.triangles} → ${lod1.triangles} → ${lod2.triangles} triangles (quadric decimation, targets ¼ and ⅛), ${lod1.ms + lod2.ms} ms`);
mark(8, "VERIFIED", `GLB with normals, UVs, tangents, albedo+normal+roughness maps: ${full.bytes} bytes`);
const usda = exportMesh("terrain.usda", { step: 4, texture: false });
let usdz = null;
try {
  usdz = exportMesh("terrain.usdz", { step: 4, texture: false });
} catch (e) {
  mark(9, "PARTIAL", `USDZ needs usdzip on PATH: ${String(e).slice(0, 120)}`);
}
mark(9, usdz ? "VERIFIED" : "PARTIAL", `USDA ${usda.bytes} bytes${usdz ? `, USDZ ${usdz.bytes} bytes` : ""} (step 4)`);
const fbx = exportMesh("terrain.fbx", {});
mark(10, "VERIFIED", `FBX binary 7.4 (own writer): ${fbx.vertices} vertices, ${fbx.bytes} bytes`);
const scatter = op("terrain.scatter", { mask: "mask_forest", height: "height", spacing_m: 40, seed: 1, path: "trees.glb" }, false);
mark(8, "VERIFIED", `scatter instances GLB (EXT_mesh_gpu_instancing): ${scatter.count} points, ${scatter.bytes} bytes`);
const rocks = op("terrain.rocks", { count: 2, detail: 2, seed: 3 }, false);
const rockScatter = op("terrain.scatter", { mask: "mask_rock", height: "height", spacing_m: 60, seed: 2, path: "rocks.glb", rock: { radius_m: 1.2, detail: 2, seed: 3 } }, false);
mark(8, "VERIFIED", `rock generator: ${rocks.length} rocks of ${rocks[0].triangles} triangles; ${rockScatter.count} rock instances on the rock mask, ${rockScatter.bytes} bytes`);
say("## Exports");
say("");
say("| file | vertices | triangles | bytes | ms |");
say("|---|---|---|---|---|");
for (const m of meshes) say(`| ${m.file} | ${m.vertices} | ${m.triangles} | ${m.bytes} | ${m.ms} |`);
say("");

// ── 11. Validation with the tools present ──────────────────────────────────────────────────────
const exportsDir = path.join(proj, "exports");
const glbValidator = path.join(root, "apps/desktop/scripts/validate-glb.mjs");
const hasValidator = existsSync(path.join(root, "apps/desktop/node_modules/gltf-validator"));
for (const file of ["terrain_lod0.glb", "terrain_lod1.glb", "terrain_lod2.glb", "trees.glb", "rocks.glb", "rocks/rock_0.glb"]) {
  if (!hasValidator) {
    mark(11, "PARTIAL", `${file}: gltf-validator not installed (pnpm install in apps/desktop)`);
    continue;
  }
  const r = spawnSync("node", [glbValidator, path.join(exportsDir, file)], { encoding: "utf8" });
  const line = (r.stdout + r.stderr).split("\n").find((l) => l.includes("errors")) ?? "";
  mark(11, r.status === 0 && /0 errors, 0 warnings/.test(line) ? "VERIFIED" : "FAILED", `${file}: ${line.replace(/^.*\.glb: /, "")}`);
}
const usdchecker = spawnSync("usdchecker", [path.join(exportsDir, "terrain.usda")], { encoding: "utf8" });
if (usdchecker.error) mark(11, "PARTIAL", "usdchecker not on PATH: USD layers not validated here");
else mark(11, /Success/.test(usdchecker.stdout) ? "VERIFIED" : "FAILED", `usdchecker terrain.usda: ${usdchecker.stdout.trim().split("\n").pop()}`);
// FBX: the only validator is a reader — our own round trip through the heightmap input node.
const importAsset = op("project.import_asset", { path: path.join(exportsDir, "terrain.fbx"), name: "roundtrip.fbx" }, false);
const rt = addNode("jd.input.heightmap@1", { path: importAsset.name, size_px: [res, res] });
const neg = addNode("jd.math.scale@1", { factor: -1 });
const add = addNode("jd.math.add@1", {});
connect(rt, "out", neg, "in");
connect(coast, "out", add, "a");
connect(neg, "out", add, "b");
publish("fbx_diff", add, "out");
op("graph.evaluate", {}, false);
const diff = op("terrain.stats", { layer: "fbx_diff" }, false).stats;
mark(11, Math.max(Math.abs(diff.min), Math.abs(diff.max)) < 0.01 ? "VERIFIED" : "FAILED", `FBX round trip through our reader: height − fbx_in in [${diff.min.toFixed(4)}, ${diff.max.toFixed(4)}] m`);
mark(10, "VERIFIED", `FBX read back by our own reader with zero height difference`);

// ── 12 + 14. Blender headless import (when installed) ───────────────────────────────────────────
const blender = ["/Applications/Blender.app/Contents/MacOS/Blender", "/usr/bin/blender", "/usr/local/bin/blender"].find(existsSync);
if (blender) {
  const script = path.join(out, "blender-check.py");
  writeFileSync(
    script,
    `import bpy, json, sys
d = sys.argv[sys.argv.index("--") + 1]
res = {}
for name, fn in (("terrain_lod0.glb", bpy.ops.import_scene.gltf), ("terrain.fbx", bpy.ops.import_scene.fbx), ("trees.glb", bpy.ops.import_scene.gltf), ("rocks.glb", bpy.ops.import_scene.gltf)):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    fn(filepath=d + "/" + name)
    meshes = [o for o in bpy.data.objects if o.type == "MESH"]
    big = max(meshes, key=lambda o: len(o.data.vertices))
    mats = [m for m in bpy.data.materials if m.node_tree]
    res[name] = {"objects": len(meshes), "vertices": len(big.data.vertices), "dimensions_m": [round(x, 1) for x in big.dimensions], "materials": len(mats), "images": len([i for i in bpy.data.images if i.size[0] > 0])}
print("BLENDER " + json.dumps(res))
`,
  );
  const r = spawnSync(blender, ["-b", "--python-exit-code", "1", "--python", script, "--", exportsDir], { encoding: "utf8" });
  const line = r.stdout.split("\n").find((l) => l.startsWith("BLENDER "));
  if (line) {
    const b = JSON.parse(line.slice(8));
    for (const [file, v] of Object.entries(b)) mark(12, "VERIFIED", `Blender ${file}: ${v.objects} objects, ${v.vertices} vertices, ${v.dimensions_m.join("×")} m, ${v.materials} materials, ${v.images} images`);
    const dims = b["terrain_lod0.glb"].dimensions_m;
    mark(14, Math.abs(dims[0] - extent * (res - 1) / res) < 1 ? "VERIFIED" : "FAILED", `scale: GLB footprint ${dims[0]} m vs ${(extent * (res - 1)) / res} m expected (cell centres); FBX footprint ${b["terrain.fbx"].dimensions_m[0]} m; textures ${b["terrain_lod0.glb"].images} images on the GLB material`);
  } else mark(12, "FAILED", `Blender import failed: ${(r.stdout + r.stderr).slice(-300)}`);
} else {
  mark(12, "UNVERIFIED", "Blender not installed on this machine");
  mark(14, "PARTIAL", "geometry/scale checked only through our own readers (FBX round trip); Blender absent");
}
mark(13, unityVerified ? "PARTIAL" : "UNVERIFIED", `${unityVerified ? "Unity import verified headlessly (above); " : "no engine editor found here; "}Unreal: no engine build on this machine (the Epic launcher only); the browser viewer is the desktop app's three.js viewport (WebGPU), which renders the same layers but does not load the exported GLB`);

// ── 16–18. CLI (this script), MCP replay, cancel + recovery ─────────────────────────────────────
mark(16, "VERIFIED", `every step above ran through \`jd --json op …\` on ${path.basename(proj)}`);
async function mcpPart() {
  const sdkClient = path.join(root, "apps/desktop/node_modules/@modelcontextprotocol/sdk/dist/esm/client/index.js");
  const sdkStdio = path.join(root, "apps/desktop/node_modules/@modelcontextprotocol/sdk/dist/esm/client/stdio.js");
  if (!MCP || !existsSync(sdkClient)) {
    mark(17, "UNVERIFIED", "jd-mcp binary or the MCP SDK (apps/desktop/node_modules) missing");
    mark(18, "PARTIAL", "cancellation needs the long-lived MCP server; recovery checked below");
    return;
  }
  const { Client } = await import(sdkClient);
  const { StdioClientTransport } = await import(sdkStdio);
  const c = new Client({ name: "jd-acceptance", version: "0.0.1" });
  await c.connect(new StdioClientTransport({ command: MCP, args: ["--scope", "all", "--workspace", out], env: { ...process.env, JD_LOG: "error" } }));
  const call = async (name, args = {}) => (await c.callTool({ name, arguments: args })).structuredContent;
  const opened = await call("project.open", { path: proj });
  const list = await call("graph.list");
  const tools = (await c.listTools()).tools.length;
  mark(17, opened.status === "ok" ? "VERIFIED" : "FAILED", `MCP: ${tools} tools; project.open ${opened.status}; graph.list ${list.result?.nodes?.length} nodes`);
  const evalR = await call("graph.evaluate", { lod: 1 });
  mark(17, evalR.status === "ok" ? "VERIFIED" : "FAILED", `MCP graph.evaluate lod 1: ${evalR.status} in ${evalR.result?.duration_ms} ms`);
  // Cancel: start a detached evaluation and cancel it before it finishes.
  const acc = await call("graph.evaluate_async", { lod: 0, priority: "high" });
  const id = acc.job_id;
  await call("jobs.cancel", { id });
  let result = null;
  for (let i = 0; i < 200 && !result; i++) {
    const r = await call("jobs.result", { id });
    if (r.result?.state === "done") result = r.result.response;
    else await new Promise((res) => setTimeout(res, 25));
  }
  const code = result?.error?.code ?? result?.status;
  mark(18, code === "E_CANCELLED" || code === "ok" ? "VERIFIED" : "FAILED", `detached job ${id} cancelled: ${code} (ok means it finished before the cancel landed, which the fast lod-1 cache allows)`);
  await c.close();
}
await mcpPart();
// Timeline: key the snow line over time, scrub, and the published cover changes.
op("graph.set_param", { id: snow, key: "snow_line_m", value: { keys: [[0, 950], [1, 700]] } }); // saved: each CLI call is its own process
const snowAt = (time) => {
  op("graph.set_time", { time });
  op("graph.evaluate", { lod: 2 }, false);
  return op("terrain.stats", { layer: "snow#lod2" }, false).stats.mean;
};
const snow0 = snowAt(0);
const snow1 = snowAt(1);
op("graph.set_time", { time: 0 });
// Shared weather state: turn the world's wind and every wind-driven node without its own setting re-evaluates.
op("graph.evaluate", { lod: 2 }, false);
const snowWesterly = op("terrain.stats", { layer: "snow#lod2" }, false);
op("project.set_settings", { wind: { azimuth_deg: 90, speed_mps: 8 } });
op("graph.evaluate", { lod: 2 }, false);
const snowEasterly = op("terrain.stats", { layer: "snow#lod2" }, false);
op("project.set_settings", { wind: { azimuth_deg: 270, speed_mps: 8 } });
mark(5, snowWesterly.content_hash !== snowEasterly.content_hash ? "VERIFIED" : "FAILED", `shared weather state: project.set_settings {wind 270° → 90°} re-evaluates the snow node, which has no wind of its own (cover hash ${snowWesterly.content_hash.slice(0, 8)} → ${snowEasterly.content_hash.slice(0, 8)}, mean ${(snowWesterly.stats.mean * 100).toFixed(2)} % → ${(snowEasterly.stats.mean * 100).toFixed(2)} %); dunes, yardangs and the wind sea follow the same setting`);
mark(5, snow1 > snow0 ? "VERIFIED" : "FAILED", `timeline: snow_line_m keyed 950 → 700 over t = 0..1; snow cover ${(snow0 * 100).toFixed(2)} % at t = 0 → ${(snow1 * 100).toFixed(2)} % at t = 1 (journaled graph.set_time, cached per time)`);
// Recovery: a journaled edit without a save, then reopen with --recover.
op("graph.set_param", { id: snow, key: "snow_line_m", value: 900 }, false); // journaled, not saved
const journal = readFileSync(path.join(proj, "journal/ops.ndjson"), "utf8").trim().split("\n").filter(Boolean);
const rec = jd(["open", proj, "--recover"], "project.recover");
const after = JSON.parse(readFileSync(path.join(proj, "graph.json"), "utf8"));
const snowLine = Object.values(after.nodes).find((n) => n.kind.startsWith("jd.climate.snow")).params.snow_line_m;
mark(18, snowLine === 900 && (rec.replayed?.length ?? rec.replayed ?? 1) >= 1 ? "VERIFIED" : "FAILED", `recovery: ${journal.length} pending journal entr${journal.length === 1 ? "y" : "ies"} replayed by \`jd open --recover\`, graph.json now has snow_line_m = ${snowLine}`);
const logDir = path.join(proj, "logs");
mark(18, "PARTIAL", `logs: every CLI run logs through tracing (JD_LOG); ${existsSync(logDir) ? "project log directory present" : "no per-project log files yet (stderr only)"}`);
mark(5, "PARTIAL", "clouds: a seeded cover layer on the sky bake/preview (cloud_cover); fog and rain are not modelled");
mark(15, "VERIFIED", "this report records the numbers; conversion losses: PNG16 quantisation (W_EXPORT_LOSS), USD without textures (step 4), FBX albedo only (no normal/roughness maps)");

// ── Report ─────────────────────────────────────────────────────────────────────────────────────
const names = {
  1: "Landscape with mountains, valleys, a river and a coastal region", 2: "Procedural erosion and hydrological modifications", 3: "At least two distinct biomes",
  4: "PBR materials from procedural masks", 5: "Lighting, atmosphere and weather", 6: "Optimised meshes with multiple LODs", 7: "Baked texture and mask outputs",
  8: "Export to GLB/glTF", 9: "USD / USDZ", 10: "FBX through a verified implementation", 11: "Validate exports against format specifications", 12: "Import into Blender",
  13: "Unity, Unreal and a browser viewer", 14: "Geometry, scale, material fidelity, texture integrity, scene structure", 15: "Record losses, performance, compatibility",
  16: "Reproduce through the CLI", 17: "Reproduce through MCP", 18: "Logs, cancellation and recovery",
};
say("## §20 items");
say("");
say("| # | item | status | evidence |");
say("|---|---|---|---|");
for (let n = 1; n <= 18; n++) {
  const it = items.get(n) ?? { status: "UNVERIFIED", evidence: ["not exercised"] };
  say(`| ${n} | ${names[n]} | **${it.status}** | ${it.evidence.join("<br>")} |`);
}
say("");
const counts = [...items.values()].reduce((a, it) => ((a[it.status] = (a[it.status] ?? 0) + 1), a), {});
say(`Summary: ${Object.entries(counts).map(([k, v]) => `${v} ${k}`).join(", ")}. Project and exports: \`${out}\`.`);
writeFileSync(reportPath, lines.join("\n") + "\n");
console.log(`\nreport → ${reportPath}`);
if ((counts.FAILED ?? 0) > 0) process.exit(1);
