//! Point scatter for vegetation / rocks / instances: a stratified jittered grid thinned by a
//! 0..1 density mask (`02-subsystems.md` §7). Deterministic from `seed` — the same mask and
//! settings give the same points on every machine, so instance exports are reproducible.
//!
//! ponytail: one candidate per grid cell (stratified jitter, no blue-noise relaxation) and a
//! single density mask; clustering, slope/height limits and per-species rules come with the
//! biome/material graph.

use crate::Heightfield;
use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

/// Scatter settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScatterParams {
    /// Target spacing between points in metres (grid cell size); must be ≥ one sample spacing.
    pub spacing_m: f32,
    /// Random offset inside the cell, 0 (regular grid) .. 1 (anywhere in the cell).
    #[serde(default = "one")]
    pub jitter: f32,
    /// Seed for the jitter and the density test.
    #[serde(default)]
    pub seed: u64,
}
fn one() -> f32 {
    1.0
}

/// A scattered instance in world metres (`u` east, `v` north from the south edge, `z` height).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScatterPoint {
    /// Easting from the west edge, metres.
    pub u: f32,
    /// Northing from the south edge, metres.
    pub v: f32,
    /// Terrain height at the point (0 when no height layer is given).
    pub z: f32,
    /// Deterministic 0..1 value per point for the caller's own variation (scale, rotation).
    pub r: f32,
}

/// splitmix64 — small, fast, deterministic.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Bilinear sample of a field at world metres (`u` east, `v` north; row 0 = south), clamped.
fn sample(f: &Heightfield, extent_m: [f64; 2], u: f32, v: f32) -> f32 {
    let (w, h) = (f.width() as f32, f.height() as f32);
    let x = (u as f64 / extent_m[0] * w as f64) as f32 - 0.5;
    let y = (v as f64 / extent_m[1] * h as f64) as f32 - 0.5;
    let x0 = x.floor().clamp(0.0, w - 1.0);
    let y0 = y.floor().clamp(0.0, h - 1.0);
    let x1 = (x0 + 1.0).min(w - 1.0);
    let y1 = (y0 + 1.0).min(h - 1.0);
    let (tx, ty) = ((x - x0).clamp(0.0, 1.0), (y - y0).clamp(0.0, 1.0));
    let g = |xx: f32, yy: f32| f.get(xx as u32, yy as u32);
    let a = g(x0, y0) + (g(x1, y0) - g(x0, y0)) * tx;
    let b = g(x0, y1) + (g(x1, y1) - g(x0, y1)) * tx;
    a + (b - a) * ty
}

/// Scatter points over the world: one jittered candidate per `spacing_m` cell, kept with
/// probability equal to the mask value (0..1) sampled at the candidate.
pub fn scatter_points(
    mask: &Heightfield,
    height: Option<&Heightfield>,
    extent_m: [f64; 2],
    p: &ScatterParams,
) -> JdResult<Vec<ScatterPoint>> {
    if !(extent_m[0] > 0.0 && extent_m[1] > 0.0) {
        return Err(JdError::invalid("extent_m", "must be positive"));
    }
    let cell = (extent_m[0] / mask.width() as f64).min(extent_m[1] / mask.height() as f64) as f32;
    if !(p.spacing_m.is_finite() && p.spacing_m >= cell) {
        return Err(JdError::invalid(
            "spacing_m",
            format!("must be finite and ≥ the sample spacing ({cell} m)"),
        ));
    }
    if !(0.0..=1.0).contains(&p.jitter) {
        return Err(JdError::invalid("jitter", "must be within 0..=1"));
    }
    if let Some(h) = height
        && (h.width(), h.height()) != (mask.width(), mask.height())
    {
        return Err(JdError::invalid("height", "height and mask sizes differ"));
    }
    let nx = (extent_m[0] / p.spacing_m as f64).ceil() as u64;
    let ny = (extent_m[1] / p.spacing_m as f64).ceil() as u64;
    if nx * ny > 50_000_000 {
        return Err(JdError::invalid(
            "spacing_m",
            "would place more than 50 M candidates; use a larger spacing",
        ));
    }
    let mut out = Vec::new();
    for j in 0..ny {
        for i in 0..nx {
            let h = mix(p.seed ^ mix(i ^ (j << 32)));
            let (ju, jv, keep, r) = (
                unit(h),
                unit(mix(h ^ 1)),
                unit(mix(h ^ 2)),
                unit(mix(h ^ 3)),
            );
            let u = ((i as f32 + 0.5 + (ju - 0.5) * p.jitter) * p.spacing_m)
                .min(extent_m[0] as f32 - 1e-3);
            let v = ((j as f32 + 0.5 + (jv - 0.5) * p.jitter) * p.spacing_m)
                .min(extent_m[1] as f32 - 1e-3);
            if u < 0.0 || v < 0.0 {
                continue;
            }
            let density = sample(mask, extent_m, u, v).clamp(0.0, 1.0);
            if keep < density {
                let z = height.map_or(0.0, |hf| sample(hf, extent_m, u, v));
                out.push(ScatterPoint { u, v, z, r });
            }
        }
    }
    Ok(out)
}

/// Drop every point within `radius_m` of any of `others` — ecosystem layers that exclude each
/// other (trees keep clear of the rocks placed before them). Hash grid of `radius_m` cells, so a
/// point checks nine buckets. Returns the survivors in their original order.
pub fn exclude_near(
    points: Vec<ScatterPoint>,
    others: &[[f32; 2]],
    radius_m: f32,
) -> JdResult<Vec<ScatterPoint>> {
    if !(radius_m > 0.0 && radius_m.is_finite()) {
        return Err(JdError::invalid("radius_m", "must be positive"));
    }
    if others.is_empty() {
        return Ok(points);
    }
    let cell = |x: f32| (x / radius_m).floor() as i64;
    let mut grid: std::collections::HashMap<(i64, i64), Vec<[f32; 2]>> =
        std::collections::HashMap::new();
    for q in others {
        grid.entry((cell(q[0]), cell(q[1]))).or_default().push(*q);
    }
    let r2 = radius_m * radius_m;
    Ok(points
        .into_iter()
        .filter(|p| {
            let (cx, cy) = (cell(p.u), cell(p.v));
            !(-1..=1).any(|dx| {
                (-1..=1).any(|dy| {
                    grid.get(&(cx + dx, cy + dy)).is_some_and(|b| {
                        b.iter()
                            .any(|q| (q[0] - p.u).powi(2) + (q[1] - p.v).powi(2) < r2)
                    })
                })
            })
        })
        .collect())
}

/// Read the `u,v` columns of a scatter CSV (`u,v,z,r` header) back as positions.
pub fn read_points_csv(path: &std::path::Path) -> JdResult<Vec<[f32; 2]>> {
    parse_points_csv(&std::fs::read_to_string(path)?)
        .map_err(|e| e.detail("path", path.display().to_string()))
}

/// Parse the `u,v` columns of scatter CSV text (`u,v,z,r` header).
pub fn parse_points_csv(text: &str) -> JdResult<Vec<[f32; 2]>> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate().skip(1) {
        let mut cols = line.split(',');
        let (u, v) = (
            cols.next().and_then(|c| c.trim().parse::<f32>().ok()),
            cols.next().and_then(|c| c.trim().parse::<f32>().ok()),
        );
        match (u, v) {
            (Some(u), Some(v)) if u.is_finite() && v.is_finite() => out.push([u, v]),
            _ => {
                return Err(JdError::new(
                    jd_types::ErrorCode::E_FORMAT_CORRUPT,
                    format!("line {} is not `u,v,…`", n + 1),
                ));
            }
        }
    }
    Ok(out)
}

/// Write points as CSV (`u,v,z,r` header), JSON (array of objects) or GLB (one proxy mesh
/// instanced with `EXT_mesh_gpu_instancing`), by extension. `extent_m` places GLB instances in
/// the terrain mesh frame (+X east, +Y up, +Z south, world-centred).
/// [`write_points_with`] using the 1 m cone proxy for `.glb`.
pub fn write_points(
    path: &std::path::Path,
    points: &[ScatterPoint],
    extent_m: [f64; 2],
) -> JdResult<u64> {
    write_points_with(path, points, extent_m, None)
}

/// Write scatter points as `.csv`, `.json` or `.glb`; a `.glb` instances `proxy` (a generated
/// rock, any mesh) or, without one, the 1 m cone.
pub fn write_points_with(
    path: &std::path::Path,
    points: &[ScatterPoint],
    extent_m: [f64; 2],
    proxy: Option<&crate::mesh::GridMesh>,
) -> JdResult<u64> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    let bytes = match ext.as_deref() {
        Some("csv") => {
            let mut s = String::from("u,v,z,r\n");
            for p in points {
                s.push_str(&format!("{},{},{},{}\n", p.u, p.v, p.z, p.r));
            }
            s.into_bytes()
        }
        Some("json") => serde_json::to_vec(points)?,
        Some("glb") => return write_instances_glb(path, points, extent_m, proxy),
        _ => {
            return Err(JdError::new(
                jd_types::ErrorCode::E_FORMAT_UNSUPPORTED,
                "unsupported scatter format",
            )
            .next("Use .csv, .json or .glb."));
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &bytes)?;
    Ok(bytes.len() as u64)
}

/// GLB with one 1 m proxy pyramid and an `EXT_mesh_gpu_instancing` node: per instance a
/// translation in the terrain mesh frame, a yaw from `r` and a 0.75..1.25 uniform scale.
/// Engines swap the proxy for the real asset and keep the transforms.
fn write_instances_glb(
    path: &std::path::Path,
    points: &[ScatterPoint],
    extent_m: [f64; 2],
    proxy: Option<&crate::mesh::GridMesh>,
) -> JdResult<u64> {
    if points.is_empty() {
        return Err(JdError::invalid("mask", "no points to export"));
    }
    let f32s = |bin: &mut Vec<u8>, vals: &[f32]| {
        for v in vals {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    };
    let mut bin = Vec::with_capacity(points.len() * 40 + 128);
    // The instanced mesh: a rock (or any mesh) when given, else the 1 m cone proxy — apex up,
    // 0.6 m square base; 4 sides + 2 base triangles, counter-clockwise outside.
    let cone_verts: [f32; 15] = [
        0.0, 1.0, 0.0, -0.3, 0.0, 0.3, 0.3, 0.0, 0.3, 0.3, 0.0, -0.3, -0.3, 0.0, -0.3,
    ];
    let cone_idx: [u32; 18] = [0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1, 1, 4, 3, 1, 3, 2];
    let (verts, nrms, idx): (&[f32], Option<&[f32]>, &[u32]) = match proxy {
        Some(m) => (&m.positions, Some(&m.normals), &m.indices),
        None => (&cone_verts, None, &cone_idx),
    };
    f32s(&mut bin, verts);
    let nrm_off = bin.len();
    if let Some(n) = nrms {
        f32s(&mut bin, n);
    }
    let idx_off = bin.len();
    for i in idx {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let (mut vmin, mut vmax) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in verts.as_chunks::<3>().0 {
        for k in 0..3 {
            vmin[k] = vmin[k].min(v[k]);
            vmax[k] = vmax[k].max(v[k]);
        }
    }
    let n_verts = verts.len() / 3;
    let tr_off = bin.len();
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in points {
        let t = [
            (p.u as f64 - extent_m[0] / 2.0) as f32,
            p.z,
            (extent_m[1] / 2.0 - p.v as f64) as f32,
        ];
        for k in 0..3 {
            min[k] = min[k].min(t[k]);
            max[k] = max[k].max(t[k]);
        }
        f32s(&mut bin, &t);
    }
    let rot_off = bin.len();
    for p in points {
        let half = std::f32::consts::PI * p.r; // yaw 2π·r as a quaternion about +Y
        f32s(&mut bin, &[0.0, half.sin(), 0.0, half.cos()]);
    }
    let sc_off = bin.len();
    for p in points {
        let s = 0.75 + 0.5 * (p.r * 7.31).fract();
        f32s(&mut bin, &[s, s, s]);
    }
    let n = points.len();
    // Buffer views and accessors: the NORMAL pair exists only when the proxy brings normals
    // (an empty view or a zero-count accessor is invalid glTF).
    let mut views = vec![
        serde_json::json!({ "buffer": 0, "byteOffset": 0, "byteLength": nrm_off, "target": 34962 }),
    ];
    let mut accessors = vec![
        serde_json::json!({ "bufferView": 0, "componentType": 5126, "count": n_verts, "type": "VEC3", "min": vmin, "max": vmax }),
    ];
    let mut attributes = serde_json::json!({ "POSITION": 0 });
    if nrms.is_some() {
        views.push(serde_json::json!({ "buffer": 0, "byteOffset": nrm_off, "byteLength": idx_off - nrm_off, "target": 34962 }));
        accessors.push(serde_json::json!({ "bufferView": 1, "componentType": 5126, "count": n_verts, "type": "VEC3" }));
        attributes["NORMAL"] = serde_json::json!(1);
    }
    let base = views.len();
    views.push(serde_json::json!({ "buffer": 0, "byteOffset": idx_off, "byteLength": tr_off - idx_off, "target": 34963 }));
    views.push(
        serde_json::json!({ "buffer": 0, "byteOffset": tr_off, "byteLength": rot_off - tr_off }),
    );
    views.push(
        serde_json::json!({ "buffer": 0, "byteOffset": rot_off, "byteLength": sc_off - rot_off }),
    );
    views.push(
        serde_json::json!({ "buffer": 0, "byteOffset": sc_off, "byteLength": bin.len() - sc_off }),
    );
    accessors.push(serde_json::json!({ "bufferView": base, "componentType": 5125, "count": idx.len(), "type": "SCALAR" }));
    accessors.push(serde_json::json!({ "bufferView": base + 1, "componentType": 5126, "count": n, "type": "VEC3", "min": min, "max": max }));
    accessors.push(serde_json::json!({ "bufferView": base + 2, "componentType": 5126, "count": n, "type": "VEC4" }));
    accessors.push(serde_json::json!({ "bufferView": base + 3, "componentType": 5126, "count": n, "type": "VEC3" }));
    let (idx_acc, tr_acc, rot_acc, sc_acc) = (base, base + 1, base + 2, base + 3);
    let json = serde_json::json!({
        "asset": { "version": "2.0", "generator": format!("jenga-dunia {}", env!("CARGO_PKG_VERSION")) },
        "extensionsUsed": ["EXT_mesh_gpu_instancing"],
        "extensionsRequired": ["EXT_mesh_gpu_instancing"],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": views,
        "accessors": accessors,
        "materials": [{ "name": "proxy", "pbrMetallicRoughness": { "baseColorFactor": if proxy.is_some() { [0.45, 0.42, 0.38, 1.0] } else { [0.2, 0.5, 0.2, 1.0] }, "metallicFactor": 0.0, "roughnessFactor": 0.9 } }],
        "meshes": [{ "name": if proxy.is_some() { "rock" } else { "proxy" }, "primitives": [{ "attributes": attributes, "indices": idx_acc, "material": 0, "mode": 4 }] }],
        "nodes": [{ "mesh": 0, "name": "instances", "extensions": { "EXT_mesh_gpu_instancing": { "attributes": { "TRANSLATION": tr_acc, "ROTATION": rot_acc, "SCALE": sc_acc } } } }],
        "scenes": [{ "nodes": [0] }],
        "scene": 0
    });
    crate::mesh::write_glb(path, &json, &bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusion_matches_brute_force_and_csv_round_trips() {
        let pt = |u: f32, v: f32| ScatterPoint {
            u,
            v,
            z: 0.0,
            r: 0.5,
        };
        let mut h = 7u64;
        let mut rnd = || {
            h = mix(h);
            unit(h) * 1000.0
        };
        let points: Vec<ScatterPoint> = (0..2000).map(|_| pt(rnd(), rnd())).collect();
        let others: Vec<[f32; 2]> = (0..300).map(|_| [rnd(), rnd()]).collect();
        let kept = exclude_near(points.clone(), &others, 25.0).expect("exclude");
        let brute: Vec<ScatterPoint> = points
            .iter()
            .copied()
            .filter(|p| {
                others
                    .iter()
                    .all(|q| (q[0] - p.u).powi(2) + (q[1] - p.v).powi(2) >= 625.0)
            })
            .collect();
        assert_eq!(kept, brute);
        assert!(
            kept.len() < points.len() && !kept.is_empty(),
            "{} of {}",
            kept.len(),
            points.len()
        );
        assert_eq!(
            exclude_near(points.clone(), &[], 25.0).expect("none").len(),
            points.len()
        );
        assert!(exclude_near(points.clone(), &others, 0.0).is_err());
        let dir = tempfile::tempdir().expect("tmp");
        let csv = dir.path().join("o.csv");
        let as_points: Vec<ScatterPoint> = others.iter().map(|q| pt(q[0], q[1])).collect();
        write_points_with(&csv, &as_points, [1000.0, 1000.0], None).expect("csv");
        assert_eq!(read_points_csv(&csv).expect("read"), others);
        std::fs::write(&csv, "u,v,z,r\n1,x,0,0\n").expect("write");
        assert!(read_points_csv(&csv).is_err());
    }

    #[test]
    fn density_controls_count_points_stay_inside_and_results_are_deterministic() {
        let mut full = Heightfield::new(64, 64, 64, 0).expect("hf");
        full.fill_with(|_, _| 1.0);
        let p = ScatterParams {
            spacing_m: 10.0,
            jitter: 1.0,
            seed: 7,
        };
        let pts = scatter_points(&full, None, [640.0, 640.0], &p).expect("scatter");
        assert_eq!(pts.len(), 64 * 64, "density 1 keeps every candidate");
        assert!(
            pts.iter()
                .all(|q| (0.0..640.0).contains(&q.u) && (0.0..640.0).contains(&q.v))
        );
        let mut half = full.clone();
        half.fill_with(|x, _| if x < 32 { 0.5 } else { 0.0 });
        let hp = scatter_points(&half, None, [640.0, 640.0], &p).expect("scatter");
        let n = hp.len() as f32;
        assert!(
            (n - 1024.0).abs() < 120.0,
            "half density on half the area ≈ 1024, got {n}"
        );
        assert!(
            hp.iter().all(|q| q.u < 320.0 + 10.0),
            "nothing where the mask is 0"
        );
        // Heights come from the height layer; regular grid when jitter = 0.
        let mut hgt = full.clone();
        hgt.fill_with(|x, _| x as f32);
        let g = scatter_points(
            &full,
            Some(&hgt),
            [640.0, 640.0],
            &ScatterParams {
                jitter: 0.0,
                ..p.clone()
            },
        )
        .expect("scatter");
        assert!((g[0].u - 5.0).abs() < 1e-4 && (g[0].v - 5.0).abs() < 1e-4);
        assert!(g[0].z >= 0.0 && g[63].z > g[0].z);
        let again = scatter_points(&half, None, [640.0, 640.0], &p).expect("scatter");
        assert_eq!(hp, again);
        assert!(
            scatter_points(
                &full,
                None,
                [640.0, 640.0],
                &ScatterParams {
                    spacing_m: 1.0,
                    ..p.clone()
                }
            )
            .is_err(),
            "finer than a sample"
        );
        let dir = tempfile::tempdir().expect("tmp");
        let ext = [640.0, 640.0];
        let n = write_points(&dir.path().join("p.csv"), &g[..3], ext).expect("csv");
        let txt = std::fs::read_to_string(dir.path().join("p.csv")).expect("read");
        assert!(txt.starts_with("u,v,z,r\n5,5,") && n as usize == txt.len());
        write_points(&dir.path().join("p.json"), &g[..3], ext).expect("json");
        assert!(write_points(&dir.path().join("p.txt"), &g[..3], ext).is_err());
        // GLB instances: valid container, one instancing node, translations in the mesh frame.
        let glb = dir.path().join("p.glb");
        let bytes = write_points(&glb, &g, ext).expect("glb");
        let raw = std::fs::read(&glb).expect("read");
        assert_eq!(bytes as usize, raw.len());
        assert_eq!(&raw[..4], b"glTF");
        let json_len = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]) as usize;
        let doc: serde_json::Value = serde_json::from_slice(&raw[20..20 + json_len]).expect("json");
        let attrs = &doc["nodes"][0]["extensions"]["EXT_mesh_gpu_instancing"]["attributes"];
        let tr = attrs["TRANSLATION"].as_u64().expect("tr") as usize;
        assert_eq!(doc["accessors"][tr]["count"], g.len());
        let bin_off = 20 + json_len + 8;
        let view =
            &doc["bufferViews"][doc["accessors"][tr]["bufferView"].as_u64().expect("v") as usize];
        let o = bin_off + view["byteOffset"].as_u64().expect("o") as usize;
        let t0: Vec<f32> = (0..3)
            .map(|k| f32::from_le_bytes(raw[o + 4 * k..o + 4 * k + 4].try_into().expect("f32")))
            .collect();
        // First point (u 5, v 5, z = height at x≈0): x = 5 − 320, y = z, z = 320 − 5.
        assert!(
            (t0[0] + 315.0).abs() < 1e-3
                && (t0[1] - g[0].z).abs() < 1e-6
                && (t0[2] - 315.0).abs() < 1e-3,
            "{t0:?}"
        );
        assert!(
            write_points(&dir.path().join("e.glb"), &[], ext).is_err(),
            "no points"
        );
    }
}
