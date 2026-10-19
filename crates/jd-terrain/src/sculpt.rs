//! Stroke sculpting: brushes recorded as data and replayed onto a field (`02-subsystems.md` §4).
//! A stroke is a polyline in layer metres (`u` east from the west edge, `v` south from the north
//! edge, i.e. row 0), a radius and a signed strength. Height changes by `strength · falloff(d/r)`
//! where `d` is the distance to the polyline, so strokes are order-independent and undo removes a
//! whole stroke by dropping it from the node's params.
//!
//! ponytail: every pixel tests every segment (pixels × segments); per-stroke bounding boxes and
//! tile-level dirty sets when strokes count in the thousands.

use crate::Heightfield;
use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

/// One brush stroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    /// Polyline in layer metres: `[u east, v south]` from the north-west corner.
    pub points: Vec<[f32; 2]>,
    /// Brush radius in metres.
    pub radius_m: f32,
    /// Height added at the centre (negative lowers), metres.
    pub strength_m: f32,
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
    out.fill_with(|x, y| {
        let p = [
            (x as f32 + 0.5) * spacing_m[0],
            (y as f32 + 0.5) * spacing_m[1],
        ];
        let mut v = h.get(x, y);
        for s in strokes {
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
            v += s.strength_m * w;
        }
        v
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stroke_raises_along_its_path_and_nothing_beyond_the_radius() {
        let flat = Heightfield::new(64, 64, 32, 4).expect("hf");
        let stroke = Stroke {
            points: vec![[10.0, 32.0], [50.0, 32.0]],
            radius_m: 6.0,
            strength_m: 20.0,
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
                    strength_m: 1.0
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
                    strength_m: 1.0
                }]
            )
            .is_err()
        );
    }
}
