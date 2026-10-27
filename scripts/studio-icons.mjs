#!/usr/bin/env node
// Writes the lucide icons Studio draws (the `icons!` list in apps/studio/src/look.rs) as SVG files
// in apps/studio/assets/icons/, rendered by the desktop's own lucide-react so both windows draw the
// same glyphs.
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

// Studio's icon list (`icons!` in apps/studio/src/look.rs) is the source: every kebab-case name
// that is a lucide icon is rendered; Studio's own (horizon) are kept as drawn.
const listed = readFileSync(join(root, "apps/studio/src/look.rs"), "utf8").match(/icons!\(([\s\S]*?)\);/);
if (!listed) throw new Error("no icons!(…) list in apps/studio/src/look.rs");
const pascal = (k) => k.replace(/(^|-)([a-z0-9])/g, (_, __, c) => c.toUpperCase());
const own = new Set(["horizon"]);
const names = [...listed[1].matchAll(/"([a-z0-9-]+)"/g)].map((m) => m[1]).filter((k) => !own.has(k)).map(pascal);
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
writeFileSync(join(out, "README.md"), `# Icons\n\nLucide ${version} (ISC licence, https://lucide.dev/license), rendered from the desktop's \`lucide-react\` by \`scripts/studio-icons.mjs\` — the ${names.length} lucide icons in Studio's \`icons!\` list (apps/studio/src/look.rs), plus Studio's own horizon. Regenerate after changing that list.\n`);
console.log(`wrote ${names.length} icons (lucide ${version}) to apps/studio/assets/icons/`);
