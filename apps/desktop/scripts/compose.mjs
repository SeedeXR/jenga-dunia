// Lays images side by side (each scaled to the same height, captioned) into one PNG — the raw
// figure annotate.mjs then draws over. Usage: node scripts/compose.mjs <out.png> <height_px> <img>[=caption] ...
// ponytail: an HTML page screenshotted with the local Chromium, no image library.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { chromium } from "@playwright/test";

const [out, heightArg, ...items] = process.argv.slice(2);
if (!out || !items.length) {
  console.error("usage: compose.mjs <out.png> <height_px> <img>[=caption] ...");
  process.exit(2);
}
const height = Number(heightArg) || 256;
const cells = items.map((it) => {
  const eq = it.indexOf("=");
  const [file, caption] = eq < 0 ? [it, ""] : [it.slice(0, eq), it.slice(eq + 1)];
  const data = `data:image/png;base64,${readFileSync(resolve(file)).toString("base64")}`;
  return `<figure><img src="${data}" style="height:${height}px;image-rendering:pixelated"><figcaption>${caption}</figcaption></figure>`;
});
const html = `<!doctype html><body style="margin:0;background:#111;color:#ddd;font:14px Helvetica,Arial,sans-serif">
<div id="row" style="display:inline-flex;gap:12px;padding:12px 12px 84px">${cells.join("")}</div>
<style>figure{margin:0;display:flex;flex-direction:column;gap:6px;max-width:${height}px}img{display:block}figcaption{white-space:normal}</style></body>`;
const browser = await chromium.launch();
const page = await browser.newPage();
await page.setContent(html);
await page.locator("#row").screenshot({ path: resolve(out) });
await browser.close();
console.log(`wrote ${out}`);
