//! Hydrology-first terrain (Génevaux et al. 2013 in spirit, `docs/research/algorithms.md`):
//! the drainage network comes first and the heights are built along it, so every cell drains to
//! a river mouth by construction. A random spanning tree grown from the mouths (seeded costs)
//! is the network; Strahler orders come from a post-order pass; the height of a cell is its
//! parent's plus the slope of its stream order times the step, headwaters steep and big rivers
//! flat.
//!
//! River primitives (Génevaux's second half, `valley_relief_m` > 0): every river carries a
//! valley cross-section — the terrain climbs `relief × (d / valley_width_m)^shape` above the
//! tree height with d the distance along the drainage to the river the cell feeds, reaching the
//! full relief at the width and staying there beyond it (shape 1 = a V, 2 = a U with a wide
//! flat floor, 0.5 = a canyon notch, steep at the river and flattening to the rim). The tree
//! is the plateau, the valleys are incised into it along the rivers; d only grows upstream, so
//! every cell still sits above its parent and the field drains by construction.
//!
//! ponytail: a spanning tree over smooth lattice-noise costs; one slope law per order; mouths
//! along one edge or the whole border; one profile for all orders (no profile per Rosgen
//! class, no sum over overlapping primitives).

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
    /// Valley floor: within this distance (metres along the stream) of a river the slope eases to a
    /// floodplain (15 % of the headwater law), so valleys get a floor instead of a V (0 = off).
    /// With `valley_relief_m` it is also the half-width of the valley cross-section.
    #[serde(default = "d_valley_width")]
    pub valley_width_m: f32,
    /// Valley cross-section as a river primitive (0 = off): the valley side climbs this many
    /// metres above the river over `valley_width_m` (measured along the drainage), and the
    /// ground beyond the width sits that much above the tree height — the tree becomes the
    /// plateau the valleys are cut into.
    #[serde(default)]
    pub valley_relief_m: f32,
    /// Shape of that cross-section, the exponent of (distance / width): 1 a V, 2 a U with a
    /// wide flat floor (default), 0.5 a canyon notch — steep at the river, flattening to the
    /// rim (> 0).
    #[serde(default = "d_valley_shape")]
    pub valley_shape: f32,
}
fn d_valley_shape() -> f32 {
    2.0
}
fn d_valley_width() -> f32 {
    600.0
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
            valley_width_m: d_valley_width(),
            valley_relief_m: 0.0,
            valley_shape: d_valley_shape(),
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
    if !(p.valley_relief_m >= 0.0 && p.valley_relief_m.is_finite()) {
        return Err(JdError::invalid(
            "valley_relief_m",
            "must be finite and ≥ 0",
        ));
    }
    if p.valley_relief_m > 0.0 {
        if !(p.valley_shape > 0.0 && p.valley_shape.is_finite()) {
            return Err(JdError::invalid("valley_shape", "must be positive"));
        }
        if p.valley_width_m <= 0.0 {
            return Err(JdError::invalid(
                "valley_width_m",
                "must be > 0 with valley_relief_m",
            ));
        }
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
    if !(p.valley_width_m >= 0.0 && p.valley_width_m.is_finite()) {
        return Err(JdError::invalid("valley_width_m", "must be finite and ≥ 0"));
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
    // Heights, parents before children: a cell sits above its parent by the slope of its order,
    // eased towards a floodplain within `valley_width_m` (along the stream) of a river cell —
    // `to_river` only grows upstream, so every cell still sits above its parent.
    let mut hgt = vec![p.base_m; n];
    let mut to_river = vec![0.0f32; n];
    // With primitives on, a mouth that is not a river in its own right (order below
    // `river_order`) is no valley bottom: it starts at the full width so its subtree sits on the
    // plateau. (Relief off keeps the floodplain easing exactly as it was.)
    if p.valley_relief_m > 0.0 {
        for i in 0..n {
            if parent[i] == NONE && order[i] < p.river_order {
                to_river[i] = p.valley_width_m;
            }
        }
    }
    for &i in by_dist.iter().rev() {
        let i = i as usize;
        let pi = parent[i];
        if pi == NONE {
            continue;
        }
        let pi = pi as usize;
        let step = dist[i] - dist[pi];
        to_river[i] = if order[i] >= p.river_order {
            0.0
        } else {
            to_river[pi] + step
        };
        let floor = if p.valley_width_m > 0.0 {
            let t = (to_river[i] / p.valley_width_m).clamp(0.0, 1.0);
            0.15 + 0.85 * t * t * (3.0 - 2.0 * t)
        } else {
            1.0
        };
        let slope = p.slope_m_per_km / 1000.0 * p.order_falloff.powi(order[i] as i32 - 1) * floor;
        hgt[i] = hgt[pi] + step * slope;
    }
    let max_order = order.iter().copied().max().unwrap_or(1).max(1) as f32;
    // River primitives: the ground climbs the cross-section profile above the tree height with
    // the drainage distance to the river it feeds (`to_river`), to the full relief at the width
    // and beyond. `to_river` only grows upstream, so the lift does too and every cell still
    // sits above its parent; added on output so the tree heights stay the parents' reference.
    let lift = |i: usize| {
        if p.valley_relief_m > 0.0 {
            p.valley_relief_m
                * (to_river[i] / p.valley_width_m)
                    .min(1.0)
                    .powf(p.valley_shape)
        } else {
            0.0
        }
    };
    let mut out = Heightfield::new(width, height, tile_size, halo)?;
    out.fill_with(|x, y| {
        let i = (y as usize) * w + x as usize;
        hgt[i] + lift(i)
    });
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
    fn river_primitives_give_valleys_a_cross_section_and_leave_the_ridges_to_the_tree() {
        // 128² at 20 m, rivers of order ≥ 6; a 300 m wide, 80 m deep U valley around every river.
        let base = NetworkParams {
            seed: 5,
            river_order: 6,
            valley_width_m: 300.0,
            ..NetworkParams::default()
        };
        let (tree, rivers, _) =
            hydrology_first(128, 128, 128, 0, [20.0, 20.0], &base).expect("tree");
        let with = NetworkParams {
            valley_relief_m: 80.0,
            valley_shape: 2.0,
            ..base.clone()
        };
        let (out, rivers2, _) =
            hydrology_first(128, 128, 128, 0, [20.0, 20.0], &with).expect("primitives");
        assert_eq!(
            rivers.to_vec(),
            rivers2.to_vec(),
            "the network itself is unchanged"
        );
        // Straight-line distance to the nearest river cell (a lower bound on the drainage
        // distance the profile uses).
        let river_cells: Vec<(f32, f32)> = (0..128u32)
            .flat_map(|y| (0..128u32).map(move |x| (x, y)))
            .filter(|&(x, y)| rivers.get(x, y) > 0.0)
            .map(|(x, y)| (x as f32 * 20.0, y as f32 * 20.0))
            .collect();
        assert!(river_cells.len() > 20, "{}", river_cells.len());
        let dist = |x: u32, y: u32| {
            river_cells
                .iter()
                .map(|(rx, ry)| (x as f32 * 20.0 - rx).hypot(y as f32 * 20.0 - ry))
                .fold(f32::INFINITY, f32::min)
        };
        // River cells keep their height; the first cell up the drainage sits barely above its
        // river while the valley side climbs to the full relief; beyond the width (straight-line,
        // a lower bound on the drainage distance) the plateau sits exactly that much above the
        // tree — mouths that are not rivers included, since they start at the width.
        let easing = 0.85 * base.slope_m_per_km / 1000.0 * base.valley_width_m + 1e-3;
        let (mut beside, mut outside, mut same, mut max_lift) = (Vec::new(), 0, 0, 0.0f32);
        for y in 0..128 {
            for x in 0..128 {
                let dd = dist(x, y);
                let lift = out.get(x, y) - tree.get(x, y);
                if dd == 0.0 {
                    assert!(lift.abs() < 1e-3, "river cell moved by {lift}");
                    same += 1;
                } else if dd > 300.0 * 1.5 {
                    // (to the floodplain easing the relief-0 baseline still applies below a
                    // non-river mouth and the primitives switch off: up to 85 % of the headwater
                    // slope over the width)
                    assert!(
                        (lift - 80.0).abs() < easing,
                        "beyond the width the plateau sits the full relief above the tree: {lift}"
                    );
                    outside += 1;
                } else if lift > 0.0 {
                    beside.push(lift);
                }
                max_lift = max_lift.max(lift);
            }
        }
        assert!(
            same > 20 && outside > 20 && beside.len() > 20,
            "{same} {outside} {}",
            beside.len()
        );
        // The first cell up the drainage from a river is one step in: U gives 80 × (20/300)²
        // ≈ 0.4 m there, so the smallest positive lift is under 2 m whatever the seed.
        let smallest = beside.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            smallest < 2.0,
            "beside the river the floor is flat: {smallest}"
        );
        assert!(
            (max_lift - 80.0).abs() < easing,
            "the valley side climbs the full relief: {max_lift}"
        );
        // Still drains by construction: priority-flood filling finds nothing.
        let (_, lakes) = crate::fill_depressions(&out, 0.0).expect("fill");
        let worst = lakes.to_vec().iter().copied().fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "no pits with primitives: max fill {worst} m");
        let bad = |q: NetworkParams| hydrology_first(128, 128, 128, 0, [20.0, 20.0], &q).is_err();
        assert!(bad(NetworkParams {
            valley_shape: 0.0,
            ..with.clone()
        }));
        assert!(bad(NetworkParams {
            valley_width_m: 0.0,
            ..with
        }));
    }

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
        // Valley floors: easing the slope near rivers lowers the land next to them; the drainage
        // property survives (the fill still finds nothing).
        let (v, _, _) = hydrology_first(
            96,
            64,
            64,
            4,
            [50.0, 50.0],
            &NetworkParams {
                valley_width_m: 1500.0,
                ..p.clone()
            },
        )
        .expect("valley");
        let (sharp, _, _) = hydrology_first(
            96,
            64,
            64,
            4,
            [50.0, 50.0],
            &NetworkParams {
                valley_width_m: 0.0,
                ..p.clone()
            },
        )
        .expect("v");
        let mean = |f: &Heightfield| f.to_vec().iter().sum::<f32>() / (96.0 * 64.0);
        assert!(
            mean(&v) < 0.8 * mean(&sharp),
            "floodplains lower the land: {} vs {}",
            mean(&v),
            mean(&sharp)
        );
        let (vf, _) = crate::fill_depressions(&v, 0.0).expect("fill v");
        assert!(
            v.to_vec()
                .iter()
                .zip(&vf.to_vec())
                .all(|(a, b)| (a - b).abs() < 1e-4),
            "valley floors still drain"
        );
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
