//! Stroke sculpting: brushes recorded as data and replayed onto a field (`02-subsystems.md` §4).
//! A stroke is a polyline in layer metres (`u` east from the west edge, `v` north from the south
//! edge, i.e. row 0 — the field convention), a radius and a signed strength. Height changes by `strength · falloff(d/r)`
//! where `d` is the distance to the polyline, so strokes are order-independent and undo removes a
//! whole stroke by dropping it from the node's params. `hardness` widens the flat core of the
//! falloff (1 = a hard edge); a `square` stroke stamps a square of side 2·radius at every point
//! instead of sweeping a round brush along the polyline — one grid selection of cells is one
//! stroke, so one undo step; `target_m` flattens to a given height instead of the height under
//! the first point.
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

/// The brush's footprint.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StrokeShape {
    /// A round brush swept along the polyline (distance to the nearest segment).
    #[default]
    Round,
    /// A square of side 2·radius stamped at each point (Chebyshev distance to the nearest point);
    /// grid selections.
    Square,
}

/// Brush alpha: the pattern inside the footprint (World Creator's brush palette). It multiplies the
/// falloff; patterns are laid out in brush-sized units from the layer origin, so a long stroke
/// reads as one continuous texture.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BrushAlpha {
    /// The plain smooth falloff.
    #[default]
    Plain,
    /// Fractal value noise, 0.25..1 of the stroke.
    Noise,
    /// A raised rim at 3/4 of the radius around a sunken bowl (raise mode: a crater; lower: a ring dyke).
    Crater,
    /// Parallel bands across the stroke, three per radius, running north–south.
    Ridges,
    /// Rounded rock bumps (a cellular pattern, half-radius cells).
    Rocks,
    /// Scattered round dots.
    Dots,
    /// A checkerboard of full and quarter strength.
    Checker,
}

fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

fn value_noise(u: f32, v: f32, seed: u32) -> f32 {
    let (x0, y0) = (u.floor() as i32, v.floor() as i32);
    let (fx, fy) = (u - u.floor(), v - v.floor());
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let a = hash2(x0, y0, seed) + (hash2(x0 + 1, y0, seed) - hash2(x0, y0, seed)) * sx;
    let b = hash2(x0, y0 + 1, seed) + (hash2(x0 + 1, y0 + 1, seed) - hash2(x0, y0 + 1, seed)) * sx;
    a + (b - a) * sy
}

impl BrushAlpha {
    /// The multiplier at layer point `p` for a brush of `radius` at normalised distance `t` (0 centre, 1 edge).
    fn at(self, p: [f32; 2], radius: f32, t: f32) -> f32 {
        let (u, v) = (p[0] / radius, p[1] / radius);
        match self {
            BrushAlpha::Plain => 1.0,
            BrushAlpha::Noise => {
                let n = 0.55 * value_noise(u * 2.0, v * 2.0, 11)
                    + 0.3 * value_noise(u * 4.0, v * 4.0, 23)
                    + 0.15 * value_noise(u * 8.0, v * 8.0, 37);
                0.25 + 0.75 * n
            }
            BrushAlpha::Crater => {
                let rim = (-((t - 0.75) / 0.12).powi(2)).exp();
                let bowl = 1.0 - (t / 0.6).clamp(0.0, 1.0);
                rim - 0.6 * bowl * bowl * (3.0 - 2.0 * bowl)
            }
            BrushAlpha::Ridges => 0.5 + 0.5 * (std::f32::consts::TAU * u * 3.0).cos(),
            BrushAlpha::Rocks => {
                let (cu, cv) = (u * 2.0, v * 2.0);
                let (ix, iy) = (cu.floor() as i32, cv.floor() as i32);
                let mut best = f32::MAX;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (gx, gy) = (ix + dx, iy + dy);
                        let (fx, fy) = (gx as f32 + hash2(gx, gy, 5), gy as f32 + hash2(gx, gy, 7));
                        best = best.min((cu - fx).hypot(cv - fy));
                    }
                }
                (1.0 - best).clamp(0.0, 1.0)
            }
            BrushAlpha::Dots => {
                let (cu, cv) = (u * 3.0, v * 3.0);
                let (ix, iy) = (cu.floor() as i32, cv.floor() as i32);
                let (fx, fy) = (ix as f32 + 0.5, iy as f32 + 0.5);
                if hash2(ix, iy, 13) > 0.45 && (cu - fx).hypot(cv - fy) < 0.32 {
                    1.0
                } else {
                    0.0
                }
            }
            BrushAlpha::Checker => {
                if ((u * 3.0).floor() as i32 + (v * 3.0).floor() as i32).rem_euclid(2) == 0 {
                    1.0
                } else {
                    0.25
                }
            }
        }
    }
}

/// Where a sculpt layer applies: a rectangle in layer metres, turned about its centre, fading over
/// `blend_m` at its edge (World Creator's Area Settings). No area = the whole world.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerArea {
    /// Centre `[u east, v north]`, metres.
    pub center_m: [f32; 2],
    /// Width and depth, metres.
    pub size_m: [f32; 2],
    /// Rotation about the centre, degrees clockwise from north.
    #[serde(default)]
    pub rotation_deg: f32,
    /// Fade width inside the edge, metres.
    #[serde(default)]
    pub blend_m: f32,
}

/// Sculpt-layer settings (World Creator's Layer Settings): the strokes' change to the terrain is
/// scaled, offset, stepped and blended before it is added back.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerSettings {
    /// Multiplies the change the strokes made (0 hides the layer, 2 doubles it).
    #[serde(default = "one_f32")]
    pub height_scale: f32,
    /// Added to the change wherever the layer applies, metres.
    #[serde(default)]
    pub height_offset_m: f32,
    /// Opacity of the whole layer, 0..1.
    #[serde(default = "one_f32")]
    pub blending: f32,
    /// Quantise the change to multiples of this many metres (World Creator's Multiple Steps); 0 = off.
    #[serde(default)]
    pub level_step_m: f32,
    /// Where the layer applies; None = everywhere ("Fit to terrain").
    #[serde(default)]
    pub area: Option<LayerArea>,
}
fn one_f32() -> f32 {
    1.0
}
impl Default for LayerSettings {
    fn default() -> Self {
        Self {
            height_scale: 1.0,
            height_offset_m: 0.0,
            blending: 1.0,
            level_step_m: 0.0,
            area: None,
        }
    }
}

/// Combine a sculpted field with its base through the layer settings.
pub fn apply_layer_settings(
    base: &Heightfield,
    sculpted: &Heightfield,
    spacing_m: [f32; 2],
    ls: &LayerSettings,
) -> JdResult<Heightfield> {
    if !(0.0..=10.0).contains(&ls.height_scale)
        || !ls.height_offset_m.is_finite()
        || !(0.0..=1.0).contains(&ls.blending)
        || !(ls.level_step_m >= 0.0 && ls.level_step_m.is_finite())
    {
        return Err(JdError::invalid(
            "layer",
            "height_scale 0..=10, offset finite, blending 0..=1, level_step_m ≥ 0",
        ));
    }
    if let Some(a) = &ls.area
        && !(a.size_m.iter().all(|v| *v > 0.0 && v.is_finite())
            && a.center_m.iter().all(|v| v.is_finite())
            && a.rotation_deg.is_finite()
            && a.blend_m >= 0.0
            && a.blend_m.is_finite())
    {
        return Err(JdError::invalid(
            "layer.area",
            "size positive, centre / rotation finite, blend ≥ 0",
        ));
    }
    let (sr, cr) = ls
        .area
        .map_or((0.0, 1.0), |a| a.rotation_deg.to_radians().sin_cos());
    let mut out = base.clone();
    out.fill_with(|x, y| {
        let b = base.get(x, y);
        let w = match &ls.area {
            None => 1.0,
            Some(a) => {
                let (u, v) = (
                    (x as f32 + 0.5) * spacing_m[0] - a.center_m[0],
                    (y as f32 + 0.5) * spacing_m[1] - a.center_m[1],
                );
                let (lx, ly) = (u * cr - v * sr, u * sr + v * cr);
                let edge = (a.size_m[0] * 0.5 - lx.abs()).min(a.size_m[1] * 0.5 - ly.abs());
                if edge <= 0.0 {
                    0.0
                } else if a.blend_m > 0.0 {
                    (edge / a.blend_m).min(1.0)
                } else {
                    1.0
                }
            }
        } * ls.blending;
        if w <= 0.0 {
            return b;
        }
        let mut d = (sculpted.get(x, y) - b) * ls.height_scale + ls.height_offset_m;
        if ls.level_step_m > 0.0 {
            d = (d / ls.level_step_m).round() * ls.level_step_m;
        }
        b + d * w
    });
    Ok(out)
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
    /// Edge hardness 0..1: the share of the radius that gets the full effect before the falloff
    /// starts (0 = the falloff spans the whole radius, 1 = a hard edge).
    #[serde(default)]
    pub hardness: f32,
    /// Footprint (default `round`).
    #[serde(default)]
    pub shape: StrokeShape,
    /// `flatten` only: the height to flatten to, metres; unset = the height under the first point.
    #[serde(default)]
    pub target_m: Option<f32>,
    /// Brush alpha: the pattern inside the footprint (default plain).
    #[serde(default)]
    pub alpha: BrushAlpha,
}
fn half() -> f32 {
    0.5
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            radius_m: 1.0,
            strength_m: 0.0,
            mode: StrokeMode::Raise,
            amount: 0.5,
            hardness: 0.0,
            shape: StrokeShape::Round,
            target_m: None,
            alpha: BrushAlpha::Plain,
        }
    }
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
        if !(0.0..=1.0).contains(&s.hardness) || s.target_m.is_some_and(|t| !t.is_finite()) {
            return Err(JdError::invalid(
                "strokes",
                format!("stroke {i}: hardness must be within 0..=1 and target_m finite"),
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
                s.target_m.unwrap_or_else(|| sample(s.points[0]))
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
            let d = match s.shape {
                StrokeShape::Round => s
                    .points
                    .windows(2)
                    .map(|w| dist_to_segment(p, w[0], w[1]))
                    .chain(std::iter::once(dist_to_segment(
                        p,
                        s.points[0],
                        s.points[0],
                    )))
                    .fold(f32::MAX, f32::min),
                StrokeShape::Square => s
                    .points
                    .iter()
                    .map(|q| (p[0] - q[0]).abs().max((p[1] - q[1]).abs()))
                    .fold(f32::MAX, f32::min),
            };
            let t = (d / s.radius_m).min(1.0);
            // Smooth falloff over the ring outside the hard core: 1 at the centre, 0 at the
            // radius, zero slope at both ends.
            let t = if s.hardness >= 1.0 {
                if t < 1.0 { 0.0 } else { 1.0 }
            } else {
                ((t - s.hardness) / (1.0 - s.hardness)).clamp(0.0, 1.0)
            };
            let w = 1.0 - t * t * (3.0 - 2.0 * t);
            let w = if s.alpha == BrushAlpha::Plain {
                w
            } else {
                w * s.alpha.at(p, s.radius_m, (d / s.radius_m).min(1.0))
            };
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
    fn layer_settings_scale_offset_step_and_confine_the_strokes() {
        let mut base = Heightfield::new(64, 64, 64, 0).expect("hf");
        base.fill_with(|_, _| 100.0);
        let raised = apply_strokes(
            &base,
            [10.0, 10.0],
            &[Stroke {
                points: vec![[320.0, 320.0]],
                radius_m: 250.0,
                strength_m: 37.0,
                hardness: 0.8,
                ..Stroke::default()
            }],
        )
        .expect("raise");
        let ls = |f: fn(&mut LayerSettings)| {
            let mut l = LayerSettings::default();
            f(&mut l);
            apply_layer_settings(&base, &raised, [10.0, 10.0], &l).expect("layer")
        };
        assert_eq!(
            ls(|_| {}).to_vec(),
            raised.to_vec(),
            "defaults change nothing"
        );
        assert_eq!(
            ls(|l| l.height_scale = 0.0).to_vec(),
            base.to_vec(),
            "scale 0 hides the layer"
        );
        assert!((ls(|l| l.height_scale = 2.0).get(32, 32) - 174.0).abs() < 1e-3);
        assert!(
            (ls(|l| l.height_offset_m = 5.0).get(2, 2) - 105.0).abs() < 1e-3,
            "the offset applies everywhere the layer does"
        );
        assert!(
            (ls(|l| l.level_step_m = 10.0).get(32, 32) - 140.0).abs() < 1e-3,
            "37 m steps to 40 m"
        );
        assert!((ls(|l| l.blending = 0.5).get(32, 32) - 118.5).abs() < 1e-3);
        let boxed = ls(|l| {
            l.area = Some(LayerArea {
                center_m: [100.0, 100.0],
                size_m: [100.0, 100.0],
                rotation_deg: 0.0,
                blend_m: 0.0,
            })
        });
        assert_eq!(
            boxed.get(32, 32),
            100.0,
            "outside the area the layer does nothing"
        );
        assert!(
            apply_layer_settings(
                &base,
                &raised,
                [10.0, 10.0],
                &LayerSettings {
                    blending: 2.0,
                    ..LayerSettings::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn alphas_pattern_the_stroke_crater_raises_a_rim_round_a_bowl() {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|_, _| 0.0);
        let dot = |alpha| Stroke {
            points: vec![[320.0, 320.0]],
            radius_m: 200.0,
            strength_m: 10.0,
            alpha,
            ..Stroke::default()
        };
        let crater = apply_strokes(&h, [10.0, 10.0], &[dot(BrushAlpha::Crater)]).expect("crater");
        assert!(
            crater.get(31, 31) < -1.0,
            "a bowl in the middle: {}",
            crater.get(31, 31)
        );
        assert!(
            crater.get(31 + 15, 31) > 1.0,
            "a rim at 3/4 radius: {}",
            crater.get(31 + 15, 31)
        );
        let plain = apply_strokes(&h, [10.0, 10.0], &[dot(BrushAlpha::Plain)]).expect("plain");
        let noise = apply_strokes(&h, [10.0, 10.0], &[dot(BrushAlpha::Noise)]).expect("noise");
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for y in 26..37 {
            for x in 26..37 {
                let r = noise.get(x, y) / plain.get(x, y);
                lo = lo.min(r);
                hi = hi.max(r);
            }
        }
        assert!(
            lo >= 0.25 - 1e-4 && hi <= 1.0 + 1e-4 && hi - lo > 0.1,
            "noise varies within 0.25..1: {lo}..{hi}"
        );
        let checker =
            apply_strokes(&h, [10.0, 10.0], &[dot(BrushAlpha::Checker)]).expect("checker");
        let ratios: std::collections::BTreeSet<i32> = (28..35)
            .flat_map(|y| (28..35).map(move |x| (x, y)))
            .map(|(x, y)| (checker.get(x, y) / plain.get(x, y) * 100.0).round() as i32)
            .collect();
        assert_eq!(
            ratios.into_iter().collect::<Vec<_>>(),
            vec![25, 100],
            "two levels"
        );
        let again = apply_strokes(&h, [10.0, 10.0], &[dot(BrushAlpha::Rocks)]).expect("rocks");
        assert_eq!(
            again.to_vec(),
            apply_strokes(&h, [10.0, 10.0], &[dot(BrushAlpha::Rocks)])
                .expect("rocks")
                .to_vec(),
            "deterministic"
        );
    }

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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
                    amount: 1.5,
                    ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
                    amount: 0.5,
                    ..Default::default()
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
                    amount: 0.5,
                    ..Default::default()
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn hard_square_stamps_level_their_cells_to_a_target_and_nothing_between_them() {
        // Two cells of a grid selection, 20 m apart, levelled to 300 m with a hard edge: every
        // pixel inside either square lands on the target, the gap between them stays untouched.
        let mut h = Heightfield::new(64, 64, 32, 4).expect("hf");
        h.fill_with(|x, y| (x + y) as f32);
        let out = apply_strokes(
            &h,
            [1.0, 1.0],
            &[Stroke {
                points: vec![[10.0, 10.0], [30.0, 10.0]],
                radius_m: 4.0,
                mode: StrokeMode::Flatten,
                amount: 1.0,
                hardness: 1.0,
                shape: StrokeShape::Square,
                target_m: Some(300.0),
                ..Default::default()
            }],
        )
        .expect("stamps");
        for (x, y) in [(6u32, 6u32), (13, 13), (26, 7), (33, 13)] {
            assert_eq!(out.get(x, y), 300.0, "inside a stamp ({x},{y})");
        }
        assert_eq!(
            out.get(20, 10),
            h.get(20, 10),
            "between the stamps (a round stroke would sweep it)"
        );
        assert_eq!(out.get(10, 15), h.get(10, 15), "outside the square");
        // Hardness 0.5 keeps the full effect over the inner half of the radius and fades after it.
        let soft = apply_strokes(
            &h,
            [1.0, 1.0],
            &[Stroke {
                points: vec![[32.5, 32.5]],
                radius_m: 10.0,
                strength_m: 10.0,
                hardness: 0.5,
                ..Default::default()
            }],
        )
        .expect("soft");
        assert!(
            (soft.get(36, 32) - h.get(36, 32) - 10.0).abs() < 1e-4,
            "4 m out, inside the hard core"
        );
        let edge = soft.get(39, 32) - h.get(39, 32);
        assert!(edge > 0.0 && edge < 9.0, "7 m out: in the falloff ({edge})");
        assert!(
            apply_strokes(
                &h,
                [1.0, 1.0],
                &[Stroke {
                    points: vec![[1.0, 1.0]],
                    hardness: 1.5,
                    ..Default::default()
                }]
            )
            .is_err()
        );
    }
}
