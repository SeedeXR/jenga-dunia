//! Wind-sea spectrum: what waves a wind of a given speed raises over a given fetch, as the numbers
//! a Gerstner surface needs (`docs/research/algorithms.md`, "Water surface": Gerstner in the
//! viewport, FFT ocean later).
//!
//! Deep-water, fetch-limited growth after JONSWAP (Hasselmann et al. 1973) in its U10 form, capped
//! at the fully developed Pierson–Moskowitz (1964) sea: with χ = gF/U² the significant height is
//! Hs = 1.6·10⁻³ √χ U²/g and the peak period Tp = 0.2857 χ^⅓ U/g; a fully developed sea reaches
//! Hs ≈ 0.0246 U² and Tp ≈ 0.78 U (U in m/s, U10 ≈ U19.5/1.07). Wavelength from the deep-water
//! dispersion relation L = g Tp²/2π. These are the textbook forecasting curves (SPM 1984 without
//! its wind-stress factor, which over-predicts strong winds); a 20 m/s wind over 100 km gives
//! Hs 3.2 m / Tp 7.9 s, fully developed 9.8 m / 15.6 s.
//!
//! ponytail: three Gerstner components (peak, half, fifth wavelength) spread ±30° about the wind;
//! a seeded directional spectrum with dozens of components is the upgrade when the surface needs
//! to look less regular.

use crate::Heightfield;
use crate::sim::check_cancel;
use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

const G: f32 = 9.81;

/// Wind-sea settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaveParams {
    /// Wind speed 10 m above the sea, m/s (≥ 0; 0 = flat calm): 3 ripples, 8 moderate sea, 20 gale.
    #[serde(default = "d_wind")]
    pub wind_speed_mps: f32,
    /// Fetch, km: how far the wind has blown over open water (> 0). Omit for a fully developed sea.
    #[serde(default)]
    pub fetch_km: Option<f32>,
    /// Where the wind blows FROM, degrees clockwise from north; waves travel the opposite way.
    #[serde(default = "d_azimuth")]
    pub wind_azimuth_deg: f32,
}

fn d_wind() -> f32 {
    8.0
}
fn d_azimuth() -> f32 {
    270.0
}

impl Default for WaveParams {
    fn default() -> Self {
        Self {
            wind_speed_mps: d_wind(),
            fetch_km: None,
            wind_azimuth_deg: d_azimuth(),
        }
    }
}

/// One trochoidal (Gerstner) wave.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WaveComponent {
    /// Crest height above the mean surface, m.
    pub amplitude_m: f32,
    /// Crest-to-crest distance, m.
    pub wavelength_m: f32,
    /// Travel direction, degrees clockwise from north.
    pub direction_deg: f32,
    /// Deep-water phase speed √(g L / 2π), m/s.
    pub speed_mps: f32,
    /// Phase offset, radians.
    pub phase: f32,
}

/// The wind sea: bulk statistics plus the Gerstner components that draw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WaveSpectrum {
    /// Significant wave height Hs (mean of the highest third), m.
    pub significant_height_m: f32,
    /// Peak period Tp, s.
    pub peak_period_s: f32,
    /// Peak wavelength, m.
    pub wavelength_m: f32,
    /// Whether the fetch was long enough for the sea to be fully developed for this wind.
    pub fully_developed: bool,
    /// Depth (m) shallower than which waves of this height break (H/d = 0.78): the surf zone.
    pub breaking_depth_m: f32,
    /// Gerstner components, largest first.
    pub components: Vec<WaveComponent>,
}

/// The wind sea raised by `p`.
pub fn wave_spectrum(p: &WaveParams) -> JdResult<WaveSpectrum> {
    let u = p.wind_speed_mps;
    if !(u >= 0.0 && u.is_finite()) {
        return Err(JdError::invalid("wind_speed_mps", "must be ≥ 0"));
    }
    if let Some(f) = p.fetch_km
        && !(f > 0.0 && f.is_finite())
    {
        return Err(JdError::invalid("fetch_km", "must be positive"));
    }
    if !p.wind_azimuth_deg.is_finite() {
        return Err(JdError::invalid("wind_azimuth_deg", "must be finite"));
    }
    // Fully developed Pierson–Moskowitz sea.
    let (hs_fd, tp_fd) = (0.0246 * u * u, 0.78 * u);
    let (mut hs, mut tp, mut fully) = (hs_fd, tp_fd, true);
    if let Some(f) = p.fetch_km
        && u > 0.0
    {
        let chi = G * f * 1000.0 / (u * u);
        let hs_f = 1.6e-3 * chi.sqrt() * u * u / G;
        let tp_f = 0.2857 * chi.cbrt() * u / G;
        if hs_f < hs_fd {
            (hs, tp, fully) = (hs_f, tp_f.min(tp_fd), false);
        }
    }
    let wavelength = G * tp * tp / (2.0 * std::f32::consts::PI);
    let travel = (p.wind_azimuth_deg + 180.0).rem_euclid(360.0);
    let component = |share: f32, l_div: f32, spread: f32, phase: f32| {
        let l = wavelength / l_div;
        WaveComponent {
            amplitude_m: hs * share,
            wavelength_m: l,
            direction_deg: (travel + spread).rem_euclid(360.0),
            speed_mps: (G * l / (2.0 * std::f32::consts::PI)).sqrt(),
            phase,
        }
    };
    let components = if hs > 0.0 {
        vec![
            component(0.4, 1.0, 0.0, 0.0),
            component(0.2, 2.2, 25.0, 1.3),
            component(0.1, 5.0, -35.0, 2.9),
        ]
    } else {
        Vec::new()
    };
    Ok(WaveSpectrum {
        significant_height_m: hs,
        peak_period_s: tp,
        wavelength_m: wavelength,
        fully_developed: fully,
        breaking_depth_m: hs / 0.78,
        components,
    })
}

/// Wave exposure 0..1 per water cell: the open-water fetch upwind of it (land resets it, the field
/// border counts as open sea), as a share of `fetch_km`, square-rooted because wave height grows
/// with the square root of the fetch. Lee shores behind islands and headlands read low — the
/// wave shadow — and open coasts 1. Land is 0. One upwind sweep with the wind rounded to an
/// 8-neighbour direction (like the orographic rain).
pub fn wave_exposure(
    ocean: &Heightfield,
    spacing_m: [f32; 2],
    wind_azimuth_deg: f32,
    fetch_km: f32,
) -> JdResult<Heightfield> {
    if !(fetch_km > 0.0 && fetch_km.is_finite()) {
        return Err(JdError::invalid("fetch_km", "must be positive"));
    }
    if !wind_azimuth_deg.is_finite() {
        return Err(JdError::invalid("wind_azimuth_deg", "must be finite"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let (w, h) = (ocean.width() as i64, ocean.height() as i64);
    let to = (wind_azimuth_deg + 180.0).to_radians();
    let (ddx, ddy) = (to.sin().round() as i64, to.cos().round() as i64);
    let step_m = ((ddx as f32 * spacing_m[0]).powi(2) + (ddy as f32 * spacing_m[1]).powi(2)).sqrt();
    let fetch_m = fetch_km * 1000.0;
    let mut reach = vec![0.0f32; (w * h) as usize];
    let xs = |k: i64| if ddx >= 0 { k } else { w - 1 - k };
    let ys = |k: i64| if ddy >= 0 { k } else { h - 1 - k };
    for y in (0..h).map(ys) {
        for x in (0..w).map(xs) {
            let i = (y * w + x) as usize;
            if ocean.get(x as u32, y as u32) <= 0.5 {
                continue; // land: the fetch starts over downwind of it
            }
            let (ux, uy) = (x - ddx, y - ddy);
            reach[i] = if ux >= 0 && uy >= 0 && ux < w && uy < h {
                (reach[(uy * w + ux) as usize] + step_m).min(fetch_m)
            } else {
                fetch_m
            };
        }
    }
    let mut out = Heightfield::new(
        ocean.width(),
        ocean.height(),
        ocean.tile_size(),
        ocean.halo(),
    )?;
    out.fill_with(|x, y| (reach[(y as i64 * w + x as i64) as usize] / fetch_m).sqrt());
    Ok(out)
}

/// Refraction coefficient Kr per water cell for waves of deep-water length `wavelength_m` arriving
/// on the wind. The wave front's arrival time T from the upwind field edge is solved on the grid
/// (the eikonal equation |∇T| = 1/c by fast marching with the Godunov upwind update) with the
/// phase speed of the depth — Fenton & McKee (1990), c/c₀ = tanh^⅔((2πd/L₀)^¾), within 1.7 % of
/// the dispersion relation from deep water to the beach — so the front slows and turns over the
/// shelf. Rays run along ∇T; ray theory conserves the energy between neighbouring rays, so the
/// height scales by Kr = √(b₀/b) with b the ray spacing and d ln b/ds = ∇·û along a ray:
/// Kr = exp(−½ ∫ ∇·û ds), integrated from the upwind edge down the steepest-descent path of T.
/// Rays converge on headlands (Kr > 1, capped at 2 — ray theory diverges at caustics) and spread
/// in bays (Kr < 1), and an oblique front over straight contours spreads as Snell's law turns it
/// towards the shore (unit test against the textbook √(cos θ₀/cos θ)); deep water (d ≥ L₀/2)
/// keeps the plane front and lets the focus a shoal gave the ray fade over five wavelengths —
/// the scale on which diffraction smears it in a real sea — land, calm and water no upwind edge
/// reaches (a bay that opens downwind only) are 1. Kr is held within 0.5..2: ray theory
/// diverges at caustics and the first-arrival front has kinks where its families meet.
/// The swell enters through the upwind field edges as if the deep ocean started there, so an
/// oblique wind's two edge fronts cross along a line from the upwind corner (a lens trail of
/// ±30 % where that line runs over a shelf). Not modelled: shoaling (the coastal node works at
/// the shore, where every wave has broken) and diffraction, which in a real sea smears a shoal's
/// focus within a few wavelengths downwind (hence the fade, not a model of it).
/// ponytail: a binary heap of (time bits, cell); at 4096² it peaks around 400 MB — a bucketed
/// fast marching (untidy priority queue) is the upgrade for bigger seas.
pub fn wave_refraction(
    ocean: &Heightfield,
    depth: &Heightfield,
    spacing_m: [f32; 2],
    wind_azimuth_deg: f32,
    wavelength_m: f32,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    if !wind_azimuth_deg.is_finite() {
        return Err(JdError::invalid("wind_azimuth_deg", "must be finite"));
    }
    if !(wavelength_m >= 0.0 && wavelength_m.is_finite()) {
        return Err(JdError::invalid("wavelength_m", "must be finite and ≥ 0"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    if (ocean.width(), ocean.height()) != (depth.width(), depth.height()) {
        return Err(JdError::invalid(
            "depth",
            "size differs from the ocean mask",
        ));
    }
    let mut out = Heightfield::new(
        ocean.width(),
        ocean.height(),
        ocean.tile_size(),
        ocean.halo(),
    )?;
    if wavelength_m == 0.0 {
        out.fill_with(|_, _| 1.0);
        return Ok(out);
    }
    let (w, h) = (ocean.width() as usize, ocean.height() as usize);
    let n = w * h;
    let water: Vec<bool> = ocean.to_vec().iter().map(|o| *o > 0.5).collect();
    // Slowness c₀/c from d/L₀, floored so a cell lapping the shore cannot stall the front; the
    // front only bends where the bottom is felt (d < L₀/2), so that is where the ray spacing is
    // integrated — in deep water ∇·û is 0 by construction and would only add the kinks of the
    // first-arrival solution where fronts have crossed.
    let (slowness, shallow): (Vec<f32>, Vec<bool>) = depth
        .to_vec()
        .iter()
        .map(|d| {
            let d = d.max(0.0);
            let s = (2.0 * std::f32::consts::PI * d / wavelength_m)
                .powf(0.75)
                .tanh()
                .powf(2.0 / 3.0);
            (1.0 / s.max(0.05), d < 0.5 * wavelength_m)
        })
        .unzip();
    let to = (wind_azimuth_deg + 180.0).to_radians();
    let snap = |v: f32| if v.abs() < 1e-6 { 0.0 } else { v };
    let (dx, dy) = (snap(to.sin()), snap(to.cos())); // travel direction on the grid axes
    let [sx, sy] = spacing_m;
    let mut t = vec![f32::INFINITY; n];
    let mut known = vec![false; n];
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new(); // T ≥ 0: its bits order like the float
    // Sources: the water cells of the edges the front enters through, at the plane wave's time.
    let plane = |x: usize, y: usize| x as f32 * sx * dx + y as f32 * sy * dy;
    let t0 = plane(
        if dx < 0.0 { w - 1 } else { 0 },
        if dy < 0.0 { h - 1 } else { 0 },
    );
    for i in 0..n {
        let (x, y) = (i % w, i / w);
        let edge = (x == 0 && dx > 0.0)
            || (x == w - 1 && dx < 0.0)
            || (y == 0 && dy > 0.0)
            || (y == h - 1 && dy < 0.0);
        if edge && water[i] {
            t[i] = plane(x, y) - t0;
            heap.push(Reverse((t[i].to_bits(), i as u32)));
        }
    }
    // Godunov update: ((T−a)/hx)² + ((T−b)/hy)² = f² over the known neighbours on each axis.
    let eikonal = |a: f32, b: f32, f: f32| -> f32 {
        match (a.is_finite(), b.is_finite()) {
            (false, false) => f32::INFINITY,
            (true, false) => a + f * sx,
            (false, true) => b + f * sy,
            (true, true) => {
                // Solved for τ = T − min(a, b) so the discriminant is a sum, not a difference of
                // squares (in f32 the plain form loses the plane front to cancellation).
                let (lo, hi, ilo, ihi) = if a <= b {
                    (a, b, 1.0 / (sx * sx), 1.0 / (sy * sy))
                } else {
                    (b, a, 1.0 / (sy * sy), 1.0 / (sx * sx))
                };
                let d = hi - lo;
                let rad = (ilo + ihi) * f * f - ilo * ihi * d * d;
                let tau = (ihi * d + rad.max(0.0).sqrt()) / (ilo + ihi);
                if rad >= 0.0 && tau >= d {
                    lo + tau
                } else {
                    (a + f * sx).min(b + f * sy)
                }
            }
        }
    };
    let mut order: Vec<u32> = Vec::with_capacity(n);
    while let Some(Reverse((_, i))) = heap.pop() {
        let i = i as usize;
        if known[i] {
            continue;
        }
        known[i] = true;
        order.push(i as u32);
        if order.len().is_multiple_of(65_536) {
            check_cancel(cancel)?;
        }
        let (x, y) = (i % w, i / w);
        for (nx, ny) in [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ] {
            if nx >= w || ny >= h || known[ny * w + nx] || !water[ny * w + nx] {
                continue;
            }
            // (borrows `known`/`t` only until the update below, so it lives inside the loop)
            let at = |xx: usize, yy: usize| {
                if xx < w && yy < h && known[yy * w + xx] {
                    t[yy * w + xx]
                } else {
                    f32::INFINITY
                }
            };
            let a = at(nx.wrapping_sub(1), ny).min(at(nx + 1, ny));
            let b = at(nx, ny.wrapping_sub(1)).min(at(nx, ny + 1));
            let j = ny * w + nx;
            let cand = eikonal(a, b, slowness[j]);
            if cand < t[j] {
                t[j] = cand;
                heap.push(Reverse((cand.to_bits(), j as u32)));
            }
        }
    }
    // Unit ray direction û = ∇T/|∇T| (central differences over reached cells, one-sided at the
    // edges), then its divergence the same way.
    let diff = |f: &dyn Fn(usize, usize) -> Option<f32>, x: usize, y: usize| -> [f32; 2] {
        let d1 = |l: Option<f32>, r: Option<f32>, c: f32, h: f32| match (l, r) {
            (Some(l), Some(r)) => (r - l) / (2.0 * h),
            (Some(l), None) => (c - l) / h,
            (None, Some(r)) => (r - c) / h,
            (None, None) => 0.0,
        };
        let c = f(x, y).unwrap_or(0.0);
        [
            d1(x.checked_sub(1).and_then(|xm| f(xm, y)), f(x + 1, y), c, sx),
            d1(y.checked_sub(1).and_then(|ym| f(x, ym)), f(x, y + 1), c, sy),
        ]
    };
    let tt = |x: usize, y: usize| (x < w && y < h && known[y * w + x]).then(|| t[y * w + x]);
    let u: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let g = diff(&tt, i % w, i / w);
            let m = (g[0] * g[0] + g[1] * g[1]).sqrt();
            if m > 0.0 {
                [g[0] / m, g[1] / m]
            } else {
                [dx, dy]
            }
        })
        .collect();
    let (kn, uu) = (&known, &u);
    let uc = |k: usize| {
        move |x: usize, y: usize| (x < w && y < h && kn[y * w + x]).then(|| uu[y * w + x][k])
    };
    // ∫ ∇·û ds down the path the front took: each cell adds its divergence over the step from the
    // reached 8-neighbour with the earliest arrival.
    let mut s = vec![0.0f32; n];
    for (k, &i) in order.iter().enumerate() {
        if k.is_multiple_of(65_536) {
            check_cancel(cancel)?;
        }
        let i = i as usize;
        let (x, y) = (i % w, i / w);
        let mut from: Option<(usize, f32)> = None;
        for (ox, oy) in (-1i64..=1).flat_map(|oy| (-1i64..=1).map(move |ox| (ox, oy))) {
            let (nx, ny) = (x as i64 + ox, y as i64 + oy);
            if (ox, oy) == (0, 0) || nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let j = ny as usize * w + nx as usize;
            if known[j] && t[j] < t[i] && from.is_none_or(|(k, _)| t[j] < t[k]) {
                from = Some((
                    j,
                    ((ox as f32 * sx).powi(2) + (oy as f32 * sy).powi(2)).sqrt(),
                ));
            }
        }
        if let Some((j, step)) = from {
            // Over the shelf the ray spacing integrates exactly; in deep water a shoal's lens
            // trail fades over five wavelengths, the scale on which diffraction smears it.
            s[i] = if shallow[i] {
                s[j] + (diff(&uc(0), x, y)[0] + diff(&uc(1), x, y)[1]) * step
            } else {
                s[j] * (-step / (5.0 * wavelength_m)).exp()
            };
        }
    }
    out.fill_with(|x, y| {
        let i = y as usize * w + x as usize;
        if known[i] {
            (-0.5 * s[i]).exp().clamp(0.5, 2.0)
        } else {
            1.0
        }
    });
    Ok(out)
}

/// Surface current direction per cell, degrees clockwise from north the water flows TOWARDS:
/// wind drift in open water (the air's travel direction), bent to run alongshore within
/// `band_m` of land — the tangent of the distance-to-land field, whichever way lies closer to
/// the wind — with a smooth blend between (`d / band_m`). A land cell beside water carries its
/// water neighbour's direction (so a river mouth or a shore cell, where the seabed classifier
/// samples it, reads the alongshore flow); land further in carries the wind.
/// ponytail: no Coriolis, no pressure set-up, one band width; the distance to land is the
/// shared 8-neighbour Dijkstra (`dist::water_distance`), seeded with the coast cells only.
pub fn surface_current(
    ocean: &Heightfield,
    spacing_m: [f32; 2],
    wind_azimuth_deg: f32,
    band_m: f32,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    if !wind_azimuth_deg.is_finite() {
        return Err(JdError::invalid("wind_azimuth_deg", "must be finite"));
    }
    if !(band_m > 0.0 && band_m.is_finite()) {
        return Err(JdError::invalid("current_band_m", "must be positive"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let (w, h) = (ocean.width() as usize, ocean.height() as usize);
    let water: Vec<bool> = ocean.to_vec().iter().map(|o| *o > 0.5).collect();
    let neighbours = |i: usize| {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        (-1i64..=1)
            .flat_map(move |oy| (-1i64..=1).map(move |ox| (x + ox, y + oy)))
            .filter(move |&(nx, ny)| {
                (nx, ny) != (x, y) && nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64
            })
            .map(move |(nx, ny)| ny as usize * w + nx as usize)
    };
    let coast = (0..w * h).filter(|&i| !water[i] && neighbours(i).any(|j| water[j]));
    let d = crate::dist::water_distance(&water, coast, w, h, spacing_m, band_m, cancel)?;
    let to = (wind_azimuth_deg + 180.0).to_radians();
    let (wx, wy) = (to.sin(), to.cos());
    let wind = (wind_azimuth_deg + 180.0).rem_euclid(360.0);
    let [sx, sy] = spacing_m;
    let dd =
        |x: usize, y: usize| (x < w && y < h && water[y * w + x]).then(|| d[y * w + x].min(band_m));
    let water_dir = |i: usize| -> f32 {
        let (x, y) = (i % w, i / w);
        if d[i] >= band_m {
            return wind;
        }
        // Gradient of the distance field (central where possible), its tangent the alongshore line.
        let g = |l: Option<f32>, r: Option<f32>, c: f32, hh: f32| match (l, r) {
            (Some(l), Some(r)) => (r - l) / (2.0 * hh),
            (Some(l), None) => (c - l) / hh,
            (None, Some(r)) => (r - c) / hh,
            (None, None) => 0.0,
        };
        let c = d[i].min(band_m);
        let gx = g(
            x.checked_sub(1).and_then(|xm| dd(xm, y)),
            dd(x + 1, y),
            c,
            sx,
        );
        let gy = g(
            y.checked_sub(1).and_then(|ym| dd(x, ym)),
            dd(x, y + 1),
            c,
            sy,
        );
        let m = (gx * gx + gy * gy).sqrt();
        if m <= 0.0 {
            return wind;
        }
        let (mut tx, mut ty) = (-gy / m, gx / m);
        if tx * wx + ty * wy < 0.0 {
            (tx, ty) = (-tx, -ty);
        }
        let s = (c / band_m).clamp(0.0, 1.0);
        let (vx, vy) = (wx * s + tx * (1.0 - s), wy * s + ty * (1.0 - s));
        if vx.abs() + vy.abs() < 1e-6 {
            return wind;
        }
        vx.atan2(vy).to_degrees().rem_euclid(360.0)
    };
    let mut out = Heightfield::new(
        ocean.width(),
        ocean.height(),
        ocean.tile_size(),
        ocean.halo(),
    )?;
    out.fill_with(|x, y| {
        let i = y as usize * w + x as usize;
        if water[i] {
            water_dir(i)
        } else {
            // Land: the nearest water neighbour's flow, the wind further inland.
            neighbours(i)
                .filter(|&j| water[j])
                .map(water_dir)
                .next()
                .unwrap_or(wind)
        }
    });
    Ok(out)
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_current_runs_with_the_wind_offshore_and_alongshore_at_the_coast() {
        // 64 × 32 sea, 50 m cells, land from x = 48 east; a south-westerly (air travels north-east).
        let mut ocean = Heightfield::new(64, 32, 64, 0).expect("o");
        ocean.fill_with(|x, _| if x >= 48 { 0.0 } else { 1.0 });
        let cur = surface_current(&ocean, [50.0, 50.0], 225.0, 1000.0, None).expect("current");
        assert!(
            (cur.get(4, 16) - 45.0).abs() < 1.0,
            "offshore it is the wind: {}",
            cur.get(4, 16)
        );
        assert!(
            cur.get(47, 16) < 5.0 || cur.get(47, 16) > 355.0,
            "at the coast it runs north, alongshore: {}",
            cur.get(47, 16)
        );
        let mid = cur.get(38, 16); // 500 m out: half way round
        assert!(mid > 5.0 && mid < 40.0, "between, it bends: {mid}");
        assert!(
            (cur.get(60, 16) - 45.0).abs() < 1.0,
            "inland the land carries the wind"
        );
        assert!(
            cur.get(48, 16) < 5.0 || cur.get(48, 16) > 355.0,
            "the shore cell carries its water neighbour's alongshore flow: {}",
            cur.get(48, 16)
        );
        // A straight-on westerly: alongshore either way; the convention picks the left-hand tangent.
        let on = surface_current(&ocean, [50.0, 50.0], 270.0, 1000.0, None).expect("onshore");
        let a = on.get(47, 16);
        assert!(
            a < 5.0 || a > 355.0 || (a - 180.0).abs() < 5.0,
            "alongshore: {a}"
        );
        assert!(surface_current(&ocean, [50.0, 50.0], 270.0, 0.0, None).is_err());
    }

    #[test]
    fn refraction_focuses_waves_on_a_headland_and_spreads_them_in_the_bays() {
        // 128 × 64 sea, 50 m cells: deep water (80 m) in the west, a shelf shoaling to 2 m at the
        // coast (x ≥ 100) with a shallow ridge running out from the middle of it — the headland —
        // and a westerly sending 100 m waves (Tp 8 s) straight at it.
        let (w, h) = (128u32, 64u32);
        let mut ocean = Heightfield::new(w, h, 64, 0).expect("o");
        ocean.fill_with(|x, _| if x >= 100 { 0.0 } else { 1.0 });
        let mut depth = Heightfield::new(w, h, 64, 0).expect("d");
        depth.fill_with(|x, y| {
            let shelf = (100.0 - x as f32).clamp(0.0, 40.0) / 40.0; // 1 at 40 cells out, 0 at the coast
            let ridge = (-((y as f32 - 32.0) / 10.0).powi(2)).exp() * (1.0 - shelf);
            (2.0 + 78.0 * shelf) * (1.0 - 0.8 * ridge)
        });
        let kr = wave_refraction(&ocean, &depth, [50.0, 50.0], 270.0, 100.0, None).expect("kr");
        assert!(
            (kr.get(30, 32) - 1.0).abs() < 1e-3,
            "deep water keeps the plane front: {}",
            kr.get(30, 32)
        );
        assert!((kr.get(30, 5) - 1.0).abs() < 1e-3, "{}", kr.get(30, 5));
        let (tip, bay) = (kr.get(97, 32), kr.get(97, 26)); // the tip and a flank the rays bend away from
        assert!(tip > 1.15, "rays converge on the ridge: {tip}");
        assert!(bay < 0.92, "and spread beside it: {bay}");
        assert!(
            (kr.get(97, 2) - 1.0).abs() < 0.01,
            "far from the ridge the shelf is straight: {}",
            kr.get(97, 2)
        );
        assert_eq!(kr.get(110, 32), 1.0, "land is 1");
        assert!(
            kr.to_vec().iter().all(|k| (0.5..=2.0).contains(k)),
            "Kr stays within its floor and cap"
        );
        // Straight contours leave straight rays: without the ridge the shelf alone does nothing.
        depth.fill_with(|x, _| 2.0 + 78.0 * (100.0 - x as f32).clamp(0.0, 40.0) / 40.0);
        let flat = wave_refraction(&ocean, &depth, [50.0, 50.0], 270.0, 100.0, None).expect("flat");
        assert!(
            (flat.get(97, 32) - 1.0).abs() < 1e-3,
            "{}",
            flat.get(97, 32)
        );
        // An oblique front over straight contours follows Snell's law and spreads as it turns
        // towards the normal: Kr = √(cos θ₀ / cos θ) (Dean & Dalrymple 1991, eq. 4.100). 45° in
        // deep water, c/c₀ = 0.653 at the 7.85 m of column 97 → sin θ = 0.462, Kr = 0.893 — read
        // away from the south edge, whose own front crosses the west edge's near it.
        let obl =
            wave_refraction(&ocean, &depth, [50.0, 50.0], 225.0, 100.0, None).expect("oblique");
        assert!(
            (obl.get(97, 50) - 0.893).abs() < 0.03,
            "{}",
            obl.get(97, 50)
        );
        // Calm: no waves to bend.
        assert_eq!(
            wave_refraction(&ocean, &depth, [50.0, 50.0], 270.0, 0.0, None)
                .expect("calm")
                .get(97, 32),
            1.0
        );
        assert!(wave_refraction(&ocean, &depth, [50.0, 50.0], f32::NAN, 100.0, None).is_err());
    }

    #[test]
    fn an_island_casts_a_wave_shadow_downwind_and_open_coasts_read_full() {
        // 64 × 32 sea, 50 m cells, with an island block in the middle; a westerly, 1 km fetch.
        let mut ocean = Heightfield::new(64, 32, 64, 0).expect("o");
        ocean.fill_with(|x, y| {
            if (24..40).contains(&x) && (10..22).contains(&y) {
                0.0
            } else {
                1.0
            }
        });
        let e = wave_exposure(&ocean, [50.0, 50.0], 270.0, 1.0).expect("exposure");
        assert_eq!(e.get(0, 16), 1.0, "the upwind border is open sea");
        assert_eq!(e.get(30, 16), 0.0, "land is 0");
        assert!(e.get(40, 16) < 0.3, "just in the lee: {}", e.get(40, 16));
        assert!(
            e.get(63, 16) > e.get(40, 16) && e.get(63, 16) > 0.9,
            "the shadow fades downwind: {}",
            e.get(63, 16)
        );
        assert_eq!(e.get(63, 2), 1.0, "a row with no island is fully exposed");
        // An easterly mirrors the shadow to the west side.
        let w = wave_exposure(&ocean, [50.0, 50.0], 90.0, 1.0).expect("easterly");
        assert!(
            w.get(23, 16) < 0.3 && w.get(40, 16) > 0.9,
            "{} / {}",
            w.get(23, 16),
            w.get(40, 16)
        );
        assert!(wave_exposure(&ocean, [50.0, 50.0], 270.0, 0.0).is_err());
    }
    use super::*;

    #[test]
    fn gale_fetch_limited_and_fully_developed_match_the_forecasting_curves() {
        // 20 m/s over 100 km: the JONSWAP curves give Hs ≈ 3.2 m, Tp ≈ 7.9 s.
        let s = wave_spectrum(&WaveParams {
            wind_speed_mps: 20.0,
            fetch_km: Some(100.0),
            wind_azimuth_deg: 270.0,
        })
        .expect("spectrum");
        assert!(
            (s.significant_height_m - 3.23).abs() < 0.05,
            "Hs {}",
            s.significant_height_m
        );
        assert!(
            (s.peak_period_s - 7.85).abs() < 0.05,
            "Tp {}",
            s.peak_period_s
        );
        assert!(!s.fully_developed);
        assert_eq!(s.components.len(), 3);
        assert_eq!(
            s.components[0].direction_deg, 90.0,
            "a west wind sends waves east"
        );
        // Unlimited fetch: Pierson–Moskowitz, Hs ≈ 9.8 m, Tp ≈ 15.6 s, L ≈ 380 m.
        let fd = wave_spectrum(&WaveParams {
            wind_speed_mps: 20.0,
            fetch_km: None,
            wind_azimuth_deg: 270.0,
        })
        .expect("fd");
        assert!(fd.fully_developed);
        assert!((fd.significant_height_m - 9.84).abs() < 0.05);
        assert!(
            (fd.wavelength_m - 380.0).abs() < 2.0,
            "L {}",
            fd.wavelength_m
        );
        assert!((fd.breaking_depth_m - 12.6).abs() < 0.1);
        // Steepness stays well below the Gerstner cusp limit k·A < 1 for every component.
        for c in &fd.components {
            let ka = 2.0 * std::f32::consts::PI / c.wavelength_m * c.amplitude_m;
            assert!(ka < 0.2, "k·A {ka}");
        }
        // A 5 000 km fetch is more than a 20 m/s sea needs: it reports fully developed.
        assert!(
            wave_spectrum(&WaveParams {
                wind_speed_mps: 20.0,
                fetch_km: Some(5000.0),
                wind_azimuth_deg: 0.0,
            })
            .expect("long fetch")
            .fully_developed
        );
        // Calm: no components; bad inputs are refused.
        assert!(
            wave_spectrum(&WaveParams {
                wind_speed_mps: 0.0,
                ..WaveParams::default()
            })
            .expect("calm")
            .components
            .is_empty()
        );
        assert!(
            wave_spectrum(&WaveParams {
                wind_speed_mps: -1.0,
                ..WaveParams::default()
            })
            .is_err()
        );
        assert!(
            wave_spectrum(&WaveParams {
                fetch_km: Some(0.0),
                ..WaveParams::default()
            })
            .is_err()
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Water bakes: what the viewport draws, as textures an engine can use.

/// Water clarity, mirrored from the sea-level node.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaterLook {
    /// Depth at which the bottom fades out, metres (> 0): ~30 crystal clear, 15 open ocean, 3 murky.
    #[serde(default = "d_visibility")]
    pub visibility_m: f32,
    /// Colour of the suspended matter scattered back, linear RGB 0..1.
    #[serde(default = "d_tint")]
    pub tint: [f32; 3],
}

fn d_visibility() -> f32 {
    15.0
}
fn d_tint() -> [f32; 3] {
    [0.1, 0.35, 0.4]
}

impl Default for WaterLook {
    fn default() -> Self {
        Self {
            visibility_m: d_visibility(),
            tint: d_tint(),
        }
    }
}

/// Pure-water absorption per channel, m⁻¹ (Pope & Fry 1997, rounded): red dies first, blue last.
const PURE_WATER_ABSORPTION: [f32; 3] = [0.4, 0.07, 0.005];
/// Wet sand seen through the water, linear RGB.
const BOTTOM: [f32; 3] = [0.62, 0.55, 0.38];

/// What a sea floor `depth_m` down looks like through the water (linear RGB), and how opaque the
/// surface is there: Beer–Lambert transmittance per channel, down and back up, with the absorption
/// scaled by 15 m / visibility; the tint takes over as the bottom disappears. The viewport uses
/// the same formula.
pub fn water_colour(depth_m: f32, look: &WaterLook) -> ([f32; 3], f32) {
    let k = 15.0 / look.visibility_m.max(0.5);
    let mut rgb = [0.0f32; 3];
    let mut seen = 0.0;
    for c in 0..3 {
        let t = (-PURE_WATER_ABSORPTION[c] * k * depth_m * 2.0).exp();
        rgb[c] = BOTTOM[c] * t + look.tint[c] * (1.0 - t);
        seen += t;
    }
    let clarity = seen / 3.0;
    (rgb, 0.35 + 0.55 * (1.0 - clarity))
}

/// Baked water textures: PNG bytes, row 0 = south like every other bake.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WaterMaps {
    /// Texture size in pixels.
    pub px: [u32; 2],
    /// Deepest water, metres: the depth map is `depth / max_depth_m` as 16-bit grey.
    pub max_depth_m: f32,
    /// Depth PNG (16-bit grey, 0 = land or the waterline).
    #[serde(skip)]
    pub depth: Vec<u8>,
    /// Water colour PNG (sRGB 8-bit): the bottom seen through the water, the tint where it is deep or murky.
    #[serde(skip)]
    pub colour: Vec<u8>,
    /// Foam PNG (8-bit grey): the shoreline band plus the surf zone where the wind sea breaks.
    #[serde(skip)]
    pub foam: Vec<u8>,
}

/// Bake depth, colour and foam for the sea at `level_m` over `hf` (the ocean mask rule of the
/// sea-level node: below the level and connected to the border), sampled so no side exceeds
/// `max_px`. `breaking_depth_m` (from the wind sea) widens the foam band to the surf zone.
pub fn bake_water(
    hf: &crate::Heightfield,
    level_m: f32,
    look: &WaterLook,
    breaking_depth_m: f32,
    max_px: u32,
) -> JdResult<WaterMaps> {
    bake_water_with(hf, level_m, look, breaking_depth_m, max_px, None)
}

/// [`bake_water`] with the coastal node's `shore` mask (0..1, the wave-cut band): its value is
/// added to the foam map on water cells and on the shore band itself, so exposed, eroding shores
/// wash white while sheltered ones keep only the thin depth band.
pub fn bake_water_with(
    hf: &crate::Heightfield,
    level_m: f32,
    look: &WaterLook,
    breaking_depth_m: f32,
    max_px: u32,
    shore: Option<&crate::Heightfield>,
) -> JdResult<WaterMaps> {
    if let Some(s) = shore
        && (s.width(), s.height()) != (hf.width(), hf.height())
    {
        return Err(JdError::invalid(
            "shore",
            "field size differs from the layer",
        ));
    }
    if !(look.visibility_m > 0.0 && look.visibility_m.is_finite()) {
        return Err(JdError::invalid("visibility_m", "must be positive"));
    }
    if look.tint.iter().any(|c| !(0.0..=1.0).contains(c)) {
        return Err(JdError::invalid("tint", "components must be within 0..=1"));
    }
    if !(breaking_depth_m >= 0.0 && breaking_depth_m.is_finite()) {
        return Err(JdError::invalid("breaking_depth_m", "must be ≥ 0"));
    }
    let (ocean, depth) = crate::flow::sea_level(hf, level_m, 0.0)?;
    let (w, h) = (hf.width(), hf.height());
    let ts = w.max(h).div_ceil(max_px.max(16)).max(1);
    let (tw, th) = (w.div_ceil(ts), h.div_ceil(ts));
    let stats = hf.stats();
    let foam_m = (0.5f32)
        .max((stats.max - stats.min) * 0.01)
        .max(breaking_depth_m);
    let max_depth = depth.stats().max.max(1e-3);
    let at = |x: u32, y: u32| ((x * ts).min(w - 1), (y * ts).min(h - 1));
    let mut depth_px = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::new(tw, th);
    let mut colour_px = image::RgbImage::new(tw, th);
    let mut foam_px = image::GrayImage::new(tw, th);
    for y in 0..th {
        for x in 0..tw {
            let (sx, sy) = at(x, y);
            let d = if ocean.get(sx, sy) > 0.5 {
                depth.get(sx, sy).max(0.0)
            } else {
                0.0
            };
            let (rgb, _) = water_colour(d, look);
            let mut foam = if d > 0.0 && d < foam_m {
                1.0 - d / foam_m
            } else {
                0.0
            };
            if let Some(s) = shore {
                let wash = s.get(sx, sy).clamp(0.0, 1.0);
                foam = foam.max(wash); // wherever the waves worked: water or the cliff foot
            }
            let srgb = |v: f32| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
            depth_px.put_pixel(
                x,
                y,
                image::Luma([((d / max_depth) * 65535.0).round() as u16]),
            );
            colour_px.put_pixel(x, y, image::Rgb([srgb(rgb[0]), srgb(rgb[1]), srgb(rgb[2])]));
            foam_px.put_pixel(x, y, image::Luma([(foam * 255.0).round() as u8]));
        }
    }
    Ok(WaterMaps {
        px: [tw, th],
        max_depth_m: max_depth,
        depth: png(&depth_px)?,
        colour: png(&colour_px)?,
        foam: png(&foam_px)?,
    })
}

fn png<P: image::Pixel + image::PixelWithColorType>(
    img: &image::ImageBuffer<P, Vec<P::Subpixel>>,
) -> JdResult<Vec<u8>>
where
    [P::Subpixel]: image::EncodableLayout,
{
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| {
            JdError::new(
                jd_types::ErrorCode::E_INTERNAL,
                format!("PNG encode failed: {e}"),
            )
        })?;
    Ok(out.into_inner())
}

/// A tileable wave normal map: every component is snapped to the nearest wave vector that fits
/// a whole number of times into `tile_m` (so the tile repeats seamlessly), then the Gerstner
/// surface at t = 0 is differentiated analytically. Tangent space +X east, +Y north, +Z up,
/// row 0 = south, `px` square. Returns the PNG and the components as snapped.
pub fn bake_wave_normal(
    components: &[WaveComponent],
    tile_m: f32,
    px: u32,
) -> JdResult<(Vec<u8>, Vec<WaveComponent>)> {
    if !(tile_m > 0.0 && tile_m.is_finite()) {
        return Err(JdError::invalid("wave_tile_m", "must be positive"));
    }
    if !(16..=4096).contains(&px) {
        return Err(JdError::invalid("wave_px", "must be within 16..=4096"));
    }
    let two_pi = 2.0 * std::f32::consts::PI;
    let snapped: Vec<WaveComponent> = components
        .iter()
        .map(|c| {
            let k = two_pi / c.wavelength_m;
            let rad = c.direction_deg.to_radians();
            let (dx, dz) = (rad.sin(), -rad.cos());
            // Integer wave numbers along x and z; a wave longer than the tile becomes one tile long.
            let mut n = (tile_m * k * dx / two_pi).round();
            let mut m = (tile_m * k * dz / two_pi).round();
            if n == 0.0 && m == 0.0 {
                (n, m) = if dx.abs() >= dz.abs() {
                    (dx.signum(), 0.0)
                } else {
                    (0.0, dz.signum())
                };
            }
            let len = (n * n + m * m).sqrt();
            WaveComponent {
                wavelength_m: tile_m / len,
                direction_deg: (n / len).atan2(-(m / len)).to_degrees().rem_euclid(360.0),
                ..c.clone()
            }
        })
        .collect();
    let mut img = image::RgbImage::new(px, px);
    let step = tile_m / px as f32;
    for row in 0..px {
        for col in 0..px {
            let x = col as f32 * step;
            let z = -(row as f32 * step); // row 0 = south: north is −z
            let (mut gx, mut gz) = (0.0f32, 0.0f32);
            for c in &snapped {
                let k = two_pi / c.wavelength_m;
                let rad = c.direction_deg.to_radians();
                let (dx, dz) = (rad.sin(), -rad.cos());
                let slope = c.amplitude_m * k * (k * (dx * x + dz * z) + c.phase).cos();
                gx += slope * dx;
                gz += slope * dz;
            }
            // Normal of y = h(x, z): (−∂h/∂x, 1, −∂h/∂z); tangent-space G is the north component.
            let (nx, ny, nz) = (-gx, 1.0, -gz);
            let inv = 1.0 / (nx * nx + ny * ny + nz * nz).sqrt();
            let enc = |v: f32| ((v * inv * 0.5 + 0.5) * 255.0).round() as u8;
            img.put_pixel(col, row, image::Rgb([enc(nx), enc(-nz), enc(ny)]));
        }
    }
    Ok((png(&img)?, snapped))
}

/// Bake the Gerstner displacement of the wind sea at t = 0 as a tileable 16-bit RGB map:
/// R = east, G = up, B = north offset, each `v / scale · 0.5 + 0.5`, where `scale` (returned,
/// metres) is the largest offset any pixel can reach, Σ A·(1 + Q). Pass the components as
/// snapped by [`bake_wave_normal`] so the tile wraps; an engine animates by scrolling each
/// component's phase, or vertex-displaces a still frame from this map.
pub fn bake_wave_displacement(
    components: &[WaveComponent],
    tile_m: f32,
    px: u32,
) -> JdResult<(Vec<u8>, f32)> {
    if !(tile_m > 0.0 && tile_m.is_finite()) {
        return Err(JdError::invalid("wave_tile_m", "must be positive"));
    }
    if !(16..=4096).contains(&px) {
        return Err(JdError::invalid("wave_px", "must be within 16..=4096"));
    }
    let two_pi = 2.0 * std::f32::consts::PI;
    let n = components.len().max(1) as f32;
    // GPU Gems 1 ch. 1: Qᵢ = Q / (kᵢ Aᵢ N) with Q = 0.5, capped at 1 — the viewport's convention.
    let q = |c: &WaveComponent| (0.5 / (two_pi / c.wavelength_m * c.amplitude_m * n)).min(1.0);
    let scale = components
        .iter()
        .map(|c| c.amplitude_m * (1.0 + q(c)))
        .sum::<f32>()
        .max(1e-3);
    let mut img = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::new(px, px);
    let step = tile_m / px as f32;
    for row in 0..px {
        for col in 0..px {
            let x = col as f32 * step;
            let z = -(row as f32 * step);
            let (mut ox, mut oy, mut oz) = (0.0f32, 0.0f32, 0.0f32);
            for c in components {
                let k = two_pi / c.wavelength_m;
                let rad = c.direction_deg.to_radians();
                let (dx, dz) = (rad.sin(), -rad.cos());
                let phi = k * (dx * x + dz * z) + c.phase;
                let qa = q(c) * c.amplitude_m * phi.cos();
                ox += qa * dx;
                oz += qa * dz;
                oy += c.amplitude_m * phi.sin();
            }
            let enc = |v: f32| ((v / scale * 0.5 + 0.5).clamp(0.0, 1.0) * 65535.0).round() as u16;
            img.put_pixel(col, row, image::Rgb([enc(ox), enc(oy), enc(-oz)]));
        }
    }
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .map_err(|e| JdError::new(jd_types::ErrorCode::E_INTERNAL, format!("png encode: {e}")))?;
    Ok((bytes, scale))
}

#[cfg(test)]
mod bake_tests {
    use super::*;

    #[test]
    fn the_shore_mask_adds_wash_foam() {
        let mut h = crate::Heightfield::new(64, 16, 64, 0).expect("hf");
        h.fill_with(|x, _| x as f32 / 63.0 * 300.0);
        let mut shore = crate::Heightfield::new(64, 16, 64, 0).expect("s");
        shore.fill_with(|x, _| f32::from((20..=26).contains(&x))); // the wave-cut band around the 110 m waterline (x ≈ 23)
        let plain = bake_water(&h, 110.0, &WaterLook::default(), 0.0, 64).expect("plain");
        let washed = bake_water_with(&h, 110.0, &WaterLook::default(), 0.0, 64, Some(&shore))
            .expect("washed");
        let foam = |m: &WaterMaps| image::load_from_memory(&m.foam).expect("png").to_luma8();
        let (a, b) = (foam(&plain), foam(&washed));
        assert_eq!(b.get_pixel(20, 8)[0], 255, "washed where the shore mask is");
        assert!(
            a.get_pixel(20, 8)[0] < 255,
            "the depth band alone is thinner"
        );
        assert_eq!(b.get_pixel(0, 8)[0], 0, "deep water unchanged");
        assert_eq!(b.get_pixel(40, 8)[0], 0, "dry land unchanged");
        assert!(
            bake_water_with(
                &h,
                110.0,
                &WaterLook::default(),
                0.0,
                64,
                Some(&crate::Heightfield::new(4, 4, 64, 0).expect("x"))
            )
            .is_err()
        );
    }

    #[test]
    fn displacement_tile_is_zero_mean_bounded_and_wraps() {
        let waves = wave_spectrum(&WaveParams {
            wind_speed_mps: 12.0,
            fetch_km: None,
            wind_azimuth_deg: 270.0,
        })
        .expect("spectrum");
        let (_, snapped) = bake_wave_normal(&waves.components, 320.0, 64).expect("normal");
        let (png, scale) = bake_wave_displacement(&snapped, 320.0, 64).expect("displacement");
        assert!(scale > 0.0 && scale < 3.0 * waves.significant_height_m);
        let img = image::load_from_memory(&png).expect("png").to_rgb16();
        assert_eq!(img.dimensions(), (64, 64));
        let up: Vec<f32> = img.pixels().map(|p| p[1] as f32 / 65535.0 - 0.5).collect();
        let mean = up.iter().sum::<f32>() / up.len() as f32;
        assert!(mean.abs() < 0.02, "zero-mean surface, got {mean}");
        assert!(up.iter().any(|v| v.abs() > 0.05), "the sea is not flat");
        // Snapped components make the tile periodic: the first column continues the last.
        let wrap = |col: u32, row: u32| img.get_pixel(col, row)[1] as f32;
        let (a, b, c) = (wrap(0, 10), wrap(63, 10), wrap(1, 10));
        assert!(
            (a - b).abs() <= (a - c).abs() * 1.5 + 300.0,
            "wraps: {a} {b} {c}"
        );
        assert!(bake_wave_displacement(&snapped, 0.0, 64).is_err());
    }

    #[test]
    fn water_maps_follow_depth_and_the_wave_tile_wraps() {
        // West → east ramp from 0 to 300 m with the sea at 110 m: the west edge is 110 m deep.
        let mut h = crate::Heightfield::new(64, 16, 64, 0).expect("hf");
        h.fill_with(|x, _| x as f32 / 63.0 * 300.0);
        let maps = bake_water(&h, 110.0, &WaterLook::default(), 0.0, 64).expect("bake");
        assert_eq!(maps.px, [64, 16]);
        assert!((maps.max_depth_m - 110.0).abs() < 0.01);
        let depth = image::load_from_memory(&maps.depth)
            .expect("png")
            .to_luma16();
        let colour = image::load_from_memory(&maps.colour)
            .expect("png")
            .to_rgb8();
        let foam = image::load_from_memory(&maps.foam).expect("png").to_luma8();
        assert_eq!(depth.get_pixel(0, 8)[0], 65535, "deepest at the west edge");
        assert!(depth.get_pixel(10, 8)[0] < depth.get_pixel(0, 8)[0]);
        assert_eq!(depth.get_pixel(40, 8)[0], 0, "land is 0");
        let deep = colour.get_pixel(0, 8);
        assert!(deep[2] > deep[0], "deep water is blue, not sandy");
        let shallow = colour.get_pixel(22, 8); // ~5 m deep: the sand still shows
        assert!(shallow[0] > deep[0]);
        assert!(foam.get_pixel(0, 8)[0] == 0, "no foam out deep");
        assert!(
            foam.get_pixel(23, 8)[0] > 100,
            "foam within 3 m of the shore (0.5 m deep)"
        );
        // A surf zone of 60 m puts foam on the whole shelf.
        let surf = bake_water(&h, 110.0, &WaterLook::default(), 60.0, 64).expect("surf");
        let surf_foam = image::load_from_memory(&surf.foam).expect("png").to_luma8();
        assert!(
            surf_foam.get_pixel(12, 8)[0] > 0,
            "53 m deep: inside the 60 m surf zone"
        );
        assert!(
            bake_water(
                &h,
                110.0,
                &WaterLook {
                    visibility_m: 0.0,
                    ..WaterLook::default()
                },
                0.0,
                64
            )
            .is_err()
        );

        // The wave tile: a 100 m wave on a 256 m tile snaps to 256/3 m; the tile's edges match.
        let spectrum = wave_spectrum(&WaveParams {
            wind_speed_mps: 10.0,
            fetch_km: None,
            wind_azimuth_deg: 270.0,
        })
        .expect("spectrum");
        let (png, snapped) = bake_wave_normal(&spectrum.components, 256.0, 64).expect("tile");
        assert_eq!(snapped.len(), 3);
        for c in &snapped {
            let n = 256.0 / c.wavelength_m;
            let rad = c.direction_deg.to_radians();
            let (nx, nz) = (n * rad.sin(), -n * rad.cos());
            assert!(
                (nx - nx.round()).abs() < 1e-3 && (nz - nz.round()).abs() < 1e-3,
                "{c:?} fits the tile"
            );
        }
        let img = image::load_from_memory(&png).expect("png").to_rgb8();
        // Periodicity is the integer wave numbers above; the 19 m component is sampled under
        // 5× per wavelength at 64 px, so a pixel seam comparison would only measure aliasing.
        // Mostly up: the mean normal points along +Z.
        let mean_b = img.pixels().map(|p| p[2] as u32).sum::<u32>() / (64 * 64);
        assert!(mean_b > 240, "mean blue {mean_b}");
    }
}
