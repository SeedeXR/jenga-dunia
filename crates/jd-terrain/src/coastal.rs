//! Coastal erosion: waves attack the land within reach of the sea and lay the material down on
//! the sea floor next to it — cliffs retreat, a wave-cut platform and beaches form
//! (`docs/research/materials-geomorphology.md`, "Coastal": erosion dominant on exposed coasts,
//! deposition where it slows). Mass is conserved exactly: what the waves take off a shore cell
//! is shared among its ocean neighbours in the same step.
//!
//! ponytail: exposure is the share of ocean among the 8 neighbours (no fetch, no wave direction,
//! no longshore drift) and the ocean is re-flooded from the field border every step so a beach
//! that builds above the level turns into land. Spits, bars and deltas need the drift term.

use std::sync::atomic::AtomicBool;

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::flow::sea_level;
use crate::sim::check_cancel;

/// Coastal erosion settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoastalParams {
    /// Sea level, metres.
    #[serde(default)]
    pub level_m: f32,
    /// How far above the level the waves reach (notch height / storm run-up), metres (> 0).
    #[serde(default = "d_band")]
    pub band_m: f32,
    /// Steps.
    #[serde(default = "d_iterations")]
    pub iterations: u32,
    /// Metres taken off a fully exposed shore cell per step (> 0).
    #[serde(default = "d_rate")]
    pub rate_m: f32,
    /// Direction the waves come from (the wind), degrees clockwise from north. In a graph, unset →
    /// the world's shared wind. Only matters when `drift` > 0.
    #[serde(default)]
    pub wind_azimuth_deg: Option<f32>,
    /// Fetch and longshore drift, 0..1: shores facing the wind erode up to (1 + drift)× more than
    /// sheltered ones, and the eroded material is laid down preferentially on the sea floor
    /// downwind — spits and bars grow along the coast. 0 = isotropic waves (the plain model).
    #[serde(default)]
    pub drift: f32,
}

fn d_band() -> f32 {
    8.0
}
fn d_iterations() -> u32 {
    50
}
fn d_rate() -> f32 {
    0.25
}

impl Default for CoastalParams {
    fn default() -> Self {
        Self {
            level_m: 0.0,
            band_m: d_band(),
            iterations: d_iterations(),
            rate_m: d_rate(),
            wind_azimuth_deg: None,
            drift: 0.0,
        }
    }
}

/// Erode the shore of `h` for `p.iterations` steps. Returns the terrain and a 0..1 `shore` mask of
/// how much wave work each cell received, normalised to the most-worked cell.
pub fn coastal_erode(
    h: &Heightfield,
    p: &CoastalParams,
    cancel: Option<&AtomicBool>,
) -> JdResult<(Heightfield, Heightfield)> {
    if !p.level_m.is_finite() {
        return Err(JdError::invalid("level_m", "must be finite"));
    }
    if !(p.band_m > 0.0 && p.band_m.is_finite()) {
        return Err(JdError::invalid("band_m", "must be positive"));
    }
    if !(p.rate_m > 0.0 && p.rate_m.is_finite()) {
        return Err(JdError::invalid("rate_m", "must be positive"));
    }
    if !(0.0..=1.0).contains(&p.drift) {
        return Err(JdError::invalid("drift", "must be within 0..=1"));
    }
    // Wave travel direction as (east, north) components: the wind blows towards azimuth + 180°.
    let travel = {
        let to = (p.wind_azimuth_deg.unwrap_or(270.0) + 180.0).to_radians();
        (to.sin(), to.cos())
    };
    let (w, hh) = (h.width() as i64, h.height() as i64);
    let n = (w * hh) as usize;
    let mut cur = h.clone();
    let mut work = vec![0.0f32; n];
    let at = |x: i64, y: i64| (y.clamp(0, hh - 1) * w + x.clamp(0, w - 1)) as usize;
    let neighbours: [(i64, i64); 8] = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ];
    for _ in 0..p.iterations {
        check_cancel(cancel)?;
        // Hard ocean mask: below the level and connected to the border.
        let (ocean, _) = sea_level(&cur, p.level_m, 0.0)?;
        let sea: Vec<bool> = ocean.to_vec().iter().map(|v| *v > 0.5).collect();
        let v = cur.to_vec();
        let mut delta = vec![0.0f32; n];
        for y in 0..hh {
            for x in 0..w {
                let i = at(x, y);
                if sea[i] || v[i] > p.level_m + p.band_m {
                    continue;
                }
                // Ocean neighbours with their unit direction from this cell (east, north).
                let wet: Vec<(usize, (f32, f32))> = neighbours
                    .iter()
                    .filter(|(dx, dy)| (0..w).contains(&(x + dx)) && (0..hh).contains(&(y + dy)))
                    .filter(|(dx, dy)| sea[at(x + dx, y + dy)])
                    .map(|(dx, dy)| {
                        let len = ((dx * dx + dy * dy) as f32).sqrt();
                        (at(x + dx, y + dy), (*dx as f32 / len, *dy as f32 / len))
                    })
                    .collect();
                if wet.is_empty() {
                    continue;
                }
                // Fetch: waves arriving from an ocean neighbour travel along −dir; the more that
                // agrees with the wind's travel the harder they hit. Drift 0 counts every neighbour once.
                let exposure = (wet
                    .iter()
                    .map(|(_, d)| 1.0 + p.drift * (-(d.0 * travel.0 + d.1 * travel.1)))
                    .sum::<f32>()
                    / 8.0)
                    .min(1.0);
                // The notch cuts down to one step below the level, so the cell floods next step
                // and exposes the next one inland: that is how the cliff retreats and a
                // wave-cut platform just under the surface is left behind.
                let cut = (p.rate_m * exposure).min((v[i] - (p.level_m - p.rate_m)).max(0.0));
                if cut <= 0.0 {
                    continue;
                }
                delta[i] -= cut;
                work[i] += cut;
                // Longshore drift: the material goes to the sea floor downwind first.
                let weights: Vec<f32> = wet
                    .iter()
                    .map(|(_, d)| 1.0 + p.drift * (d.0 * travel.0 + d.1 * travel.1))
                    .collect();
                let total: f32 = weights.iter().sum();
                for ((j, _), wgt) in wet.iter().zip(&weights) {
                    // drift 1 with the only sea cell straight upwind weighs 0: share evenly then.
                    delta[*j] += cut
                        * if total > 0.0 {
                            wgt / total
                        } else {
                            1.0 / wet.len() as f32
                        };
                }
            }
        }
        cur.fill_with(|x, y| {
            v[(y as usize) * w as usize + x as usize]
                + delta[(y as usize) * w as usize + x as usize]
        });
    }
    let peak = work.iter().cloned().fold(0.0f32, f32::max).max(1e-9);
    let mut shore = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    shore.fill_with(|x, y| work[(y as usize) * w as usize + x as usize] / peak);
    Ok((cur, shore))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drift_moves_the_deposits_downwind_and_keeps_the_mass() {
        // A 20 m island on a 64² sea at −10 m; westerly waves travel east.
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| {
            let d = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
            (20.0 - d * 1.5).max(-10.0)
        });
        let base = CoastalParams {
            level_m: 0.0,
            band_m: 6.0,
            iterations: 30,
            rate_m: 0.5,
            wind_azimuth_deg: Some(270.0),
            ..CoastalParams::default()
        };
        let run = |drift: f32| {
            coastal_erode(
                &h,
                &CoastalParams {
                    drift,
                    ..base.clone()
                },
                None,
            )
            .expect("coast")
            .0
        };
        let (plain, drifted) = (run(0.0), run(0.8));
        let sum = |f: &Heightfield| f.to_vec().iter().map(|v| *v as f64).sum::<f64>();
        assert!(
            (sum(&h) - sum(&drifted)).abs() < 1e-2,
            "mass kept under drift"
        );
        assert_ne!(plain.content_hash(), drifted.content_hash());
        // Longshore drift on a straight north–south coast under a northerly: the deposits' centroid
        // moves south (rows grow north), where the isotropic model keeps it on the mid row.
        let mut ramp = Heightfield::new(64, 32, 64, 0).expect("ramp");
        ramp.fill_with(|x, _| -20.0 + x as f32 * 1.25);
        let along = |drift: f32| {
            let f = coastal_erode(
                &ramp,
                &CoastalParams {
                    drift,
                    wind_azimuth_deg: Some(0.0),
                    ..base.clone()
                },
                None,
            )
            .expect("ramp")
            .0;
            let (mut m, mut my) = (0.0f64, 0.0f64);
            for y in 0..32 {
                for x in 0..64 {
                    let d = (f.get(x, y) - ramp.get(x, y)).max(0.0) as f64;
                    m += d;
                    my += d * y as f64;
                }
            }
            my / m.max(1e-9)
        };
        assert!(
            (along(0.0) - 15.5).abs() < 0.3,
            "isotropic deposits centre on the mid row: {}",
            along(0.0)
        );
        assert!(
            along(0.8) < along(0.0) - 0.5,
            "deposits move downwind (south): {} → {}",
            along(0.0),
            along(0.8)
        );
        // Exposure follows the wind: the west (windward) shore is cut more than the east shore.
        let cut = |f: &Heightfield, x0: u32, x1: u32| {
            (x0..x1)
                .flat_map(|x| (0..64).map(move |y| (x, y)))
                .map(|(x, y)| (h.get(x, y) - f.get(x, y)).max(0.0) as f64)
                .sum::<f64>()
        };
        assert!(
            cut(&drifted, 0, 32) > cut(&drifted, 32, 64) * 1.2,
            "windward shore cut harder: {} vs {}",
            cut(&drifted, 0, 32),
            cut(&drifted, 32, 64)
        );
        assert!(
            coastal_erode(
                &h,
                &CoastalParams {
                    drift: 1.5,
                    ..base.clone()
                },
                None
            )
            .is_err()
        );
    }

    #[test]
    fn waves_cut_the_shore_fill_the_shallows_and_keep_the_mass() {
        // A ramp from −20 m (west, sea) to +60 m (east, land); sea level 0 crosses at column 16.
        let mut h = Heightfield::new(64, 16, 64, 0).expect("hf");
        h.fill_with(|x, _| -20.0 + x as f32 * 1.25);
        let p = CoastalParams {
            level_m: 0.0,
            band_m: 8.0,
            iterations: 40,
            rate_m: 0.5,
            ..CoastalParams::default()
        };
        let (out, shore) = coastal_erode(&h, &p, None).expect("coast");
        let sum = |f: &Heightfield| f.to_vec().iter().map(|v| *v as f64).sum::<f64>();
        assert!(
            (sum(&h) - sum(&out)).abs() < 1e-2,
            "mass {} → {}",
            sum(&h),
            sum(&out)
        );
        // The first land columns are cut down towards the level; the last sea column is raised.
        assert!(
            out.get(17, 8) < h.get(17, 8) - 1.0,
            "shore cut: {} → {}",
            h.get(17, 8),
            out.get(17, 8)
        );
        assert!(
            out.get(15, 8) > h.get(15, 8) + 1.0,
            "shallows filled: {} → {}",
            h.get(15, 8),
            out.get(15, 8)
        );
        assert!(
            out.get(17, 8) >= -p.rate_m,
            "the platform stays within one step of the level"
        );
        // Far inland and far out to sea nothing changes.
        assert_eq!(out.get(60, 8), h.get(60, 8));
        assert_eq!(out.get(2, 8), h.get(2, 8));
        // The shore mask peaks on the worked cells and is zero where no wave reached.
        assert!(shore.get(17, 8) > 0.5 && shore.get(60, 8) == 0.0 && shore.get(2, 8) == 0.0);
        // Deterministic; bad params refused.
        let (again, _) = coastal_erode(&h, &p, None).expect("again");
        assert_eq!(out.to_vec(), again.to_vec());
        assert!(
            coastal_erode(
                &h,
                &CoastalParams {
                    band_m: 0.0,
                    ..p.clone()
                },
                None
            )
            .is_err()
        );
        assert!(
            coastal_erode(
                &h,
                &CoastalParams {
                    rate_m: -1.0,
                    ..p.clone()
                },
                None
            )
            .is_err()
        );
    }
}
