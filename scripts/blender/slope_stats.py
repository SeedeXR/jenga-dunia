# Slope histogram of a Jenga Dunia GLB: the share of faces steeper than 30°, 45° and 60°.
# Usage: /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/blender/slope_stats.py -- in.glb [in2.glb ...]
import bpy, sys, math, json
for glb in sys.argv[sys.argv.index("--") + 1:]:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=glb)
    me = next(o for o in bpy.data.objects if o.type == "MESH").data
    n = len(me.polygons); counts = {30: 0, 45: 0, 60: 0}
    for f in me.polygons:
        deg = math.degrees(math.acos(max(-1.0, min(1.0, abs(f.normal.z)))))
        for t in counts:
            if deg > t: counts[t] += 1
    print("SLOPES " + json.dumps({"file": glb.split("/")[-1], "faces": n, **{f"over_{t}_pct": round(100.0 * c / n, 2) for t, c in counts.items()}}))
