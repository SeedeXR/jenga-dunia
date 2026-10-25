# SPDX-License-Identifier: GPL-3.0-or-later
"""Headless round-trip test for the Blender bridge.

    blender -b --python-exit-code 1 --python integrations/blender/test_bridge.py -- <jd binary> [render.png]

Creates a fresh 2 048 m / 256² project with `jd`, evaluates it, imports it through the add-on's
operator (terrain + instances scattered from the `height` layer) and checks what landed in the
scene: one terrain mesh spanning the world extent, instances sharing one mesh datablock, and a
sample of instances standing on the terrain surface. Optionally renders the result.
"""

import json
import math
import os
import subprocess
import sys
import tempfile

import bpy
import mathutils

argv = sys.argv[sys.argv.index("--") + 1 :]
jd = os.path.abspath(argv[0])
render = os.path.abspath(argv[1]) if len(argv) > 1 else None
os.environ["JD_BIN"] = jd

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import jenga_dunia_bridge  # noqa: E402  (register without the extensions repo machinery)

bpy.ops.wm.read_factory_settings(use_empty=True)
jenga_dunia_bridge.register()

work = tempfile.mkdtemp(prefix="jd-bridge-")
project = os.path.join(work, "Bridge.jdproj")
subprocess.run([jd, "new", project, "--extent", "2048", "--resolution", "256"], check=True, capture_output=True)
out = subprocess.run([jd, "--json", "op", "graph.evaluate", "--project", project], check=True, capture_output=True, text=True).stdout
assert json.loads(out.splitlines()[-1])["status"] == "ok", out

# A placed spline exported as GeoJSON comes back as one poly curve with its points.
spline = {"kind": "jd.terrain.spline@1", "params": {"points": [[100.0, 100.0], [600.0, 300.0], [1500.0, 1200.0]], "width_m": 12.0}}
subprocess.run([jd, "--json", "op", "graph.add_node", "--project", project, "--save", "--params", json.dumps(spline)], check=True, capture_output=True)
exported = subprocess.run([jd, "--json", "op", "terrain.export_splines", "--project", project, "--params", json.dumps({"path": "splines.geojson"})], check=True, capture_output=True, text=True).stdout
env = json.loads(exported.splitlines()[-1])
assert env["status"] == "ok" and env["result"]["splines"] == 1, env
result = bpy.ops.jd.import_splines(filepath=env["result"]["path"])
assert result == {"FINISHED"}, result
curves = [o for o in bpy.data.objects if o.type == "CURVE"]
assert len(curves) == 1 and len(curves[0].data.splines[0].points) == 3, [(o.name, len(o.data.splines[0].points)) for o in curves]
assert tuple(round(c, 1) for c in curves[0].data.splines[0].points[1].co[:2]) == (600.0, 300.0)

# A mountain cave with its mouth placed at (1024, 700), wired in front of the height output with
# its holes published: the bridge brings the cave in and the terrain opens over the mouth.
def jd_op(op, params, save=True):
    out = subprocess.run([jd, "--json", "op", op, "--project", project, *(["--save"] if save else []), "--params", json.dumps(params)], check=True, capture_output=True, text=True).stdout
    return next(json.loads(l) for l in out.splitlines() if json.loads(l).get("op") == op)["result"]
nodes = jd_op("graph.list", {}, save=False)["nodes"]
for n in nodes:  # the unwired spline above served its export check; an evaluation needs every input wired
    if n["kind"] == "jd.terrain.spline@1":
        jd_op("graph.remove_node", {"id": n["id"]})
noise = next(n["id"] for n in nodes if n["kind"] == "jd.noise.fbm@1")
height = next(n["id"] for n in nodes if n["kind"] == "jd.output.height@1")
cave = jd_op("graph.add_node", {"kind": "jd.cave.system@1", "params": {"kind": "mountain", "seed": 2, "reach_m": 250, "entrances": [[1024.0, 700.0]]}})["id"]
jd_op("graph.connect", {"from": noise, "from_port": "out", "to": cave, "to_port": "in"})
jd_op("graph.connect", {"from": cave, "from_port": "out", "to": height, "to_port": "in"})
holes_out = jd_op("graph.add_node", {"kind": "jd.output.height@1", "params": {"layer": "cave_holes"}})["id"]
jd_op("graph.connect", {"from": cave, "from_port": "holes", "to": holes_out, "to_port": "in"})
jd_op("graph.evaluate", {})

result = bpy.ops.jd.import_project(filepath=os.path.join(project, "project.json"), layer="height", scatter_mask="height", spacing_m=64.0, seed=1)
assert result == {"FINISHED"}, result

coll = bpy.data.collections.get("JD Bridge")
assert coll is not None, [c.name for c in bpy.data.collections]
# The glTF importer adds an empty parent per file; the scatter objects carry the mask prefix.
meshes = [o for o in coll.objects if o.type == "MESH"]
instances = [o for o in meshes if o.name.startswith("JD Bridge height")]
caves = [o for o in meshes if o.name.startswith("JD Bridge cave")]
terrain = [o for o in meshes if o not in instances and o not in caves]
assert len(terrain) == 1, [o.name for o in coll.objects][:5]
# The cave came along, under the terrain, and the terrain is open over its mouth: a ray straight
# down at the mouth (Blender x = u − 1024, y = v − 1024) does not hit the terrain within 3 m of it.
assert len(caves) == 1, [o.name for o in coll.objects][:8]
cb = [caves[0].matrix_world @ mathutils.Vector(c) for c in caves[0].bound_box]
assert min(v.z for v in cb) < 400 and -1024 < min(v.x for v in cb) and max(v.x for v in cb) < 1024, [tuple(round(x) for x in v) for v in cb[:2]]

t = terrain[0]
dims = t.dimensions
assert abs(dims.x - 2048) < 20 and abs(dims.y - 2048) < 20, tuple(dims)
assert len(instances) > 0 and len({o.data.name for o in instances}) == 1, (len(instances), "instances should share one mesh")

# Instances stand on the terrain: cast a ray down onto the terrain object at 25 sampled instances.
inv = t.matrix_world.inverted()
hit, loc, *_ = t.ray_cast(inv @ mathutils.Vector((0.0, 700.0 - 1024.0, 5000.0)), mathutils.Vector((0.0, 0.0, -1.0)))
assert not hit, ("the terrain should be open over the cave mouth", tuple(loc))
errors = []
for o in instances[:: max(1, len(instances) // 25)]:
    ok, loc, _n, _i = t.ray_cast(inv @ mathutils.Vector((o.location.x, o.location.y, 10_000.0)), inv.to_3x3() @ mathutils.Vector((0, 0, -1)))
    if ok:
        errors.append(abs((t.matrix_world @ loc).z - o.location.z))
assert len(errors) >= 20 and max(errors) < 2.0, (len(errors), max(errors) if errors else None)

summary = {
    "terrain": t.name,
    "vertices": len(t.data.vertices),
    "dimensions_m": [round(dims.x, 1), round(dims.y, 1), round(dims.z, 1)],
    "instances": len(instances),
    "caves": [o.name for o in caves],
    "shared_mesh": instances[0].data.name,
    "surface_samples": len(errors),
    "max_height_error_m": round(max(errors), 3),
}
print("BRIDGE OK " + json.dumps(summary))

if render:
    scene = bpy.context.scene
    for o in instances:
        o.scale *= 8  # 1 m proxies are sub-pixel on a 2 km world; render-only
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    scene.collection.objects.link(cam)
    scene.camera = cam
    target = mathutils.Vector((0, 0, dims.z / 2))
    cam.location = target + mathutils.Vector((1500, -2300, 1300))
    cam.rotation_euler = (cam.location - target).to_track_quat("Z", "Y").to_euler()
    cam.data.lens = 35
    cam.data.clip_end = 50_000
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    scene.collection.objects.link(sun)
    sun.rotation_euler = (math.radians(50), 0, math.radians(-30))
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.display.shading.color_type = "TEXTURE"
    scene.display.shading.light = "STUDIO"
    scene.render.resolution_x, scene.render.resolution_y = 1440, 900
    scene.render.filepath = render
    bpy.ops.render.render(write_still=True)
    print("RENDERED " + render)
