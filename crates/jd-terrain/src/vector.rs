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
