//! Terrain mesh export as binary glTF (`.glb`), written by hand: a regular grid with
//! positions (metres, +Y up, centred on the world like the viewport), normals, UVs, u32
//! indices and one PBR material. No dependency beyond `serde_json`; validated in CI with the
//! Khronos glTF validator (`apps/desktop/scripts/validate-glb.mjs`).
//!
//! ponytail: one dense grid primitive, optional `step` decimation; LOD/chunked meshes, draco
//! and textures come with the material graph (08-io-formats.md).

use std::fs;
use std::path::{Path, PathBuf};

use crate::Heightfield;
use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

/// What `export_glb` wrote.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MeshExportReport {
    /// Written file.
    pub path: PathBuf,
    /// Always `glb`.
    pub format: String,
    /// Vertex count.
    pub vertices: u32,
    /// Triangle count.
    pub triangles: u32,
    /// Grid step used (1 = every sample).
    pub step: u32,
    /// Bytes written.
    pub bytes: u64,
    /// World extent covered, metres.
    pub extent_m: [f64; 2],
}

/// Smallest step so the grid has at most `max_vertices` vertices.
pub fn auto_step(width: u32, height: u32, max_vertices: u32) -> u32 {
    let mut step = 1u32;
    let grid = |n: u32, s: u32| ((n.saturating_sub(1)).div_ceil(s) + 1) as u64;
    while grid(width, step) * grid(height, step) > max_vertices as u64 {
        step += 1;
    }
    step
}

fn pad4(v: &mut Vec<u8>, byte: u8) {
    while !v.len().is_multiple_of(4) {
        v.push(byte);
    }
}

/// Write the field as a `.glb` grid mesh covering `extent_m`: +X east, +Z south (field row 0 is
/// south and lands at +Z/2, matching the viewport and the image exports).
pub fn export_glb(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    hf.validate_finite()?;
    if step == 0 {
        return Err(JdError::invalid("step", "must be ≥ 1"));
    }
    if !(extent_m[0] > 0.0 && extent_m[1] > 0.0) {
        return Err(JdError::invalid("extent_m", "must be positive"));
    }
    let (w, h) = (hf.width(), hf.height());
    // Sample 0, step, 2·step, … and always the last row/column so the mesh reaches the edge.
    let (gw, gh) = ((w - 1).div_ceil(step) + 1, (h - 1).div_ceil(step) + 1);
    if gw < 2 || gh < 2 {
        return Err(JdError::invalid(
            "step",
            "grid must keep at least 2×2 samples",
        ));
    }
    let n_vertices = (gw as u64) * (gh as u64);
    if n_vertices > u32::MAX as u64 {
        return Err(JdError::invalid(
            "step",
            "too many vertices for 32-bit indices",
        ));
    }
    let (sx, sz) = (extent_m[0] / w as f64, extent_m[1] / h as f64);
    let sample = |gx: u32, gz: u32| hf.get((gx * step).min(w - 1), (gz * step).min(h - 1));
    let xm = |gx: u32| ((gx * step).min(w - 1) as f64 + 0.5) * sx - extent_m[0] / 2.0;
    let zm = |gz: u32| extent_m[1] / 2.0 - ((gz * step).min(h - 1) as f64 + 0.5) * sz;

    let mut bin: Vec<u8> =
        Vec::with_capacity((n_vertices as usize) * 32 + ((gw - 1) * (gh - 1)) as usize * 24);
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for gz in 0..gh {
        for gx in 0..gw {
            let p = [xm(gx) as f32, sample(gx, gz), zm(gz) as f32];
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
            for v in p {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let pos_len = bin.len();
    for gz in 0..gh {
        for gx in 0..gw {
            // Central differences over the exported grid (clamped at the border).
            let (x0, x1) = (gx.saturating_sub(1), (gx + 1).min(gw - 1));
            let (z0, z1) = (gz.saturating_sub(1), (gz + 1).min(gh - 1));
            let dx = (sample(x1, gz) - sample(x0, gz)) as f64 / (xm(x1) - xm(x0)).max(1e-9);
            let dz = (sample(gx, z1) - sample(gx, z0)) as f64 / (zm(z1) - zm(z0)).max(1e-9);
            let len = (dx * dx + 1.0 + dz * dz).sqrt();
            for v in [(-dx / len) as f32, (1.0 / len) as f32, (-dz / len) as f32] {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let nrm_len = bin.len() - pos_len;
    for gz in 0..gh {
        for gx in 0..gw {
            for v in [gx as f32 / (gw - 1) as f32, gz as f32 / (gh - 1) as f32] {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let uv_len = bin.len() - pos_len - nrm_len;
    let idx_off = bin.len();
    for gz in 0..gh - 1 {
        for gx in 0..gw - 1 {
            let a = gz * gw + gx;
            let b = a + 1;
            let c = a + gw;
            let d = c + 1;
            // Row c is north of row a (−Z): (a, b, c) and (b, d, c) are counter-clockwise from +Y.
            for i in [a, b, c, b, d, c] {
                bin.extend_from_slice(&i.to_le_bytes());
            }
        }
    }
    let idx_len = bin.len() - idx_off;
    let n_triangles = (gw - 1) * (gh - 1) * 2;
    pad4(&mut bin, 0);

    let json = serde_json::json!({
        "asset": { "version": "2.0", "generator": format!("jenga-dunia {}", env!("CARGO_PKG_VERSION")) },
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": pos_len, "target": 34962 },
            { "buffer": 0, "byteOffset": pos_len, "byteLength": nrm_len, "target": 34962 },
            { "buffer": 0, "byteOffset": pos_len + nrm_len, "byteLength": uv_len, "target": 34962 },
            { "buffer": 0, "byteOffset": idx_off, "byteLength": idx_len, "target": 34963 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": n_vertices, "type": "VEC3", "min": min, "max": max },
            { "bufferView": 1, "componentType": 5126, "count": n_vertices, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": n_vertices, "type": "VEC2" },
            { "bufferView": 3, "componentType": 5125, "count": n_triangles * 3, "type": "SCALAR" }
        ],
        "materials": [{ "name": "terrain", "pbrMetallicRoughness": { "baseColorFactor": [0.55, 0.5, 0.45, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.95 } }],
        "meshes": [{ "name": "terrain", "primitives": [{ "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 }, "indices": 3, "material": 0, "mode": 4 }] }],
        "nodes": [{ "mesh": 0, "name": "terrain" }],
        "scenes": [{ "nodes": [0] }],
        "scene": 0
    });
    let mut json_bytes = serde_json::to_vec(&json)?;
    pad4(&mut json_bytes, b' ');

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &out)?;
    Ok(MeshExportReport {
        path: path.to_path_buf(),
        format: "glb".into(),
        vertices: n_vertices as u32,
        triangles: n_triangles,
        step,
        bytes: out.len() as u64,
        extent_m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(bytes: &[u8]) -> (serde_json::Value, &[u8]) {
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            bytes.len()
        );
        let jlen = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(&bytes[16..20], b"JSON");
        let json: serde_json::Value =
            serde_json::from_slice(&bytes[20..20 + jlen]).expect("json chunk");
        let boff = 20 + jlen;
        let blen = u32::from_le_bytes(bytes[boff..boff + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[boff + 4..boff + 8], b"BIN\0");
        (json, &bytes[boff + 8..boff + 8 + blen])
    }

    #[test]
    fn glb_is_well_formed_and_faces_point_up() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut hf = Heightfield::new(9, 7, 8, 0).expect("hf");
        hf.fill_with(|x, y| (x as f32) * 2.0 + (y as f32) * 0.5);
        let p = dir.path().join("t.glb");
        let r = export_glb(&hf, [90.0, 70.0], &p, 1).expect("export");
        assert_eq!((r.vertices, r.triangles, r.step), (63, 96, 1));
        let bytes = fs::read(&p).expect("read");
        assert_eq!(bytes.len() as u64, r.bytes);
        let (json, bin) = chunks(&bytes);
        assert_eq!(json["asset"]["version"], "2.0");
        assert_eq!(json["buffers"][0]["byteLength"], bin.len());
        assert_eq!(json["accessors"][0]["count"], 63);
        assert_eq!(json["accessors"][3]["count"], 96 * 3);
        // Positions are centred on the world: min/max symmetric in x and z.
        let (mn, mx) = (&json["accessors"][0]["min"], &json["accessors"][0]["max"]);
        assert!((mn[0].as_f64().unwrap() + mx[0].as_f64().unwrap()).abs() < 1e-3);
        assert!((mn[2].as_f64().unwrap() + mx[2].as_f64().unwrap()).abs() < 1e-3);
        // Every normal has a positive Y component (a gentle slope never flips a face).
        let nrm_off = json["bufferViews"][1]["byteOffset"].as_u64().unwrap() as usize;
        for v in 0..63 {
            let o = nrm_off + v * 12 + 4;
            let ny = f32::from_le_bytes(bin[o..o + 4].try_into().unwrap());
            assert!(ny > 0.9, "normal y {ny}");
        }
        // Triangle winding: the first triangle's geometric normal points up.
        let pos = |i: usize| -> [f32; 3] {
            let o = i * 12;
            [0, 4, 8].map(|k| f32::from_le_bytes(bin[o + k..o + k + 4].try_into().unwrap()))
        };
        let idx_off = json["bufferViews"][3]["byteOffset"].as_u64().unwrap() as usize;
        let ix = |k: usize| {
            u32::from_le_bytes(
                bin[idx_off + k * 4..idx_off + k * 4 + 4]
                    .try_into()
                    .unwrap(),
            ) as usize
        };
        let (a, b, c) = (pos(ix(0)), pos(ix(1)), pos(ix(2)));
        let (u, v) = (
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
            [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
        );
        let ny = u[2] * v[0] - u[0] * v[2];
        assert!(
            ny > 0.0,
            "first triangle must be counter-clockwise from above: {ny}"
        );
    }

    #[test]
    fn south_row_lands_at_positive_z() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut hf = Heightfield::new(8, 8, 8, 0).expect("hf");
        hf.fill_with(|_, y| if y == 0 { 100.0 } else { 0.0 }); // row 0 = south is high
        let p = dir.path().join("o.glb");
        export_glb(&hf, [80.0, 80.0], &p, 1).expect("export");
        let bytes = fs::read(&p).expect("read");
        let (_, bin) = chunks(&bytes);
        let v = |i: usize| -> [f32; 3] {
            let o = i * 12;
            [0, 4, 8].map(|k| f32::from_le_bytes(bin[o + k..o + k + 4].try_into().unwrap()))
        };
        let [_, y0, z0] = v(0); // first vertex: row 0, column 0
        assert_eq!(y0, 100.0);
        assert!(z0 > 0.0, "south must be +Z like the viewport: {z0}");
        let [_, y7, z7] = v(7 * 8);
        assert!(y7 == 0.0 && z7 < 0.0);
    }

    #[test]
    fn step_decimates_and_auto_step_respects_the_budget() {
        assert_eq!(auto_step(4096, 4096, 1_048_576), 4);
        assert_eq!(
            auto_step(4097, 4097, 1_048_576),
            4,
            "2ⁿ+1: 1025 samples per side"
        );
        assert_eq!(auto_step(100, 100, 1_048_576), 1);
        let dir = tempfile::tempdir().expect("tmp");
        let hf = Heightfield::new(64, 64, 32, 4).expect("hf");
        let r = export_glb(&hf, [640.0, 640.0], &dir.path().join("s.glb"), 4).expect("export");
        assert_eq!(
            (r.vertices, r.triangles),
            (289, 512),
            "64 px at step 4 → 17 samples incl. the edge"
        );
        let (json, _) = chunks(&fs::read(dir.path().join("s.glb")).expect("read"));
        let mx = &json["accessors"][0]["max"];
        assert!(
            (mx[0].as_f64().unwrap() - 320.0 + 5.0).abs() < 1e-3,
            "last column sits on the east edge: {mx}"
        );
        assert!(export_glb(&hf, [640.0, 640.0], &dir.path().join("x.glb"), 0).is_err());
        assert!(
            export_glb(&hf, [640.0, 640.0], &dir.path().join("x.glb"), 64).is_err(),
            "1×1 grid has no faces"
        );
    }
}
