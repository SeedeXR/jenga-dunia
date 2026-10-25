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

#[cfg(test)]
mod tests {
    use super::*;

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
