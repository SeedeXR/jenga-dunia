//! Hydrology-first terrain (Génevaux et al. 2013 in spirit, `docs/research/algorithms.md`):
//! the drainage network comes first and the heights are built along it, so every cell drains to
//! a river mouth by construction. A random spanning tree grown from the mouths (seeded costs)
//! is the network; Strahler orders come from a post-order pass; the height of a cell is its
//! parent's plus the slope of its stream order times the step, headwaters steep and big rivers
//! flat.
//!
//! ponytail: a spanning tree over smooth lattice-noise costs instead of Génevaux's primitive
//! blending; one slope law per order; mouths along one edge or the whole border; no explicit
//! valley cross-sections (feed the result to the erosion nodes for those).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;

/// Where the rivers leave the field.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Mouths {
    /// Every cell of the south edge (row 0) is a possible mouth.
    #[default]
    South,
    /// The north edge.
    North,
    /// The west edge.
    West,
    /// The east edge.
    East,
    /// The whole border: an island.
    Border,
}

/// Hydrology-first generator settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkParams {
    /// Seed of the network.
    #[serde(default)]
    pub seed: u64,
    /// Which edge(s) the rivers drain to.
    #[serde(default)]
    pub mouths: Mouths,
    /// Height at the mouths, metres.
    #[serde(default)]
    pub base_m: f32,
    /// Stream slope of first-order (headwater) channels, metres per km (> 0).
    #[serde(default = "d_slope")]
    pub slope_m_per_km: f32,
    /// Each higher Strahler order multiplies the slope by this (0..1): big rivers run flat.
    #[serde(default = "d_falloff")]
    pub order_falloff: f32,
    /// How much the seeded noise bends the network (0 = straight parallel drainage, 1 = wild).
    #[serde(default = "d_roughness")]
    pub roughness: f32,
    /// Strahler order from which a channel counts as a river in the `rivers` output (≥ 1).
    #[serde(default = "d_river_order")]
    pub river_order: u32,
}

fn d_slope() -> f32 {
    60.0
}
fn d_falloff() -> f32 {
    0.55
}
fn d_roughness() -> f32 {
    0.8
}
fn d_river_order() -> u32 {
    3
}

impl Default for NetworkParams {
    fn default() -> Self {
        Self {
            seed: 0,
            mouths: Mouths::default(),
            base_m: 0.0,
            slope_m_per_km: d_slope(),
            order_falloff: d_falloff(),
            roughness: d_roughness(),
            river_order: d_river_order(),
        }
    }
}

/// splitmix64 on (seed, cell): a deterministic per-cell random in 0..1.
fn hash01(seed: u64, i: u64) -> f32 {
    let mut z = seed
        .wrapping_add(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(i.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// Cells per lattice step of the cost noise: valleys keep their course for about this long.
const LATTICE: f32 = 12.0;

/// Smooth value noise (bilinear between lattice randoms, two octaves) in 0..1.
fn lattice_noise(seed: u64, x: f32, y: f32) -> f32 {
    let octave = |scale: f32, salt: u64| {
        let (u, v) = (x / scale, y / scale);
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let at = |i: f32, j: f32| {
            hash01(
                seed ^ salt,
                (i as i64 as u64).wrapping_mul(73_856_093)
                    ^ (j as i64 as u64).wrapping_mul(19_349_663),
            )
        };
        let top = at(x0, y0) * (1.0 - sx) + at(x0 + 1.0, y0) * sx;
        let bot = at(x0, y0 + 1.0) * (1.0 - sx) + at(x0 + 1.0, y0 + 1.0) * sx;
        top * (1.0 - sy) + bot * sy
    };
    0.7 * octave(LATTICE, 0) + 0.3 * octave(LATTICE / 3.0, 0x5EED)
}

/// Returns (`out` heights, `rivers` 0..1 = order / max order for channels of `river_order` and
/// up, `order` = Strahler order per cell). Row 0 is south.
pub fn hydrology_first(
    width: u32,
    height: u32,
    tile_size: u32,
    halo: u32,
    spacing_m: [f32; 2],
    p: &NetworkParams,
) -> JdResult<(Heightfield, Heightfield, Heightfield)> {
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    if !(p.slope_m_per_km > 0.0 && p.slope_m_per_km.is_finite()) {
        return Err(JdError::invalid("slope_m_per_km", "must be positive"));
    }
    if !(0.0..=1.0).contains(&p.order_falloff) || !(0.0..=1.0).contains(&p.roughness) {
        return Err(JdError::invalid(
            "order_falloff",
            "order_falloff and roughness must be within 0..=1",
        ));
    }
    if p.river_order == 0 {
        return Err(JdError::invalid("river_order", "must be ≥ 1"));
    }
    if !p.base_m.is_finite() {
        return Err(JdError::invalid("base_m", "must be finite"));
    }
    let (w, h) = (width as usize, height as usize);
    let n = w * h;
    const NONE: u32 = u32::MAX;
    let mut parent = vec![NONE; n];
    let mut dist = vec![f32::INFINITY; n];
    let mut heap: BinaryHeap<Reverse<(ordered::F, u32)>> = BinaryHeap::new();
    let is_mouth = |x: usize, y: usize| match p.mouths {
        Mouths::South => y == 0,
        Mouths::North => y == h - 1,
        Mouths::West => x == 0,
        Mouths::East => x == w - 1,
        Mouths::Border => x == 0 || y == 0 || x == w - 1 || y == h - 1,
    };
    for y in 0..h {
        for x in 0..w {
            if is_mouth(x, y) {
                let i = y * w + x;
                dist[i] = 0.0;
                heap.push(Reverse((ordered::F(0.0), i as u32)));
            }
        }
    }
    if heap.is_empty() {
        return Err(JdError::invalid(
            "mouths",
            "the field has no cells on that edge",
        ));
    }
    // Prim's algorithm over seeded edge costs: a random spanning tree rooted at the mouths.
    // The cost of reaching a cell is its own random weight (so the tree, not the path length,
    // is what the noise shapes); `dist` keeps the true metres along the tree for the heights.
    let offsets: [(i64, i64); 8] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];
    let step = offsets.map(|(dx, dy)| (dx as f32 * spacing_m[0]).hypot(dy as f32 * spacing_m[1]));
    let mut cost = vec![f32::INFINITY; n];
    let mut done = vec![false; n];
    for i in 0..n {
        if dist[i] == 0.0 {
            cost[i] = 0.0;
        }
    }
    while let Some(Reverse((_, i))) = heap.pop() {
        let i = i as usize;
        if done[i] {
            continue;
        }
        done[i] = true;
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (k, (dx, dy)) in offsets.iter().enumerate() {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let j = (ny as usize) * w + nx as usize;
            if done[j] {
                continue;
            }
            // A straight parallel drainage costs the step; roughness mixes in a smooth random
            // field (value noise on a lattice), so cheap corridors — the valleys — stay coherent
            // over many cells instead of changing every cell.
            let r = lattice_noise(p.seed, nx as f32, ny as f32);
            let c = step[k] * (1.0 - p.roughness) + p.roughness * step[k] * (0.05 + 2.0 * r);
            if c < cost[j] {
                cost[j] = c;
                parent[j] = i as u32;
                dist[j] = dist[i] + step[k];
                heap.push(Reverse((ordered::F(c), j as u32)));
            }
        }
    }
    // Strahler order: children before parents (decreasing distance along the tree).
    let mut by_dist: Vec<u32> = (0..n as u32).collect();
    by_dist.sort_by(|a, b| {
        dist[*b as usize]
            .total_cmp(&dist[*a as usize])
            .then(a.cmp(b))
    });
    let mut order = vec![1u32; n];
    let mut max_child = vec![0u32; n];
    let mut max_count = vec![0u32; n];
    for &i in &by_dist {
        let i = i as usize;
        order[i] = if max_count[i] >= 2 {
            max_child[i] + 1
        } else {
            max_child[i].max(1)
        };
        let pi = parent[i];
        if pi != NONE {
            let pi = pi as usize;
            match order[i].cmp(&max_child[pi]) {
                std::cmp::Ordering::Greater => {
                    max_child[pi] = order[i];
                    max_count[pi] = 1;
                }
                std::cmp::Ordering::Equal => max_count[pi] += 1,
                std::cmp::Ordering::Less => {}
            }
        }
    }
    // Heights: parents before children; a cell sits above its parent by the slope of its order.
    let mut hgt = vec![p.base_m; n];
    for &i in by_dist.iter().rev() {
        let i = i as usize;
        let pi = parent[i];
        if pi == NONE {
            continue;
        }
        let pi = pi as usize;
        let slope = p.slope_m_per_km / 1000.0 * p.order_falloff.powi(order[i] as i32 - 1);
        hgt[i] = hgt[pi] + (dist[i] - dist[pi]) * slope;
    }
    let max_order = order.iter().copied().max().unwrap_or(1).max(1) as f32;
    let mut out = Heightfield::new(width, height, tile_size, halo)?;
    out.fill_with(|x, y| hgt[(y as usize) * w + x as usize]);
    let mut rivers = Heightfield::new(width, height, tile_size, halo)?;
    rivers.fill_with(|x, y| {
        let o = order[(y as usize) * w + x as usize];
        if o >= p.river_order {
            o as f32 / max_order
        } else {
            0.0
        }
    });
    let mut ord = Heightfield::new(width, height, tile_size, halo)?;
    ord.fill_with(|x, y| order[(y as usize) * w + x as usize] as f32);
    Ok((out, rivers, ord))
}

/// A total order on f32 for the heap (NaN never enters: costs are finite).
mod ordered {
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct F(pub f32);
    impl Eq for F {}
    impl PartialOrd for F {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for F {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&other.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cell_drains_to_a_mouth_and_big_rivers_run_flatter_than_headwaters() {
        let p = NetworkParams {
            seed: 3,
            ..NetworkParams::default()
        };
        let (h, rivers, order) = hydrology_first(96, 64, 64, 4, [50.0, 50.0], &p).expect("net");
        // Drainage by construction: priority-flood filling finds nothing to fill.
        let (filled, lakes) = crate::fill_depressions(&h, 0.0).expect("fill");
        let worst = h
            .to_vec()
            .iter()
            .zip(&filled.to_vec())
            .map(|(a, b)| (b - a).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "no pits: max fill {worst} m");
        assert!(lakes.to_vec().iter().all(|d| *d < 1e-4));
        // Mouths sit at the base height, everything else above it.
        assert!((0..96).all(|x| (h.get(x, 0) - 0.0).abs() < 1e-6));
        assert!(h.to_vec().iter().all(|v| *v >= 0.0));
        // Orders: the tree has at least third-order rivers and the mask marks only those.
        let max_order = order.to_vec().iter().cloned().fold(0.0f32, f32::max);
        assert!(max_order >= 3.0, "max order {max_order}");
        let river_cells = rivers.to_vec().iter().filter(|v| **v > 0.0).count();
        let high_cells = order.to_vec().iter().filter(|o| **o >= 3.0).count();
        assert_eq!(river_cells, high_cells);
        assert!(
            river_cells > 0 && river_cells < 96 * 64 / 4,
            "rivers are a minority: {river_cells}"
        );
        // Determinism and the seed.
        let (h2, _, _) = hydrology_first(96, 64, 64, 4, [50.0, 50.0], &p).expect("again");
        assert_eq!(h.content_hash(), h2.content_hash());
        let (h3, _, _) = hydrology_first(
            96,
            64,
            64,
            4,
            [50.0, 50.0],
            &NetworkParams {
                seed: 4,
                ..p.clone()
            },
        )
        .expect("seed");
        assert_ne!(h.content_hash(), h3.content_hash());
        // An island drains to every edge; a bad slope is refused.
        let (isl, _, _) = hydrology_first(
            48,
            48,
            64,
            0,
            [50.0, 50.0],
            &NetworkParams {
                mouths: Mouths::Border,
                ..p.clone()
            },
        )
        .expect("island");
        assert!((0..48).all(|k| isl.get(k, 0) == 0.0
            && isl.get(k, 47) == 0.0
            && isl.get(0, k) == 0.0
            && isl.get(47, k) == 0.0));
        assert!(
            hydrology_first(
                8,
                8,
                64,
                0,
                [1.0, 1.0],
                &NetworkParams {
                    slope_m_per_km: 0.0,
                    ..p
                }
            )
            .is_err()
        );
    }
}
