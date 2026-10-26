#!/usr/bin/env node
// Writes the lucide icons the desktop UI imports (apps/desktop/src/App.tsx) as SVG files for
// Studio (apps/studio/assets/icons/), rendered by the desktop's own lucide-react, so the egui
// window draws exactly the same glyphs (ADR-0017, owner rule: no visible UI change).
// Usage: node scripts/studio-icons.mjs
import { createRequire } from "node:module";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(join(root, "apps/desktop/package.json"));
const React = require("react");
const { renderToStaticMarkup } = require("react-dom/server");
const lucide = require("lucide-react");

const app = readFileSync(join(root, "apps/desktop/src/App.tsx"), "utf8");
const m = app.match(/import\s*\{([^}]*)\}\s*from\s*"lucide-react"/);
if (!m) throw new Error("App.tsx imports nothing from lucide-react");
const names = m[1].split(",").map((s) => s.trim().split(/\s+as\s+/)[0]).filter(Boolean);
const out = join(root, "apps/studio/assets/icons");
mkdirSync(out, { recursive: true });
const kebab = (n) => n.replace(/([a-z0-9])([A-Z])/g, "$1-$2").replace(/([A-Za-z])([0-9])/g, "$1-$2").toLowerCase();
for (const n of names) {
  const Icon = lucide[n];
  if (!Icon) throw new Error(`lucide-react has no ${n}`);
  // White strokes: Studio tints them to the text colour, as `currentColor` does in the webview.
  const svg = renderToStaticMarkup(React.createElement(Icon, { size: 24, color: "#ffffff" }));
  writeFileSync(join(out, `${kebab(n)}.svg`), `${svg}\n`);
}
const version = require("lucide-react/package.json").version;
writeFileSync(join(out, "README.md"), `# Icons\n\nLucide ${version} (ISC licence, https://lucide.dev/license), rendered from the desktop's \`lucide-react\` by \`scripts/studio-icons.mjs\` — the ${names.length} icons \`apps/desktop/src/App.tsx\` imports. Regenerate after changing those imports.\n`);
console.log(`wrote ${names.length} icons (lucide ${version}) to apps/studio/assets/icons/`);
