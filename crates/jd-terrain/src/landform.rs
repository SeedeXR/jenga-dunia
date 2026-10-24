//! Landform measures: geomorphons (Jasiewicz & Stepinski 2013, *Geomorphology* 182 147–156) and
//! a terrain fingerprint — relief, hypsometric integral, slope distribution, geomorphon shares —
//! to compare a generated world with a real DEM of the same landform
//! (`docs/research/image-reference-world-design.md`).
//!
//! Geomorphons: from each cell, look along the 8 compass directions up to `search_m`; the
//! highest line of sight (zenith elevation) and the lowest one (nadir elevation) decide whether
//! that direction is higher (their sum above `flat_deg`), lower (below −`flat_deg`) or flat. The counts of + and −
//! pick one of ten forms from the lookup table of the paper as implemented in WhiteboxTools
//! (MIT, `geomorphons.rs`): 1 flat, 2 peak, 3 ridge, 4 shoulder, 5 spur, 6 slope, 7 hollow,
//! 8 footslope, 9 valley, 10 pit; 0 on the border ring.

use jd_types::{JdError, JdResult};
use rayon::prelude::*;
use serde::Serialize;

use crate::Heightfield;

/// Names of geomorphon classes 1..=10.
pub const GEOMORPHON_NAMES: [&str; 10] = [
    "flat",
    "peak",
    "ridge",
    "shoulder",
    "spur",
    "slope",
    "hollow",
    "footslope",
    "valley",
    "pit",
];

/// `CLASSES[count_lower][count_higher]` (WhiteboxTools' table; 0 = impossible).
const CLASSES: [[u8; 9]; 9] = [
    [1, 1, 1, 8, 8, 9, 9, 9, 10],
    [1, 1, 8, 8, 8, 9, 9, 9, 0],
    [1, 4, 6, 6, 7, 7, 9, 0, 0],
    [4, 4, 6, 6, 6, 7, 0, 0, 0],
    [4, 4, 5, 6, 6, 0, 0, 0, 0],
    [3, 3, 5, 5, 0, 0, 0, 0, 0],
    [3, 3, 3, 0, 0, 0, 0, 0, 0],
    [3, 3, 0, 0, 0, 0, 0, 0, 0],
    [2, 0, 0, 0, 0, 0, 0, 0, 0],
];

const DIRS: [(i64, i64); 8] = [
    (0, 1),
    (1, 1),
    (1, 0),
    (1, -1),
    (0, -1),
    (-1, -1),
    (-1, 0),
    (-1, 1),
];

/// Geomorphon class (1..=10, 0 on the border) of every cell.
pub fn geomorphons(
    h: &Heightfield,
    spacing_m: [f32; 2],
    search_m: f32,
    flat_deg: f32,
) -> JdResult<Heightfield> {
    if !(search_m > 0.0 && search_m.is_finite()) {
        return Err(JdError::invalid("search_m", "must be positive"));
    }
    if !(0.0..45.0).contains(&flat_deg) {
        return Err(JdError::invalid("flat_deg", "must be within [0, 45)"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    if search_m < spacing_m[0].max(spacing_m[1]) {
        return Err(
            JdError::invalid("search_m", "must reach at least one neighbouring cell").why(
                "A search shorter than a cell sees nothing and classifies everything as flat.",
            ),
        );
    }
    let (w, hh) = (h.width() as i64, h.height() as i64);
    let z = h.to_vec();
    let flat = flat_deg.to_radians();
    let mut out = vec![0f32; z.len()];
    out.par_chunks_mut(w as usize)
        .enumerate()
        .for_each(|(y, row)| {
            let y = y as i64;
            if y == 0 || y == hh - 1 {
                return;
            }
            for x in 1..w - 1 {
                let z0 = z[(y * w + x) as usize];
                let (mut higher, mut lower) = (0usize, 0usize);
                for (dx, dy) in DIRS {
                    let step = (dx as f32 * spacing_m[0]).hypot(dy as f32 * spacing_m[1]);
                    let (mut zen, mut nad) =
                        (-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
                    let mut k = 1i64;
                    while (k as f32) * step <= search_m {
                        let (nx, ny) = (x + dx * k, y + dy * k);
                        if nx < 0 || ny < 0 || nx >= w || ny >= hh {
                            break;
                        }
                        let a = (z[(ny * w + nx) as usize] - z0).atan2(k as f32 * step);
                        zen = zen.max(a);
                        nad = nad.min(a);
                        k += 1;
                    }
                    if k == 1 {
                        continue; // nothing in reach that way: flat
                    }
                    // Jasiewicz & Stepinski: the nadir angle minus the zenith angle, which in
                    // elevation terms is zenith + nadir; within ±flat the direction is flat.
                    let d = zen + nad;
                    if d > flat {
                        higher += 1;
                    } else if d < -flat {
                        lower += 1;
                    }
                }
                row[x as usize] = CLASSES[lower][higher] as f32;
            }
        });
    let mut o = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    o.fill_with(|x, y| out[(y * h.width() + x) as usize]);
    Ok(o)
}

/// A terrain's fingerprint.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Fingerprint {
    /// Sample spacing, metres.
    pub cell_m: [f32; 2],
    /// Highest minus lowest, metres.
    pub relief_m: f64,
    /// (mean − min) / (max − min): ~0.2–0.4 for old, eroded uplands, ~0.6+ for young plateaus
    /// (Strahler's hypsometric integral via Pike & Wilson's elevation–relief ratio).
    pub hypsometric_integral: f64,
    /// Slope statistics, degrees.
    pub slope: SlopeStats,
    /// Share of interior cells in each geomorphon class.
    pub geomorphons: std::collections::BTreeMap<String, f64>,
    /// The `search_m` the geomorphons used.
    pub search_m: f32,
}

/// Slope statistics.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SlopeStats {
    /// Mean slope, degrees.
    pub mean_deg: f64,
    /// Median, degrees.
    pub p50_deg: f64,
    /// 90th percentile, degrees.
    pub p90_deg: f64,
    /// Share of cells per 5° band: 0–5, 5–10, …, 85–90.
    pub histogram_5deg: Vec<f64>,
}

/// Measure `h`.
pub fn fingerprint(
    h: &Heightfield,
    spacing_m: [f32; 2],
    search_m: f32,
    flat_deg: f32,
) -> JdResult<Fingerprint> {
    let z = h.to_vec();
    let (w, hh) = (h.width() as usize, h.height() as usize);
    if w < 3 || hh < 3 {
        return Err(JdError::invalid("layer", "needs at least 3 × 3 samples"));
    }
    let (lo, hi, sum) = z
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY, 0.0), |(lo, hi, s), v| {
            (lo.min(*v as f64), hi.max(*v as f64), s + *v as f64)
        });
    let relief = hi - lo;
    let hi_int = if relief > 0.0 {
        (sum / z.len() as f64 - lo) / relief
    } else {
        0.0
    };
    let mut slopes: Vec<f32> = (1..hh - 1)
        .flat_map(|y| (1..w - 1).map(move |x| (x, y)))
        .map(|(x, y)| {
            let gx = (z[y * w + x + 1] - z[y * w + x - 1]) / (2.0 * spacing_m[0]);
            let gy = (z[(y + 1) * w + x] - z[(y - 1) * w + x]) / (2.0 * spacing_m[1]);
            gx.hypot(gy).atan().to_degrees()
        })
        .collect();
    slopes.sort_by(f32::total_cmp);
    let n = slopes.len() as f64;
    let mut hist = vec![0f64; 18];
    for s in &slopes {
        hist[((*s / 5.0) as usize).min(17)] += 1.0 / n;
    }
    let pct = |p: f64| slopes[((p * (n - 1.0)).round() as usize).min(slopes.len() - 1)] as f64;
    let g = geomorphons(h, spacing_m, search_m, flat_deg)?.to_vec();
    let interior = ((w - 2) * (hh - 2)) as f64;
    let mut shares = std::collections::BTreeMap::new();
    for (i, name) in GEOMORPHON_NAMES.iter().enumerate() {
        let c = g.iter().filter(|v| **v as usize == i + 1).count() as f64;
        shares.insert((*name).to_owned(), c / interior);
    }
    Ok(Fingerprint {
        cell_m: spacing_m,
        relief_m: relief,
        hypsometric_integral: hi_int,
        slope: SlopeStats {
            mean_deg: slopes.iter().map(|s| *s as f64).sum::<f64>() / n,
            p50_deg: pct(0.5),
            p90_deg: pct(0.9),
            histogram_5deg: hist,
        },
        geomorphons: shares,
        search_m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(f: impl Fn(f32, f32) -> f32 + Sync) -> Heightfield {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| f(x as f32 - 32.0, y as f32 - 32.0));
        h
    }

    #[test]
    fn cones_pits_ridges_valleys_planes_and_flats_classify_as_such() {
        let at = |h: &Heightfield| {
            geomorphons(h, [10.0, 10.0], 100.0, 1.0)
                .expect("g")
                .get(32, 32) as usize
        };
        assert_eq!(
            GEOMORPHON_NAMES[at(&field(|x, y| 100.0 - x.hypot(y) * 3.0)) - 1],
            "peak"
        );
        assert_eq!(
            GEOMORPHON_NAMES[at(&field(|x, y| x.hypot(y) * 3.0)) - 1],
            "pit"
        );
        assert_eq!(
            GEOMORPHON_NAMES[at(&field(|x, _| 100.0 - x.abs() * 3.0)) - 1],
            "ridge"
        );
        assert_eq!(
            GEOMORPHON_NAMES[at(&field(|x, _| x.abs() * 3.0)) - 1],
            "valley"
        );
        assert_eq!(
            GEOMORPHON_NAMES[at(&field(|x, y| x * 3.0 + y)) - 1],
            "slope"
        );
        assert_eq!(GEOMORPHON_NAMES[at(&field(|_, _| 5.0)) - 1], "flat");
        // The border ring is unclassified; bad params are refused.
        assert_eq!(
            geomorphons(&field(|_, _| 0.0), [10.0, 10.0], 100.0, 1.0)
                .expect("g")
                .get(0, 5),
            0.0
        );
        assert!(geomorphons(&field(|_, _| 0.0), [10.0, 10.0], 0.0, 1.0).is_err());
        assert!(
            geomorphons(&field(|_, _| 0.0), [10.0, 10.0], 5.0, 1.0).is_err(),
            "shorter than a cell"
        );
        // A 0.5° tilt stays flat inside a 1° band (the paper's |nadir − zenith| ≤ t rule), not "slope".
        let tilt = field(|x, _| x * 10.0 * 0.5f32.to_radians().tan());
        assert_eq!(GEOMORPHON_NAMES[at(&tilt) - 1], "flat");
    }

    #[test]
    fn fingerprint_of_a_cone() {
        // A 45° cone: slope 45° almost everywhere inside it, HI of a cone = 1/3 of its relief above the floor.
        let h = field(|x, y| (300.0 - x.hypot(y) * 10.0).max(0.0));
        let f = fingerprint(&h, [10.0, 10.0], 100.0, 1.0).expect("fp");
        assert!((f.relief_m - 300.0).abs() < 1e-3);
        assert!(f.slope.histogram_5deg.iter().sum::<f64>() > 0.999);
        assert!(
            f.slope.p90_deg > 40.0 && f.slope.p90_deg < 50.0,
            "{:?}",
            f.slope
        );
        assert!(f.geomorphons["slope"] > f.geomorphons["peak"]);
        assert!(
            (f.geomorphons.values().sum::<f64>() - 1.0).abs() < 1e-9,
            "every interior cell has a class"
        );
    }
}
