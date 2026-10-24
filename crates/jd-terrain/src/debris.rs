//! Rockfall debris: cliffs shed blocks that roll down the fall line and come to rest as talus
//! cones and boulder fields at the slope foot — World Creator's Debris simulation layer, done as
//! a run-out model instead of splats.
//!
//! Sources are cells steeper than `source_slope_deg`. Each releases `release_m` of rock (times an
//! optional 0..1 `source` mask) as `blocks_per_cell` blocks of seeded sizes in
//! [`size_min_m`, `size_max_m`]; a block's volume is ∝ size³ and the source loses exactly what its
//! blocks carry. A block steps to a downhill neighbour (chosen with probability ∝ slope) and keeps
//! going while either the ground ahead is steeper than the angle of repose (`repose_deg`: scree
//! cannot rest there) or the energy line back to its source is still steeper than its reach angle
//! — the Fahrböschung of Heim (1932, ≥ 30°) and the shadow angle of Evans & Hungr (1993, ≥ ~27°,
//! measured from the talus apex), `docs/research/simulation-debris-lava.md`. `sorting` gives big
//! blocks the full `reach_deg` and small ones the repose angle, so boulders run out past the scree
//! (fall sorting). Blocks deposit where they stop; later blocks roll over earlier deposits, so
//! cones build up.
//!
//! ponytail: blocks run one at a time in row-major source order (deterministic, single thread);
//! no bouncing or fragmentation. A ballistic first leg off overhangs is the upgrade.

use std::sync::atomic::AtomicBool;

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::sim::check_cancel;

/// Debris settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebrisParams {
    /// Block seed.
    #[serde(default)]
    pub seed: u64,
    /// Cells steeper than this shed rock, degrees (0, 90).
    #[serde(default = "d_source")]
    pub source_slope_deg: f32,
    /// Depth of rock each source cell releases, metres (> 0).
    #[serde(default = "d_release")]
    pub release_m: f32,
    /// Blocks per source cell (1..=64).
    #[serde(default = "d_blocks")]
    pub blocks_per_cell: u32,
    /// Energy-line (Fahrböschung) angle of the largest blocks, degrees (0, 90): a block still
    /// rolls while the line back to its source is steeper than this.
    #[serde(default = "d_reach")]
    pub reach_deg: f32,
    /// Angle of repose of scree, degrees (0, 90): no block rests on ground steeper than this.
    #[serde(default = "d_repose")]
    pub repose_deg: f32,
    /// Smallest block, metres (> 0).
    #[serde(default = "d_size_min")]
    pub size_min_m: f32,
    /// Largest block, metres (≥ `size_min_m`).
    #[serde(default = "d_size_max")]
    pub size_max_m: f32,
    /// Fall sorting 0..1: at 1 the smallest blocks stop at the repose angle and only the largest
    /// use `reach_deg`; at 0 every block uses `reach_deg`.
    #[serde(default = "d_sorting")]
    pub sorting: f32,
    /// Longest path a block may roll, metres (> 0).
    #[serde(default = "d_max_run")]
    pub max_run_m: f32,
}

fn d_source() -> f32 {
    45.0
}
fn d_release() -> f32 {
    0.5
}
fn d_blocks() -> u32 {
    4
}
fn d_reach() -> f32 {
    30.0
}
fn d_repose() -> f32 {
    35.0
}
fn d_size_min() -> f32 {
    0.3
}
fn d_size_max() -> f32 {
    3.0
}
fn d_sorting() -> f32 {
    0.5
}
fn d_max_run() -> f32 {
    3000.0
}

impl Default for DebrisParams {
    fn default() -> Self {
        Self {
            seed: 0,
            source_slope_deg: d_source(),
            release_m: d_release(),
            blocks_per_cell: d_blocks(),
            reach_deg: d_reach(),
            repose_deg: d_repose(),
            size_min_m: d_size_min(),
            size_max_m: d_size_max(),
            sorting: d_sorting(),
            max_run_m: d_max_run(),
        }
    }
}

/// What a debris run produced.
#[derive(Debug)]
pub struct DebrisResult {
    /// Terrain after release (cliffs lowered) and deposition (talus raised).
    pub out: Heightfield,
    /// Block cover 0..1 per cell: `1 − exp(−blocks / blocks_per_cell)` — a scatter mask for boulders.
    pub debris: Heightfield,
    /// Mean size of the blocks resting in each cell, metres (0 = none).
    pub size: Heightfield,
    /// Blocks released.
    pub blocks: usize,
    /// Rock moved, m³.
    pub moved_m3: f64,
}

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}
fn unit(x: u64) -> f32 {
    (x >> 40) as f32 / (1u64 << 24) as f32
}

const NB: [(i64, i64); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Run the rockfall over `h`.
pub fn rockfall(
    h: &Heightfield,
    spacing_m: [f32; 2],
    source: Option<&Heightfield>,
    p: &DebrisParams,
    cancel: Option<&AtomicBool>,
) -> JdResult<DebrisResult> {
    for (name, deg) in [
        ("source_slope_deg", p.source_slope_deg),
        ("reach_deg", p.reach_deg),
        ("repose_deg", p.repose_deg),
    ] {
        if !(0.0 < deg && deg < 90.0) {
            return Err(JdError::invalid(name, "must be within (0, 90)"));
        }
    }
    if !(p.release_m > 0.0 && p.release_m.is_finite()) {
        return Err(JdError::invalid("release_m", "must be positive"));
    }
    if !(1..=64).contains(&p.blocks_per_cell) {
        return Err(JdError::invalid("blocks_per_cell", "must be within 1..=64"));
    }
    if !(p.size_min_m > 0.0 && p.size_max_m >= p.size_min_m && p.size_max_m.is_finite()) {
        return Err(JdError::invalid(
            "size_max_m",
            "need 0 < size_min_m ≤ size_max_m",
        ));
    }
    if !(0.0..=1.0).contains(&p.sorting) {
        return Err(JdError::invalid("sorting", "must be within 0..=1"));
    }
    if !(p.max_run_m > 0.0 && p.max_run_m.is_finite()) {
        return Err(JdError::invalid("max_run_m", "must be positive"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    if let Some(s) = source
        && (s.width(), s.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid("source", "field size differs from `in`"));
    }
    let (w, hh) = (h.width() as i64, h.height() as i64);
    let idx = |x: i64, y: i64| (y * w + x) as usize;
    let dist = NB.map(|(dx, dy)| (dx as f32 * spacing_m[0]).hypot(dy as f32 * spacing_m[1]));
    let cell_m2 = (spacing_m[0] * spacing_m[1]) as f64;
    let mut z = h.to_vec();
    let src = source.map(|s| s.to_vec());
    let (tan_src, tan_repose, tan_reach) = (
        p.source_slope_deg.to_radians().tan(),
        p.repose_deg.to_radians().tan(),
        p.reach_deg.to_radians().tan(),
    );
    // Sources from the input surface (not updated as blocks leave): steepest drop to a neighbour.
    let steepest = |z: &[f32], x: i64, y: i64| -> f32 {
        let here = z[idx(x, y)];
        (0..8)
            .filter_map(|k| {
                let (nx, ny) = (x + NB[k].0, y + NB[k].1);
                (nx >= 0 && ny >= 0 && nx < w && ny < hh).then(|| (here - z[idx(nx, ny)]) / dist[k])
            })
            .fold(0.0f32, f32::max)
    };
    let sources: Vec<(i64, i64, f32)> = (0..hh)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let m = src.as_ref().map_or(1.0, |s| s[idx(x, y)].clamp(0.0, 1.0));
            (m > 0.0 && steepest(&z, x, y) > tan_src).then_some((x, y, m))
        })
        .collect();
    let mut count = vec![0u32; z.len()];
    let mut size_sum = vec![0f32; z.len()];
    let mut moved = 0f64;
    let n = p.blocks_per_cell as usize;
    let mut state = mix(p.seed ^ 0x6465_6272_6973_0000); // "debris"
    let mut sizes = vec![0f32; n];
    for (si, &(sx, sy, m)) in sources.iter().enumerate() {
        if si % 4096 == 0 {
            check_cancel(cancel)?;
        }
        for s in sizes.iter_mut() {
            state = mix(state);
            *s = p.size_min_m + (p.size_max_m - p.size_min_m) * unit(state);
        }
        let cubes: f32 = sizes.iter().map(|s| s.powi(3)).sum();
        let depth = p.release_m * m; // metres of rock this source loses
        z[idx(sx, sy)] -= depth;
        moved += depth as f64 * cell_m2;
        let z_top = z[idx(sx, sy)] + depth; // the energy line starts at the original cliff cell
        for &size in &sizes {
            let share = depth * size.powi(3) / cubes; // this block's thickness when spread on one cell
            let frac = if p.size_max_m > p.size_min_m {
                (size - p.size_min_m) / (p.size_max_m - p.size_min_m)
            } else {
                1.0
            };
            let tan_stop = tan_reach + p.sorting * (1.0 - frac) * (tan_repose - tan_reach);
            let (mut x, mut y, mut run) = (sx, sy, 0.0f32);
            loop {
                let here = z[idx(x, y)];
                let mut total = 0.0f32;
                let mut slopes = [0.0f32; 8];
                for k in 0..8 {
                    let (nx, ny) = (x + NB[k].0, y + NB[k].1);
                    if nx >= 0 && ny >= 0 && nx < w && ny < hh {
                        slopes[k] = ((here - z[idx(nx, ny)]) / dist[k]).max(0.0);
                        total += slopes[k];
                    }
                }
                let steep = slopes.iter().copied().fold(0.0f32, f32::max);
                let energy = if run > 0.0 {
                    (z_top - here) / run
                } else {
                    f32::INFINITY
                };
                if total <= 0.0 || run >= p.max_run_m || (steep <= tan_repose && energy <= tan_stop)
                {
                    break;
                }
                state = mix(state);
                let mut pick = unit(state) * total;
                let mut k = 0;
                while k < 7 && (slopes[k] == 0.0 || pick > slopes[k]) {
                    pick -= slopes[k];
                    k += 1;
                }
                if slopes[k] == 0.0 {
                    k = (0..8)
                        .max_by(|&a, &b| slopes[a].total_cmp(&slopes[b]))
                        .unwrap_or(0);
                }
                x += NB[k].0;
                y += NB[k].1;
                run += dist[k];
            }
            let i = idx(x, y);
            z[i] += share;
            count[i] += 1;
            size_sum[i] += size;
        }
    }
    let mk = |f: &(dyn Fn(usize) -> f32 + Sync)| -> JdResult<Heightfield> {
        let mut o = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
        o.fill_with(|x, y| f(y as usize * w as usize + x as usize));
        Ok(o)
    };
    Ok(DebrisResult {
        out: mk(&|i| z[i])?,
        debris: mk(&|i| 1.0 - (-(count[i] as f32) / n as f32).exp())?,
        size: mk(&|i| {
            if count[i] > 0 {
                size_sum[i] / count[i] as f32
            } else {
                0.0
            }
        })?,
        blocks: sources.len() * n,
        moved_m3: moved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20 m cliff (east third high) above a 5° apron sloping east→west, 2 m cells.
    fn cliff() -> Heightfield {
        let mut h = Heightfield::new(96, 32, 128, 0).expect("hf");
        h.fill_with(|x, _| {
            let apron = (x as f32 * 2.0) * 5f32.to_radians().tan();
            if x >= 60 { 20.0 + apron } else { apron }
        });
        h
    }

    #[test]
    fn cliffs_shed_blocks_that_rest_below_with_mass_conserved_and_boulders_running_furthest() {
        let h = cliff();
        let p = DebrisParams {
            blocks_per_cell: 16,
            sorting: 1.0,
            ..DebrisParams::default()
        };
        let r = rockfall(&h, [2.0, 2.0], None, &p, None).expect("rockfall");
        assert!(r.blocks > 0 && r.moved_m3 > 0.0);
        let (before, after): (f64, f64) = (0..32u32)
            .flat_map(|y| (0..96u32).map(move |x| (x, y)))
            .fold((0.0, 0.0), |(a, b), (x, y)| {
                (a + h.get(x, y) as f64, b + r.out.get(x, y) as f64)
            });
        assert!(
            ((after - before) * 4.0).abs() < 1e-2 * r.moved_m3,
            "mass conserved: {before} → {after}"
        );
        // Blocks rest on the apron below the cliff, none on the plateau.
        let on = |x0: u32, x1: u32| {
            (x0..x1)
                .flat_map(|x| (0..32).map(move |y| (x, y)))
                .filter(|&(x, y)| r.debris.get(x, y) > 0.0)
                .count()
        };
        assert!(on(0, 59) > 20, "talus below the cliff");
        assert_eq!(on(61, 96), 0, "nothing comes to rest on the plateau");
        // Fall sorting: the mean size grows away from the cliff foot.
        let mean = |x0: u32, x1: u32| {
            let v: Vec<f32> = (x0..x1)
                .flat_map(|x| (0..32).map(move |y| (x, y)))
                .map(|(x, y)| r.size.get(x, y))
                .filter(|s| *s > 0.0)
                .collect();
            v.iter().sum::<f32>() / v.len().max(1) as f32
        };
        let near = mean(52, 59);
        let far_x = (0..52)
            .find(|&x| (0..32).any(|y| r.debris.get(x, y) > 0.0))
            .expect("some run-out");
        assert!(
            mean(far_x, far_x + 6) > near,
            "boulders run out past the scree: far {} near {near}",
            mean(far_x, far_x + 6)
        );
        // Deterministic per seed; a zero source mask releases nothing; bad params refused.
        let again = rockfall(&h, [2.0, 2.0], None, &p, None).expect("again");
        assert_eq!(again.out.to_vec(), r.out.to_vec());
        let mut none = Heightfield::new(96, 32, 128, 0).expect("m");
        none.fill_with(|_, _| 0.0);
        assert_eq!(
            rockfall(&h, [2.0, 2.0], Some(&none), &p, None)
                .expect("masked")
                .blocks,
            0
        );
        assert!(
            rockfall(
                &h,
                [2.0, 2.0],
                None,
                &DebrisParams {
                    repose_deg: 90.0,
                    ..p.clone()
                },
                None
            )
            .is_err()
        );
        assert!(
            rockfall(
                &h,
                [2.0, 2.0],
                None,
                &DebrisParams {
                    size_min_m: 4.0,
                    ..p
                },
                None
            )
            .is_err()
        );
    }
}
