// Validates a .glb with the Khronos glTF Validator (npm `gltf-validator`): exit 1 on any error.
// Usage: node scripts/validate-glb.mjs <file.glb>
import { readFileSync } from "node:fs";
import { validateBytes } from "gltf-validator";

const file = process.argv[2];
if (!file) {
  console.error("usage: node scripts/validate-glb.mjs <file.glb>");
  process.exit(2);
}
const report = await validateBytes(new Uint8Array(readFileSync(file)));
const { numErrors, numWarnings, numInfos, numHints } = report.issues;
for (const m of report.issues.messages) console.log(`${m.severity === 0 ? "ERROR" : m.severity === 1 ? "WARN" : "info"} ${m.code}: ${m.message} (${m.pointer ?? ""})`);
console.log(`${file}: ${numErrors} errors, ${numWarnings} warnings, ${numInfos} infos, ${numHints} hints; ${report.info?.totalTriangleCount ?? "?"} triangles, ${report.info?.totalVertexCount ?? "?"} vertices`);
process.exit(numErrors > 0 ? 1 : 0);
