# Jenga Dunia FBX → Maya, headless. Imports a file through Maya's FBX plugin (the Autodesk FBX
# SDK) and prints one JSON line with what Maya got: meshes, vertices, faces, triangles, UV sets,
# bounds (Maya's linear unit — cm by default — so a metre file reads ×100), units, up axis.
#   mayapy integrations/maya/fbx_check.py <file.fbx>        (scripts/maya-fbx-check.sh finds mayapy)
# MIT, like the rest of the repository.
import json
import sys
import time

import maya.standalone

t0 = time.time()
maya.standalone.initialize(name="python")
import maya.cmds as cmds  # noqa: E402

src = sys.argv[1]
report = {"maya": None, "fbx_plugin": None, "file": src, "meshes": [], "ok": False, "error": None}
try:
    cmds.loadPlugin("fbxmaya", quiet=True)
    report["maya"] = cmds.about(version=True)
    report["fbx_plugin"] = cmds.pluginInfo("fbxmaya", q=True, version=True)
    cmds.file(src, i=True, type="FBX", ignoreVersion=True, mergeNamespacesOnClash=False, options="fbx")
    for m in cmds.ls(type="mesh", long=True) or []:
        tr = cmds.listRelatives(m, parent=True, fullPath=True)[0]
        bb = cmds.exactWorldBoundingBox(tr)
        report["meshes"].append({
            "name": tr.split("|")[-1],
            "vertices": cmds.polyEvaluate(m, vertex=True),
            "faces": cmds.polyEvaluate(m, face=True),
            "triangles": cmds.polyEvaluate(m, triangle=True),
            "uv_sets": cmds.polyUVSet(m, q=True, allUVSets=True),
            "bbox_size": [round(bb[3] - bb[0], 2), round(bb[4] - bb[1], 2), round(bb[5] - bb[2], 2)],
            "bbox_min": [round(v, 2) for v in bb[:3]],
        })
    report["units"] = cmds.currentUnit(q=True, linear=True)
    report["up_axis"] = cmds.upAxis(q=True, axis=True)
    report["ok"] = bool(report["meshes"])
except Exception as e:  # noqa: BLE001 — the report carries the error
    report["error"] = str(e)
report["seconds"] = round(time.time() - t0, 1)
print("JD_MAYA_FBX " + json.dumps(report))
with open(src + ".jd-maya-import.json", "w", encoding="utf-8") as f:
    json.dump(report, f, indent=2)
maya.standalone.uninitialize()
sys.exit(0 if report["ok"] else 1)
