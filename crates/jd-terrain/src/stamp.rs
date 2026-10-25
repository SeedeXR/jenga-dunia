//! Placed shapes: a stamp (a heightmap file set down at a position, size and rotation) and
//! primitives (cone, dome, plateau, ramp, crater) — World Creator's landscape, stamp and
//! primitive layers, `docs/research/world-creator.md` §3.2. The shape is evaluated in its own
//! frame, 0..1 across its footprint, scaled to `height_range_m`, faded over `falloff_m` at its
//! edge, and combined with the input by `op` at `opacity`: add, max (raise onto the terrain),
//! min (carve into it) or lerp (overwrite). Everything outside the footprint is untouched.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;

/// How a placed shape meets the terrain.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PlaceOp {
    /// Add the shape's height to the terrain.
    #[default]
    Add,
    /// Keep the higher: the shape rises out of the terrain.
    Max,
    /// Keep the lower: the shape is carved into the terrain.
    Min,
    /// Replace the terrain with the shape's height.
    Lerp,
}

/// Where and how a shape is placed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// Centre in layer metres: `[u east, v north]` from the south-west corner.
    pub center_m: [f32; 2],
    /// Footprint in metres (width along the shape's x, depth along its y), both > 0.
    pub size_m: [f32; 2],
    /// Rotation about the centre, degrees clockwise from north (0 = the shape's y points north).
    #[serde(default)]
    pub rotation_deg: f32,
    /// Heights the shape's 0..1 maps to, metres: `[low, high]`.
    pub height_range_m: [f32; 2],
    /// Soft edge inside the footprint, metres (≥ 0): the shape's effect fades to nothing over it.
    #[serde(default)]
    pub falloff_m: f32,
    /// Combination with the terrain (default `add`).
    #[serde(default)]
    pub op: PlaceOp,
    /// Opacity 0..1 of the whole effect.
    #[serde(default = "one")]
    pub opacity: f32,
}
fn one() -> f32 {
    1.0
}

impl Placement {
    fn validate(&self) -> JdResult<()> {
        if !self.center_m.iter().all(|c| c.is_finite()) {
            return Err(JdError::invalid("center_m", "must be finite"));
        }
        if !self.size_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
            return Err(JdError::invalid("size_m", "must be positive"));
        }
        if !self.height_range_m.iter().all(|h| h.is_finite()) || !self.rotation_deg.is_finite() {
            return Err(JdError::invalid("height_range_m", "must be finite"));
        }
        if !(self.falloff_m >= 0.0 && self.falloff_m.is_finite()) {
            return Err(JdError::invalid("falloff_m", "must be ≥ 0"));
        }
        if !(0.0..=1.0).contains(&self.opacity) {
            return Err(JdError::invalid("opacity", "must be within 0..=1"));
        }
        Ok(())
    }
}

/// The primitive shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Primitive {
    /// Linear cone: 1 at the centre, 0 at the edge.
    Cone,
    /// Rounded dome (half a cosine).
    Dome,
    /// Flat top of `plateau` × radius, then a cosine slope to the edge.
    Plateau,
    /// Ramp from 0 at the south edge to 1 at the north edge (rotate for other directions).
    Ramp,
    /// Crater: a raised rim at `rim` × radius, a bowl down to `floor` inside, fading outside.
    Crater,
}

/// Primitive settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveParams {
    /// Which shape.
    pub shape: Primitive,
    /// Placement.
    #[serde(flatten)]
    pub place: Placement,
    /// Plateau: share of the radius that is flat (0..1). Crater: share of the radius where the rim peaks.
    #[serde(default = "d_ratio")]
    pub ratio: f32,
    /// Crater floor height as a share of the rim height (−1..1; negative digs below the base).
    #[serde(default = "d_floor")]
    pub floor: f32,
}
fn d_ratio() -> f32 {
    0.6
}
fn d_floor() -> f32 {
    -0.4
}

/// Shape value 0..1 of a primitive at local coordinates `(x, y)` in −1..1 (edge of the footprint
/// at radius 1), or `None` outside the footprint.
fn primitive_at(p: &PrimitiveParams, x: f32, y: f32) -> Option<f32> {
    let r = x.hypot(y);
    match p.shape {
        Primitive::Ramp => (x.abs() <= 1.0 && y.abs() <= 1.0).then_some((y + 1.0) * 0.5),
        _ if r > 1.0 => None,
        Primitive::Cone => Some(1.0 - r),
        Primitive::Dome => Some(0.5 + 0.5 * (std::f32::consts::PI * r).cos()),
        Primitive::Plateau => {
            let flat = p.ratio.clamp(0.0, 0.999);
            Some(if r <= flat {
                1.0
            } else {
                0.5 + 0.5 * (std::f32::consts::PI * (r - flat) / (1.0 - flat)).cos()
            })
        }
        Primitive::Crater => {
            let rim = p.ratio.clamp(0.05, 0.95);
            Some(if r <= rim {
                // Bowl: the floor at the centre, the rim at `rim`.
                let t = r / rim;
                p.floor + (1.0 - p.floor) * t * t
            } else {
                // Ejecta blanket fading from the rim to the edge.
                let t = (r - rim) / (1.0 - rim);
                (1.0 - t) * (1.0 - t)
            })
        }
    }
}

/// Place a shape given by `sample(x, y) -> Option<0..1>` in local −1..1 coordinates.
fn place_with(
    h: &Heightfield,
    spacing_m: [f32; 2],
    place: &Placement,
    boxed: bool,
    sample: &(dyn Fn(f32, f32) -> Option<f32> + Sync),
) -> JdResult<Heightfield> {
    place.validate()?;
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let (cu, cv) = (place.center_m[0], place.center_m[1]);
    let (hw, hd) = (place.size_m[0] * 0.5, place.size_m[1] * 0.5);
    let rot = place.rotation_deg.to_radians();
    let (sr, cr) = (rot.sin(), rot.cos());
    let [lo, hi] = place.height_range_m;
    let fall = place.falloff_m;
    let mut out = h.clone();
    out.fill_with(|x, y| {
        let v0 = h.get(x, y);
        let (u, v) = (
            (x as f32 + 0.5) * spacing_m[0] - cu,
            (y as f32 + 0.5) * spacing_m[1] - cv,
        );
        // Into the shape's frame: undo a clockwise rotation (clockwise from north = towards east).
        let (lx, ly) = (u * cr - v * sr, u * sr + v * cr);
        let (nx, ny) = (lx / hw, ly / hd);
        let Some(s) = sample(nx, ny) else { return v0 };
        // Edge fade: distance to the footprint's edge in metres (the inscribed circle for radial
        // shapes, the nearest side for boxed ones).
        let edge_m = if boxed {
            ((1.0 - nx.abs()) * hw).min((1.0 - ny.abs()) * hd)
        } else {
            (1.0 - nx.hypot(ny)) * hw.min(hd)
        };
        let w = if fall > 0.0 {
            (edge_m / fall).clamp(0.0, 1.0)
        } else {
            1.0
        } * place.opacity;
        let target = lo + (hi - lo) * s;
        let blended = match place.op {
            PlaceOp::Add => v0 + target,
            PlaceOp::Max => v0.max(target),
            PlaceOp::Min => v0.min(target),
            PlaceOp::Lerp => target,
        };
        v0 + (blended - v0) * w
    });
    Ok(out)
}

/// A polygon layer: a closed outline in layer metres set to one height.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolygonParams {
    /// Corners in layer metres `[u east, v north]`, in order (≥ 3, any winding; the last joins the first).
    pub points: Vec<[f32; 2]>,
    /// The height the polygon takes, metres (what `op` combines with the terrain).
    pub height_m: f32,
    /// Soft edge outside the outline, metres (≥ 0): the effect fades to nothing over it.
    #[serde(default)]
    pub falloff_m: f32,
    /// Combination with the terrain (default `lerp`: a flattened pad; `add` lifts, `max` / `min` clip).
    #[serde(default = "polygon_op")]
    pub op: PlaceOp,
    /// Opacity 0..1 of the whole effect.
    #[serde(default = "one")]
    pub opacity: f32,
}
fn polygon_op() -> PlaceOp {
    PlaceOp::Lerp
}

/// Even-odd point-in-polygon and the distance to the nearest edge, metres.
fn polygon_inside_and_distance(points: &[[f32; 2]], u: f32, v: f32) -> (bool, f32) {
    let mut inside = false;
    let mut best = f32::INFINITY;
    let n = points.len();
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        if (a[1] > v) != (b[1] > v) {
            let x = a[0] + (v - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if u < x {
                inside = !inside;
            }
        }
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((u - a[0]) * dx + (v - a[1]) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        best = best.min((u - (a[0] + t * dx)).hypot(v - (a[1] + t * dy)));
    }
    (inside, best)
}

/// Place a polygon on `h`: inside the outline the terrain takes `height_m` through `op` at full
/// `opacity`; outside, the effect fades over `falloff_m` (a flattened building pad, a raised
/// field, a clipped plateau — World Creator's polygon shape layer).
pub fn place_polygon(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &PolygonParams,
) -> JdResult<Heightfield> {
    if p.points.len() < 3 || !p.points.iter().all(|c| c.iter().all(|v| v.is_finite())) {
        return Err(JdError::invalid(
            "points",
            "need at least three finite corners",
        ));
    }
    if !p.height_m.is_finite()
        || !(p.falloff_m >= 0.0 && p.falloff_m.is_finite())
        || !(0.0..=1.0).contains(&p.opacity)
    {
        return Err(JdError::invalid(
            "height_m",
            "height finite, falloff ≥ 0, opacity within 0..=1",
        ));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    // Only cells within the outline's box (+ falloff) need the per-edge test.
    let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for c in &p.points {
        for k in 0..2 {
            lo[k] = lo[k].min(c[k]);
            hi[k] = hi[k].max(c[k]);
        }
    }
    let mut out = h.clone();
    out.fill_with(|x, y| {
        let v0 = h.get(x, y);
        let (u, v) = (
            (x as f32 + 0.5) * spacing_m[0],
            (y as f32 + 0.5) * spacing_m[1],
        );
        if u < lo[0] - p.falloff_m
            || u > hi[0] + p.falloff_m
            || v < lo[1] - p.falloff_m
            || v > hi[1] + p.falloff_m
        {
            return v0;
        }
        let (inside, d) = polygon_inside_and_distance(&p.points, u, v);
        let w = if inside {
            1.0
        } else if p.falloff_m > 0.0 {
            (1.0 - d / p.falloff_m).clamp(0.0, 1.0)
        } else {
            0.0
        } * p.opacity;
        if w <= 0.0 {
            return v0;
        }
        let blended = match p.op {
            PlaceOp::Add => v0 + p.height_m,
            PlaceOp::Max => v0.max(p.height_m),
            PlaceOp::Min => v0.min(p.height_m),
            PlaceOp::Lerp => p.height_m,
        };
        v0 + (blended - v0) * w
    });
    Ok(out)
}

/// Place a primitive on `h`.
pub fn place_primitive(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &PrimitiveParams,
) -> JdResult<Heightfield> {
    if !(0.0..=1.0).contains(&p.ratio) || !(-1.0..=1.0).contains(&p.floor) {
        return Err(JdError::invalid(
            "ratio",
            "ratio must be within 0..=1 and floor within -1..=1",
        ));
    }
    place_with(
        h,
        spacing_m,
        &p.place,
        matches!(p.shape, Primitive::Ramp),
        &|x, y| primitive_at(p, x, y),
    )
}

/// Place a stamp: `data` is a `w × h` grid (row 0 = south) whose values are normalised to 0..1
/// over the stamp's own range, sampled bilinearly across the footprint.
pub fn place_stamp(
    h: &Heightfield,
    spacing_m: [f32; 2],
    place: &Placement,
    data: &[f32],
    w: u32,
    hh: u32,
) -> JdResult<Heightfield> {
    if data.len() != (w * hh) as usize || w < 2 || hh < 2 {
        return Err(JdError::invalid(
            "stamp",
            "needs a grid of at least 2 × 2 finite samples",
        ));
    }
    let (lo, hi) = data
        .iter()
        .filter(|v| v.is_finite())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| {
            (a.min(*v), b.max(*v))
        });
    if !(lo.is_finite() && hi > lo) {
        return Err(JdError::invalid("stamp", "needs varying finite samples"));
    }
    let norm = |v: f32| {
        if v.is_finite() {
            (v - lo) / (hi - lo)
        } else {
            0.0
        }
    };
    let sample = move |x: f32, y: f32| -> Option<f32> {
        if x.abs() > 1.0 || y.abs() > 1.0 {
            return None;
        }
        let (fx, fy) = (
            (x + 1.0) * 0.5 * (w - 1) as f32,
            (y + 1.0) * 0.5 * (hh - 1) as f32,
        );
        let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(hh - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |xx: u32, yy: u32| norm(data[(yy * w + xx) as usize]);
        let a = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
        let b = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
        Some(a + (b - a) * ty)
    };
    place_with(h, spacing_m, place, true, &sample)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_polygon_flattens_its_inside_and_fades_outside() {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, _| x as f32); // a ramp east
        let pad = PolygonParams {
            points: vec![
                [200.0, 200.0],
                [400.0, 200.0],
                [400.0, 400.0],
                [200.0, 400.0],
            ],
            height_m: 100.0,
            falloff_m: 50.0,
            op: PlaceOp::Lerp,
            opacity: 1.0,
        };
        let out = place_polygon(&h, [10.0, 10.0], &pad).expect("pad");
        assert_eq!(out.get(30, 30), 100.0, "inside: the pad height");
        assert_eq!(out.get(5, 5), h.get(5, 5), "far outside: untouched");
        // 25 m outside the east edge (u = 425): half-way through the falloff.
        let mid = out.get(42, 30);
        assert!(
            (mid - (42.0 + (100.0 - 42.0) * 0.5)).abs() < 1.0,
            "fade: {mid}"
        );
        let lifted = place_polygon(
            &h,
            [10.0, 10.0],
            &PolygonParams {
                op: PlaceOp::Add,
                falloff_m: 0.0,
                ..pad.clone()
            },
        )
        .expect("add");
        assert_eq!(lifted.get(30, 30), 130.0);
        assert_eq!(lifted.get(42, 30), 42.0, "no falloff: a hard edge");
        assert!(
            place_polygon(
                &h,
                [10.0, 10.0],
                &PolygonParams {
                    points: vec![[0.0, 0.0], [1.0, 1.0]],
                    ..pad
                }
            )
            .is_err()
        );
    }

    fn flat() -> Heightfield {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|_, _| 100.0);
        h
    }
    fn place(op: PlaceOp) -> Placement {
        Placement {
            center_m: [325.0, 325.0], // the centre of cell (32, 32) at 10 m cells
            size_m: [200.0, 200.0],
            rotation_deg: 0.0,
            height_range_m: [0.0, 50.0],
            falloff_m: 0.0,
            op,
            opacity: 1.0,
        }
    }

    #[test]
    fn primitives_rise_carve_and_stay_inside_their_footprint() {
        let h = flat();
        let cone = place_primitive(
            &h,
            [10.0, 10.0],
            &PrimitiveParams {
                shape: Primitive::Cone,
                place: place(PlaceOp::Add),
                ratio: 0.6,
                floor: -0.4,
            },
        )
        .expect("cone");
        assert!(
            (cone.get(32, 32) - 150.0).abs() < 0.5,
            "centre +50: {}",
            cone.get(32, 32)
        );
        assert_eq!(cone.get(5, 5), 100.0, "outside the footprint untouched");
        assert!(
            (cone.get(37, 32) - 125.0).abs() < 1.0,
            "half-way out the cone is half height: {}",
            cone.get(37, 32)
        );
        // Min carves: a crater's floor is below the base, its rim above the floor.
        let crater = place_primitive(
            &h,
            [10.0, 10.0],
            &PrimitiveParams {
                shape: Primitive::Crater,
                place: Placement {
                    height_range_m: [100.0, 140.0],
                    ..place(PlaceOp::Lerp)
                },
                ratio: 0.6,
                floor: -0.5,
            },
        )
        .expect("crater");
        assert!(
            crater.get(32, 32) < 100.0,
            "the floor digs below the base: {}",
            crater.get(32, 32)
        );
        let rim = crater.get(32 + 6, 32);
        assert!(
            rim > crater.get(32, 32) && rim > 120.0,
            "the rim stands up: {rim}"
        );
        // A ramp rotated 90° climbs eastwards.
        let ramp = place_primitive(
            &h,
            [10.0, 10.0],
            &PrimitiveParams {
                shape: Primitive::Ramp,
                place: Placement {
                    rotation_deg: 90.0,
                    ..place(PlaceOp::Add)
                },
                ratio: 0.6,
                floor: 0.0,
            },
        )
        .expect("ramp");
        assert!(
            ramp.get(40, 32) > ramp.get(24, 32) + 20.0,
            "east is higher than west: {} vs {}",
            ramp.get(40, 32),
            ramp.get(24, 32)
        );
        assert!(
            (ramp.get(32, 40) - ramp.get(32, 24)).abs() < 1.0,
            "no change along north"
        );
        // Falloff softens the edge; opacity scales everything; bad params refused.
        let soft = place_primitive(
            &h,
            [10.0, 10.0],
            &PrimitiveParams {
                shape: Primitive::Plateau,
                place: Placement {
                    falloff_m: 60.0,
                    opacity: 0.5,
                    height_range_m: [100.0, 150.0],
                    ..place(PlaceOp::Max)
                },
                ratio: 0.6,
                floor: 0.0,
            },
        )
        .expect("plateau");
        assert!(
            (soft.get(32, 32) - 125.0).abs() < 0.5,
            "max to 150 at half opacity: {}",
            soft.get(32, 32)
        );
        assert!(
            soft.get(41, 32) < soft.get(32, 32) && soft.get(41, 32) > 100.0,
            "faded near the edge"
        );
        assert!(
            place_primitive(
                &h,
                [10.0, 10.0],
                &PrimitiveParams {
                    shape: Primitive::Cone,
                    place: Placement {
                        size_m: [0.0, 1.0],
                        ..place(PlaceOp::Add)
                    },
                    ratio: 0.6,
                    floor: 0.0
                }
            )
            .is_err()
        );
    }

    #[test]
    fn a_stamp_is_normalised_scaled_rotated_and_sampled_bilinearly() {
        let h = flat();
        // A 3 × 3 stamp: a single peak in the middle, 7..11 → 0..1.
        let data = [7.0, 7.0, 7.0, 7.0, 11.0, 7.0, 7.0, 7.0, 7.0];
        let out = place_stamp(&h, [10.0, 10.0], &place(PlaceOp::Add), &data, 3, 3).expect("stamp");
        assert!(
            (out.get(32, 32) - 150.0).abs() < 2.0,
            "peak +50: {}",
            out.get(32, 32)
        );
        assert!(
            out.get(27, 32) > 100.0 && out.get(27, 32) < 150.0,
            "bilinear slope: {}",
            out.get(27, 32)
        );
        assert_eq!(out.get(5, 5), 100.0);
        assert!(
            place_stamp(&h, [10.0, 10.0], &place(PlaceOp::Add), &[1.0; 9], 3, 3).is_err(),
            "a flat stamp has no range"
        );
    }
}
