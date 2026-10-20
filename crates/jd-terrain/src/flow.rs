//! Flow routing (`docs/research/algorithms.md` §6): D8 and D∞ (Tarboton 1997) receivers,
//! flow accumulation by descending-height order, priority-flood filling, rivers and wetness.
//! Deterministic (ties broken by cell index), O(N log N) for the sort.
//!
//! ponytail: MFD exponent fixed at 1.1 (Freeman); make it a parameter when someone needs Quinn-style tuning.

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

/// Flow routing method.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FlowMethod {
    /// Single steepest neighbour (O'Callaghan & Mark 1984): fast, 45° quantised.
    #[default]
    D8,
    /// Tarboton 1997: steepest of eight triangular facets, flow split between its two neighbours.
    #[serde(rename = "dinf")]
    DInf,
    /// Freeman 1991 multiple-flow-direction: flow shared among all lower neighbours in proportion
    /// to slope^1.1 — the most diffuse (hillslope wash, wetness); streams come out wide.
    Mfd,
}

/// D∞ receivers: up to two downslope neighbours with weights summing to 1 (or none for pits).
pub fn dinf_receivers(
    h: &[f32],
    w: usize,
    hh: usize,
    spacing_m: [f32; 2],
) -> Vec<[(Option<u32>, f32); 2]> {
    // Facets go from a cardinal neighbour (e1) to the adjacent diagonal (e2); d1 is the spacing
    // along e1's axis, d2 the spacing along the other axis.
    const FACETS: [((i64, i64), (i64, i64)); 8] = [
        ((1, 0), (1, -1)),
        ((0, -1), (1, -1)),
        ((0, -1), (-1, -1)),
        ((-1, 0), (-1, -1)),
        ((-1, 0), (-1, 1)),
        ((0, 1), (-1, 1)),
        ((0, 1), (1, 1)),
        ((1, 0), (1, 1)),
    ];
    let mut out = vec![[(None, 0.0f32); 2]; w * hh];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, cell) in row.iter_mut().enumerate() {
            let h0 = h[y * w + x];
            let mut best: Option<(f32, f32, usize, usize, f32)> = None; // (slope, angle r, e1, e2, rmax)
            for (e1, e2) in FACETS {
                let (x1, y1) = (x as i64 + e1.0, y as i64 + e1.1);
                let (x2, y2) = (x as i64 + e2.0, y as i64 + e2.1);
                let inside =
                    |xx: i64, yy: i64| xx >= 0 && yy >= 0 && xx < w as i64 && yy < hh as i64;
                if !inside(x1, y1) || !inside(x2, y2) {
                    continue;
                }
                let (i1, i2) = (y1 as usize * w + x1 as usize, y2 as usize * w + x2 as usize);
                let (d1, d2) = if e1.0 != 0 {
                    (spacing_m[0], spacing_m[1])
                } else {
                    (spacing_m[1], spacing_m[0])
                };
                let s1 = (h0 - h[i1]) / d1;
                let s2 = (h[i1] - h[i2]) / d2;
                let rmax = (d2 / d1).atan();
                let (mut r, mut s) = (s2.atan2(s1), (s1 * s1 + s2 * s2).sqrt());
                if r < 0.0 {
                    r = 0.0;
                    s = s1;
                } else if r > rmax {
                    r = rmax;
                    s = (h0 - h[i2]) / d1.hypot(d2);
                }
                if s > 0.0 && best.is_none_or(|(bs, ..)| s > bs) {
                    best = Some((s, r, i1, i2, rmax));
                }
            }
            if let Some((_, r, i1, i2, rmax)) = best {
                let w2 = if rmax > 0.0 {
                    (r / rmax).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                *cell = [(Some(i1 as u32), 1.0 - w2), (Some(i2 as u32), w2)];
            }
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
    flow_accumulation_with(h, spacing_m, log_scale, FlowMethod::D8)
}

/// [`flow_accumulation`] with a selectable routing method.
pub fn flow_accumulation_with(
    h: &Heightfield,
    spacing_m: [f32; 2],
    log_scale: bool,
    method: FlowMethod,
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
    // D8 / D∞ precompute ≤ 2 receivers per cell; MFD has up to 8 and is computed on the fly in
    // the descending pass (no 8-slot table: 2048² would need ~270 MB).
    let recv: Vec<[(Option<u32>, f32); 2]> = match method {
        FlowMethod::D8 => d8_receivers(&v, w, hh, spacing_m)
            .into_iter()
            .map(|r| [(r, 1.0), (None, 0.0)])
            .collect(),
        FlowMethod::DInf => dinf_receivers(&v, w, hh, spacing_m),
        FlowMethod::Mfd => Vec::new(),
    };
    let dist =
        NEIGHBOURS.map(|(dx, dy)| (dx as f32 * spacing_m[0]).hypot(dy as f32 * spacing_m[1]));
    let mut order: Vec<u32> = (0..(w * hh) as u32).collect();
    // Descending height, ties by index: identical output regardless of thread count.
    order.par_sort_unstable_by(|a, b| v[*b as usize].total_cmp(&v[*a as usize]).then(a.cmp(b)));
    let mut acc = vec![1.0f32; w * hh];
    let mut slopes = [(0usize, 0.0f32); 8];
    for &i in &order {
        let a = acc[i as usize];
        if method == FlowMethod::Mfd {
            let (x, y) = ((i as usize % w) as i64, (i as usize / w) as i64);
            let mut n = 0;
            let mut total = 0.0f32;
            for (k, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= hh as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                let drop = v[i as usize] - v[j];
                if drop > 0.0 {
                    let s = (drop / dist[k]).powf(1.1);
                    slopes[n] = (j, s);
                    total += s;
                    n += 1;
                }
            }
            if total > 0.0 {
                for &(j, s) in &slopes[..n] {
                    acc[j] += a * (s / total);
                }
            }
            continue;
        }
        for (j, wt) in recv[i as usize] {
            if let Some(j) = j
                && wt > 0.0
            {
                acc[j as usize] += a * wt;
            }
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
    fill_depressions_with(h, epsilon_m, None)
}

/// [`fill_depressions`] with underground drainage: cells where `drains` > 0.5 are outlets like
/// the border — water reaching them leaves the surface (a karst sinkhole, a soakaway), so no lake
/// forms there and the basins that spill into them are not raised. The desert's dolines and
/// swallow holes (`jd.erosion.karst@1`'s `sinkholes` mask) are the use.
pub fn fill_depressions_with(
    h: &Heightfield,
    epsilon_m: f32,
    drains: Option<&Heightfield>,
) -> JdResult<(Heightfield, Heightfield)> {
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
    if let Some(d) = drains
        && (d.width(), d.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid("drains", "field size differs from `in`"));
    }
    let mut filled = orig.clone();
    let mut seen = vec![false; w * hh];
    let mut heap = BinaryHeap::new();
    for y in 0..hh {
        for x in 0..w {
            let border = x == 0 || y == 0 || x == w - 1 || y == hh - 1;
            if border || drains.is_some_and(|d| d.get(x as u32, y as u32) > 0.5) {
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
    method: FlowMethod,
) -> JdResult<(Heightfield, Heightfield)> {
    carve_rivers_with(h, spacing_m, threshold, depth_m, method, None)
}

/// [`carve_rivers`] with underground drainage: `drains` > 0.5 cells are outlets (see
/// [`fill_depressions_with`]) — a wadi that ends in a swallow hole stops there instead of
/// ponding and spilling on.
pub fn carve_rivers_with(
    h: &Heightfield,
    spacing_m: [f32; 2],
    threshold: f32,
    depth_m: f32,
    method: FlowMethod,
    drains: Option<&Heightfield>,
) -> JdResult<(Heightfield, Heightfield)> {
    if !(0.0..1.0).contains(&threshold) {
        return Err(JdError::invalid("threshold", "must be within 0..1"));
    }
    if !(depth_m >= 0.0 && depth_m.is_finite()) {
        return Err(JdError::invalid("depth_m", "must be finite and ≥ 0"));
    }
    let (filled, _) = fill_depressions_with(h, 0.001, drains)?;
    let acc = flow_accumulation_with(&filled, spacing_m, true, method)?;
    let mut mask = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    mask.fill_with(|x, y| {
        let t = ((acc.get(x, y) - threshold) / (1.0 - threshold)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    });
    let mut out = h.clone();
    out.fill_with(|x, y| h.get(x, y) - depth_m * mask.get(x, y));
    Ok((out, mask))
}

/// Sea level: `ocean` is 1 where the terrain lies below `level_m` **and** is connected to the
/// field border through such cells (inland basins below sea level stay 0); the mask softens to 0
/// over `shore_m` of height above the level. `depth` is `level_m - h` (≥ 0) on ocean cells.
pub fn sea_level(
    h: &Heightfield,
    level_m: f32,
    shore_m: f32,
) -> JdResult<(Heightfield, Heightfield)> {
    if !level_m.is_finite() {
        return Err(JdError::invalid("level_m", "must be finite"));
    }
    if !(shore_m >= 0.0 && shore_m.is_finite()) {
        return Err(JdError::invalid("shore_m", "must be finite and ≥ 0"));
    }
    let (w, hh) = (h.width() as usize, h.height() as usize);
    let v = h.to_vec();
    if v.iter().any(|x| !x.is_finite()) {
        return Err(JdError::invalid("in", "heightfield has non-finite samples"));
    }
    // Flood from every border cell below the level through 4-connected cells below the level.
    let below = |i: usize| v[i] < level_m + shore_m;
    let mut sea = vec![false; w * hh];
    let mut stack: Vec<usize> = (0..w * hh)
        .filter(|&i| (i % w == 0 || i % w == w - 1 || i < w || i >= w * (hh - 1)) && below(i))
        .collect();
    for &i in &stack {
        sea[i] = true;
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        for j in [
            (x > 0).then(|| i - 1),
            (x + 1 < w).then(|| i + 1),
            (y > 0).then(|| i - w),
            (y + 1 < hh).then(|| i + w),
        ]
        .into_iter()
        .flatten()
        {
            if !sea[j] && below(j) {
                sea[j] = true;
                stack.push(j);
            }
        }
    }
    let mut ocean = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    let mut depth = ocean.clone();
    ocean.fill_with(|x, y| {
        let i = y as usize * w + x as usize;
        if !sea[i] {
            return 0.0;
        }
        if shore_m <= 0.0 {
            return 1.0;
        }
        let t = ((level_m + shore_m - v[i]) / shore_m).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    });
    depth.fill_with(|x, y| {
        let i = y as usize * w + x as usize;
        if sea[i] {
            (level_m - v[i]).max(0.0)
        } else {
            0.0
        }
    });
    Ok((ocean, depth))
}

/// Topographic wetness index (Beven & Kirkby 1979): `ln(a / tan β)` with `a` the upslope area per
/// unit contour length (D8 accumulation × cell area / cell width) and `β` the local slope; the
/// result is normalised to 0..1 across the field so it reads as a mask (wet valleys → 1).
pub fn wetness_index(h: &Heightfield, spacing_m: [f32; 2]) -> JdResult<Heightfield> {
    wetness_index_with(h, spacing_m, FlowMethod::D8)
}

/// [`wetness_index`] with a choice of flow routing: `dinf` removes the 45° streaks of D8 on
/// smooth slopes, `mfd` spreads the drainage area so the index reads as moisture rather than
/// as channels.
pub fn wetness_index_with(
    h: &Heightfield,
    spacing_m: [f32; 2],
    method: FlowMethod,
) -> JdResult<Heightfield> {
    let acc = flow_accumulation_with(h, spacing_m, false, method)?;
    let slope = crate::slope_mask(h, spacing_m, 90.0)?; // slope angle / 90 → degrees = ×90
    let (w, hh) = (h.width(), h.height());
    let cell_w = spacing_m[0].min(spacing_m[1]);
    let area = spacing_m[0] * spacing_m[1];
    let mut twi = vec![0.0f32; (w * hh) as usize];
    for y in 0..hh {
        for x in 0..w {
            let a = acc.get(x, y) * area / cell_w;
            let tan_b = (slope.get(x, y) * 90.0).to_radians().tan().max(1e-3);
            twi[(y * w + x) as usize] = (a / tan_b).ln();
        }
    }
    let (lo, hi) = twi
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let span = (hi - lo).max(1e-6);
    let mut out = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    out.fill_with(|x, y| (twi[(y * w + x) as usize] - lo) / span);
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
        // Underground drainage: a swallow hole at the pit floor drains the basin — no lake, no fill.
        let mut drains =
            Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo()).expect("d");
        let (lx, ly) = {
            let mut best = (0, 0, f32::MAX);
            for y in 0..h.height() {
                for x in 0..h.width() {
                    if lakes.get(x, y) > best.2 || best.2 == f32::MAX && lakes.get(x, y) > 0.0 {
                        best = (x, y, lakes.get(x, y));
                    }
                }
            }
            (best.0, best.1)
        };
        drains.fill_with(|x, y| f32::from(x == lx && y == ly));
        let (drained, dry) = fill_depressions_with(&h, 0.0, Some(&drains)).expect("drained");
        assert_eq!(dry.get(lx, ly), 0.0, "the drain cell holds no water");
        assert!(
            dry.to_vec().iter().sum::<f32>() < lakes.to_vec().iter().sum::<f32>() * 0.5,
            "the basin drained"
        );
        assert_eq!(drained.get(lx, ly), h.get(lx, ly));
        assert!(
            fill_depressions_with(&h, 0.0, Some(&Heightfield::new(4, 4, 64, 0).expect("s")))
                .is_err()
        );
    }

    #[test]
    fn rivers_carve_where_flow_concentrates() {
        // A V-shaped valley: water from both slopes gathers along x = 16 and flows north (y → 0).
        let mut h = Heightfield::new(33, 24, 32, 4).expect("hf");
        h.fill_with(|x, y| (x as f32 - 16.0).abs() * 4.0 + y as f32 * 0.5);
        let (carved, mask) =
            carve_rivers(&h, [1.0, 1.0], 0.6, 10.0, FlowMethod::D8).expect("carve");
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
            carve_rivers(&h, [1.0, 1.0], 0.6, 10.0, FlowMethod::D8)
                .expect("again")
                .0
                .content_hash(),
            carved.content_hash()
        );
        assert!(carve_rivers(&h, [1.0, 1.0], 1.0, 10.0, FlowMethod::D8).is_err());
    }

    #[test]
    fn wetness_is_high_on_flat_valley_floors_and_low_on_steep_ridges() {
        let mut h = Heightfield::new(33, 24, 32, 4).expect("hf");
        h.fill_with(|x, y| (x as f32 - 16.0).abs() * 4.0 + y as f32 * 0.5);
        let twi = wetness_index(&h, [1.0, 1.0]).expect("twi");
        let s = twi.stats();
        assert_eq!(s.non_finite, 0);
        assert!(s.min >= 0.0 && s.max <= 1.0 + 1e-6);
        assert!(
            twi.get(16, 2) > twi.get(3, 20),
            "valley outlet {} wetter than the ridge {}",
            twi.get(16, 2),
            twi.get(3, 20)
        );
        assert_eq!(
            wetness_index(&h, [1.0, 1.0]).expect("again").content_hash(),
            twi.content_hash()
        );
    }

    #[test]
    fn dinf_matches_d8_on_cardinal_slopes_and_splits_diagonal_flow() {
        // Westward plane: D∞ sends everything to the single cardinal neighbour, like D8.
        let mut h = Heightfield::new(32, 8, 32, 0).expect("hf");
        h.fill_with(|x, _| x as f32);
        let d8 = flow_accumulation(&h, [1.0, 1.0], false).expect("d8");
        let dinf = flow_accumulation_with(&h, [1.0, 1.0], false, FlowMethod::DInf).expect("dinf");
        assert_eq!(d8.to_vec(), dinf.to_vec());
        // A plane sloping between west and north-west (≈ 22.5°) splits each cell's flow two ways.
        let mut p = Heightfield::new(16, 16, 16, 0).expect("hf");
        let a = 22.5f32.to_radians();
        p.fill_with(|x, y| x as f32 * a.cos() + y as f32 * a.sin());
        let v = p.to_vec();
        let recv = dinf_receivers(&v, 16, 16, [1.0, 1.0]);
        let [(r1, w1), (r2, w2)] = recv[8 * 16 + 8];
        assert!(r1.is_some() && r2.is_some(), "two receivers");
        assert!(
            (w1 + w2 - 1.0).abs() < 1e-6 && (w1 - 0.5).abs() < 0.05 && (w2 - 0.5).abs() < 0.05,
            "weights {w1} / {w2}"
        );
        // Total area is conserved: cells without receivers (the low edge) hold exactly N.
        let acc = flow_accumulation_with(&p, [1.0, 1.0], false, FlowMethod::DInf).expect("dinf");
        let total: f32 = (0..256)
            .filter(|&i| recv[i][0].0.is_none())
            .map(|i| acc.get((i % 16) as u32, (i / 16) as u32))
            .sum();
        assert!(
            (total - 256.0).abs() < 1e-2,
            "outlets hold the whole area: {total}"
        );
    }

    #[test]
    fn sea_level_floods_from_the_border_but_not_inland_basins() {
        // Flat land at 50 m with a coast (x < 8 at −20 m) and an inland pit at −30 m.
        let mut h = Heightfield::new(32, 32, 32, 0).expect("hf");
        h.fill_with(|x, y| {
            if x < 8 {
                -20.0
            } else if (15..17).contains(&x) && (15..17).contains(&y) {
                -30.0
            } else {
                50.0
            }
        });
        let (ocean, depth) = sea_level(&h, 0.0, 0.0).expect("sea");
        assert_eq!(ocean.get(2, 10), 1.0);
        assert_eq!(depth.get(2, 10), 20.0);
        assert_eq!(ocean.get(15, 15), 0.0, "inland basin is not ocean");
        assert_eq!(depth.get(15, 15), 0.0);
        assert_eq!(ocean.get(20, 20), 0.0);
        // A soft shore ramps 1 → 0 across shore_m above the level.
        let mut ramp = Heightfield::new(64, 4, 64, 0).expect("hf");
        ramp.fill_with(|x, _| x as f32 - 32.0);
        let (o, _) = sea_level(&ramp, 0.0, 10.0).expect("sea");
        assert_eq!(o.get(20, 1), 1.0);
        assert!((o.get(37, 1) - 0.5).abs() < 1e-5, "{}", o.get(37, 1));
        assert_eq!(o.get(50, 1), 0.0);
        assert!(sea_level(&ramp, f32::NAN, 1.0).is_err());
        assert!(sea_level(&ramp, 0.0, -1.0).is_err());
    }

    #[test]
    fn mfd_conserves_area_and_is_more_diffuse_than_d8() {
        // Everything drains to the valley mouth. The cells
        // with no lower neighbour (the outlets) must collect exactly N units under every method;
        // MFD spreads flow over more cells than D8 does.
        let mut h = Heightfield::new(32, 16, 32, 0).expect("hf");
        // V-valley along x = 16 sloping down towards y = 0 (plus a ripple so no two cells tie).
        h.fill_with(|x, y| {
            (x as f32 - 16.0).abs() * 2.0 + y as f32 * 0.5 + (y as f32 * 0.37).sin() * 0.01
        });
        let v = h.to_vec();
        let (w, hh) = (32usize, 16usize);
        let is_outlet = |i: usize| {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            !NEIGHBOURS.iter().any(|(dx, dy)| {
                let (nx, ny) = (x + dx, y + dy);
                nx >= 0
                    && ny >= 0
                    && nx < w as i64
                    && ny < hh as i64
                    && v[ny as usize * w + nx as usize] < v[i]
            })
        };
        let outlet_sum = |acc: &Heightfield| -> f32 {
            (0..w * hh)
                .filter(|&i| is_outlet(i))
                .map(|i| acc.get((i % w) as u32, (i / w) as u32))
                .sum()
        };
        let d8 = flow_accumulation(&h, [1.0, 1.0], false).expect("d8");
        let mfd = flow_accumulation_with(&h, [1.0, 1.0], false, FlowMethod::Mfd).expect("mfd");
        assert!(
            (outlet_sum(&d8) - 512.0).abs() < 1e-2,
            "d8 outlets: {}",
            outlet_sum(&d8)
        );
        assert!(
            (outlet_sum(&mfd) - 512.0).abs() < 1e-2,
            "mfd outlets: {}",
            outlet_sum(&mfd)
        );
        // Diffuse: D8 funnels everything into the valley line, MFD shares it with the slopes —
        // the same total is less concentrated (smaller sum of squares).
        let concentration =
            |acc: &Heightfield| acc.to_vec().into_iter().map(|a| a * a).sum::<f32>();
        assert!(
            concentration(&mfd) < concentration(&d8),
            "mfd {} vs d8 {}",
            concentration(&mfd),
            concentration(&d8)
        );
        let again = flow_accumulation_with(&h, [1.0, 1.0], false, FlowMethod::Mfd).expect("mfd");
        assert_eq!(mfd.content_hash(), again.content_hash());
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
