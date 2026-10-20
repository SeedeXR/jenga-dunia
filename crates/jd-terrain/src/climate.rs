//! Climate masks. First kernel: snow cover from the heightfield alone (`02-subsystems.md` §5),
//! in the form the research notes give for heightfields (Festenberg & Gumhold 2009/2011,
//! `docs/research/materials-geomorphology.md`): cover ∝ altitude band × slope retention ×
//! sun exposure × wind shelter, every factor a 0..1 multiplier on a smooth snow line.
//!
//! ponytail: no sky-visibility (ambient occlusion) term and no melt/accumulation time stepping —
//! a static coverage mask that biome rules, scatter and colour overlays can consume now.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;

/// Snow cover settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnowParams {
    /// Height where cover reaches half, metres.
    #[serde(default = "default_snow_line")]
    pub snow_line_m: f32,
    /// Width of the soft transition around the snow line, metres (> 0).
    #[serde(default = "default_transition")]
    pub transition_m: f32,
    /// Slope (degrees, (0, 90]) at which snow no longer holds; retention falls linearly to 0 there.
    #[serde(default = "default_max_slope")]
    pub max_slope_deg: f32,
    /// Compass direction of the sun (degrees clockwise from north; 180 = south, the northern-hemisphere default).
    #[serde(default = "default_south")]
    pub sun_azimuth_deg: f32,
    /// How much sun-facing slopes lose, 0..1 (0 = aspect ignored).
    #[serde(default = "default_half")]
    pub sun_strength: f32,
    /// Compass direction the wind blows from, degrees clockwise from north. In a graph, unset → the
    /// world's shared wind.
    #[serde(default = "default_west")]
    pub wind_azimuth_deg: f32,
    /// How much windward slopes lose to scouring, 0..1 (0 = wind ignored).
    #[serde(default)]
    pub wind_strength: f32,
}

fn default_snow_line() -> f32 {
    800.0
}
fn default_transition() -> f32 {
    150.0
}
fn default_max_slope() -> f32 {
    60.0
}
fn default_south() -> f32 {
    180.0
}
fn default_half() -> f32 {
    0.5
}
fn default_west() -> f32 {
    270.0
}

impl Default for SnowParams {
    fn default() -> Self {
        Self {
            snow_line_m: default_snow_line(),
            transition_m: default_transition(),
            max_slope_deg: default_max_slope(),
            sun_azimuth_deg: default_south(),
            sun_strength: default_half(),
            wind_azimuth_deg: default_west(),
            wind_strength: 0.0,
        }
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Snow cover 0..1 per cell. Row 0 is south, x grows east, so the downslope compass direction
/// (aspect) is `atan2(−dh/dx, −dh/dnorth)`.
pub fn snow_cover(h: &Heightfield, spacing_m: [f32; 2], p: &SnowParams) -> JdResult<Heightfield> {
    if !(p.transition_m > 0.0 && p.transition_m.is_finite()) {
        return Err(JdError::invalid("transition_m", "must be positive"));
    }
    if !(0.0 < p.max_slope_deg && p.max_slope_deg <= 90.0) {
        return Err(JdError::invalid("max_slope_deg", "must be within (0, 90]"));
    }
    for (name, v) in [
        ("sun_strength", p.sun_strength),
        ("wind_strength", p.wind_strength),
    ] {
        if !(0.0..=1.0).contains(&v) {
            return Err(JdError::invalid(name, "must be within 0..=1"));
        }
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let (w, hh) = (h.width(), h.height());
    let mut out = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    let (sun, wind) = (
        p.sun_azimuth_deg.to_radians(),
        p.wind_azimuth_deg.to_radians(),
    );
    out.fill_with(|x, y| {
        let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
        let (y0, y1) = (y.saturating_sub(1), (y + 1).min(hh - 1));
        let dx = (h.get(x1, y) - h.get(x0, y)) / ((x1 - x0).max(1) as f32 * spacing_m[0]);
        let dn = (h.get(x, y1) - h.get(x, y0)) / ((y1 - y0).max(1) as f32 * spacing_m[1]);
        let grad = (dx * dx + dn * dn).sqrt();
        let band = smoothstep((h.get(x, y) - p.snow_line_m) / p.transition_m + 0.5);
        let retention = (1.0 - grad.atan().to_degrees() / p.max_slope_deg).clamp(0.0, 1.0);
        // Aspect: compass bearing of the downslope direction. Flat cells have no aspect: the
        // directional factors fade in with the gradient so they never flicker on plains.
        let aspect = (-dx).atan2(-dn);
        let facing = (grad * 20.0).min(1.0); // full effect from a 1:20 grade upwards
        let sunlit = 0.5 * (1.0 + (aspect - sun).cos()) * facing;
        let windward = 0.5 * (1.0 + (aspect - wind).cos()) * facing;
        band * retention * (1.0 - p.sun_strength * sunlit) * (1.0 - p.wind_strength * windward)
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snow_follows_altitude_sheds_on_cliffs_and_prefers_shaded_lee_slopes() {
        // Ridge running east–west at row 16 of a 32-row field: north-facing slope above, south-facing below.
        let mut h = Heightfield::new(16, 32, 16, 0).expect("hf");
        h.fill_with(|_, y| 1000.0 - (y as f32 - 16.0).abs() * 10.0); // 1000 m crest, 10 m per 10 m cell = 45° slopes
        let sp = [10.0, 10.0];
        let p = SnowParams {
            snow_line_m: 800.0,
            transition_m: 100.0,
            max_slope_deg: 60.0,
            sun_azimuth_deg: 180.0,
            sun_strength: 0.5,
            ..SnowParams::default()
        };
        let s = snow_cover(&h, sp, &p).expect("snow");
        // Crest (flat, 1000 m) is fully covered; the valley floors (840 m, steep) carry less.
        assert!(s.get(8, 16) > 0.95, "crest {}", s.get(8, 16));
        // Below the band nothing: 1000 − 16·10 = 840 m is inside the band, row 0 = 840 m… use a lower field.
        let mut low = Heightfield::new(8, 8, 8, 0).expect("hf");
        low.fill_with(|_, _| 500.0);
        assert!(snow_cover(&low, sp, &p).expect("low").get(3, 3) < 1e-3);
        // South-facing slope (rows < 16: downslope direction = south) gets less than the north-facing one.
        let (south, north) = (s.get(8, 8), s.get(8, 24));
        assert!(north > south * 1.3, "north {north} south {south}");
        // Without the sun term both flanks match; a 45° slope keeps 1 − 45/60 = 25 % retention.
        let flat_sun = snow_cover(
            &h,
            sp,
            &SnowParams {
                sun_strength: 0.0,
                ..p.clone()
            },
        )
        .expect("no sun");
        assert!((flat_sun.get(8, 8) - flat_sun.get(8, 24)).abs() < 1e-5);
        assert!(
            (flat_sun.get(8, 20) - 0.25 * smoothstep((960.0 - 800.0) / 100.0 + 0.5)).abs() < 0.02,
            "{}",
            flat_sun.get(8, 20)
        );
        // Wind from the north scours the north-facing (windward) flank.
        let windy = snow_cover(
            &h,
            sp,
            &SnowParams {
                sun_strength: 0.0,
                wind_azimuth_deg: 0.0,
                wind_strength: 0.8,
                ..p.clone()
            },
        )
        .expect("wind");
        assert!(
            windy.get(8, 24) < windy.get(8, 8) * 0.5,
            "windward {} lee {}",
            windy.get(8, 24),
            windy.get(8, 8)
        );
        // Cliffs steeper than max_slope_deg hold nothing.
        let mut cliff = Heightfield::new(8, 8, 8, 0).expect("hf");
        cliff.fill_with(|x, _| 900.0 + x as f32 * 30.0); // 71.6°
        assert!(snow_cover(&cliff, sp, &p).expect("cliff").get(4, 4) < 1e-6);
        assert!(
            snow_cover(
                &h,
                sp,
                &SnowParams {
                    transition_m: 0.0,
                    ..p.clone()
                }
            )
            .is_err()
        );
        assert!(
            snow_cover(
                &h,
                sp,
                &SnowParams {
                    sun_strength: 1.5,
                    ..p.clone()
                }
            )
            .is_err()
        );
    }
}
