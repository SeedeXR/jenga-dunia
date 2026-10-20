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
        [Serializable] class MaterialLayer { public string name; public string weightmap; public float[] albedo_srgb; public float roughness; }
        [Serializable] class Materials { public MaterialLayer[] layers; }
        [Serializable] class TreeInst { public float x, y, z, nx, nz, yaw_deg, scale; }
        [Serializable] class Trees { public int count; public TreeInst[] instances; }
        [Serializable] class Report
        {
            public string unity, dir; public int heightmap_resolution, source_px, alphamap_layers, trees;
            public float[] terrain_size, sampled_height_min_max; public string[] layers; public List<string> warnings = new List<string>(); public bool ok;
        }

        static string Arg(string name, string fallback = null)
        {
            var args = Environment.GetCommandLineArgs();
            for (var i = 0; i < args.Length - 1; i++) if (args[i] == name) return args[i + 1];
            return fallback;
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
                var matPath = Path.Combine(dir, "materials.json");
                if (File.Exists(matPath))
                    foreach (var m in JsonUtility.FromJson<Materials>(File.ReadAllText(matPath)).layers)
                        if (!string.IsNullOrEmpty(m.weightmap)) colours[m.weightmap] = new Color(m.albedo_srgb[0], m.albedo_srgb[1], m.albedo_srgb[2]);
                foreach (var f in Directory.GetFiles(dir, "weight_*.png")) layerFiles.Add(Path.GetFileName(f));
                layerFiles.Sort();
                AssetDatabase.CreateFolder("Assets", "JengaDunia");
                var layers = new List<TerrainLayer>();
                var weights = new List<Texture2D>();
                foreach (var f in layerFiles)
                {
                    var tex = new Texture2D(2, 2);
                    if (!ImageConversion.LoadImage(tex, File.ReadAllBytes(Path.Combine(dir, f)))) throw new Exception($"cannot decode {f}");
                    if (tex.width != w || tex.height != h) report.warnings.Add($"{f} is {tex.width}×{tex.height}, heightmap is {w}×{h}");
                    weights.Add(tex);
                    var diffuse = new Texture2D(2, 2);
                    var c = colours.TryGetValue(f, out var cc) ? cc : Color.gray;
                    diffuse.SetPixels(new[] { c, c, c, c }); diffuse.Apply();
                    AssetDatabase.CreateAsset(diffuse, $"Assets/JengaDunia/{f}.diffuse.asset");
                    var layer = new TerrainLayer { diffuseTexture = diffuse, tileSize = new Vector2(ex, ez), name = f };
                    AssetDatabase.CreateAsset(layer, $"Assets/JengaDunia/{f}.terrainlayer");
                    layers.Add(layer);
                }
                if (layers.Count > 0)
                {
                    data.terrainLayers = layers.ToArray();
                    data.alphamapResolution = res - 1;
                    var n = layers.Count;
                    var alpha = new float[res - 1, res - 1, n];
                    for (var y = 0; y < res - 1; y++)
                        for (var x = 0; x < res - 1; x++)
                        {
                            int sx = Math.Min(x, w - 1), sy = Math.Min(y, h - 1);
                            float sum = 0;
                            for (var k = 0; k < n; k++) { var v = weights[k].GetPixel(sx, h - 1 - sy).r; alpha[y, x, k] = v; sum += v; } // PNG row 0 = north
                            if (sum > 0) for (var k = 0; k < n; k++) alpha[y, x, k] /= sum; else alpha[y, x, 0] = 1;
                        }
                    data.SetAlphamaps(0, 0, alpha);
                }
                report.alphamap_layers = layers.Count;
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
