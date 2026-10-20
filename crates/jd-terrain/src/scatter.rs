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

/// Write points as CSV (`u,v,z,r` header) or JSON (array of objects), by extension.
pub fn write_points(path: &std::path::Path, points: &[ScatterPoint]) -> JdResult<u64> {
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
        _ => {
            return Err(JdError::new(
                jd_types::ErrorCode::E_FORMAT_UNSUPPORTED,
                "unsupported scatter format",
            )
            .next("Use .csv or .json."));
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &bytes)?;
    Ok(bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let n = write_points(&dir.path().join("p.csv"), &g[..3]).expect("csv");
        let txt = std::fs::read_to_string(dir.path().join("p.csv")).expect("read");
        assert!(txt.starts_with("u,v,z,r\n5,5,") && n as usize == txt.len());
        write_points(&dir.path().join("p.json"), &g[..3]).expect("json");
        assert!(write_points(&dir.path().join("p.txt"), &g[..3]).is_err());
    }
}
