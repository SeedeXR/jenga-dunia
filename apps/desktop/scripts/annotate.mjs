// Renders tutorial annotations (red boxes, circles, arrows, numbered labels) over a screenshot.
// Deterministic and CI-friendly: an HTML/SVG overlay rasterised with the local Chrome via Playwright.
// Usage: node scripts/annotate.mjs <spec.json> [...]   (spec: { src, out, width?, annotations: [...] })
// Annotation shapes (coordinates in source-image pixels):
//   { "type": "box",    "x", "y", "w", "h", "label": "1", "text": "...", "lx"?, "ly"? }  (lx/ly: the badge's centre)
//   { "type": "circle", "cx", "cy", "r", "label": "2", "text": "..." }
//   { "type": "arrow",  "x1", "y1", "x2", "y2", "text": "..." }
import { readFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { chromium } from "@playwright/test";

const RED = "#ff3b30";

function svgFor(spec, w, h) {
  const parts = [];
  const labels = [];
  for (const a of spec.annotations) {
    // Optional explicit caption placement: tx/ty in image pixels, anchor start|middle|end.
    const cap = (dx, dy) => (a.text ? text(a.tx ?? dx, a.ty ?? dy, a.text, a.anchor ?? "start") : "");
    if (a.type === "box") {
      parts.push(`<rect x="${a.x}" y="${a.y}" width="${a.w}" height="${a.h}" rx="6" fill="none" stroke="${RED}" stroke-width="4"/>`);
      if (a.label) labels.push({ x: a.lx ?? a.x - 18, y: a.ly ?? a.y - 18, label: a.label });
      parts.push(cap(a.x, a.y + a.h + 28));
    } else if (a.type === "circle") {
      parts.push(`<circle cx="${a.cx}" cy="${a.cy}" r="${a.r}" fill="none" stroke="${RED}" stroke-width="4"/>`);
      if (a.label) labels.push({ x: a.cx - a.r - 18, y: a.cy - a.r - 18, label: a.label });
      parts.push(cap(a.cx - a.r, a.cy + a.r + 28));
    } else if (a.type === "arrow") {
      parts.push(`<line x1="${a.x1}" y1="${a.y1}" x2="${a.x2}" y2="${a.y2}" stroke="${RED}" stroke-width="4" marker-end="url(#head)"/>`);
      parts.push(cap(a.x1, a.y1 - 12));
    }
  }
  for (const l of labels) {
    parts.push(`<circle cx="${l.x}" cy="${l.y}" r="16" fill="${RED}"/><text x="${l.x}" y="${l.y + 6}" text-anchor="middle" font-size="18" font-weight="700" fill="#fff" font-family="Helvetica, Arial, sans-serif">${l.label}</text>`);
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="100%" viewBox="0 0 ${w} ${h}" style="position:absolute;left:0;top:0">
    <defs><marker id="head" markerWidth="10" markerHeight="10" refX="8" refY="5" orient="auto"><path d="M0,0 L10,5 L0,10 z" fill="${RED}"/></marker></defs>
    ${parts.join("\n")}</svg>`;
}

function text(x, y, s, anchor = "start") {
  const esc = s.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  return `<text x="${x}" y="${y}" text-anchor="${anchor}" font-size="20" font-weight="600" fill="${RED}" font-family="Helvetica, Arial, sans-serif" paint-order="stroke" stroke="#000" stroke-width="4" stroke-linejoin="round">${esc}</text>`;
}

async function run(specPath) {
  const spec = JSON.parse(readFileSync(specPath, "utf8"));
  const base = dirname(resolve(specPath));
  const src = resolve(base, spec.src);
  const out = resolve(base, spec.out);
  const png = readFileSync(src);
  const dataUri = `data:image/png;base64,${png.toString("base64")}`;
  const browser = await chromium.launch({ channel: process.env.CI ? undefined : "chrome" });
  const probe = await browser.newPage();
  await probe.setContent(`<img id="i" src="${dataUri}">`);
  const { w, h } = await probe.evaluate(() => {
    const i = document.getElementById("i");
    return { w: i.naturalWidth, h: i.naturalHeight };
  });
  await probe.close();
  // Headless Chrome on macOS wraps content taller than the physical screen (rows ≥ ~815 showed the
  // page top again), so render at half CSS size with deviceScaleFactor 2: same output pixels,
  // half the window.
  const dsf = h > 800 ? 2 : 1;
  const page = await browser.newPage({ deviceScaleFactor: dsf, viewport: { width: w / dsf, height: h / dsf } });
  await page.setContent(
    `<html><body style="margin:0"><div style="position:relative;width:${w / dsf}px;height:${h / dsf}px"><img src="${dataUri}" style="position:absolute;left:0;top:0;width:${w / dsf}px;height:${h / dsf}px">${svgFor(spec, w, h)}</div></body></html>`,
  );
  mkdirSync(dirname(out), { recursive: true });
  await page.screenshot({ path: out, clip: { x: 0, y: 0, width: w / dsf, height: h / dsf }, scale: "device" });
  await browser.close();
  console.log(`annotated ${spec.src} → ${spec.out} (${spec.annotations.length} annotations)`);
}

for (const p of process.argv.slice(2)) await run(p);
