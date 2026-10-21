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
use std::sync::atomic::{AtomicBool, Ordering};

/// 8-neighbourhood offsets; `NEIGHBOURS[7 - k]` is the opposite direction of `k`.
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

/// Metres between a cell and each neighbour for the given sample spacing (pixels may be
/// anisotropic: `WorldSettings::spacing_m`).
fn neighbour_dist(spacing: [f32; 2]) -> [f32; 8] {
    NEIGHBOURS.map(|(dx, dy)| (dx as f32 * spacing[0]).hypot(dy as f32 * spacing[1]))
}

/// Largest `rate` for which a cell cannot receive more than it is below its 8 neighbours in one
/// step (inflow ≤ 8·rate·(H−t)/2 ≤ H−t); above it a pit can overshoot into a spike.
pub const THERMAL_MAX_RATE: f32 = 0.25;

/// Cooperative cancellation: long kernels poll the job's flag once per iteration.
pub(crate) fn check_cancel(cancel: Option<&AtomicBool>) -> JdResult<()> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err(JdError::new(jd_types::ErrorCode::E_CANCELLED, "cancelled")
            .why("The job's cancel flag was set.")
            .next("Nothing was published; re-run when ready."));
    }
    Ok(())
}

fn check_spacing(spacing_m: [f32; 2]) -> JdResult<()> {
    if spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        Ok(())
    } else {
        Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ))
    }
}

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
/// Per-cell talus tangent and rate: hard rock (hardness → 1) holds an 88° face and sheds at a
/// tenth of the rate; soft rock keeps the node's values.
struct Rock<'a> {
    tan_t: f32,
    rate: f32,
    hardness: Option<&'a [f32]>,
}

impl Rock<'_> {
    #[inline]
    fn tan_at(&self, i: usize) -> f32 {
        match self.hardness {
            Some(h) => {
                let k = h[i].clamp(0.0, 1.0);
                // Interpolate the angle, not the tangent, so a 50 % rock reads as a mid slope.
                let deg = self.tan_t.atan().to_degrees();
                (deg + k * (88.0 - deg)).to_radians().tan()
            }
            None => self.tan_t,
        }
    }
    #[inline]
    fn rate_at(&self, i: usize) -> f32 {
        match self.hardness {
            Some(h) => self.rate * (1.0 - 0.9 * h[i].clamp(0.0, 1.0)),
            None => self.rate,
        }
    }
}

fn shed_factors(g: &Grid, rock: &Rock, dist: &[f32; 8]) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0f32; 2]; (g.w * g.hh) as usize];
    out.par_chunks_mut(g.w as usize)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, cell) in row.iter_mut().enumerate() {
                let (xi, yi) = (x as i64, y as i64);
                let hc = g.at(xi, yi);
                let tan_t = rock.tan_at((yi * g.w + xi) as usize);
                let rate = rock.rate_at((yi * g.w + xi) as usize);
                let (mut total, mut dmax) = (0.0f32, 0.0f32);
                for (k, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
                    let (nx, ny) = (xi + dx, yi + dy);
                    if !g.inside(nx, ny) {
                        continue; // material never leaves the field: exact conservation
                    }
                    let d = hc - g.at(nx, ny) - tan_t * dist[k];
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
fn gather(g: &Grid, shed: &[[f32; 2]], rock: &Rock, dist: &[f32; 8]) -> Vec<f32> {
    let mut out = vec![0.0f32; (g.w * g.hh) as usize];
    out.par_chunks_mut(g.w as usize)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, cell) in row.iter_mut().enumerate() {
                let (xi, yi) = (x as i64, y as i64);
                let hc = g.at(xi, yi);
                let mut gained = 0.0f32;
                for (k, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
                    let (nx, ny) = (xi + dx, yi + dy);
                    if !g.inside(nx, ny) {
                        continue;
                    }
                    let factor = shed[(ny * g.w + nx) as usize][1];
                    if factor > 0.0 {
                        // The shedding neighbour's own talus decides what it lets go of.
                        let excess =
                            g.at(nx, ny) - hc - rock.tan_at((ny * g.w + nx) as usize) * dist[k];
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
/// `spacing_m` is the horizontal sample spacing (x, y); `rate` in (0, 0.25] scales how much of
/// the excess moves per step ([`THERMAL_MAX_RATE`] keeps a pit from overshooting its rim). Mass is
/// conserved exactly up to float rounding and the result is independent of thread count.
pub fn thermal_erode(
    h: &Heightfield,
    spacing_m: [f32; 2],
    talus_deg: f32,
    iterations: u32,
    rate: f32,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    thermal_erode_with(h, spacing_m, talus_deg, iterations, rate, None, cancel)
}

/// [`thermal_erode`] with an optional per-cell `hardness` field (0..1, e.g. from
/// [`crate::strata_hardness`]): hard rock holds a near-vertical face and sheds at a tenth of
/// the rate, so caprock makes cliffs and shale makes talus — mesas, buttes, stepped walls.
pub fn thermal_erode_with(
    h: &Heightfield,
    spacing_m: [f32; 2],
    talus_deg: f32,
    iterations: u32,
    rate: f32,
    hardness: Option<&Heightfield>,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    if let Some(k) = hardness
        && (k.width(), k.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid("hardness", "field size differs from `in`"));
    }
    if !(0.0..90.0).contains(&talus_deg) {
        return Err(JdError::invalid("talus_deg", "must be within 0..90"));
    }
    if !(0.0 < rate && rate <= THERMAL_MAX_RATE) {
        return Err(JdError::invalid(
            "rate",
            format!("must be within (0, {THERMAL_MAX_RATE}]"),
        ));
    }
    check_spacing(spacing_m)?;
    let dist = neighbour_dist(spacing_m);
    let tan_t = talus_deg.to_radians().tan();
    let (w, hh) = (h.width() as i64, h.height() as i64);
    let hard_vec = hardness.map(|k| k.to_vec());
    let rock = Rock {
        tan_t,
        rate,
        hardness: hard_vec.as_deref(),
    };
    let mut cur = h.to_vec();
    for _ in 0..iterations {
        check_cancel(cancel)?;
        let g = Grid { h: &cur, w, hh };
        let shed = shed_factors(&g, &rock, &dist);
        cur = gather(&g, &shed, &rock, &dist);
    }
    let mut out = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    out.fill_with(|x, y| cur[(y as usize) * (w as usize) + x as usize]);
    Ok(out)
}

/// [`thermal_erode_with`] tile by tile: every `tile_px` square is eroded on its own sub-field
/// with a halo of `iterations` cells around it — one step moves material one cell, so a halo
/// that wide makes the tile's interior identical to the whole-field result — and only the
/// interior is kept. Scratch is a sub-field's worth instead of 12 B per cell of the whole
/// field, which is what lets a 16k² layer erode inside the memory budget (ADR-0005).
#[allow(clippy::too_many_arguments)]
pub fn thermal_erode_tiled(
    h: &Heightfield,
    spacing_m: [f32; 2],
    talus_deg: f32,
    iterations: u32,
    rate: f32,
    hardness: Option<&Heightfield>,
    tile_px: u32,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    if tile_px < 16 {
        return Err(JdError::invalid(
            "tile_px",
            "must be ≥ 16 (0 erodes the whole field at once)",
        ));
    }
    if let Some(k) = hardness
        && (k.width(), k.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid("hardness", "field size differs from `in`"));
    }
    let (w, hh) = (h.width(), h.height());
    let halo = iterations;
    let mut out = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    // Tiles go band by band: a band's rows are assembled in one buffer and written with
    // `set_row`, which also pads the edge tiles exactly as `fill_with` does.
    for y0 in (0..hh).step_by(tile_px as usize) {
        let y1 = (y0 + tile_px).min(hh);
        let mut band = vec![0.0f32; (w * (y1 - y0)) as usize];
        for x0 in (0..w).step_by(tile_px as usize) {
            check_cancel(cancel)?;
            let x1 = (x0 + tile_px).min(w);
            let (sx0, sy0) = (x0.saturating_sub(halo), y0.saturating_sub(halo));
            let (sx1, sy1) = ((x1 + halo).min(w), (y1 + halo).min(hh));
            let (sw, sh) = (sx1 - sx0, sy1 - sy0);
            let mut sub = Heightfield::new(sw, sh, 64, 0)?;
            sub.fill_with(|x, y| h.get(sx0 + x, sy0 + y));
            let sub_hard = hardness
                .map(|k| -> JdResult<Heightfield> {
                    let mut f = Heightfield::new(sw, sh, 64, 0)?;
                    f.fill_with(|x, y| k.get(sx0 + x, sy0 + y));
                    Ok(f)
                })
                .transpose()?;
            let eroded = thermal_erode_with(
                &sub,
                spacing_m,
                talus_deg,
                iterations,
                rate,
                sub_hard.as_ref(),
                cancel,
            )?;
            for y in y0..y1 {
                for x in x0..x1 {
                    band[((y - y0) * w + x) as usize] = eroded.get(x - sx0, y - sy0);
                }
            }
        }
        for y in y0..y1 {
            out.set_row(
                y,
                &band[((y - y0) * w) as usize..((y - y0 + 1) * w) as usize],
            );
        }
    }
    out.rebuild_halos();
    Ok(out)
}

/// Hydraulic erosion parameters (virtual pipes / shallow water, Mei–Decaudin–Hu 2007).
#[derive(Debug, Clone, Copy)]
pub struct HydraulicParams {
    /// Simulation steps.
    pub iterations: u32,
    /// Time step (s); 0.02 with metre cells is stable for the pipe model.
    pub dt: f32,
    /// Rain added every step, metres of water per second.
    pub rain_rate: f32,
    /// Evaporation per second (fraction of depth).
    pub evaporation: f32,
    /// Sediment capacity constant Kc.
    pub capacity_k: f32,
    /// Dissolving constant Ks.
    pub dissolve_k: f32,
    /// Deposition constant Kd.
    pub deposit_k: f32,
    /// Floor for sin(tilt) so flat water still carries some sediment.
    pub min_tilt: f32,
    /// Suspended sediment may not exceed this fraction of the water depth; the excess settles
    /// back onto the terrain the same step (keeps thin films from carrying metres of rock).
    pub max_concentration: f32,
}

impl Default for HydraulicParams {
    fn default() -> Self {
        Self {
            iterations: 100,
            dt: 0.02,
            rain_rate: 0.012,
            evaporation: 0.015,
            capacity_k: 1.0,
            dissolve_k: 0.5,
            deposit_k: 1.0,
            min_tilt: 0.1,
            max_concentration: 0.3,
        }
    }
}

/// Layers produced by hydraulic erosion.
#[derive(Debug)]
pub struct HydraulicResult {
    /// Eroded terrain (metres).
    pub height: Heightfield,
    /// Remaining water depth (metres).
    pub water: Heightfield,
    /// Suspended sediment (metres).
    pub sediment: Heightfield,
    /// Loose material on the surface (metres): what the `soil` input brought plus every deposit,
    /// less what the flow picked up again — the top of the material stack (without a `soil`
    /// input: the deposits alone).
    pub soil: Heightfield,
}

/// Pipe directions: left, right, top (−y), bottom (+y); `OPPOSITE[k]` is the neighbour's pipe back to us.
const PIPES: [(i64, i64); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
const OPPOSITE: [usize; 4] = [1, 0, 3, 2];

/// Hydraulic erosion with the virtual-pipe shallow-water model. Closed boundary (no flow leaves
/// the field), so water changes only by rain and evaporation and `terrain + suspended sediment`
/// is conserved exactly up to float rounding; every pass gathers from the previous arrays, so
/// the result is independent of thread count.
///
/// ponytail: 4 whole-field passes per step on flat arrays (~40 B/cell scratch); donor-cell
/// sediment transport (conservative) instead of semi-Lagrangian; GPU twin and open-boundary
/// drains come with `jd-gpu`.
pub fn hydraulic_erode(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &HydraulicParams,
    cancel: Option<&AtomicBool>,
) -> JdResult<HydraulicResult> {
    hydraulic_erode_with(h, spacing_m, p, None, None, None, cancel)
}

/// [`hydraulic_erode`] with an optional per-cell `hardness` field (0..1): the dissolving constant
/// drops to a tenth on hard rock, so water cuts shale and deposits on caprock as before — and an
/// optional `rain` field (0..1, e.g. `orographic_rain`) that scales `rain_rate` per cell, and an
/// optional `drains` mask (> 0.5): water and its sediment leave the field there (a river mouth,
/// a sink), instead of ponding against the closed boundary.
#[allow(clippy::too_many_arguments)]
pub fn hydraulic_erode_with(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &HydraulicParams,
    hardness: Option<&Heightfield>,
    rain: Option<&Heightfield>,
    drains: Option<&Heightfield>,
    cancel: Option<&AtomicBool>,
) -> JdResult<HydraulicResult> {
    hydraulic_erode_layered(h, spacing_m, p, hardness, rain, drains, None, cancel)
}

/// [`hydraulic_erode_with`] plus a material stack (Šťava et al. 2008 in spirit): an optional
/// `soil` field is the thickness of loose material lying on the rock (metres, ≥ 0). The flow
/// picks up soil at the full dissolving rate before it touches the rock underneath (which keeps
/// the `hardness` rate), every deposit lands on the soil layer, and the layer comes back as
/// `HydraulicResult::soil`. Without `soil` the layer rule is off: `height`, `water` and
/// `sediment` are exactly what they always were (saved projects keep their numbers) and `soil`
/// just records the deposits.
#[allow(clippy::too_many_arguments)]
pub fn hydraulic_erode_layered(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &HydraulicParams,
    hardness: Option<&Heightfield>,
    rain: Option<&Heightfield>,
    drains: Option<&Heightfield>,
    soil: Option<&Heightfield>,
    cancel: Option<&AtomicBool>,
) -> JdResult<HydraulicResult> {
    for (name, f) in [
        ("hardness", hardness),
        ("rain", rain),
        ("drains", drains),
        ("soil", soil),
    ] {
        if let Some(k) = f
            && (k.width(), k.height()) != (h.width(), h.height())
        {
            return Err(JdError::invalid(name, "field size differs from `in`"));
        }
    }
    let drain: Option<Vec<bool>> = drains.map(|d| d.to_vec().iter().map(|v| *v > 0.5).collect());
    let rain_scale: Option<Vec<f32>> =
        rain.map(|r| r.to_vec().iter().map(|v| v.clamp(0.0, 1.0)).collect());
    let soft: Option<Vec<f32>> = hardness.map(|k| {
        k.to_vec()
            .iter()
            .map(|v| 1.0 - 0.9 * v.clamp(0.0, 1.0))
            .collect()
    });
    check_spacing(spacing_m)?;
    if !(p.dt > 0.0 && p.dt <= 1.0) {
        return Err(JdError::invalid("dt", "must be within (0, 1]"));
    }
    if !(p.max_concentration > 0.0 && p.max_concentration.is_finite()) {
        return Err(JdError::invalid("max_concentration", "must be positive"));
    }
    for (name, v) in [
        ("rain_rate", p.rain_rate),
        ("evaporation", p.evaporation),
        ("capacity_k", p.capacity_k),
        ("dissolve_k", p.dissolve_k),
        ("deposit_k", p.deposit_k),
        ("min_tilt", p.min_tilt),
    ] {
        if !(v >= 0.0 && v.is_finite()) {
            return Err(JdError::invalid(name, "must be finite and ≥ 0"));
        }
    }
    if p.evaporation * p.dt >= 1.0 || p.dissolve_k * p.dt > 1.0 || p.deposit_k * p.dt > 1.0 {
        return Err(JdError::invalid(
            "dt",
            "evaporation·dt must be < 1 and dissolve_k·dt, deposit_k·dt ≤ 1",
        ));
    }
    let (w, hh) = (h.width() as i64, h.height() as i64);
    let n = (w * hh) as usize;
    let area = spacing_m[0] * spacing_m[1];
    let (lx, ly, g) = (spacing_m[0], spacing_m[1], 9.81f32);
    // Pipe length per direction: left/right span x, top/bottom span y.
    let pipe_len = [lx, lx, ly, ly];
    let mut terrain = h.to_vec();
    let mut water = vec![0.0f32; n];
    let mut sediment = vec![0.0f32; n];
    // The layer rule (loose first) only runs with a `soil` field wired: without one the layer
    // merely records deposits and the erosion is what it always was, hardness or not.
    let layered = soil.is_some();
    let mut loose: Vec<f32> = match soil {
        Some(f) => {
            let v = f.to_vec();
            if v.iter().any(|x| !x.is_finite() || *x < 0.0) {
                return Err(JdError::invalid("soil", "must be finite and ≥ 0"));
            }
            v
        }
        None => vec![0.0f32; n],
    };
    let mut flux = vec![[0.0f32; 4]; n];
    let inside = |x: i64, y: i64| x >= 0 && y >= 0 && x < w && y < hh;
    let idx = |x: i64, y: i64| (y * w + x) as usize;

    for _ in 0..p.iterations {
        check_cancel(cancel)?;
        // 1. Rain.
        let water_pre: Vec<f32> = water
            .iter()
            .enumerate()
            .map(|(i, d)| d + p.rain_rate * p.dt * rain_scale.as_ref().map_or(1.0, |s| s[i]))
            .collect();
        // 2. Outflow flux per pipe, scaled so a cell never sends more water than it holds.
        let mut new_flux = vec![[0.0f32; 4]; n];
        new_flux
            .par_chunks_mut(w as usize)
            .enumerate()
            .for_each(|(y, row)| {
                for (x, f) in row.iter_mut().enumerate() {
                    let (xi, yi) = (x as i64, y as i64);
                    let i = idx(xi, yi);
                    let surface = terrain[i] + water_pre[i];
                    let mut total = 0.0f32;
                    for (k, (dx, dy)) in PIPES.iter().enumerate() {
                        let (nx, ny) = (xi + dx, yi + dy);
                        if !inside(nx, ny) {
                            continue;
                        }
                        let j = idx(nx, ny);
                        let dh = surface - (terrain[j] + water_pre[j]);
                        let v = (flux[i][k] + p.dt * area * g * dh / pipe_len[k]).max(0.0);
                        f[k] = v;
                        total += v;
                    }
                    let volume = water_pre[i] * area;
                    if total * p.dt > volume && total > 0.0 {
                        let k = volume / (total * p.dt);
                        for v in f.iter_mut() {
                            *v *= k;
                        }
                    }
                }
            });
        flux = new_flux;
        // 3. Water update, velocity, erosion/deposition, and conservative sediment transport.
        let mut terrain2 = vec![0.0f32; n];
        let mut water2 = vec![0.0f32; n];
        let mut sediment2 = vec![0.0f32; n];
        let mut loose2 = vec![0.0f32; n];
        terrain2
            .par_chunks_mut(w as usize)
            .zip(water2.par_chunks_mut(w as usize))
            .zip(sediment2.par_chunks_mut(w as usize))
            .zip(loose2.par_chunks_mut(w as usize))
            .enumerate()
            .for_each(|(y, (((trow, wrow), srow), lrow))| {
                for x in 0..w as usize {
                    let (xi, yi) = (x as i64, y as i64);
                    let i = idx(xi, yi);
                    let out_total: f32 = flux[i].iter().sum();
                    let mut in_total = 0.0f32;
                    let mut sed_in = 0.0f32;
                    let mut in_lr = 0.0f32; // inflow from left minus inflow from right (x momentum)
                    let mut in_tb = 0.0f32;
                    for (k, (dx, dy)) in PIPES.iter().enumerate() {
                        let (nx, ny) = (xi + dx, yi + dy);
                        if !inside(nx, ny) {
                            continue;
                        }
                        let j = idx(nx, ny);
                        let f_in = flux[j][OPPOSITE[k]];
                        in_total += f_in;
                        // Donor-cell transport: the neighbour's sediment travels at its concentration.
                        if water_pre[j] > 0.0 {
                            sed_in += sediment[j] / water_pre[j] * f_in * p.dt / area;
                        }
                        match k {
                            0 => in_lr += f_in,
                            1 => in_lr -= f_in,
                            2 => in_tb += f_in,
                            _ => in_tb -= f_in,
                        }
                    }
                    let d_new = (water_pre[i] + (in_total - out_total) * p.dt / area).max(0.0);
                    // Velocity from net horizontal transport through the cell (Mei eq. 8–9).
                    let d_bar = 0.5 * (water_pre[i] + d_new);
                    let vel = if d_bar > 1e-6 {
                        let vx = 0.5 * (in_lr + flux[i][1] - flux[i][0]) / (ly * d_bar);
                        let vy = 0.5 * (in_tb + flux[i][3] - flux[i][2]) / (lx * d_bar);
                        (vx * vx + vy * vy).sqrt()
                    } else {
                        0.0
                    };
                    // Local tilt from the terrain gradient.
                    let (x0, x1) = ((xi - 1).max(0), (xi + 1).min(w - 1));
                    let (y0, y1) = ((yi - 1).max(0), (yi + 1).min(hh - 1));
                    let gx = (terrain[idx(x1, yi)] - terrain[idx(x0, yi)])
                        / ((x1 - x0).max(1) as f32 * lx);
                    let gy = (terrain[idx(xi, y1)] - terrain[idx(xi, y0)])
                        / ((y1 - y0).max(1) as f32 * ly);
                    let grad = (gx * gx + gy * gy).sqrt();
                    let sin_tilt = (grad / (1.0 + grad * grad).sqrt()).max(p.min_tilt);
                    // Capacity scales with the water column (volume concentration × depth): with
                    // metre-scale heights the pipe velocities are tens of m/s, and without the depth
                    // factor a 2 cm rain film would carry tens of metres of rock.
                    let capacity = p.capacity_k * sin_tilt * vel * d_bar;
                    // Sediment that stays: own minus what left with the outflow, plus what arrived.
                    let sed_out = if water_pre[i] > 0.0 {
                        sediment[i] / water_pre[i] * out_total * p.dt / area
                    } else {
                        0.0
                    };
                    let mut s = (sediment[i] - sed_out).max(0.0) + sed_in;
                    let mut t = terrain[i];
                    let mut l = loose[i];
                    if capacity > s {
                        // Loose material first, at the full rate; the rock underneath at its own.
                        let demand = p.dissolve_k * (capacity - s) * p.dt;
                        let from_soil = if layered { demand.min(l) } else { 0.0 };
                        let delta =
                            from_soil + (demand - from_soil) * soft.as_ref().map_or(1.0, |f| f[i]);
                        l -= from_soil;
                        t -= delta;
                        s += delta;
                    } else {
                        let delta = (p.deposit_k * (s - capacity) * p.dt).min(s);
                        t += delta;
                        l += delta;
                        s -= delta;
                    }
                    // 4. Evaporation.
                    let d_final = d_new * (1.0 - p.evaporation * p.dt);
                    // Concentration cap against the water that remains: the excess settles now.
                    let cap = p.max_concentration * d_final;
                    if s > cap {
                        t += s - cap;
                        l += s - cap;
                        s = cap;
                    }
                    // 5. Drains: the water and what it carries leave the field here.
                    let drained = drain.as_ref().is_some_and(|d| d[i]);
                    trow[x] = t;
                    lrow[x] = l;
                    srow[x] = if drained { 0.0 } else { s };
                    wrow[x] = if drained { 0.0 } else { d_final };
                }
            });
        terrain = terrain2;
        water = water2;
        sediment = sediment2;
        loose = loose2;
    }
    let build = |v: &[f32]| -> JdResult<Heightfield> {
        let mut f = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
        f.fill_with(|x, y| v[(y as usize) * (w as usize) + x as usize]);
        Ok(f)
    };
    Ok(HydraulicResult {
        height: build(&terrain)?,
        water: build(&water)?,
        sediment: build(&sediment)?,
        soil: build(&loose)?,
    })
}

/// Slope mask in 0..1: central-difference gradient angle divided by `max_deg`, clamped.
pub fn slope_mask(h: &Heightfield, spacing_m: [f32; 2], max_deg: f32) -> JdResult<Heightfield> {
    if !(0.0 < max_deg && max_deg <= 90.0) {
        return Err(JdError::invalid("max_deg", "must be within (0, 90]"));
    }
    check_spacing(spacing_m)?;
    let (w, hh) = (h.width(), h.height());
    let mut out = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    out.fill_with(|x, y| {
        let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
        let (y0, y1) = (y.saturating_sub(1), (y + 1).min(hh - 1));
        // `.max(1)` keeps 1-pixel fields (deep LODs) finite instead of 0/0.
        let dx = (h.get(x1, y) - h.get(x0, y)) / ((x1 - x0).max(1) as f32 * spacing_m[0]);
        let dy = (h.get(x, y1) - h.get(x, y0)) / ((y1 - y0).max(1) as f32 * spacing_m[1]);
        ((dx * dx + dy * dy).sqrt().atan().to_degrees() / max_deg).clamp(0.0, 1.0)
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_soil_goes_before_the_rock_and_deposits_land_on_the_soil_layer() {
        // A valley with 0.5 m of soil over rock as hard as it gets: the flow strips the soil
        // where it runs and the rock below barely moves; deposits downstream thicken the layer.
        let mut valley = Heightfield::new(48, 32, 64, 0).expect("v");
        valley.fill_with(|x, y| 2.0 * (y as f32) + 0.05 * (x as f32 - 24.0).abs());
        let mut rock = Heightfield::new(48, 32, 64, 0).expect("k");
        rock.fill_with(|_, _| 1.0);
        let mut soil = Heightfield::new(48, 32, 64, 0).expect("s");
        soil.fill_with(|_, _| 0.5);
        let p = HydraulicParams {
            iterations: 60,
            rain_rate: 0.02,
            ..HydraulicParams::default()
        };
        let bare = hydraulic_erode_with(&valley, [10.0, 10.0], &p, Some(&rock), None, None, None)
            .expect("bare");
        let layered = hydraulic_erode_layered(
            &valley,
            [10.0, 10.0],
            &p,
            Some(&rock),
            None,
            None,
            Some(&soil),
            None,
        )
        .expect("layered");
        let cut = |r: &HydraulicResult| {
            (0..32)
                .flat_map(|y| (0..48).map(move |x| (x, y)))
                .map(|(x, y)| (valley.get(x, y) - r.height.get(x, y)).max(0.0))
                .sum::<f32>()
        };
        // Both runs are capacity-limited (the flow carries what it can either way), so the layer
        // shows in where the material comes from, not in the total: never less with soil on.
        assert!(
            cut(&layered) >= cut(&bare) - 1e-3,
            "{} vs {}",
            cut(&layered),
            cut(&bare)
        );
        // Where the layer is not exhausted every metre the terrain loses or gains is the layer's
        // (the rock never moves): Δheight == Δsoil cell by cell, to the f32 rounding of 60 steps
        // on heights up to 64 m.
        let (mut stripped, mut thickened, mut off_layer) = (0, 0, 0);
        for y in 0..32 {
            for x in 0..48 {
                let d_soil = layered.soil.get(x, y) - 0.5;
                let d_height = layered.height.get(x, y) - valley.get(x, y);
                if d_soil < -1e-4 {
                    stripped += 1;
                }
                if d_soil > 1e-4 {
                    thickened += 1;
                }
                if layered.soil.get(x, y) > 0.0 && (d_height - d_soil).abs() > 5e-3 {
                    off_layer += 1;
                }
            }
        }
        // (deposition needs the flow to slow below its load; on this straight slope it may not
        // happen within 60 steps, so only the stripping is required)
        assert!(stripped > 50, "stripped {stripped} thickened {thickened}");
        assert_eq!(
            off_layer, 0,
            "the rock under an unexhausted layer never moves"
        );
        // Mass: terrain + sediment conserved (closed boundary) with the layer on.
        let total = |r: &HydraulicResult| {
            (0..32)
                .flat_map(|y| (0..48).map(move |x| (x, y)))
                .map(|(x, y)| (r.height.get(x, y) + r.sediment.get(x, y)) as f64)
                .sum::<f64>()
        };
        let before: f64 = (0..32)
            .flat_map(|y| (0..48).map(move |x| (x, y)))
            .map(|(x, y)| valley.get(x, y) as f64)
            .sum();
        assert!(
            (total(&layered) - before).abs() < 1e-2,
            "{} vs {before}",
            total(&layered)
        );
        let mut bad = soil.clone();
        bad.set(0, 0, -1.0);
        assert!(
            hydraulic_erode_layered(
                &valley,
                [10.0, 10.0],
                &p,
                None,
                None,
                None,
                Some(&bad),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn hard_rock_dissolves_a_tenth_as_fast_under_rain() {
        let mut valley = Heightfield::new(48, 32, 64, 0).expect("v");
        valley.fill_with(|x, y| 200.0 + y as f32 * 2.0 + (x as f32 - 24.0).abs() * 1.5);
        // Enough rain and an open concentration cap that dissolution, not the thin-film cap, sets the pace.
        let p = HydraulicParams {
            iterations: 60,
            rain_rate: 0.2,
            max_concentration: 10.0,
            ..HydraulicParams::default()
        };
        let soft = hydraulic_erode(&valley, [10.0, 10.0], &p, None).expect("soft");
        let mut cap = Heightfield::new(48, 32, 64, 0).expect("k");
        cap.fill_with(|_, _| 1.0);
        let hard = hydraulic_erode_with(&valley, [10.0, 10.0], &p, Some(&cap), None, None, None)
            .expect("hard");
        let moved = |f: &Heightfield| {
            (0..32)
                .flat_map(|y| (0..48).map(move |x| (x, y)))
                .map(|(x, y)| (valley.get(x, y) - f.get(x, y)).abs())
                .sum::<f32>()
        };
        let (ms, mh) = (moved(&soft.height), moved(&hard.height));
        assert!(ms > 0.0 && mh < ms * 0.5, "hard {mh} vs soft {ms}");
        let mut zero = Heightfield::new(48, 32, 64, 0).expect("z");
        zero.fill_with(|_, _| 0.0);
        let same = hydraulic_erode_with(&valley, [10.0, 10.0], &p, Some(&zero), None, None, None)
            .expect("zero");
        assert_eq!(same.height.to_vec(), soft.height.to_vec());
    }

    #[test]
    fn hard_rock_keeps_its_cliff_while_soft_rock_slumps_to_talus() {
        // A 40 m step in the middle of the field; the whole field is one rock type per run.
        let mut h = Heightfield::new(64, 16, 64, 0).expect("hf");
        h.fill_with(|x, _| if x < 32 { 140.0 } else { 100.0 });
        let steepest = |f: &Heightfield| {
            (1..64)
                .map(|x| {
                    ((f.get(x - 1, 8) - f.get(x, 8)).abs() / 10.0)
                        .atan()
                        .to_degrees()
                })
                .fold(0.0f32, f32::max)
        };
        let soft = thermal_erode(&h, [10.0, 10.0], 35.0, 60, 0.2, None).expect("soft");
        let mut cap = Heightfield::new(64, 16, 64, 0).expect("k");
        cap.fill_with(|_, _| 1.0);
        let hard =
            thermal_erode_with(&h, [10.0, 10.0], 35.0, 60, 0.2, Some(&cap), None).expect("hard");
        assert!(
            steepest(&soft) < 40.0,
            "soft step slumped to the 35° talus: {}",
            steepest(&soft)
        );
        assert!(
            steepest(&hard) > 70.0,
            "hard step still a cliff: {}",
            steepest(&hard)
        );
        // Mass is conserved either way, and a zero-hardness field equals the plain kernel.
        let sum = |f: &Heightfield| f.to_vec().iter().map(|v| *v as f64).sum::<f64>();
        assert!((sum(&soft) - sum(&h)).abs() < 1e-2 && (sum(&hard) - sum(&h)).abs() < 1e-2);
        let mut zero = Heightfield::new(64, 16, 64, 0).expect("z");
        zero.fill_with(|_, _| 0.0);
        let same =
            thermal_erode_with(&h, [10.0, 10.0], 35.0, 60, 0.2, Some(&zero), None).expect("zero");
        assert_eq!(same.to_vec(), soft.to_vec());
        // Stream power: a hard bed cuts a tenth as deep as a soft one.
        let mut valley = Heightfield::new(48, 32, 64, 0).expect("v");
        valley.fill_with(|x, y| 200.0 + y as f32 * 2.0 + (x as f32 - 24.0).abs() * 1.5);
        let sp = StreamPowerParams {
            k: 5e-4,
            m: 0.5,
            uplift_m: 0.0,
            iterations: 20,
            method: crate::flow::FlowMethod::D8,
        };
        let cut_soft = stream_power_erode(&valley, [10.0, 10.0], &sp, None).expect("sp");
        let mut cap2 = Heightfield::new(48, 32, 64, 0).expect("k");
        cap2.fill_with(|_, _| 1.0);
        let cut_hard = stream_power_erode_with(&valley, [10.0, 10.0], &sp, Some(&cap2), None)
            .expect("sp hard");
        let depth = |f: &Heightfield| {
            (0..32)
                .map(|y| valley.get(24, y) - f.get(24, y))
                .fold(0.0f32, f32::max)
        };
        assert!(
            depth(&cut_hard) < depth(&cut_soft) * 0.5,
            "hard {} vs soft {}",
            depth(&cut_hard),
            depth(&cut_soft)
        );
    }

    #[test]
    fn stream_power_incises_channels_keeps_slopes_non_negative_and_uplifts_uniformly() {
        // Tilted plane with a central valley: the valley line collects the most area.
        let mut h = Heightfield::new(48, 32, 64, 0).expect("hf");
        h.fill_with(|x, y| 200.0 + y as f32 * 2.0 + (x as f32 - 24.0).abs() * 1.5);
        let p = StreamPowerParams {
            k: 5e-4,
            m: 0.5,
            uplift_m: 0.0,
            iterations: 10,
            method: crate::flow::FlowMethod::D8,
        };
        let out = stream_power_erode(&h, [10.0, 10.0], &p, None).expect("erode");
        let valley_drop = h.get(24, 12) - out.get(24, 12);
        let slope_drop = h.get(4, 12) - out.get(4, 12);
        assert!(valley_drop > 0.0, "valley incised: {valley_drop}");
        assert!(
            valley_drop > 5.0 * slope_drop.max(1e-6),
            "channels erode much faster than hillslopes: {valley_drop} vs {slope_drop}"
        );
        // Implicit scheme: every cell stays at or above its steepest-descent receiver.
        let v = out.to_vec();
        let recv = crate::flow::d8_receivers(&h.to_vec(), 48, 32, [10.0, 10.0]);
        for (i, r) in recv.iter().enumerate() {
            if let Some(j) = r {
                assert!(
                    v[i] >= v[*j as usize] - 1e-4,
                    "cell {i} fell below its receiver"
                );
            }
        }
        // k = 0 with uplift raises the whole field by exactly uplift × iterations.
        let lifted = stream_power_erode(
            &h,
            [10.0, 10.0],
            &StreamPowerParams {
                k: 0.0,
                uplift_m: 0.5,
                iterations: 4,
                ..p.clone()
            },
            None,
        )
        .expect("uplift");
        assert!((lifted.get(10, 10) - h.get(10, 10) - 2.0).abs() < 1e-4);
        // Deterministic; invalid params and cancellation are refused.
        let again = stream_power_erode(&h, [10.0, 10.0], &p, None).expect("again");
        assert_eq!(out.content_hash(), again.content_hash());
        assert!(
            stream_power_erode(
                &h,
                [10.0, 10.0],
                &StreamPowerParams {
                    m: 3.0,
                    ..p.clone()
                },
                None
            )
            .is_err()
        );
        let flag = AtomicBool::new(true);
        assert_eq!(
            stream_power_erode(&h, [10.0, 10.0], &p, Some(&flag))
                .err()
                .map(|e| e.code),
            Some(jd_types::ErrorCode::E_CANCELLED)
        );
    }

    fn sum(h: &Heightfield) -> f64 {
        h.to_vec().iter().map(|&v| v as f64).sum()
    }

    #[test]
    fn tiled_thermal_equals_the_whole_field_result_when_the_halo_covers_the_steps() {
        // A rough 200 × 150 field with a hardness band; 64-px tiles, 12 steps → 12-cell halos.
        let mut h = Heightfield::new(200, 150, 64, 4).expect("hf");
        h.fill_with(|x, y| {
            let (fx, fy) = (x as f32, y as f32);
            let spike = if (x, y) == (90, 70) { 300.0 } else { 0.0 };
            spike
                + 40.0 * ((fx * 0.3).sin() * (fy * 0.2).cos())
                + ((fx * 1.7 + fy * 0.9).sin() * 9.0)
        });
        let mut k = Heightfield::new(200, 150, 64, 4).expect("k");
        k.fill_with(|x, _| if (120..160).contains(&x) { 0.9 } else { 0.0 });
        let full =
            thermal_erode_with(&h, [10.0, 10.0], 35.0, 12, 0.2, Some(&k), None).expect("full");
        let tiled = thermal_erode_tiled(&h, [10.0, 10.0], 35.0, 12, 0.2, Some(&k), 64, None)
            .expect("tiled");
        let worst = full
            .to_vec()
            .iter()
            .zip(&tiled.to_vec())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst <= 1e-4, "tiled vs whole field: {worst} m");
        assert_eq!(
            full.content_hash(),
            tiled.content_hash(),
            "bit-identical interiors"
        );
        assert!(thermal_erode_tiled(&h, [10.0, 10.0], 35.0, 12, 0.2, None, 8, None).is_err());
    }

    #[test]
    fn thermal_conserves_mass_and_flattens_a_spike() {
        let mut h = Heightfield::new(64, 48, 32, 4).expect("hf");
        h.fill_with(|x, y| if (x, y) == (30, 20) { 500.0 } else { 0.0 });
        let before = sum(&h);
        let after = thermal_erode(&h, [10.0, 10.0], 35.0, 40, 0.25, None).expect("erode");
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
            thermal_erode(&h, [10.0, 10.0], 35.0, 40, 0.25, None)
                .expect("again")
                .content_hash(),
            after.content_hash()
        );
    }

    #[test]
    fn thermal_leaves_gentle_terrain_alone_and_rejects_bad_params() {
        let mut h = Heightfield::new(40, 40, 32, 4).expect("hf");
        h.fill_with(|x, _| x as f32 * 0.1); // 0.1 m rise per 10 m pixel ≈ 0.6°, far below talus
        let after = thermal_erode(&h, [10.0, 10.0], 35.0, 5, 0.25, None).expect("erode");
        assert_eq!(after.content_hash(), h.content_hash());
        assert!(thermal_erode(&h, [10.0, 10.0], 95.0, 1, 0.25, None).is_err());
        assert!(thermal_erode(&h, [10.0, 10.0], 35.0, 1, 0.0, None).is_err());
        assert!(
            thermal_erode(&h, [10.0, 10.0], 35.0, 1, 0.5, None).is_err(),
            "above the stability bound"
        );
        assert!(thermal_erode(&h, [0.0, 10.0], 35.0, 1, 0.25, None).is_err());
        // A one-cell pit in a plateau fills from all 8 sides in one step but never rises above the
        // plateau it sat in (the THERMAL_MAX_RATE bound); at 0.5 it would overshoot to ~198 m.
        let mut pit = Heightfield::new(16, 16, 16, 0).expect("hf");
        pit.fill_with(|x, y| if (x, y) == (8, 8) { 0.0 } else { 100.0 });
        let after =
            thermal_erode(&pit, [1.0, 1.0], 35.0, 1, THERMAL_MAX_RATE, None).expect("erode");
        assert!(
            after.get(8, 8) > 50.0 && after.get(8, 8) <= 100.0 + 1e-3,
            "pit {}",
            after.get(8, 8)
        );
        assert!((sum(&after) - sum(&pit)).abs() < 1e-2);
        // Anisotropic pixels: a y-ramp just below the talus angle at 2 m y-spacing stays put only
        // when the y spacing is honoured.
        let mut ramp = Heightfield::new(16, 16, 16, 0).expect("hf");
        ramp.fill_with(|_, y| y as f32 * 2.0 * 30f32.to_radians().tan() * 0.99);
        let kept =
            thermal_erode(&ramp, [1.0, 2.0], 30.0, 3, THERMAL_MAX_RATE, None).expect("erode");
        assert_eq!(
            kept.content_hash(),
            ramp.content_hash(),
            "below talus along y with 2 m spacing"
        );
        let moved =
            thermal_erode(&ramp, [1.0, 1.0], 30.0, 3, THERMAL_MAX_RATE, None).expect("erode");
        assert_ne!(
            moved.content_hash(),
            ramp.content_hash(),
            "steeper than talus if y spacing were 1 m"
        );
    }

    #[test]
    fn drains_let_water_and_sediment_leave_at_the_marked_cells() {
        // The valley of the hardness test; the south row is the river mouth.
        let mut valley = Heightfield::new(48, 32, 64, 0).expect("v");
        valley.fill_with(|x, y| 200.0 + y as f32 * 2.0 + (x as f32 - 24.0).abs() * 1.5);
        let mut mouth = Heightfield::new(48, 32, 64, 0).expect("m");
        mouth.fill_with(|_, y| if y == 0 { 1.0 } else { 0.0 });
        let p = HydraulicParams {
            iterations: 80,
            rain_rate: 0.2,
            ..Default::default()
        };
        let closed = hydraulic_erode_with(&valley, [10.0, 10.0], &p, None, None, None, None)
            .expect("closed");
        let open = hydraulic_erode_with(&valley, [10.0, 10.0], &p, None, None, Some(&mouth), None)
            .expect("open");
        let sum = |f: &Heightfield| f.to_vec().iter().map(|v| *v as f64).sum::<f64>();
        assert!(
            sum(&open.water) < 0.9 * sum(&closed.water),
            "water leaves: {} vs {}",
            sum(&open.water),
            sum(&closed.water)
        );
        assert!(
            (0..48).all(|x| open.water.get(x, 0) == 0.0 && open.sediment.get(x, 0) == 0.0),
            "the mouth row holds nothing"
        );
        assert!(
            sum(&open.height) + sum(&open.sediment) < sum(&closed.height) + sum(&closed.sediment),
            "sediment left with the water"
        );
        assert!(
            hydraulic_erode_with(
                &valley,
                [10.0, 10.0],
                &p,
                None,
                None,
                Some(&Heightfield::new(4, 4, 64, 0).expect("s")),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn hydraulic_conserves_terrain_plus_sediment_and_water_budget() {
        let mut h = Heightfield::new(64, 48, 32, 4).expect("hf");
        h.fill_with(|x, y| {
            200.0 * ((x as f32 * 0.2).sin() + (y as f32 * 0.15).cos()) + (x as f32) * 1.5
        });
        let p = HydraulicParams {
            iterations: 60,
            ..Default::default()
        };
        let r = hydraulic_erode(&h, [4.0, 4.0], &p, None).expect("erode");
        let before = sum(&h);
        let after = sum(&r.height) + sum(&r.sediment);
        assert!(
            (after - before).abs() < 1e-3 * before.abs().max(1.0),
            "terrain+sediment drifted: {before} → {after}"
        );
        // Closed boundary: water = Σ over steps of (rain − evaporation), i.e. positive and bounded by total rain.
        let cells = 64.0 * 48.0;
        let total_rain = cells * (p.rain_rate * p.dt) as f64 * p.iterations as f64;
        let w = sum(&r.water);
        assert!(
            w > 0.0 && w <= total_rain + 1e-6,
            "water {w} vs rain {total_rain}"
        );
        assert_ne!(
            r.height.content_hash(),
            h.content_hash(),
            "terrain must change"
        );
        for f in [&r.height, &r.water, &r.sediment] {
            assert_eq!(f.stats().non_finite, 0);
            assert!(f.stats().min >= -1e3);
        }
        assert!(r.water.stats().min >= 0.0 && r.sediment.stats().min >= 0.0);
        // Suspended sediment never exceeds the concentration cap anywhere.
        let (wv, sv) = (r.water.to_vec(), r.sediment.to_vec());
        assert!(
            wv.iter()
                .zip(&sv)
                .all(|(d, s)| *s <= p.max_concentration * d + 1e-5),
            "sediment within cap"
        );
        // Deterministic.
        assert_eq!(
            hydraulic_erode(&h, [4.0, 4.0], &p, None)
                .expect("again")
                .height
                .content_hash(),
            r.height.content_hash()
        );
        // Rejects unstable steps.
        assert!(
            hydraulic_erode(
                &h,
                [4.0, 4.0],
                &HydraulicParams {
                    dt: 2.0,
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
        assert!(
            hydraulic_erode(
                &h,
                [4.0, 4.0],
                &HydraulicParams {
                    evaporation: 100.0,
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
    }

    #[test]
    fn kernels_stop_when_the_cancel_flag_is_set() {
        let mut h = Heightfield::new(16, 16, 16, 0).expect("hf");
        h.fill_with(|x, _| x as f32 * 30.0);
        let flag = AtomicBool::new(true);
        let e = thermal_erode(&h, [1.0, 1.0], 30.0, 10, THERMAL_MAX_RATE, Some(&flag))
            .expect_err("cancelled");
        assert_eq!(e.code, jd_types::ErrorCode::E_CANCELLED);
        let e = hydraulic_erode(&h, [1.0, 1.0], &HydraulicParams::default(), Some(&flag))
            .expect_err("cancelled");
        assert_eq!(e.code, jd_types::ErrorCode::E_CANCELLED);
        let clear = AtomicBool::new(false);
        assert!(thermal_erode(&h, [1.0, 1.0], 30.0, 2, THERMAL_MAX_RATE, Some(&clear)).is_ok());
    }

    #[test]
    fn slope_of_a_plane_is_constant() {
        let mut h = Heightfield::new(50, 30, 32, 4).expect("hf");
        h.fill_with(|x, _| x as f32 * 10.0); // rise 10 m per 10 m pixel = 45°
        let m = slope_mask(&h, [10.0, 10.0], 90.0).expect("slope");
        for (x, y) in [(1u32, 1u32), (25, 15), (48, 28)] {
            assert!(
                (m.get(x, y) - 0.5).abs() < 1e-4,
                "({x},{y}) = {}",
                m.get(x, y)
            );
        }
        let flat = Heightfield::new(8, 8, 8, 0).expect("hf");
        assert_eq!(
            slope_mask(&flat, [1.0, 1.0], 45.0)
                .expect("flat")
                .stats()
                .max,
            0.0
        );
        assert!(slope_mask(&h, [10.0, 10.0], 0.0).is_err());
        // Anisotropic spacing: the same x-ramp over 20 m pixels is half as steep.
        let m2 = slope_mask(&h, [20.0, 10.0], 90.0).expect("slope");
        assert!((m2.get(25, 15) - (0.5f32).atan().to_degrees() / 90.0).abs() < 1e-4);
        // One-pixel-wide fields (deep LODs) stay finite.
        let mut thin = Heightfield::new(1, 8, 8, 0).expect("hf");
        thin.fill_with(|_, y| y as f32);
        assert_eq!(
            slope_mask(&thin, [1.0, 1.0], 90.0)
                .expect("thin")
                .stats()
                .non_finite,
            0
        );
    }
}

/// Stream-power (detachment-limited fluvial) erosion parameters.
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamPowerParams {
    /// Erodibility K: metres of incision per step for unit drainage area and unit slope
    /// (`dz = -K·A^m·S`, A in m², S dimensionless). 1e-3 carves visible valleys on a 5 km world
    /// in 20 steps; 1e-4 is subtle, 1e-2 is a badlands.
    #[serde(default = "sp_k")]
    pub k: f32,
    /// Area exponent m (0.3–0.6 in nature; n is fixed at 1 for the implicit solve).
    #[serde(default = "sp_m")]
    pub m: f32,
    /// Uniform uplift per step in metres (0 = none); outlets rise too, so base level is kept.
    #[serde(default)]
    pub uplift_m: f32,
    /// Steps (1..=1000).
    #[serde(default = "sp_iterations")]
    pub iterations: u32,
    /// Flow routing used for the drainage area.
    #[serde(default)]
    pub method: crate::flow::FlowMethod,
}
fn sp_k() -> f32 {
    1e-3
}
fn sp_m() -> f32 {
    0.5
}
fn sp_iterations() -> u32 {
    20
}

/// Stream-power erosion (Howard 1994) with the implicit, unconditionally stable scheme of
/// Braun & Willett 2013: cells are solved from the outlets upstream along steepest descent,
/// `h' = (h + U + f·h'_recv) / (1 + f)` with `f = K·A^m / dist`, so a cell never drops below
/// its receiver and no CFL limit applies. Drainage area comes from [`crate::flow_accumulation_with`]
/// with the chosen method; outlets (no lower neighbour) only receive uplift.
///
/// ponytail: n = 1 (linear in slope) keeps the per-cell solve closed-form; n ≠ 1 needs a Newton
/// iteration per cell. Hillslope diffusion is the thermal node's job.
pub fn stream_power_erode(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &StreamPowerParams,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    stream_power_erode_with(h, spacing_m, p, None, cancel)
}

/// [`stream_power_erode`] with an optional per-cell `hardness` field (0..1): the erodibility
/// `k` of a cell drops to a tenth where the rock is hard, so channels cut shale and step over cap.
pub fn stream_power_erode_with(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &StreamPowerParams,
    hardness: Option<&Heightfield>,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    if let Some(k) = hardness
        && (k.width(), k.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid("hardness", "field size differs from `in`"));
    }
    if !(p.k >= 0.0 && p.k.is_finite()) {
        return Err(JdError::invalid("k", "must be finite and ≥ 0"));
    }
    if !(0.0..=2.0).contains(&p.m) {
        return Err(JdError::invalid("m", "must be within 0..=2"));
    }
    if !p.uplift_m.is_finite() {
        return Err(JdError::invalid("uplift_m", "must be finite"));
    }
    if !(1..=1000).contains(&p.iterations) {
        return Err(JdError::invalid("iterations", "must be within 1..=1000"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    h.validate_finite()?;
    let (w, hh) = (h.width() as usize, h.height() as usize);
    let cell_area = spacing_m[0] * spacing_m[1];
    let dist = neighbour_dist(spacing_m);
    let hard_vec = hardness.map(|k| k.to_vec());
    let k_at = |i: usize| match &hard_vec {
        Some(k) => p.k * (1.0 - 0.9 * k[i].clamp(0.0, 1.0)),
        None => p.k,
    };
    let mut cur = h.clone();
    let mut order: Vec<u32> = (0..(w * hh) as u32).collect();
    for _ in 0..p.iterations {
        check_cancel(cancel)?;
        let v = cur.to_vec();
        let acc = crate::flow::flow_accumulation_with(&cur, spacing_m, false, p.method)?.to_vec();
        let recv = crate::flow::d8_receivers(&v, w, hh, spacing_m);
        // Ascending height: a receiver is always lower, so it is solved before its donors.
        order.par_sort_unstable_by(|a, b| v[*a as usize].total_cmp(&v[*b as usize]).then(a.cmp(b)));
        let mut next = v.clone();
        for &i in &order {
            let i = i as usize;
            match recv[i] {
                None => next[i] = v[i] + p.uplift_m,
                Some(j) => {
                    let j = j as usize;
                    let (xi, yi) = ((i % w) as i64, (i / w) as i64);
                    let (xj, yj) = ((j % w) as i64, (j / w) as i64);
                    let k = NEIGHBOURS
                        .iter()
                        .position(|(dx, dy)| xi + dx == xj && yi + dy == yj)
                        .unwrap_or(4);
                    let f = k_at(i) * (acc[i] * cell_area).powf(p.m) / dist[k];
                    next[i] = (v[i] + p.uplift_m + f * next[j]) / (1.0 + f);
                }
            }
        }
        cur.fill_with(|x, y| next[y as usize * w + x as usize]);
    }
    check_cancel(cancel)?;
    Ok(cur)
}
