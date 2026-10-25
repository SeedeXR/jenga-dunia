//! Engine export profiles (`docs/research/formats-sdks.md` §7, interoperability matrix): one
//! call writes everything an engine's terrain importer wants — heightmap in its preferred
//! encoding and row order, one weightmap per mask at the matching resolution, and a README with
//! the exact import settings (Unity Terrain Height / Unreal Z scale) computed from the data.
//!
//! The engine-side steps are documented, and verified headlessly where an editor is installed
//! (`scripts/unity-import-check.sh` rebuilds the Terrain; `scripts/unreal-import-check.sh` imports
//! the textures and the glTF mesh — the Landscape actor has no scripting entry point); the
//! README still spells out the manual steps for the editor.
//!
//! ponytail: two profiles, PNG8 weightmaps, no resampling to Unreal's recommended sizes (a
//! warning instead); resampling and Unity splatmap packing come with the material graph.

use std::path::{Path, PathBuf};

use crate::Heightfield;
use crate::export::{ExportOptions, HeightmapFormat};
use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

/// Target engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EngineProfile {
    /// Unity Terrain: RAW 16-bit little-endian, row 0 = south (no flip), Terrain Height = range.
    Unity,
    /// Unreal Landscape: PNG16, row 0 = north, Z scale = range_m × 100 / 512, 8-bit weightmaps.
    Unreal,
}

/// Engine-specific numbers the importer needs.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProfileSettings {
    /// Lowest encoded height (metres) — the terrain's base offset.
    pub min_m: f32,
    /// Highest encoded height (metres).
    pub max_m: f32,
    /// Unity: Terrain Height in metres (max − min); place the terrain at y = min.
    pub unity_terrain_height_m: Option<f32>,
    /// Unreal: Landscape Z scale for this encoding (range × 100 / 512).
    pub unreal_z_scale: Option<f32>,
    /// Unreal: Z location in centimetres so mid-grey (32768) lands at (min + max) / 2.
    pub unreal_location_z_cm: Option<f32>,
}

/// What the profile wrote.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProfileReport {
    /// Engine.
    pub profile: EngineProfile,
    /// Output directory.
    pub dir: PathBuf,
    /// Files written, relative to `dir`, in write order (heightmap, sidecar, weightmaps, README).
    pub files: Vec<String>,
    /// Heightmap size in pixels.
    pub size_px: [u32; 2],
    /// Import numbers.
    pub settings: ProfileSettings,
    /// Non-fatal notes (e.g. a size Unreal does not recommend).
    pub notes: Vec<String>,
    /// What the engine's material model cannot take from the material stack (empty without `materials`).
    #[serde(default)]
    pub loss: Vec<String>,
}

const UNREAL_SIZES: [u32; 7] = [127, 253, 505, 1009, 2017, 4033, 8129];

fn is_pow2_plus_1(n: u32) -> bool {
    n >= 33 && (n - 1).is_power_of_two()
}

/// A material layer to translate for an engine: a named weight (its mask) plus the look.
#[derive(Debug, Clone)]
pub struct ProfileMaterial<'a> {
    /// Layer name (file-safe); the weightmap is `weight_<name>.png`.
    pub name: String,
    /// 0..1 weight field at the heightmap's resolution; None = everywhere (cannot be a weightmap).
    pub mask: Option<&'a Heightfield>,
    /// Base colour, sRGB 0..1.
    pub albedo: [f32; 3],
    /// Roughness 0..1.
    pub roughness: f32,
    /// Emitted light, linear RGB 0..1 (black = none).
    pub emissive: [f32; 3],
    /// Tileable detail: written as `detail_<name>_albedo.png` / `detail_<name>_normal.png`
    /// with `tile_m` in the manifest.
    pub detail: Option<crate::detail::Detail>,
}

/// Write the profile for `height` with `masks` (name, 0..1 field) as weightmaps.
pub fn export_profile(
    profile: EngineProfile,
    height: &Heightfield,
    masks: &[(String, &Heightfield)],
    dir: &Path,
    range_m: Option<[f32; 2]>,
) -> JdResult<ProfileReport> {
    export_profile_with(profile, height, masks, &[], dir, range_m, None)
}

/// [`export_profile_with`] cut into `tiles × tiles` blocks, one engine folder per block
/// (`tile_<x>_<y>/`, x east, y north from the south-west corner) sharing one height range so
/// the blocks line up, plus a `tiles.json` manifest with each block's origin and size in metres.
/// Blocks are plain cuts (no overlap): engines that want 2ⁿ+1 samples per tile get a loss note.
#[allow(clippy::too_many_arguments)]
pub fn export_profile_tiled(
    profile: EngineProfile,
    height: &Heightfield,
    masks: &[(String, &Heightfield)],
    materials: &[ProfileMaterial<'_>],
    dir: &Path,
    range_m: Option<[f32; 2]>,
    geo: Option<&crate::GeoRef>,
    extent_m: [f64; 2],
    tiles: u32,
) -> JdResult<ProfileReport> {
    if !(2..=16).contains(&tiles) {
        return Err(JdError::invalid("tiles", "must be within 2..=16"));
    }
    let (w, h) = (height.width(), height.height());
    if w % tiles != 0 || h % tiles != 0 {
        return Err(JdError::invalid(
            "tiles",
            format!("{w}×{h} does not divide into {tiles}×{tiles} blocks"),
        )
        .next("Pick a tile count that divides the resolution, or a resolution that divides."));
    }
    let (tw, th) = (w / tiles, h / tiles);
    let stats = height.stats();
    let range = range_m.unwrap_or([stats.min, stats.max]);
    std::fs::create_dir_all(dir)?;
    let mut files = Vec::new();
    let mut manifest = Vec::new();
    let mut first: Option<ProfileReport> = None;
    for ty in 0..tiles {
        for tx in 0..tiles {
            let name = format!("tile_{tx}_{ty}");
            let sub = dir.join(&name);
            let hf = height.crop(tx * tw, ty * th, tw, th)?;
            let cut: Vec<(String, Heightfield)> = masks
                .iter()
                .map(|(n, m)| Ok((n.clone(), m.crop(tx * tw, ty * th, tw, th)?)))
                .collect::<JdResult<_>>()?;
            let refs: Vec<(String, &Heightfield)> =
                cut.iter().map(|(n, m)| (n.clone(), m)).collect();
            let mats: Vec<ProfileMaterial<'_>> = materials
                .iter()
                .map(|m| ProfileMaterial {
                    name: m.name.clone(),
                    mask: m
                        .mask
                        .and_then(|_| refs.iter().find(|(n, _)| *n == m.name).map(|(_, f)| *f)),
                    albedo: m.albedo,
                    roughness: m.roughness,
                    emissive: m.emissive,
                    detail: m.detail,
                })
                .collect();
            let r = export_profile_with(profile, &hf, &refs, &mats, &sub, Some(range), geo)?;
            for f in &r.files {
                files.push(format!("{name}/{f}"));
            }
            manifest.push(serde_json::json!({
                "tile": [tx, ty], "dir": name, "size_px": r.size_px,
                "origin_m": [tx as f64 * extent_m[0] / tiles as f64, ty as f64 * extent_m[1] / tiles as f64],
                "extent_m": [extent_m[0] / tiles as f64, extent_m[1] / tiles as f64],
            }));
            if first.is_none() {
                first = Some(r);
            }
        }
    }
    let mut loss = vec!["tiles are plain cuts: neighbouring blocks share no edge row, so an engine that stitches tiles needs its seams welded (Unity: neighbour terrains; Unreal: World Partition's landscape streaming proxies)".to_owned()];
    if profile == EngineProfile::Unity && !(tw - 1).is_power_of_two() {
        loss.push(format!(
            "Unity wants 2ⁿ+1 samples per terrain tile; these are {tw}×{th}"
        ));
    }
    std::fs::write(
        dir.join("tiles.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({ "engine": match profile { EngineProfile::Unity => "unity", EngineProfile::Unreal => "unreal" }, "tiles": tiles, "range_m": range, "blocks": manifest, "loss": loss }),
        )?,
    )?;
    files.push("tiles.json".to_owned());
    let mut report =
        first.ok_or_else(|| JdError::new(jd_types::ErrorCode::E_INTERNAL, "no tiles written"))?;
    report.dir = dir.to_path_buf();
    report.files = files;
    report.size_px = [w, h];
    Ok(report)
}

/// [`export_profile`] with the material stack translated for the engine: every masked layer
/// becomes a weightmap, `materials.json` carries the look per layer, and `loss` says what the
/// engine's own material model cannot take from the stack (the translation layer's report).
#[allow(clippy::too_many_arguments)]
pub fn export_profile_with(
    profile: EngineProfile,
    height: &Heightfield,
    masks: &[(String, &Heightfield)],
    materials: &[ProfileMaterial<'_>],
    dir: &Path,
    range_m: Option<[f32; 2]>,
    geo: Option<&crate::GeoRef>,
) -> JdResult<ProfileReport> {
    height.validate_finite()?;
    let safe = |name: &str| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    for m in materials {
        if !safe(&m.name) {
            return Err(JdError::invalid(
                "materials",
                format!("layer `{}` is not a safe file name", m.name),
            ));
        }
        if let Some(f) = m.mask
            && (f.width(), f.height()) != (height.width(), height.height())
        {
            return Err(JdError::invalid(
                "materials",
                format!("layer `{}`: mask size differs from the heightmap", m.name),
            ));
        }
    }
    for (name, m) in masks {
        if (m.width(), m.height()) != (height.width(), height.height()) {
            return Err(JdError::invalid("masks", format!("mask `{name}` size differs from the heightmap"))
                .next("Engines require weightmaps at the heightmap's resolution; evaluate both from the same graph."));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            || name.is_empty()
        {
            return Err(JdError::invalid(
                "masks",
                format!("mask `{name}` is not a safe file name"),
            ));
        }
    }
    std::fs::create_dir_all(dir)?;
    let (w, h) = (height.width(), height.height());
    let mut notes = Vec::new();
    let (hm_name, format, flip) = match profile {
        EngineProfile::Unity => {
            if !(is_pow2_plus_1(w) && w == h) {
                notes.push(format!("Unity Terrain resolutions are 2ⁿ+1 squares (33…4097); this is {w}×{h} — Unity will clamp or resample on import"));
            }
            ("heightmap.r16", HeightmapFormat::RawR16, Some(false))
        }
        EngineProfile::Unreal => {
            if !(UNREAL_SIZES.contains(&w) && w == h) {
                notes.push(format!("Unreal's recommended overall resolutions are 127/253/505/1009/2017/4033/8129; this is {w}×{h} — the importer will offer to resample or pad"));
            }
            ("heightmap.png", HeightmapFormat::Png16, Some(true))
        }
    };
    let report = crate::export_heightmap_geo(
        height,
        &dir.join(hm_name),
        &ExportOptions {
            format,
            flip_y: flip,
            range_m,
            georeference: true,
        },
        geo,
    )?;
    let [min_m, max_m] = report.range_m;
    let range = max_m - min_m;
    let mut files = vec![
        hm_name.to_owned(),
        report
            .sidecar
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("heightmap.jd-height.json")
            .to_owned(),
    ];
    // Weightmaps: 8-bit grayscale PNG (Unreal's format; Unity reads them through the script).
    let write_weight = |name: &str, m: &Heightfield, files: &mut Vec<String>| -> JdResult<String> {
        let mut img = image::GrayImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = (m.get(x, y).clamp(0.0, 1.0) * 255.0).round() as u8;
                // Image row 0 is north for both engines' weightmap importers.
                img.put_pixel(x, h - 1 - y, image::Luma([v]));
            }
        }
        let file = format!("weight_{name}.png");
        if files.contains(&file) {
            return Ok(file); // named in `masks` and as a material mask: one weightmap
        }
        img.save(dir.join(&file)).map_err(|e| {
            JdError::new(
                jd_types::ErrorCode::E_INTERNAL,
                format!("PNG write failed: {e}"),
            )
        })?;
        files.push(file.clone());
        Ok(file)
    };
    for (name, m) in masks {
        write_weight(name, m, &mut files)?;
    }
    // Ambient occlusion: 8-bit, 1 − sky occlusion within 16 cells (Unity: a custom terrain
    // shader's AO input or a mask-map G channel; Unreal: a landscape material texture sample).
    {
        let sp = geo.map_or([1.0, 1.0], |g| [g.spacing[0] as f32, g.spacing[1] as f32]);
        let occ =
            crate::occlusion_mask(height, sp, crate::mesh::AO_REACH_CELLS * sp[0].max(sp[1]))?;
        let mut img = image::GrayImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((1.0 - occ.get(x, y).clamp(0.0, 1.0)) * 255.0).round() as u8;
                img.put_pixel(x, h - 1 - y, image::Luma([v]));
            }
        }
        img.save(dir.join("ao.png")).map_err(|e| {
            JdError::new(
                jd_types::ErrorCode::E_INTERNAL,
                format!("PNG write failed: {e}"),
            )
        })?;
        files.push("ao.png".to_owned());
    }
    // Material translation: masked layers → weightmaps + a manifest; what does not fit → `loss`.
    let mut loss = Vec::new();
    if !materials.is_empty() {
        let mut layers = Vec::new();
        for m in materials {
            let weightmap = match m.mask {
                Some(f) => Some(write_weight(&m.name, f, &mut files)?),
                None => {
                    loss.push(format!("layer `{}` has no mask (it applies everywhere): no weightmap — treat it as the base layer", m.name));
                    None
                }
            };
            let mut entry = serde_json::json!({
                "name": m.name, "weightmap": weightmap, "albedo_srgb": m.albedo, "roughness": m.roughness,
                "smoothness": 1.0 - m.roughness, "emissive": m.emissive,
            });
            if let Some(d) = &m.detail {
                let tile = crate::detail::bake_detail(d, m.albedo)?;
                let (fa, fn_) = (
                    format!("detail_{}_albedo.png", m.name),
                    format!("detail_{}_normal.png", m.name),
                );
                std::fs::write(dir.join(&fa), &tile.albedo)?;
                std::fs::write(dir.join(&fn_), &tile.normal)?;
                files.push(fa.clone());
                files.push(fn_.clone());
                entry["detail_albedo"] = serde_json::json!(fa);
                entry["detail_normal"] = serde_json::json!(fn_);
                entry["tile_m"] = serde_json::json!(d.tile_m);
                entry["detail_px"] = serde_json::json!(tile.px);
            }
            layers.push(entry);
        }
        if materials.iter().any(|m| m.detail.is_some()) {
            loss.push("detail tiles: `detail_<name>_albedo.png` (the colour with its grain) and `detail_<name>_normal.png` (tangent space, +Y up) repeat every `tile_m` metres — Unity: TerrainLayer diffuse + normal with tileSize = tile_m (the headless importer does this); Unreal: the layer's material samples them with UV × extent ÷ tile_m".to_owned());
        } else {
            loss.push("flat colours: both engines paint terrain layers with textures, not colours — assign a texture per layer and use `albedo_srgb` as its tint, or give the layer a `detail`".to_owned());
        }
        loss.push("blend order: the stack lerps layers in order; engine weightmaps blend by normalised weight, so overlapping masks mix instead of the later one winning — order your masks to overlap little or pre-multiply them in the graph".to_owned());
        loss.push("weights are 8-bit (256 steps) at the heightmap's resolution".to_owned());
        if materials
            .iter()
            .any(|m| m.emissive.iter().any(|c| *c > 0.0))
        {
            loss.push("emissive: listed per layer; a terrain layer's material must take it as an emissive colour input (Unity: a custom terrain shader; Unreal: the Landscape material's Emissive Color, weighted by the layer)".to_owned());
        }
        match profile {
            EngineProfile::Unity => loss.push("roughness: a Unity Terrain Layer takes smoothness (= 1 − roughness, listed) through its mask map's alpha, not a scalar".to_owned()),
            EngineProfile::Unreal => loss.push("roughness: the Landscape material must expose a roughness input per layer; the scalar is listed, not applied".to_owned()),
        }
        // Packed splat maps: four weightmaps per RGBA PNG in layer order (Unity's alphamap, Unreal's
        // weight-blended layer packs), named splat_<k>.png with the layer names in the manifest.
        let mut splats = Vec::new();
        let weights: Vec<(&str, &Heightfield)> = materials
            .iter()
            .filter_map(|m| m.mask.map(|f| (m.name.as_str(), f)))
            .collect();
        for (k, chunk) in weights.chunks(4).enumerate() {
            let mut img = image::RgbaImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let mut px = [0u8; 4];
                    for (c, (_, f)) in chunk.iter().enumerate() {
                        px[c] = (f.get(x, y).clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                    img.put_pixel(x, h - 1 - y, image::Rgba(px));
                }
            }
            let file = format!("splat_{k}.png");
            img.save(dir.join(&file)).map_err(|e| {
                JdError::new(
                    jd_types::ErrorCode::E_INTERNAL,
                    format!("PNG write failed: {e}"),
                )
            })?;
            files.push(file.clone());
            splats.push(serde_json::json!({ "file": file, "channels": chunk.iter().map(|(n, _)| *n).collect::<Vec<_>>() }));
        }
        let manifest = serde_json::json!({ "engine": match profile { EngineProfile::Unity => "unity", EngineProfile::Unreal => "unreal" }, "layers": layers, "ao": "ao.png", "splatmaps": splats, "loss": loss });
        std::fs::write(
            dir.join("materials.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        files.push("materials.json".to_owned());
    }
    let settings = match profile {
        EngineProfile::Unity => ProfileSettings {
            min_m,
            max_m,
            unity_terrain_height_m: Some(range),
            unreal_z_scale: None,
            unreal_location_z_cm: None,
        },
        EngineProfile::Unreal => ProfileSettings {
            min_m,
            max_m,
            unity_terrain_height_m: None,
            unreal_z_scale: Some(range * 100.0 / 512.0),
            unreal_location_z_cm: Some((min_m + max_m) * 0.5 * 100.0),
        },
    };
    let mut readme = readme(profile, w, h, &settings, masks, &notes);
    if !materials.is_empty() {
        readme.push_str("\nMaterials: `materials.json` lists one entry per layer of the material stack (weightmap, albedo_srgb, roughness/smoothness, emissive) and the RGBA-packed `splat_<k>.png` maps (four layers per image, in order); `ao.png` is the terrain's ambient occlusion (8-bit, 1 − sky occlusion). Lost in translation:\n");
        for l in &loss {
            readme.push_str(&format!("- {l}\n"));
        }
    }
    std::fs::write(dir.join("README.md"), readme)?;
    files.push("README.md".to_owned());
    Ok(ProfileReport {
        profile,
        dir: dir.to_path_buf(),
        files,
        size_px: [w, h],
        settings,
        notes,
        loss,
    })
}

fn readme(
    profile: EngineProfile,
    w: u32,
    h: u32,
    s: &ProfileSettings,
    masks: &[(String, &Heightfield)],
    notes: &[String],
) -> String {
    let mut out = String::new();
    let layers: Vec<String> = masks
        .iter()
        .map(|(n, _)| format!("`weight_{n}.png`"))
        .collect();
    match profile {
        EngineProfile::Unity => {
            out.push_str(&format!("# Unity Terrain import (Jenga Dunia)\n\nHeightmap: `heightmap.r16` — {w}×{h}, 16-bit, little-endian, row 0 = south.\n\n"));
            out.push_str("1. Create a Terrain (GameObject → 3D Object → Terrain). In Terrain Settings set **Heightmap Resolution** to the file size.\n");
            out.push_str("2. Terrain Settings → **Import Raw**: Depth **Bit 16**, Byte Order **Windows**, **Flip Vertically: off**, Resolution = file size.\n");
            out.push_str(&format!("3. Set **Terrain Height** to **{:.2} m** (max − min of the encoded range) and move the Terrain to **y = {:.2} m** so heights are absolute.\n", s.unity_terrain_height_m.unwrap_or(0.0), s.min_m));
            out.push_str("4. Terrain Width/Length = the world extent in metres (see `heightmap.jd-height.json` → `size_px` and your project's `extent_m`).\n");
            if !layers.is_empty() {
                out.push_str(&format!("5. Splat layers: {} are 8-bit weights (row 0 = north); load them with `TerrainData.SetAlphamaps` (float[y, x, layer], 0..1 — normalise so each texel's weights sum to 1).\n", layers.join(", ")));
            }
            out.push_str("\nStatus: these steps follow the Unity 6 manual (Import Raw, TerrainData API). The repository's headless importer (`integrations/unity/Editor/JengaDuniaImport.cs`, run by `scripts/unity-import-check.sh`) rebuilds the Terrain from this directory on Unity 6000.4 and writes `jd-unity-import.json` with what Unity got.\n");
        }
        EngineProfile::Unreal => {
            out.push_str(&format!("# Unreal Landscape import (Jenga Dunia)\n\nHeightmap: `heightmap.png` — {w}×{h}, 16-bit grayscale PNG, row 0 = north.\n\n"));
            out.push_str("1. Landscape mode → **Import from File** → pick `heightmap.png` (keep the suggested component layout; see notes below for recommended sizes).\n");
            out.push_str(&format!("2. Location Z = **{:.1} cm**, Scale Z = **{:.3}**, Scale X/Y = **100** for 1 m per quad (scale X/Y by extent_m ÷ (size − 1) × 100 otherwise).\n", s.unreal_location_z_cm.unwrap_or(0.0), s.unreal_z_scale.unwrap_or(100.0)));
            out.push_str(&format!("   Derivation: the file encodes {:.2}…{:.2} m over 0…65535; Unreal maps that span to −256…+256 units × Z scale, so Z scale = range × 100 / 512 and mid-grey sits at (min + max) / 2.\n", s.min_m, s.max_m));
            if !layers.is_empty() {
                out.push_str(&format!("3. Layers: add one Landscape Layer per weightmap and assign {} (8-bit grayscale, same resolution as the heightmap — required by the importer).\n", layers.join(", ")));
            }
            out.push_str("\nStatus: follows the Unreal 5 Landscape Technical Guide (PNG16/r16 import, Z-scale formula, weightmap rules). The repository's headless check (`integrations/unreal/jd_import_check.py`, run by `scripts/unreal-import-check.sh`) imports this directory's PNGs through UnrealEditor-Cmd 5.8 and writes `jd-unreal-import.json` with what Unreal got; the Landscape actor itself is the editor's Landscape mode → Import from file, with the Z scale above — that step has no scripting entry point.\n");
        }
    }
    if !notes.is_empty() {
        out.push_str("\nNotes:\n");
        for n in notes {
            out.push_str(&format!("- {n}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiled_export_cuts_the_world_into_blocks_that_share_one_range() {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| (x + y) as f32);
        let dir = tempfile::tempdir().expect("tmp");
        let r = export_profile_tiled(
            EngineProfile::Unreal,
            &h,
            &[],
            &[],
            &dir.path().join("tiles"),
            None,
            None,
            [6400.0, 6400.0],
            2,
        )
        .expect("tiled");
        assert_eq!(r.size_px, [64, 64]);
        for name in ["tile_0_0", "tile_1_0", "tile_0_1", "tile_1_1"] {
            assert!(
                dir.path()
                    .join("tiles")
                    .join(name)
                    .join("heightmap.png")
                    .exists(),
                "{name}"
            );
        }
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("tiles/tiles.json")).expect("manifest"),
        )
        .expect("json");
        assert_eq!(manifest["blocks"].as_array().expect("blocks").len(), 4);
        assert_eq!(
            manifest["blocks"][3]["origin_m"],
            serde_json::json!([3200.0, 3200.0])
        );
        assert_eq!(
            manifest["range_m"],
            serde_json::json!([0.0, 126.0]),
            "one range for every block"
        );
        assert!(
            export_profile_tiled(
                EngineProfile::Unity,
                &h,
                &[],
                &[],
                &dir.path().join("bad"),
                None,
                None,
                [6400.0, 6400.0],
                3
            )
            .is_err(),
            "64 does not divide by 3"
        );
    }

    #[test]
    fn materials_translate_to_weightmaps_a_manifest_and_a_loss_list() {
        let mut h = Heightfield::new(33, 33, 64, 0).expect("hf");
        h.fill_with(|x, y| 100.0 + x as f32 * 10.0 + y as f32);
        let mut m = h.clone();
        m.fill_with(|x, _| if x < 16 { 1.0 } else { 0.0 });
        let stack = [
            ProfileMaterial {
                name: "base".into(),
                mask: None,
                albedo: [0.5, 0.5, 0.5],
                roughness: 0.9,
                emissive: [0.0; 3],
                detail: None,
            },
            ProfileMaterial {
                name: "snow".into(),
                mask: Some(&m),
                albedo: [0.95, 0.96, 0.98],
                roughness: 0.45,
                emissive: [0.0; 3],
                detail: None,
            },
        ];
        let dir = tempfile::tempdir().expect("tmp");
        let r = export_profile_with(
            EngineProfile::Unity,
            &h,
            &[],
            &stack,
            &dir.path().join("u"),
            None,
            None,
        )
        .expect("unity");
        assert!(
            r.files.contains(&"weight_snow.png".to_owned())
                && r.files.contains(&"materials.json".to_owned())
        );
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("u/materials.json")).expect("read"),
        )
        .expect("json");
        assert_eq!(manifest["engine"], "unity");
        assert_eq!(manifest["layers"][1]["weightmap"], "weight_snow.png");
        assert_eq!(
            manifest["splatmaps"][0]["channels"][0], "snow",
            "the masked layers are packed four per RGBA splat map"
        );
        assert!(dir.path().join("u/splat_0.png").exists());
        assert_eq!(manifest["ao"], "ao.png");
        assert!(
            dir.path().join("u/ao.png").exists(),
            "the ambient-occlusion map is part of every profile"
        );
        assert!(manifest["layers"][0]["weightmap"].is_null());
        assert!((manifest["layers"][1]["smoothness"].as_f64().expect("s") - 0.55).abs() < 1e-6);
        assert!(
            r.loss.iter().any(|l| l.contains("base layer"))
                && r.loss.iter().any(|l| l.contains("smoothness")),
            "{:?}",
            r.loss
        );
        let readme = std::fs::read_to_string(dir.path().join("u/README.md")).expect("readme");
        assert!(readme.contains("Lost in translation"));
        let plain = export_profile(EngineProfile::Unreal, &h, &[], &dir.path().join("p"), None)
            .expect("plain");
        assert!(plain.loss.is_empty() && !plain.files.contains(&"materials.json".to_owned()));
        let bad = [ProfileMaterial {
            name: "no/slash".into(),
            mask: None,
            albedo: [0.0; 3],
            roughness: 0.5,
            emissive: [0.0; 3],
            detail: None,
        }];
        assert!(
            export_profile_with(
                EngineProfile::Unity,
                &h,
                &[],
                &bad,
                &dir.path().join("b"),
                None,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn profiles_write_heightmap_weightmaps_and_import_numbers() {
        let mut h = Heightfield::new(33, 33, 64, 0).expect("hf");
        h.fill_with(|x, y| 100.0 + x as f32 * 10.0 + y as f32); // 100..452
        let mut m = h.clone();
        m.fill_with(|x, _| if x < 16 { 1.0 } else { 0.0 });
        let dir = tempfile::tempdir().expect("tmp");
        let u = export_profile(
            EngineProfile::Unity,
            &h,
            &[("forest".into(), &m)],
            &dir.path().join("unity"),
            None,
        )
        .expect("unity");
        assert_eq!(
            u.files,
            [
                "heightmap.r16",
                "heightmap.r16.jd-height.json",
                "weight_forest.png",
                "ao.png",
                "README.md"
            ]
        );
        assert!(
            u.notes.is_empty(),
            "33² is a valid Unity size: {:?}",
            u.notes
        );
        assert!((u.settings.unity_terrain_height_m.expect("th") - 352.0).abs() < 1e-3);
        let raw = std::fs::read(dir.path().join("unity/heightmap.r16")).expect("raw");
        assert_eq!(raw.len(), 33 * 33 * 2);
        assert_eq!(
            u16::from_le_bytes([raw[0], raw[1]]),
            0,
            "row 0 = south, x = 0 is the minimum"
        );
        let readme = std::fs::read_to_string(dir.path().join("unity/README.md")).expect("readme");
        assert!(
            readme.contains("Flip Vertically: off")
                && readme.contains("352.00 m")
                && readme.contains("unity-import-check")
        );
        let wm = image::open(dir.path().join("unity/weight_forest.png"))
            .expect("png")
            .into_luma8();
        assert_eq!(wm.dimensions(), (33, 33));
        assert_eq!(
            (wm.get_pixel(0, 0).0[0], wm.get_pixel(32, 0).0[0]),
            (255, 0)
        );

        let r = export_profile(
            EngineProfile::Unreal,
            &h,
            &[],
            &dir.path().join("unreal"),
            None,
        )
        .expect("unreal");
        assert_eq!(
            r.files,
            [
                "heightmap.png",
                "heightmap.png.jd-height.json",
                "ao.png",
                "README.md"
            ]
        );
        assert_eq!(r.notes.len(), 1, "33 is not an Unreal size");
        let z = r.settings.unreal_z_scale.expect("z");
        assert!((z - 352.0 * 100.0 / 512.0).abs() < 1e-3, "{z}");
        assert!((r.settings.unreal_location_z_cm.expect("loc") - 27600.0).abs() < 1e-2);
        let png = image::open(dir.path().join("unreal/heightmap.png"))
            .expect("png")
            .into_luma16();
        assert_eq!(
            png.get_pixel(0, 32).0[0],
            0,
            "row 0 = north: the south-west minimum is on the last row"
        );

        let mut bad = Heightfield::new(16, 16, 64, 0).expect("hf");
        bad.fill_with(|_, _| 1.0);
        assert!(
            export_profile(
                EngineProfile::Unity,
                &h,
                &[("x".into(), &bad)],
                &dir.path().join("bad"),
                None
            )
            .is_err()
        );
        assert!(
            export_profile(
                EngineProfile::Unity,
                &h,
                &[("../x".into(), &m)],
                &dir.path().join("bad2"),
                None
            )
            .is_err()
        );
    }
}
