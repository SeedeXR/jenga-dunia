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
//! ponytail: a static infill, no transport or sorting; the sediment budget and down-slope
//! transport belong to the coastal node's drift term when it exists.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::sculpt::box_mean;

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

impl Default for BathymetryParams {
    fn default() -> Self {
        Self {
            level_m: 0.0,
            smooth_m: d_smooth(),
            max_fill_m: d_fill(),
            strength: d_strength(),
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
    let radius = |s: f32| ((p.smooth_m / s).round() as u32).clamp(1, 4096);
    let mean = box_mean(h, radius(spacing_m[0]), radius(spacing_m[1]));
    let mut out = h.clone();
    out.fill_with(|x, y| {
        let z = h.get(x, y);
        let class = seabed.get(x, y).round();
        if class != 1.0 && class != 3.0 {
            return z;
        }
        let fill = (mean.get(x, y) - z).clamp(0.0, p.max_fill_m) * p.strength;
        (z + fill).min(p.level_m.max(z))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
