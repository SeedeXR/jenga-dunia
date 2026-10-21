//! Bathymetry: sediment infill of the sea floor. Sand and mud settle into hollows and level the
//! shelf; rock and reef keep the shape erosion gave them. The seabed class map decides which is
//! which (`classify_seabed`), so the node composes with the rest of the coast chain:
//! coastal → seabed → bathymetry.
//!
//! Rule per sediment cell (class 1 sand or 3 mud): raise the floor towards the local mean height
//! over `smooth_m` (a box filter, so an O(N) summed-area table), never lower it, never by more
//! than `max_fill_m`, and never above `level_m` — sediment fills hollows, it does not build
//! mountains. Everything else (land, rock, reef) is returned untouched.
//!
//! Transport and budget, when a `supply` mask is wired (the coastal node's `shore`, a river
//! mask, or their sum): sediment comes from those cells and thins away from them — the infill is
//! scaled by exp(−distance / `reach_m`), distance measured through water from the nearest
//! supply cell — and `budget_m3` caps the total volume laid down (every fill scaled by the same
//! factor once the sum exceeds it). Without `supply` every sediment floor fills alike.
//!
//! ponytail: a static, distance-weighted infill — no sorting by grain size (the seabed classifier
//! already separates sand from mud by depth) and no down-slope creep.

use std::sync::atomic::AtomicBool;

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::sculpt::box_mean;
use crate::sim::check_cancel;

/// Bathymetry settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BathymetryParams {
    /// Sea level, metres: sediment never builds above it.
    #[serde(default)]
    pub level_m: f32,
    /// Smoothing reach, metres (> 0): hollows narrower than this fill in.
    #[serde(default = "d_smooth")]
    pub smooth_m: f32,
    /// Thickest sediment layer laid down anywhere, metres (≥ 0).
    #[serde(default = "d_fill")]
    pub max_fill_m: f32,
    /// How much of the infill to apply, 0..1 (1 = level the hollow completely).
    #[serde(default = "d_strength")]
    pub strength: f32,
    /// With a `supply` mask: how far sediment travels from its source, metres (> 0) — the
    /// infill fades by e⁻¹ per `reach_m` of water between a cell and the nearest supply cell.
    #[serde(default = "d_reach")]
    pub reach_m: f32,
    /// Total sediment available, cubic metres (≥ 0): when the infill would lay down more, every
    /// cell's fill is scaled back by the same factor. Omit for no limit.
    #[serde(default)]
    pub budget_m3: Option<f32>,
}

fn d_smooth() -> f32 {
    200.0
}
fn d_fill() -> f32 {
    20.0
}
fn d_strength() -> f32 {
    1.0
}
fn d_reach() -> f32 {
    2000.0
}

impl Default for BathymetryParams {
    fn default() -> Self {
        Self {
            level_m: 0.0,
            smooth_m: d_smooth(),
            max_fill_m: d_fill(),
            strength: d_strength(),
            reach_m: d_reach(),
            budget_m3: None,
        }
    }
}

/// Fill the sediment floors of `h` (cells whose `seabed` class is 1 sand or 3 mud).
pub fn bathymetry(
    h: &Heightfield,
    spacing_m: [f32; 2],
    seabed: &Heightfield,
    p: &BathymetryParams,
) -> JdResult<Heightfield> {
    bathymetry_with(h, spacing_m, seabed, None, p, None)
}

/// [`bathymetry`] with an optional `supply` mask (any cell above 0 is a sediment source — the
/// coastal `shore` mask, whose wave work is normalised to the busiest cell, a river mask): the fill fades with the water distance from the nearest source
/// (`reach_m`), and `budget_m3` caps the volume laid down. A wired mask with no source cell is
/// an error (it would silently deposit nothing). Supply cells on land — a river channel above
/// its mouth — carry the distance along themselves into the water.
pub fn bathymetry_with(
    h: &Heightfield,
    spacing_m: [f32; 2],
    seabed: &Heightfield,
    supply: Option<&Heightfield>,
    p: &BathymetryParams,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    if !(p.smooth_m > 0.0 && p.smooth_m.is_finite()) {
        return Err(JdError::invalid("smooth_m", "must be positive"));
    }
    if !(p.max_fill_m >= 0.0 && p.max_fill_m.is_finite()) {
        return Err(JdError::invalid("max_fill_m", "must be ≥ 0"));
    }
    if !(0.0..=1.0).contains(&p.strength) {
        return Err(JdError::invalid("strength", "must be within 0..=1"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    if (seabed.width(), seabed.height()) != (h.width(), h.height()) {
        return Err(JdError::invalid("seabed", "field size differs from `in`"));
    }
    if !(p.reach_m > 0.0 && p.reach_m.is_finite()) {
        return Err(JdError::invalid("reach_m", "must be positive"));
    }
    if p.budget_m3.is_some_and(|b| !(b >= 0.0 && b.is_finite())) {
        return Err(JdError::invalid("budget_m3", "must be finite and ≥ 0"));
    }
    if let Some(s) = supply
        && (s.width(), s.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid("supply", "field size differs from `in`"));
    }
    let (w, hh) = (h.width() as usize, h.height() as usize);
    if w * hh > u32::MAX as usize {
        return Err(JdError::invalid("in", "field has more than 2³² cells"));
    }
    let radius = |s: f32| ((p.smooth_m / s).round() as u32).clamp(1, 4096);
    let mean = box_mean(h, radius(spacing_m[0]), radius(spacing_m[1]));
    // Supply: water distance (8-neighbour Dijkstra over the sea floor and the supply cells
    // themselves) from every source cell, stopped where the weight has vanished.
    let weight: Option<Vec<f32>> = match supply {
        None => None,
        Some(s) => {
            use std::cmp::Reverse;
            use std::collections::BinaryHeap;
            let (src, classes) = (s.to_vec(), seabed.to_vec());
            let mut dist = vec![f32::INFINITY; w * hh];
            let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new(); // (metres as bits, cell)
            for (i, v) in src.iter().enumerate() {
                if *v > 0.0 {
                    dist[i] = 0.0;
                    heap.push(Reverse((0, i as u32)));
                }
            }
            if heap.is_empty() {
                return Err(JdError::invalid(
                    "supply",
                    "no cell above 0: nothing would be deposited",
                )
                .next(
                    "Unwire `supply` to fill every sediment floor, or feed a mask with sources.",
                ));
            }
            let far = 10.0 * p.reach_m; // e⁻¹⁰: below any visible fill
            let mut pops = 0usize;
            while let Some(Reverse((d, i))) = heap.pop() {
                let (i, d) = (i as usize, f32::from_bits(d));
                if d > dist[i] || d > far {
                    continue;
                }
                pops += 1;
                if pops.is_multiple_of(65_536) {
                    check_cancel(cancel)?;
                }
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                for oy in -1i64..=1 {
                    for ox in -1i64..=1 {
                        let (nx, ny) = (x + ox, y + oy);
                        if (ox, oy) == (0, 0)
                            || nx < 0
                            || ny < 0
                            || nx >= w as i64
                            || ny >= hh as i64
                        {
                            continue;
                        }
                        let j = ny as usize * w + nx as usize;
                        if classes[j].round() == 0.0 && src[j] <= 0.0 {
                            continue; // land does not carry sediment (a supply channel does)
                        }
                        let nd = d
                            + ((ox as f32 * spacing_m[0]).powi(2)
                                + (oy as f32 * spacing_m[1]).powi(2))
                            .sqrt();
                        if nd < dist[j] {
                            dist[j] = nd;
                            heap.push(Reverse((nd.to_bits(), j as u32)));
                        }
                    }
                }
            }
            Some(dist.iter().map(|d| (-d / p.reach_m).exp()).collect()) // unreached → exp(−∞) = 0
        }
    };
    let mut fill = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    fill.fill_with(|x, y| {
        let z = h.get(x, y);
        let class = seabed.get(x, y).round();
        if class != 1.0 && class != 3.0 {
            return 0.0;
        }
        let f = (mean.get(x, y) - z).clamp(0.0, p.max_fill_m)
            * p.strength
            * weight
                .as_ref()
                .map_or(1.0, |wt| wt[y as usize * w + x as usize]);
        (z + f).min(p.level_m.max(z)) - z
    });
    let scale = match p.budget_m3 {
        Some(budget) => {
            let total = fill.to_vec().iter().map(|f| *f as f64).sum::<f64>()
                * (spacing_m[0] * spacing_m[1]) as f64;
            if total > budget as f64 {
                (budget as f64 / total) as f32
            } else {
                1.0
            }
        }
        None => 1.0,
    };
    let mut out = h.clone();
    out.fill_with(|x, y| h.get(x, y) + fill.get(x, y) * scale);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supply_thins_the_fill_away_from_the_shore_and_the_budget_caps_the_volume() {
        // A −100 m mud basin pocked with 10 m hollows; the shore (supply) is the west column.
        let mut h = Heightfield::new(64, 16, 64, 0).expect("hf");
        h.fill_with(|x, y| if (x + y) % 2 == 0 { -100.0 } else { -110.0 });
        let mut classes = Heightfield::new(64, 16, 64, 0).expect("c");
        classes.fill_with(|_, _| 3.0);
        let mut supply = Heightfield::new(64, 16, 64, 0).expect("s");
        supply.fill_with(|x, _| if x == 0 { 1.0 } else { 0.0 });
        let p = BathymetryParams {
            smooth_m: 30.0,
            reach_m: 100.0, // 10 cells of 10 m
            ..BathymetryParams::default()
        };
        let plain = bathymetry(&h, [10.0, 10.0], &classes, &p).expect("plain");
        let fed =
            bathymetry_with(&h, [10.0, 10.0], &classes, Some(&supply), &p, None).expect("fed");
        let lift = |o: &Heightfield, x: u32| o.get(x, 7) - h.get(x, 7); // row 7, x even = a hollow
        assert!(
            lift(&plain, 4) > 4.0 && (lift(&plain, 4) - lift(&plain, 50)).abs() < 1e-3,
            "no supply: every hollow fills alike"
        );
        assert!(
            lift(&fed, 4) > 0.6 * lift(&plain, 4),
            "near the shore the hollow fills: {}",
            lift(&fed, 4)
        );
        let one_reach = lift(&fed, 10) / lift(&plain, 10); // 100 m from the shore column
        assert!(
            (one_reach - (-1.0f32).exp()).abs() < 0.05,
            "one reach out: e⁻¹, got {one_reach}"
        );
        assert!(
            lift(&fed, 22) < 0.2 * lift(&fed, 4),
            "two reaches out it barely does: {}",
            lift(&fed, 22)
        );
        assert!(
            lift(&fed, 50) < 0.01 * lift(&plain, 4),
            "five reaches out nothing arrives: {}",
            lift(&fed, 50)
        );
        // Budget: half the unlimited volume → the same pattern at half the height.
        let volume = |o: &Heightfield| {
            (0..16)
                .flat_map(|y| (0..64).map(move |x| (x, y)))
                .map(|(x, y)| (o.get(x, y) - h.get(x, y)) * 100.0)
                .sum::<f32>()
        };
        let half = volume(&plain) / 2.0;
        let capped = bathymetry(
            &h,
            [10.0, 10.0],
            &classes,
            &BathymetryParams {
                budget_m3: Some(half),
                ..p.clone()
            },
        )
        .expect("capped");
        assert!(
            (volume(&capped) - half).abs() < 1e-2 * half,
            "{} vs {half}",
            volume(&capped)
        );
        assert!((lift(&capped, 4) - lift(&plain, 4) / 2.0).abs() < 1e-3);
        assert!(
            bathymetry(
                &h,
                [10.0, 10.0],
                &classes,
                &BathymetryParams {
                    reach_m: 0.0,
                    ..p.clone()
                }
            )
            .is_err()
        );
        // A wired supply with no source is an error, not a silent no-op; a supply channel on land
        // (class 0) still reaches the water through itself.
        let empty = Heightfield::new(64, 16, 64, 0).expect("e");
        assert!(bathymetry_with(&h, [10.0, 10.0], &classes, Some(&empty), &p, None).is_err());
        let mut land_classes = classes.clone();
        land_classes.fill_with(|x, y| if x < 4 && y != 7 { 0.0 } else { 3.0 }); // a land block with a channel along row 7
        let mut channel = Heightfield::new(64, 16, 64, 0).expect("ch");
        channel.fill_with(|x, y| if x == 0 && y == 7 { 1.0 } else { 0.0 }); // the source sits at the channel head, on land
        let via = bathymetry_with(&h, [10.0, 10.0], &land_classes, Some(&channel), &p, None)
            .expect("channel");
        assert!(
            via.get(6, 7) - h.get(6, 7) > 1.0,
            "sediment came down the channel: {}",
            via.get(6, 7) - h.get(6, 7)
        );
        assert!(
            bathymetry(
                &h,
                [10.0, 10.0],
                &classes,
                &BathymetryParams {
                    budget_m3: Some(-1.0),
                    ..p
                }
            )
            .is_err()
        );
    }

    #[test]
    fn hollows_fill_on_sediment_floors_only_and_never_above_the_sea() {
        // A −100 m basin pocked with 10 m hollows every other cell; the east half is rock.
        let mut h = Heightfield::new(32, 32, 32, 0).expect("hf");
        h.fill_with(|x, y| match (x, y) {
            (3, 3) => -0.5, // a shallow cell right under the surface
            _ if (x + y) % 2 == 0 => -100.0,
            _ => -110.0,
        });
        let mut classes = Heightfield::new(32, 32, 32, 0).expect("c");
        classes.fill_with(|x, _| if x < 16 { 3.0 } else { 2.0 });
        let p = BathymetryParams {
            smooth_m: 30.0,
            ..BathymetryParams::default()
        };
        let out = bathymetry(&h, [10.0, 10.0], &classes, &p).expect("bathymetry");
        let (mut filled, mut lowered) = (0, 0);
        for y in 0..32 {
            for x in 0..32 {
                let (a, b) = (h.get(x, y), out.get(x, y));
                if x >= 16 {
                    assert_eq!(a, b, "rock is untouched at ({x}, {y})");
                }
                assert!(b <= 0.0 || b == a, "nothing builds above sea level");
                assert!(b - a <= p.max_fill_m + 1e-4, "fill capped");
                if b > a + 1e-4 {
                    filled += 1;
                }
                if b < a - 1e-4 {
                    lowered += 1;
                }
            }
        }
        assert_eq!(lowered, 0, "sediment never cuts");
        assert!(filled > 100, "the mud hollows fill ({filled})");
        // A mud hollow (−110) rises to the local mean (≈ −105); the hump beside it is not cut.
        assert!(
            (out.get(10, 11) + 105.0).abs() < 0.6,
            "hollow {}",
            out.get(10, 11)
        );
        assert_eq!(out.get(11, 11), -100.0);
        assert!(
            bathymetry(
                &h,
                [10.0, 10.0],
                &classes,
                &BathymetryParams {
                    strength: 2.0,
                    ..p.clone()
                }
            )
            .is_err()
        );
        assert!(
            bathymetry(
                &h,
                [10.0, 10.0],
                &classes,
                &BathymetryParams { smooth_m: 0.0, ..p }
            )
            .is_err()
        );
    }
}
