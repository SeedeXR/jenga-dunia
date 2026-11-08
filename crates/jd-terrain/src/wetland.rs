//! Wetlands — marshes and swamps — where water collects on flat ground
//! (`docs/research/research-log/2026-11-08-water-features.md`).
//!
//! A wetland needs wetland hydrology, hydric soil and wetland plants (USACE); on a heightfield the
//! hydrology is what can be read: how close the ground lies to the water table, and how flat it
//! is. **HAND** — the height above the nearest drainage, each cell's height above where its flow
//! path meets the stream network — correlates strongly with the water table's depth; Nobre et al.
//! (2011) field-optimised its classes over the lower Rio Negro: ≤ 5.3 m waterlogged, 5.3–15 m
//! ecotone, above that slope and plateau. The Wetland Intrinsic Potential tool scores a per-cell
//! probability from slope and wetness and cuts it at 0.5. So:
//!
//! - **probability** = wet × flat, where *wet* falls from 1 to 0 through `waterlogged_m` of HAND
//!   (± 20 %; the network is every cell draining `drainage_km2` or more), and *flat* falls from 1
//!   at half `max_slope_deg` to 0 at it (no published slope cut-off exists; low slope is a
//!   proxy, so it is soft). A wetness index (TWI) cut at mean + 1σ, as an Illinois study did,
//!   picks the drainage lines rather than the flats a marsh fills, so it is only an input here;
//! - **marsh**: the persistently flooded core (probability ≥ `marsh_at`), where emergent plants
//!   grow in standing water; **swamp**: the saturated fringe (≥ 0.5 below that), where trees grow
//!   (Cowardin's emergent and forested classes);
//! - **water**: a sheet `water_depth_m` deep over the marsh, deeper in the wetland's own hollows,
//!   never more than 2 m — the depth where emergent plants stop and open water begins (Cowardin).
//!
//! ponytail: no seasonal cycle and no peat build-up; the depth is a growing-season snapshot.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::flow::{FlowMethod, d8_receivers, fill_depressions, flow_accumulation_with};

/// Deepest standing water a wetland holds, metres (Cowardin et al. 1979).
pub const MAX_WETLAND_WATER_M: f32 = 2.0;

/// Wetland settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WetlandParams {
    /// Height above the nearest drainage up to which the ground is waterlogged, metres (> 0);
    /// 5.3 = Nobre et al.'s field-optimised waterlogged class.
    #[serde(default = "d_waterlogged")]
    pub waterlogged_m: f32,
    /// Upstream area at which a cell is part of the drainage network, km² (> 0).
    #[serde(default = "d_drainage")]
    pub drainage_km2: f32,
    /// Steepest ground a wetland holds, degrees (0, 90); the probability fades out from half of it.
    #[serde(default = "d_slope")]
    pub max_slope_deg: f32,
    /// Probability above which a wetland is marsh (flooded core) rather than swamp (0.5..1).
    #[serde(default = "d_marsh_at")]
    pub marsh_at: f32,
    /// Standing water over the marsh, metres (0..2): marshes hold more than ~15 cm all year,
    /// wooded swamps a few centimetres.
    #[serde(default = "d_water")]
    pub water_depth_m: f32,
}

fn d_waterlogged() -> f32 {
    5.3
}
fn d_drainage() -> f32 {
    0.1
}
fn d_slope() -> f32 {
    5.0
}
fn d_marsh_at() -> f32 {
    0.75
}
fn d_water() -> f32 {
    0.3
}

impl Default for WetlandParams {
    fn default() -> Self {
        Self {
            waterlogged_m: d_waterlogged(),
            drainage_km2: d_drainage(),
            max_slope_deg: d_slope(),
            marsh_at: d_marsh_at(),
            water_depth_m: d_water(),
        }
    }
}

/// What [`wetland`] produces, all at the input's size.
#[derive(Debug, Clone)]
pub struct WetlandResult {
    /// 0..1 wetland probability (≥ 0.5 = wetland).
    pub wetland: Heightfield,
    /// 0..1 marsh: the flooded core.
    pub marsh: Heightfield,
    /// 0..1 swamp: the saturated fringe.
    pub swamp: Heightfield,
    /// Standing water depth, metres (0..2).
    pub water: Heightfield,
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Height above the nearest drainage of every cell (on the depression-filled terrain, D8): its
/// height above the first network cell its flow path reaches; cells whose path leaves the world
/// without reaching the network count from their own outlet.
pub fn height_above_drainage(
    h: &Heightfield,
    spacing_m: [f32; 2],
    drainage_km2: f32,
) -> JdResult<Heightfield> {
    let (w, hh) = (h.width() as usize, h.height() as usize);
    let (filled, _) = fill_depressions(h, 0.001)?;
    let fv = filled.to_vec();
    let recv = d8_receivers(&fv, w, hh, spacing_m);
    let acc = flow_accumulation_with(&filled, spacing_m, false, FlowMethod::D8)?.to_vec();
    let cell_km2 = spacing_m[0] * spacing_m[1] / 1.0e6;
    // Lowest first: a cell's receiver (strictly lower after the fill) has its base already.
    let mut order: Vec<usize> = (0..fv.len()).collect();
    order.sort_by(|a, b| fv[*a].total_cmp(&fv[*b]));
    let mut base = vec![0.0f32; fv.len()];
    for i in order {
        base[i] = if acc[i] * cell_km2 >= drainage_km2 {
            fv[i]
        } else {
            recv[i].map_or(fv[i], |r| base[r as usize])
        };
    }
    let mut out = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
    out.fill_with(|x, y| {
        let i = y as usize * w + x as usize;
        (fv[i] - base[i]).max(0.0)
    });
    Ok(out)
}

/// The wetlands of `h`. `wetness` is a 0..1 wetness to use instead of the HAND-based one (a
/// published `jd.hydrology.wetness@1` layer, a painted mask).
pub fn wetland(
    h: &Heightfield,
    spacing_m: [f32; 2],
    wetness: Option<&Heightfield>,
    p: &WetlandParams,
) -> JdResult<WetlandResult> {
    if !(p.waterlogged_m.is_finite() && p.waterlogged_m > 0.0) {
        return Err(JdError::invalid("waterlogged_m", "must be finite and > 0"));
    }
    if !(p.drainage_km2.is_finite() && p.drainage_km2 > 0.0) {
        return Err(JdError::invalid("drainage_km2", "must be finite and > 0"));
    }
    if !(p.max_slope_deg > 0.0 && p.max_slope_deg < 90.0) {
        return Err(JdError::invalid("max_slope_deg", "must be within (0, 90)"));
    }
    if !(0.5..1.0).contains(&p.marsh_at) {
        return Err(JdError::invalid("marsh_at", "must be within 0.5..1"));
    }
    if !(0.0..=MAX_WETLAND_WATER_M).contains(&p.water_depth_m) {
        return Err(JdError::invalid(
            "water_depth_m",
            "must be within 0..2 (deeper is a lake)",
        ));
    }
    if let Some(w) = wetness
        && (w.width(), w.height()) != (h.width(), h.height())
    {
        return Err(JdError::invalid("wetness", "must match the terrain's size"));
    }
    let wet: Vec<f32> = match wetness {
        Some(w) => w.to_vec(),
        None => {
            let lo = 0.8 * p.waterlogged_m;
            let hi = 1.2 * p.waterlogged_m;
            height_above_drainage(h, spacing_m, p.drainage_km2)?
                .to_vec()
                .iter()
                .map(|d| 1.0 - smoothstep(lo, hi, *d))
                .collect()
        }
    };
    let slope = crate::slope_mask(h, spacing_m, 90.0)?.to_vec(); // degrees / 90
    let (_, hollows) = fill_depressions(h, 0.001)?;
    let hollows = hollows.to_vec();
    let w = h.width() as usize;
    let field = |f: &(dyn Fn(usize) -> f32 + Sync)| -> JdResult<Heightfield> {
        let mut out = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
        out.fill_with(|x, y| f(y as usize * w + x as usize));
        Ok(out)
    };
    let prob = |i: usize| {
        let wetter = wet[i].clamp(0.0, 1.0);
        let flat = 1.0 - smoothstep(0.5 * p.max_slope_deg, p.max_slope_deg, slope[i] * 90.0);
        wetter * flat
    };
    let marsh = |i: usize| smoothstep(p.marsh_at - 0.05, p.marsh_at + 0.05, prob(i));
    let swamp = |i: usize| smoothstep(0.45, 0.55, prob(i)) * (1.0 - marsh(i));
    let water = |i: usize| {
        let in_wetland = smoothstep(0.45, 0.55, prob(i));
        (p.water_depth_m * marsh(i))
            .max(hollows[i] * in_wetland)
            .min(MAX_WETLAND_WATER_M)
    };
    Ok(WetlandResult {
        wetland: field(&prob)?,
        marsh: field(&marsh)?,
        swamp: field(&swamp)?,
        water: field(&water)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hills draining into a flat, slightly dished valley floor down the middle (x 20..44 of 64,
    /// 10 m cells): the floor gathers the water and is flat; the hillsides are neither.
    fn valley() -> Heightfield {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| {
            let off = (x as f32 - 32.0).abs();
            let side = if off > 12.0 { (off - 12.0) * 6.0 } else { 0.0 }; // ~31° hillsides
            let dish = -0.002 * (12.0 - off.min(12.0)).powi(2); // a shallow dip mid-floor
            100.0 + side + dish + y as f32 * 0.05 // the floor falls gently south
        });
        h
    }

    #[test]
    fn the_flat_wet_valley_floor_is_marsh_and_swamp_and_the_hillsides_are_dry() {
        let r = wetland(&valley(), [10.0, 10.0], None, &WetlandParams::default()).expect("wetland");
        let floor = r.wetland.get(32, 20);
        assert!(floor >= 0.5, "the valley floor is wetland: {floor}");
        for x in [2, 10, 54, 62] {
            assert!(
                r.wetland.get(x, 20) < 0.5,
                "the ~31° hillside at x {x}: {}",
                r.wetland.get(x, 20)
            );
            assert_eq!(r.water.get(x, 20), 0.0);
        }
        // Each wetland cell is marsh or swamp; marsh cells stand in water, the rest is dry.
        for y in 0..64 {
            for x in 0..64 {
                let (m, s, d) = (r.marsh.get(x, y), r.swamp.get(x, y), r.water.get(x, y));
                assert!(
                    m + s <= 1.0 + 1e-5 && (0.0..=MAX_WETLAND_WATER_M).contains(&d),
                    "{x},{y}"
                );
                if m > 0.99 {
                    assert!(
                        d >= 0.3 * m - 1e-4,
                        "marsh holds standing water at {x},{y}: {d}"
                    );
                }
            }
        }
        let marsh: f32 = (0..64)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .map(|(x, y)| r.marsh.get(x, y))
            .sum();
        assert!(marsh > 0.0, "the wettest floor is marsh");
    }

    #[test]
    fn height_above_drainage_is_zero_on_the_stream_and_the_climb_to_it_beside() {
        // A V-valley falling south: the stream runs down the middle column.
        let mut h = Heightfield::new(33, 64, 64, 0).expect("hf");
        h.fill_with(|x, y| 100.0 + (x as f32 - 16.0).abs() * 2.0 + y as f32 * 0.5);
        let hand = height_above_drainage(&h, [10.0, 10.0], 0.01).expect("hand");
        assert!(
            hand.get(16, 10) < 1e-3,
            "on the stream: {}",
            hand.get(16, 10)
        );
        // Five cells up the side the cell sits 10 m above the stream cell it drains into.
        let up = hand.get(21, 10);
        assert!((9.0..=12.0).contains(&up), "{up}");
        assert!(
            hand.get(30, 10) > up,
            "higher up the side, higher above the drainage"
        );
    }

    #[test]
    fn a_steep_wet_gorge_is_not_a_wetland_and_a_given_wetness_is_used() {
        let mut h = Heightfield::new(48, 48, 64, 0).expect("hf");
        h.fill_with(|x, y| 500.0 - y as f32 * 8.0 + (x as f32 - 24.0).abs() * 8.0); // a 39° V-gorge
        let r = wetland(&h, [10.0, 10.0], None, &WetlandParams::default()).expect("gorge");
        assert!(
            r.wetland.to_vec().iter().all(|p| *p < 0.5),
            "water runs off steep ground"
        );
        // A painted wetness: everything wet on the flat valley floor follows it, nothing else.
        let v = valley();
        let mut paint = v.clone();
        paint.fill_with(|x, _| if x < 32 { 1.0 } else { 0.0 });
        let r = wetland(&v, [10.0, 10.0], Some(&paint), &WetlandParams::default()).expect("paint");
        assert!(r.wetland.get(26, 20) >= 0.5 && r.wetland.get(38, 20) < 0.5);
    }

    #[test]
    fn bad_settings_are_refused_and_the_water_never_passes_two_metres() {
        let v = valley();
        for bad in [
            WetlandParams {
                max_slope_deg: 0.0,
                ..WetlandParams::default()
            },
            WetlandParams {
                marsh_at: 0.4,
                ..WetlandParams::default()
            },
            WetlandParams {
                water_depth_m: 2.5,
                ..WetlandParams::default()
            },
            WetlandParams {
                waterlogged_m: f32::NAN,
                ..WetlandParams::default()
            },
            WetlandParams {
                drainage_km2: 0.0,
                ..WetlandParams::default()
            },
        ] {
            assert!(wetland(&v, [10.0, 10.0], None, &bad).is_err(), "{bad:?}");
        }
        // A deep pit in the floor: a wetland around it, but its water stops at 2 m (a lake beyond).
        let mut pit = v.clone();
        pit.set(32, 32, 80.0);
        let p = WetlandParams {
            water_depth_m: 2.0,
            ..WetlandParams::default()
        };
        let r = wetland(&pit, [10.0, 10.0], None, &p).expect("pit");
        assert!(r.water.to_vec().iter().all(|d| *d <= MAX_WETLAND_WATER_M));
        let small = Heightfield::new(8, 8, 8, 0).expect("hf");
        assert!(wetland(&v, [10.0, 10.0], Some(&small), &WetlandParams::default()).is_err());
    }
}
