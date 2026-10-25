# SPDX-License-Identifier: GPL-3.0-or-later
# Jenga Dunia ↔ Blender bridge. This add-on is GPL (Blender add-on requirement) and lives
# outside the MIT core; it talks to Jenga Dunia only through the `jd` command-line tool.
"""Import a Jenga Dunia project (`.jdproj`) into Blender.

File ▸ Import ▸ Jenga Dunia project: pick the `project.json` inside a `.jdproj` folder. The
add-on runs `jd export mesh` (textured glTF, metres, +Y up, world-centred) and optionally
`jd op terrain.scatter … .glb` (instances via EXT_mesh_gpu_instancing), then imports both with
Blender's glTF importer into a collection named after the project.

Requires the `jd` executable: on PATH, in the JD_BIN environment variable, or set in the
add-on preferences.
"""

import json
import os
import shutil
import subprocess
import tempfile

import bpy
from bpy.props import BoolProperty, FloatProperty, IntProperty, StringProperty
from bpy_extras.io_utils import ImportHelper


def jd_binary(preferred=""):
    """The `jd` executable to use: preference, then $JD_BIN, then PATH; None when absent."""
    for candidate in (preferred, os.environ.get("JD_BIN", "")):
        if candidate and os.path.isfile(candidate):
            return candidate
    return shutil.which("jd")


def run_jd(binary, args):
    """Run `jd --json …` and return the last envelope's result; raise on a non-ok envelope."""
    proc = subprocess.run([binary, "--json", *args], capture_output=True, text=True, check=False)
    envelopes = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
    last = envelopes[-1] if envelopes else {}
    if last.get("status") != "ok":
        err = last.get("error", {})
        detail = err.get("message") or proc.stderr.strip()[-400:] or "no output"
        hint = f" — {err['next']}" if err.get("next") else ""
        raise RuntimeError(f"jd {' '.join(args[:2])}: {err.get('code', 'E_PROCESS')} {detail}{hint}")
    return last["result"]


def _adopt(objects, collection, prefix):
    for obj in objects:
        for owner in obj.users_collection:
            owner.objects.unlink(obj)
        collection.objects.link(obj)
        if not obj.name.startswith(prefix):
            obj.name = f"{prefix} {obj.name}"


def import_project(context, binary, project, layer, step, texture, scatter_mask, spacing_m, seed, caves=True):
    """Export through `jd` and import into a new collection. Returns (terrain objects, instance count, mesh report).

    With `caves`, every `jd.cave.system@1` node comes along: its walls and speleothems as a mesh in
    the terrain's frame, and the terrain exported with the `cave_holes` layer so it opens over the mouths."""
    project = os.path.abspath(project)
    name = os.path.splitext(os.path.basename(project))[0]
    exports = tempfile.mkdtemp(prefix="jd-blender-")
    mesh_path = os.path.join(exports, f"{layer}.glb")
    graph = run_jd(binary, ["op", "graph.list", "--project", project]) if caves else {"nodes": []}
    cave_ids = [n["id"] for n in graph.get("nodes", []) if n.get("kind") == "jd.cave.system@1"]
    holes = any(n.get("kind", "").startswith("jd.output.height@") and (n.get("params") or {}).get("layer") == "cave_holes" for n in graph.get("nodes", []))
    if cave_ids and holes:
        params = {"layer": layer, "path": mesh_path, "texture": bool(texture), "holes": "cave_holes"}
        if step > 0:
            params["step"] = step
        report = run_jd(binary, ["op", "terrain.export_mesh", "--project", project, "--params", json.dumps(params)])
    else:
        args = ["export", "mesh", "--project", project, "--layer", layer, mesh_path]
        if step > 0:
            args += ["--step", str(step)]
        if not texture:
            args.append("--no-texture")
        report = run_jd(binary, args)

    collection = bpy.data.collections.new(f"JD {name}")
    context.scene.collection.children.link(collection)
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=mesh_path)
    terrain = [o for o in bpy.data.objects if o not in before]
    _adopt(terrain, collection, f"JD {name}")

    count = 0
    if scatter_mask:
        points = os.path.join(exports, f"{scatter_mask}.glb")
        params = {"mask": scatter_mask, "height": layer, "spacing_m": spacing_m, "seed": seed, "path": points}
        scatter = run_jd(binary, ["op", "terrain.scatter", "--project", project, "--params", json.dumps(params)])
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=points)
        _adopt([o for o in bpy.data.objects if o not in before], collection, f"JD {name} {scatter_mask}")
        count = int(scatter["count"])
    for i, cave in enumerate(cave_ids):
        path = os.path.join(exports, f"cave_{i + 1}.glb")
        run_jd(binary, ["op", "caves.export", "--project", project, "--params", json.dumps({"id": cave, "path": path})])
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=path)
        _adopt([o for o in bpy.data.objects if o not in before], collection, f"JD {name} cave {i + 1}")
    return terrain, count, report


class JDBridgePreferences(bpy.types.AddonPreferences):
    bl_idname = __package__

    jd_path: StringProperty(name="jd executable", subtype="FILE_PATH", default="", description="Path to the Jenga Dunia CLI (leave empty to use $JD_BIN or PATH)")

    def draw(self, _context):
        self.layout.prop(self, "jd_path")
        self.layout.label(text=f"detected: {jd_binary(self.jd_path) or 'not found'}")


class JD_OT_import_project(bpy.types.Operator, ImportHelper):
    """Export a layer (and optional scatter instances) with the jd CLI and import them"""

    bl_idname = "jd.import_project"
    bl_label = "Import Jenga Dunia project"
    bl_options = {"REGISTER", "UNDO"}

    filename_ext = ".json"
    filter_glob: StringProperty(default="project.json;*.jdproj", options={"HIDDEN"})
    layer: StringProperty(name="Layer", default="height", description="Published heightfield layer to export as the mesh")
    step: IntProperty(name="Grid step", default=0, min=0, description="Sample every N cells (0 = automatic, ≈ 1 M vertices)")
    texture: BoolProperty(name="Baked texture", default=True, description="Embed the hypsometric base-colour texture")
    scatter_mask: StringProperty(name="Scatter mask", default="", description="0..1 layer to scatter instances from (empty = none)")
    spacing_m: FloatProperty(name="Spacing (m)", default=40.0, min=0.1)
    seed: IntProperty(name="Seed", default=1, min=0)

    def execute(self, context):
        addon = context.preferences.addons.get(__package__)
        binary = jd_binary(addon.preferences.jd_path if addon else "")
        if not binary:
            self.report({"ERROR"}, "jd CLI not found: set it in the add-on preferences, in $JD_BIN, or put `jd` on PATH")
            return {"CANCELLED"}
        project = os.path.dirname(self.filepath) if os.path.isfile(self.filepath) else self.filepath
        try:
            terrain, count, report = import_project(context, binary, project, self.layer, self.step, self.texture, self.scatter_mask, self.spacing_m, self.seed)
        except (RuntimeError, OSError) as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        if not terrain:
            self.report({"ERROR"}, "glTF import produced no objects")
            return {"CANCELLED"}
        instances = f", {count:,} instances" if count else ""
        self.report({"INFO"}, f"Imported {self.layer}: {report['vertices']:,} vertices, {report['triangles']:,} triangles{instances}")
        return {"FINISHED"}


def import_splines(context, path, name="JD splines"):
    """Read a `terrain.export_splines` GeoJSON (layer metres) and build one poly curve per
    LineString in a new collection, in the terrain's frame (x = u − extent/2, y = v − extent/2
    is not known here: the file is in layer metres, so the curves are placed with the south-west
    corner at the origin and the caller moves the collection if the terrain was recentred)."""
    with open(path, encoding="utf-8") as fh:
        doc = json.load(fh)
    if doc.get("jd:coordinates", "local_m") != "local_m":
        raise RuntimeError("import the splines exported with coordinates = local_m (layer metres); this file is " + str(doc.get("jd:coordinates")))
    collection = bpy.data.collections.new(name)
    context.scene.collection.children.link(collection)
    made = 0
    for k, feature in enumerate(doc.get("features", [])):
        geom = feature.get("geometry") or {}
        if geom.get("type") != "LineString":
            continue
        pts = geom.get("coordinates") or []
        if len(pts) < 2:
            continue
        props = feature.get("properties") or {}
        curve = bpy.data.curves.new(f"{props.get('kind', 'line')}_{k}", type="CURVE")
        curve.dimensions = "3D"
        spline = curve.splines.new("POLY")
        spline.points.add(len(pts) - 1)
        for point, (u, v) in zip(spline.points, pts):
            point.co = (float(u), float(v), 0.0, 1.0)
        obj = bpy.data.objects.new(curve.name, curve)
        for key, value in props.items():
            if isinstance(value, (int, float, str)):
                obj[f"jd_{key}"] = value
        collection.objects.link(obj)
        made += 1
    return collection, made


class JD_OT_import_splines(bpy.types.Operator, ImportHelper):
    """Import rivers and placed splines (terrain.export_splines GeoJSON, layer metres) as poly curves"""

    bl_idname = "jd.import_splines"
    bl_label = "Import Jenga Dunia splines (.geojson)"
    bl_options = {"REGISTER", "UNDO"}

    filename_ext = ".geojson"
    filter_glob: StringProperty(default="*.geojson;*.json", options={"HIDDEN"})

    def execute(self, context):
        try:
            _, made = import_splines(context, self.filepath)
        except (RuntimeError, OSError, ValueError) as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        self.report({"INFO"}, f"Imported {made} curves")
        return {"FINISHED"}


def menu_func(self, _context):
    self.layout.operator(JD_OT_import_project.bl_idname, text="Jenga Dunia project (.jdproj)")
    self.layout.operator(JD_OT_import_splines.bl_idname, text="Jenga Dunia splines (.geojson)")


classes = (JDBridgePreferences, JD_OT_import_project, JD_OT_import_splines,)


def register():
    for cls in classes:
        bpy.utils.register_class(cls)
    bpy.types.TOPBAR_MT_file_import.append(menu_func)


def unregister():
    bpy.types.TOPBAR_MT_file_import.remove(menu_func)
    for cls in reversed(classes):
        bpy.utils.unregister_class(cls)
