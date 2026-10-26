"""Build a demo project with one cave through the `jd` CLI and export terrain + cave meshes.

    python3 scripts/caves/demo.py <workdir> <kind> ['{"reach_m": 900, "entrances": [[1000, 700]], "_world": {"extent": 2048, "res": 256}}']

Writes <workdir>/C.jdproj, terrain.glb (opened over the mouths) and cave.glb / cave.fbx, and prints
timings and the cave's entrances. Render with scripts/caves/render.py.
"""
import json, os, shutil, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
JD = os.environ.get("JD_BIN") or f"{ROOT}/target/debug/jd"
W, KIND = sys.argv[1], sys.argv[2]
EXTRA = json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}
WORLD = EXTRA.pop("_world", {"extent": 2048, "res": 256})
shutil.rmtree(W, ignore_errors=True)
os.makedirs(W)
P = f"{W}/C.jdproj"
subprocess.run([JD, "new", P, "--extent", str(WORLD["extent"]), "--resolution", str(WORLD["res"])], check=True, capture_output=True)


def op(name, params, save=True):
    out = subprocess.run([JD, "--json", "op", name, "--project", P, "--params", json.dumps(params)] + (["--save"] if save else []), capture_output=True, text=True).stdout
    for line in out.splitlines():
        try:
            d = json.loads(line)
        except ValueError:
            continue
        if d.get("op") == name:
            if d["status"] == "error":
                print("ERROR", name, d["error"].get("message"), d["error"].get("next", ""))
            return d
    return {}


nodes = op("graph.list", {}, save=False)["result"]["nodes"]
noise = next(n["id"] for n in nodes if n["kind"].startswith("jd.noise."))
height = next(n["id"] for n in nodes if n["kind"] == "jd.output.height@1")
cave = op("graph.add_node", {"kind": "jd.cave.system@1", "params": {"kind": KIND, "seed": 3, **EXTRA}})["result"]["id"]
op("graph.connect", {"from": noise, "from_port": "out", "to": cave, "to_port": "in"})
op("graph.connect", {"from": cave, "from_port": "out", "to": height, "to_port": "in"})
for port, layer in [("holes", "cave_holes"), ("plan", "cave_plan")]:
    o = op("graph.add_node", {"kind": "jd.output.height@1", "params": {"layer": layer}})["result"]["id"]
    op("graph.connect", {"from": cave, "from_port": port, "to": o, "to_port": "in"})
t = time.time()
ev = op("graph.evaluate", {})
print("evaluate", ev.get("status"), f"{time.time() - t:.2f}s")
t = time.time()
r = op("caves.export", {"id": cave, "path": f"{W}/cave.glb"}, save=False).get("result", {})
print("cave.glb", f"{time.time() - t:.2f}s", {k: r.get(k) for k in ["triangles", "voxel_m", "length_m", "notes"]})
print("entrances", [(e["kind"], [round(x) for x in e["at"]], e["underwater"]) for e in r.get("entrances", [])])
op("caves.export", {"id": cave, "path": f"{W}/cave.fbx"}, save=False)
op("terrain.export_mesh", {"layer": "height", "path": f"{W}/terrain.glb", "holes": "cave_holes", "texture": True}, save=False)
json.dump({"cave": cave}, open(f"{W}/ids.json", "w"))
