"""Blender: render terrain.glb + cave.glb from a scripts/caves/demo.py folder — a top view and an
oblique view with the terrain x-rayed, and optionally a close-up of the cave alone.

    blender -b --python scripts/caves/render.py -- <dir> [x y dist [z]]

(x, y, z) is the close-up target in Blender coordinates: x = u − extent/2, y = v − extent/2, z = height.
"""
import bpy, sys, math, os
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
D = argv[0]
bpy.ops.wm.read_factory_settings(use_empty=True)
sc = bpy.context.scene
for name in ["terrain.glb", "cave.glb"]:
    bpy.ops.import_scene.gltf(filepath=os.path.join(D, name))
objs = [o for o in sc.objects if o.type == "MESH"]
cave = [o for o in objs if "cave" in (o.data.name + o.name).lower()] or objs[-1:]
terrain = [o for o in objs if o not in cave]


def mat(name, rgba):
    m = bpy.data.materials.new(name)
    m.diffuse_color = rgba
    return m


for o in terrain:
    o.data.materials.clear(); o.data.materials.append(mat("t", (0.45, 0.5, 0.42, 1)))
for o in cave:
    o.data.materials.clear(); o.data.materials.append(mat("c", (0.95, 0.45, 0.1, 1)))
sc.render.engine = "BLENDER_WORKBENCH"
sh = sc.display.shading
sh.light = "STUDIO"; sh.color_type = "MATERIAL"; sh.show_xray = True; sh.xray_alpha = 0.35
sc.render.resolution_x, sc.render.resolution_y = 1400, 900
cam = bpy.data.cameras.new("cam"); co = bpy.data.objects.new("cam", cam); sc.collection.objects.link(co); sc.camera = co
cam.clip_end = 100000
pts = [c.matrix_world @ Vector(v) for c in cave for v in c.bound_box]
lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
centre = (lo + hi) / 2
span = max((hi - lo).x, (hi - lo).y, 50)


def shoot(target, dist, elev_deg, azim_deg, out, xray=True):
    sh.show_xray = xray
    e, a = math.radians(elev_deg), math.radians(azim_deg)
    co.location = target + Vector((math.cos(e) * math.cos(a), math.cos(e) * math.sin(a), math.sin(e))) * dist
    co.rotation_euler = (target - co.location).to_track_quat("-Z", "Y").to_euler()
    sc.render.filepath = out
    bpy.ops.render.render(write_still=True)


cam.type = "ORTHO"; cam.ortho_scale = span * 1.15
shoot(centre, 3000, 89.9, -90, os.path.join(D, "top.png"))
cam.type = "PERSP"
shoot(centre, span * 1.3, 35, -60, os.path.join(D, "overview.png"))
if len(argv) >= 4:
    tz = float(argv[4]) if len(argv) >= 5 else centre.z
    for o in terrain:
        o.hide_render = True
    sh.show_cavity = True
    shoot(Vector((float(argv[1]), float(argv[2]), tz)), float(argv[3]), 30, -40, os.path.join(D, "closeup.png"), xray=False)
print("cave bounds", tuple(round(x) for x in lo), tuple(round(x) for x in hi))
