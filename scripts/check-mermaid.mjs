// Validates every ```mermaid block in the repo's Markdown by rendering it with mermaid-cli.
// Usage: node scripts/check-mermaid.mjs [paths...]   (default: docs/ memory/ README.md)
// ponytail: shells out to `npx mmdc` (needs Chromium download on first run); a pure-JS parser
// for all diagram types does not exist yet — switch when @mermaid-js/parser covers flowcharts.
import { readdirSync, readFileSync, statSync, mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { join, extname, resolve } from "node:path";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";

const roots = process.argv.slice(2).length ? process.argv.slice(2) : ["docs", "memory", "README.md"];

function* mdFiles(p) {
  const st = statSync(p);
  if (st.isDirectory()) for (const e of readdirSync(p)) yield* mdFiles(join(p, e));
  else if (extname(p) === ".md") yield p;
}

const blocks = [];
for (const root of roots) {
  for (const f of mdFiles(root)) {
    const src = readFileSync(f, "utf8");
    const re = /```mermaid\s*\n([\s\S]*?)```/g;
    let m, i = 0;
    while ((m = re.exec(src))) blocks.push({ file: f, index: ++i, code: m[1] });
  }
}
if (!blocks.length) { console.log("no mermaid blocks found"); process.exit(0); }

const dir = mkdtempSync(join(tmpdir(), "jd-mermaid-"));
const puppeteerCfg = join(dir, "puppeteer.json");
writeFileSync(puppeteerCfg, JSON.stringify({ args: ["--no-sandbox", "--disable-setuid-sandbox"] }));

let failed = 0;
for (const b of blocks) {
  const inFile = join(dir, `${b.file.replace(/[\\/]/g, "_")}.${b.index}.mmd`);
  writeFileSync(inFile, b.code);
  const r = spawnSync(
    process.platform === "win32" ? "npx.cmd" : "npx",
    ["--yes", "-p", "@mermaid-js/mermaid-cli", "mmdc", "-q", "-p", puppeteerCfg, "-i", inFile, "-o", inFile + ".svg"],
    { encoding: "utf8", shell: process.platform === "win32" }
  );
  if (r.status !== 0) {
    failed++;
    console.error(`FAIL ${b.file} (block ${b.index})\n${(r.stderr || r.stdout).trim()}\n`);
  }
}
rmSync(dir, { recursive: true, force: true });
console.log(`${blocks.length - failed}/${blocks.length} mermaid blocks valid`);
process.exit(failed ? 1 : 0);
