//! Region transforms: World Creator's brush and grid tools that move terrain rather than add to it —
//! Rotate, Scale, Tilt, Push / Pull, Align and Duplicate. Each op is data (like a stroke) replayed in
//! order onto the field: a region (round with a soft edge, or square for grid selections) in layer
//! metres (`u` east from the west edge, `v` north from the south edge), and what happens inside it.
//! Resampling is bilinear and clamps at the border; the weight fades to 0 over `falloff` (a share of
//! the radius), so every op blends into the untouched terrain.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;

/// What a region transform does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransformKind {
    /// Turn the terrain inside the region about its centre by `angle_deg` (counter-clockwise from above).
    Rotate,
    /// Scale the relief about the region's mean height by `factor` (0 levels it, 2 doubles it).
    Scale,
    /// Tilt the region: add a slope of `angle_deg` rising toward `azimuth_deg` (clockwise from north).
    Tilt,
    /// Grab and move: the terrain under the region slides by `vector_m` (fading with the weight).
    Push,
    /// Level the region onto its own best-fit plane (keeps the slope, removes the bumps).
    Align,
    /// Copy the region's terrain to the region shifted by `vector_m`.
    Duplicate,
}

/// The region's footprint.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RegionShape {
    /// A disc of `radius_m` (brush tools).
    #[default]
    Round,
    /// A square of half-width `radius_m` (grid selections).
    Square,
}

/// One region transform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegionTransform {
    /// The op.
    pub kind: TransformKind,
    /// Region centre in layer metres `[u, v]`.
    pub center_m: [f32; 2],
    /// Region radius (round) or half-width (square), metres.
    pub radius_m: f32,
    /// Footprint shape.
    #[serde(default)]
    pub shape: RegionShape,
    /// Share of the radius over which the effect fades out at the edge (0 = hard edge, 1 = all the way in).
    #[serde(default = "d_falloff")]
    pub falloff: f32,
    /// Rotate: degrees counter-clockwise. Tilt: slope angle in degrees.
    #[serde(default)]
    pub angle_deg: f32,
    /// Tilt: direction the slope rises toward, degrees clockwise from north.
    #[serde(default)]
    pub azimuth_deg: f32,
    /// Scale: relief factor about the region's mean.
    #[serde(default = "d_factor")]
    pub factor: f32,
    /// Push / Duplicate: displacement in metres `[east, north]`.
    #[serde(default)]
    pub vector_m: [f32; 2],
    /// Blend of the result over the original, 0..1.
    #[serde(default = "d_strength")]
    pub strength: f32,
}
fn d_falloff() -> f32 {
    0.3
}
fn d_factor() -> f32 {
    1.0
}
fn d_strength() -> f32 {
    1.0
}

impl RegionTransform {
    fn validate(&self, k: usize) -> JdResult<()> {
        let bad = |what: &str| JdError::invalid("ops", format!("op {k}: {what}"));
        if !self
            .center_m
            .iter()
            .chain(self.vector_m.iter())
            .all(|v| v.is_finite())
        {
            return Err(bad("centre and vector must be finite"));
        }
        if !(self.radius_m > 0.0 && self.radius_m.is_finite()) {
            return Err(bad("radius_m must be positive"));
        }
        if !(0.0..=1.0).contains(&self.falloff) || !(0.0..=1.0).contains(&self.strength) {
            return Err(bad("falloff and strength must be within 0..=1"));
        }
        if !self.angle_deg.is_finite()
            || !self.azimuth_deg.is_finite()
            || !(0.0..=10.0).contains(&self.factor)
        {
            return Err(bad("angles finite, factor within 0..=10"));
        }
        if self.kind == TransformKind::Tilt && self.angle_deg.abs() >= 89.0 {
            return Err(bad("a tilt must stay under 89°"));
        }
        Ok(())
    }

    /// Weight 0..1 of the point `(u, v)` relative to the region centred at `c`.
    fn weight(&self, c: [f32; 2], u: f32, v: f32) -> f32 {
        let (du, dv) = (u - c[0], v - c[1]);
        let d = match self.shape {
            RegionShape::Round => du.hypot(dv),
            RegionShape::Square => du.abs().max(dv.abs()),
        } / self.radius_m;
        if d >= 1.0 {
            return 0.0;
        }
        let inner = 1.0 - self.falloff;
        if d <= inner || self.falloff <= 0.0 {
            return 1.0;
        }
        let t = (1.0 - (d - inner) / self.falloff).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// Bilinear sample at layer metres `(u, v)` (cell centres at `(i + 0.5) × spacing`), clamped.
fn sample(h: &Heightfield, sp: [f32; 2], u: f32, v: f32) -> f32 {
    let (w, hh) = (h.width() as f32, h.height() as f32);
    let x = (u / sp[0] - 0.5).clamp(0.0, w - 1.0);
    let y = (v / sp[1] - 0.5).clamp(0.0, hh - 1.0);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(h.width() - 1), (y0 + 1).min(h.height() - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let a = h.get(x0, y0) + (h.get(x1, y0) - h.get(x0, y0)) * fx;
    let b = h.get(x0, y1) + (h.get(x1, y1) - h.get(x0, y1)) * fx;
    a + (b - a) * fy
}

/// Apply the ops in order.
pub fn apply_transforms(
    h: &Heightfield,
    spacing_m: [f32; 2],
    ops: &[RegionTransform],
) -> JdResult<Heightfield> {
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    for (k, op) in ops.iter().enumerate() {
        op.validate(k)?;
    }
    let mut cur = h.clone();
    for op in ops {
        cur = apply_one(&cur, spacing_m, op);
    }
    Ok(cur)
}

fn apply_one(h: &Heightfield, sp: [f32; 2], op: &RegionTransform) -> Heightfield {
    let c = op.center_m;
    let at = |x: u32, y: u32| ((x as f32 + 0.5) * sp[0], (y as f32 + 0.5) * sp[1]);
    // Region statistics the scale and align ops need: weighted mean, and the weighted plane fit.
    let (mut sw, mut sz, mut su, mut sv, mut suu, mut svv, mut suv, mut suz, mut svz) = (
        0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64,
    );
    if matches!(op.kind, TransformKind::Scale | TransformKind::Align) {
        for y in 0..h.height() {
            for x in 0..h.width() {
                let (u, v) = at(x, y);
                let w = f64::from(op.weight(c, u, v));
                if w <= 0.0 {
                    continue;
                }
                let (du, dv, z) = (
                    f64::from(u - c[0]),
                    f64::from(v - c[1]),
                    f64::from(h.get(x, y)),
                );
                sw += w;
                sz += w * z;
                su += w * du;
                sv += w * dv;
                suu += w * du * du;
                svv += w * dv * dv;
                suv += w * du * dv;
                suz += w * du * z;
                svz += w * dv * z;
            }
        }
    }
    let mean = if sw > 0.0 { sz / sw } else { 0.0 };
    // Plane z = a·du + b·dv + c0 by weighted least squares (centred normal equations).
    let plane = (sw > 0.0).then(|| {
        let (mu, mv) = (su / sw, sv / sw);
        let (cuu, cvv, cuv) = (suu / sw - mu * mu, svv / sw - mv * mv, suv / sw - mu * mv);
        let (cuz, cvz) = (suz / sw - mu * mean, svz / sw - mv * mean);
        let det = cuu * cvv - cuv * cuv;
        let (a, b) = if det.abs() > 1e-12 {
            ((cuz * cvv - cvz * cuv) / det, (cvz * cuu - cuz * cuv) / det)
        } else {
            (0.0, 0.0)
        };
        (a, b, mean - a * mu - b * mv)
    });
    let (sin, cos) = (
        op.angle_deg.to_radians().sin(),
        op.angle_deg.to_radians().cos(),
    );
    let slope = op.angle_deg.to_radians().tan();
    let (az_s, az_c) = op.azimuth_deg.to_radians().sin_cos(); // east = sin, north = cos
    let mut out = h.clone();
    out.fill_with(|x, y| {
        let z0 = h.get(x, y);
        let (u, v) = at(x, y);
        // Duplicate weighs the destination region (the source shifted by the vector).
        let w = match op.kind {
            TransformKind::Duplicate => {
                op.weight([c[0] + op.vector_m[0], c[1] + op.vector_m[1]], u, v)
            }
            _ => op.weight(c, u, v),
        } * op.strength;
        if w <= 0.0 {
            return z0;
        }
        let target = match op.kind {
            TransformKind::Rotate => {
                // Sample where this point came from: the inverse rotation about the centre.
                let (du, dv) = (u - c[0], v - c[1]);
                sample(
                    h,
                    sp,
                    c[0] + du * cos + dv * sin,
                    c[1] - du * sin + dv * cos,
                )
            }
            TransformKind::Scale => (mean + f64::from(op.factor) * (f64::from(z0) - mean)) as f32,
            TransformKind::Tilt => z0 + slope * ((u - c[0]) * az_s + (v - c[1]) * az_c),
            TransformKind::Push => {
                return sample(h, sp, u - op.vector_m[0] * w, v - op.vector_m[1] * w);
            }
            TransformKind::Align => match plane {
                Some((a, b, c0)) => (a * f64::from(u - c[0]) + b * f64::from(v - c[1]) + c0) as f32,
                None => z0,
            },
            TransformKind::Duplicate => sample(h, sp, u - op.vector_m[0], v - op.vector_m[1]),
        };
        z0 + (target - z0) * w
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(kind: TransformKind) -> RegionTransform {
        RegionTransform {
            kind,
            center_m: [320.0, 320.0],
            radius_m: 200.0,
            shape: RegionShape::Square,
            falloff: 0.0,
            angle_deg: 0.0,
            azimuth_deg: 0.0,
            factor: 1.0,
            vector_m: [0.0, 0.0],
            strength: 1.0,
        }
    }
    fn ramp() -> Heightfield {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, _| x as f32); // 1 m per 10 m cell, rising east
        h
    }
    const SP: [f32; 2] = [10.0, 10.0];

    #[test]
    fn rotate_half_a_turn_reverses_a_ramp_inside_the_region_only() {
        let h = ramp();
        let r = apply_transforms(
            &h,
            SP,
            &[RegionTransform {
                angle_deg: 180.0,
                ..op(TransformKind::Rotate)
            }],
        )
        .expect("rotate");
        // Cell 22 (u = 225) mirrors to u = 415 → cell 41: height 41.
        assert!((r.get(22, 32) - 41.0).abs() < 0.05, "{}", r.get(22, 32));
        assert_eq!(r.get(2, 32), 2.0, "outside the region untouched");
    }

    #[test]
    fn scale_levels_or_doubles_about_the_region_mean_and_tilt_adds_the_slope() {
        let h = ramp();
        let flat = apply_transforms(
            &h,
            SP,
            &[RegionTransform {
                factor: 0.0,
                ..op(TransformKind::Scale)
            }],
        )
        .expect("scale 0");
        assert!(
            (flat.get(20, 32) - flat.get(40, 32)).abs() < 1e-3,
            "levelled to one height"
        );
        assert!(
            (flat.get(30, 32) - 31.5).abs() < 0.6,
            "at the region's mean: {}",
            flat.get(30, 32)
        );
        // A 45° tilt toward the north adds 1 m per metre north of the centre.
        let t = apply_transforms(
            &h,
            SP,
            &[RegionTransform {
                angle_deg: 45.0,
                azimuth_deg: 0.0,
                ..op(TransformKind::Tilt)
            }],
        )
        .expect("tilt");
        assert!(
            (t.get(32, 42) - h.get(32, 42) - 105.0).abs() < 0.01,
            "{}",
            t.get(32, 42) - h.get(32, 42)
        );
        assert!(
            apply_transforms(
                &h,
                SP,
                &[RegionTransform {
                    angle_deg: 89.5,
                    ..op(TransformKind::Tilt)
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn push_moves_a_bump_and_duplicate_copies_it_and_align_flattens_bumps_onto_the_slope() {
        let mut h = ramp();
        h.fill_with(|x, y| x as f32 + if (x, y) == (32, 32) { 50.0 } else { 0.0 });
        let pushed = apply_transforms(
            &h,
            SP,
            &[RegionTransform {
                vector_m: [40.0, 0.0],
                ..op(TransformKind::Push)
            }],
        )
        .expect("push");
        assert!(
            pushed.get(36, 32) > 70.0,
            "the bump arrived 4 cells east: {}",
            pushed.get(36, 32)
        );
        let dup = apply_transforms(
            &h,
            SP,
            &[RegionTransform {
                radius_m: 50.0,
                vector_m: [0.0, 200.0],
                ..op(TransformKind::Duplicate)
            }],
        )
        .expect("dup");
        assert!(
            (dup.get(32, 52) - 82.0).abs() < 0.01,
            "a copy 20 cells north: {}",
            dup.get(32, 52)
        );
        assert!((dup.get(32, 32) - 82.0).abs() < 0.01, "the original stays");
        let aligned = apply_transforms(&h, SP, &[op(TransformKind::Align)]).expect("align");
        assert!(
            (aligned.get(32, 32) - 32.0).abs() < 0.2,
            "the bump is gone, the slope stays: {}",
            aligned.get(32, 32)
        );
        assert!((aligned.get(40, 32) - aligned.get(24, 32) - 16.0).abs() < 0.2);
    }
}
