# Compare a Jenga Dunia GLB's TANGENT attribute with Blender's MikkTSpace tangents (calc_tangents).
# Usage: /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/blender/tangent_check.py -- exports/terrain.glb
# Prints one TANGENT_CHECK {...} line: angular deviation (mean / p99 / max, degrees) and bitangent-sign agreement.
import bpy, json, struct, sys, math
from collections import defaultdict
glb = sys.argv[sys.argv.index("--") + 1]
data = open(glb, "rb").read()
jlen = struct.unpack_from("<I", data, 12)[0]
gltf = json.loads(data[20:20 + jlen])
boff = 20 + jlen + 8
def accessor(i):
    a = gltf["accessors"][i]; bv = gltf["bufferViews"][a["bufferView"]]
    n = {"VEC2": 2, "VEC3": 3, "VEC4": 4, "SCALAR": 1}[a["type"]]
    start = boff + bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    fmt = {5126: "f", 5125: "I", 5123: "H"}[a["componentType"]]
    size = struct.calcsize(fmt) * n
    return [struct.unpack_from("<" + fmt * n, data, start + k * size) for k in range(a["count"])]
prim = gltf["meshes"][0]["primitives"][0]
pos = accessor(prim["attributes"]["POSITION"]); tan = accessor(prim["attributes"]["TANGENT"])
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
obj = next(o for o in bpy.data.objects if o.type == "MESH")
me = obj.data
me.calc_tangents(uvmap=me.uv_layers[0].name)
per_vertex = defaultdict(list)
for loop in me.loops:
    t = loop.tangent  # Blender Z-up → glTF Y-up: (x, y, z) → (x, z, −y)
    per_vertex[loop.vertex_index].append(((t.x, t.z, -t.y), loop.bitangent_sign))
by_pos = {}
for v in me.vertices:
    c = v.co
    by_pos[(round(c.x, 3), round(c.z, 3), round(-c.y, 3))] = v.index
angles = []; signs_ok = 0; signs = 0; missing = 0; over30 = 0; over30_border = 0
xs = [q[0] for q in pos]; zs = [q[2] for q in pos]
lo = (min(xs) + 1e-3, min(zs) + 1e-3); hi = (max(xs) - 1e-3, max(zs) - 1e-3)
for p, t in zip(pos, tan):
    vi = by_pos.get((round(p[0], 3), round(p[1], 3), round(p[2], 3)))
    if vi is None:
        missing += 1; continue
    loops = per_vertex[vi]
    mx = sum(l[0][0] for l in loops) / len(loops); my = sum(l[0][1] for l in loops) / len(loops); mz = sum(l[0][2] for l in loops) / len(loops)
    n = math.sqrt(mx * mx + my * my + mz * mz) or 1.0
    dot = max(-1.0, min(1.0, (t[0] * mx + t[1] * my + t[2] * mz) / n))
    deg = math.degrees(math.acos(dot))
    angles.append(deg)
    if deg > 30:
        over30 += 1
        if p[0] < lo[0] or p[0] > hi[0] or p[2] < lo[1] or p[2] > hi[1]:
            over30_border += 1
    for l in loops:
        signs += 1; signs_ok += 1 if (l[1] > 0) != (t[3] > 0) else 0  # Blender flips V on import (v → 1 − v), so its bitangent sign is the negation of glTF w
angles.sort()
print("TANGENT_CHECK " + json.dumps({"vertices": len(pos), "matched": len(angles), "missing": missing,
    "mean_deg": round(sum(angles) / len(angles), 3), "p99_deg": round(angles[int(len(angles) * 0.99)], 3), "max_deg": round(angles[-1], 3),
    "over_30_deg": over30, "over_30_deg_on_border": over30_border,
    "bitangent_sign_agree_pct": round(100.0 * signs_ok / signs, 2)}))
