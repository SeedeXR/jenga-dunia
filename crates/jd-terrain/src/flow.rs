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
