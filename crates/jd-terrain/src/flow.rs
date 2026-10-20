//! Flow routing (`docs/research/algorithms.md` §6): D8 receivers and flow accumulation by
//! descending-height order. Deterministic (ties broken by cell index), O(N log N) for the sort.
//!
//! ponytail: D8 only (45° quantisation shows parallel-line artefacts on planar slopes); D∞ /
//! MFD and priority-flood depression filling follow with the hydrology nodes.

use crate::Heightfield;
use jd_types::{JdError, JdResult};
use rayon::prelude::*;

const NEIGHBOURS: [(i64, i64); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// D8 receiver of every cell (`None` for pits and cells with no lower neighbour), using the
/// steepest descent with anisotropic spacing.
pub fn d8_receivers(h: &[f32], w: usize, hh: usize, spacing_m: [f32; 2]) -> Vec<Option<u32>> {
    let dist =
        NEIGHBOURS.map(|(dx, dy)| (dx as f32 * spacing_m[0]).hypot(dy as f32 * spacing_m[1]));
    let mut out = vec![None; w * hh];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, r) in row.iter_mut().enumerate() {
            let hc = h[y * w + x];
            let mut best: Option<(f32, u32)> = None;
            for (k, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= hh as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                let slope = (hc - h[j]) / dist[k];
                if slope > 0.0 && best.is_none_or(|(s, _)| slope > s) {
                    best = Some((slope, j as u32));
                }
            }
            *r = best.map(|(_, j)| j);
        }
    });
    out
}

/// Flow accumulation: every cell contributes one unit of rain-weighted area and passes its
/// total to its D8 receiver, visiting cells from highest to lowest. Pits keep what they collect.
/// With `log_scale` the output is `ln(1 + acc) / ln(1 + N)` in 0..1, otherwise the raw count.
pub fn flow_accumulation(
    h: &Heightfield,
    spacing_m: [f32; 2],
    log_scale: bool,
) -> JdResult<Heightfield> {
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let (w, hh) = (h.width() as usize, h.height() as usize);
    let v = h.to_vec();
    if v.iter().any(|x| !x.is_finite()) {
        return Err(JdError::invalid("in", "heightfield has non-finite samples"));
    }
    let recv = d8_receivers(&v, w, hh, spacing_m);
    let mut order: Vec<u32> = (0..(w * hh) as u32).collect();
    // Descending height, ties by index: identical output regardless of thread count.
    order.par_sort_unstable_by(|a, b| v[*b as usize].total_cmp(&v[*a as usize]).then(a.cmp(b)));
    let mut acc = vec![1.0f32; w * hh];
    for &i in &order {
        if let Some(j) = recv[i as usize] {
            acc[j as usize] += acc[i as usize];
        }
    }
    let n = (w * hh) as f32;
    let mut out = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    out.fill_with(|x, y| {
        let a = acc[y as usize * w + x as usize];
        if log_scale {
            (1.0 + a).ln() / (1.0 + n).ln()
        } else {
            a
        }
    });
    Ok(out)
}

/// Priority-Flood depression filling (Barnes, Lehman & Mulla 2014): flood inward from the
/// border with a min-heap; every cell is raised to at least the spill level of its basin plus
/// `epsilon_m` per step so filled flats still drain. Returns the filled field and the lake depth
/// (`filled − original`, ≥ 0). O(N log N), deterministic (ties broken by cell index).
///
/// ponytail: serial whole-field pass; the tiled parallel variant (Barnes 2016) comes with
/// tile-level evaluation.
pub fn fill_depressions(h: &Heightfield, epsilon_m: f32) -> JdResult<(Heightfield, Heightfield)> {
    if !(epsilon_m >= 0.0 && epsilon_m.is_finite()) {
        return Err(JdError::invalid("epsilon_m", "must be finite and ≥ 0"));
    }
    let (w, hh) = (h.width() as usize, h.height() as usize);
    let orig = h.to_vec();
    if orig.iter().any(|v| !v.is_finite()) {
        return Err(JdError::invalid("in", "heightfield has non-finite samples"));
    }
    use std::cmp::Ordering as CmpOrd;
    use std::collections::BinaryHeap;
    #[derive(PartialEq)]
    struct Item(f32, u32); // (height, index) — min-heap via reversed ordering
    impl Eq for Item {}
    impl PartialOrd for Item {
        fn partial_cmp(&self, o: &Self) -> Option<CmpOrd> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Item {
        fn cmp(&self, o: &Self) -> CmpOrd {
            o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
        }
    }
    let mut filled = orig.clone();
    let mut seen = vec![false; w * hh];
    let mut heap = BinaryHeap::new();
    for y in 0..hh {
        for x in 0..w {
            if x == 0 || y == 0 || x == w - 1 || y == hh - 1 {
                let i = y * w + x;
                seen[i] = true;
                heap.push(Item(orig[i], i as u32));
            }
        }
    }
    while let Some(Item(level, i)) = heap.pop() {
        let (x, y) = ((i as usize % w) as i64, (i as usize / w) as i64);
        for (dx, dy) in NEIGHBOURS {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= hh as i64 {
                continue;
            }
            let j = ny as usize * w + nx as usize;
            if seen[j] {
                continue;
            }
            seen[j] = true;
            // A neighbour below the current spill level is raised just above it.
            if filled[j] < level + epsilon_m {
                filled[j] = level + epsilon_m;
            }
            heap.push(Item(filled[j], j as u32));
        }
    }
    let mut out = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    out.fill_with(|x, y| filled[y as usize * w + x as usize]);
    let mut lakes = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    lakes.fill_with(|x, y| {
        let i = y as usize * w + x as usize;
        (filled[i] - orig[i]).max(0.0)
    });
    Ok((out, lakes))
}

/// River carving from flow accumulation: fill depressions, route D8 flow, then incise where the
/// log-scaled accumulation exceeds `threshold` (0..1), down to `depth_m` at the largest rivers
/// with a smooth ramp. Returns the carved terrain and the 0..1 river mask.
///
/// ponytail: incision depth follows accumulation only; width/meanders/banks and hydrology-first
/// generation (Génevaux 2013) are later nodes.
pub fn carve_rivers(
    h: &Heightfield,
    spacing_m: [f32; 2],
    threshold: f32,
    depth_m: f32,
) -> JdResult<(Heightfield, Heightfield)> {
    if !(0.0..1.0).contains(&threshold) {
        return Err(JdError::invalid("threshold", "must be within 0..1"));
    }
    if !(depth_m >= 0.0 && depth_m.is_finite()) {
        return Err(JdError::invalid("depth_m", "must be finite and ≥ 0"));
    }
    let (filled, _) = fill_depressions(h, 0.001)?;
    let acc = flow_accumulation(&filled, spacing_m, true)?;
    let mut mask = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    mask.fill_with(|x, y| {
        let t = ((acc.get(x, y) - threshold) / (1.0 - threshold)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    });
    let mut out = h.clone();
    out.fill_with(|x, y| h.get(x, y) - depth_m * mask.get(x, y));
    Ok((out, mask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_drains_to_its_low_edge_and_conserves_area() {
        let mut h = Heightfield::new(32, 8, 32, 0).expect("hf");
        h.fill_with(|x, _| x as f32); // falls towards x = 0
        let acc = flow_accumulation(&h, [1.0, 1.0], false).expect("flow");
        // Every row drains west: column x collects itself plus everything east of it.
        for x in 0..32u32 {
            assert_eq!(acc.get(x, 3), (32 - x) as f32, "x = {x}");
        }
        // Outlets (cells without a receiver) hold exactly the total area.
        let v = h.to_vec();
        let recv = d8_receivers(&v, 32, 8, [1.0, 1.0]);
        let total: f32 = (0..32 * 8)
            .filter(|&i| recv[i].is_none())
            .map(|i| acc.get((i % 32) as u32, (i / 32) as u32))
            .sum();
        assert_eq!(total, 256.0);
        // Log scale lands in 0..1 and keeps the ordering.
        let m = flow_accumulation(&h, [1.0, 1.0], true).expect("flow");
        assert!(m.stats().min > 0.0 && m.stats().max <= 1.0 + 1e-6);
        assert!(m.get(0, 3) > m.get(31, 3));
    }

    #[test]
    fn priority_flood_fills_pits_to_their_spill_level_and_keeps_drainage() {
        // A bowl: rim at 100 m, a 3×3 pit at 20 m in the middle, and a 60 m channel cut from the
        // north border down to the pit — the lowest spill is through the channel.
        let mut h = Heightfield::new(16, 16, 16, 0).expect("hf");
        h.fill_with(|x, y| {
            if (6..9).contains(&x) && (6..9).contains(&y) {
                20.0
            } else if x == 7 && y <= 5 {
                60.0
            } else {
                100.0
            }
        });
        let (filled, lakes) = fill_depressions(&h, 0.0).expect("fill");
        // Edge cells never change; the pit rises to the lowest spill (the notch at 60 m).
        assert_eq!(filled.get(0, 0), 100.0);
        assert_eq!(filled.get(7, 0), 60.0);
        for (x, y) in [(6u32, 6u32), (7, 7), (8, 8)] {
            assert!(
                (filled.get(x, y) - 60.0).abs() < 1e-4,
                "pit → spill level: {}",
                filled.get(x, y)
            );
            assert!((lakes.get(x, y) - 40.0).abs() < 1e-4);
        }
        assert_eq!(lakes.get(1, 1), 0.0);
        // Nothing ever gets lower; with an epsilon every interior cell has a lower neighbour.
        let (eps, _) = fill_depressions(&h, 0.01).expect("fill");
        let v = eps.to_vec();
        let o = h.to_vec();
        assert!(v.iter().zip(&o).all(|(a, b)| a >= b));
        let recv = d8_receivers(&v, 16, 16, [1.0, 1.0]);
        for y in 1..15usize {
            for x in 1..15usize {
                assert!(
                    recv[y * 16 + x].is_some(),
                    "({x},{y}) must drain after filling"
                );
            }
        }
        // Deterministic and validating.
        assert_eq!(
            fill_depressions(&h, 0.01).expect("again").0.content_hash(),
            eps.content_hash()
        );
        assert!(fill_depressions(&h, -1.0).is_err());
    }

    #[test]
    fn rivers_carve_where_flow_concentrates() {
        // A V-shaped valley: water from both slopes gathers along x = 16 and flows north (y → 0).
        let mut h = Heightfield::new(33, 24, 32, 4).expect("hf");
        h.fill_with(|x, y| (x as f32 - 16.0).abs() * 4.0 + y as f32 * 0.5);
        let (carved, mask) = carve_rivers(&h, [1.0, 1.0], 0.6, 10.0).expect("carve");
        assert!(
            mask.get(16, 2) > 0.5,
            "valley floor near the outlet is river: {}",
            mask.get(16, 2)
        );
        assert_eq!(mask.get(2, 20), 0.0, "ridge top is dry");
        assert!(
            h.get(16, 2) - carved.get(16, 2) > 5.0,
            "incised: {} → {}",
            h.get(16, 2),
            carved.get(16, 2)
        );
        assert_eq!(carved.get(2, 20), h.get(2, 20));
        assert_eq!(
            carve_rivers(&h, [1.0, 1.0], 0.6, 10.0)
                .expect("again")
                .0
                .content_hash(),
            carved.content_hash()
        );
        assert!(carve_rivers(&h, [1.0, 1.0], 1.0, 10.0).is_err());
    }

    #[test]
    fn deterministic_and_spacing_aware() {
        let mut h = Heightfield::new(40, 30, 32, 4).expect("hf");
        h.fill_with(|x, y| {
            ((x as f32 * 0.3).sin() * 20.0 + (y as f32 * 0.2).cos() * 15.0) - y as f32 * 0.5
        });
        let a = flow_accumulation(&h, [2.0, 2.0], true).expect("flow");
        let b = flow_accumulation(&h, [2.0, 2.0], true).expect("flow");
        assert_eq!(a.content_hash(), b.content_hash());
        assert_eq!(a.stats().non_finite, 0);
        // A steeper y-spacing changes which neighbour is steepest, so the routing differs.
        let c = flow_accumulation(&h, [2.0, 0.5], true).expect("flow");
        assert_ne!(a.content_hash(), c.content_hash());
        assert!(flow_accumulation(&h, [0.0, 1.0], true).is_err());
    }
}
