//! `jd-geo`: the small geodesy layer the project format needs — a world's CRS (`EPSG:…`) turned
//! into a projection and points moved between CRSs (pure Rust `proj4rs`, no PROJ install).
//! Angles cross the API in degrees; proj4rs works in radians internally.
//!
//! ponytail: the EPSG table is the handful of codes terrain work meets (WGS 84, Web Mercator,
//! UTM north/south); anything else must be given as a proj string. No datum grids.

use jd_types::{JdError, JdResult};
use proj4rs::proj::Proj;

/// The proj string for a CRS identifier: `EPSG:4326`, `EPSG:3857`, `EPSG:326NN` / `EPSG:327NN`
/// (UTM zone NN north / south), `WGS84`, or a `+proj=…` string passed through.
pub fn proj_string(crs: &str) -> JdResult<String> {
    let crs = crs.trim();
    if crs.starts_with('+') {
        return Ok(crs.to_owned());
    }
    let code: Option<u32> = crs
        .strip_prefix("EPSG:")
        .or_else(|| crs.strip_prefix("epsg:"))
        .and_then(|c| c.parse().ok());
    match (crs.eq_ignore_ascii_case("WGS84"), code) {
        (true, _) | (_, Some(4326)) => Ok("+proj=longlat +datum=WGS84 +no_defs".to_owned()),
        (_, Some(3857)) => Ok("+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +no_defs".to_owned()),
        (_, Some(c)) if (32601..=32660).contains(&c) => Ok(format!("+proj=utm +zone={} +datum=WGS84 +units=m +no_defs", c - 32600)),
        (_, Some(c)) if (32701..=32760).contains(&c) => Ok(format!("+proj=utm +zone={} +south +datum=WGS84 +units=m +no_defs", c - 32700)),
        _ => Err(JdError::invalid("crs", format!("`{crs}` is not a CRS this build knows"))
            .next("Use EPSG:4326, EPSG:3857, a UTM zone (EPSG:326NN north / 327NN south) or a `+proj=…` string.")),
    }
}

/// Points moved from one CRS to another; geographic ends take and give degrees (lon, lat).
pub struct Transformer {
    src: Proj,
    dst: Proj,
}

impl Transformer {
    /// A transformer between two CRS identifiers (see [`proj_string`]).
    pub fn new(from: &str, to: &str) -> JdResult<Self> {
        let proj = |crs: &str| {
            Proj::from_proj_string(&proj_string(crs)?)
                .map_err(|e| JdError::invalid("crs", format!("`{crs}`: {e}")))
        };
        Ok(Self {
            src: proj(from)?,
            dst: proj(to)?,
        })
    }

    /// Transform `(x, y)` pairs in place.
    pub fn transform(&self, points: &mut [(f64, f64)]) -> JdResult<()> {
        if self.src.is_latlong() {
            for p in points.iter_mut() {
                *p = (p.0.to_radians(), p.1.to_radians());
            }
        }
        proj4rs::transform::transform(&self.src, &self.dst, points)
            .map_err(|e| JdError::invalid("points", format!("transform failed: {e}")))?;
        if self.dst.is_latlong() {
            for p in points.iter_mut() {
                *p = (p.0.to_degrees(), p.1.to_degrees());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utm_zones_web_mercator_and_round_trips_hold_the_textbook_values() {
        // UTM zone 37 north: the central meridian (39° E) maps to easting 500 000, the equator to 0.
        let t = Transformer::new("EPSG:4326", "EPSG:32637").expect("37N");
        let mut p = [(39.0, 0.0), (39.0, 45.0)];
        t.transform(&mut p).expect("fwd");
        assert!(
            (p[0].0 - 500_000.0).abs() < 1e-3 && p[0].1.abs() < 1e-3,
            "{:?}",
            p[0]
        );
        assert!(
            (p[1].0 - 500_000.0).abs() < 1e-3 && (p[1].1 - 4_982_950.4).abs() < 1.0,
            "45° N on the meridian: {:?}",
            p[1]
        );
        // The southern zone adds the 10 000 km false northing.
        let s = Transformer::new("EPSG:4326", "EPSG:32737").expect("37S");
        let mut q = [(39.0, 0.0)];
        s.transform(&mut q).expect("fwd");
        assert!(
            (q[0].0 - 500_000.0).abs() < 1e-3 && (q[0].1 - 10_000_000.0).abs() < 1e-3,
            "{:?}",
            q[0]
        );
        // Web Mercator: the origin stays put, 180° E is the well-known 20 037 508.34 m.
        let m = Transformer::new("EPSG:4326", "EPSG:3857").expect("3857");
        let mut r = [(0.0, 0.0), (180.0, 0.0)];
        m.transform(&mut r).expect("fwd");
        assert!(r[0].0.abs() < 1e-6 && r[0].1.abs() < 1e-6);
        assert!((r[1].0 - 20_037_508.34).abs() < 0.01, "{:?}", r[1]);
        // Round trip back to degrees within a micrometre's worth of angle.
        let back = Transformer::new("EPSG:32637", "EPSG:4326").expect("inv");
        let mut rt = [(500_000.0, 4_982_950.4)];
        back.transform(&mut rt).expect("inv");
        assert!(
            (rt[0].0 - 39.0).abs() < 1e-9 && (rt[0].1 - 45.0).abs() < 1e-7,
            "{:?}",
            rt[0]
        );
        // Unknown codes are a structured error with the next step.
        assert!(Transformer::new("EPSG:9999", "EPSG:4326").is_err());
        assert!(proj_string("+proj=utm +zone=1").is_ok());
    }
}
