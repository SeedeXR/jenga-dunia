//! Karst sinkholes (dolines): closed depressions where soluble rock — limestone, gypsum, salt —
//! dissolves and the ground sags or collapses. They sit on flat to gently sloping ground, come in
//! two shapes (a smooth **bowl** from slow subsidence, a steep-walled **collapse** pit with a flat
//! floor), and cluster at densities of a few per km² in classic karst (`docs/research/
//! materials-geomorphology.md`). Desert karst (Nullarbor, Arabian gypsum plains, Sinai) is where
//! the owner's "sink holes" request lives; the same node makes temperate dolines.
//!
//! Placement is deterministic: a splitmix64 stream from `seed` proposes cells, each accepted if
//! the slope is within `max_slope_deg`, the optional `soluble` mask says yes, and it keeps
//! `min_spacing_m` from every accepted sinkhole. Output: the carved terrain and a 0..1 mask of
//! the sinkhole floors (1 at the centre, fading to 0 at the rim) for materials and scatter.
//!
//! ponytail: no underground drainage — a sinkhole is a closed basin to the hydrology nodes
//! (`fill` will lake it); a `drains` flag that punches it through to the ocean mask is the
//! upgrade when a user asks.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;

/// Sinkhole shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SinkholeProfile {
    /// Smooth subsidence bowl: a raised-cosine dip.
    #[default]
    Bowl,
    /// Collapse pit: steep rim over the outer 25 % of the radius, flat floor inside.
    Collapse,
}

/// Karst settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KarstParams {
    /// Placement seed.
    #[serde(default)]
    pub seed: u64,
    /// Sinkholes per km² of eligible ground (> 0).
    #[serde(default = "d_density")]
    pub density_per_km2: f32,
    /// Radius range, metres, [min, max] (> 0).
    #[serde(default = "d_radius")]
    pub radius_m: [f32; 2],
    /// Depth range, metres, [min, max] (> 0).
    #[serde(default = "d_depth")]
    pub depth_m: [f32; 2],
    /// Steepest ground a sinkhole forms on, degrees (0, 90).
    #[serde(default = "d_slope")]
    pub max_slope_deg: f32,
    /// Closest two centres may be, metres (≥ 0).
    #[serde(default = "d_spacing")]
    pub min_spacing_m: f32,
    /// Bowl or collapse.
    #[serde(default)]
    pub profile: SinkholeProfile,
}

fn d_density() -> f32 {
    3.0
}
fn d_radius() -> [f32; 2] {
    [20.0, 80.0]
}
fn d_depth() -> [f32; 2] {
    [3.0, 15.0]
}
fn d_slope() -> f32 {
    6.0
}
fn d_spacing() -> f32 {
    120.0
}

impl Default for KarstParams {
    fn default() -> Self {
        Self {
            seed: 0,
            density_per_km2: d_density(),
            radius_m: d_radius(),
            depth_m: d_depth(),
            max_slope_deg: d_slope(),
            min_spacing_m: d_spacing(),
            profile: SinkholeProfile::Bowl,
        }
    }
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

/// One placed sinkhole (metres, `u` east / `v` north from the south-west corner).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Sinkhole {
    /// Centre, metres east of the west edge.
    pub u: f32,
    /// Centre, metres north of the south edge.
    pub v: f32,
    /// Rim radius, metres.
    pub radius_m: f32,
    /// Depth at the centre, metres.
    pub depth_m: f32,
}

/// Carve sinkholes into `h`. Returns the terrain, the floor mask and the sinkholes placed.
pub fn karst_sinkholes(
    h: &Heightfield,
    spacing_m: [f32; 2],
    soluble: Option<&Heightfield>,
    p: &KarstParams,
) -> JdResult<(Heightfield, Heightfield, Vec<Sinkhole>)> {
    if !(p.density_per_km2 > 0.0 && p.density_per_km2.is_finite()) {
        return Err(JdError::invalid("density_per_km2", "must be positive"));
    }
    for (name, r) in [("radius_m", p.radius_m), ("depth_m", p.depth_m)] {
        if !(r[0] > 0.0 && r[1] >= r[0] && r[1].is_finite()) {
            return Err(JdError::invalid(
                name,
                "must be [min, max] with 0 < min ≤ max",
            ));
        }
    }
    if !(0.0 < p.max_slope_deg && p.max_slope_deg < 90.0) {
        return Err(JdError::invalid("max_slope_deg", "must be within (0, 90)"));
    }
    if !(p.min_spacing_m >= 0.0 && p.min_spacing_m.is_finite()) {
        return Err(JdError::invalid("min_spacing_m", "must be ≥ 0"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    if let Some(s) = soluble
        && (s.width(), s.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid("soluble", "field size differs from `in`"));
    }
    let (w, hh) = (h.width(), h.height());
    // Eligible ground: gentler than `max_slope_deg` (the slope mask is angle / max, clamped to 1)
    // and, if a mask is given, soluble.
    let slope = crate::slope_mask(h, spacing_m, p.max_slope_deg)?;
    let eligible =
        |x: u32, y: u32| slope.get(x, y) < 1.0 && soluble.is_none_or(|s| s.get(x, y) > 0.5);
    let mut eligible_cells = 0u64;
    for y in 0..hh {
        for x in 0..w {
            if eligible(x, y) {
                eligible_cells += 1;
            }
        }
    }
    let cell_km2 = (spacing_m[0] as f64 * spacing_m[1] as f64) / 1.0e6;
    let want = (eligible_cells as f64 * cell_km2 * p.density_per_km2 as f64).round() as usize;
    let mut placed: Vec<Sinkhole> = Vec::with_capacity(want);
    let mut state = mix(p.seed ^ 0x6B61_7273_7400_0000); // "karst"
    // Rejection sampling with a bounded budget: 50 proposals per wanted sinkhole.
    let mut tries = 0usize;
    while placed.len() < want && tries < want.max(1) * 50 {
        tries += 1;
        state = mix(state);
        let x = (unit(state) * w as f32) as u32;
        state = mix(state);
        let y = (unit(state) * hh as f32) as u32;
        let (x, y) = (x.min(w - 1), y.min(hh - 1));
        if !eligible(x, y) {
            continue;
        }
        let (u, v) = (
            (x as f32 + 0.5) * spacing_m[0],
            (y as f32 + 0.5) * spacing_m[1],
        );
        if placed
            .iter()
            .any(|s| ((s.u - u).powi(2) + (s.v - v).powi(2)).sqrt() < p.min_spacing_m)
        {
            continue;
        }
        state = mix(state);
        let radius_m = p.radius_m[0] + unit(state) * (p.radius_m[1] - p.radius_m[0]);
        state = mix(state);
        let depth_m = p.depth_m[0] + unit(state) * (p.depth_m[1] - p.depth_m[0]);
        placed.push(Sinkhole {
            u,
            v,
            radius_m,
            depth_m,
        });
    }
    // Carve: each cell takes the deepest sinkhole covering it (they rarely overlap after spacing).
    let dip = |t: f32| -> f32 {
        // t = r / R in 0..1 → 1 at the centre, 0 at the rim.
        match p.profile {
            SinkholeProfile::Bowl => 0.5 + 0.5 * (std::f32::consts::PI * t).cos(),
            SinkholeProfile::Collapse => {
                let s = ((1.0 - t) / 0.25).clamp(0.0, 1.0);
                s * s * (3.0 - 2.0 * s)
            }
        }
    };
    let mut out = h.clone();
    let mut mask = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    let mut cut = vec![0.0f32; (w * hh) as usize];
    let mut floor = vec![0.0f32; (w * hh) as usize];
    for s in &placed {
        let rx = (s.radius_m / spacing_m[0]).ceil() as i64;
        let ry = (s.radius_m / spacing_m[1]).ceil() as i64;
        let (cx, cy) = (
            (s.u / spacing_m[0] - 0.5).round() as i64,
            (s.v / spacing_m[1] - 0.5).round() as i64,
        );
        for dy in -ry..=ry {
            for dx in -rx..=rx {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= w as i64 || y >= hh as i64 {
                    continue;
                }
                let (u, v) = (
                    (x as f32 + 0.5) * spacing_m[0],
                    (y as f32 + 0.5) * spacing_m[1],
                );
                let t = ((u - s.u).powi(2) + (v - s.v).powi(2)).sqrt() / s.radius_m;
                if t >= 1.0 {
                    continue;
                }
                let d = dip(t);
                let i = (y as u32 * w + x as u32) as usize;
                cut[i] = cut[i].max(d * s.depth_m);
                floor[i] = floor[i].max(d);
            }
        }
    }
    out.fill_with(|x, y| h.get(x, y) - cut[(y * w + x) as usize]);
    mask.fill_with(|x, y| floor[(y * w + x) as usize]);
    Ok((out, mask, placed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sinkholes_land_on_flat_soluble_ground_at_the_asked_density_and_spacing() {
        // 2 km × 2 km at 10 m: the west half is flat (eligible), the east half a 20° slope.
        let mut h = Heightfield::new(200, 200, 256, 0).expect("hf");
        h.fill_with(|x, _| {
            if x < 100 {
                100.0
            } else {
                100.0 + (x - 100) as f32 * 3.64
            }
        });
        let p = KarstParams {
            density_per_km2: 5.0,
            min_spacing_m: 100.0,
            ..KarstParams::default()
        };
        let (out, mask, placed) = karst_sinkholes(&h, [10.0, 10.0], None, &p).expect("karst");
        // Eligible ground is 2 km² → 10 sinkholes asked; spacing may cost a few.
        assert!((7..=10).contains(&placed.len()), "{} placed", placed.len());
        for (i, a) in placed.iter().enumerate() {
            assert!(a.u < 1000.0 + a.radius_m, "on the flat half: {a:?}");
            assert!((20.0..=80.0).contains(&a.radius_m) && (3.0..=15.0).contains(&a.depth_m));
            for b in &placed[i + 1..] {
                assert!(((a.u - b.u).powi(2) + (a.v - b.v).powi(2)).sqrt() >= 100.0);
            }
            // The centre is the deepest point, carved by exactly the sinkhole's depth.
            let (cx, cy) = (
                (a.u / 10.0 - 0.5).round() as u32,
                (a.v / 10.0 - 0.5).round() as u32,
            );
            assert!(
                (h.get(cx, cy) - out.get(cx, cy) - a.depth_m).abs() < 0.2,
                "{a:?}"
            );
            assert!(mask.get(cx, cy) > 0.99);
        }
        // Nothing moved on the slope, nothing was raised anywhere.
        for y in 0..200 {
            for x in 0..200 {
                assert!(out.get(x, y) <= h.get(x, y) + 1e-6);
                if x >= 110 {
                    assert_eq!(out.get(x, y), h.get(x, y));
                }
            }
        }
        // Same seed → same holes; another seed → different; a soluble mask of zeros → none.
        let again = karst_sinkholes(&h, [10.0, 10.0], None, &p)
            .expect("again")
            .2;
        assert_eq!(again, placed);
        let other = karst_sinkholes(
            &h,
            [10.0, 10.0],
            None,
            &KarstParams {
                seed: 7,
                ..p.clone()
            },
        )
        .expect("seed")
        .2;
        assert_ne!(other, placed);
        let none = Heightfield::new(200, 200, 256, 0).expect("m");
        assert!(
            karst_sinkholes(&h, [10.0, 10.0], Some(&none), &p)
                .expect("none")
                .2
                .is_empty()
        );
        // Collapse pits have a flat floor: the cell beside the centre is as deep as the centre.
        let (pit, _, holes) = karst_sinkholes(
            &h,
            [10.0, 10.0],
            None,
            &KarstParams {
                profile: SinkholeProfile::Collapse,
                radius_m: [60.0, 60.0],
                ..p.clone()
            },
        )
        .expect("pit");
        let a = &holes[0];
        let (cx, cy) = (
            (a.u / 10.0 - 0.5).round() as u32,
            (a.v / 10.0 - 0.5).round() as u32,
        );
        assert!((pit.get(cx + 1, cy) - pit.get(cx, cy)).abs() < 1e-4);
        assert!(
            karst_sinkholes(
                &h,
                [10.0, 10.0],
                None,
                &KarstParams {
                    radius_m: [80.0, 20.0],
                    ..p
                }
            )
            .is_err()
        );
    }
}
