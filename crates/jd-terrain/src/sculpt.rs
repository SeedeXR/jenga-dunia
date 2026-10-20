//! Stroke sculpting: brushes recorded as data and replayed onto a field (`02-subsystems.md` §4).
//! A stroke is a polyline in layer metres (`u` east from the west edge, `v` north from the south
//! edge, i.e. row 0 — the field convention), a radius and a signed strength. Height changes by `strength · falloff(d/r)`
//! where `d` is the distance to the polyline, so strokes are order-independent and undo removes a
//! whole stroke by dropping it from the node's params.
//!
//! Each pixel only tests the strokes whose bounding box (polyline ± radius) contains it; the
//! smooth brush blurs via a summed-area table, so cost is O(pixels) regardless of brush size.
//!
//! ponytail: the blurred copy still covers the whole field; tile-level dirty sets when strokes
//! count in the thousands.

use crate::Heightfield;
use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

/// What a stroke does to the heights under the brush.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StrokeMode {
    /// Add `strength_m` (negative digs).
    #[default]
    Raise,
    /// Blend towards the local mean (box filter ~ radius/4) by `amount`.
    Smooth,
    /// Blend towards the height under the first point by `amount`.
    Flatten,
}

/// One brush stroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    /// Polyline in layer metres: `[u east, v north]` from the south-west corner (row 0 = south).
    pub points: Vec<[f32; 2]>,
    /// Brush radius in metres.
    pub radius_m: f32,
    /// Height added at the centre for `raise` (negative lowers), metres.
    #[serde(default)]
    pub strength_m: f32,
    /// Brush behaviour (default `raise`).
    #[serde(default)]
    pub mode: StrokeMode,
    /// Blend factor 0..1 for `smooth` / `flatten` (default 0.5).
    #[serde(default = "half")]
    pub amount: f32,
}
fn half() -> f32 {
    0.5
}

/// Box-filtered copy of `h` (radius in pixels, edge-clamped) for smooth strokes.
pub(crate) fn box_mean(h: &Heightfield, rx: u32, ry: u32) -> Heightfield {
    let (w, hh) = (h.width() as usize, h.height() as usize);
    // Summed-area table in f64 (one extra zero row/column) so any box is four lookups.
    let v = h.to_vec();
    let stride = w + 1;
    let mut sat = vec![0.0f64; (w + 1) * (hh + 1)];
    for y in 0..hh {
        let mut row = 0.0f64;
        for x in 0..w {
            row += f64::from(v[y * w + x]);
            sat[(y + 1) * stride + x + 1] = sat[y * stride + x + 1] + row;
        }
    }
    let mut out = h.clone();
    out.fill_with(|x, y| {
        let (x, y) = (x as usize, y as usize);
        let (x0, x1) = (
            x.saturating_sub(rx as usize),
            (x + rx as usize).min(w - 1) + 1,
        );
        let (y0, y1) = (
            y.saturating_sub(ry as usize),
            (y + ry as usize).min(hh - 1) + 1,
        );
        let sum = sat[y1 * stride + x1] - sat[y0 * stride + x1] - sat[y1 * stride + x0]
            + sat[y0 * stride + x0];
        (sum / ((x1 - x0) * (y1 - y0)) as f64) as f32
    });
    out
}

fn dist_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
    let len2 = abx * abx + aby * aby;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a[0] + t * abx, a[1] + t * aby);
    (p[0] - cx).hypot(p[1] - cy)
}

/// Apply strokes to a copy of `h`; `spacing_m` is the (x, y) sample spacing.
pub fn apply_strokes(
    h: &Heightfield,
    spacing_m: [f32; 2],
    strokes: &[Stroke],
) -> JdResult<Heightfield> {
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    for (i, s) in strokes.iter().enumerate() {
        if s.points.is_empty()
            || !(s.radius_m > 0.0 && s.radius_m.is_finite())
            || !s.strength_m.is_finite()
        {
            return Err(JdError::invalid(
                "strokes",
                format!("stroke {i}: needs ≥ 1 point, a positive radius and a finite strength"),
            ));
        }
        if s.points.iter().flatten().any(|c| !c.is_finite()) {
            return Err(JdError::invalid(
                "strokes",
                format!("stroke {i}: non-finite point"),
            ));
        }
    }
    let mut out = h.clone();
    if strokes.is_empty() {
        return Ok(out);
    }
    for (i, s) in strokes.iter().enumerate() {
        if s.mode != StrokeMode::Raise && !(0.0..=1.0).contains(&s.amount) {
            return Err(JdError::invalid(
                "strokes",
                format!("stroke {i}: amount must be within 0..=1"),
            ));
        }
    }
    // Smooth strokes blend towards a blurred copy of the input (radius ≈ brush/4, ≥ 1 px).
    let smoothed = strokes
        .iter()
        .filter(|s| s.mode == StrokeMode::Smooth)
        .map(|s| s.radius_m)
        .fold(None, |m: Option<f32>, r| Some(m.map_or(r, |x| x.max(r))))
        .map(|r| {
            // Radius/4 in metres along each axis, so the box is round in metres, not in pixels.
            let r4 = |sp: f32| ((r / sp) / 4.0).round().max(1.0) as u32;
            box_mean(h, r4(spacing_m[0]), r4(spacing_m[1]))
        });
    let sample = |p: [f32; 2]| -> f32 {
        let x = ((p[0] / spacing_m[0]) as i64).clamp(0, h.width() as i64 - 1) as u32;
        let y = ((p[1] / spacing_m[1]) as i64).clamp(0, h.height() as i64 - 1) as u32;
        h.get(x, y)
    };
    let targets: Vec<f32> = strokes
        .iter()
        .map(|s| {
            if s.mode == StrokeMode::Flatten {
                sample(s.points[0])
            } else {
                0.0
            }
        })
        .collect();
    // Outside polyline ± radius the falloff is exactly 0, so those pixels can skip the stroke.
    let bboxes: Vec<[f32; 4]> = strokes
        .iter()
        .map(|s| {
            s.points
                .iter()
                .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| {
                    [
                        b[0].min(p[0] - s.radius_m),
                        b[1].min(p[1] - s.radius_m),
                        b[2].max(p[0] + s.radius_m),
                        b[3].max(p[1] + s.radius_m),
                    ]
                })
        })
        .collect();
    out.fill_with(|x, y| {
        let p = [
            (x as f32 + 0.5) * spacing_m[0],
            (y as f32 + 0.5) * spacing_m[1],
        ];
        let mut v = h.get(x, y);
        for ((s, target), b) in strokes.iter().zip(&targets).zip(&bboxes) {
            if p[0] < b[0] || p[1] < b[1] || p[0] > b[2] || p[1] > b[3] {
                continue;
            }
            let d = s
                .points
                .windows(2)
                .map(|w| dist_to_segment(p, w[0], w[1]))
                .chain(std::iter::once(dist_to_segment(
                    p,
                    s.points[0],
                    s.points[0],
                )))
                .fold(f32::MAX, f32::min);
            let t = (d / s.radius_m).min(1.0);
            // Smooth falloff: 1 at the centre, 0 at the radius, zero slope at both ends.
            let w = 1.0 - t * t * (3.0 - 2.0 * t);
            match s.mode {
                StrokeMode::Raise => v += s.strength_m * w,
                StrokeMode::Smooth => {
                    if let Some(sm) = &smoothed {
                        v += (sm.get(x, y) - v) * s.amount * w;
                    }
                }
                StrokeMode::Flatten => v += (target - v) * s.amount * w,
            }
        }
        v
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_mean_matches_brute_force_and_bbox_skip_leaves_far_pixels_untouched() {
        let mut h = Heightfield::new(37, 23, 64, 0).expect("hf");
        h.fill_with(|x, y| ((x * 7919 + y * 104729) % 1000) as f32 - 500.0);
        let fast = box_mean(&h, 3, 2);
        for y in 0..23u32 {
            for x in 0..37u32 {
                let (x0, x1) = (x.saturating_sub(3), (x + 3).min(36));
                let (y0, y1) = (y.saturating_sub(2), (y + 2).min(22));
                let mut sum = 0.0f64;
                for yy in y0..=y1 {
                    for xx in x0..=x1 {
                        sum += f64::from(h.get(xx, yy));
                    }
                }
                let brute = (sum / f64::from((x1 - x0 + 1) * (y1 - y0 + 1))) as f32;
                assert!(
                    (fast.get(x, y) - brute).abs() < 1e-3,
                    "({x},{y}) {} vs {brute}",
                    fast.get(x, y)
                );
            }
        }
        // A raise stroke in one corner leaves the opposite corner bit-identical.
        let out = apply_strokes(
            &h,
            [1.0, 1.0],
            &[Stroke {
                points: vec![[3.0, 3.0], [6.0, 4.0]],
                radius_m: 4.0,
                strength_m: 10.0,
                mode: StrokeMode::Raise,
                amount: 0.0,
            }],
        )
        .expect("strokes");
        assert!(out.get(4, 3) > h.get(4, 3));
        assert_eq!(out.get(30, 20), h.get(30, 20));
        assert_eq!(out.get(11, 3), h.get(11, 3), "just outside the bbox");
    }

    #[test]
    fn flatten_and_smooth_pull_towards_target_and_local_mean() {
        let mut h = Heightfield::new(64, 64, 32, 4).expect("hf");
        h.fill_with(|x, y| if (x + y) % 2 == 0 { 100.0 } else { 0.0 }); // checkerboard: mean 50
        let flat = apply_strokes(
            &h,
            [1.0, 1.0],
            &[Stroke {
                points: vec![[32.5, 32.5]],
                radius_m: 12.0,
                strength_m: 0.0,
                mode: StrokeMode::Flatten,
                amount: 1.0,
            }],
        )
        .expect("flatten");
        let target = h.get(32, 32);
        assert_eq!(flat.get(32, 32), target);
        // One pixel off-centre the falloff is ≈ 0.98, so the neighbour lands just under the target.
        assert!(
            flat.get(33, 32) > target - 5.0 && flat.get(33, 32) <= target,
            "neighbour pulled to the target: {}",
            flat.get(33, 32)
        );
        assert_eq!(flat.get(5, 5), h.get(5, 5), "outside the radius untouched");
        let smooth = apply_strokes(
            &h,
            [1.0, 1.0],
            &[Stroke {
                points: vec![[32.5, 32.5]],
                radius_m: 12.0,
                strength_m: 0.0,
                mode: StrokeMode::Smooth,
                amount: 1.0,
            }],
        )
        .expect("smooth");
        for (x, y) in [(32u32, 32u32), (33, 32), (31, 33)] {
            assert!(
                (smooth.get(x, y) - 50.0).abs() < 12.0,
                "checkerboard smoothed towards its mean: {}",
                smooth.get(x, y)
            );
        }
        assert_eq!(smooth.get(5, 5), h.get(5, 5));
        assert!(
            apply_strokes(
                &h,
                [1.0, 1.0],
                &[Stroke {
                    points: vec![[1.0, 1.0]],
                    radius_m: 2.0,
                    strength_m: 0.0,
                    mode: StrokeMode::Smooth,
                    amount: 1.5
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn smooth_box_is_round_in_metres_under_anisotropic_spacing() {
        // 2 m × 0.5 m pixels: a 16 m brush smooths over ±2 px in x and ±8 px in y.
        let mut h = Heightfield::new(64, 64, 32, 4).expect("hf");
        h.fill_with(|x, y| if (x, y) == (32, 32) { 100.0 } else { 0.0 }); // single spike
        let s = Stroke {
            points: vec![[65.0, 16.25]],
            radius_m: 16.0,
            strength_m: 0.0,
            mode: StrokeMode::Smooth,
            amount: 1.0,
        };
        let out = apply_strokes(&h, [2.0, 0.5], &[s]).expect("smooth");
        // The spike's mass spreads over a (2·2+1)×(2·8+1) = 85-cell box: ~1.18 m each at the centre.
        assert!(
            (out.get(32, 32) - 100.0 / 85.0).abs() < 0.5,
            "centre after smoothing: {}",
            out.get(32, 32)
        );
        assert!(
            out.get(34, 32) > 0.5 && out.get(35, 32) == 0.0,
            "x reach is 2 px: {} / {}",
            out.get(34, 32),
            out.get(35, 32)
        );
        assert!(
            out.get(32, 40) > 0.5 && out.get(32, 41) == 0.0,
            "y reach is 8 px: {} / {}",
            out.get(32, 40),
            out.get(32, 41)
        );
    }

    #[test]
    fn stroke_raises_along_its_path_and_nothing_beyond_the_radius() {
        let flat = Heightfield::new(64, 64, 32, 4).expect("hf");
        let stroke = Stroke {
            points: vec![[10.0, 32.0], [50.0, 32.0]],
            radius_m: 6.0,
            strength_m: 20.0,
            mode: StrokeMode::Raise,
            amount: 0.5,
        };
        let out = apply_strokes(&flat, [1.0, 1.0], &[stroke]).expect("sculpt");
        // Pixel centres sit 0.5 m off the line, so the crest is just under full strength.
        assert!(
            out.get(30, 32) > 19.0 && out.get(30, 32) <= 20.0,
            "crest: {}",
            out.get(30, 32)
        );
        assert!(
            out.get(10, 32) > 18.5 && out.get(10, 32) <= 20.0,
            "endpoint near full strength: {}",
            out.get(10, 32)
        );
        assert!(
            out.get(30, 35) > 5.0 && out.get(30, 35) < 19.0,
            "falloff inside the radius: {}",
            out.get(30, 35)
        );
        assert_eq!(out.get(30, 45), 0.0, "beyond the radius untouched");
        assert_eq!(out.get(2, 2), 0.0);
        let stats = out.stats();
        assert_eq!(stats.non_finite, 0);
        assert!(stats.max <= 20.0 + 1e-4);
    }

    #[test]
    fn strokes_are_additive_and_empty_list_is_identity() {
        let mut h = Heightfield::new(16, 16, 16, 0).expect("hf");
        h.fill_with(|x, _| x as f32);
        assert_eq!(
            apply_strokes(&h, [2.0, 2.0], &[])
                .expect("id")
                .content_hash(),
            h.content_hash()
        );
        let dig = Stroke {
            points: vec![[16.0, 16.0]],
            radius_m: 8.0,
            strength_m: -4.0,
            mode: StrokeMode::Raise,
            amount: 0.5,
        };
        let out = apply_strokes(&h, [2.0, 2.0], &[dig.clone(), dig.clone()]).expect("sculpt");
        let single = apply_strokes(&h, [2.0, 2.0], &[dig]).expect("sculpt");
        let (d1, d2) = (h.get(7, 7) - single.get(7, 7), h.get(7, 7) - out.get(7, 7));
        assert!(d1 > 3.0 && d1 <= 4.0, "one dig near the centre: {d1}");
        assert!(
            (d2 - 2.0 * d1).abs() < 1e-4,
            "two identical digs stack linearly: {d2} vs {d1}"
        );
        assert!(
            apply_strokes(
                &h,
                [2.0, 2.0],
                &[Stroke {
                    points: vec![],
                    radius_m: 1.0,
                    strength_m: 1.0,
                    mode: StrokeMode::Raise,
                    amount: 0.5
                }]
            )
            .is_err()
        );
        assert!(
            apply_strokes(
                &h,
                [2.0, 2.0],
                &[Stroke {
                    points: vec![[0.0, 0.0]],
                    radius_m: 0.0,
                    strength_m: 1.0,
                    mode: StrokeMode::Raise,
                    amount: 0.5
                }]
            )
            .is_err()
        );
    }
}
