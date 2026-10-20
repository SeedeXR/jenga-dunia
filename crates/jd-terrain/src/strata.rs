//! Strata: layered rock hardness. Sedimentary beds alternate hard caprock (limestone, sandstone,
//! basalt sills) and soft shale; erosion then carves mesas, buttes and stepped canyon walls —
//! cliffs where the cap holds, talus slopes where the shale goes (`docs/research/
//! materials-geomorphology.md`, "Layered rock"). VUE's "strata" filter is the reference feature
//! (`docs/research/vue-features.md`).
//!
//! The model is 2.5-D: beds are planes of thickness `layer_m`, tilted by `dip_deg` towards
//! `dip_azimuth_deg`, their surfaces warped by a gentle seeded undulation of amplitude `warp_m`.
//! Hardness at a surface point is the hardness of the bed the surface currently sits in, so
//! the field must be re-evaluated after the surface moves (the erosion nodes take it as an
//! input; run strata → erode → strata → erode for deep cuts).
//!
//! ponytail: a repeating two-bed cycle (hard fraction of every layer) rather than a bed table;
//! per-bed hardness lists are the upgrade when a user asks for a specific column.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;

/// Strata settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrataParams {
    /// Thickness of one hard+soft cycle, metres (> 0).
    #[serde(default = "d_layer")]
    pub layer_m: f32,
    /// Share of each cycle that is hard caprock, 0..1 (the rest is soft).
    #[serde(default = "d_hard_fraction")]
    pub hard_fraction: f32,
    /// Hardness of the cap, 0..1 (1 = holds vertical cliffs, erodes at a tenth of the rate).
    #[serde(default = "d_hard")]
    pub hard: f32,
    /// Hardness of the soft bed, 0..1.
    #[serde(default = "d_soft")]
    pub soft: f32,
    /// Width of the hard↔soft transition as a fraction of the cycle, 0..0.5.
    #[serde(default = "d_transition")]
    pub transition: f32,
    /// Dip of the beds, degrees from horizontal, [0, 60].
    #[serde(default)]
    pub dip_deg: f32,
    /// Direction the beds dip towards, degrees clockwise from north.
    #[serde(default)]
    pub dip_azimuth_deg: f32,
    /// Undulation of the bed surfaces, metres (≥ 0): 0 for flat beds.
    #[serde(default = "d_warp")]
    pub warp_m: f32,
    /// Undulation seed.
    #[serde(default)]
    pub seed: u64,
}

fn d_layer() -> f32 {
    25.0
}
fn d_hard_fraction() -> f32 {
    0.35
}
fn d_hard() -> f32 {
    1.0
}
fn d_soft() -> f32 {
    0.15
}
fn d_transition() -> f32 {
    0.08
}
fn d_warp() -> f32 {
    3.0
}

impl Default for StrataParams {
    fn default() -> Self {
        Self {
            layer_m: d_layer(),
            hard_fraction: d_hard_fraction(),
            hard: d_hard(),
            soft: d_soft(),
            transition: d_transition(),
            dip_deg: 0.0,
            dip_azimuth_deg: 0.0,
            warp_m: d_warp(),
            seed: 0,
        }
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Hardness (0..1) of the bed the surface of `h` sits in at every cell.
pub fn strata_hardness(
    h: &Heightfield,
    spacing_m: [f32; 2],
    p: &StrataParams,
) -> JdResult<Heightfield> {
    if !(p.layer_m > 0.0 && p.layer_m.is_finite()) {
        return Err(JdError::invalid("layer_m", "must be positive"));
    }
    for (name, v) in [
        ("hard_fraction", p.hard_fraction),
        ("hard", p.hard),
        ("soft", p.soft),
    ] {
        if !(0.0..=1.0).contains(&v) {
            return Err(JdError::invalid(name, "must be within 0..=1"));
        }
    }
    if !(0.0..=0.5).contains(&p.transition) {
        return Err(JdError::invalid("transition", "must be within 0..=0.5"));
    }
    if !(0.0..=60.0).contains(&p.dip_deg) {
        return Err(JdError::invalid("dip_deg", "must be within 0..=60"));
    }
    if !(p.warp_m >= 0.0 && p.warp_m.is_finite()) || !p.dip_azimuth_deg.is_finite() {
        return Err(JdError::invalid("warp_m", "must be finite and ≥ 0"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let tan_dip = p.dip_deg.to_radians().tan();
    let az = p.dip_azimuth_deg.to_radians();
    let (dir_e, dir_n) = (az.sin(), az.cos());
    // Undulation: two gentle seeded waves, wavelength 12 cycles, so beds roll rather than ripple.
    let wavelength = p.layer_m * 12.0;
    let s1 = (p.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f32 / (1u64 << 24) as f32;
    let s2 = (p.seed.wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 40) as f32 / (1u64 << 24) as f32;
    let (w, hh) = (h.width(), h.height());
    let mut out = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    out.fill_with(|x, y| {
        let u = (x as f32 + 0.5) * spacing_m[0];
        let v = (y as f32 + 0.5) * spacing_m[1];
        // Height measured perpendicular to the dipping beds, plus the undulation.
        let along = u * dir_e + v * dir_n;
        let warp = p.warp_m
            * ((u / wavelength + s1 * std::f32::consts::TAU).sin()
                * (v / wavelength * 0.7 + s2 * std::f32::consts::TAU).cos());
        let z = h.get(x, y) - along * tan_dip + warp;
        let phase = (z / p.layer_m).rem_euclid(1.0);
        // Hard for phase < hard_fraction, soft after, with smooth edges of width `transition`.
        let t = p.transition.max(1e-4);
        let into_hard = smoothstep((p.hard_fraction - phase) / t + 0.5);
        let from_zero = smoothstep(phase / t + 0.5);
        let wrap = smoothstep((1.0 - phase) / t + 0.5); // soft again at the top of the cycle
        let hardness_mix = into_hard.min(from_zero.max(1.0 - wrap));
        p.soft + (p.hard - p.soft) * hardness_mix
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beds_repeat_with_height_tilt_with_dip_and_stay_in_range() {
        // A ramp from 0 to 200 m west → east: eight 25 m cycles.
        let mut h = Heightfield::new(200, 8, 256, 0).expect("hf");
        h.fill_with(|x, _| x as f32);
        let flat = StrataParams {
            warp_m: 0.0,
            ..StrataParams::default()
        };
        let s = strata_hardness(&h, [1.0, 1.0], &flat).expect("strata");
        // Phase 0.1 of a cycle (2.5 m above a bed base) is cap rock; phase 0.6 is shale.
        assert!(
            (s.get(2, 3) - 1.0).abs() < 1e-3,
            "cap at 2.5 m: {}",
            s.get(2, 3)
        );
        assert!(
            (s.get(15, 3) - 0.15).abs() < 1e-3,
            "shale at 15.5 m: {}",
            s.get(15, 3)
        );
        assert!((s.get(27, 3) - 1.0).abs() < 1e-3, "cap again at 27.5 m");
        let hard: usize = (0..200).filter(|&x| s.get(x, 3) > 0.9).count();
        assert!((60..=80).contains(&hard), "about 35 % cap: {hard}");
        for x in 0..200 {
            assert!((0.15..=1.0).contains(&s.get(x, 3)));
        }
        // Dipping beds: on a flat surface the bands march across the ground along the dip.
        let mut flat_h = Heightfield::new(200, 8, 256, 0).expect("hf");
        flat_h.fill_with(|_, _| 50.0);
        let dipped = strata_hardness(
            &flat_h,
            [1.0, 1.0],
            &StrataParams {
                dip_deg: 30.0,
                dip_azimuth_deg: 90.0,
                warp_m: 0.0,
                ..StrataParams::default()
            },
        )
        .expect("dip");
        let bands: usize = (1..200)
            .filter(|&x| (dipped.get(x, 3) > 0.5) != (dipped.get(x - 1, 3) > 0.5))
            .count();
        // tan 30° = 0.577: a 25 m cycle spans 43 m of ground → ~4.6 cycles, two edges each.
        assert!(
            (8..=10).contains(&bands),
            "band edges across the dip: {bands}"
        );
        // Level beds on level ground: one hardness everywhere; warp changes it.
        let level = strata_hardness(&flat_h, [1.0, 1.0], &flat).expect("level");
        assert_eq!(level.get(10, 2), level.get(150, 6));
        let warped = strata_hardness(&flat_h, [1.0, 1.0], &StrataParams::default()).expect("warp");
        assert_ne!(warped.get(10, 2), warped.get(150, 6));
        assert!(
            strata_hardness(
                &h,
                [1.0, 1.0],
                &StrataParams {
                    layer_m: 0.0,
                    ..flat.clone()
                }
            )
            .is_err()
        );
        assert!(
            strata_hardness(
                &h,
                [1.0, 1.0],
                &StrataParams {
                    dip_deg: 80.0,
                    ..flat
                }
            )
            .is_err()
        );
    }
}
