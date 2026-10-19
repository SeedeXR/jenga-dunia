//! Geomorphic kernels (ADR-0006 order: thermal first). CPU, deterministic, whole-field Jacobi
//! passes through [`Heightfield::fill_with`]; every pass reads the previous field only, so the
//! result is independent of thread count.
//!
//! ponytail: two whole-field passes per iteration over flat rows (8 B/cell scratch, ~130 MB at
//! 4096²); tile-local passes with halo exchange and the WGSL twin land with `jd-gpu` / tile-level
//! dirty sets.

use crate::Heightfield;
use jd_types::{JdError, JdResult};
use rayon::prelude::*;

const SQRT2: f32 = std::f32::consts::SQRT_2;
/// 8-neighbourhood offsets and their distances in pixels.
const NEIGHBOURS: [(i64, i64, f32); 8] = [
    (-1, -1, SQRT2),
    (0, -1, 1.0),
    (1, -1, SQRT2),
    (-1, 0, 1.0),
    (1, 0, 1.0),
    (-1, 1, SQRT2),
    (0, 1, 1.0),
    (1, 1, SQRT2),
];

/// Flat row-major view used inside the kernels (tile `get` per sample is too slow for Jacobi loops).
struct Grid<'a> {
    h: &'a [f32],
    w: i64,
    hh: i64,
}

impl Grid<'_> {
    #[inline]
    fn at(&self, x: i64, y: i64) -> f32 {
        self.h[(y * self.w + x) as usize]
    }
    #[inline]
    fn inside(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.hh
    }
}

/// Pass 1 of a thermal step (Olsen 2004 talus rule): per cell, the material it sheds this step
/// (`amount = rate · dmax / 2`) and `amount / total_excess`, the factor that turns a neighbour's
/// excess into its share. Both are 0 when nothing is steeper than the talus.
fn shed_factors(g: &Grid, talus_h: f32, rate: f32) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0f32; 2]; (g.w * g.hh) as usize];
    out.par_chunks_mut(g.w as usize)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, cell) in row.iter_mut().enumerate() {
                let (xi, yi) = (x as i64, y as i64);
                let hc = g.at(xi, yi);
                let (mut total, mut dmax) = (0.0f32, 0.0f32);
                for (dx, dy, dist) in NEIGHBOURS {
                    let (nx, ny) = (xi + dx, yi + dy);
                    if !g.inside(nx, ny) {
                        continue; // material never leaves the field: exact conservation
                    }
                    let d = hc - g.at(nx, ny) - talus_h * dist;
                    if d > 0.0 {
                        total += d;
                        dmax = dmax.max(d);
                    }
                }
                if total > 0.0 {
                    let amount = rate * dmax * 0.5;
                    *cell = [amount, amount / total];
                }
            }
        });
    out
}

/// Pass 2: every cell loses what it sheds and gains each neighbour's share directed at it.
fn gather(g: &Grid, shed: &[[f32; 2]], talus_h: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; (g.w * g.hh) as usize];
    out.par_chunks_mut(g.w as usize)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, cell) in row.iter_mut().enumerate() {
                let (xi, yi) = (x as i64, y as i64);
                let hc = g.at(xi, yi);
                let mut gained = 0.0f32;
                for (dx, dy, dist) in NEIGHBOURS {
                    let (nx, ny) = (xi + dx, yi + dy);
                    if !g.inside(nx, ny) {
                        continue;
                    }
                    let factor = shed[(ny * g.w + nx) as usize][1];
                    if factor > 0.0 {
                        let excess = g.at(nx, ny) - hc - talus_h * dist;
                        if excess > 0.0 {
                            gained += factor * excess;
                        }
                    }
                }
                *cell = hc - shed[(yi * g.w + xi) as usize][0] + gained;
            }
        });
    out
}

/// Thermal erosion: material steeper than `talus_deg` slides downhill for `iterations` steps.
/// `pixel_m` is the horizontal sample spacing; `rate` in (0, 1] scales how much of the excess
/// moves per step (0.5 is the classic stable choice). Mass is conserved exactly up to float
/// rounding and the result is independent of thread count.
pub fn thermal_erode(
    h: &Heightfield,
    pixel_m: f32,
    talus_deg: f32,
    iterations: u32,
    rate: f32,
) -> JdResult<Heightfield> {
    if !(0.0..90.0).contains(&talus_deg) {
        return Err(JdError::invalid("talus_deg", "must be within 0..90"));
    }
    if !(0.0 < rate && rate <= 1.0) {
        return Err(JdError::invalid("rate", "must be within (0, 1]"));
    }
    if !(pixel_m > 0.0 && pixel_m.is_finite()) {
        return Err(JdError::invalid("pixel_m", "must be positive"));
    }
    let talus_h = talus_deg.to_radians().tan() * pixel_m;
    let (w, hh) = (h.width() as i64, h.height() as i64);
    let mut cur = h.to_vec();
    for _ in 0..iterations {
        let g = Grid { h: &cur, w, hh };
        let shed = shed_factors(&g, talus_h, rate);
        cur = gather(&g, &shed, talus_h);
    }
    let mut out = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    out.fill_with(|x, y| cur[(y as usize) * (w as usize) + x as usize]);
    Ok(out)
}

/// Slope mask in 0..1: central-difference gradient angle divided by `max_deg`, clamped.
pub fn slope_mask(h: &Heightfield, pixel_m: f32, max_deg: f32) -> JdResult<Heightfield> {
    if !(0.0 < max_deg && max_deg <= 90.0) {
        return Err(JdError::invalid("max_deg", "must be within (0, 90]"));
    }
    if !(pixel_m > 0.0 && pixel_m.is_finite()) {
        return Err(JdError::invalid("pixel_m", "must be positive"));
    }
    let (w, hh) = (h.width(), h.height());
    let mut out = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    out.fill_with(|x, y| {
        let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
        let (y0, y1) = (y.saturating_sub(1), (y + 1).min(hh - 1));
        let dx = (h.get(x1, y) - h.get(x0, y)) / ((x1 - x0) as f32 * pixel_m);
        let dy = (h.get(x, y1) - h.get(x, y0)) / ((y1 - y0) as f32 * pixel_m);
        ((dx * dx + dy * dy).sqrt().atan().to_degrees() / max_deg).clamp(0.0, 1.0)
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum(h: &Heightfield) -> f64 {
        h.to_vec().iter().map(|&v| v as f64).sum()
    }

    #[test]
    fn thermal_conserves_mass_and_flattens_a_spike() {
        let mut h = Heightfield::new(64, 48, 32, 4).expect("hf");
        h.fill_with(|x, y| if (x, y) == (30, 20) { 500.0 } else { 0.0 });
        let before = sum(&h);
        let after = thermal_erode(&h, 10.0, 35.0, 40, 0.5).expect("erode");
        assert!(
            (sum(&after) - before).abs() < 1e-2 * before.abs().max(1.0),
            "mass drifted: {before} → {}",
            sum(&after)
        );
        assert!(
            after.get(30, 20) < 500.0 * 0.5,
            "spike should spread: {}",
            after.get(30, 20)
        );
        assert!(
            after.get(31, 20) > 0.0 && after.get(29, 19) > 0.0,
            "material reaches neighbours"
        );
        assert_eq!(after.stats().non_finite, 0);
        // Deterministic: same input, same output.
        assert_eq!(
            thermal_erode(&h, 10.0, 35.0, 40, 0.5)
                .expect("again")
                .content_hash(),
            after.content_hash()
        );
    }

    #[test]
    fn thermal_leaves_gentle_terrain_alone_and_rejects_bad_params() {
        let mut h = Heightfield::new(40, 40, 32, 4).expect("hf");
        h.fill_with(|x, _| x as f32 * 0.1); // 0.1 m rise per 10 m pixel ≈ 0.6°, far below talus
        let after = thermal_erode(&h, 10.0, 35.0, 5, 0.5).expect("erode");
        assert_eq!(after.content_hash(), h.content_hash());
        assert!(thermal_erode(&h, 10.0, 95.0, 1, 0.5).is_err());
        assert!(thermal_erode(&h, 10.0, 35.0, 1, 0.0).is_err());
        assert!(thermal_erode(&h, 0.0, 35.0, 1, 0.5).is_err());
    }

    #[test]
    fn slope_of_a_plane_is_constant() {
        let mut h = Heightfield::new(50, 30, 32, 4).expect("hf");
        h.fill_with(|x, _| x as f32 * 10.0); // rise 10 m per 10 m pixel = 45°
        let m = slope_mask(&h, 10.0, 90.0).expect("slope");
        for (x, y) in [(1u32, 1u32), (25, 15), (48, 28)] {
            assert!(
                (m.get(x, y) - 0.5).abs() < 1e-4,
                "({x},{y}) = {}",
                m.get(x, y)
            );
        }
        let flat = Heightfield::new(8, 8, 8, 0).expect("hf");
        assert_eq!(slope_mask(&flat, 1.0, 45.0).expect("flat").stats().max, 0.0);
        assert!(slope_mask(&h, 10.0, 0.0).is_err());
    }
}
