"""One Figma Plugin API script per page from the manifest (see README); ids are those of file fxyeV62hgLoNmG3dU1LX6G."""
import json, sys
D=sys.argv[1]
figs=json.load(open(D+"/manifest.json"))
PAGES={"Desktop app":("12:2","docs/user/desktop-app.md"),"CLI quickstart":("12:3","docs/user/cli-quickstart.md"),"Other":("12:4","docs/user/blender.md and images not embedded in a guide")}
JS=r'''
const PAGE_ID = @@page@@, PAGE_TITLE = @@title@@, PAGE_SRC = @@src@@;
const BADGE_ID = "12:21", LABEL = "Label#12:0";
const PS = {red:"S:16412078a941835fdd1ace04fc21fe5d18df126e,", outline:"S:da04ff15b38502735303a951a6b9c39db8991fa7,", card:"S:480a4f949f079fd4327cfef423270c544aa9c420,", border:"S:eca14b990b4cf64d7c0ff0eb9072434be4313952,", text:"S:de94f6508a4c244c4e6755d2e92881710cbe943c,", muted:"S:7cbf9c13b5d5f205562b1ee52907ee46cf216afd,", placeholder:"S:1321d0e6d19bc48da3f0d3ee936fc022132f1b12,"};
const TS = {title:"S:a109ba3e8f41b2dce0480e145c58d3f7e0cd7dbd,", body:"S:76d3c150fa397fb1683b45ee0bb004cf0ec0e1c4,", file:"S:8ba67c3a87e12919c56a282078a4cf51d7af9c4a,", caption:"S:8f83b902e081e8cd36db136c0b101ae0e32343fd,", meta:"S:d219271202dd1a65090323c73d52871042cbb91d,", ann:"S:a6f8a4d84cb2bc7435c4ae8ae6b782ae19d03da6,"};
const DATA = @@data@@;
await Promise.all(["Regular","Medium","Semi Bold","Bold"].map(s => figma.loadFontAsync({family:"Inter", style:s})).concat([figma.loadFontAsync({family:"JetBrains Mono", style:"Medium"})]));
const page = await figma.getNodeByIdAsync(PAGE_ID);
await figma.setCurrentPageAsync(page);
const badge = await figma.getNodeByIdAsync(BADGE_ID);
const out = { pageId: PAGE_ID, sections: [], images: {}, cards: 0, annotations: 0, errors: [] };
function text(content, style, fill, width) {
  const t = figma.createText();
  t.textStyleId = style; t.fillStyleId = fill; t.characters = content;
  if (width) { t.resize(width, 20); t.textAutoResize = "HEIGHT"; } else t.textAutoResize = "WIDTH_AND_HEIGHT";
  return t;
}
async function annotate(layer, anns, s) {
  for (const a of anns) {
    let capX, capY;
    if (a.type === "circle") {
      const e = figma.createEllipse(); e.name = `Circle ${a.label ?? ""}`.trim();
      e.resize(2 * a.r * s, 2 * a.r * s); e.x = (a.cx - a.r) * s; e.y = (a.cy - a.r) * s;
      e.fills = []; e.strokeStyleId = PS.red; e.strokeWeight = 4 * s; layer.appendChild(e);
      if (a.label) { const b = badge.createInstance(); b.setProperties({ [LABEL]: String(a.label) }); if (s !== 1) b.rescale(s); b.x = (a.cx - a.r - 18 - 16) * s; b.y = (a.cy - a.r - 18 - 16) * s; b.name = `Badge ${a.label}`; layer.appendChild(b); }
      capX = a.tx ?? (a.cx - a.r); capY = a.ty ?? (a.cy + a.r + 28);
    } else if (a.type === "arrow") {
      const v = figma.createVector(); v.name = "Arrow";
      const ox = Math.min(a.x1, a.x2), oy = Math.min(a.y1, a.y2);
      await v.setVectorNetworkAsync({ vertices: [{ x: (a.x1 - ox) * s, y: (a.y1 - oy) * s }, { x: (a.x2 - ox) * s, y: (a.y2 - oy) * s, strokeCap: "ARROW_EQUILATERAL" }], segments: [{ start: 0, end: 1 }], regions: [] });
      v.strokeStyleId = PS.red; v.strokeWeight = 4 * s; v.x = ox * s; v.y = oy * s; layer.appendChild(v);
      capX = a.tx ?? a.x1; capY = a.ty ?? (a.y1 - 12);
    } else { out.errors.push(`unsupported ${a.type}`); continue; }
    if (a.text) {
      const t = text(a.text, TS.ann, PS.red); t.name = "Caption";
      if (s !== 1) t.fontSize = 20 * s;
      t.strokeStyleId = PS.outline; t.strokeWeight = 2 * s; t.strokeAlign = "OUTSIDE";
      layer.appendChild(t);
      const anchor = a.anchor ?? "start";
      t.x = capX * s - (anchor === "middle" ? t.width / 2 : anchor === "end" ? t.width : 0);
      t.y = capY * s - 19 * s; // SVG y is the baseline; Inter's ascent at 20 px is about 19 px
    }
    out.annotations++;
  }
}
async function card(f) {
  const s = f.w <= 512 ? 2 : f.w > 1600 ? 1600 / f.w : 1;
  const dw = Math.round(f.w * s), dh = Math.round(f.h * s);
  const c = figma.createAutoLayout("VERTICAL", { name: `Figure / ${f.file}`, itemSpacing: 12 });
  c.paddingTop = c.paddingBottom = c.paddingLeft = c.paddingRight = 24;
  c.fillStyleId = PS.card; c.strokeStyleId = PS.border; c.strokeWeight = 1; c.cornerRadius = 12;
  const title = text(f.file, TS.file, PS.text); title.name = "File"; c.appendChild(title);
  const img = figma.createFrame(); img.name = "Image"; img.resize(dw, dh); img.fillStyleId = PS.placeholder; img.clipsContent = true; c.appendChild(img);
  if (f.n) { const layer = figma.createFrame(); layer.name = "Annotations"; layer.fills = []; layer.resize(dw, dh); layer.clipsContent = false; img.appendChild(layer); await annotate(layer, f.n, s); }
  if (f.a) { const cap = text(f.a, TS.caption, PS.text, dw); cap.name = "Alt text"; c.appendChild(cap); }
  const status = f.n ? `${f.n.length} editable annotations over the unannotated capture` : f.fl ? "annotations are baked into this PNG (its unannotated source is gone)" : "no annotation spec; any markings are part of the PNG";
  const meta = text(`${f.w}×${f.h} px${s !== 1 ? ` · shown at ${s === 2 ? "2×" : Math.round(s * 100) + " %"}` : ""} · docs/user/images/${f.file} · ${status}`, TS.meta, PS.muted, dw); meta.name = "Meta"; c.appendChild(meta);
  out.images[f.file] = img.id; out.cards++;
  return c;
}
const head = figma.createAutoLayout("VERTICAL", { name: `Header / ${PAGE_TITLE}`, itemSpacing: 12 });
head.fills = [];
head.appendChild(text(PAGE_TITLE, TS.title, PS.text));
head.appendChild(text(`${DATA.length} figures from ${PAGE_SRC}, grouped by the guide's headings in reading order. Red layers are editable where the unannotated capture still exists.`, TS.body, PS.muted, 1400));
head.x = 0; head.y = 0;
let y = head.height + 120;
const groups = [];
for (const f of DATA) { const g = groups[groups.length - 1]; if (g && g.h === f.h_) g.items.push(f); else groups.push({ h: f.h_, items: [f] }); }
for (const g of groups) {
  const sec = figma.createSection(); sec.name = g.h;
  let cx = 80, cy = 80, rowH = 0, maxX = 0;
  for (const f of g.items) {
    const c = await card(f); sec.appendChild(c);
    if (cx > 80 && cx + c.width > 80 + 3400) { cx = 80; cy += rowH + 80; rowH = 0; }
    c.x = cx; c.y = cy; cx += c.width + 80; rowH = Math.max(rowH, c.height); maxX = Math.max(maxX, cx);
  }
  sec.resizeWithoutConstraints(maxX, cy + rowH + 80);
  sec.x = 0; sec.y = y; y += sec.height + 160;
  out.sections.push({ id: sec.id, name: g.h, figures: g.items.length });
}
out.headerId = head.id;
return out;
'''
for doc,(pid,src) in PAGES.items():
    data=[{"file":x["file"],"h_":x["heading"],"a":x["alt"],"w":x["w"],"h":x["h"],"n":x["annotations"],"fl":x["flattened"]} for x in figs if x["doc"]==doc]
    js=JS.replace("@@page@@",json.dumps(pid)).replace("@@title@@",json.dumps(doc)).replace("@@src@@",json.dumps(src)).replace("@@data@@",json.dumps(data,ensure_ascii=False,separators=(",",":")))
    name={"Desktop app":"desktop","CLI quickstart":"cli","Other":"other"}[doc]
    open(f"{D}/page_{name}.js","w").write(js)
    print(name, len(js), "chars")
