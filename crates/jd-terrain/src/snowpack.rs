//! Snowpack: a time-stepped snow depth. Each step snow falls where it is cold enough and holds on
//! gentle ground, a degree-day melt follows the lapse rate and the sun, and wind drifts snow from
//! windward faces to the cell downwind (v. Festenberg & Gumhold 2009 in spirit; `docs/research/
//! materials-geomorphology.md` B8). `jd.climate.snow@1` is a static snapshot; this one evolves,
//! and a keyed `temperature_c` over the timeline melts it season by season.
//!
//! ponytail: one downwind neighbour for drift, one downhill neighbour for avalanches, no albedo or
//! density feedback; a step is "a day" only in the sense that the rates say so.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;

/// Snowpack settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnowpackParams {
    /// Steps (1..=2000).
    #[serde(default = "d_steps")]
    pub steps: u32,
    /// Snowfall per step on flat ground where it is colder than +1 °C, metres of snow (≥ 0).
    #[serde(default = "d_accumulation")]
    pub accumulation_m: f32,
    /// Air temperature at sea level, °C — key it over the timeline for a season.
    #[serde(default = "d_temperature")]
    pub temperature_c: f32,
    /// Lapse rate, °C lost per km of height (≥ 0).
    #[serde(default = "d_lapse")]
    pub lapse_c_per_km: f32,
    /// Melt per step per degree above freezing, metres of snow (≥ 0).
    #[serde(default = "d_melt")]
    pub melt_m_per_c: f32,
    /// Slope at which snow no longer holds (0, 90]; snowfall retained falls linearly to 0 there.
    #[serde(default = "d_max_slope")]
    pub max_slope_deg: f32,
    /// Compass direction of the sun, degrees clockwise from north (180 = south).
    #[serde(default = "d_south")]
    pub sun_azimuth_deg: f32,
    /// Sun-facing slopes run up to `sun_strength` × 5 °C warmer (0..1).
    #[serde(default = "d_half")]
    pub sun_strength: f32,
    /// Compass direction the wind blows from, degrees clockwise from north. In a graph, unset →
    /// the world's shared wind.
    #[serde(default = "d_west")]
    pub wind_azimuth_deg: f32,
    /// Fraction of a windward face's snow that drifts to the cell downwind each step (0..1).
    #[serde(default = "d_drift")]
    pub wind_strength: f32,
    /// Depth that counts as full cover in the `cover` output, metres (> 0).
    #[serde(default = "d_cover")]
    pub cover_depth_m: f32,
    /// Slope above which snow avalanches each step, degrees (0, 90].
    #[serde(default = "d_avalanche_slope")]
    pub avalanche_slope_deg: f32,
    /// Share of a steep cell's snow that slides to its lowest neighbour each step (0..1; 0 = off).
    #[serde(default = "d_avalanche")]
    pub avalanche_strength: f32,
}

fn d_avalanche_slope() -> f32 {
    38.0
}
fn d_avalanche() -> f32 {
    0.5
}

fn d_steps() -> u32 {
    50
}
fn d_accumulation() -> f32 {
    0.02
}
fn d_temperature() -> f32 {
    -5.0
}
fn d_lapse() -> f32 {
    6.5
}
fn d_melt() -> f32 {
    0.004
}
fn d_max_slope() -> f32 {
    60.0
}
fn d_south() -> f32 {
    180.0
}
fn d_half() -> f32 {
    0.5
}
fn d_west() -> f32 {
    270.0
}
fn d_drift() -> f32 {
    0.1
}
fn d_cover() -> f32 {
    0.2
}

impl Default for SnowpackParams {
    fn default() -> Self {
        Self {
            steps: d_steps(),
            accumulation_m: d_accumulation(),
            temperature_c: d_temperature(),
            lapse_c_per_km: d_lapse(),
            melt_m_per_c: d_melt(),
            max_slope_deg: d_max_slope(),
            sun_azimuth_deg: d_south(),
            sun_strength: d_half(),
            wind_azimuth_deg: d_west(),
            wind_strength: d_drift(),
            cover_depth_m: d_cover(),
            avalanche_slope_deg: d_avalanche_slope(),
            avalanche_strength: d_avalanche(),
        }
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Run the snowpack for `p.steps` from bare ground. Returns (`depth` in metres of snow, `cover`
/// 0..1 = smoothstep(depth / cover_depth_m)). Row 0 is south, x grows east.
pub fn snowpack(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &SnowpackParams,
) -> JdResult<(Heightfield, Heightfield)> {
    if !(1..=2000).contains(&p.steps) {
        return Err(JdError::invalid("steps", "must be within 1..=2000"));
    }
    for (name, v) in [
        ("accumulation_m", p.accumulation_m),
        ("lapse_c_per_km", p.lapse_c_per_km),
        ("melt_m_per_c", p.melt_m_per_c),
    ] {
        if !(v >= 0.0 && v.is_finite()) {
            return Err(JdError::invalid(name, "must be finite and ≥ 0"));
        }
    }
    for (name, v) in [
        ("sun_strength", p.sun_strength),
        ("wind_strength", p.wind_strength),
        ("avalanche_strength", p.avalanche_strength),
    ] {
        if !(0.0..=1.0).contains(&v) {
            return Err(JdError::invalid(name, "must be within 0..=1"));
        }
    }
    for (name, v) in [
        ("max_slope_deg", p.max_slope_deg),
        ("avalanche_slope_deg", p.avalanche_slope_deg),
    ] {
        if !(0.0 < v && v <= 90.0) {
            return Err(JdError::invalid(name, "must be within (0, 90]"));
        }
    }
    if !(p.cover_depth_m > 0.0 && p.cover_depth_m.is_finite()) {
        return Err(JdError::invalid("cover_depth_m", "must be positive"));
    }
    for (name, v) in [
        ("temperature_c", p.temperature_c),
        ("wind_azimuth_deg", p.wind_azimuth_deg),
        ("sun_azimuth_deg", p.sun_azimuth_deg),
    ] {
        if !v.is_finite() {
            return Err(JdError::invalid(name, "must be finite"));
        }
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let (w, hh) = (h.width(), h.height());
    let n = (w * hh) as usize;
    let (sun, wind) = (
        p.sun_azimuth_deg.to_radians(),
        p.wind_azimuth_deg.to_radians(),
    );
    // Per-cell constants: retention from slope, sun and windward exposure from aspect, local air temperature.
    let mut retention = vec![0.0f32; n];
    let mut t_local = vec![0.0f32; n];
    let mut windward = vec![0.0f32; n];
    let mut steep = vec![false; n];
    for y in 0..hh {
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
            let (y0, y1) = (y.saturating_sub(1), (y + 1).min(hh - 1));
            let dx = (h.get(x1, y) - h.get(x0, y)) / ((x1 - x0).max(1) as f32 * spacing_m[0]);
            let dn = (h.get(x, y1) - h.get(x, y0)) / ((y1 - y0).max(1) as f32 * spacing_m[1]);
            let grad = (dx * dx + dn * dn).sqrt();
            let aspect = (-dx).atan2(-dn);
            let facing = (grad * 20.0).min(1.0);
            let sunlit = 0.5 * (1.0 + (aspect - sun).cos()) * facing;
            let i = (y * w + x) as usize;
            let slope_deg = grad.atan().to_degrees();
            retention[i] = (1.0 - slope_deg / p.max_slope_deg).clamp(0.0, 1.0);
            steep[i] = p.avalanche_strength > 0.0 && slope_deg > p.avalanche_slope_deg;
            windward[i] = 0.5 * (1.0 + (aspect - wind).cos()) * facing;
            t_local[i] = p.temperature_c - p.lapse_c_per_km * h.get(x, y) / 1000.0
                + p.sun_strength * 5.0 * sunlit;
        }
    }
    // The cell downwind: the wind blows towards azimuth + 180°; rows grow north.
    let to = p.wind_azimuth_deg + 180.0;
    let (ddx, ddy) = (
        to.to_radians().sin().round() as i64,
        to.to_radians().cos().round() as i64,
    );
    // The avalanche loop reads the terrain flat; nothing to read when it is off.
    let hv = if p.avalanche_strength > 0.0 {
        h.to_vec()
    } else {
        Vec::new()
    };
    let mut depth = vec![0.0f32; n];
    let mut next = vec![0.0f32; n];
    for _ in 0..p.steps {
        for (i, d) in depth.iter_mut().enumerate() {
            if t_local[i] < 1.0 {
                *d += p.accumulation_m * retention[i];
            }
            *d = (*d - p.melt_m_per_c * t_local[i].max(0.0)).max(0.0);
        }
        // Avalanche: a steep cell sends a share of its snow to the lowest neighbouring snow surface.
        next.copy_from_slice(&depth);
        for y in 0..hh as i64 {
            for x in 0..w as i64 {
                let i = (y * w as i64 + x) as usize;
                if !steep[i] || depth[i] <= 0.0 {
                    continue;
                }
                let surface = |x: i64, y: i64| {
                    let j = (y * w as i64 + x) as usize;
                    (hv[j] + depth[j], j)
                };
                let (mut low, mut target) = surface(x, y);
                for (nx, ny) in [
                    (x - 1, y - 1),
                    (x, y - 1),
                    (x + 1, y - 1),
                    (x - 1, y),
                    (x + 1, y),
                    (x - 1, y + 1),
                    (x, y + 1),
                    (x + 1, y + 1),
                ] {
                    if nx >= 0 && ny >= 0 && nx < w as i64 && ny < hh as i64 {
                        let (s, j) = surface(nx, ny);
                        if s < low {
                            (low, target) = (s, j);
                        }
                    }
                }
                if target != i {
                    let moved = p.avalanche_strength * depth[i];
                    next[i] -= moved;
                    next[target] += moved;
                }
            }
        }
        std::mem::swap(&mut depth, &mut next);
        // Drift: a share of each windward face's snow moves one cell downwind (stays put at the edge).
        next.copy_from_slice(&depth);
        for y in 0..hh as i64 {
            for x in 0..w as i64 {
                let i = (y * w as i64 + x) as usize;
                let moved = p.wind_strength * windward[i] * depth[i];
                if moved <= 0.0 {
                    continue;
                }
                let (tx, ty) = (x + ddx, y + ddy);
                if tx < 0 || ty < 0 || tx >= w as i64 || ty >= hh as i64 {
                    continue;
                }
                next[i] -= moved;
                next[(ty * w as i64 + tx) as usize] += moved;
            }
        }
        std::mem::swap(&mut depth, &mut next);
    }
    let mut out_depth = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    out_depth.fill_with(|x, y| depth[(y * w + x) as usize]);
    let mut cover = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    cover.fill_with(|x, y| smoothstep(depth[(y * w + x) as usize] / p.cover_depth_m));
    Ok((out_depth, cover))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snow_builds_with_altitude_melts_in_warmth_and_drifts_to_the_lee() {
        // A 32×32 ramp from sea level (west) to 2 000 m (east), 100 m cells.
        let mut h = Heightfield::new(32, 32, 64, 0).expect("hf");
        h.fill_with(|x, _| x as f32 / 31.0 * 2000.0);
        // A shoulder season: +4 °C at sea level melts the lowlands, the summits stay under −7 °C.
        let cold = SnowpackParams {
            temperature_c: 4.0,
            wind_strength: 0.0,
            ..SnowpackParams::default()
        };
        let (depth, cover) = snowpack(&h, [100.0, 100.0], &cold).expect("cold");
        let mean = |f: &Heightfield, x0: u32, x1: u32| {
            (x0..x1)
                .flat_map(|x| (0..32).map(move |y| (x, y)))
                .map(|(x, y)| f.get(x, y))
                .sum::<f32>()
                / ((x1 - x0) * 32) as f32
        };
        assert!(
            mean(&depth, 16, 32) > mean(&depth, 0, 16) * 1.5,
            "deeper up high: {} vs {}",
            mean(&depth, 0, 16),
            mean(&depth, 16, 32)
        );
        assert!(
            cover.get(31, 16) > 0.9 && depth.get(31, 16) > 0.3,
            "summits are white: {} m",
            depth.get(31, 16)
        );
        assert!(
            depth.get(0, 16) < 1e-3,
            "the lowlands are bare: {} m",
            depth.get(0, 16)
        );
        // A warm summer (+15 °C at sea level, +2 °C on the summit) melts it all away.
        let (summer, _) = snowpack(
            &h,
            [100.0, 100.0],
            &SnowpackParams {
                temperature_c: 15.0,
                steps: 200,
                wind_strength: 0.0,
                ..SnowpackParams::default()
            },
        )
        .expect("summer");
        assert!(
            summer.to_vec().iter().all(|d| *d < 1e-3),
            "summer melts everything: max {}",
            summer.to_vec().iter().cloned().fold(0.0, f32::max)
        );
        // A north–south ridge under a westerly: the lee (east) face ends up deeper than the windward face.
        let mut ridge = Heightfield::new(32, 32, 64, 0).expect("ridge");
        ridge.fill_with(|x, _| 1500.0 - (x as f32 - 16.0).abs() * 40.0);
        let (d, _) = snowpack(
            &ridge,
            [100.0, 100.0],
            &SnowpackParams {
                wind_strength: 0.3,
                ..SnowpackParams::default()
            },
        )
        .expect("ridge");
        assert!(
            mean(&d, 17, 24) > mean(&d, 8, 15) * 1.2,
            "lee deeper: windward {} lee {}",
            mean(&d, 8, 15),
            mean(&d, 17, 24)
        );
        let total = d.to_vec().iter().sum::<f32>();
        assert!(total > 0.0);
        // Avalanche: a 45° face from 1 000 m down to a plain at 0 m — the slides empty the face
        // and pile up at its foot; nothing melts (cold everywhere), so the mass is the same.
        let mut face = Heightfield::new(32, 8, 64, 0).expect("face");
        face.fill_with(|x, _| (10.0 - x as f32).max(0.0) * 100.0);
        let run = |strength: f32| {
            snowpack(
                &face,
                [100.0, 100.0],
                &SnowpackParams {
                    wind_strength: 0.0,
                    avalanche_strength: strength,
                    ..SnowpackParams::default()
                },
            )
            .expect("face")
            .0
        };
        let (still, slid) = (run(0.0), run(0.5));
        let row = |f: &Heightfield, x0: u32, x1: u32| {
            (x0..x1).map(|x| f.get(x, 4)).sum::<f32>() / (x1 - x0) as f32
        };
        assert!(
            row(&slid, 1, 9) < 0.3 * row(&still, 1, 9),
            "the face empties: {} → {}",
            row(&still, 1, 9),
            row(&slid, 1, 9)
        );
        assert!(
            slid.get(10, 4) > 1.5 * slid.get(20, 4),
            "a cone at the foot: {} vs plain {}",
            slid.get(10, 4),
            slid.get(20, 4)
        );
        let mass = |f: &Heightfield| f.to_vec().iter().sum::<f32>();
        assert!(
            (mass(&slid) - mass(&still)).abs() < 1e-3 * mass(&still),
            "mass {} vs {}",
            mass(&slid),
            mass(&still)
        );
        assert!(
            snowpack(
                &h,
                [100.0, 100.0],
                &SnowpackParams {
                    steps: 0,
                    ..SnowpackParams::default()
                }
            )
            .is_err()
        );
        assert!(
            snowpack(
                &h,
                [100.0, 100.0],
                &SnowpackParams {
                    wind_strength: 2.0,
                    ..SnowpackParams::default()
                }
            )
            .is_err()
        );
    }
}
