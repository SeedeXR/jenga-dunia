//! Waterfalls on the river network: where a river meets a near-vertical step in its bed
//! (`docs/research/research-log/2026-11-08-water-features.md`).
//!
//! A heightfield cannot hold a free-falling jet, but it holds the step: following each river's
//! D8 path, a **fall** is a run of consecutive river steps at least `min_gradient` steep (rise
//! over run; 1 = 45°) whose total drop is at least `min_drop_m` (20 ft, 6.1 m, by convention).
//! The drop threshold is what separates a fall from steep rapids, which a gradient threshold
//! alone would also flag (Hayakawa & Oguchi's knickzone caveat).
//!
//! Below each fall a **plunge pool** is dug into the floor — never into the cliff above it — as a
//! bowl `pool_depth × drop` deep and `pool_radius × drop` wide. The defaults are medians of
//! Scheingross & Lamb's 75 surveyed pools (JGR 2016, Table S1): depth 0.33 × drop for pools not
//! filled by sediment (0.20 counting the filled ones), radius 0.57 × drop. The pool holds water
//! up to the foot's level (its outlet), returned as a depth layer — only the pools, unlike a fill
//! node, which would lake every hollow of the terrain.
//!
//! ponytail: one fall per steep run (a cascade of small steps separated by flat reaches stays
//! several falls or none); retreat, undercutting and spray are not simulated.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::flow::{FlowMethod, d8_receivers, fill_depressions, flow_accumulation_with};

/// Waterfall settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FallsParams {
    /// Smallest total drop that counts as a waterfall, metres (> 0). 6.1 m = the 20 ft convention.
    #[serde(default = "d_min_drop")]
    pub min_drop_m: f32,
    /// Smallest step gradient (rise over run) inside a fall (> 0): 1 = 45°, 4 = 76° (Mabin's
    /// near-vertical falls, which need metre cells to show).
    #[serde(default = "d_min_gradient")]
    pub min_gradient: f32,
    /// Plunge pool depth as a share of the drop (≥ 0; 0 = no pool).
    #[serde(default = "d_pool_depth")]
    pub pool_depth: f32,
    /// Plunge pool radius as a share of the drop (≥ 0).
    #[serde(default = "d_pool_radius")]
    pub pool_radius: f32,
    /// Without a `rivers` mask: rivers are where the log-scaled flow accumulation exceeds this
    /// (0..1), as in `jd.hydrology.rivers@1`.
    #[serde(default = "d_threshold")]
    pub threshold: f32,
}

fn d_min_drop() -> f32 {
    6.1
}
fn d_min_gradient() -> f32 {
    1.0
}
fn d_pool_depth() -> f32 {
    0.33
}
fn d_pool_radius() -> f32 {
    0.57
}
fn d_threshold() -> f32 {
    0.6
}

impl Default for FallsParams {
    fn default() -> Self {
        Self {
            min_drop_m: d_min_drop(),
            min_gradient: d_min_gradient(),
            pool_depth: d_pool_depth(),
            pool_radius: d_pool_radius(),
            threshold: d_threshold(),
        }
    }
}

impl FallsParams {
    fn validate(&self) -> JdResult<()> {
        let positive = |v: f32| v.is_finite() && v > 0.0;
        if !positive(self.min_drop_m) {
            return Err(JdError::invalid("min_drop_m", "must be finite and > 0"));
        }
        if !positive(self.min_gradient) {
            return Err(JdError::invalid("min_gradient", "must be finite and > 0"));
        }
        for (k, v) in [
            ("pool_depth", self.pool_depth),
            ("pool_radius", self.pool_radius),
        ] {
            if !(v.is_finite() && v >= 0.0) {
                return Err(JdError::invalid(k, "must be finite and ≥ 0"));
            }
        }
        if !(0.0..1.0).contains(&self.threshold) {
            return Err(JdError::invalid("threshold", "must be within 0..1"));
        }
        Ok(())
    }
}

/// One waterfall, in cells of the input grid (x east, y north) and metres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Waterfall {
    /// Cell at the top of the fall (the lip).
    pub lip: [u32; 2],
    /// Cell at the foot of the fall (the pool's centre).
    pub base: [u32; 2],
    /// Bed height at the lip, metres.
    pub lip_m: f32,
    /// Bed height at the foot before the pool, metres.
    pub base_m: f32,
    /// Lip minus foot, metres.
    pub drop_m: f32,
    /// Horizontal length of the fall along the river, metres.
    pub run_m: f32,
    /// Width of the fall, metres: the river's wet width across the flow at the lip (with a
    /// `river_water` layer), else its lips side by side × the cell size.
    pub width_m: f32,
    /// Where the falling sheet's centre sits from the lip cell's centre, east and north metres:
    /// the river's water may spread more to one bank than the other.
    pub shift_m: [f32; 2],
    /// Upstream area draining over the fall, km².
    pub area_km2: f32,
    /// Plunge pool depth below the foot, metres.
    pub pool_depth_m: f32,
    /// Plunge pool radius, metres.
    pub pool_radius_m: f32,
}

/// What [`waterfalls`] produces.
#[derive(Debug, Clone)]
pub struct FallsResult {
    /// The terrain with the plunge pools dug.
    pub height: Heightfield,
    /// 0..1: 1 on the fall faces, the pools fading from 1 at their centre to 0 at the rim.
    pub mask: Heightfield,
    /// Water depth in the plunge pools, metres (the surface at each foot's level).
    pub water: Heightfield,
    /// The falls, ordered by lip cell (south to north, then west to east), so the list is
    /// deterministic.
    pub falls: Vec<Waterfall>,
}

/// Find the waterfalls on the rivers of `h` and dig their plunge pools. `rivers` is a 0..1 river
/// mask (≥ 0.5 = river; `jd.hydrology.rivers@1`'s); without one, rivers come from flow
/// accumulation over `p.threshold`. `river_water` (the rivers node's `water`, metres) sets each
/// fall's width: the river's wet width across the flow at the lip, never less than its lips.
pub fn waterfalls(
    h: &Heightfield,
    spacing_m: [f32; 2],
    rivers: Option<&Heightfield>,
    river_water: Option<&Heightfield>,
    p: &FallsParams,
) -> JdResult<FallsResult> {
    p.validate()?;
    if !(spacing_m[0] > 0.0 && spacing_m[1] > 0.0) {
        return Err(JdError::invalid("spacing_m", "must be > 0"));
    }
    let (w, hh) = (h.width() as usize, h.height() as usize);
    for (k, f) in [("rivers", rivers), ("water", river_water)] {
        if let Some(r) = f
            && (r.width() as usize, r.height() as usize) != (w, hh)
        {
            return Err(JdError::invalid(k, "must match the terrain's size"));
        }
    }
    let wet = river_water.map(|r| r.to_vec());
    let heights = h.to_vec();
    // Paths follow the drainage the rivers node carved along: routing on the filled terrain, so a
    // pit does not end a river above its fall.
    let (filled, _) = fill_depressions(h, 0.001)?;
    let recv = d8_receivers(&filled.to_vec(), w, hh, spacing_m);
    let acc = flow_accumulation_with(&filled, spacing_m, false, FlowMethod::D8)?.to_vec();
    let n = w * hh;
    let river: Vec<bool> = match rivers {
        Some(r) => r.to_vec().iter().map(|v| *v >= 0.5).collect(),
        // The log scale of `flow_accumulation_with`: ln(1 + a) / ln(1 + cells).
        None => acc
            .iter()
            .map(|a| (1.0 + a).ln() / (1.0 + n as f32).ln() > p.threshold)
            .collect(),
    };
    let step = |i: usize, j: usize| {
        let (dx, dy) = (
            (i % w).abs_diff(j % w) as f32,
            (i / w).abs_diff(j / w) as f32,
        );
        (dx * spacing_m[0]).hypot(dy * spacing_m[1])
    };
    let steep: Vec<bool> = (0..n)
        .map(|i| {
            river[i]
                && recv[i].is_some_and(|r| {
                    let r = r as usize;
                    (heights[i] - heights[r]) / step(i, r) >= p.min_gradient
                })
        })
        .collect();
    // A cell is fed by its largest donor (ties to the lower index): a small steep tributary joining
    // at the top of a big fall does not take the main river's lip.
    let mut main_donor: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        if let Some(r) = recv[i] {
            let r = r as usize;
            if main_donor[r].is_none_or(|d| acc[i] > acc[d]) {
                main_donor[r] = Some(i);
            }
        }
    }
    let fed_steep: Vec<bool> = main_donor
        .iter()
        .map(|d| d.is_some_and(|d| steep[d]))
        .collect();
    let cell_area_km2 = spacing_m[0] * spacing_m[1] / 1.0e6;
    // Every steep run that drops far enough: (lip, foot, run length, face cells).
    let mut runs: Vec<(usize, usize, f32, Vec<usize>)> = Vec::new();
    for lip in 0..n {
        // A run starts at a steep river step that no steep step flows into.
        if !steep[lip] || fed_steep[lip] {
            continue;
        }
        let (mut cur, mut run, mut path) = (lip, 0.0f32, vec![lip]);
        while steep[cur] {
            let Some(r) = recv[cur] else { break };
            let r = r as usize;
            run += step(cur, r);
            cur = r;
            path.push(cur); // receivers on the ε-filled terrain strictly descend: no cycles
        }
        if heights[lip] - heights[cur] >= p.min_drop_m {
            path.pop();
            runs.push((lip, cur, run, path));
        }
    }
    // A river several cells wide crosses a cliff in parallel runs: runs whose lips touch
    // (8-connected), or that land on the same foot (a tributary steep down to the main lip), are
    // one fall, led by the lip carrying the most water; how many lips it has is its width.
    let mut group: Vec<usize> = (0..runs.len()).collect();
    fn root(g: &mut [usize], mut i: usize) -> usize {
        while g[i] != i {
            g[i] = g[g[i]];
            i = g[i];
        }
        i
    }
    let at: std::collections::HashMap<usize, usize> =
        runs.iter().enumerate().map(|(k, r)| (r.0, k)).collect();
    let mut by_foot: std::collections::HashMap<usize, usize> = Default::default();
    for (k, r) in runs.iter().enumerate() {
        if let Some(&j) = by_foot.get(&r.1) {
            let (a, b) = (root(&mut group, k), root(&mut group, j));
            group[a.max(b)] = a.min(b);
        } else {
            by_foot.insert(r.1, k);
        }
        let (x, y) = ((r.0 % w) as i64, (r.0 / w) as i64);
        for (dx, dy) in [(1i64, 0i64), (-1, 1), (0, 1), (1, 1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= hh as i64 {
                continue;
            }
            if let Some(&j) = at.get(&(ny as usize * w + nx as usize)) {
                let (a, b) = (root(&mut group, k), root(&mut group, j));
                group[a.max(b)] = a.min(b);
            }
        }
    }
    let mut members: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for k in 0..runs.len() {
        let g = root(&mut group, k);
        members.entry(g).or_default().push(k);
    }
    let cell = spacing_m[0].min(spacing_m[1]);
    let mut falls = Vec::new();
    let mut faces: Vec<usize> = Vec::new();
    for ks in members.values() {
        let Some(&lead) = ks.iter().max_by(|a, b| {
            acc[runs[**a].0]
                .total_cmp(&acc[runs[**b].0])
                .then(runs[**b].0.cmp(&runs[**a].0))
        }) else {
            continue;
        };
        let (lip, foot, run, _) = &runs[lead];
        let (lip, foot) = (*lip, *foot);
        let drop = heights[lip] - heights[foot];
        // The river's wet span across the flow at the lip: wider than the lips, centred on the water.
        let span = wet
            .as_ref()
            .map(|d| wet_span(d, w, hh, lip, foot, spacing_m))
            .filter(|(width, _)| *width > ks.len() as f32 * cell)
            .unwrap_or((0.0, [0.0, 0.0]));
        for &k in ks {
            faces.extend_from_slice(&runs[k].3);
        }
        falls.push(Waterfall {
            lip: [(lip % w) as u32, (lip / w) as u32],
            base: [(foot % w) as u32, (foot / w) as u32],
            lip_m: heights[lip],
            base_m: heights[foot],
            drop_m: drop,
            run_m: *run,
            width_m: (ks.len() as f32 * cell).max(span.0),
            shift_m: span.1,
            area_km2: ks.iter().map(|&k| acc[runs[k].0]).sum::<f32>() * cell_area_km2,
            pool_depth_m: p.pool_depth * drop,
            pool_radius_m: if p.pool_radius > 0.0 {
                (p.pool_radius * drop).max(cell)
            } else {
                0.0
            },
        });
    }
    falls.sort_by_key(|f| (f.lip[1], f.lip[0]));
    let mut height = h.clone();
    let mut mask = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    for &i in &faces {
        mask.set((i % w) as u32, (i / w) as u32, 1.0);
    }
    let mut pool = vec![false; n];
    for f in &falls {
        dig_pool(&mut height, &mut mask, &mut pool, f, spacing_m);
    }
    // The pools hold water to their own spill level — the lowest outlet of the dug bowl, found by
    // filling the dug terrain — and the water is the lake each pool sits in, nothing else.
    let (_, lakes) = fill_depressions(&height, 0.0)?;
    let lakes = lakes.to_vec();
    let mut wet = vec![false; n];
    let mut todo: Vec<usize> = (0..n).filter(|i| pool[*i] && lakes[*i] > 0.0).collect();
    while let Some(i) = todo.pop() {
        if std::mem::replace(&mut wet[i], true) {
            continue;
        }
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx >= 0 && ny >= 0 && nx < w as i64 && ny < hh as i64 {
                let j = ny as usize * w + nx as usize;
                if !wet[j] && lakes[j] > 0.0 {
                    todo.push(j);
                }
            }
        }
    }
    let mut water = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    water.fill_with(|x, y| {
        let i = y as usize * w + x as usize;
        if wet[i] { lakes[i] } else { 0.0 }
    });
    Ok(FallsResult {
        height,
        mask,
        water,
        falls,
    })
}

/// The river's wet span (water ≥ [`crate::water::WET_M`]) across the flow at `lip`: from the lip
/// cell, steps of one cell along the perpendicular to `lip → foot`, both ways, while still wet.
/// Returns its width and its centre's offset from the lip (east, north metres). Meant for the
/// rivers node's water: a lake layer would stretch the fall across the whole lake.
fn wet_span(
    depth: &[f32],
    w: usize,
    hh: usize,
    lip: usize,
    foot: usize,
    spacing_m: [f32; 2],
) -> (f32, [f32; 2]) {
    let (lx, ly) = ((lip % w) as f32, (lip / w) as f32);
    let (dx, dy) = (
        ((foot % w) as f32 - lx) * spacing_m[0],
        ((foot / w) as f32 - ly) * spacing_m[1],
    );
    let len = dx.hypot(dy).max(f32::EPSILON);
    let cell = spacing_m[0].min(spacing_m[1]);
    // Across the flow, in cells per step of one `cell`.
    let (px, py) = (
        -dy / len * cell / spacing_m[0],
        dx / len * cell / spacing_m[1],
    );
    let wet_at = |k: f32| {
        let (x, y) = ((lx + px * k).round(), (ly + py * k).round());
        x >= 0.0
            && y >= 0.0
            && (x as usize) < w
            && (y as usize) < hh
            && depth[y as usize * w + x as usize] >= crate::water::WET_M
    };
    if !wet_at(0.0) {
        return (0.0, [0.0, 0.0]);
    }
    let side = |sign: f32| {
        (1..)
            .take_while(|k| *k <= w.max(hh) && wet_at(sign * *k as f32))
            .count()
    };
    let (right, left) = (side(1.0) as f32, side(-1.0) as f32);
    let mid = (right - left) / 2.0; // in steps of one cell along the perpendicular
    (
        (1.0 + right + left) * cell,
        [px * mid * spacing_m[0], py * mid * spacing_m[1]],
    )
}

/// Dig `f`'s plunge pool: a raised-cosine bowl centred on the foot, lowering only ground at or
/// below the foot (the floor), never the cliff above it; `pool` marks the cells dug. No depth or
/// no radius: no pool.
fn dig_pool(
    height: &mut Heightfield,
    mask: &mut Heightfield,
    pool: &mut [bool],
    f: &Waterfall,
    spacing_m: [f32; 2],
) {
    if f.pool_radius_m <= 0.0 || f.pool_depth_m <= 0.0 {
        return;
    }
    let (w, hh) = (height.width() as i64, height.height() as i64);
    let (cx, cy) = (f.base[0] as i64, f.base[1] as i64);
    let (rx, ry) = (
        (f.pool_radius_m / spacing_m[0]).ceil() as i64,
        (f.pool_radius_m / spacing_m[1]).ceil() as i64,
    );
    for y in (cy - ry).max(0)..=(cy + ry).min(hh - 1) {
        for x in (cx - rx).max(0)..=(cx + rx).min(w - 1) {
            let d = (((x - cx) as f32) * spacing_m[0]).hypot(((y - cy) as f32) * spacing_m[1]);
            if d > f.pool_radius_m {
                continue;
            }
            let t = 0.5 + 0.5 * (std::f32::consts::PI * d / f.pool_radius_m).cos(); // 1 → 0
            let (ux, uy) = (x as u32, y as u32);
            let ground = height.get(ux, uy);
            if ground <= f.base_m + 1e-3 {
                height.set(ux, uy, ground.min(f.base_m - f.pool_depth_m * t));
                pool[(y * w + x) as usize] = true;
            }
            mask.set(ux, uy, mask.get(ux, uy).max(t));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valley running north → south down a 100 m-wide plain with a `step_m` cliff across it at
    /// row 32 (5 m cells): a river along the middle column and its mask.
    fn valley(step_m: f32) -> (Heightfield, Heightfield) {
        let mut h = Heightfield::new(48, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| {
            let across = (x as f32 - 24.0).abs() * 0.5; // a V across the valley, river in the middle
            let down = y as f32 * 0.3 + if y >= 32 { step_m } else { 0.0 };
            200.0 + down + across
        });
        let mut rivers = h.clone();
        rivers.fill_with(|x, _| if (23..=25).contains(&x) { 1.0 } else { 0.0 });
        (h, rivers)
    }

    #[test]
    fn a_cliff_across_a_river_is_one_fall_with_a_plunge_pool_dug_below_it() {
        // y grows north and the valley rises northwards, so the river runs south over the cliff.
        let (h, rivers) = valley(20.0);
        let r = waterfalls(&h, [5.0, 5.0], Some(&rivers), None, &FallsParams::default())
            .expect("falls");
        assert_eq!(r.falls.len(), 1, "{:?}", r.falls);
        let f = &r.falls[0];
        assert_eq!(
            (f.lip[0], f.lip[1]),
            (24, 32),
            "the lip is the cliff's top cell on the river"
        );
        assert_eq!(f.base[1], 31, "the foot is the cell below it");
        assert!((f.drop_m - 20.3).abs() < 0.01, "{}", f.drop_m);
        assert!((f.pool_depth_m - 0.33 * f.drop_m).abs() < 1e-4);
        assert!((f.pool_radius_m - 0.57 * f.drop_m).abs() < 1e-4);
        assert!(
            (f.width_m - 15.0).abs() < 1e-3,
            "three 5 m river cells side by side: {}",
            f.width_m
        );
        // The pool: the foot lowered by the full depth, the cliff above untouched.
        let foot = r.height.get(24, 31);
        assert!(
            (h.get(24, 31) - f.pool_depth_m - foot).abs() < 1e-3,
            "{foot}"
        );
        assert_eq!(
            r.height.get(24, 33),
            h.get(24, 33),
            "the cliff top is never dug"
        );
        assert_eq!(r.mask.get(24, 32), 1.0, "the face is in the mask");
        // The pool holds water to its spill level: one flat surface, never above the foot.
        let level = |x, y| r.height.get(x, y) + r.water.get(x, y);
        assert!(r.water.get(24, 31) > 1.0, "{}", r.water.get(24, 31));
        let wet: Vec<f32> = (0..48)
            .flat_map(|x| (0..64).map(move |y| (x, y)))
            .filter(|(x, y)| r.water.get(*x, *y) > 0.0)
            .map(|(x, y)| level(x, y))
            .collect();
        assert!(
            wet.iter()
                .all(|l| (l - wet[0]).abs() < 1e-3 && *l <= f.base_m + 1e-3),
            "{wet:?}"
        );
        assert_eq!(r.water.get(24, 33), 0.0, "no water above the fall");
        assert_eq!(r.water.get(10, 31), 0.0, "nor beyond the pool");
        assert!(r.mask.get(24, 31) > 0.99 && r.mask.get(24, 20) == 0.0);
    }

    #[test]
    fn a_fall_is_as_wide_as_the_rivers_water_at_its_lip() {
        // The mask's river is 3 cells wide; its water spreads over 5 (x 22..=26).
        let (h, rivers) = valley(20.0);
        let mut water = h.clone();
        water.fill_with(|x, _| if (22..=26).contains(&x) { 1.0 } else { 0.0 });
        let r = waterfalls(
            &h,
            [5.0, 5.0],
            Some(&rivers),
            Some(&water),
            &FallsParams::default(),
        )
        .expect("falls");
        assert_eq!(r.falls.len(), 1);
        assert!(
            (r.falls[0].width_m - 25.0).abs() < 1e-3,
            "{}",
            r.falls[0].width_m
        );
        assert_eq!(r.falls[0].shift_m, [0.0, 0.0], "centred on the lip");
        // Water spread to one bank (x 23..=30, lip at 24): the sheet is 40 m wide and its centre
        // sits on the water's, 2.5 cells (12.5 m) across from the lip.
        water.fill_with(|x, _| if (23..=30).contains(&x) { 1.0 } else { 0.0 });
        let r = waterfalls(
            &h,
            [5.0, 5.0],
            Some(&rivers),
            Some(&water),
            &FallsParams::default(),
        )
        .expect("falls");
        let f = &r.falls[0];
        assert!((f.width_m - 40.0).abs() < 1e-3, "{}", f.width_m);
        assert!(
            (f.shift_m[0] - 12.5).abs() < 1e-3 && f.shift_m[1].abs() < 1e-3,
            "{:?}",
            f.shift_m
        );
        // Water narrower than the lips never narrows the fall; a mismatched layer is refused.
        water.fill_with(|x, _| if x == 24 { 1.0 } else { 0.0 });
        let r = waterfalls(
            &h,
            [5.0, 5.0],
            Some(&rivers),
            Some(&water),
            &FallsParams::default(),
        )
        .expect("falls");
        assert!((r.falls[0].width_m - 15.0).abs() < 1e-3);
        let small = Heightfield::new(8, 8, 8, 0).expect("hf");
        assert!(
            waterfalls(
                &h,
                [5.0, 5.0],
                Some(&rivers),
                Some(&small),
                &FallsParams::default()
            )
            .is_err()
        );
    }

    #[test]
    fn a_pool_above_a_gorge_spills_over_its_rim_and_no_pool_digs_nothing() {
        // Below the cliff the valley drops 2 m a cell: the pool's rim downstream is far below the
        // foot, so the water stands at the rim, not at the foot over the open gorge.
        let mut h = Heightfield::new(48, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| {
            let across = (x as f32 - 24.0).abs() * 0.5;
            let down = if y >= 32 {
                y as f32 * 0.3 + 30.0
            } else {
                y as f32 * 2.0
            };
            200.0 + down + across
        });
        let mut rivers = h.clone();
        rivers.fill_with(|x, _| if (23..=25).contains(&x) { 1.0 } else { 0.0 });
        let r = waterfalls(&h, [5.0, 5.0], Some(&rivers), None, &FallsParams::default())
            .expect("falls");
        let f = &r.falls[0];
        for y in 0..31 {
            let surface = r.height.get(24, y) + r.water.get(24, y);
            assert!(
                r.water.get(24, y) == 0.0 || surface < f.base_m - 1.0,
                "row {y}: water up to {surface} over the gorge (foot {})",
                f.base_m
            );
        }
        // pool_depth 0: falls, but nothing dug, no water, no pool disk in the mask.
        let p = FallsParams {
            pool_depth: 0.0,
            ..FallsParams::default()
        };
        let r = waterfalls(&h, [5.0, 5.0], Some(&rivers), None, &p).expect("falls");
        assert_eq!(r.falls.len(), 1);
        assert_eq!(r.height.to_vec(), h.to_vec());
        assert!(r.water.to_vec().iter().all(|w| *w == 0.0));
        assert_eq!(r.mask.get(20, 31), 0.0, "no pool disk beside the foot");
    }

    #[test]
    fn small_steps_are_rapids_and_steps_off_the_river_are_cliffs_not_falls() {
        // A 3 m step: steep, but under 6.1 m — rapids.
        let (h, rivers) = valley(3.0);
        let r = waterfalls(&h, [5.0, 5.0], Some(&rivers), None, &FallsParams::default())
            .expect("falls");
        assert!(r.falls.is_empty(), "{:?}", r.falls);
        assert_eq!(r.height.to_vec(), h.to_vec(), "nothing dug");
        // A 20 m step with no river on it: a cliff.
        let (h, _) = valley(20.0);
        let mut dry = h.clone();
        dry.fill_with(|_, _| 0.0);
        let r =
            waterfalls(&h, [5.0, 5.0], Some(&dry), None, &FallsParams::default()).expect("falls");
        assert!(r.falls.is_empty());
        // The 3 m step over a 5 m cell is a 0.66 gradient: lower both thresholds and it is a fall.
        let (h, rivers) = valley(3.0);
        let p = FallsParams {
            min_drop_m: 2.0,
            min_gradient: 0.5,
            ..FallsParams::default()
        };
        assert_eq!(
            waterfalls(&h, [5.0, 5.0], Some(&rivers), None, &p)
                .expect("falls")
                .falls
                .len(),
            1
        );
    }

    #[test]
    fn without_a_mask_the_rivers_come_from_the_flow_and_bad_settings_are_refused() {
        let (h, _) = valley(20.0);
        let a = waterfalls(&h, [5.0, 5.0], None, None, &FallsParams::default()).expect("falls");
        assert_eq!(
            a.falls.len(),
            1,
            "the valley's own flow finds the fall: {:?}",
            a.falls
        );
        let b = waterfalls(&h, [5.0, 5.0], None, None, &FallsParams::default()).expect("again");
        assert_eq!(a.falls, b.falls, "deterministic");
        assert_eq!(a.height.to_vec(), b.height.to_vec());
        for bad in [
            FallsParams {
                min_drop_m: 0.0,
                ..FallsParams::default()
            },
            FallsParams {
                min_gradient: f32::NAN,
                ..FallsParams::default()
            },
            FallsParams {
                pool_depth: -1.0,
                ..FallsParams::default()
            },
            FallsParams {
                threshold: 1.0,
                ..FallsParams::default()
            },
        ] {
            assert!(
                waterfalls(&h, [5.0, 5.0], None, None, &bad).is_err(),
                "{bad:?}"
            );
        }
        let small = Heightfield::new(8, 8, 8, 0).expect("hf");
        assert!(waterfalls(&h, [5.0, 5.0], Some(&small), None, &FallsParams::default()).is_err());
    }
}
