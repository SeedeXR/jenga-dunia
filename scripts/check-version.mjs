// Verifies that every manifest carries the version from the repo's VERSION file.
// Usage: node scripts/check-version.mjs   (exit 1 on drift; CI runs it in the docs job)
import { readFileSync } from "node:fs";

const version = readFileSync(new URL("../VERSION", import.meta.url), "utf8").trim();
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`VERSION is not semver: ${JSON.stringify(version)}`);
  process.exit(1);
}
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8");
const checks = [
  ["Cargo.toml", /\[workspace\.package\][^[]*?\nversion = "([^"]+)"/],
  ["apps/desktop/package.json", (s) => JSON.parse(s).version],
  ["apps/desktop/src-tauri/tauri.conf.json", (s) => JSON.parse(s).version],
  ["integrations/blender/jenga_dunia_bridge/blender_manifest.toml", /\nversion = "([^"]+)"/],
];
let bad = 0;
for (const [file, get] of checks) {
  const s = read(file);
  const found = typeof get === "function" ? get(s) : s.match(get)?.[1];
  if (found !== version) {
    console.error(`${file}: ${found ?? "(not found)"} ≠ VERSION ${version}`);
    bad++;
  }
}
if (bad) process.exit(1);
console.log(`version ${version} consistent across ${checks.length} manifests`);
