// Jenga Dunia → Unity Terrain, headless. Rebuilds a Terrain from a `terrain.export_profile`
// directory (heightmap.r16 + sidecar, weight_<name>.png, materials.json, trees_unity.json) and
// writes `jd-unity-import.json` next to it with what Unity actually got — the verification the
// README's manual steps describe, run by `scripts/unity-import-check.sh`:
//   Unity -batchmode -nographics -quit -projectPath <proj> -executeMethod JengaDunia.Import.Run -jdDir <dir> [-jdExtent 5120,5120]
// MIT, like the rest of the repository.
using System;
using System.Collections.Generic;
using System.IO;
using UnityEditor;
using UnityEngine;

namespace JengaDunia
{
    public static class Import
    {
        [Serializable] class Sidecar { public int[] size_px; public float[] range_m; public bool flip_y; public double[] spacing_m; }
        [Serializable] class MaterialLayer { public string name; public string weightmap; public float[] albedo_srgb; public float roughness; public string detail_albedo; public string detail_normal; public float tile_m; }
        [Serializable] class Materials { public MaterialLayer[] layers; }
        [Serializable] class TreeInst { public float x, y, z, nx, nz, yaw_deg, scale; }
        [Serializable] class Trees { public int count; public TreeInst[] instances; }
        [Serializable] class Report
        {
            public string unity, dir; public int heightmap_resolution, source_px, alphamap_layers, trees, holes;
            public float[] terrain_size, sampled_height_min_max; public string[] layers; public float mean_height; public float[] layer_mean_height; public string[] layer_detail; public float[] layer_tile_m; public List<string> warnings = new List<string>(); public bool ok;
        }

        static string Arg(string name, string fallback = null)
        {
            var args = Environment.GetCommandLineArgs();
            for (var i = 0; i < args.Length - 1; i++) if (args[i] == name) return args[i + 1];
            return fallback;
        }

        [Serializable] class FbxReport { public string unity, file; public int meshes, vertices, triangles; public float[] bounds_size, bounds_min; public string[] mesh_names; public List<string> warnings = new List<string>(); public bool ok; }

        /// Import a Jenga Dunia FBX through Unity's own FBX importer (the Autodesk FBX SDK behind it) and report the mesh it produced:
        ///   Unity -batchmode -nographics -quit -projectPath <proj> -executeMethod JengaDunia.Import.Fbx -jdFbx <file.fbx>
        public static void Fbx()
        {
            var report = new FbxReport { unity = Application.unityVersion };
            try
            {
                var src = Arg("-jdFbx") ?? throw new Exception("-jdFbx <file.fbx> is required");
                report.file = src;
                if (!AssetDatabase.IsValidFolder("Assets/JengaDunia")) AssetDatabase.CreateFolder("Assets", "JengaDunia");
                var dst = "Assets/JengaDunia/" + Path.GetFileName(src);
                File.Copy(src, dst, true);
                AssetDatabase.ImportAsset(dst, ImportAssetOptions.ForceSynchronousImport | ImportAssetOptions.ForceUpdate);
                var names = new List<string>();
                var size = Vector3.zero; var min = Vector3.positiveInfinity;
                foreach (var obj in AssetDatabase.LoadAllAssetsAtPath(dst))
                {
                    if (!(obj is Mesh mesh)) continue;
                    names.Add(mesh.name);
                    report.meshes++;
                    report.vertices += mesh.vertexCount;
                    for (var i = 0; i < mesh.subMeshCount; i++) report.triangles += (int)(mesh.GetIndexCount(i) / 3);
                    size = Vector3.Max(size, mesh.bounds.size); min = Vector3.Min(min, mesh.bounds.min);
                }
                if (report.meshes == 0) throw new Exception("Unity's importer produced no Mesh from " + dst + " (see the editor log for the importer's error)");
                report.mesh_names = names.ToArray();
                report.bounds_size = new[] { size.x, size.y, size.z };
                report.bounds_min = new[] { min.x, min.y, min.z };
                var importer = AssetImporter.GetAtPath(dst) as ModelImporter;
                if (importer != null) report.warnings.Add($"ModelImporter: scale {importer.globalScale}, units {(importer.useFileUnits ? "file" : "unity")}, scale factor {importer.fileScale}");
                report.ok = true;
            }
            catch (Exception e)
            {
                report.ok = false;
                report.warnings.Add("ERROR: " + e.Message);
                Debug.LogError(e);
            }
            var json = JsonUtility.ToJson(report, true);
            Debug.Log("JD_UNITY_FBX " + json);
            if (report.file != null) File.WriteAllText(report.file + ".jd-unity-import.json", json);
            EditorApplication.Exit(report.ok ? 0 : 1);
        }

        public static void Run()
        {
            var report = new Report { unity = Application.unityVersion };
            try
            {
                var dir = Arg("-jdDir") ?? throw new Exception("-jdDir <profile directory> is required");
                report.dir = dir;
                var side = JsonUtility.FromJson<Sidecar>(File.ReadAllText(Path.Combine(dir, "heightmap.r16.jd-height.json")));
                int w = side.size_px[0], h = side.size_px[1];
                report.source_px = w;
                var extentArg = Arg("-jdExtent");
                float ex, ez;
                if (extentArg != null) { var p = extentArg.Split(','); ex = float.Parse(p[0]); ez = float.Parse(p.Length > 1 ? p[1] : p[0]); }
                else if (side.spacing_m != null && side.spacing_m.Length == 2) { ex = (float)(side.spacing_m[0] * w); ez = (float)(side.spacing_m[1] * h); }
                else throw new Exception("no extent: pass -jdExtent x,z or export with a sidecar that carries spacing_m");

                // Heightmap: 16-bit little-endian RAW, row 0 = south (the Unity profile writes no flip).
                var raw = File.ReadAllBytes(Path.Combine(dir, "heightmap.r16"));
                if (raw.Length != w * h * 2) throw new Exception($"heightmap.r16 is {raw.Length} bytes, expected {w * h * 2}");
                var res = 33; while (res < Math.Max(w, h)) res = (res - 1) * 2 + 1; // nearest 2ⁿ+1 above
                if (res != w || w != h) report.warnings.Add($"source {w}×{h} is not a 2ⁿ+1 square: TerrainData resolution {res}, the extra row/column is left at the edge value");
                var data = new TerrainData { heightmapResolution = res };
                float range = side.range_m[1] - side.range_m[0];
                data.size = new Vector3(ex, range, ez);
                var heights = new float[res, res];
                for (var y = 0; y < res; y++)
                    for (var x = 0; x < res; x++)
                    {
                        int sx = Math.Min(x, w - 1), sy = Math.Min(y, h - 1);
                        int row = side.flip_y ? (h - 1 - sy) : sy;
                        var o = (row * w + sx) * 2;
                        heights[y, x] = (raw[o] | (raw[o + 1] << 8)) / 65535f;
                    }
                data.SetHeights(0, 0, heights);

                // Weightmaps → terrain layers (solid-colour textures tinted from materials.json when present).
                var layerFiles = new List<string>();
                var colours = new Dictionary<string, Color>();
                var details = new Dictionary<string, MaterialLayer>();
                var matPath = Path.Combine(dir, "materials.json");
                if (File.Exists(matPath))
                    foreach (var m in JsonUtility.FromJson<Materials>(File.ReadAllText(matPath)).layers)
                        if (!string.IsNullOrEmpty(m.weightmap)) { colours[m.weightmap] = new Color(m.albedo_srgb[0], m.albedo_srgb[1], m.albedo_srgb[2]); if (!string.IsNullOrEmpty(m.detail_albedo)) details[m.weightmap] = m; }
                foreach (var f in Directory.GetFiles(dir, "weight_*.png")) layerFiles.Add(Path.GetFileName(f));
                layerFiles.Sort();
                AssetDatabase.CreateFolder("Assets", "JengaDunia");
                var layers = new List<TerrainLayer>();
                var weights = new List<Texture2D>();
                var layerDetail = new List<string>(); var layerTile = new List<float>();
                foreach (var f in layerFiles)
                {
                    var tex = new Texture2D(2, 2);
                    if (!ImageConversion.LoadImage(tex, File.ReadAllBytes(Path.Combine(dir, f)))) throw new Exception($"cannot decode {f}");
                    if (tex.width != w || tex.height != h) report.warnings.Add($"{f} is {tex.width}×{tex.height}, heightmap is {w}×{h}");
                    weights.Add(tex);
                    TerrainLayer layer;
                    if (details.TryGetValue(f, out var d))
                    {
                        // Detail tiles: the colour tile as the diffuse, the tangent-space tile as the normal (linear), repeated every tile_m metres.
                        var albedo = new Texture2D(2, 2);
                        if (!ImageConversion.LoadImage(albedo, File.ReadAllBytes(Path.Combine(dir, d.detail_albedo)))) throw new Exception($"cannot decode {d.detail_albedo}");
                        albedo.wrapMode = TextureWrapMode.Repeat;
                        var normal = new Texture2D(2, 2, TextureFormat.RGBA32, true, true);
                        if (!ImageConversion.LoadImage(normal, File.ReadAllBytes(Path.Combine(dir, d.detail_normal)))) throw new Exception($"cannot decode {d.detail_normal}");
                        normal.wrapMode = TextureWrapMode.Repeat;
                        AssetDatabase.CreateAsset(albedo, $"Assets/JengaDunia/{f}.detail-albedo.asset");
                        AssetDatabase.CreateAsset(normal, $"Assets/JengaDunia/{f}.detail-normal.asset");
                        layer = new TerrainLayer { diffuseTexture = albedo, normalMapTexture = normal, normalScale = 1f, tileSize = new Vector2(d.tile_m, d.tile_m), name = f };
                        layerDetail.Add(d.detail_albedo); layerTile.Add(d.tile_m);
                    }
                    else
                    {
                        var diffuse = new Texture2D(2, 2);
                        var c = colours.TryGetValue(f, out var cc) ? cc : Color.gray;
                        diffuse.SetPixels(new[] { c, c, c, c }); diffuse.Apply();
                        AssetDatabase.CreateAsset(diffuse, $"Assets/JengaDunia/{f}.diffuse.asset");
                        layer = new TerrainLayer { diffuseTexture = diffuse, tileSize = new Vector2(ex, ez), name = f };
                        layerDetail.Add(""); layerTile.Add(ex);
                    }
                    AssetDatabase.CreateAsset(layer, $"Assets/JengaDunia/{f}.terrainlayer");
                    layers.Add(layer);
                }
                report.layer_detail = layerDetail.ToArray(); report.layer_tile_m = layerTile.ToArray();
                if (layers.Count > 0)
                {
                    data.terrainLayers = layers.ToArray();
                    data.alphamapResolution = res - 1;
                    var n = layers.Count;
                    var alpha = new float[res - 1, res - 1, n];
                    // Registration check: the mean terrain height (0..1) under each layer's > 0.5 cells vs the whole —
                    // a snow layer that lands on the low ground means the rows are flipped.
                    var layerSum = new double[n]; var layerCount = new int[n]; double allSum = 0;
                    for (var y = 0; y < res - 1; y++)
                        for (var x = 0; x < res - 1; x++)
                        {
                            int sx = Math.Min(x, w - 1), sy = Math.Min(y, h - 1);
                            float sum = 0;
                            for (var k = 0; k < n; k++) { var v = weights[k].GetPixel(sx, sy).r; alpha[y, x, k] = v; sum += v; } // PNG row 0 = north; LoadImage puts it at texture y = h-1, so y = 0 is already south
                            if (sum > 0) for (var k = 0; k < n; k++) alpha[y, x, k] /= sum; else alpha[y, x, 0] = 1;
                            allSum += heights[y, x];
                            for (var k = 0; k < n; k++) if (alpha[y, x, k] > 0.5f) { layerSum[k] += heights[y, x]; layerCount[k]++; }
                        }
                    data.SetAlphamaps(0, 0, alpha);
                    report.mean_height = (float)(allSum / ((res - 1) * (res - 1)));
                    report.layer_mean_height = new float[n];
                    for (var k = 0; k < n; k++) report.layer_mean_height[k] = layerCount[k] > 0 ? (float)(layerSum[k] / layerCount[k]) : -1f;
                }
                report.alphamap_layers = layers.Count;
                // Holes: holes.png (255 = hole, PNG row 0 = north, LoadImage bottom-up like the weightmaps);
                // SetHoles takes true = terrain present, one cell per quad.
                var holesPath = Path.Combine(dir, "holes.png");
                if (File.Exists(holesPath))
                {
                    var tex = new Texture2D(2, 2);
                    if (!ImageConversion.LoadImage(tex, File.ReadAllBytes(holesPath))) throw new Exception("cannot decode holes.png");
                    data.holesResolution = res - 1;
                    var keep = new bool[res - 1, res - 1];
                    var holed = 0;
                    for (var y = 0; y < res - 1; y++)
                        for (var x = 0; x < res - 1; x++)
                        {
                            int sx = Math.Min(x, w - 1), sy = Math.Min(y, h - 1);
                            var hole = tex.GetPixel(sx, sy).r >= 0.5f;
                            keep[y, x] = !hole;
                            if (hole) holed++;
                        }
                    data.SetHoles(0, 0, keep);
                    report.holes = holed;
                }
                report.layers = layerFiles.ToArray();

                // Trees: one primitive prototype, instances from trees_unity.json (normalised nx/nz, height sampled from the terrain).
                var treePath = Path.Combine(dir, "trees_unity.json");
                if (File.Exists(treePath))
                {
                    var trees = JsonUtility.FromJson<Trees>(File.ReadAllText(treePath));
                    var proto = GameObject.CreatePrimitive(PrimitiveType.Capsule);
                    var prefab = PrefabUtility.SaveAsPrefabAsset(proto, "Assets/JengaDunia/tree.prefab");
                    UnityEngine.Object.DestroyImmediate(proto);
                    data.treePrototypes = new[] { new TreePrototype { prefab = prefab } };
                    var inst = new List<TreeInstance>();
                    foreach (var t in trees.instances)
                        inst.Add(new TreeInstance { prototypeIndex = 0, position = new Vector3(t.nx, data.GetInterpolatedHeight(t.nx, t.nz) / range, t.nz), widthScale = t.scale, heightScale = t.scale, rotation = t.yaw_deg * Mathf.Deg2Rad, color = Color.white, lightmapColor = Color.white });
                    data.SetTreeInstances(inst.ToArray(), true);
                    report.trees = inst.Count;
                }

                AssetDatabase.CreateAsset(data, "Assets/JengaDunia/Terrain.asset");
                var go = Terrain.CreateTerrainGameObject(data);
                go.transform.position = new Vector3(0, side.range_m[0], 0);
                var scene = UnityEditor.SceneManagement.EditorSceneManager.GetActiveScene();
                UnityEditor.SceneManagement.EditorSceneManager.SaveScene(scene, "Assets/JengaDunia/Terrain.unity");
                AssetDatabase.SaveAssets();
                var terrain = go.GetComponent<Terrain>();
                float mn = float.MaxValue, mx = float.MinValue;
                for (var y = 0; y < res; y += Math.Max(1, res / 64))
                    for (var x = 0; x < res; x += Math.Max(1, res / 64))
                    { var v = terrain.SampleHeight(new Vector3(x * ex / (res - 1), 0, y * ez / (res - 1))) + side.range_m[0]; mn = Math.Min(mn, v); mx = Math.Max(mx, v); }
                report.terrain_size = new[] { data.size.x, data.size.y, data.size.z };
                report.sampled_height_min_max = new[] { mn, mx };
                report.heightmap_resolution = data.heightmapResolution;
                report.ok = true;
            }
            catch (Exception e)
            {
                report.ok = false;
                report.warnings.Add("ERROR: " + e.Message);
                Debug.LogError(e);
            }
            var json = JsonUtility.ToJson(report, true);
            Debug.Log("JD_UNITY_IMPORT " + json);
            if (report.dir != null) File.WriteAllText(Path.Combine(report.dir, "jd-unity-import.json"), json);
            EditorApplication.Exit(report.ok ? 0 : 1);
        }
    }
}
