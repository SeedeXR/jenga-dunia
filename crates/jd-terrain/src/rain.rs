//! Orographic rain: moisture rides the wind across the field, windward climbs wring it out and
//! the lee sits in a rain shadow (Smith & Barstad 2004 in spirit; `docs/research/
//! materials-geomorphology.md` B8). Saturated air holds less water the higher it goes
//! (`exp(-h / moisture_scale_m)`): one upwind sweep carries the air's water from cell to cell,
//! condenses whatever exceeds the local capacity as rain, drizzles a steady share over flat ground
//! (`base_per_km`) and lets the ground re-moisten dry air towards saturation over `recovery_km`.
//! Descending air rains nothing (its capacity grows), so hills lower than the range upwind of them
//! stay dry — the shadow — until the air has recovered.
//!
//! ponytail: the wind is rounded to one of 8 neighbours (like the snow drift), one scalar of water
//! per cell, no advection delay, no temperature; `rain` is normalised to its own 99th percentile so it
//! reads as a 0..1 mask.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;

/// Orographic rain settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RainParams {
    /// Compass direction the wind blows from, degrees clockwise from north. In a graph, unset →
    /// the world's shared wind.
    #[serde(default = "d_west")]
    pub wind_azimuth_deg: f32,
    /// Height over which saturated air's water capacity falls to 1/e, metres (> 0): the climb that
    /// wrings out most of the moisture.
    #[serde(default = "d_scale")]
    pub moisture_scale_m: f32,
    /// Share of the air's water that drizzles out per km of travel regardless of the ground (0..1).
    #[serde(default = "d_base")]
    pub base_per_km: f32,
    /// Distance over which dry air re-moistens towards saturation, km (> 0): the length of a rain shadow.
    #[serde(default = "d_recovery")]
    pub recovery_km: f32,
}

fn d_west() -> f32 {
    270.0
}
fn d_scale() -> f32 {
    2500.0
}
fn d_base() -> f32 {
    0.01
}
fn d_recovery() -> f32 {
    40.0
}

impl Default for RainParams {
    fn default() -> Self {
        Self {
            wind_azimuth_deg: d_west(),
            moisture_scale_m: d_scale(),
            base_per_km: d_base(),
            recovery_km: d_recovery(),
        }
    }
}

/// Returns (`rain` 0..1 with the field's 99th percentile at 1, `moisture` 0..1 = the relative
/// humidity of the air leaving each cell). Row 0 is south, x grows east; the air enters saturated
/// for its height at the upwind edge.
pub fn orographic_rain(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &RainParams,
) -> JdResult<(Heightfield, Heightfield)> {
    if !(0.0..=1.0).contains(&p.base_per_km) {
        return Err(JdError::invalid("base_per_km", "must be within 0..=1"));
    }
    for (name, v) in [
        ("moisture_scale_m", p.moisture_scale_m),
        ("recovery_km", p.recovery_km),
    ] {
        if !(v > 0.0 && v.is_finite()) {
            return Err(JdError::invalid(name, "must be positive"));
        }
    }
    if !p.wind_azimuth_deg.is_finite() {
        return Err(JdError::invalid("wind_azimuth_deg", "must be finite"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let (w, hh) = (h.width() as i64, h.height() as i64);
    // The air travels towards azimuth + 180°, rounded to a neighbour; rows grow north.
    let to = (p.wind_azimuth_deg + 180.0).to_radians();
    let (ddx, ddy) = (to.sin().round() as i64, to.cos().round() as i64);
    let step_km =
        ((ddx as f32 * spacing_m[0]).powi(2) + (ddy as f32 * spacing_m[1]).powi(2)).sqrt() / 1000.0;
    let cap = |x: i64, y: i64| (-h.get(x as u32, y as u32) / p.moisture_scale_m).exp();
    let n = (w * hh) as usize;
    let mut rain = vec![0.0f32; n];
    let mut q = vec![0.0f32; n];
    let mut rh = vec![0.0f32; n];
    // Sweep so the upwind neighbour is always visited first.
    let xs = |k: i64| if ddx >= 0 { k } else { w - 1 - k };
    let ys = |k: i64| if ddy >= 0 { k } else { hh - 1 - k };
    for y in (0..hh).map(ys) {
        for x in (0..w).map(xs) {
            let i = (y * w + x) as usize;
            let c = cap(x, y);
            let (ux, uy) = (x - ddx, y - ddy);
            let water = if ux >= 0 && uy >= 0 && ux < w && uy < hh {
                q[(uy * w + ux) as usize]
            } else {
                c
            };
            let condensed = (water - c).max(0.0);
            let drizzle = (water - condensed) * (p.base_per_km * step_km).min(1.0);
            rain[i] = condensed + drizzle;
            let left = water - rain[i];
            q[i] = left + (c - left).max(0.0) * (step_km / p.recovery_km).min(1.0);
            rh[i] = (q[i] / c).clamp(0.0, 1.0);
        }
    }
    // Normalise to the 99th percentile, not the maximum: one steep cell would otherwise own the mask.
    let mut sorted = rain.clone();
    let idx = (sorted.len() - 1) * 99 / 100;
    let peak = sorted
        .select_nth_unstable_by(idx, f32::total_cmp)
        .1
        .max(f32::MIN_POSITIVE);
    let wu = w as u32;
    let mut out = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    out.fill_with(|x, y| (rain[(y * wu + x) as usize] / peak).min(1.0));
    let mut moist = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    moist.fill_with(|x, y| rh[(y * wu + x) as usize]);
    Ok((out, moist))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windward_face_is_wet_and_the_lee_sits_in_a_rain_shadow() {
        // A north–south ridge on a plain, 100 m cells, under a westerly: the west face climbs
        // 1 500 m from x = 17 to x = 32, the east face drops back to the plain by x = 47.
        let mut ridge = Heightfield::new(64, 16, 64, 0).expect("ridge");
        ridge.fill_with(|x, _| (1500.0 - (x as f32 - 32.0).abs() * 100.0).max(0.0));
        let (rain, moist) =
            orographic_rain(&ridge, [100.0, 100.0], &RainParams::default()).expect("rain");
        let row = |f: &Heightfield, x0: u32, x1: u32| {
            (x0..x1).map(|x| f.get(x, 8)).sum::<f32>() / (x1 - x0) as f32
        };
        let (windward, lee, plain) = (row(&rain, 17, 33), row(&rain, 33, 48), row(&rain, 0, 16));
        assert!(windward > 5.0 * lee, "windward {windward} vs lee {lee}");
        assert!(
            windward > 5.0 * plain,
            "climb {windward} vs upwind plain {plain}"
        );
        // The air arrives saturated, leaves the range dry and re-moistens over the plain.
        assert!(
            moist.get(2, 8) > 0.99,
            "upwind humidity {}",
            moist.get(2, 8)
        );
        assert!(
            moist.get(50, 8) < 0.7,
            "shadow humidity {}",
            moist.get(50, 8)
        );
        assert!(
            moist.get(63, 8) > moist.get(50, 8),
            "recovery {} → {}",
            moist.get(50, 8),
            moist.get(63, 8)
        );
        assert!((rain.to_vec().iter().cloned().fold(0.0f32, f32::max) - 1.0).abs() < 1e-6);
        assert!(
            rain.to_vec().iter().filter(|v| **v >= 1.0).count() >= 10,
            "the top percentile saturates"
        );
        // The same ridge under an easterly mirrors: now the east face is the wet one.
        let (east, _) = orographic_rain(
            &ridge,
            [100.0, 100.0],
            &RainParams {
                wind_azimuth_deg: 90.0,
                ..RainParams::default()
            },
        )
        .expect("east");
        assert!(row(&east, 33, 48) > 5.0 * row(&east, 17, 33));
        assert!(
            orographic_rain(
                &ridge,
                [100.0, 100.0],
                &RainParams {
                    recovery_km: 0.0,
                    ..RainParams::default()
                }
            )
            .is_err()
        );
    }
}
