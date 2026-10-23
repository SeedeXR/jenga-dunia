//! OpenStreetMap features through the Overpass API (ODbL): a bbox query for roads, waterways,
//! water bodies and buildings, and the conversion of Overpass JSON into GeoJSON so the result
//! can be kept as a separately labelled derivative-database file next to the project's assets.

use std::collections::HashMap;

use jd_types::{ErrorCode, JdError, JdResult};

use crate::dem::Bbox;

/// Feature families the query asks for, in the project's words.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// `highway=*` ways.
    Roads,
    /// `waterway` rivers, streams and canals.
    Waterways,
    /// `natural=water` ways and relations (outer rings only).
    Water,
    /// `building=*` ways.
    Buildings,
}

impl Kind {
    /// Every kind, in query order.
    pub fn all() -> [Kind; 4] {
        [Kind::Roads, Kind::Waterways, Kind::Water, Kind::Buildings]
    }

    fn selector(self) -> &'static str {
        match self {
            Kind::Roads => "way[\"highway\"]",
            Kind::Waterways => "way[\"waterway\"~\"^(river|stream|canal)$\"]",
            Kind::Water => "way[\"natural\"=\"water\"];relation[\"natural\"=\"water\"]",
            Kind::Buildings => "way[\"building\"]",
        }
    }

    fn matches(self, tags: &HashMap<String, String>) -> bool {
        match self {
            Kind::Roads => tags.contains_key("highway"),
            Kind::Waterways => tags
                .get("waterway")
                .is_some_and(|w| matches!(w.as_str(), "river" | "stream" | "canal")),
            Kind::Water => tags.get("natural").is_some_and(|n| n == "water"),
            Kind::Buildings => tags.contains_key("building"),
        }
    }
}

/// The Overpass QL for `kinds` inside `bbox` (south, west, north, east), JSON out, geometry
/// inline so no second pass over nodes is needed.
pub fn query(bbox: &Bbox, kinds: &[Kind], timeout_s: u32) -> String {
    let mut q = format!(
        "[out:json][timeout:{timeout_s}][bbox:{:.6},{:.6},{:.6},{:.6}];(",
        bbox.south, bbox.west, bbox.north, bbox.east
    );
    for k in kinds {
        q.push_str(k.selector());
        q.push(';');
    }
    q.push_str(");out geom;");
    q
}

/// How many features of each kind came back.
#[derive(
    Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Counts {
    /// `highway` ways.
    pub roads: usize,
    /// Rivers, streams, canals.
    pub waterways: usize,
    /// Water bodies.
    pub water: usize,
    /// Buildings.
    pub buildings: usize,
    /// Coordinates written in total.
    pub nodes: usize,
}

/// Overpass JSON (`out geom`) → a GeoJSON FeatureCollection string. Ways become LineStrings
/// (closed buildings and water become Polygons); relations keep their outer members as
/// Polygons. Each feature carries its OSM id, type and tags.
pub fn to_geojson(overpass_json: &[u8], kinds: &[Kind]) -> JdResult<(String, Counts)> {
    let v: serde_json::Value = serde_json::from_slice(overpass_json).map_err(|e| {
        JdError::new(
            ErrorCode::E_NETWORK,
            format!("Overpass did not return JSON: {e}"),
        )
    })?;
    if let Some(remark) = v.get("remark").and_then(|r| r.as_str()) {
        return Err(JdError::new(ErrorCode::E_NETWORK, format!("Overpass: {remark}"))
            .next("Shrink the bbox or ask for fewer kinds; the public instance limits query size and time."));
    }
    let elements = v
        .get("elements")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let mut features = Vec::new();
    let mut counts = Counts::default();
    for el in &elements {
        let tags: HashMap<String, String> = el
            .get("tags")
            .and_then(|t| t.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let Some(kind) = kinds.iter().copied().find(|k| k.matches(&tags)) else {
            continue;
        };
        let ty = el.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let id = el.get("id").and_then(|i| i.as_i64()).unwrap_or(0);
        let geometry = match ty {
            "way" => {
                let coords: Vec<[f64; 2]> = el
                    .get("geometry")
                    .and_then(|g| g.as_array())
                    .map(|pts| {
                        pts.iter()
                            .filter_map(|p| {
                                Some([p.get("lon")?.as_f64()?, p.get("lat")?.as_f64()?])
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if coords.len() < 2 {
                    continue;
                }
                counts.nodes += coords.len();
                let closed = coords.first() == coords.last() && coords.len() >= 4;
                if closed && matches!(kind, Kind::Buildings | Kind::Water) {
                    serde_json::json!({ "type": "Polygon", "coordinates": [coords] })
                } else {
                    serde_json::json!({ "type": "LineString", "coordinates": coords })
                }
            }
            "relation" => {
                let rings: Vec<Vec<[f64; 2]>> = el
                    .get("members")
                    .and_then(|m| m.as_array())
                    .map(|ms| {
                        ms.iter()
                            .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("outer"))
                            .filter_map(|m| m.get("geometry").and_then(|g| g.as_array()))
                            .map(|pts| {
                                pts.iter()
                                    .filter_map(|p| {
                                        Some([p.get("lon")?.as_f64()?, p.get("lat")?.as_f64()?])
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .filter(|r| r.len() >= 4)
                            .collect()
                    })
                    .unwrap_or_default();
                if rings.is_empty() {
                    continue;
                }
                counts.nodes += rings.iter().map(Vec::len).sum::<usize>();
                serde_json::json!({ "type": "MultiPolygon", "coordinates": rings.iter().map(|r| vec![r.clone()]).collect::<Vec<_>>() })
            }
            _ => continue,
        };
        match kind {
            Kind::Roads => counts.roads += 1,
            Kind::Waterways => counts.waterways += 1,
            Kind::Water => counts.water += 1,
            Kind::Buildings => counts.buildings += 1,
        }
        features.push(serde_json::json!({
            "type": "Feature",
            "id": format!("{ty}/{id}"),
            "properties": { "osm_type": ty, "osm_id": id, "kind": serde_json::to_value(kind)?, "tags": tags },
            "geometry": geometry,
        }));
    }
    let fc = serde_json::json!({
        "type": "FeatureCollection",
        "licence": "ODbL 1.0 — © OpenStreetMap contributors, https://www.openstreetmap.org/copyright",
        "features": features,
    });
    Ok((serde_json::to_string(&fc)?, counts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_and_geojson_round_trip() {
        let bbox = Bbox {
            south: -1.30,
            west: 36.80,
            north: -1.25,
            east: 36.86,
        };
        let q = query(&bbox, &[Kind::Roads, Kind::Water], 60);
        assert!(
            q.starts_with("[out:json][timeout:60][bbox:-1.300000,36.800000,-1.250000,36.860000];(")
        );
        assert!(
            q.contains("way[\"highway\"];")
                && q.contains("relation[\"natural\"=\"water\"];")
                && q.ends_with(");out geom;")
        );
        let sample = serde_json::json!({ "elements": [
            { "type": "way", "id": 1, "tags": { "highway": "primary", "name": "Uhuru" }, "geometry": [ { "lat": -1.29, "lon": 36.81 }, { "lat": -1.28, "lon": 36.82 } ] },
            { "type": "way", "id": 2, "tags": { "natural": "water" }, "geometry": [ { "lat": -1.29, "lon": 36.81 }, { "lat": -1.29, "lon": 36.82 }, { "lat": -1.28, "lon": 36.82 }, { "lat": -1.29, "lon": 36.81 } ] },
            { "type": "way", "id": 3, "tags": { "building": "yes" }, "geometry": [ { "lat": 0, "lon": 0 }, { "lat": 0, "lon": 1 }, { "lat": 1, "lon": 1 }, { "lat": 0, "lon": 0 } ] },
            { "type": "node", "id": 4, "lat": 0, "lon": 0 }
        ] });
        let (geojson, counts) = to_geojson(
            serde_json::to_vec(&sample).unwrap().as_slice(),
            &[Kind::Roads, Kind::Water],
        )
        .unwrap();
        assert_eq!(
            counts,
            Counts {
                roads: 1,
                waterways: 0,
                water: 1,
                buildings: 0,
                nodes: 6
            },
            "buildings were not asked for"
        );
        let fc: serde_json::Value = serde_json::from_str(&geojson).unwrap();
        assert_eq!(fc["features"][0]["geometry"]["type"], "LineString");
        assert_eq!(fc["features"][1]["geometry"]["type"], "Polygon");
        assert_eq!(fc["features"][0]["properties"]["tags"]["name"], "Uhuru");
        assert!(
            fc["licence"]
                .as_str()
                .unwrap()
                .contains("OpenStreetMap contributors")
        );
        let remark =
            serde_json::json!({ "remark": "runtime error: Query timed out", "elements": [] });
        assert_eq!(
            to_geojson(
                serde_json::to_vec(&remark).unwrap().as_slice(),
                &Kind::all()
            )
            .unwrap_err()
            .code,
            ErrorCode::E_NETWORK
        );
    }
}
