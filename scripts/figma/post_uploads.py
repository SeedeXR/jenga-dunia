"""POST each image to its Figma upload URL and check it landed on the intended frame (see README)."""
import json, subprocess, sys, concurrent.futures as cf
D, page, pid = sys.argv[1], sys.argv[2], sys.argv[3]
rows = {r["node"]: r for r in json.load(open(f"{D}/upload_{page}.json"))}
pairs = [l.split() for l in open(f"{D}/urls_{page}.txt") if l.strip()]
assert len(pairs) == len(rows) and all(n in rows for n, _ in pairs), "url/node mismatch"
def post(pair):
    node, uuid = pair; r = rows[node]
    url = f"https://mcp.figma.com/mcp/upload/{uuid}/submit?scaleMode=FILL&currentPageId={pid.replace(':','%3A')}"
    out = subprocess.run(["curl", "-sS", "-m", "180", "-X", "POST", "-F", f"file=@{r['src']};type=image/png;filename={r['file']}", url], capture_output=True, text=True)
    try:
        j = json.loads(out.stdout); ok = j.get("success") and j.get("placedOnNodeId") == node
    except Exception:
        ok = False; j = out.stdout[:200] + out.stderr[:200]
    return node, r["file"], ok, j
with cf.ThreadPoolExecutor(6) as ex:
    res = list(ex.map(post, pairs))
bad = [(n, f, j) for n, f, ok, j in res if not ok]
print(f"{page}: {len(res) - len(bad)}/{len(res)} placed")
for b in bad: print("FAILED", b)
