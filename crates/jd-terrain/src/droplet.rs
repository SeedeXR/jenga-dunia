//! Particle (droplet) hydraulic erosion after Beyer 2015 (`docs/research/algorithms.md` §(d)):
//! each droplet starts at a seeded random position with some water and speed, follows the
//! bilinear gradient with inertia, carries sediment up to `capacity ∝ slope · speed · water`,
//! deposits the excess (or a fraction when it slows) and erodes the deficit through a
//! radius-weighted brush, while its water evaporates. Deterministic from `seed`; sequential by
//! nature (droplets interact through the field), so cost is O(droplets · max_steps · radius²).
//!
//! ponytail: CPU, one thread, sediment leaving the map is lost (reported as `lost_m3`); the
//! GPU twin and McDonald's stream/pool extensions come with `jd-gpu` / Phase 5.

use crate::Heightfield;
use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

/// Droplet erosion parameters (Beyer's defaults, metres scaled by the sample spacing).
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DropletParams {
    /// Number of droplets (1..=5 000 000). Aim for 1–2 per cell: 512² wants ~400 000; fewer gives
    /// diffuse, unstructured lowering, more deepens the same gullies.
    #[serde(default = "d_droplets")]
    pub droplets: u32,
    /// Seed.
    #[serde(default)]
    pub seed: u64,
    /// Steps a droplet may travel before it is dropped (8..=256).
    #[serde(default = "d_max_steps")]
    pub max_steps: u32,
    /// Direction inertia 0..1 (0 follows the gradient exactly).
    #[serde(default = "d_inertia")]
    pub inertia: f32,
    /// Sediment capacity factor.
    #[serde(default = "d_capacity")]
    pub capacity: f32,
    /// Minimum capacity so droplets on flats still carry a little.
    #[serde(default = "d_min_capacity")]
    pub min_capacity: f32,
    /// Fraction of surplus sediment deposited per step, 0..1.
    #[serde(default = "d_deposit")]
    pub deposit: f32,
    /// Fraction of the capacity deficit eroded per step, 0..1.
    #[serde(default = "d_erode")]
    pub erode: f32,
    /// Water lost per step, 0..1.
    #[serde(default = "d_evaporation")]
    pub evaporation: f32,
    /// Gravity (speed gain per unit drop).
    #[serde(default = "d_gravity")]
    pub gravity: f32,
    /// Erosion brush radius in cells (1..=8).
    #[serde(default = "d_radius")]
    pub radius: u32,
}
fn d_droplets() -> u32 {
    200_000
}
fn d_max_steps() -> u32 {
    64
}
fn d_inertia() -> f32 {
    0.05
}
fn d_capacity() -> f32 {
    4.0
}
fn d_min_capacity() -> f32 {
    0.01
}
fn d_deposit() -> f32 {
    0.3
}
fn d_erode() -> f32 {
    0.3
}
fn d_evaporation() -> f32 {
    0.01
}
fn d_gravity() -> f32 {
    4.0
}
fn d_radius() -> u32 {
    3
}

impl Default for DropletParams {
    fn default() -> Self {
        serde_json::from_value(serde_json::json!({})).unwrap_or_else(|_| Self {
            droplets: d_droplets(),
            seed: 0,
            max_steps: d_max_steps(),
            inertia: d_inertia(),
            capacity: d_capacity(),
            min_capacity: d_min_capacity(),
            deposit: d_deposit(),
            erode: d_erode(),
            evaporation: d_evaporation(),
            gravity: d_gravity(),
            radius: d_radius(),
        })
    }
}

/// Result of a droplet run.
#[derive(Debug)]
pub struct DropletResult {
    /// Eroded terrain.
    pub terrain: Heightfield,
    /// Signed height change (out − in), metres.
    pub delta: Heightfield,
    /// Sediment carried off the map by droplets that left, in height·cells (m³ / cell area).
    pub lost: f64,
}

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

/// Bilinear height and gradient at cell-space position (x, y), both in cell units.
fn sample(h: &[f32], w: usize, hh: usize, x: f32, y: f32) -> (f32, f32, f32) {
    let xi = (x.floor() as i64).clamp(0, w as i64 - 2) as usize;
    let yi = (y.floor() as i64).clamp(0, hh as i64 - 2) as usize;
    let (u, v) = (
        (x - xi as f32).clamp(0.0, 1.0),
        (y - yi as f32).clamp(0.0, 1.0),
    );
    let (a, b, c, d) = (
        h[yi * w + xi],
        h[yi * w + xi + 1],
        h[(yi + 1) * w + xi],
        h[(yi + 1) * w + xi + 1],
    );
    let gx = (b - a) * (1.0 - v) + (d - c) * v;
    let gy = (c - a) * (1.0 - u) + (d - b) * u;
    let height = a * (1.0 - u) * (1.0 - v) + b * u * (1.0 - v) + c * (1.0 - u) * v + d * u * v;
    (height, gx, gy)
}

/// Run droplet erosion. Internally heights are normalised to the field's range (0..1 over the
/// map, Beyer's regime, so his default capacity/erode/deposit values behave the same on any
/// terrain) with one cell as the horizontal unit; results are scaled back to metres. Deposits
/// go through the same radius brush as erosion, so no single cell spikes.
pub fn droplet_erode(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &DropletParams,
    cancel: Option<&AtomicBool>,
) -> JdResult<DropletResult> {
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    if !(1..=5_000_000).contains(&p.droplets) {
        return Err(JdError::invalid("droplets", "must be within 1..=5000000"));
    }
    if !(8..=256).contains(&p.max_steps) {
        return Err(JdError::invalid("max_steps", "must be within 8..=256"));
    }
    if !(1..=8).contains(&p.radius) {
        return Err(JdError::invalid("radius", "must be within 1..=8"));
    }
    for (name, v) in [
        ("inertia", p.inertia),
        ("deposit", p.deposit),
        ("erode", p.erode),
        ("evaporation", p.evaporation),
    ] {
        if !(0.0..=1.0).contains(&v) {
            return Err(JdError::invalid(name, "must be within 0..=1"));
        }
    }
    if !(p.capacity > 0.0
        && p.capacity.is_finite()
        && p.min_capacity >= 0.0
        && p.gravity >= 0.0
        && p.gravity.is_finite())
    {
        return Err(JdError::invalid(
            "capacity",
            "capacity must be positive; min_capacity and gravity ≥ 0",
        ));
    }
    h.validate_finite()?;
    let (w, hh) = (h.width() as usize, h.height() as usize);
    if w < 3 || hh < 3 {
        return Err(JdError::invalid("in", "field must be at least 3×3"));
    }
    let original = h.to_vec();
    let (lo, hi) = original
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), z| (a.min(*z), b.max(*z)));
    let cell = (hi - lo).max(1e-6); // metres per normalised unit
    let mut v: Vec<f32> = original.iter().map(|z| (z - lo) / cell).collect();
    // Brush: offsets within `radius` with weights ∝ (radius − distance), normalised per droplet.
    let r = p.radius as i64;
    let mut brush: Vec<(i64, i64, f32)> = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d < p.radius as f32 {
                brush.push((dx, dy, p.radius as f32 - d));
            }
        }
    }
    let deposit_brush = |v: &mut Vec<f32>, x: f32, y: f32, amount: f32| {
        let (cx, cy) = (x.round() as i64, y.round() as i64);
        let mut wsum = 0.0f32;
        for (bx, by, bw) in &brush {
            let (px, py) = (cx + bx, cy + by);
            if px >= 0 && py >= 0 && px < w as i64 && py < hh as i64 {
                wsum += bw;
            }
        }
        if wsum > 0.0 {
            for (bx, by, bw) in &brush {
                let (px, py) = (cx + bx, cy + by);
                if px >= 0 && py >= 0 && px < w as i64 && py < hh as i64 {
                    v[py as usize * w + px as usize] += amount * bw / wsum;
                }
            }
        }
    };
    let mut lost = 0.0f64;
    for i in 0..p.droplets {
        if i % 1024 == 0 {
            crate::sim::check_cancel(cancel)?;
        }
        let s0 = mix(p.seed ^ (i as u64).wrapping_mul(0x9E37_79B9));
        let (mut x, mut y) = (
            unit(s0) * (w as f32 - 1.0),
            unit(mix(s0 ^ 1)) * (hh as f32 - 1.0),
        );
        let (mut dx, mut dy) = (0.0f32, 0.0f32);
        let (mut speed, mut water, mut sediment) = (1.0f32, 1.0f32, 0.0f32);
        let mut exited = false;
        for _ in 0..p.max_steps {
            let (h0, gx, gy) = sample(&v, w, hh, x, y);
            dx = dx * p.inertia - gx * (1.0 - p.inertia);
            dy = dy * p.inertia - gy * (1.0 - p.inertia);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 {
                // No slope: pick a deterministic direction so the droplet does not stall.
                let a = unit(mix(s0 ^ 7)) * std::f32::consts::TAU;
                dx = a.cos();
                dy = a.sin();
            } else {
                dx /= len;
                dy /= len;
            }
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0.0 || ny < 0.0 || nx >= (w - 1) as f32 || ny >= (hh - 1) as f32 {
                lost += sediment as f64;
                exited = true;
                break;
            }
            let (h1, _, _) = sample(&v, w, hh, nx, ny);
            // Bilinear weights do not sum to exactly 1 in f32: treat sub-millimetre differences
            // as flat so a level field is never nibbled by rounding noise.
            let dh = if (h1 - h0).abs() < 1e-4 { 0.0 } else { h1 - h0 };
            let capacity =
                ((-dh).max(p.min_capacity) * speed * water * p.capacity).max(p.min_capacity);
            if sediment > capacity || dh > 0.0 {
                // Deposit: fill the pit when climbing, else a fraction of the surplus; bilinear onto the old cell.
                let amount = if dh > 0.0 {
                    dh.min(sediment)
                } else {
                    (sediment - capacity) * p.deposit
                };
                sediment -= amount;
                // Bilinear onto the cell the droplet stands on (Beyer): this is what fills pits so
                // droplets keep moving instead of circling and pockmarking the slope.
                let xi = (x.floor() as i64).clamp(0, w as i64 - 2) as usize;
                let yi = (y.floor() as i64).clamp(0, hh as i64 - 2) as usize;
                let (u, t) = (
                    (x - xi as f32).clamp(0.0, 1.0),
                    (y - yi as f32).clamp(0.0, 1.0),
                );
                v[yi * w + xi] += amount * (1.0 - u) * (1.0 - t);
                v[yi * w + xi + 1] += amount * u * (1.0 - t);
                v[(yi + 1) * w + xi] += amount * (1.0 - u) * t;
                v[(yi + 1) * w + xi + 1] += amount * u * t;
            } else {
                // Erode a fraction of the deficit, never more than the drop itself, through the brush.
                let amount = ((capacity - sediment) * p.erode).min(-dh);
                let cx = x.round() as i64;
                let cy = y.round() as i64;
                let mut wsum = 0.0f32;
                for (bx, by, bw) in &brush {
                    let (px, py) = (cx + bx, cy + by);
                    if px >= 0 && py >= 0 && px < w as i64 && py < hh as i64 {
                        wsum += bw;
                    }
                }
                if wsum > 0.0 {
                    for (bx, by, bw) in &brush {
                        let (px, py) = (cx + bx, cy + by);
                        if px >= 0 && py >= 0 && px < w as i64 && py < hh as i64 {
                            let idx = py as usize * w + px as usize;
                            let take = amount * bw / wsum;
                            v[idx] -= take;
                            sediment += take;
                        }
                    }
                }
            }
            speed = (speed * speed + (-dh).max(0.0) * p.gravity).sqrt();
            water *= 1.0 - p.evaporation;
            x = nx;
            y = ny;
            if water < 1e-3 {
                break;
            }
        }
        // A droplet that dries up or runs out of steps drops what it carries where it stands,
        // so eroded = deposited + lost (mass balance).
        if !exited && sediment > 0.0 {
            deposit_brush(&mut v, x, y, sediment);
        }
    }
    crate::sim::check_cancel(cancel)?;
    let mut terrain = h.clone();
    terrain.fill_with(|x, y| lo + v[y as usize * w + x as usize] * cell);
    let mut delta = h.clone();
    delta.fill_with(|x, y| {
        lo + v[y as usize * w + x as usize] * cell - original[y as usize * w + x as usize]
    });
    Ok(DropletResult {
        terrain,
        delta,
        lost: lost * cell as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn droplets_carve_a_slope_leave_flats_alone_and_are_deterministic() {
        let mut flat = Heightfield::new(32, 32, 32, 0).expect("hf");
        flat.fill_with(|_, _| 100.0);
        let p = DropletParams {
            droplets: 2000,
            seed: 3,
            ..Default::default()
        };
        let r = droplet_erode(&flat, [1.0, 1.0], &p, None).expect("flat");
        assert_eq!(
            r.terrain.content_hash(),
            flat.content_hash(),
            "nothing to erode on a flat"
        );
        let mut slope = Heightfield::new(64, 64, 64, 0).expect("hf");
        slope.fill_with(|x, y| 200.0 - x as f32 * 1.5 + ((y as f32) * 0.7).sin() * 3.0);
        let r = droplet_erode(&slope, [1.0, 1.0], &p, None).expect("slope");
        let d = r.delta.to_vec();
        assert!(d.iter().all(|v| v.is_finite()));
        let eroded: f64 = d.iter().filter(|v| **v < 0.0).map(|v| -*v as f64).sum();
        let deposited: f64 = d.iter().filter(|v| **v > 0.0).map(|v| *v as f64).sum();
        assert!(eroded > 1.0, "something was eroded: {eroded}");
        // Mass balance: what left the surface is deposited elsewhere or carried off the map.
        assert!(
            (eroded - deposited - r.lost).abs() < 1e-2 * eroded,
            "eroded {eroded} deposited {deposited} lost {}",
            r.lost
        );
        let again = droplet_erode(&slope, [1.0, 1.0], &p, None).expect("again");
        assert_eq!(r.terrain.content_hash(), again.terrain.content_hash());
        let other = droplet_erode(
            &slope,
            [1.0, 1.0],
            &DropletParams {
                seed: 4,
                ..p.clone()
            },
            None,
        )
        .expect("seed");
        assert_ne!(
            r.terrain.content_hash(),
            other.terrain.content_hash(),
            "seed changes the result"
        );
        assert!(
            droplet_erode(
                &slope,
                [1.0, 1.0],
                &DropletParams {
                    radius: 0,
                    ..p.clone()
                },
                None
            )
            .is_err()
        );
        let flag = AtomicBool::new(true);
        assert_eq!(
            droplet_erode(&slope, [1.0, 1.0], &p, Some(&flag))
                .err()
                .map(|e| e.code),
            Some(jd_types::ErrorCode::E_CANCELLED)
        );
    }
}
