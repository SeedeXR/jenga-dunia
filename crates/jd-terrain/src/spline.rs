//! Splines: roads, paths, canals and dykes along a polyline. The terrain under the line is
//! reshaped to a profile that follows the line's own (smoothed) grade — flattened crosswise for a
//! road bed, cut below it for a canal or river bed, raised above it for an embankment — with a
//! soft shoulder outside the width, and a 0..1 mask of the corridor comes out for materials and
//! scatter. VUE's spline toolkit (roads, EcoSystems along splines) as a node
//! (`docs/research/vue-features.md`).
//!
//! The polyline is sampled every half cell; the grade along it is the terrain height at those
//! samples, box-smoothed over `smooth_m` so the road climbs gently instead of copying every
//! bump. Each cell takes the nearest sample (plain scan: cells × samples, fine to 512² and a
//! few km of road — ponytail: a bucket grid is the upgrade for long networks).

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;

/// What the corridor does to the terrain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SplineMode {
    /// Level the corridor crosswise onto the line's grade (a road bed, a path).
    #[default]
    Flatten,
    /// Sink the corridor `profile_m` below the grade (a canal, a river bed, a trench).
    Cut,
    /// Lift the corridor `profile_m` above the grade (an embankment, a dyke, a causeway).
    Raise,
}

/// Spline settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SplineParams {
    /// Polyline in layer metres: `[u east, v north]` from the south-west corner (≥ 2 points).
    #[serde(default)]
    pub points: Vec<[f32; 2]>,
    /// Corridor width, metres (> 0).
    #[serde(default = "d_width")]
    pub width_m: f32,
    /// Shoulder beyond the width over which the reshaping fades out, metres (≥ 0).
    #[serde(default = "d_falloff")]
    pub falloff_m: f32,
    /// Flatten, cut or raise.
    #[serde(default)]
    pub mode: SplineMode,
    /// Depth of a cut or height of a raise, metres (≥ 0; ignored by flatten).
    #[serde(default = "d_profile")]
    pub profile_m: f32,
    /// Length along the line over which the grade is smoothed, metres (≥ 0).
    #[serde(default = "d_smooth")]
    pub smooth_m: f32,
}

fn d_width() -> f32 {
    12.0
}
fn d_falloff() -> f32 {
    10.0
}
fn d_profile() -> f32 {
    3.0
}
fn d_smooth() -> f32 {
    200.0
}

impl Default for SplineParams {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            width_m: d_width(),
            falloff_m: d_falloff(),
            mode: SplineMode::Flatten,
            profile_m: d_profile(),
            smooth_m: d_smooth(),
        }
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear height at layer metres (`u` east, `v` north), clamped to the field.
fn sample(h: &Heightfield, spacing_m: [f32; 2], u: f32, v: f32) -> f32 {
    let (w, hh) = (h.width() as f32, h.height() as f32);
    let x = (u / spacing_m[0] - 0.5).clamp(0.0, w - 1.0);
    let y = (v / spacing_m[1] - 0.5).clamp(0.0, hh - 1.0);
    let (x0, y0) = (x.floor(), y.floor());
    let (x1, y1) = ((x0 + 1.0).min(w - 1.0), (y0 + 1.0).min(hh - 1.0));
    let (fx, fy) = (x - x0, y - y0);
    let g = |xx: f32, yy: f32| h.get(xx as u32, yy as u32);
    (g(x0, y0) * (1.0 - fx) + g(x1, y0) * fx) * (1.0 - fy)
        + (g(x0, y1) * (1.0 - fx) + g(x1, y1) * fx) * fy
}

/// Reshape `h` along the spline. Returns the terrain and the corridor mask (1 inside the width,
/// fading to 0 across the shoulder).
pub fn apply_spline(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &SplineParams,
) -> JdResult<(Heightfield, Heightfield)> {
    if p.points.len() < 2
        || p.points
            .iter()
            .any(|q| !q[0].is_finite() || !q[1].is_finite())
    {
        return Err(JdError::invalid(
            "points",
            "need at least two finite points",
        ));
    }
    if !(p.width_m > 0.0 && p.width_m.is_finite()) {
        return Err(JdError::invalid("width_m", "must be positive"));
    }
    for (name, v) in [
        ("falloff_m", p.falloff_m),
        ("profile_m", p.profile_m),
        ("smooth_m", p.smooth_m),
    ] {
        if !(v >= 0.0 && v.is_finite()) {
            return Err(JdError::invalid(name, "must be finite and ≥ 0"));
        }
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    // Sample the polyline every half cell; remember the distance along it for smoothing.
    let step = spacing_m[0].min(spacing_m[1]) * 0.5;
    let mut samples: Vec<([f32; 2], f32)> = Vec::new(); // (position, distance along)
    let mut along = 0.0f32;
    for seg in p.points.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let n = (len / step).ceil().max(1.0) as usize;
        for k in 0..n {
            let t = k as f32 / n as f32;
            samples.push((
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                along + len * t,
            ));
        }
        along += len;
    }
    samples.push((p.points[p.points.len() - 1], along));
    // The grade: terrain height at each sample, box-smoothed along the line.
    let raw: Vec<f32> = samples
        .iter()
        .map(|(q, _)| sample(h, spacing_m, q[0], q[1]))
        .collect();
    let half = p.smooth_m * 0.5;
    let grade: Vec<f32> = (0..samples.len())
        .map(|i| {
            let d = samples[i].1;
            let (mut sum, mut n) = (0.0f32, 0u32);
            for (j, (_, dj)) in samples.iter().enumerate() {
                if (dj - d).abs() <= half {
                    sum += raw[j];
                    n += 1;
                }
            }
            sum / n.max(1) as f32
        })
        .collect();
    let offset = match p.mode {
        SplineMode::Flatten => 0.0,
        SplineMode::Cut => -p.profile_m,
        SplineMode::Raise => p.profile_m,
    };
    let reach = p.width_m * 0.5 + p.falloff_m;
    let (w, hh) = (h.width(), h.height());
    let mut out = h.clone();
    let mut mask = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    let mut target = vec![f32::NAN; (w * hh) as usize];
    let mut weight = vec![0.0f32; (w * hh) as usize];
    for y in 0..hh {
        for x in 0..w {
            let (u, v) = (
                (x as f32 + 0.5) * spacing_m[0],
                (y as f32 + 0.5) * spacing_m[1],
            );
            // Nearest sample (squared distance), skipping cells clearly out of reach early.
            let mut best = f32::MAX;
            let mut best_i = 0usize;
            for (i, (q, _)) in samples.iter().enumerate() {
                let d2 = (q[0] - u).powi(2) + (q[1] - v).powi(2);
                if d2 < best {
                    best = d2;
                    best_i = i;
                }
            }
            let dist = best.sqrt();
            if dist > reach {
                continue;
            }
            let inside = (dist - p.width_m * 0.5).max(0.0);
            let wgt = if p.falloff_m > 0.0 {
                1.0 - smoothstep(inside / p.falloff_m)
            } else {
                f32::from(inside == 0.0)
            };
            let i = (y * w + x) as usize;
            target[i] = grade[best_i] + offset;
            weight[i] = wgt;
        }
    }
    out.fill_with(|x, y| {
        let i = (y * w + x) as usize;
        let z = h.get(x, y);
        if weight[i] > 0.0 {
            z + (target[i] - z) * weight[i]
        } else {
            z
        }
    });
    mask.fill_with(|x, y| weight[(y * w + x) as usize]);
    Ok((out, mask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_road_across_a_slope_is_level_crosswise_follows_a_smoothed_grade_and_masks_its_corridor() {
        // A 10 m-cell hillside rising 0.3 m per metre to the north with 4 m bumps.
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| y as f32 * 3.0 + if (x + y) % 2 == 0 { 2.0 } else { -2.0 });
        // A road from west to east along the middle: cross-slope is the 0.3 grade, bumps ±2 m.
        let p = SplineParams {
            points: vec![[0.0, 320.0], [640.0, 320.0]],
            width_m: 30.0,
            falloff_m: 20.0,
            smooth_m: 100.0,
            ..SplineParams::default()
        };
        let (out, mask) = apply_spline(&h, [10.0, 10.0], &p).expect("spline");
        // Inside the corridor (|v − 320| ≤ 15 → rows 31 and 32) the bumps are gone and the two rows are level.
        let (r1, r2) = (out.get(20, 31), out.get(20, 32));
        assert!((r1 - r2).abs() < 0.05, "crosswise level: {r1} vs {r2}");
        assert!(
            (out.get(20, 31) - out.get(21, 31)).abs() < 0.05,
            "no bumps along the road"
        );
        assert!(
            (mask.get(20, 31) - 1.0).abs() < 1e-6 && mask.get(20, 40) == 0.0,
            "corridor mask"
        );
        assert!(
            mask.get(20, 34) > 0.0 && mask.get(20, 34) < 1.0,
            "shoulder fades"
        );
        // Untouched far from the road; the road sits on the hillside's own grade (≈ 95 m).
        assert_eq!(out.get(20, 10), h.get(20, 10));
        assert!(
            (out.get(20, 31) - 95.0).abs() < 3.0,
            "grade {}",
            out.get(20, 31)
        );
        // A canal cuts 3 m below, an embankment rises 3 m above that grade.
        let cut = apply_spline(
            &h,
            [10.0, 10.0],
            &SplineParams {
                mode: SplineMode::Cut,
                ..p.clone()
            },
        )
        .expect("cut")
        .0;
        let raise = apply_spline(
            &h,
            [10.0, 10.0],
            &SplineParams {
                mode: SplineMode::Raise,
                ..p.clone()
            },
        )
        .expect("raise")
        .0;
        assert!((out.get(20, 31) - cut.get(20, 31) - 3.0).abs() < 0.05);
        assert!((raise.get(20, 31) - out.get(20, 31) - 3.0).abs() < 0.05);
        assert!(
            apply_spline(
                &h,
                [10.0, 10.0],
                &SplineParams {
                    points: vec![[0.0, 0.0]],
                    ..p.clone()
                }
            )
            .is_err()
        );
        assert!(apply_spline(&h, [10.0, 10.0], &SplineParams { width_m: 0.0, ..p }).is_err());
    }
}
