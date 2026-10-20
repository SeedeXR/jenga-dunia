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
                let wet: Vec<usize> = neighbours
                    .iter()
                    .filter(|(dx, dy)| (0..w).contains(&(x + dx)) && (0..hh).contains(&(y + dy)))
                    .map(|(dx, dy)| at(x + dx, y + dy))
                    .filter(|&j| sea[j])
                    .collect();
                if wet.is_empty() {
                    continue;
                }
                let exposure = wet.len() as f32 / 8.0;
                // The notch cuts down to one step below the level, so the cell floods next step
                // and exposes the next one inland: that is how the cliff retreats and a
                // wave-cut platform just under the surface is left behind.
                let cut = (p.rate_m * exposure).min((v[i] - (p.level_m - p.rate_m)).max(0.0));
                if cut <= 0.0 {
                    continue;
                }
                delta[i] -= cut;
                work[i] += cut;
                let share = cut / wet.len() as f32;
                for j in wet {
                    delta[j] += share;
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
    fn waves_cut_the_shore_fill_the_shallows_and_keep_the_mass() {
        // A ramp from −20 m (west, sea) to +60 m (east, land); sea level 0 crosses at column 16.
        let mut h = Heightfield::new(64, 16, 64, 0).expect("hf");
        h.fill_with(|x, _| -20.0 + x as f32 * 1.25);
        let p = CoastalParams {
            level_m: 0.0,
            band_m: 8.0,
            iterations: 40,
            rate_m: 0.5,
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
