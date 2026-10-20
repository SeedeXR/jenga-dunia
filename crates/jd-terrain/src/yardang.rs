//! Yardangs: wind abrasion. Sand-laden wind sandblasts the faces that meet it and leaves the
//! sheltered lee alone, so ridges streamline into keels elongated along the wind — the
//! yardang fields of the Lut, Tibesti and the Qaidam basin (`docs/research/
//! materials-geomorphology.md`, "Aeolian"). Unlike Werner's slab model (`aeolian`), which moves
//! sand, this removes rock: the dust leaves the field, so mass is not conserved.
//!
//! Per step and cell: `exposure` is the slope the wind meets (the cell is higher than its upwind
//! neighbour), normalised by `ref_slope_deg`; `shelter` is 1 when a higher cell upwind within
//! `shelter_m` casts a wind shadow steeper than `shadow_deg` (Werner's 15°); the cut is
//! `rate_m × exposure × (1 − shelter) × (1 − 0.9·hardness)`. An optional `hardness` field
//! (from `strata`) lets caprock keels survive.
//!
//! ponytail: 8-direction wind (azimuth snapped to the nearest 45°), one neighbour of exposure;
//! a D∞-style fractional direction and lateral undercutting are the upgrades.

use std::sync::atomic::AtomicBool;

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::sim::check_cancel;

/// Yardang settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct YardangParams {
    /// Abrasion steps (1..=2000).
    #[serde(default = "d_iterations")]
    pub iterations: u32,
    /// Compass direction the wind blows from, degrees clockwise from north (270 = westerly). In a
    /// graph, unset → the world's shared wind.
    #[serde(default = "d_wind")]
    pub wind_azimuth_deg: f32,
    /// Depth removed per step from a fully exposed, unsheltered face, metres (> 0).
    #[serde(default = "d_rate")]
    pub rate_m: f32,
    /// Windward slope at which exposure saturates, degrees (0, 90).
    #[serde(default = "d_ref_slope")]
    pub ref_slope_deg: f32,
    /// How far upwind a higher cell can shelter this one, metres (≥ 0).
    #[serde(default = "d_shelter")]
    pub shelter_m: f32,
    /// A shelter line steeper than this (seen along the wind) is a wind shadow, degrees (0, 90).
    #[serde(default = "d_shadow")]
    pub shadow_deg: f32,
}

fn d_iterations() -> u32 {
    100
}
fn d_wind() -> f32 {
    270.0
}
fn d_rate() -> f32 {
    0.05
}
fn d_ref_slope() -> f32 {
    20.0
}
fn d_shelter() -> f32 {
    60.0
}
fn d_shadow() -> f32 {
    15.0
}

impl Default for YardangParams {
    fn default() -> Self {
        Self {
            iterations: d_iterations(),
            wind_azimuth_deg: d_wind(),
            rate_m: d_rate(),
            ref_slope_deg: d_ref_slope(),
            shelter_m: d_shelter(),
            shadow_deg: d_shadow(),
        }
    }
}

/// Abrade the windward faces of `h` for `p.iterations` steps.
pub fn wind_abrasion(
    h: &Heightfield,
    spacing_m: [f32; 2],
    hardness: Option<&Heightfield>,
    p: &YardangParams,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    if !(1..=2000).contains(&p.iterations) {
        return Err(JdError::invalid("iterations", "must be within 1..=2000"));
    }
    if !(p.rate_m > 0.0 && p.rate_m.is_finite()) {
        return Err(JdError::invalid("rate_m", "must be positive"));
    }
    for (name, deg) in [
        ("ref_slope_deg", p.ref_slope_deg),
        ("shadow_deg", p.shadow_deg),
    ] {
        if !(0.0 < deg && deg < 90.0) {
            return Err(JdError::invalid(name, "must be within (0, 90)"));
        }
    }
    if !(p.shelter_m >= 0.0 && p.shelter_m.is_finite()) || !p.wind_azimuth_deg.is_finite() {
        return Err(JdError::invalid("shelter_m", "must be finite and ≥ 0"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    if let Some(k) = hardness
        && (k.width(), k.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid("hardness", "field size differs from `in`"));
    }
    let (w, hh) = (h.width() as i64, h.height() as i64);
    // Wind FROM azimuth → the upwind neighbour lies towards the azimuth: north is +y (row 0 = south).
    let rad = p.wind_azimuth_deg.to_radians();
    let (ux, uy) = (rad.sin().round() as i64, rad.cos().round() as i64);
    if ux == 0 && uy == 0 {
        return Err(JdError::invalid("wind_azimuth_deg", "must be finite"));
    }
    let step_len = ((ux as f32 * spacing_m[0]).powi(2) + (uy as f32 * spacing_m[1]).powi(2)).sqrt();
    let reach = ((p.shelter_m / step_len).floor() as i64).max(1);
    let tan_ref = p.ref_slope_deg.to_radians().tan();
    let tan_shadow = p.shadow_deg.to_radians().tan();
    let hard = hardness.map(|k| k.to_vec());
    let mut cur = h.to_vec();
    let idx = |x: i64, y: i64| (y * w + x) as usize;
    for _ in 0..p.iterations {
        check_cancel(cancel)?;
        let mut next = cur.clone();
        for y in 0..hh {
            for x in 0..w {
                let (upx, upy) = (x + ux, y + uy); // towards the azimuth = upwind
                if upx < 0 || upy < 0 || upx >= w || upy >= hh {
                    continue; // the border takes the wind as it comes: nothing to compare with
                }
                let here = cur[idx(x, y)];
                let exposure = ((here - cur[idx(upx, upy)]) / step_len / tan_ref).clamp(0.0, 1.0);
                if exposure == 0.0 {
                    continue;
                }
                // Shelter: any higher cell upwind within reach whose line of sight is steeper than the shadow angle.
                let mut shelter = 0.0f32;
                for k in 1..=reach {
                    let (sx, sy) = (x + ux * k, y + uy * k);
                    if sx < 0 || sy < 0 || sx >= w || sy >= hh {
                        break;
                    }
                    let rise = (cur[idx(sx, sy)] - here) / (k as f32 * step_len);
                    if rise > tan_shadow {
                        shelter = 1.0;
                        break;
                    }
                }
                let soft = 1.0 - 0.9 * hard.as_ref().map_or(0.0, |k| k[idx(x, y)].clamp(0.0, 1.0));
                next[idx(x, y)] = here - p.rate_m * exposure * (1.0 - shelter) * soft;
            }
        }
        cur = next;
    }
    let mut out = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    out.fill_with(|x, y| cur[(y as i64 * w + x as i64) as usize]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn westerly_abrades_the_windward_face_and_spares_the_lee_and_hard_rock() {
        // A 30 m cone in the middle of a 64² field at 10 m cells.
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| {
            let d = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
            (30.0 - d * 2.5).max(0.0)
        });
        let p = YardangParams {
            iterations: 40,
            ..YardangParams::default()
        };
        let out = wind_abrasion(&h, [10.0, 10.0], None, &p, None).expect("abrade");
        let loss = |x0: u32, x1: u32| -> f32 {
            (x0..x1)
                .flat_map(|x| (20..44).map(move |y| (x, y)))
                .map(|(x, y)| h.get(x, y) - out.get(x, y))
                .sum::<f32>()
        };
        let (windward, lee) = (loss(20, 32), loss(33, 45));
        assert!(windward > 0.0, "the west face is abraded: {windward}");
        assert!(
            lee < windward * 0.05,
            "the lee keeps its shape: lee {lee} vs windward {windward}"
        );
        for y in 0..64 {
            for x in 0..64 {
                assert!(out.get(x, y) <= h.get(x, y) + 1e-6, "abrasion never builds");
            }
        }
        // A sheltered cell (just downwind of the summit) loses nothing; hard rock loses a tenth.
        assert_eq!(out.get(34, 32), h.get(34, 32));
        let mut cap = Heightfield::new(64, 64, 64, 0).expect("k");
        cap.fill_with(|_, _| 1.0);
        let hard = wind_abrasion(&h, [10.0, 10.0], Some(&cap), &p, None).expect("hard");
        let hard_loss: f32 = (20..32)
            .flat_map(|x| (20..44).map(move |y| (x, y)))
            .map(|(x, y)| h.get(x, y) - hard.get(x, y))
            .sum();
        assert!(
            (hard_loss / windward - 0.1).abs() < 0.03,
            "hard loss {hard_loss} vs {windward}"
        );
        // Bad inputs are refused.
        assert!(
            wind_abrasion(
                &h,
                [10.0, 10.0],
                None,
                &YardangParams {
                    rate_m: 0.0,
                    ..p.clone()
                },
                None
            )
            .is_err()
        );
        assert!(
            wind_abrasion(
                &h,
                [10.0, 10.0],
                None,
                &YardangParams { iterations: 0, ..p },
                None
            )
            .is_err()
        );
    }
}
