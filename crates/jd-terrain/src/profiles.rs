//! Engine export profiles (`docs/research/formats-sdks.md` §7, interoperability matrix): one
//! call writes everything an engine's terrain importer wants — heightmap in its preferred
//! encoding and row order, one weightmap per mask at the matching resolution, and a README with
//! the exact import settings (Unity Terrain Height / Unreal Z scale) computed from the data.
//!
//! The engine-side steps are documented, not automated: no Unity/Unreal runner exists here, so
//! the README marks them UNVERIFIED until someone imports once and reports back.
//!
//! ponytail: two profiles, PNG8 weightmaps, no resampling to Unreal's recommended sizes (a
//! warning instead); resampling and Unity splatmap packing come with the material graph.

use std::path::{Path, PathBuf};

use crate::Heightfield;
use crate::export::{ExportOptions, HeightmapFormat, export_heightmap};
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
}

const UNREAL_SIZES: [u32; 7] = [127, 253, 505, 1009, 2017, 4033, 8129];

fn is_pow2_plus_1(n: u32) -> bool {
    n >= 33 && (n - 1).is_power_of_two()
}

/// Write the profile for `height` with `masks` (name, 0..1 field) as weightmaps.
pub fn export_profile(
    profile: EngineProfile,
    height: &Heightfield,
    masks: &[(String, &Heightfield)],
    dir: &Path,
    range_m: Option<[f32; 2]>,
) -> JdResult<ProfileReport> {
    height.validate_finite()?;
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
    let report = export_heightmap(
        height,
        &dir.join(hm_name),
        &ExportOptions {
            format,
            flip_y: flip,
            range_m,
            georeference: true,
        },
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
    for (name, m) in masks {
        let mut img = image::GrayImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = (m.get(x, y).clamp(0.0, 1.0) * 255.0).round() as u8;
                // Image row 0 is north for both engines' weightmap importers.
                img.put_pixel(x, h - 1 - y, image::Luma([v]));
            }
        }
        let file = format!("weight_{name}.png");
        img.save(dir.join(&file)).map_err(|e| {
            JdError::new(
                jd_types::ErrorCode::E_INTERNAL,
                format!("PNG write failed: {e}"),
            )
        })?;
        files.push(file);
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
    let readme = readme(profile, w, h, &settings, masks, &notes);
    std::fs::write(dir.join("README.md"), readme)?;
    files.push("README.md".to_owned());
    Ok(ProfileReport {
        profile,
        dir: dir.to_path_buf(),
        files,
        size_px: [w, h],
        settings,
        notes,
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
            out.push_str("\nStatus: these steps follow the Unity 6 manual (Import Raw, TerrainData API); UNVERIFIED on a live Unity until someone imports once — please report back.\n");
        }
        EngineProfile::Unreal => {
            out.push_str(&format!("# Unreal Landscape import (Jenga Dunia)\n\nHeightmap: `heightmap.png` — {w}×{h}, 16-bit grayscale PNG, row 0 = north.\n\n"));
            out.push_str("1. Landscape mode → **Import from File** → pick `heightmap.png` (keep the suggested component layout; see notes below for recommended sizes).\n");
            out.push_str(&format!("2. Location Z = **{:.1} cm**, Scale Z = **{:.3}**, Scale X/Y = **100** for 1 m per quad (scale X/Y by extent_m ÷ (size − 1) × 100 otherwise).\n", s.unreal_location_z_cm.unwrap_or(0.0), s.unreal_z_scale.unwrap_or(100.0)));
            out.push_str(&format!("   Derivation: the file encodes {:.2}…{:.2} m over 0…65535; Unreal maps that span to −256…+256 units × Z scale, so Z scale = range × 100 / 512 and mid-grey sits at (min + max) / 2.\n", s.min_m, s.max_m));
            if !layers.is_empty() {
                out.push_str(&format!("3. Layers: add one Landscape Layer per weightmap and assign {} (8-bit grayscale, same resolution as the heightmap — required by the importer).\n", layers.join(", ")));
            }
            out.push_str("\nStatus: follows the Unreal 5 Landscape Technical Guide (PNG16/r16 import, Z-scale formula, weightmap rules); UNVERIFIED on a live Unreal until someone imports once — please report back.\n");
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
                && readme.contains("UNVERIFIED")
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
            ["heightmap.png", "heightmap.png.jd-height.json", "README.md"]
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
