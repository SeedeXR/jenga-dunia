//! Lava flows: vents effuse lava that spreads as a Bingham fluid, cools by radiation and freezes
//! into new ground — the MAGFLOW / GPUFLOW cellular automaton (Vicari et al. 2007; Cappello,
//! Bilotta & Ganci 2022, Appl. Sci. 12 4395), `docs/research/simulation-debris-lava.md`.
//!
//! Each cell holds ground (terrain + frozen lava), molten thickness `h` and temperature `T`. Lava
//! moves from a cell to each of its 8 neighbours whose lava surface is lower, at the steady-state
//! rate of a Bingham fluid on an incline (GPUFLOW eqs. 1–2):
//!
//! `h_cr = S_y √(Δz² + Δx²) / (ρ g (Δz + Δh))`, and when `h > h_cr`:
//! `q = S_y h_cr² Δx / (3 η) · (a³ − 3/2 a² + 1/2)`, `a = h / h_cr`,
//!
//! where Δx is the distance to the neighbour and the flux crosses a face `cell area / Δx` wide.
//! That face width alone spreads lava round on a flat plane (covered area / πr² = 1.01, measured);
//! GPUFLOW's diagonal factor 2⁻⁶ (§2.1.1) belongs to its own discretisation and makes a diamond
//! here (0.71). Heat travels with the lava (volume-weighted mixing) and leaves the surface by
//! radiation `ε(T) σ (T⁴ − T_air⁴)` with GPUFLOW's emissivity fit (eq. 3). A cell whose lava drops
//! below `solid_temp_k` freezes into ground. Viscosity and yield strength rise log-linearly as the
//! lava cools from `vent_temp_k` to `solid_temp_k` (`viscosity_decades`, `yield_decades`) — our
//! simplification of the empirical laws (Giordano & Dingwell 2003; Ishihara et al. 1990) that
//! MAGFLOW uses. Defaults are Etna's (2600 kg/m³, 1150 J/kg/K, 1360 K vent, 1143 K solidus).
//!
//! The step is adaptive: no cell sheds more than a quarter of its lava, no pair moves more than a
//! quarter of what would level them, and a vent adds at most 0.25 m per step.
//!
//! ponytail: a cell freezes whole when it crosses the solidus (MAGFLOW's original rule); GPUFLOW
//! freezes the fraction the heat deficit pays for. Lava leaving the field edge pools there.

use std::sync::atomic::AtomicBool;

use jd_types::{JdError, JdResult};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::sim::check_cancel;

/// One vent.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LavaVent {
    /// Position in layer metres: `[u east, v north]` from the south-west corner.
    pub at: [f32; 2],
    /// Effusion rate, m³/s (> 0).
    pub rate_m3_s: f32,
}

/// Lava settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LavaParams {
    /// Vents; empty → one vent on the highest cell at `vent_rate_m3_s`.
    #[serde(default)]
    pub vents: Vec<LavaVent>,
    /// Effusion rate of the automatic summit vent, m³/s (> 0).
    #[serde(default = "d_rate")]
    pub vent_rate_m3_s: f32,
    /// How long the vents effuse, seconds (> 0).
    #[serde(default = "d_eruption")]
    pub eruption_s: f32,
    /// Simulated time, seconds (≥ 1); lava still molten at the end stays molten (see `heat`).
    #[serde(default = "d_duration")]
    pub duration_s: f32,
    /// Lava density, kg/m³ (> 0).
    #[serde(default = "d_density")]
    pub density_kg_m3: f32,
    /// Specific heat, J/(kg·K) (> 0).
    #[serde(default = "d_cp")]
    pub specific_heat: f32,
    /// Temperature at the vent, kelvin.
    #[serde(default = "d_vent_t")]
    pub vent_temp_k: f32,
    /// Solidification temperature, kelvin (< `vent_temp_k`).
    #[serde(default = "d_solid_t")]
    pub solid_temp_k: f32,
    /// Viscosity at the vent, Pa·s (> 0).
    #[serde(default = "d_visc")]
    pub viscosity_pa_s: f32,
    /// Orders of magnitude the viscosity rises between vent and solidus (0..=8).
    #[serde(default = "d_visc_dec")]
    pub viscosity_decades: f32,
    /// Yield strength at the vent, Pa (> 0).
    #[serde(default = "d_yield")]
    pub yield_pa: f32,
    /// Orders of magnitude the yield strength rises between vent and solidus (0..=8).
    #[serde(default = "d_yield_dec")]
    pub yield_decades: f32,
    /// Share of the bulk-temperature radiation the surface really loses (> 0). A flow radiates
    /// from a crust cooler than its core, so 1 (MAGFLOW's rule) over-cools: on Etna's DEM a
    /// 30 m³/s day-long flank eruption then freezes into a 70 m tower; 0.3 gives a 25 m-thick
    /// flow over twice the area. Higher → shorter, thicker flows.
    #[serde(default = "d_cooling")]
    pub cooling: f32,
    /// Step budget (1..=2 000 000); a run that needs more fails and says how far it got.
    #[serde(default = "d_max_steps")]
    pub max_steps: u32,
}

fn d_rate() -> f32 {
    20.0
}
fn d_eruption() -> f32 {
    6.0 * 3600.0
}
fn d_duration() -> f32 {
    12.0 * 3600.0
}
fn d_density() -> f32 {
    2600.0
}
fn d_cp() -> f32 {
    1150.0
}
fn d_vent_t() -> f32 {
    1360.0
}
fn d_solid_t() -> f32 {
    1143.0
}
fn d_visc() -> f32 {
    1.0e4
}
fn d_visc_dec() -> f32 {
    3.0
}
fn d_yield() -> f32 {
    1.0e3
}
fn d_yield_dec() -> f32 {
    2.0
}
fn d_cooling() -> f32 {
    0.3
}
fn d_max_steps() -> u32 {
    200_000
}

impl Default for LavaParams {
    fn default() -> Self {
        Self {
            vents: Vec::new(),
            vent_rate_m3_s: d_rate(),
            eruption_s: d_eruption(),
            duration_s: d_duration(),
            density_kg_m3: d_density(),
            specific_heat: d_cp(),
            vent_temp_k: d_vent_t(),
            solid_temp_k: d_solid_t(),
            viscosity_pa_s: d_visc(),
            viscosity_decades: d_visc_dec(),
            yield_pa: d_yield(),
            yield_decades: d_yield_dec(),
            cooling: d_cooling(),
            max_steps: d_max_steps(),
        }
    }
}

/// What a lava run produced.
#[derive(Debug)]
pub struct LavaResult {
    /// Terrain with the frozen and still-molten lava on top.
    pub out: Heightfield,
    /// Lava thickness (frozen + molten), metres.
    pub lava: Heightfield,
    /// 0..1 glow of the molten lava: (T − solidus) / (vent − solidus); 0 where frozen or bare.
    pub heat: Heightfield,
    /// Steps taken.
    pub steps: u32,
    /// Seconds simulated (less than `duration_s` when `max_steps` ran out).
    pub simulated_s: f64,
    /// Lava erupted, m³.
    pub erupted_m3: f64,
    /// Lava frozen into ground, m³.
    pub frozen_m3: f64,
}

const NB: [(i64, i64); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];
const G: f32 = 9.81;
const SIGMA: f32 = 5.670_374e-8;
const T_AIR: f32 = 293.15;
/// Fraction of a cell's lava (or of a pair's levelling volume) one step may move.
const CFL: f32 = 0.25;
const MAX_DT: f32 = 60.0;

/// GPUFLOW eq. 3: emissivity of 'a'ā lava as a function of temperature (fit over 773–1373 K).
fn emissivity(t: f32) -> f32 {
    (0.976_716 + 4.088_81e-5 * t - 1.950_62e-7 * t * t).clamp(0.1, 1.0)
}

/// Run the lava flow over `h`.
pub fn lava_flow(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &LavaParams,
    cancel: Option<&AtomicBool>,
) -> JdResult<LavaResult> {
    let pos = |v: f32| v > 0.0 && v.is_finite();
    for (name, v) in [
        ("vent_rate_m3_s", p.vent_rate_m3_s),
        ("eruption_s", p.eruption_s),
        ("density_kg_m3", p.density_kg_m3),
        ("specific_heat", p.specific_heat),
        ("viscosity_pa_s", p.viscosity_pa_s),
        ("yield_pa", p.yield_pa),
        ("cooling", p.cooling),
    ] {
        if !pos(v) {
            return Err(JdError::invalid(name, "must be positive"));
        }
    }
    if !(p.duration_s >= 1.0 && p.duration_s.is_finite()) {
        return Err(JdError::invalid("duration_s", "must be at least 1 s"));
    }
    if !(pos(p.solid_temp_k) && p.solid_temp_k < p.vent_temp_k && p.vent_temp_k.is_finite()) {
        return Err(JdError::invalid(
            "solid_temp_k",
            "need 0 < solid_temp_k < vent_temp_k",
        ));
    }
    for (name, v) in [
        ("viscosity_decades", p.viscosity_decades),
        ("yield_decades", p.yield_decades),
    ] {
        if !(0.0..=8.0).contains(&v) {
            return Err(JdError::invalid(name, "must be within 0..=8"));
        }
    }
    if !(1..=2_000_000).contains(&p.max_steps) {
        return Err(JdError::invalid("max_steps", "must be within 1..=2000000"));
    }
    if !spacing_m.iter().all(|s| pos(*s)) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let (w, hh) = (h.width() as i64, h.height() as i64);
    let wu = w as usize;
    let idx = |x: i64, y: i64| (y * w + x) as usize;
    let area = spacing_m[0] * spacing_m[1];
    let dist = NB.map(|(dx, dy)| (dx as f32 * spacing_m[0]).hypot(dy as f32 * spacing_m[1]));
    let face = std::array::from_fn::<f32, 8, _>(|k| area / dist[k]);
    let terrain = h.to_vec();
    // Vents → (cell, rate).
    let vents: Vec<(usize, f32)> = if p.vents.is_empty() {
        let top = (0..terrain.len()).fold(0, |b, i| if terrain[i] > terrain[b] { i } else { b });
        vec![(top, p.vent_rate_m3_s)]
    } else {
        p.vents
            .iter()
            .map(|v| {
                let (x, y) = (
                    (v.at[0] / spacing_m[0]).floor() as i64,
                    (v.at[1] / spacing_m[1]).floor() as i64,
                );
                if !(0..w).contains(&x)
                    || !(0..hh).contains(&y)
                    || !v.at.iter().all(|c| c.is_finite())
                {
                    return Err(JdError::invalid("vents", "a vent lies outside the layer"));
                }
                if !pos(v.rate_m3_s) {
                    return Err(JdError::invalid("vents", "rate_m3_s must be positive"));
                }
                Ok((idx(x, y), v.rate_m3_s))
            })
            .collect::<JdResult<_>>()?
    };
    let total_rate: f32 = vents.iter().map(|v| v.1).sum();
    // Frozen thickness on its own: added to a 3 000 m ground in f32, thin films would round away.
    let mut solid = vec![0f32; terrain.len()];
    let mut lh = vec![0f32; terrain.len()];
    let mut lt = vec![T_AIR; terrain.len()];
    let mut q = vec![[0f32; 8]; terrain.len()];
    let (mut nh, mut nt) = (lh.clone(), lt.clone());
    // Active box (inclusive), grown to hold every cell with lava plus one ring.
    let (mut x0, mut x1, mut y0, mut y1) = (w, -1i64, hh, -1i64);
    let grow = |b: &mut (i64, i64, i64, i64), i: usize| {
        let (x, y) = ((i % wu) as i64, (i / wu) as i64);
        *b = (
            (x - 1).max(0).min(b.0),
            (x + 1).min(w - 1).max(b.1),
            (y - 1).max(0).min(b.2),
            (y + 1).min(hh - 1).max(b.3),
        );
    };
    {
        let mut b = (x0, x1, y0, y1);
        for &(i, _) in &vents {
            grow(&mut b, i);
        }
        (x0, x1, y0, y1) = b;
    }
    let (rho_c, dt_range) = (
        p.density_kg_m3 * p.specific_heat,
        p.vent_temp_k - p.solid_temp_k,
    );
    let rheology = |t: f32| {
        let th = ((p.vent_temp_k - t) / dt_range).clamp(0.0, 1.0);
        (
            p.yield_pa * 10f32.powf(p.yield_decades * th),
            p.viscosity_pa_s * 10f32.powf(p.viscosity_decades * th),
        )
    };
    let (mut t, mut steps, mut erupted, mut frozen) = (0f64, 0u32, 0f64, 0f64);
    let rho_g = p.density_kg_m3 * G;
    while t < p.duration_s as f64 && steps < p.max_steps {
        if steps % 64 == 0 {
            check_cancel(cancel)?;
        }
        // Pass 1: fluxes out of every active cell and the largest stable step.
        let (sr, lhr, ltr) = (&solid, &lh, &lt);
        let dt_flux = q
            .par_chunks_mut(wu)
            .enumerate()
            .filter(|(y, _)| (y0..=y1).contains(&(*y as i64)))
            .map(|(y, row)| {
                let y = y as i64;
                let mut dt = MAX_DT;
                for x in x0..=x1 {
                    let i = idx(x, y);
                    let out = &mut row[x as usize];
                    *out = [0.0; 8];
                    let hi = lhr[i];
                    if hi <= 0.0 {
                        continue;
                    }
                    let (sy, eta) = rheology(ltr[i]);
                    let mut total = 0.0;
                    for k in 0..8 {
                        let (nx, ny) = (x + NB[k].0, y + NB[k].1);
                        if nx < 0 || ny < 0 || nx >= w || ny >= hh {
                            continue;
                        }
                        let j = idx(nx, ny);
                        let dz = (terrain[i] - terrain[j]) + (sr[i] - sr[j]);
                        let drop = dz + hi - lhr[j];
                        if drop <= 0.0 {
                            continue;
                        }
                        let hcr = sy * dz.hypot(dist[k]) / (rho_g * drop);
                        if hi <= hcr {
                            continue;
                        }
                        let a = hi / hcr;
                        let qk =
                            sy * hcr * hcr * face[k] / (3.0 * eta) * (a - 1.0).powi(2) * (a + 0.5); // = a³ − 3/2 a² + 1/2, without the cancellation near a = 1
                        out[k] = qk;
                        total += qk;
                        dt = dt.min(CFL * drop * area * 0.5 / qk);
                    }
                    if total > 0.0 {
                        dt = dt.min(CFL * hi * area / total);
                    }
                }
                dt
            })
            .reduce(|| MAX_DT, f32::min);
        let erupting = t < p.eruption_s as f64;
        let mut dt = dt_flux.min((p.duration_s as f64 - t) as f32).max(1e-3);
        if erupting {
            dt = dt
                .min(CFL * area / total_rate)
                .min((p.eruption_s as f64 - t).max(1e-3) as f32);
        }
        // Pass 2: gather, cool, freeze.
        let qr = &q;
        let frozen_step: f64 = nh
            .par_chunks_mut(wu)
            .zip(nt.par_chunks_mut(wu))
            .zip(solid.par_chunks_mut(wu))
            .enumerate()
            .filter(|(y, _)| (y0..=y1).contains(&(*y as i64)))
            .map(|(y, ((hrow, trow), grow_))| {
                let y = y as i64;
                let mut froze = 0f64;
                for x in x0..=x1 {
                    let i = idx(x, y);
                    let out: f32 = qr[i].iter().sum::<f32>() * dt / area;
                    let mut vol = lhr[i] - out;
                    let mut heat = (lhr[i] - out) * ltr[i];
                    for k in 0..8 {
                        let (nx, ny) = (x - NB[k].0, y - NB[k].1); // the neighbour whose direction k points here
                        if nx < 0
                            || ny < 0
                            || nx >= w
                            || ny >= hh
                            || nx < x0
                            || nx > x1
                            || ny < y0
                            || ny > y1
                        {
                            continue;
                        }
                        let j = idx(nx, ny);
                        let inflow = qr[j][k] * dt / area;
                        vol += inflow;
                        heat += inflow * ltr[j];
                    }
                    let (hcell, tcell) = (&mut hrow[x as usize], &mut trow[x as usize]);
                    if vol <= 1e-6 {
                        // A film too thin to carry heat freezes where it is (keeps the volume exact).
                        grow_[x as usize] += vol.max(0.0);
                        froze += vol.max(0.0) as f64;
                        (*hcell, *tcell) = (0.0, T_AIR);
                        continue;
                    }
                    let mut temp = heat / vol;
                    let loss =
                        p.cooling * emissivity(temp) * SIGMA * (temp.powi(4) - T_AIR.powi(4)) * dt
                            / (rho_c * vol);
                    temp = (temp - loss).max(T_AIR);
                    if temp < p.solid_temp_k {
                        grow_[x as usize] += vol;
                        froze += vol as f64;
                        (*hcell, *tcell) = (0.0, T_AIR);
                    } else {
                        (*hcell, *tcell) = (vol, temp);
                    }
                }
                froze
            })
            .sum();
        std::mem::swap(&mut lh, &mut nh);
        std::mem::swap(&mut lt, &mut nt);
        frozen += frozen_step * area as f64;
        if erupting {
            for &(i, rate) in &vents {
                let add = rate * dt / area;
                lt[i] = (lh[i] * lt[i] + add * p.vent_temp_k) / (lh[i] + add);
                lh[i] += add;
                erupted += (rate * dt) as f64;
            }
        }
        let mut b = (x0, x1, y0, y1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if lh[idx(x, y)] > 0.0 && (x == x0 || x == x1 || y == y0 || y == y1) {
                    grow(&mut b, idx(x, y));
                }
            }
        }
        (x0, x1, y0, y1) = b;
        t += dt as f64;
        steps += 1;
    }
    if t + 1e-3 < p.duration_s as f64 {
        return Err(JdError::invalid("max_steps", format!("the step budget ran out after {t:.0} of {:.0} simulated seconds", p.duration_s))
            .why("Runny lava on fine cells needs short steps: each step may move at most a quarter of a cell's lava.")
            .next("Raise `max_steps`, shorten `duration_s`, raise `viscosity_pa_s`, or evaluate at a coarser preview level."));
    }
    let mk = |f: &(dyn Fn(usize) -> f32 + Sync)| -> JdResult<Heightfield> {
        let mut o = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
        o.fill_with(|x, y| f(y as usize * wu + x as usize));
        Ok(o)
    };
    Ok(LavaResult {
        out: mk(&|i| terrain[i] + (solid[i] + lh[i]))?,
        lava: mk(&|i| solid[i] + lh[i])?,
        heat: mk(&|i| {
            if lh[i] > 0.0 {
                ((lt[i] - p.solid_temp_k) / dt_range).clamp(0.0, 1.0)
            } else {
                0.0
            }
        })?,
        steps,
        simulated_s: t,
        erupted_m3: erupted,
        frozen_m3: frozen,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 6° slope falling to the south (row 0), 10 m cells, 48 × 96.
    fn slope() -> Heightfield {
        let mut h = Heightfield::new(48, 96, 128, 0).expect("hf");
        h.fill_with(|_, y| y as f32 * 10.0 * 6f32.to_radians().tan());
        h
    }

    #[test]
    fn a_vent_flows_downhill_conserves_volume_cools_and_freezes() {
        let h = slope();
        let p = LavaParams {
            vents: vec![LavaVent {
                at: [245.0, 905.0],
                rate_m3_s: 10.0,
            }],
            eruption_s: 3600.0,
            duration_s: 4.0 * 3600.0,
            ..LavaParams::default()
        };
        let r = lava_flow(&h, [10.0, 10.0], &p, None).expect("lava");
        assert!(
            (r.simulated_s - 4.0 * 3600.0).abs() < 1.0,
            "ran to the end: {}",
            r.simulated_s
        );
        assert!(
            (r.erupted_m3 - 36_000.0).abs() < 1.0,
            "erupted {}",
            r.erupted_m3
        );
        let cells: Vec<(u32, u32)> = (0..96).flat_map(|y| (0..48).map(move |x| (x, y))).collect();
        let lava_m3: f64 = cells
            .iter()
            .map(|&(x, y)| r.lava.get(x, y) as f64 * 100.0)
            .sum();
        assert!(
            (lava_m3 - r.erupted_m3).abs() < 1e-3 * r.erupted_m3,
            "volume conserved: {lava_m3} vs {}",
            r.erupted_m3
        );
        // Downhill only: nothing above the vent row beyond a cell or two of spreading; the front runs south.
        let rows: Vec<u32> = cells
            .iter()
            .filter(|&&(x, y)| r.lava.get(x, y) > 0.01)
            .map(|&(_, y)| y)
            .collect();
        let (lowest, highest) = (
            *rows.iter().min().expect("lava"),
            *rows.iter().max().expect("lava"),
        );
        assert!(highest <= 92, "no uphill flow: {highest}");
        assert!(lowest < 80, "the flow ran downhill: front at row {lowest}");
        assert!(r.frozen_m3 > 0.0, "the margins froze");
        // Longer than wide: a lobe down the fall line.
        let cols: Vec<u32> = cells
            .iter()
            .filter(|&&(x, y)| r.lava.get(x, y) > 0.01)
            .map(|&(x, _)| x)
            .collect();
        let width = cols.iter().max().unwrap() - cols.iter().min().unwrap() + 1;
        assert!(
            highest - lowest + 1 > width,
            "length {} vs width {width}",
            highest - lowest + 1
        );
        // Cooler lava runs shorter; outputs stay finite; bad params refused.
        let cold = lava_flow(
            &h,
            [10.0, 10.0],
            &LavaParams {
                cooling: 20.0,
                ..p.clone()
            },
            None,
        )
        .expect("cold");
        let cold_lowest = cells
            .iter()
            .filter(|&&(x, y)| cold.lava.get(x, y) > 0.01)
            .map(|&(_, y)| y)
            .min()
            .expect("lava");
        assert!(
            cold_lowest > lowest,
            "faster cooling, shorter flow: {cold_lowest} vs {lowest}"
        );
        assert!(
            cells
                .iter()
                .all(|&(x, y)| r.out.get(x, y).is_finite()
                    && (0.0..=1.0).contains(&r.heat.get(x, y)))
        );
        assert!(
            lava_flow(
                &h,
                [10.0, 10.0],
                &LavaParams {
                    solid_temp_k: 1400.0,
                    ..p.clone()
                },
                None
            )
            .is_err()
        );
        assert!(
            lava_flow(
                &h,
                [10.0, 10.0],
                &LavaParams {
                    vents: vec![LavaVent {
                        at: [9999.0, 0.0],
                        rate_m3_s: 1.0
                    }],
                    ..p
                },
                None
            )
            .is_err()
        );
    }

    #[test]
    fn on_a_flat_plane_the_flow_spreads_round_and_the_summit_vent_is_automatic() {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| if (x, y) == (32, 32) { 0.01 } else { 0.0 });
        let p = LavaParams {
            eruption_s: 7200.0,
            duration_s: 7200.0,
            cooling: 1e-3,
            ..LavaParams::default()
        };
        let r = lava_flow(&h, [10.0, 10.0], &p, None).expect("lava");
        let reach = |dx: i32, dy: i32| {
            (1..32)
                .take_while(|s| r.lava.get((32 + dx * s) as u32, (32 + dy * s) as u32) > 0.01)
                .count() as f32
        };
        let n = (0..64u32)
            .flat_map(|y| (0..64u32).map(move |x| (x, y)))
            .filter(|&(x, y)| r.lava.get(x, y) > 0.01)
            .count();
        let ax4 = (reach(1, 0) + reach(-1, 0) + reach(0, 1) + reach(0, -1)) / 4.0 + 0.5;
        let round = n as f32 / (std::f32::consts::PI * ax4 * ax4);
        assert!(ax4 > 10.0, "it spread: {ax4} cells");
        assert!(
            (round - 1.0).abs() < 0.1,
            "round, not a diamond (0.64) or a square (1.27): {round}"
        );
    }
}
