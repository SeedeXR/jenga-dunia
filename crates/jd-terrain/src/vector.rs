//! Vector output: river centrelines traced from a river mask, and GeoJSON for polylines.
//!
//! A river mask (`rivers` from `jd.hydrology.rivers@1` or `jd.hydrology.network@1`) is a raster.
//! Engines and GIS want lines: every river cell drains to one D8 receiver on the height, so a
//! trace from each *headwater* river cell (one no other river cell flows into) downstream, until
//! the mask ends or an already traced cell is reached, yields the reach tree as polylines whose
//! last point sits on the reach they join. Deterministic: heads in cell order.

use jd_types::{JdError, JdResult};

use crate::Heightfield;
use crate::flow::d8_receivers;

/// One traced reach: cell centres in layer metres (`u` east, `v` north from the south-west
/// corner), the number of cells and the length along the line.
#[derive(Debug, Clone, PartialEq)]
pub struct Polyline {
    /// Points in layer metres.
    pub points: Vec<[f32; 2]>,
    /// Cells traced (points).
    pub cells: u32,
    /// Length along the line, metres.
    pub length_m: f32,
}

/// River centrelines of `mask` (≥ `threshold` is river) over `height`, as polylines from each
/// headwater down to the sea, a drain or the reach they join; reaches shorter than
/// `min_length_m` are dropped.
pub fn river_polylines(
    height: &Heightfield,
    mask: &Heightfield,
    spacing_m: [f32; 2],
    threshold: f32,
    min_length_m: f32,
) -> JdResult<Vec<Polyline>> {
    if (mask.width(), mask.height()) != (height.width(), height.height()) {
        return Err(JdError::invalid(
            "rivers",
            "the river mask must match the height layer's size",
        ));
    }
    if !(0.0..=1.0).contains(&threshold) || !(min_length_m >= 0.0 && min_length_m.is_finite()) {
        return Err(JdError::invalid(
            "threshold",
            "threshold within 0..=1, min_length_m ≥ 0",
        ));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let (w, h) = (height.width() as usize, height.height() as usize);
    let hv = height.to_vec();
    let recv = d8_receivers(&hv, w, h, spacing_m);
    let river: Vec<bool> = mask.to_vec().iter().map(|v| *v >= threshold).collect();
    // A river cell is a head when no river cell drains into it.
    let mut fed = vec![false; w * h];
    for (i, r) in recv.iter().enumerate() {
        if let Some(j) = r
            && river[i]
            && river[*j as usize]
        {
            fed[*j as usize] = true;
        }
    }
    let centre = |i: usize| {
        [
            ((i % w) as f32 + 0.5) * spacing_m[0],
            ((i / w) as f32 + 0.5) * spacing_m[1],
        ]
    };
    let mut visited = vec![false; w * h];
    let mut out = Vec::new();
    for head in (0..w * h).filter(|&i| river[i] && !fed[i]) {
        if visited[head] {
            continue;
        }
        let mut points = vec![centre(head)];
        let mut length = 0.0f32;
        visited[head] = true;
        let mut i = head;
        while let Some(j) = recv[i] {
            let j = j as usize;
            if !river[j] {
                break;
            }
            let (a, b) = (centre(i), centre(j));
            length += (b[0] - a[0]).hypot(b[1] - a[1]);
            points.push(b);
            if visited[j] {
                break; // joined a reach already traced: the junction is this line's last point
            }
            visited[j] = true;
            i = j;
        }
        if points.len() >= 2 && length >= min_length_m {
            out.push(Polyline {
                cells: points.len() as u32,
                points,
                length_m: length,
            });
        }
    }
    Ok(out)
}

/// Contour lines of `h` every `interval_m` (levels at multiples of the interval inside the range):
/// marching squares over the cell centres, the segments of each level joined end to end into
/// polylines (closed rings repeat their first point). Returns `(level, polyline)` pairs, points in
/// layer metres. Lines shorter than `min_length_m` are dropped.
pub fn contour_lines(
    h: &Heightfield,
    spacing_m: [f32; 2],
    interval_m: f32,
    min_length_m: f32,
) -> JdResult<Vec<(f32, Vec<[f32; 2]>)>> {
    if !(interval_m > 0.0 && interval_m.is_finite())
        || !(min_length_m >= 0.0 && min_length_m.is_finite())
    {
        return Err(JdError::invalid(
            "interval_m",
            "interval positive, min_length_m ≥ 0",
        ));
    }
    if !spacing_m.iter().all(|s| *s > 0.0 && s.is_finite()) {
        return Err(JdError::invalid(
            "spacing_m",
            "sample spacing must be positive",
        ));
    }
    let st = h.stats();
    let (lo, hi) = (st.min, st.max);
    let first = (lo / interval_m).ceil() as i64;
    let last = (hi / interval_m).floor() as i64;
    if last - first > 2000 {
        return Err(JdError::invalid(
            "interval_m",
            format!("{} levels: pick a larger interval", last - first + 1),
        ));
    }
    let (w, hh) = (h.width(), h.height());
    let at = |x: u32, y: u32| {
        [
            (x as f32 + 0.5) * spacing_m[0],
            (y as f32 + 0.5) * spacing_m[1],
        ]
    };
    let key = |p: [f32; 2]| ((p[0] * 64.0).round() as i64, (p[1] * 64.0).round() as i64);
    let mut out = Vec::new();
    for k in first..=last {
        let level = k as f32 * interval_m;
        // Segments of this level, then chained by shared end points.
        let mut segs: Vec<([f32; 2], [f32; 2])> = Vec::new();
        for y in 0..hh.saturating_sub(1) {
            for x in 0..w.saturating_sub(1) {
                let corners = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)];
                let mut hits: Vec<[f32; 2]> = Vec::with_capacity(4);
                for e in 0..4 {
                    let (a, b) = (corners[e], corners[(e + 1) % 4]);
                    let (za, zb) = (h.get(a.0, a.1) - level, h.get(b.0, b.1) - level);
                    if (za < 0.0) != (zb < 0.0) {
                        let t = za / (za - zb);
                        let (pa, pb) = (at(a.0, a.1), at(b.0, b.1));
                        hits.push([pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t]);
                    }
                }
                if hits.len() == 2 {
                    segs.push((hits[0], hits[1]));
                } else if hits.len() == 4 {
                    // A saddle: pair the crossings by the cell-centre value.
                    let centre =
                        (h.get(x, y) + h.get(x + 1, y) + h.get(x + 1, y + 1) + h.get(x, y + 1))
                            * 0.25
                            - level;
                    if (centre < 0.0) == (h.get(x, y) - level < 0.0) {
                        segs.push((hits[0], hits[1]));
                        segs.push((hits[2], hits[3]));
                    } else {
                        segs.push((hits[0], hits[3]));
                        segs.push((hits[1], hits[2]));
                    }
                }
            }
        }
        // Chain: an end point index from rounded coordinates to segment ids.
        let mut ends: std::collections::HashMap<(i64, i64), Vec<usize>> =
            std::collections::HashMap::new();
        for (i, (a, b)) in segs.iter().enumerate() {
            ends.entry(key(*a)).or_default().push(i);
            ends.entry(key(*b)).or_default().push(i);
        }
        let mut used = vec![false; segs.len()];
        for start in 0..segs.len() {
            if used[start] {
                continue;
            }
            used[start] = true;
            let mut line = vec![segs[start].0, segs[start].1];
            // Grow forward from the tail, then backward from the head.
            for forward in [true, false] {
                loop {
                    let tip = if forward {
                        line[line.len() - 1]
                    } else {
                        line[0]
                    };
                    let next = ends
                        .get(&key(tip))
                        .and_then(|ids| ids.iter().copied().find(|&j| !used[j]));
                    let Some(j) = next else { break };
                    used[j] = true;
                    let (a, b) = segs[j];
                    let other = if key(a) == key(tip) { b } else { a };
                    if forward {
                        line.push(other);
                    } else {
                        line.insert(0, other);
                    }
                }
            }
            let len: f32 = line
                .windows(2)
                .map(|p| (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]))
                .sum();
            if len >= min_length_m && line.len() >= 2 {
                out.push((level, line));
            }
        }
    }
    Ok(out)
}

/// A GeoJSON `FeatureCollection` of `LineString`s: `(properties, points)` per feature; `crs`
/// goes into a foreign member `jd:coordinates` so a reader knows whether the numbers are layer
/// metres, world CRS units or WGS 84 longitude / latitude.
pub fn geojson_lines(
    features: &[(serde_json::Value, Vec<[f64; 2]>)],
    coordinates: &str,
) -> serde_json::Value {
    serde_json::json!({
        "type": "FeatureCollection",
        "jd:coordinates": coordinates,
        "features": features.iter().map(|(props, pts)| serde_json::json!({
            "type": "Feature",
            "properties": props,
            "geometry": { "type": "LineString", "coordinates": pts }
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_cone_gives_one_closed_ring_per_level() {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        // A cone 300 m high at the centre, falling 1 m per metre: rings every 50 m.
        h.fill_with(|x, y| {
            (300.0 - ((x as f32 - 31.5) * 10.0).hypot((y as f32 - 31.5) * 10.0)).max(0.0)
        });
        let lines = contour_lines(&h, [10.0, 10.0], 50.0, 0.0).expect("contours");
        let levels: std::collections::BTreeSet<i32> =
            lines.iter().map(|(l, _)| *l as i32).collect();
        assert_eq!(
            levels.into_iter().collect::<Vec<_>>(),
            vec![50, 100, 150, 200, 250, 300]
                .into_iter()
                .filter(|l| *l < 300)
                .collect::<Vec<_>>()
        );
        for (level, line) in &lines {
            let first = line[0];
            let last = *line.last().expect("pt");
            assert!(
                (first[0] - last[0]).hypot(first[1] - last[1]) < 1e-3,
                "level {level}: a closed ring"
            );
            let r_expect = 300.0 - level;
            for p in line {
                let r = (p[0] - 320.0).hypot(p[1] - 320.0);
                assert!(
                    (r - r_expect).abs() < 8.0,
                    "level {level}: radius {r} vs {r_expect}"
                );
            }
        }
        assert_eq!(lines.len(), 5, "one ring per level");
        assert!(contour_lines(&h, [10.0, 10.0], 0.0, 0.0).is_err());
    }

    use super::*;
    use crate::flow::{FlowMethod, carve_rivers_sourced};

    #[test]
    fn a_spring_fed_river_traces_to_one_polyline_running_east() {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        h.fill_with(|x, _| 1000.0 - x as f32);
        let mut springs = h.clone();
        springs.fill_with(|x, y| if (x, y) == (10, 32) { 4096.0 } else { 0.0 });
        let (carved, mask) = carve_rivers_sourced(
            &h,
            [10.0, 10.0],
            0.55,
            2.0,
            FlowMethod::D8,
            None,
            Some(&springs),
        )
        .expect("rivers");
        let lines = river_polylines(&carved, &mask, [10.0, 10.0], 0.5, 50.0).expect("trace");
        assert_eq!(
            lines.len(),
            1,
            "one reach: {:?}",
            lines.iter().map(|l| l.cells).collect::<Vec<_>>()
        );
        let l = &lines[0];
        assert_eq!(
            l.points[0],
            [105.0, 325.0],
            "starts at the spring's cell centre"
        );
        assert!(
            l.points.last().expect("end")[0] > 600.0,
            "runs to the east edge"
        );
        assert!(
            l.points.windows(2).all(|p| p[1][0] > p[0][0]),
            "monotone east"
        );
        assert!((l.length_m - (l.cells as f32 - 1.0) * 10.0).abs() < 1e-3);
        assert!(
            river_polylines(&carved, &mask, [10.0, 10.0], 0.5, 1e6)
                .expect("short")
                .is_empty(),
            "min length drops it"
        );
        let gj = geojson_lines(
            &[(
                serde_json::json!({"kind": "river"}),
                l.points
                    .iter()
                    .map(|p| [f64::from(p[0]), f64::from(p[1])])
                    .collect(),
            )],
            "local_m",
        );
        assert_eq!(gj["features"][0]["geometry"]["type"], "LineString");
        assert_eq!(gj["jd:coordinates"], "local_m");
    }
}
