//! Aeolian (wind) transport after Werner's slab model (Werner 1995, *Geology* 23(12); the cellular
//! automaton behind every "dune" paper since — `docs/research/materials-geomorphology.md`
//! Part C). Each step picks every cell once in a seeded shuffle; a cell that holds sand and is not
//! in a wind shadow loses a slab, which hops downwind `hop_m` and lands with a probability that
//! depends on what it lands on (sand sticks more than bare ground), landing in a shadow always.
//! After every move the angle of repose is enforced locally. Mass is conserved exactly: slabs are
//! moved, never created or lost (the field wraps around at its edges), and the result depends only
//! on the seed.
//!
//! Scale: Werner's is a lattice model — a slab must be comparable to `cell × tan(shadow_deg)`
//! for the shadow feedback (and the repose rule) to engage at all; thinner slabs only diffuse.
//! The defaults therefore follow the sample spacing: slab = cell / 4, hop = 4 cells.
//!
//! ponytail: one wind direction, one grain size, erodibility from an optional 0..1 `sand` mask
//! (default: everything is sand). Multi-directional winds and vegetation come with the weather
//! timeline.

use std::sync::atomic::AtomicBool;

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::sim::check_cancel;

/// Wind transport settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AeolianParams {
    /// Steps; every step moves up to one slab per sandy cell.
    #[serde(default = "d_iterations")]
    pub iterations: u32,
    /// Compass direction the wind blows from, degrees clockwise from north (270 = westerly). In a
    /// graph, unset → the world's shared wind.
    #[serde(default = "d_wind")]
    pub wind_azimuth_deg: f32,
    /// Saltation hop length in metres (≥ one cell). Default: 4 cells.
    #[serde(default)]
    pub hop_m: Option<f32>,
    /// Slab thickness in metres (> 0). Default: a quarter of the cell size — thinner slabs never
    /// cast wind shadows and only roughen the surface.
    #[serde(default)]
    pub slab_m: Option<f32>,
    /// Angle of repose, degrees (dry sand ≈ 34°).
    #[serde(default = "d_repose")]
    pub repose_deg: f32,
    /// Lee slopes steeper than this, seen along the wind, are wind shadows (Werner: 15°).
    #[serde(default = "d_shadow")]
    pub shadow_deg: f32,
    /// Probability a hopping slab stops on sand.
    #[serde(default = "d_p_sand")]
    pub deposit_on_sand: f32,
    /// Probability a hopping slab stops on bare ground (mask < 0.5).
    #[serde(default = "d_p_bare")]
    pub deposit_on_bare: f32,
    /// Seed.
    #[serde(default)]
    pub seed: u64,
}

fn d_iterations() -> u32 {
    20
}
fn d_wind() -> f32 {
    270.0
}
fn d_repose() -> f32 {
    34.0
}
fn d_shadow() -> f32 {
    15.0
}
fn d_p_sand() -> f32 {
    0.6
}
fn d_p_bare() -> f32 {
    0.4
}

impl Default for AeolianParams {
    fn default() -> Self {
        Self {
            iterations: d_iterations(),
            wind_azimuth_deg: d_wind(),
            hop_m: None,
            slab_m: None,
            repose_deg: d_repose(),
            shadow_deg: d_shadow(),
            deposit_on_sand: d_p_sand(),
            deposit_on_bare: d_p_bare(),
            seed: 0,
        }
    }
}

/// splitmix64.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

type Cell = (i64, i64);

struct Field {
    h: Vec<f32>,
    w: i64,
    hh: i64,
}

impl Field {
    fn idx(&self, x: i64, y: i64) -> usize {
        (y.rem_euclid(self.hh) * self.w + x.rem_euclid(self.w)) as usize
    }
    fn get(&self, x: i64, y: i64) -> f32 {
        self.h[self.idx(x, y)]
    }
}

/// Enforce the angle of repose around a changed cell: a slab slides from the higher side of any
/// neighbour pair steeper than `tan_repose` (down from the cell, or down onto it from an uphill
/// neighbour after erosion), and the cells touched are re-checked until they rest (bounded).
fn relax(f: &mut Field, x: i64, y: i64, slab: f32, tan_repose: f32, spacing: [f32; 2]) {
    const NEIGHBOURS: [(i64, i64); 8] = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ];
    let mut queue = vec![(x, y)];
    let mut moves = 0;
    while let Some((cx, cy)) = queue.pop() {
        if moves > 512 {
            break;
        }
        let here = f.get(cx, cy);
        // The steepest offending pair around this cell, as (from, to).
        let mut best: Option<(Cell, Cell, f32)> = None;
        for (dx, dy) in NEIGHBOURS {
            let dist = ((dx as f32 * spacing[0]).powi(2) + (dy as f32 * spacing[1]).powi(2)).sqrt();
            let there = f.get(cx + dx, cy + dy);
            let slope = (here - there) / dist;
            if slope > tan_repose && best.is_none_or(|b| slope > b.2) {
                best = Some(((cx, cy), (cx + dx, cy + dy), slope));
            } else if -slope > tan_repose && best.is_none_or(|b| -slope > b.2) {
                best = Some(((cx + dx, cy + dy), (cx, cy), -slope));
            }
        }
        if let Some((from, to, _)) = best {
            let i = f.idx(from.0, from.1);
            let j = f.idx(to.0, to.1);
            f.h[i] -= slab;
            f.h[j] += slab;
            moves += 1;
            queue.push(from);
            queue.push(to);
        }
    }
}

/// Werner slab transport on `h` (metres). `sand` (0..1, same size) scales erodibility and decides
/// what a slab lands on; `None` treats every cell as sand. Returns the new heightfield.
pub fn aeolian_transport(
    h: &Heightfield,
    spacing_m: [f32; 2],
    sand: Option<&Heightfield>,
    p: &AeolianParams,
    cancel: Option<&AtomicBool>,
) -> JdResult<Heightfield> {
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let cell = spacing_m[0].min(spacing_m[1]);
    let slab_m = p.slab_m.unwrap_or(cell * 0.25);
    let hop_m = p.hop_m.unwrap_or(cell * 4.0);
    if !(slab_m > 0.0 && slab_m.is_finite()) {
        return Err(JdError::invalid("slab_m", "must be positive"));
    }
    if !(hop_m >= cell && hop_m.is_finite()) {
        return Err(JdError::invalid("hop_m", "must be at least one cell"));
    }
    if !(0.0 < p.repose_deg && p.repose_deg < 90.0) || !(0.0 < p.shadow_deg && p.shadow_deg < 90.0)
    {
        return Err(JdError::invalid(
            "repose_deg",
            "angles must be within (0, 90)",
        ));
    }
    for (name, v) in [
        ("deposit_on_sand", p.deposit_on_sand),
        ("deposit_on_bare", p.deposit_on_bare),
    ] {
        if !(0.0..=1.0).contains(&v) {
            return Err(JdError::invalid(name, "must be within 0..=1"));
        }
    }
    if let Some(m) = sand
        && (m.width(), m.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid(
            "sand",
            "sand mask and terrain sizes differ",
        ));
    }
    let (w, hh) = (h.width() as i64, h.height() as i64);
    let n = (w * hh) as usize;
    let mut f = Field {
        h: h.to_vec(),
        w,
        hh,
    };
    let mask: Vec<f32> = match sand {
        Some(m) => m.to_vec().iter().map(|v| v.clamp(0.0, 1.0)).collect(),
        None => vec![1.0; n],
    };
    // Downwind hop in cells: the wind blows *towards* azimuth + 180°. x grows east, rows grow north.
    let to = (p.wind_azimuth_deg + 180.0).to_radians();
    let cells = hop_m / cell;
    let hop = (
        (cells * to.sin()).round() as i64,
        (cells * to.cos()).round() as i64,
    );
    let hop = if hop == (0, 0) {
        (to.sin().signum() as i64, to.cos().signum() as i64)
    } else {
        hop
    };
    let hop_len =
        ((hop.0 as f32 * spacing_m[0]).powi(2) + (hop.1 as f32 * spacing_m[1]).powi(2)).sqrt();
    // Upwind step (one cell against the wind) for the shadow test.
    let up = (-(to.sin().round() as i64), -(to.cos().round() as i64));
    let up_len = ((up.0 as f32 * spacing_m[0]).powi(2) + (up.1 as f32 * spacing_m[1]).powi(2))
        .sqrt()
        .max(1e-6);
    let tan_shadow = p.shadow_deg.to_radians().tan();
    let tan_repose = p.repose_deg.to_radians().tan();
    let shadow_reach = (hop_len / up_len).ceil().clamp(1.0, 64.0) as i64;
    let in_shadow = |f: &Field, x: i64, y: i64| -> bool {
        let here = f.get(x, y);
        (1..=shadow_reach)
            .any(|k| f.get(x + up.0 * k, y + up.1 * k) - here > tan_shadow * k as f32 * up_len)
    };
    for it in 0..p.iterations {
        check_cancel(cancel)?;
        // Seeded shuffle of the cell order: a different permutation every step, same for every run.
        let mut order: Vec<u32> = (0..n as u32).collect();
        for i in (1..n).rev() {
            let j = (mix(p.seed ^ mix(it as u64) ^ (i as u64)) % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        for (k, &c) in order.iter().enumerate() {
            let (x, y) = ((c as i64) % w, (c as i64) / w);
            let erodibility = mask[c as usize];
            if erodibility <= 0.0 || in_shadow(&f, x, y) {
                continue;
            }
            let slab = slab_m * erodibility;
            let i = f.idx(x, y);
            f.h[i] -= slab;
            relax(&mut f, x, y, slab, tan_repose, spacing_m);
            // Hop until the slab sticks (bounded so a pathological wind cannot loop forever).
            let (mut lx, mut ly) = (x, y);
            let mut r = mix(p.seed ^ mix((it as u64) << 32 | k as u64));
            for _ in 0..16 {
                lx += hop.0;
                ly += hop.1;
                let li = f.idx(lx, ly);
                let on_sand = mask[li] >= 0.5;
                let stick = if in_shadow(&f, lx, ly) {
                    1.0
                } else if on_sand {
                    p.deposit_on_sand
                } else {
                    p.deposit_on_bare
                };
                r = mix(r);
                if unit(r) < stick {
                    break;
                }
            }
            let li = f.idx(lx, ly);
            f.h[li] += slab;
            relax(&mut f, lx, ly, slab, tan_repose, spacing_m);
        }
    }
    let mut out = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    out.fill_with(|x, y| f.h[(y as i64 * w + x as i64) as usize]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slabs_are_conserved_dunes_grow_from_a_sheet_and_the_repose_angle_holds() {
        // A gently rolling sand sheet (±0.5 m, 300 m swells), 10 m cells, westerly wind. Ridges
        // need a few hundred steps and a field some tens of hops across.
        const N: u32 = 96;
        let mut h = Heightfield::new(N, N, 64, 0).expect("hf");
        h.fill_with(|x, y| {
            100.0
                + 0.5 * (x as f32 * 0.21).sin() * (y as f32 * 0.21).sin()
                + 0.02 * unit(mix((x as u64) << 32 | y as u64))
        });
        let p = AeolianParams {
            iterations: 160,
            hop_m: Some(50.0),
            slab_m: Some(3.0),
            ..AeolianParams::default()
        };
        let out = aeolian_transport(&h, [10.0, 10.0], None, &p, None).expect("wind");
        let sum = |f: &Heightfield| -> f64 { f.to_vec().iter().map(|v| *v as f64).sum() };
        assert!(
            (sum(&h) - sum(&out)).abs() < 1e-2,
            "mass {} → {}",
            sum(&h),
            sum(&out)
        );
        let var = |f: &Heightfield| -> f64 {
            let v = f.to_vec();
            let m = v.iter().map(|x| *x as f64).sum::<f64>() / v.len() as f64;
            v.iter().map(|x| (*x as f64 - m).powi(2)).sum::<f64>() / v.len() as f64
        };
        assert!(
            var(&out) > var(&h) * 100.0,
            "relief grows: {} → {}",
            var(&h),
            var(&out)
        );
        // Nothing steeper than the repose angle (plus one slab of tolerance over a 10 m cell).
        let tan = (34.0f32).to_radians().tan();
        let mut steepest = 0.0f32;
        for y in 0..N {
            for x in 0..N {
                for (dx, dy) in [(1i64, 0i64), (0, 1)] {
                    let (nx, ny) = (
                        ((x as i64 + dx).rem_euclid(N as i64)) as u32,
                        ((y as i64 + dy).rem_euclid(N as i64)) as u32,
                    );
                    steepest = steepest.max((out.get(x, y) - out.get(nx, ny)).abs() / 10.0);
                }
            }
        }
        assert!(
            steepest <= tan + 3.0 / 10.0,
            "steepest {steepest} vs repose {tan}"
        );
        // Deterministic, and a different seed gives a different field.
        let again = aeolian_transport(&h, [10.0, 10.0], None, &p, None).expect("again");
        assert_eq!(out.to_vec(), again.to_vec());
        let other = aeolian_transport(
            &h,
            [10.0, 10.0],
            None,
            &AeolianParams {
                seed: 7,
                ..p.clone()
            },
            None,
        )
        .expect("seed");
        assert_ne!(out.to_vec(), other.to_vec());
        // A zero mask freezes the field; an all-bare mask (0.3) still moves but deposits less readily.
        let mut rock = Heightfield::new(N, N, 64, 0).expect("m");
        rock.fill_with(|_, _| 0.0);
        assert_eq!(
            aeolian_transport(&h, [10.0, 10.0], Some(&rock), &p, None)
                .expect("rock")
                .to_vec(),
            h.to_vec()
        );
        assert!(
            aeolian_transport(
                &h,
                [10.0, 10.0],
                None,
                &AeolianParams {
                    hop_m: Some(1.0),
                    ..p.clone()
                },
                None
            )
            .is_err()
        );
        assert!(
            aeolian_transport(
                &h,
                [10.0, 10.0],
                None,
                &AeolianParams {
                    slab_m: Some(0.0),
                    ..p.clone()
                },
                None
            )
            .is_err()
        );
        // Transverse ridges: with the wind along +x the field varies far more along x than along y.
        let v = out.to_vec();
        let diff = |dx: usize, dy: usize| -> f64 {
            let n = N as usize;
            (0..n)
                .flat_map(|y| (0..n).map(move |x| (y, x)))
                .map(|(y, x)| {
                    (v[y * n + x] as f64 - v[((y + dy) % n) * n + (x + dx) % n] as f64).powi(2)
                })
                .sum::<f64>()
        };
        assert!(
            diff(1, 0) > 3.0 * diff(0, 1),
            "along-wind roughness {} vs across {}",
            diff(1, 0),
            diff(0, 1)
        );
    }
}
