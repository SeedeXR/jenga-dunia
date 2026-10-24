"""Jenga Dunia → Unreal Engine import check, run inside UnrealEditor-Cmd (`-run=pythonscript`).

Imports what the exporters wrote and reports what Unreal made of it, as JSON next to the input:
  JD_PROFILE=<dir>   a `terrain.export_profile {profile: "unreal"}` directory: the 16-bit heightmap
                     and the 8-bit weightmaps become Texture2D assets (format, size), the README's
                     Z scale is parsed back — the Landscape actor itself is created in the editor's
                     Landscape mode (no scripting entry point for that step; documented as manual).
  JD_MESH=<file>     a .glb / .fbx from `terrain.export_mesh`: Interchange import → StaticMesh
                     (triangles, vertices, bounds in cm, sections, UVs).
Everything is read back from the imported assets, never assumed.
"""
import json, os, re, sys, traceback
import unreal

OUT = os.environ.get("JD_OUT") or ""
report = {"ok": False, "unreal": unreal.SystemLibrary.get_engine_version(), "python": sys.version.split()[0], "notes": []}


def import_file(path, dest, name):
    task = unreal.AssetImportTask()
    task.set_editor_property("filename", path)
    task.set_editor_property("destination_path", dest)
    task.set_editor_property("destination_name", name)
    task.set_editor_property("automated", True)
    task.set_editor_property("replace_existing", True)
    task.set_editor_property("save", False)
    unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
    paths = list(task.get_editor_property("imported_object_paths") or [])
    return paths


def texture_info(asset):
    info = {"class": asset.get_class().get_name()}
    try:
        info["size"] = [asset.blueprint_get_size_x(), asset.blueprint_get_size_y()]
    except Exception:
        pass
    for prop in ("compression_settings", "srgb", "mip_gen_settings", "lod_group"):
        try:
            info[prop] = str(asset.get_editor_property(prop))
        except Exception:
            pass
    # The asset registry tags carry the platform format and dimensions; the source bit depth is
    # read through whichever accessor this engine exposes.
    try:
        reg = unreal.AssetRegistryHelpers.get_asset_registry()
        data = reg.get_asset_by_object_path(asset.get_path_name())
        tags = {}
        for tag in ("Format", "Dimensions", "SourceFormat", "SourceCompression", "ResourceSize"):
            try:
                v = data.get_tag_value(tag)
                if v:
                    tags[tag] = str(v)
            except Exception:
                pass
        info["registry"] = tags
    except Exception as e:
        info["registry"] = f"n/a ({e.__class__.__name__})"
    for prop in ("source_format", "SourceFormat", "source_size_x", "has_alpha_channel"):
        try:
            info[prop] = str(asset.get_editor_property(prop))
        except Exception:
            pass
    return info


def mesh_info(asset):
    info = {"class": asset.get_class().get_name()}
    try:
        info["lods"] = asset.get_num_lods()
        info["sections"] = asset.get_num_sections(0)
        # Interchange builds glTF/FBX meshes as Nanite by default (UE 5.8: InterchangeGenericMeshPipeline
        # bBuildNanite = true). LOD0 render data is then the Nanite *fallback* (FallbackRelativeError
        # 1.0 by default), not the mesh: the full mesh is what the asset registry's Nanite tags count.
        reg = unreal.AssetRegistryHelpers.get_asset_registry()
        data = reg.get_asset_by_object_path(asset.get_path_name())
        tag = lambda t: (lambda v: int(v) if v and str(v).isdigit() else None)(data.get_tag_value(t))
        nanite = str(data.get_tag_value("NaniteEnabled") or "").lower() == "true"
        info["nanite"] = {"enabled": nanite, "triangles": tag("NaniteTriangles"), "vertices": tag("NaniteVertices"),
                          "fallback_triangles": asset.get_num_triangles(0), "fallback_vertices": asset.get_num_vertices(0)}
        try:
            ns = asset.get_editor_property("nanite_settings")
            info["nanite"]["fallback_relative_error"] = float(ns.get_editor_property("fallback_relative_error"))
        except Exception:
            pass
        # The mesh as imported: Nanite's counts when Nanite is on, LOD0 otherwise.
        info["triangles"] = info["nanite"]["triangles"] if nanite and info["nanite"]["triangles"] else asset.get_num_triangles(0)
        info["vertices"] = info["nanite"]["vertices"] if nanite and info["nanite"]["vertices"] else asset.get_num_vertices(0)
    except Exception as e:
        info["error"] = repr(e)
    try:
        sub = unreal.get_editor_subsystem(unreal.StaticMeshEditorSubsystem)
        info["uv_channels"] = sub.get_num_uv_channels(asset, 0)
    except Exception as e:
        info["uv_channels"] = f"n/a ({e.__class__.__name__})"
    try:
        box = asset.get_bounding_box()
        info["bounds_cm"] = {"min": [box.min.x, box.min.y, box.min.z], "max": [box.max.x, box.max.y, box.max.z]}
    except Exception as e:
        info["bounds_error"] = repr(e)
    try:
        info["materials"] = [m.material_interface.get_name() if m.material_interface else None for m in asset.static_materials]
    except Exception:
        pass
    return info


try:
    profile = os.environ.get("JD_PROFILE")
    mesh = os.environ.get("JD_MESH")
    if profile:
        report["mode"] = "profile"
        readme = open(os.path.join(profile, "README.md"), encoding="utf-8").read() if os.path.exists(os.path.join(profile, "README.md")) else ""
        z = re.search(r"Z scale[^0-9\-]*([0-9.]+)", readme)
        report["readme_z_scale"] = float(z.group(1)) if z else None
        manifest = os.path.join(profile, "materials.json")
        report["manifest"] = json.load(open(manifest)) if os.path.exists(manifest) else None
        imported = {}
        for f in sorted(os.listdir(profile)):
            if f.lower().endswith(".png"):
                name = re.sub(r"[^A-Za-z0-9_]", "_", os.path.splitext(f)[0])
                paths = import_file(os.path.join(profile, f), "/Game/JengaDunia/Profile", "T_" + name)
                asset = unreal.EditorAssetLibrary.load_asset(paths[0]) if paths else None
                imported[f] = texture_info(asset) if asset else {"error": "no asset imported"}
        report["textures"] = imported
        hm = imported.get("heightmap.png", {})
        fmt = lambda v: (v.get("registry") or {}).get("SourceFormat") if isinstance(v.get("registry"), dict) else None
        weights = {k: v for k, v in imported.items() if k.startswith("weight_")}
        report["heightmap_source_format"] = fmt(hm)
        report["weightmap_source_formats"] = sorted({fmt(v) for v in weights.values()})
        details = {k: v for k, v in imported.items() if k.startswith("detail_")}
        report["detail_textures"] = {k: {"size": v.get("size"), "source_format": fmt(v)} for k, v in details.items()}
        # The profile promises a 16-bit grayscale heightmap and 8-bit grayscale weightmaps.
        report["ok"] = (
            bool(hm.get("size"))
            and fmt(hm) == "TSF G16"
            and all(fmt(v) == "TSF G8" for v in weights.values())
            and all("error" not in v for v in imported.values())
        )
        report["notes"].append("the Landscape actor is created in the editor (Landscape mode → Import from file) with the README's Z scale; no scripting entry point imports a heightmap into a new Landscape")
    elif mesh:
        report["mode"] = "mesh"
        name = re.sub(r"[^A-Za-z0-9_]", "_", os.path.splitext(os.path.basename(mesh))[0])
        paths = import_file(mesh, "/Game/JengaDunia/Mesh", "SM_" + name)
        report["imported_object_paths"] = paths
        meshes = [p for p in paths if unreal.EditorAssetLibrary.load_asset(p).get_class().get_name() == "StaticMesh"]
        report["static_meshes"] = {p: mesh_info(unreal.EditorAssetLibrary.load_asset(p)) for p in meshes}
        others = [unreal.EditorAssetLibrary.load_asset(p).get_class().get_name() for p in paths if p not in meshes]
        report["other_assets"] = others
        report["ok"] = len(meshes) > 0 and all("error" not in v for v in report["static_meshes"].values())
    else:
        report["notes"].append("set JD_PROFILE or JD_MESH")
except Exception:
    report["error"] = traceback.format_exc()
if OUT:
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2)
unreal.log("JD_REPORT " + json.dumps(report)[:4000])
