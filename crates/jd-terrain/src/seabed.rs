//! Seabed classification: what lies on the sea floor, from the bathymetry alone plus the two
//! things that bring sediment — rivers and wave work (`docs/research/materials-geomorphology.md`,
//! "Coastal"; Wentworth grain sizes in Part B). The rule of thumb coastal geomorphology gives:
//! wave-sorted sand on the gentle shelf above wave base, bare rock where the floor is steep,
//! fine mud where it is deep and still, and reef where the shelf is shallow, gentle, clear of
//! river sediment and (optionally) warm enough.
//!
//! Classes: 0 = dry land (or an inland basin that is not sea), 1 = sand, 2 = rock, 3 = mud,
//! 4 = reef. `jd.mask.class@1` turns any of them into a 0/1 mask for materials and scatter.
//!
//! ponytail: no sediment transport, no currents — a static classification from depth, slope and
//! the river/shore masks. Deltas and sand waves come with the drift term of the coastal node.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::Heightfield;
use crate::flow::sea_level;

/// Seabed settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeabedParams {
    /// Sea level, metres.
    #[serde(default)]
    pub level_m: f32,
    /// Wave base: sand is sorted and kept moving down to this depth (metres, > 0).
    #[serde(default = "d_sand")]
    pub sand_max_depth_m: f32,
    /// Below this depth the water is still enough for fine sediment to settle: mud (metres, ≥ wave base).
    #[serde(default = "d_mud")]
    pub mud_min_depth_m: f32,
    /// Submarine slopes steeper than this (degrees, (0, 90)) hold no sediment: bare rock.
    #[serde(default = "d_rock")]
    pub rock_slope_deg: f32,
    /// Reef band: depths [min, max] where a gentle, sediment-free shelf becomes reef. `[0, 0]` disables reefs.
    #[serde(default = "d_reef")]
    pub reef_depth_m: [f32; 2],
    /// How far (metres) river mouths keep the shelf sandy rather than reef.
    #[serde(default = "d_river_reach")]
    pub river_reach_m: f32,
}

fn d_sand() -> f32 {
    30.0
}
fn d_mud() -> f32 {
    60.0
}
fn d_rock() -> f32 {
    20.0
}
fn d_reef() -> [f32; 2] {
    [2.0, 25.0]
}
fn d_river_reach() -> f32 {
    400.0
}

impl Default for SeabedParams {
    fn default() -> Self {
        Self {
            level_m: 0.0,
            sand_max_depth_m: d_sand(),
            mud_min_depth_m: d_mud(),
            rock_slope_deg: d_rock(),
            reef_depth_m: d_reef(),
            river_reach_m: d_river_reach(),
        }
    }
}

/// Classify the sea floor of `h`. `rivers` (0..1) marks river cells whose sediment keeps the
/// nearby shelf sandy; `shore` (0..1 wave work from the coastal node) is accepted for the same
/// purpose at the shoreline. Returns the class map (0 land, 1 sand, 2 rock, 3 mud, 4 reef).
pub fn classify_seabed(
    h: &Heightfield,
    spacing_m: [f32; 2],
    rivers: Option<&Heightfield>,
    shore: Option<&Heightfield>,
    p: &SeabedParams,
) -> JdResult<Heightfield> {
    classify_seabed_with(h, spacing_m, rivers, shore, None, p)
}

/// [`classify_seabed`] with an optional wave `exposure` field (0..2, the sea-level node's —
/// above 1 where refraction focuses the waves): sheltered water is as calm as deeper water, so
/// mud settles in a bay at depths where an open coast would still be sand, and a headland's
/// floor stays sand deeper — the effective depth for the mud rule is
/// `depth + (1 − exposure) × mud_min_depth_m`.
pub fn classify_seabed_with(
    h: &Heightfield,
    spacing_m: [f32; 2],
    rivers: Option<&Heightfield>,
    shore: Option<&Heightfield>,
    exposure: Option<&Heightfield>,
    p: &SeabedParams,
) -> JdResult<Heightfield> {
    classify_seabed_current(h, spacing_m, rivers, shore, exposure, None, p)
}

/// [`classify_seabed_with`] plus an optional surface `current` field (degrees clockwise from
/// north the water flows towards, the sea-level node's): the sediment reach of every river and
/// shore cell becomes an ellipse pointing downstream — the full `river_reach_m` with the current,
/// a quarter of it against, half of it across — so mud and the reef-free zone trail off
/// downstream of a mouth instead of ringing it.
#[allow(clippy::too_many_arguments)]
pub fn classify_seabed_current(
    h: &Heightfield,
    spacing_m: [f32; 2],
    rivers: Option<&Heightfield>,
    shore: Option<&Heightfield>,
    exposure: Option<&Heightfield>,
    current: Option<&Heightfield>,
    p: &SeabedParams,
) -> JdResult<Heightfield> {
    if !(p.sand_max_depth_m > 0.0 && p.sand_max_depth_m.is_finite()) {
        return Err(JdError::invalid("sand_max_depth_m", "must be positive"));
    }
    if !(p.mud_min_depth_m >= p.sand_max_depth_m && p.mud_min_depth_m.is_finite()) {
        return Err(JdError::invalid(
            "mud_min_depth_m",
            "must be at least the wave base",
        ));
    }
    if !(0.0 < p.rock_slope_deg && p.rock_slope_deg < 90.0) {
        return Err(JdError::invalid("rock_slope_deg", "must be within (0, 90)"));
    }
    if !(p.reef_depth_m[0] >= 0.0 && p.reef_depth_m[1] >= p.reef_depth_m[0]) {
        return Err(JdError::invalid(
            "reef_depth_m",
            "must be [min, max] with 0 ≤ min ≤ max",
        ));
    }
    if !(p.river_reach_m >= 0.0 && p.river_reach_m.is_finite()) {
        return Err(JdError::invalid("river_reach_m", "must be ≥ 0"));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    for (name, f) in [
        ("rivers", rivers),
        ("shore", shore),
        ("exposure", exposure),
        ("current", current),
    ] {
        if let Some(f) = f
            && (f.width(), f.height()) != (h.width(), h.height())
        {
            return Err(JdError::invalid(name, "field sizes differ from `in`"));
        }
    }
    let (w, hh) = (h.width(), h.height());
    let (ocean, _) = sea_level(h, p.level_m, 0.0)?;
    // Sediment influence: river and wave-work cells, dilated by `river_reach_m`.
    let reach = ((p.river_reach_m / spacing_m[0].min(spacing_m[1])).round() as i64).clamp(0, 64);
    let source = |x: u32, y: u32| {
        rivers.is_some_and(|r| r.get(x, y) > 0.5) || shore.is_some_and(|s| s.get(x, y) > 0.5)
    };
    let mut sediment = vec![false; (w * hh) as usize];
    if rivers.is_some() || shore.is_some() {
        for y in 0..hh {
            for x in 0..w {
                if !source(x, y) {
                    continue;
                }
                // With a current the disc becomes an ellipse pointing downstream: full reach
                // with the flow, a quarter against it, half across.
                let flow = current.and_then(|c| {
                    let a = c.get(x, y);
                    a.is_finite()
                        .then(|| (a.to_radians().sin(), a.to_radians().cos())) // else the disc
                });
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        if !(0..w as i64).contains(&nx) || !(0..hh as i64).contains(&ny) {
                            continue;
                        }
                        let inside = match flow {
                            None => dx * dx + dy * dy <= reach * reach,
                            Some((ux, uy)) => {
                                let along = dx as f32 * ux + dy as f32 * uy;
                                let across = -(dx as f32) * uy + dy as f32 * ux;
                                let along = if along >= 0.0 { along } else { 4.0 * along };
                                along * along + 4.0 * across * across <= (reach * reach) as f32
                            }
                        };
                        if inside {
                            sediment[(ny as u32 * w + nx as u32) as usize] = true;
                        }
                    }
                }
            }
        }
    }
    let tan_rock = p.rock_slope_deg.to_radians().tan();
    let reefs = p.reef_depth_m[1] > p.reef_depth_m[0];
    let mut out = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
    out.fill_with(|x, y| {
        if ocean.get(x, y) < 0.5 {
            return 0.0;
        }
        let depth = p.level_m - h.get(x, y);
        let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
        let (y0, y1) = (y.saturating_sub(1), (y + 1).min(hh - 1));
        let dx = (h.get(x1, y) - h.get(x0, y)) / ((x1 - x0).max(1) as f32 * spacing_m[0]);
        let dy = (h.get(x, y1) - h.get(x, y0)) / ((y1 - y0).max(1) as f32 * spacing_m[1]);
        if (dx * dx + dy * dy).sqrt() > tan_rock {
            return 2.0;
        }
        // Sheltered water is as still as deeper water: mud settles there sooner.
        let calm = exposure.map_or(0.0, |e| {
            (1.0 - e.get(x, y).clamp(0.0, 2.0)) * p.mud_min_depth_m
        });
        if depth + calm >= p.mud_min_depth_m {
            return 3.0;
        }
        if reefs
            && depth >= p.reef_depth_m[0]
            && depth <= p.reef_depth_m[1]
            && !sediment[(y * w + x) as usize]
        {
            return 4.0;
        }
        1.0
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_current_carries_the_river_sediment_downstream() {
        // A 64² shelf 5 m deep (reef conditions everywhere), one river mouth cell in the middle,
        // 10 m cells, reach 100 m = 10 cells; the current flows east (90°).
        let mut h = Heightfield::new(64, 64, 64, 0).expect("h");
        h.fill_with(|_, _| -5.0);
        let mut rivers = Heightfield::new(64, 64, 64, 0).expect("r");
        rivers.set(32, 32, 1.0);
        let mut east = Heightfield::new(64, 64, 64, 0).expect("c");
        east.fill_with(|_, _| 90.0);
        let p = SeabedParams {
            river_reach_m: 100.0,
            reef_depth_m: [2.0, 25.0],
            ..SeabedParams::default()
        };
        let plain =
            classify_seabed_with(&h, [10.0, 10.0], Some(&rivers), None, None, &p).expect("plain");
        let drift =
            classify_seabed_current(&h, [10.0, 10.0], Some(&rivers), None, None, Some(&east), &p)
                .expect("drift");
        let blocked = |o: &Heightfield, x0: u32, x1: u32| {
            (x0..x1)
                .flat_map(|x| (0..64u32).map(move |y| (x, y)))
                .filter(|&(x, y)| o.get(x, y) != 4.0)
                .count()
        };
        assert_eq!(
            blocked(&plain, 0, 32),
            blocked(&plain, 33, 64),
            "no current: the reef-free disc is symmetric"
        );
        assert!(
            blocked(&drift, 33, 64) > 2 * blocked(&drift, 0, 32),
            "east of the mouth the sediment reaches far, west barely: {} vs {}",
            blocked(&drift, 33, 64),
            blocked(&drift, 0, 32)
        );
        assert_ne!(
            drift.get(41, 32),
            4.0,
            "ten cells downstream still sediment (sand/mud, not reef)"
        );
        assert_eq!(
            drift.get(27, 32),
            4.0,
            "five cells upstream the reef is back"
        );
    }

    #[test]
    fn a_sheltered_shelf_turns_to_mud_where_an_open_one_stays_sand() {
        // A flat 6 m shelf under a 0 m sea; mud normally needs 20 m.
        let mut h = Heightfield::new(32, 16, 64, 0).expect("h");
        h.fill_with(|_, _| -6.0);
        let p = SeabedParams {
            level_m: 0.0,
            mud_min_depth_m: 20.0,
            sand_max_depth_m: 10.0,
            reef_depth_m: [0.0, 0.0], // no reefs: the question is sand vs mud
            ..SeabedParams::default()
        };
        let open = classify_seabed_with(&h, [10.0, 10.0], None, None, None, &p).expect("open");
        assert!(
            open.to_vec().iter().all(|c| *c == 1.0),
            "an open shelf is sand"
        );
        // West half sheltered (exposure 0.1), east half open (1.0).
        let mut e = Heightfield::new(32, 16, 64, 0).expect("e");
        e.fill_with(|x, _| if x < 16 { 0.1 } else { 1.0 });
        let bay = classify_seabed_with(&h, [10.0, 10.0], None, None, Some(&e), &p).expect("bay");
        assert_eq!(bay.get(4, 8), 3.0, "mud in the sheltered bay");
        assert_eq!(bay.get(28, 8), 1.0, "sand on the open shelf");
        assert!(
            classify_seabed_with(
                &h,
                [10.0, 10.0],
                None,
                None,
                Some(&Heightfield::new(4, 4, 64, 0).expect("s")),
                &p
            )
            .is_err()
        );
    }

    #[test]
    fn shelf_wall_basin_and_river_mouth_classify_as_expected() {
        // West → east: land (+20 m), a gentle shelf down to −40 m, a wall down to −120 m, a flat basin.
        let mut h = Heightfield::new(64, 16, 64, 0).expect("hf");
        h.fill_with(|x, _| match x {
            0..=3 => 20.0,
            4..=31 => -1.5 * (x as f32 - 3.0), // 1.5 m per 10 m cell: 8.5°, down to −42 m
            32..=35 => -42.0 - 20.0 * (x as f32 - 31.0), // 2 m per metre: 63°
            _ => -122.0,
        });
        let mut rivers = Heightfield::new(64, 16, 64, 0).expect("m");
        rivers.fill_with(|x, y| if x == 3 && y == 8 { 1.0 } else { 0.0 }); // a river mouth at the shore
        let p = SeabedParams {
            river_reach_m: 100.0,
            ..SeabedParams::default()
        };
        let c = classify_seabed(&h, [10.0, 10.0], Some(&rivers), None, &p).expect("seabed");
        assert_eq!(c.get(1, 8), 0.0, "land");
        assert_eq!(
            c.get(12, 8),
            1.0,
            "shallow shelf by the river mouth is sand (within reach)"
        );
        assert_eq!(
            c.get(12, 0),
            4.0,
            "shallow gentle shelf away from the river is reef"
        );
        assert_eq!(
            c.get(24, 8),
            1.0,
            "mid shelf (31 m) is sand: below the reef band, above the mud line"
        );
        assert_eq!(c.get(33, 8), 2.0, "the wall is rock");
        assert_eq!(c.get(50, 8), 3.0, "the deep basin is mud");
        // Reefs off → sand; a lower mud line turns the basin and the lower shelf to mud.
        let c2 = classify_seabed(
            &h,
            [10.0, 10.0],
            None,
            None,
            &SeabedParams {
                reef_depth_m: [0.0, 0.0],
                ..p.clone()
            },
        )
        .expect("no reef");
        assert_eq!(c2.get(12, 0), 1.0);
        let c3 = classify_seabed(
            &h,
            [10.0, 10.0],
            None,
            None,
            &SeabedParams {
                mud_min_depth_m: 30.0,
                ..p.clone()
            },
        )
        .expect("mud");
        assert_eq!(c3.get(28, 8), 3.0);
        assert!(
            classify_seabed(
                &h,
                [10.0, 10.0],
                None,
                None,
                &SeabedParams {
                    mud_min_depth_m: 10.0,
                    ..p.clone()
                }
            )
            .is_err()
        );
        assert!(
            classify_seabed(
                &h,
                [10.0, 10.0],
                None,
                None,
                &SeabedParams {
                    rock_slope_deg: 95.0,
                    ..p.clone()
                }
            )
            .is_err()
        );
    }
}
