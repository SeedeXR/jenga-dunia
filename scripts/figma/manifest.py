"""Every user-guide figure with its guide, heading, alt text, size and annotation spec (see README)."""
import os, re, json, struct, sys
OUT=sys.argv[1]
def png_size(p):
    with open(p,"rb") as f: h=f.read(24)
    return struct.unpack(">II", h[16:24])
docs = {"desktop-app.md":"Desktop app","cli-quickstart.md":"CLI quickstart","blender.md":"Other"}
figs=[]; seen=set()
def entry(doc, heading, im, alt):
    p="docs/user/images/"+im; w,h=png_size(p)
    annot=p[:-4]+".annot.json"; a=json.load(open(annot)) if os.path.exists(annot) else None
    raw=a.get("src") if a and a.get("src") and os.path.exists(a["src"]) else None
    return {"doc":doc,"heading":heading,"file":im,"alt":alt,"w":w,"h":h,"raw":raw,"annotations":a.get("annotations") if (a and raw) else None,"flattened":bool(a and not raw)}
for d,title in docs.items():
    heading="Introduction" if title!="Other" else "Blender bridge"
    for line in open("docs/user/"+d, encoding="utf-8"):
        m=re.match(r"^(#{2,3}) (.*)", line)
        if m and title!="Other": heading=m.group(2).strip().replace("`","")
        for alt, im in re.findall(r"!\[([^\]]*)\]\(images/([A-Za-z0-9_.\-]+\.png)\)", line):
            if im in seen: continue
            seen.add(im); figs.append(entry(title, heading, im, alt))
for im in sorted(os.listdir("docs/user/images")):
    if im.endswith(".png") and im not in seen: figs.append(entry("Other","Not embedded in a guide",im,""))
json.dump(figs, open(OUT,"w"), indent=1)
from collections import Counter
print(len(figs), Counter(x["doc"] for x in figs), "editable:", sum(1 for x in figs if x["annotations"]))
