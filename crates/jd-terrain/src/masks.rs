//! Masks and filters from World Creator's distribution and filter lists that the graph lacked
//! (`docs/research/world-creator-parity-2026-10-24.md` gaps 3 and 5): the sun-direction /
//! aspect mask, sky occlusion, and the border fade (zero edge / border blend).

use jd_types::{JdError, JdResult};

use crate::Heightfield;

fn check_spacing(spacing_m: [f32; 2]) -> JdResult<()> {
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    Ok(())
}

/// How much each cell faces `azimuth_deg` (clockwise from north): 1 for a slope falling away
/// from that direction's source — i.e. lit by a sun there — 0 facing away, 0.5 flat. `strength`
/// (0..1) is how fast the mask saturates with slope: at 1 a 45° slope facing the sun is fully 1.
pub fn aspect_mask(
    h: &Heightfield,
    spacing_m: [f32; 2],
    azimuth_deg: f32,
    strength: f32,
) -> JdResult<Heightfield> {
    check_spacing(spacing_m)?;
    if !azimuth_deg.is_finite() || !(0.0..=1.0).contains(&strength) {
        return Err(JdError::invalid(
            "azimuth_deg",
            "azimuth must be finite and strength within 0..=1",
        ));
    }
    let (w, hh) = (h.width() as i64, h.height() as i64);
    let rad = azimuth_deg.to_radians();
    // Direction the light comes from, in layer axes (u east, v north).
    let (lx, ly) = (rad.sin(), rad.cos());
    let mut out = h.clone();
    out.fill_with(|x, y| {
        let (xi, yi) = (x as i64, y as i64);
        let g = |xx: i64, yy: i64| h.get(xx.clamp(0, w - 1) as u32, yy.clamp(0, hh - 1) as u32);
        let dx = (g(xi + 1, yi) - g(xi - 1, yi)) / (2.0 * spacing_m[0]);
        let dy = (g(xi, yi + 1) - g(xi, yi - 1)) / (2.0 * spacing_m[1]);
        // A slope faces the light when its downhill side points at it, i.e. the ground falls
        // towards the light: −∇h · light > 0 (a south-facing slope descends southwards).
        let facing = -(dx * lx + dy * ly) * strength.max(1e-6);
        (0.5 + 0.5 * facing.clamp(-1.0, 1.0)).clamp(0.0, 1.0)
    });
    Ok(out)
}

/// Sky occlusion 0..1: how much of the sky above a cell is hidden by terrain within `reach_m`,
/// from the highest elevation angle seen along each of 8 directions (0 on an open plain, high
/// at the bottom of a gorge). An ambient-occlusion and cavity mask; `1 − occlusion` is the sky view.
pub fn occlusion_mask(h: &Heightfield, spacing_m: [f32; 2], reach_m: f32) -> JdResult<Heightfield> {
    check_spacing(spacing_m)?;
    if !(reach_m >= spacing_m[0].max(spacing_m[1]) && reach_m.is_finite()) {
        return Err(JdError::invalid(
            "reach_m",
            "must reach at least one neighbouring cell",
        ));
    }
    let (w, hh) = (h.width() as i64, h.height() as i64);
    const DIRS: [(i64, i64); 8] = [
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
    ];
    let mut out = h.clone();
    out.fill_with(|x, y| {
        let (xi, yi) = (x as i64, y as i64);
        let z0 = h.get(x, y);
        let mut sum = 0.0f32;
        for (dx, dy) in DIRS {
            let step = (dx as f32 * spacing_m[0]).hypot(dy as f32 * spacing_m[1]);
            let mut best = 0.0f32;
            let mut k = 1i64;
            while (k as f32) * step <= reach_m {
                let (nx, ny) = (xi + dx * k, yi + dy * k);
                if nx < 0 || ny < 0 || nx >= w || ny >= hh {
                    break;
                }
                let rise = h.get(nx as u32, ny as u32) - z0;
                if rise > 0.0 {
                    best = best.max((rise / (k as f32 * step)).atan());
                }
                k += 1;
            }
            sum += best / std::f32::consts::FRAC_PI_2;
        }
        (sum / 8.0).clamp(0.0, 1.0)
    });
    Ok(out)
}

/// Border fade: within `width_m` of the layer's edge the heights blend towards `edge_m`
/// (smoothstep), so a world tiles or sits in an engine without a cliff at its border
/// (World Creator's Zero Edge / Border Blend).
pub fn edge_fade(
    h: &Heightfield,
    spacing_m: [f32; 2],
    width_m: f32,
    edge_m: f32,
) -> JdResult<Heightfield> {
    check_spacing(spacing_m)?;
    if !(width_m > 0.0 && width_m.is_finite()) || !edge_m.is_finite() {
        return Err(JdError::invalid(
            "width_m",
            "width must be positive and edge_m finite",
        ));
    }
    let (w, hh) = (h.width() as f32, h.height() as f32);
    let mut out = h.clone();
    out.fill_with(|x, y| {
        let (u, v) = (
            (x as f32 + 0.5) * spacing_m[0],
            (y as f32 + 0.5) * spacing_m[1],
        );
        let d = u
            .min(w * spacing_m[0] - u)
            .min(v)
            .min(hh * spacing_m[1] - v);
        let t = (d / width_m).clamp(0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        let z = h.get(x, y);
        edge_m + (z - edge_m) * s
    });
    Ok(out)
}

/// Pattern masks: a regular design in world metres (World Creator's Dots / Grid / Stripes
/// distributions) — fields for scatter, materials or a stamp's `t`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Pattern {
    /// Discs of radius `width_m` on a `period_m` grid.
    Dots,
    /// Lines of `width_m` along both axes every `period_m`.
    Grid,
    /// Bands of `width_m` every `period_m`, running along the pattern's y axis.
    Stripes,
    /// Alternating `period_m` squares.
    Checker,
}

/// A 0..1 pattern mask over a field's footprint in layer metres (u east from the west edge,
/// v north from the south edge, like placed shapes); `angle_deg` rotates the design clockwise
/// from north, `softness` (0..1, as a share of `width_m`) blurs the edges.
pub fn pattern_mask(
    h: &Heightfield,
    spacing_m: [f32; 2],
    kind: Pattern,
    period_m: f32,
    width_m: f32,
    angle_deg: f32,
    softness: f32,
) -> JdResult<Heightfield> {
    check_spacing(spacing_m)?;
    if !(period_m > 0.0 && period_m.is_finite())
        || !(width_m > 0.0 && width_m.is_finite() && width_m <= period_m)
    {
        return Err(JdError::invalid("period_m", "need 0 < width_m ≤ period_m"));
    }
    if !(0.0..=1.0).contains(&softness) || !angle_deg.is_finite() {
        return Err(JdError::invalid(
            "softness",
            "softness must be within 0..=1 and the angle finite",
        ));
    }
    let (sr, cr) = angle_deg.to_radians().sin_cos();
    let soft = (softness * width_m).max(1e-6);
    let edge = |d: f32| ((d / soft) * 0.5 + 0.5).clamp(0.0, 1.0); // d ≥ 0 inside
    let mut out = h.clone();
    out.fill_with(|x, y| {
        let u = (x as f32 + 0.5) * spacing_m[0];
        let v = (y as f32 + 0.5) * spacing_m[1];
        let (px, py) = (u * cr - v * sr, u * sr + v * cr);
        let (fx, fy) = (px.rem_euclid(period_m), py.rem_euclid(period_m));
        let half = width_m * 0.5;
        match kind {
            Pattern::Dots => {
                let d = (fx - period_m * 0.5).hypot(fy - period_m * 0.5);
                edge(width_m - d)
            }
            Pattern::Grid => {
                let dx = (fx - period_m * 0.5).abs();
                let dy = (fy - period_m * 0.5).abs();
                edge((half - dx).max(half - dy))
            }
            Pattern::Stripes => edge(half - (fx - period_m * 0.5).abs()),
            Pattern::Checker => {
                let cx = (px / period_m).floor() as i64;
                let cy = (py / period_m).floor() as i64;
                let inside = (cx + cy).rem_euclid(2) == 0;
                let dx = (fx.min(period_m - fx)).min(fy.min(period_m - fy));
                let w = edge(dx);
                if inside { w } else { 1.0 - w }
            }
        }
    });
    Ok(out)
}

/// Whole-terrain actions (World Creator's Flip X / Flip Y / Rot Left / Rot Right / Flatten).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TerrainAction {
    /// Mirror east ↔ west.
    FlipX,
    /// Mirror north ↔ south.
    FlipY,
    /// Turn 90° counter-clockwise seen from above (east goes north). Square fields only.
    RotateLeft,
    /// Turn 90° clockwise seen from above (east goes south). Square fields only.
    RotateRight,
    /// Every cell to one height: `height_m`, or the field's mean when it is not given.
    Flatten,
}

/// Apply a whole-terrain action. Row 0 is the south edge, column 0 the west edge.
pub fn terrain_action(
    h: &Heightfield,
    action: TerrainAction,
    height_m: Option<f32>,
) -> JdResult<Heightfield> {
    let (w, hh) = (h.width(), h.height());
    if matches!(
        action,
        TerrainAction::RotateLeft | TerrainAction::RotateRight
    ) && w != hh
    {
        return Err(JdError::invalid(
            "action",
            format!("a 90° turn needs a square world; this one is {w}×{hh}"),
        )
        .next("Flip instead, or make the world square."));
    }
    if let Some(v) = height_m
        && !v.is_finite()
    {
        return Err(JdError::invalid("height_m", "must be finite"));
    }
    let mut out = h.clone();
    match action {
        TerrainAction::FlipX => out.fill_with(|x, y| h.get(w - 1 - x, y)),
        TerrainAction::FlipY => out.fill_with(|x, y| h.get(x, hh - 1 - y)),
        // New (x, y) is where old (x0, y0) lands: CCW sends old east (x0 = w − 1) to new north.
        TerrainAction::RotateLeft => out.fill_with(|x, y| h.get(y, hh - 1 - x)),
        TerrainAction::RotateRight => out.fill_with(|x, y| h.get(w - 1 - y, x)),
        TerrainAction::Flatten => {
            let v = height_m.unwrap_or(h.stats().mean as f32);
            out.fill_with(|_, _| v);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_flip_turn_and_flatten_with_row_zero_south() {
        let mut h = Heightfield::new(4, 4, 4, 0).expect("hf");
        h.fill_with(|x, y| (x + 10 * y) as f32); // east grows by 1, north by 10
        let fx = terrain_action(&h, TerrainAction::FlipX, None).expect("fx");
        assert_eq!(fx.get(0, 0), 3.0);
        let fy = terrain_action(&h, TerrainAction::FlipY, None).expect("fy");
        assert_eq!(fy.get(0, 0), 30.0);
        // Left: the south-east corner (3, 0) = 3 ends up north-east (3, 3).
        let l = terrain_action(&h, TerrainAction::RotateLeft, None).expect("left");
        assert_eq!(l.get(3, 3), 3.0);
        assert_eq!(
            l.get(0, 0),
            30.0,
            "the north-west corner comes to the south-west"
        );
        // Right undoes left.
        let back = terrain_action(&l, TerrainAction::RotateRight, None).expect("right");
        assert_eq!(back.to_vec(), h.to_vec());
        let flat = terrain_action(&h, TerrainAction::Flatten, None).expect("flat");
        assert!(
            flat.to_vec().iter().all(|v| (*v - 16.5).abs() < 1e-5),
            "flatten to the mean"
        );
        assert_eq!(
            terrain_action(&h, TerrainAction::Flatten, Some(7.0))
                .expect("flat7")
                .get(2, 2),
            7.0
        );
        let wide = Heightfield::new(8, 4, 4, 0).expect("hf");
        assert!(terrain_action(&wide, TerrainAction::RotateLeft, None).is_err());
    }

    #[test]
    fn aspect_lights_the_slopes_that_face_the_sun_and_leaves_flat_ground_half() {
        // A ridge along north–south: the west face rises eastwards, the east face falls.
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, _| 100.0 - (x as f32 - 32.0).abs() * 10.0);
        let east_sun = aspect_mask(&h, [10.0, 10.0], 90.0, 1.0).expect("aspect");
        assert!(
            east_sun.get(40, 32) > 0.9,
            "the east face faces an eastern sun: {}",
            east_sun.get(40, 32)
        );
        assert!(
            east_sun.get(24, 32) < 0.1,
            "the west face is in its shadow: {}",
            east_sun.get(24, 32)
        );
        let mut flat = Heightfield::new(16, 16, 16, 0).expect("flat");
        flat.fill_with(|_, _| 5.0);
        assert_eq!(
            aspect_mask(&flat, [10.0, 10.0], 180.0, 0.5)
                .expect("flat")
                .get(8, 8),
            0.5
        );
        assert!(aspect_mask(&h, [10.0, 10.0], 90.0, 1.5).is_err());
    }

    #[test]
    fn patterns_repeat_in_world_metres_and_refuse_a_width_past_the_period() {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|_, _| 0.0);
        let dots =
            pattern_mask(&h, [10.0, 10.0], Pattern::Dots, 160.0, 40.0, 0.0, 0.0).expect("dots");
        // Dot centres at (80, 80) + k·160 m → cells (7..8, 7..8); a corner cell is empty.
        assert_eq!(dots.get(7, 7), 1.0);
        assert_eq!(dots.get(0, 0), 0.0);
        let on: usize = (0..64)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .filter(|&(x, y)| dots.get(x, y) > 0.5)
            .count();
        assert!(
            on > 600 && on < 1000,
            "16 dots of ~π·4² ≈ 50 cells each: {on}"
        );
        let stripes = pattern_mask(&h, [10.0, 10.0], Pattern::Stripes, 160.0, 40.0, 0.0, 0.0)
            .expect("stripes");
        assert_eq!(stripes.get(7, 3), stripes.get(7, 60), "stripes run along y");
        let checker = pattern_mask(&h, [10.0, 10.0], Pattern::Checker, 320.0, 320.0, 0.0, 0.0)
            .expect("checker");
        assert_eq!(checker.get(5, 5), 1.0);
        assert_eq!(checker.get(40, 5), 0.0);
        assert!(pattern_mask(&h, [10.0, 10.0], Pattern::Grid, 100.0, 120.0, 0.0, 0.0).is_err());
    }

    #[test]
    fn occlusion_is_zero_on_a_plain_and_high_in_a_pit_and_the_edge_fades_to_its_level() {
        let mut plain = Heightfield::new(32, 32, 32, 0).expect("plain");
        plain.fill_with(|_, _| 50.0);
        assert_eq!(
            occlusion_mask(&plain, [10.0, 10.0], 100.0)
                .expect("occ")
                .get(16, 16),
            0.0
        );
        let mut pit = Heightfield::new(32, 32, 32, 0).expect("pit");
        pit.fill_with(|x, y| {
            if (x as i32 - 16).abs() <= 2 && (y as i32 - 16).abs() <= 2 {
                0.0
            } else {
                100.0
            }
        });
        let occ = occlusion_mask(&pit, [10.0, 10.0], 100.0).expect("occ");
        assert!(
            occ.get(16, 16) > 0.6,
            "the pit floor sees little sky: {}",
            occ.get(16, 16)
        );
        assert!(occ.get(4, 4) < 0.05, "the rim is open");
        assert!(occlusion_mask(&pit, [10.0, 10.0], 5.0).is_err());
        let faded = edge_fade(&plain, [10.0, 10.0], 60.0, 0.0).expect("fade");
        assert_eq!(
            faded.get(0, 16),
            0.0 + 50.0 * {
                let t = 5.0f32 / 60.0;
                t * t * (3.0 - 2.0 * t)
            },
            "half a cell in: almost the edge level"
        );
        assert_eq!(faded.get(16, 16), 50.0, "the middle keeps its height");
        assert!(
            faded.get(3, 16) > faded.get(1, 16) && faded.get(3, 16) < 50.0,
            "a smooth ramp inside the width"
        );
        assert!(edge_fade(&plain, [10.0, 10.0], 0.0, 0.0).is_err());
    }
}
