// Checks that relative Markdown links point at existing files. External links are not fetched.
// Usage: node scripts/check-links.mjs [paths...]   (default: docs/ memory/ README.md CONTRIBUTING.md)
import { readdirSync, readFileSync, statSync, existsSync } from "node:fs";
import { join, extname, dirname, resolve } from "node:path";

const roots = process.argv.slice(2).length ? process.argv.slice(2) : ["docs", "memory", "README.md", "CONTRIBUTING.md"];

function* mdFiles(p) {
  const st = statSync(p);
  if (st.isDirectory()) for (const e of readdirSync(p)) yield* mdFiles(join(p, e));
  else if (extname(p) === ".md") yield p;
}

let checked = 0, broken = 0;
for (const root of roots) {
  for (const f of mdFiles(root)) {
    const src = readFileSync(f, "utf8").replace(/```[\s\S]*?```/g, "");
    const re = /\[[^\]]*\]\(([^)\s]+)\)/g;
    let m;
    while ((m = re.exec(src))) {
      const target = m[1];
      if (/^(https?:|mailto:|#)/.test(target)) continue;
      checked++;
      const path = resolve(dirname(f), target.split("#")[0]);
      if (!existsSync(path)) { broken++; console.error(`BROKEN ${f} -> ${target}`); }
    }
  }
}
console.log(`${checked - broken}/${checked} relative links resolve`);
process.exit(broken ? 1 : 0);
