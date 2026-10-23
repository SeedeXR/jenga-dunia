//! Elevation providers and their rasters: AWS Terrain Tiles (Terrarium PNG, Web Mercator),
//! Copernicus DEM GLO-30 (1° GeoTIFF tiles on AWS, EPSG:4326) and OpenTopography's global DEM
//! API (bring your own key, a GeoTIFF for the bbox). Every provider ends in a [`Grid`]: f32
//! heights, row 0 = south, with the georeference the project's heightmap node already reads.

use std::io::Cursor;

use jd_types::{ErrorCode, JdError, JdResult};

use crate::fetch::Request;

/// A lon/lat box in degrees.
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Bbox {
    /// Southern edge, degrees.
    pub south: f64,
    /// Western edge, degrees.
    pub west: f64,
    /// Northern edge, degrees.
    pub north: f64,
    /// Eastern edge, degrees.
    pub east: f64,
}

impl Bbox {
    /// A finite, non-empty box inside the Web Mercator latitude range.
    pub fn validate(&self) -> JdResult<()> {
        let ok = self.south.is_finite()
            && self.north.is_finite()
            && self.west.is_finite()
            && self.east.is_finite()
            && (-85.0511..=85.0511).contains(&self.south)
            && (-85.0511..=85.0511).contains(&self.north)
            && (-180.0..=180.0).contains(&self.west)
            && (-180.0..=180.0).contains(&self.east)
            && self.north > self.south
            && self.east > self.west;
        if !ok {
            return Err(JdError::invalid(
                "bbox",
                "south < north within ±85.05°, west < east within ±180° (no antimeridian crossing)",
            ));
        }
        Ok(())
    }

    /// A box of `radius_m` around a centre.
    pub fn around(lon: f64, lat: f64, radius_m: f64) -> Self {
        let dlat = radius_m / 111_320.0;
        let dlon = radius_m / (111_320.0 * lat.to_radians().cos().max(1e-6));
        Bbox {
            south: lat - dlat,
            west: lon - dlon,
            north: lat + dlat,
            east: lon + dlon,
        }
    }

    /// The box's centre `(lon, lat)`.
    pub fn centre(&self) -> (f64, f64) {
        (
            (self.west + self.east) / 2.0,
            (self.south + self.north) / 2.0,
        )
    }

    /// Ground area in km², a quick sanity cap for provider limits.
    pub fn area_km2(&self) -> f64 {
        let h = (self.north - self.south) * 111.32;
        let w = (self.east - self.west) * 111.32 * self.centre().1.to_radians().cos();
        h * w
    }
}

/// A georeferenced height raster: `data[y * width + x]`, row 0 = south, `origin` = the outer
/// south-west corner in `crs` units, `spacing` per sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    /// Samples per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// Outer south-west corner in `crs` units.
    pub origin: [f64; 2],
    /// Sample spacing `[x, y]` in `crs` units.
    pub spacing: [f64; 2],
    /// `EPSG:3857` (Terrarium) or `EPSG:4326` (GeoTIFF sources).
    pub crs: String,
    /// Heights in metres, row 0 = south.
    pub data: Vec<f32>,
    /// Share of samples that were nodata before the fill, 0..1.
    pub void_share: f32,
}

impl Grid {
    /// Nodata (`NaN`) → the lowest valid height, as the importer does.
    fn fill_voids(&mut self) {
        let n = self.data.len().max(1);
        let voids = self.data.iter().filter(|v| !v.is_finite()).count();
        self.void_share = voids as f32 / n as f32;
        if voids == 0 {
            return;
        }
        let lo = self
            .data
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .fold(f32::INFINITY, f32::min);
        let lo = if lo.is_finite() { lo } else { 0.0 };
        for v in &mut self.data {
            if !v.is_finite() {
                *v = lo;
            }
        }
    }

    /// `(min, max)` of the heights.
    pub fn stats(&self) -> (f32, f32) {
        self.data
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| {
                (lo.min(v), hi.max(v))
            })
    }
}

/// Which provider a request names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// AWS Terrain Tiles, Terrarium PNG (no key, no quota; Mapzen + per-source credits).
    Terrarium,
    /// Copernicus DEM GLO-30 COGs on AWS (no key; the 6(b)/6(c) notices).
    Copernicus,
    /// OpenTopography global DEM API (bring your own key; 50 calls a day).
    Opentopography,
}

impl Provider {
    /// The one host each provider is fetched from (the allowlist entry).
    pub fn host(self) -> &'static str {
        match self {
            Provider::Terrarium => "elevation-tiles-prod.s3.amazonaws.com",
            Provider::Copernicus => "copernicus-dem-30m.s3.amazonaws.com",
            Provider::Opentopography => "portal.opentopography.org",
        }
    }

    /// The attribution obligation each provider carries (source, text, url, licence).
    pub fn attribution(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Provider::Terrarium => (
                "AWS Terrain Tiles (Terrarium)",
                "Terrain Tiles was accessed from https://registry.opendata.aws/terrain-tiles. Mapzen terrain tiles: SRTM and 3DEP data courtesy of the U.S. Geological Survey; produced using Copernicus data and information funded by the European Union - EU-DEM layers; © Commonwealth of Australia (Geoscience Australia) 2017; © Kartverket; and the other sources listed at https://github.com/tilezen/joerd/blob/master/docs/attribution.md.",
                "https://registry.opendata.aws/terrain-tiles/",
                "various (public domain, CC BY, Copernicus) — see the attribution page",
            ),
            Provider::Copernicus => (
                "Copernicus DEM GLO-30",
                "Produced using Copernicus WorldDEM-30 © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved. The organisations in charge of the Copernicus programme by law or by delegation do not incur any liability for any use of the Copernicus WorldDEM-30.",
                "https://spacedata.copernicus.eu/documents/20123/121239/CSCDA_ESA_Mission-specific+Annex_31_Oct_22.pdf",
                "COP-DEM-GLO-30-F licence (commercial use permitted with this notice)",
            ),
            Provider::Opentopography => (
                "OpenTopography (Copernicus DEM GLO-30)",
                "This work is based on API services provided by the OpenTopography Facility with support from the National Science Foundation under awards 1948997, 1948994 & 1948857. Produced using Copernicus WorldDEM-30 © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved.",
                "https://opentopography.org/citations",
                "OpenTopography terms (bring your own key; commercial integration needs an Enterprise key) + COP-DEM-GLO-30-F",
            ),
        }
    }
}

// ── Web Mercator and slippy tiles ──────────────────────────────────────────────────────────

const R: f64 = 6_378_137.0;
const HALF_WORLD: f64 = std::f64::consts::PI * R;
/// Terrarium's finest zoom.
pub const TERRARIUM_MAX_ZOOM: u8 = 15;
const TILE_PX: u32 = 256;

/// Lon/lat degrees → EPSG:3857 metres.
pub fn lonlat_to_merc(lon: f64, lat: f64) -> (f64, f64) {
    let x = lon.to_radians() * R;
    let y = ((std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan()).ln() * R;
    (x, y)
}

/// Fractional tile coordinates of a lon/lat at zoom `z` (x east, y south from the top).
pub fn tile_xy(lon: f64, lat: f64, z: u8) -> (f64, f64) {
    let n = (1u64 << z) as f64;
    let (x, y) = lonlat_to_merc(lon, lat);
    (
        (x + HALF_WORLD) / (2.0 * HALF_WORLD) * n,
        (HALF_WORLD - y) / (2.0 * HALF_WORLD) * n,
    )
}

/// Metres per Terrarium sample at zoom `z` and latitude `lat` (at the equator: 156 543 / 2^z).
pub fn ground_resolution_m(z: u8, lat: f64) -> f64 {
    2.0 * HALF_WORLD / ((1u64 << z) as f64 * TILE_PX as f64) * lat.to_radians().cos()
}

/// The zoom whose ground resolution is nearest to `res_m` at `lat`, capped at the provider's.
pub fn zoom_for(res_m: f64, lat: f64) -> u8 {
    (0..=TERRARIUM_MAX_ZOOM)
        .min_by(|a, b| {
            let da = (ground_resolution_m(*a, lat) - res_m).abs();
            let db = (ground_resolution_m(*b, lat) - res_m).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(TERRARIUM_MAX_ZOOM)
}

/// The tile range covering `bbox` at `z` plus a one-tile ring: `(x0, y0, x1, y1)` inclusive.
pub fn tile_range(bbox: &Bbox, z: u8) -> (u32, u32, u32, u32) {
    let n = (1u64 << z) as f64 - 1.0;
    let (xa, ya) = tile_xy(bbox.west, bbox.north, z);
    let (xb, yb) = tile_xy(bbox.east, bbox.south, z);
    let clamp = |v: f64| v.floor().clamp(0.0, n) as u32;
    (
        clamp(xa - 1.0),
        clamp(ya - 1.0),
        clamp(xb + 1.0),
        clamp(yb + 1.0),
    )
}

/// Terrarium RGB → metres: `(R * 256 + G + B / 256) - 32768`.
pub fn terrarium_m(r: u8, g: u8, b: u8) -> f32 {
    (r as f32 * 256.0 + g as f32 + b as f32 / 256.0) - 32768.0
}

/// A Terrarium fetch plan.
#[derive(Debug, Clone)]
pub struct TerrariumPlan {
    /// Zoom level.
    pub z: u8,
    /// First tile column.
    pub x0: u32,
    /// First tile row (from the north).
    pub y0: u32,
    /// Last tile column, inclusive.
    pub x1: u32,
    /// Last tile row, inclusive.
    pub y1: u32,
    /// One request per tile, row-major from the north-west.
    pub requests: Vec<Request>,
}

impl TerrariumPlan {
    /// The tiles for `bbox` at the zoom nearest `res_m`, from `base_url` (the AWS bucket, or a
    /// local server in tests).
    pub fn new(bbox: &Bbox, res_m: f64, base_url: &str) -> JdResult<Self> {
        bbox.validate()?;
        let z = zoom_for(res_m, bbox.centre().1);
        let (x0, y0, x1, y1) = tile_range(bbox, z);
        let count = (x1 - x0 + 1) as u64 * (y1 - y0 + 1) as u64;
        if count > 4096 {
            return Err(JdError::invalid(
                "bbox",
                format!(
                    "{count} tiles at zoom {z}; the cap is 4096 — shrink the bbox or coarsen `res_m`"
                ),
            ));
        }
        let mut requests = Vec::with_capacity(count as usize);
        for y in y0..=y1 {
            for x in x0..=x1 {
                requests.push(Request {
                    url: format!("{}/{z}/{x}/{y}.png", base_url.trim_end_matches('/')),
                    key: Some(format!("terrarium/{z}/{x}/{y}.png")),
                    expect_bytes: 60_000,
                });
            }
        }
        Ok(TerrariumPlan {
            z,
            x0,
            y0,
            x1,
            y1,
            requests,
        })
    }

    /// Tiles in the plan.
    pub fn tile_count(&self) -> usize {
        self.requests.len()
    }

    /// Decode the tiles (in request order), mosaic them and crop to `bbox`: a Web Mercator grid.
    pub fn mosaic(&self, bbox: &Bbox, tiles: &[Vec<u8>]) -> JdResult<Grid> {
        let (nx, ny) = (self.x1 - self.x0 + 1, self.y1 - self.y0 + 1);
        if tiles.len() != (nx * ny) as usize {
            return Err(JdError::new(
                ErrorCode::E_INTERNAL,
                format!("{} tiles for a {nx}×{ny} plan", tiles.len()),
            ));
        }
        let (mw, mh) = (nx * TILE_PX, ny * TILE_PX);
        let mut m = vec![f32::NAN; (mw * mh) as usize]; // row 0 = north
        for (i, bytes) in tiles.iter().enumerate() {
            let img = image::load_from_memory(bytes)
                .map_err(|e| {
                    JdError::new(ErrorCode::E_NETWORK, format!("tile {i} is not a PNG: {e}"))
                })?
                .to_rgb8();
            if img.dimensions() != (TILE_PX, TILE_PX) {
                return Err(JdError::new(
                    ErrorCode::E_NETWORK,
                    format!("tile {i} is {:?}, not 256²", img.dimensions()),
                ));
            }
            let (tx, ty) = (i as u32 % nx, i as u32 / nx);
            for (px, py, p) in img.enumerate_pixels() {
                let v = terrarium_m(p.0[0], p.0[1], p.0[2]);
                let idx = ((ty * TILE_PX + py) * mw + tx * TILE_PX + px) as usize;
                m[idx] = if v < -11_000.0 { f32::NAN } else { v };
            }
        }
        // Metres per sample and the mosaic's north-west corner in EPSG:3857.
        let tile_m = 2.0 * HALF_WORLD / (1u64 << self.z) as f64;
        let s = tile_m / TILE_PX as f64;
        let west_m = -HALF_WORLD + self.x0 as f64 * tile_m;
        let north_m = HALF_WORLD - self.y0 as f64 * tile_m;
        // Crop to the bbox in mercator metres.
        let (bw, bn) = lonlat_to_merc(bbox.west, bbox.north);
        let (be, bs) = lonlat_to_merc(bbox.east, bbox.south);
        let cx0 = ((bw - west_m) / s).floor().clamp(0.0, (mw - 1) as f64) as u32;
        let cx1 = ((be - west_m) / s).ceil().clamp(1.0, mw as f64) as u32;
        let cy0 = ((north_m - bn) / s).floor().clamp(0.0, (mh - 1) as f64) as u32;
        let cy1 = ((north_m - bs) / s).ceil().clamp(1.0, mh as f64) as u32;
        let (w, h) = (cx1 - cx0, cy1 - cy0);
        let mut data = vec![0.0f32; (w * h) as usize];
        for y in 0..h {
            let src_row = cy1 - 1 - y; // row 0 = south
            for x in 0..w {
                data[(y * w + x) as usize] = m[(src_row * mw + cx0 + x) as usize];
            }
        }
        let mut g = Grid {
            width: w,
            height: h,
            origin: [west_m + cx0 as f64 * s, north_m - cy1 as f64 * s],
            spacing: [s, s],
            crs: "EPSG:3857".into(),
            data,
            void_share: 0.0,
        };
        g.fill_voids();
        Ok(g)
    }
}

// ── GeoTIFF rasters (Copernicus tiles, OpenTopography responses) ───────────────────────────

/// A geographic (EPSG:4326) GeoTIFF in memory → a [`Grid`] (row 0 = south, degrees).
pub fn decode_geotiff(bytes: &[u8], what: &str) -> JdResult<Grid> {
    use tiff::tags::Tag;
    let bad = |e: &dyn std::fmt::Display| {
        JdError::new(
            ErrorCode::E_NETWORK,
            format!("{what}: not a readable GeoTIFF ({e})"),
        )
    };
    let mut dec = tiff::decoder::Decoder::new(Cursor::new(bytes)).map_err(|e| bad(&e))?;
    let (w, h) = dec.dimensions().map_err(|e| bad(&e))?;
    let f64s = |dec: &mut tiff::decoder::Decoder<Cursor<&[u8]>>, tag: Tag| -> Option<Vec<f64>> {
        dec.find_tag(tag).ok().flatten()?;
        dec.get_tag_f64_vec(tag).ok()
    };
    let scale = f64s(&mut dec, Tag::ModelPixelScaleTag).filter(|v| v.len() >= 2);
    let tie = f64s(&mut dec, Tag::ModelTiepointTag).filter(|v| v.len() >= 6);
    let nodata = dec
        .find_tag(Tag::GdalNodata)
        .ok()
        .flatten()
        .and_then(|_| dec.get_tag_ascii_string(Tag::GdalNodata).ok())
        .and_then(|s| s.trim().trim_end_matches('\0').parse::<f32>().ok());
    let (Some(scale), Some(tie)) = (scale, tie) else {
        return Err(JdError::new(
            ErrorCode::E_NETWORK,
            format!("{what}: GeoTIFF without pixel scale / tie point"),
        ));
    };
    let samples: Vec<f32> = match dec.read_image().map_err(|e| bad(&e))? {
        tiff::decoder::DecodingResult::F32(v) => v,
        tiff::decoder::DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
        tiff::decoder::DecodingResult::I16(v) => v.into_iter().map(f32::from).collect(),
        tiff::decoder::DecodingResult::U16(v) => v.into_iter().map(f32::from).collect(),
        tiff::decoder::DecodingResult::I32(v) => v.into_iter().map(|x| x as f32).collect(),
        tiff::decoder::DecodingResult::U8(v) => v.into_iter().map(f32::from).collect(),
        other => {
            return Err(JdError::new(
                ErrorCode::E_NETWORK,
                format!(
                    "{what}: unsupported sample type {:?}",
                    std::mem::discriminant(&other)
                ),
            ));
        }
    };
    if samples.len() != (w as usize) * (h as usize) {
        return Err(JdError::new(
            ErrorCode::E_NETWORK,
            format!("{what}: {} samples for {w}×{h}", samples.len()),
        ));
    }
    // Tie point (i, j) ↔ (x, y): the raster's north-west corner at (0, 0) for DEM products.
    let nw_x = tie[3] - tie[0] * scale[0];
    let nw_y = tie[4] + tie[1] * scale[1];
    let mut data = vec![0.0f32; samples.len()];
    for y in 0..h as usize {
        let src = h as usize - 1 - y;
        for x in 0..w as usize {
            let v = samples[src * w as usize + x];
            data[y * w as usize + x] = if nodata.is_some_and(|nd| v == nd) || !v.is_finite() {
                f32::NAN
            } else {
                v
            };
        }
    }
    let mut g = Grid {
        width: w,
        height: h,
        origin: [nw_x, nw_y - h as f64 * scale[1]],
        spacing: [scale[0], scale[1]],
        crs: "EPSG:4326".into(),
        data,
        void_share: 0.0,
    };
    g.fill_voids();
    Ok(g)
}

/// Mosaic geographic grids of equal spacing and crop to `bbox` (degrees).
pub fn mosaic_geographic(tiles: &[Grid], bbox: &Bbox) -> JdResult<Grid> {
    let first = tiles
        .first()
        .ok_or_else(|| JdError::new(ErrorCode::E_INTERNAL, "no tiles to mosaic"))?;
    let [sx, sy] = first.spacing;
    if tiles
        .iter()
        .any(|t| (t.spacing[0] - sx).abs() > 1e-12 || (t.spacing[1] - sy).abs() > 1e-12)
    {
        return Err(JdError::new(ErrorCode::E_NETWORK, "tiles with different spacings cannot be mosaicked").next("Copernicus tiles change their longitude spacing above 50° latitude; keep a bbox inside one band."));
    }
    let west = tiles
        .iter()
        .map(|t| t.origin[0])
        .fold(f64::INFINITY, f64::min);
    let south = tiles
        .iter()
        .map(|t| t.origin[1])
        .fold(f64::INFINITY, f64::min);
    let east = tiles
        .iter()
        .map(|t| t.origin[0] + t.width as f64 * sx)
        .fold(f64::NEG_INFINITY, f64::max);
    let north = tiles
        .iter()
        .map(|t| t.origin[1] + t.height as f64 * sy)
        .fold(f64::NEG_INFINITY, f64::max);
    let mw = ((east - west) / sx).round() as u32;
    let mh = ((north - south) / sy).round() as u32;
    let mut m = vec![f32::NAN; (mw as usize) * (mh as usize)];
    for t in tiles {
        let ox = ((t.origin[0] - west) / sx).round() as u32;
        let oy = ((t.origin[1] - south) / sy).round() as u32;
        for y in 0..t.height.min(mh.saturating_sub(oy)) {
            for x in 0..t.width.min(mw.saturating_sub(ox)) {
                m[((oy + y) * mw + ox + x) as usize] = t.data[(y * t.width + x) as usize];
            }
        }
    }
    let cx0 = ((bbox.west - west) / sx)
        .floor()
        .clamp(0.0, (mw - 1) as f64) as u32;
    let cx1 = ((bbox.east - west) / sx).ceil().clamp(1.0, mw as f64) as u32;
    let cy0 = ((bbox.south - south) / sy)
        .floor()
        .clamp(0.0, (mh - 1) as f64) as u32;
    let cy1 = ((bbox.north - south) / sy).ceil().clamp(1.0, mh as f64) as u32;
    let (w, h) = (cx1 - cx0, cy1 - cy0);
    let mut data = vec![0.0f32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            data[(y * w + x) as usize] = m[((cy0 + y) * mw + cx0 + x) as usize];
        }
    }
    let mut g = Grid {
        width: w,
        height: h,
        origin: [west + cx0 as f64 * sx, south + cy0 as f64 * sy],
        spacing: [sx, sy],
        crs: "EPSG:4326".into(),
        data,
        void_share: 0.0,
    };
    g.fill_voids();
    Ok(g)
}

/// Copernicus GLO-30 tile name for the 1° cell containing (lon, lat) — the cell's south-west
/// corner: `Copernicus_DSM_COG_10_N50_00_W125_00_DEM`.
pub fn copernicus_tile_name(lon_floor: i32, lat_floor: i32) -> String {
    let ns = if lat_floor >= 0 { 'N' } else { 'S' };
    let ew = if lon_floor >= 0 { 'E' } else { 'W' };
    format!(
        "Copernicus_DSM_COG_10_{ns}{:02}_00_{ew}{:03}_00_DEM",
        lat_floor.abs(),
        lon_floor.abs()
    )
}

/// The 1° tiles covering `bbox` as fetch requests (`base_url` = the AWS bucket).
pub fn copernicus_requests(bbox: &Bbox, base_url: &str) -> JdResult<Vec<Request>> {
    bbox.validate()?;
    let (la0, la1) = (
        bbox.south.floor() as i32,
        (bbox.north - 1e-9).floor() as i32,
    );
    let (lo0, lo1) = (bbox.west.floor() as i32, (bbox.east - 1e-9).floor() as i32);
    let count = (la1 - la0 + 1) as i64 * (lo1 - lo0 + 1) as i64;
    if count > 16 {
        return Err(JdError::invalid(
            "bbox",
            format!("{count} Copernicus tiles (1° each, ~25 MB); the cap is 16"),
        ));
    }
    let mut reqs = Vec::new();
    for la in la0..=la1 {
        for lo in lo0..=lo1 {
            let name = copernicus_tile_name(lo, la);
            reqs.push(Request {
                url: format!("{}/{name}/{name}.tif", base_url.trim_end_matches('/')),
                key: Some(format!("copernicus/{name}.tif")),
                expect_bytes: 25 << 20,
            });
        }
    }
    Ok(reqs)
}

/// The OpenTopography global DEM call for `bbox` (COP30; the key comes from the caller and is
/// never written anywhere).
pub fn opentopography_request(bbox: &Bbox, api_key: &str, base_url: &str) -> JdResult<Request> {
    bbox.validate()?;
    if api_key.trim().is_empty() || api_key.contains(['&', '/', ' ']) {
        return Err(JdError::invalid(
            "api_key",
            "an OpenTopography API key (yours; never shipped)",
        ));
    }
    if bbox.area_km2() > 450_000.0 {
        return Err(JdError::invalid(
            "bbox",
            "OpenTopography caps COP30 requests at 450 000 km²",
        ));
    }
    Ok(Request {
        url: format!(
            "{}/API/globaldem?demtype=COP30&south={:.6}&north={:.6}&west={:.6}&east={:.6}&outputFormat=GTiff&API_Key={api_key}",
            base_url.trim_end_matches('/'),
            bbox.south,
            bbox.north,
            bbox.west,
            bbox.east
        ),
        key: None, // the key is in the URL; never cache or log it
        expect_bytes: 8 << 20,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_maths_match_the_slippy_convention() {
        // Nairobi at z=12: tile 2466/2062 (OSM slippy map reference values).
        let (x, y) = tile_xy(36.82, -1.29, 12);
        assert_eq!((x.floor() as u32, y.floor() as u32), (2466, 2062));
        assert!((ground_resolution_m(0, 0.0) - 156_543.03).abs() < 0.1);
        assert_eq!(zoom_for(30.0, 0.0), 12, "30 m ≈ z12 at the equator (38 m)");
        assert_eq!(zoom_for(5.0, 0.0), 15, "capped at Terrarium's z15");
        let bbox = Bbox {
            south: -1.30,
            west: 36.80,
            north: -1.25,
            east: 36.86,
        };
        let (x0, y0, x1, y1) = tile_range(&bbox, 12);
        assert!(x0 <= 2466 && x1 >= 2466 && y0 <= 2062 && y1 >= 2062);
        assert!(
            (terrarium_m(128, 0, 0) - 0.0).abs() < 1e-6
                && (terrarium_m(128, 1, 128) - 1.5).abs() < 1e-6
        );
        assert!(bbox.validate().is_ok());
        assert!(
            Bbox {
                south: 1.0,
                west: 0.0,
                north: 0.0,
                east: 1.0
            }
            .validate()
            .is_err()
        );
        let b = Bbox::around(36.8, -1.3, 1000.0);
        assert!(
            (b.area_km2() - 4.0).abs() < 0.1,
            "2 km × 2 km: {}",
            b.area_km2()
        );
    }

    fn png_tile(f: impl Fn(u32, u32) -> f32) -> Vec<u8> {
        let mut img = image::RgbImage::new(256, 256);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let v = f(x, y) + 32768.0;
            let r = (v / 256.0).floor();
            let g = (v - r * 256.0).floor();
            let b = ((v - r * 256.0 - g) * 256.0).round().min(255.0);
            *p = image::Rgb([r as u8, g as u8, b as u8]);
        }
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn terrarium_mosaic_crops_to_the_bbox_with_row_0_south() {
        let bbox = Bbox {
            south: -1.30,
            west: 36.80,
            north: -1.25,
            east: 36.86,
        };
        let plan = TerrariumPlan::new(&bbox, 30.0, "http://127.0.0.1:1/t").unwrap();
        assert_eq!(plan.z, 12);
        assert!(plan.requests[0].url.starts_with("http://127.0.0.1:1/t/12/"));
        // Every tile: height = the mercator y of its row (north high), so the crop is checkable.
        let tiles: Vec<Vec<u8>> = (0..plan.tile_count())
            .map(|i| {
                let ty = plan.y0 + i as u32 / (plan.x1 - plan.x0 + 1);
                png_tile(move |_, py| 1000.0 - (ty * 256 + py) as f32 * 0.01)
            })
            .collect();
        let g = plan.mosaic(&bbox, &tiles).unwrap();
        assert_eq!(g.crs, "EPSG:3857");
        assert!(g.width > 100 && g.height > 50, "{}×{}", g.width, g.height);
        assert!((g.spacing[0] - 38.2185).abs() < 0.01, "{}", g.spacing[0]);
        let (bw, _) = lonlat_to_merc(bbox.west, 0.0);
        assert!(
            g.origin[0] <= bw && g.origin[0] + g.spacing[0] > bw - g.spacing[0],
            "origin at or just west of the bbox"
        );
        assert!(
            g.data[0] < g.data[((g.height - 1) * g.width) as usize],
            "row 0 is south (lower in this field)"
        );
        assert_eq!(g.void_share, 0.0);
    }

    fn geotiff(w: u32, h: u32, nw: [f64; 2], scale: f64, f: impl Fn(u32, u32) -> f32) -> Vec<u8> {
        use tiff::encoder::{TiffEncoder, colortype};
        use tiff::tags::Tag;
        let mut out = Cursor::new(Vec::new());
        let mut enc = TiffEncoder::new(&mut out).unwrap();
        let mut img = enc.new_image::<colortype::Gray32Float>(w, h).unwrap();
        img.encoder()
            .write_tag(Tag::ModelPixelScaleTag, &[scale, scale, 0.0][..])
            .unwrap();
        img.encoder()
            .write_tag(
                Tag::ModelTiepointTag,
                &[0.0, 0.0, 0.0, nw[0], nw[1], 0.0][..],
            )
            .unwrap();
        let data: Vec<f32> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| f(x, y))
            .collect();
        img.write_data(&data).unwrap();
        out.into_inner()
    }

    #[test]
    fn geotiff_tiles_decode_and_mosaic_in_degrees() {
        // Two 1° tiles side by side, 10 px each, height = lon + lat so the join is checkable.
        let t = |lon0: f64, lat0: f64| {
            geotiff(10, 10, [lon0, lat0 + 1.0], 0.1, move |x, y| {
                (lon0 + x as f64 * 0.1 + lat0 + 1.0 - y as f64 * 0.1) as f32
            })
        };
        let a = decode_geotiff(&t(36.0, -2.0), "a").unwrap();
        let b = decode_geotiff(&t(37.0, -2.0), "b").unwrap();
        assert_eq!((a.width, a.height, a.crs.as_str()), (10, 10, "EPSG:4326"));
        assert_eq!(a.origin, [36.0, -2.0]);
        assert!(
            (a.data[0] - (36.0 - 2.0 + 0.1)).abs() < 1e-4,
            "row 0 = south: {}",
            a.data[0]
        );
        let bbox = Bbox {
            south: -1.55,
            west: 36.55,
            north: -1.25,
            east: 37.35,
        };
        let g = mosaic_geographic(&[a, b], &bbox).unwrap();
        assert_eq!(g.width, 9, "36.5..37.4 at 0.1° (floor/ceil)");
        assert_eq!(g.height, 4);
        assert!((g.origin[0] - 36.5).abs() < 1e-9 && (g.origin[1] + 1.6).abs() < 1e-9);
        assert_eq!(g.void_share, 0.0, "the join has no voids");
        assert_eq!(
            copernicus_tile_name(-125, 50),
            "Copernicus_DSM_COG_10_N50_00_W125_00_DEM"
        );
        assert_eq!(
            copernicus_tile_name(36, -2),
            "Copernicus_DSM_COG_10_S02_00_E036_00_DEM"
        );
        let reqs =
            copernicus_requests(&bbox, "https://copernicus-dem-30m.s3.amazonaws.com").unwrap();
        assert_eq!(reqs.len(), 2);
        assert!(reqs[0].url.ends_with(
            "Copernicus_DSM_COG_10_S02_00_E036_00_DEM/Copernicus_DSM_COG_10_S02_00_E036_00_DEM.tif"
        ));
        let ot = opentopography_request(&bbox, "k3y", "https://portal.opentopography.org").unwrap();
        assert!(
            ot.url.contains("demtype=COP30&south=-1.550000")
                && ot.url.ends_with("API_Key=k3y")
                && ot.key.is_none()
        );
        assert!(opentopography_request(&bbox, "", "https://x").is_err());
    }
}
