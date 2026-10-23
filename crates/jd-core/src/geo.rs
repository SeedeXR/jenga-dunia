//! The `geo.*` ops (ADR-0014): a DEM for a lon/lat box from Terrarium, Copernicus or
//! OpenTopography placed into `assets/` as a georeferenced f32 GeoTIFF, OSM features from
//! Overpass as a GeoJSON asset, the project's attribution list, and the journaled mutation
//! that grows it. Every fetch goes through `jd_geo::fetch::Fetcher` under the policy — the
//! provider's host only, the project's `cache/geo/` directory, offline on request.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use jd_geo::dem::{Bbox, Grid, Provider, TerrariumPlan};
use jd_geo::fetch::{DiskCache, Fetcher, Policy, Request, Stats, UreqHttp};
use jd_geo::osm::{Counts, Kind};
use jd_types::{ErrorCode, JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::envelope::Warning;
use crate::ops::{Context, Registry, Scope};
use crate::project::{Attribution, Project};

/// Where a request points: a box, or a centre and a radius.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct Area {
    /// Lon/lat box in degrees.
    #[serde(default)]
    pub bbox: Option<Bbox>,
    /// `[lon, lat]` of the centre (with `radius_m`).
    #[serde(default)]
    pub center: Option<[f64; 2]>,
    /// Half-size of the box around `center`, metres.
    #[serde(default)]
    pub radius_m: Option<f64>,
}

impl Area {
    fn bbox(&self) -> JdResult<Bbox> {
        let b = match (self.bbox, self.center, self.radius_m) {
            (Some(b), None, None) => b,
            (None, Some([lon, lat]), Some(r)) if r.is_finite() && r > 0.0 => {
                Bbox::around(lon, lat, r)
            }
            _ => {
                return Err(JdError::invalid(
                    "area",
                    "give `bbox` {south, west, north, east}, or `center` [lon, lat] with `radius_m`",
                ));
            }
        };
        b.validate()?;
        Ok(b)
    }
}

/// `geo.fetch_dem` params.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FetchDem {
    /// The lon/lat area.
    #[serde(flatten)]
    pub area: Area,
    /// `terrarium` (default; no key), `copernicus` (1° tiles, ~25 MB each) or `opentopography`
    /// (needs `api_key`).
    #[serde(default = "default_provider")]
    pub provider: Provider,
    /// Target ground resolution in metres (Terrarium picks the nearest zoom; the others are 30 m).
    #[serde(default = "default_res")]
    #[schemars(range(min = 1.0, max = 10000.0))]
    pub res_m: f64,
    /// Asset file name (`.tif`); default `dem-<provider>-<lat>_<lon>.tif`.
    #[serde(default)]
    pub name: Option<String>,
    /// Replace an existing asset of that name.
    #[serde(default)]
    pub overwrite: bool,
    /// Your OpenTopography key (or `JD_OPENTOPOGRAPHY_KEY`); never stored.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Serve the cache only (also `JD_OFFLINE=1`).
    #[serde(default)]
    pub offline: bool,
    /// Tests only: a loopback server standing in for the provider.
    #[serde(default)]
    pub base_url: Option<String>,
}

fn default_provider() -> Provider {
    Provider::Terrarium
}
fn default_res() -> f64 {
    30.0
}

/// `geo.fetch_dem` result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FetchDemResult {
    /// The asset's file name (what `jd.input.heightmap@1 {path}` takes).
    pub asset: String,
    /// Where it was written.
    pub path: PathBuf,
    /// The provider used.
    pub provider: Provider,
    /// Raster size.
    pub px: [u32; 2],
    /// Sample spacing in the raster's CRS units.
    pub spacing: [f64; 2],
    /// CRS of the raster (`EPSG:3857` for Terrarium, `EPSG:4326` otherwise).
    pub crs: String,
    /// Outer south-west corner in `crs` units.
    pub origin: [f64; 2],
    /// Height range, metres.
    pub height_m: [f32; 2],
    /// Share of nodata samples filled, 0..1.
    pub void_share: f32,
    /// Fetch statistics.
    pub fetch: Stats,
    /// The notice appended to the project.
    pub attribution: Attribution,
    /// How to use it.
    pub next: String,
}

/// `geo.fetch_features` params.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FetchFeatures {
    /// The lon/lat area.
    #[serde(flatten)]
    pub area: Area,
    /// Feature families; default all four.
    #[serde(default)]
    pub kinds: Option<Vec<Kind>>,
    /// Asset file name (`.geojson`); default `osm-<lat>_<lon>.geojson`.
    #[serde(default)]
    pub name: Option<String>,
    /// Replace an existing asset of that name.
    #[serde(default)]
    pub overwrite: bool,
    /// Serve the cache only (also `JD_OFFLINE=1`).
    #[serde(default)]
    pub offline: bool,
    /// Tests only: a loopback server standing in for Overpass.
    #[serde(default)]
    pub base_url: Option<String>,
}

/// `geo.fetch_features` result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FetchFeaturesResult {
    /// The asset's file name.
    pub asset: String,
    /// Where it was written.
    pub path: PathBuf,
    /// Features per kind.
    pub counts: Counts,
    /// Size of the GeoJSON written.
    pub bytes: u64,
    /// Fetch statistics.
    pub fetch: Stats,
    /// The notice appended to the project.
    pub attribution: Attribution,
}

/// `geo.attributions` result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AttributionsResult {
    /// The notices on the project.
    pub attribution: Vec<Attribution>,
    /// The `ATTRIBUTION.txt` text exports carry.
    pub text: String,
}

/// Empty params.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

const OVERPASS_HOST: &str = "overpass-api.de";

/// The policy's view of one provider: its host, and the cache and offline state of the
/// project.
fn fetcher(root: &Path, cache_limit: u64, hosts: &[&str], offline: bool) -> Fetcher {
    let policy = Policy {
        allow_hosts: hosts.iter().map(|h| h.to_string()).collect(),
        offline: offline || std::env::var_os("JD_OFFLINE").is_some(),
        ..Policy::default()
    };
    let cache = DiskCache::new(root.join("cache").join("geo"), cache_limit);
    Fetcher::new(
        Arc::new(UreqHttp::new(&policy.user_agent)),
        policy,
        Some(cache),
    )
}

fn asset_path(p: &Project, name: &str, overwrite: bool) -> JdResult<PathBuf> {
    let mut comps = Path::new(name).components();
    if name.contains(':')
        || !matches!(
            (comps.next(), comps.next()),
            (Some(std::path::Component::Normal(_)), None)
        )
    {
        return Err(JdError::invalid("name", "a plain file name"));
    }
    let path = p.root().join("assets").join(name);
    if path.exists() && !overwrite {
        return Err(
            JdError::new(ErrorCode::E_INPUT_INVALID, format!("assets/{name} exists"))
                .next("Pass `overwrite: true` or another `name`."),
        );
    }
    std::fs::create_dir_all(p.root().join("assets"))?;
    Ok(path)
}

fn stem(bbox: &Bbox) -> String {
    let (lon, lat) = bbox.centre();
    format!("{:.3}_{:.3}", lat, lon).replace('-', "m")
}

/// The attribution text exports carry.
pub fn attribution_text(list: &[Attribution]) -> String {
    let mut s = String::from("Data sources and required notices\n\n");
    for a in list {
        s.push_str(&format!("{}\n{}\n", a.source, a.text));
        if !a.licence.is_empty() {
            s.push_str(&format!("Licence: {}\n", a.licence));
        }
        if !a.url.is_empty() {
            s.push_str(&format!("{}\n", a.url));
        }
        s.push('\n');
    }
    s
}

/// Write `ATTRIBUTION.txt` into `dir` when the project owes any notice; returns its path.
pub fn write_attribution_txt(p: &Project, dir: &Path) -> JdResult<Option<PathBuf>> {
    if p.doc.attribution.is_empty() {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join("ATTRIBUTION.txt");
    std::fs::write(&path, attribution_text(&p.doc.attribution))?;
    Ok(Some(path))
}

fn provider_attribution(p: Provider) -> Attribution {
    let (source, text, url, licence) = p.attribution();
    Attribution {
        source: source.into(),
        text: text.into(),
        url: url.into(),
        licence: licence.into(),
    }
}

fn osm_attribution() -> Attribution {
    Attribution {
        source: "OpenStreetMap".into(),
        text: "© OpenStreetMap contributors, ODbL — https://www.openstreetmap.org/copyright. The feature file is a derivative database under the ODbL and is kept separately labelled.".into(),
        url: "https://www.openstreetmap.org/copyright".into(),
        licence: "ODbL 1.0".into(),
    }
}

fn add_attribution(ctx: &Context, a: &Attribution) -> JdResult<()> {
    ctx.with_project(|p| {
        if p.doc
            .attribution
            .iter()
            .any(|x| x.source == a.source && x.text == a.text)
        {
            return Ok(());
        }
        crate::ops::builtin::mutate(p, "project.add_attribution", serde_json::to_value(a)?)
    })
}

fn grid_to_geotiff(grid: &Grid, path: &Path) -> JdResult<()> {
    let mut hf = jd_terrain::Heightfield::new(grid.width, grid.height, 64, 0)?;
    hf.fill_with(|x, y| grid.data[(y * grid.width + x) as usize]);
    let geo = jd_terrain::GeoRef {
        origin: grid.origin,
        spacing: grid.spacing,
        crs: Some(grid.crs.clone()),
    };
    let opts = jd_terrain::ExportOptions {
        format: jd_terrain::HeightmapFormat::Tiff32,
        flip_y: None,
        range_m: None,
        georeference: true,
    };
    jd_terrain::export_heightmap_geo(&hf, path, &opts, Some(&geo))?;
    Ok(())
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Register the `geo.*` ops.
pub fn register(r: &mut Registry) {
    r.register::<FetchDem, FetchDemResult, _>(
        "geo.fetch_dem",
        "Fetch a DEM for a lon/lat box (Terrarium tiles, Copernicus GLO-30, or OpenTopography with your key) into assets/ as a georeferenced f32 GeoTIFF, cached under cache/geo, with the provider's attribution recorded — then `jd.input.heightmap@1 {path, georeference: true}` places it",
        Scope::NetworkGeo,
        |ctx, p| {
            let bbox = p.area.bbox()?;
            let (root, cache_limit) = ctx.with_project(|pr| Ok((pr.root().to_path_buf(), pr.doc.settings.cache_limit_bytes)))?;
            let name = p.name.clone().unwrap_or_else(|| format!("dem-{}-{}.tif", serde_json::to_value(p.provider).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default(), stem(&bbox)));
            let path = ctx.with_project(|pr| asset_path(pr, &name, p.overwrite))?;
            let job = ctx.jobs.start("geo.fetch_dem");
            let cancel = job.ctl.cancel.clone();
            let progress = job.ctl.progress.clone();
            let mut tick = |done: usize, total: usize| progress(done, total, "tiles");
            let f = fetcher(&root, cache_limit, &[p.provider.host()], p.offline);
            // `JD_GEO_TERRARIUM_URL` etc. let the e2e suite point a provider at a loopback server.
            let base = |default: String| p.base_url.clone().or_else(|| std::env::var(format!("JD_GEO_{}_URL", format!("{:?}", p.provider).to_uppercase())).ok()).unwrap_or(default);
            let (grid, stats) = match p.provider {
                Provider::Terrarium => {
                    let plan = TerrariumPlan::new(&bbox, p.res_m, &base(format!("https://{}/terrarium", Provider::Terrarium.host())))?;
                    let (tiles, stats) = f.fetch_all(&plan.requests, &cancel, &mut tick)?;
                    let bytes: Vec<Vec<u8>> = tiles.into_iter().map(|t| t.bytes).collect();
                    (plan.mosaic(&bbox, &bytes)?, stats)
                }
                Provider::Copernicus => {
                    let reqs = jd_geo::dem::copernicus_requests(&bbox, &base(format!("https://{}", Provider::Copernicus.host())))?;
                    let (tiles, stats) = f.fetch_all(&reqs, &cancel, &mut tick)?;
                    let grids = tiles
                        .iter()
                        .zip(&reqs)
                        .map(|(t, r)| jd_geo::dem::decode_geotiff(&t.bytes, &r.url))
                        .collect::<JdResult<Vec<_>>>()?;
                    (jd_geo::dem::mosaic_geographic(&grids, &bbox)?, stats)
                }
                Provider::Opentopography => {
                    let key = p.api_key.clone().or_else(|| std::env::var("JD_OPENTOPOGRAPHY_KEY").ok()).unwrap_or_default();
                    let req = jd_geo::dem::opentopography_request(&bbox, &key, &base(format!("https://{}", Provider::Opentopography.host())))?;
                    let (tiles, stats) = f.fetch_all(std::slice::from_ref(&req), &cancel, &mut tick)?;
                    let grid = jd_geo::dem::decode_geotiff(&tiles[0].bytes, "OpenTopography COP30")?;
                    (jd_geo::dem::mosaic_geographic(std::slice::from_ref(&grid), &bbox)?, stats)
                }
            };
            drop(job);
            grid_to_geotiff(&grid, &path)?;
            let attribution = provider_attribution(p.provider);
            add_attribution(ctx, &attribution)?;
            let (lo, hi) = grid.stats();
            let mut warnings = Vec::new();
            if grid.void_share > 0.0 {
                warnings.push(Warning::new("W_DEM_VOIDS", format!("{:.1} % of the samples were nodata and were filled with the lowest valid height", grid.void_share * 100.0)));
            }
            Ok((
                FetchDemResult {
                    asset: name.clone(),
                    path,
                    provider: p.provider,
                    px: [grid.width, grid.height],
                    spacing: grid.spacing,
                    crs: grid.crs.clone(),
                    origin: grid.origin,
                    height_m: [lo, hi],
                    void_share: grid.void_share,
                    fetch: stats,
                    attribution,
                    next: format!("graph.add_node {{kind: \"jd.input.heightmap@1\", params: {{path: \"{name}\", georeference: true}}}} — set the world's `crs` (a UTM zone) and `origin` first so the DEM lands in metres"),
                },
                warnings,
            ))
        },
    );
    r.register::<FetchFeatures, FetchFeaturesResult, _>(
        "geo.fetch_features",
        "Fetch OpenStreetMap roads, waterways, water bodies and buildings for a lon/lat box through the Overpass API into assets/ as a GeoJSON file (ODbL; kept separately labelled), with the OSM attribution recorded",
        Scope::NetworkGeo,
        |ctx, p| {
            let bbox = p.area.bbox()?;
            if bbox.area_km2() > 2_500.0 {
                return Err(JdError::invalid("area", "Overpass queries are capped at 2 500 km² here; split a larger area"));
            }
            let kinds = p.kinds.clone().unwrap_or_else(|| Kind::all().to_vec());
            if kinds.is_empty() {
                return Err(JdError::invalid("kinds", "at least one of roads, waterways, water, buildings"));
            }
            let (root, cache_limit) = ctx.with_project(|pr| Ok((pr.root().to_path_buf(), pr.doc.settings.cache_limit_bytes)))?;
            let name = p.name.clone().unwrap_or_else(|| format!("osm-{}.geojson", stem(&bbox)));
            let path = ctx.with_project(|pr| asset_path(pr, &name, p.overwrite))?;
            let query = jd_geo::osm::query(&bbox, &kinds, 60);
            let base = p.base_url.clone().or_else(|| std::env::var("JD_GEO_OVERPASS_URL").ok()).unwrap_or_else(|| format!("https://{OVERPASS_HOST}/api/interpreter"));
            let req = Request {
                url: format!("{base}?data={}", percent_encode(&query)),
                key: Some(format!("overpass/{}-{}.json", stem(&bbox), kinds.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>().join("+").to_lowercase())),
                expect_bytes: 2 << 20,
            };
            let job = ctx.jobs.start("geo.fetch_features");
            let f = fetcher(&root, cache_limit, &[OVERPASS_HOST], p.offline);
            let (got, stats) = f.fetch_all(std::slice::from_ref(&req), &job.ctl.cancel.clone(), &mut |_, _| {})?;
            drop(job);
            let (geojson, counts) = jd_geo::osm::to_geojson(&got[0].bytes, &kinds)?;
            std::fs::write(&path, &geojson)?;
            let attribution = osm_attribution();
            add_attribution(ctx, &attribution)?;
            Ok((FetchFeaturesResult { asset: name, path, counts, bytes: geojson.len() as u64, fetch: stats, attribution }, Vec::new()))
        },
    );
    r.register::<NoParams, AttributionsResult, _>(
        "geo.attributions",
        "The project's consolidated data-source notices (what every export's ATTRIBUTION.txt carries)",
        Scope::ProjectRead,
        |ctx, _| {
            let list = ctx.with_project(|p| Ok(p.doc.attribution.clone()))?;
            Ok((AttributionsResult { text: attribution_text(&list), attribution: list }, Vec::new()))
        },
    );
    r.register::<Attribution, AttributionsResult, _>(
        "project.add_attribution",
        "Record a data-source notice on the project (journaled; deduplicated by source + text) so exports carry it",
        Scope::ProjectWrite,
        |ctx, a| {
            if a.source.trim().is_empty() || a.text.trim().is_empty() {
                return Err(JdError::invalid("attribution", "`source` and `text` are required"));
            }
            add_attribution(ctx, &a)?;
            let list = ctx.with_project(|p| Ok(p.doc.attribution.clone()))?;
            Ok((AttributionsResult { text: attribution_text(&list), attribution: list }, Vec::new()))
        },
    );
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A loopback HTTP server: Terrarium tiles for `/t/{z}/{x}/{y}.png` (height = 1000 + y row
    /// of the tile, so the mosaic is checkable) and a canned Overpass answer for
    /// `/api/interpreter?data=…`. Returns its base URL.
    pub(crate) fn tile_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let target = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let (ctype, body): (&str, Vec<u8>) = if target.starts_with("/t/") {
                    let parts: Vec<&str> = target
                        .trim_start_matches("/t/")
                        .trim_end_matches(".png")
                        .split('/')
                        .collect();
                    let ty: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let mut img = image::RgbImage::new(256, 256);
                    for (_, py, p) in img.enumerate_pixels_mut() {
                        let v = 1000.0 + (ty * 256 + py) as f32 * 0.01 + 32768.0;
                        let r = (v / 256.0).floor();
                        let g = (v - r * 256.0).floor();
                        *p = image::Rgb([r as u8, g as u8, 0]);
                    }
                    let mut out = Vec::new();
                    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                        .expect("png");
                    ("image/png", out)
                } else if target.starts_with("/api/interpreter") {
                    let json = serde_json::json!({ "elements": [
                        { "type": "way", "id": 1, "tags": { "highway": "primary" }, "geometry": [ { "lat": -1.29, "lon": 36.81 }, { "lat": -1.28, "lon": 36.82 } ] },
                        { "type": "way", "id": 2, "tags": { "waterway": "river" }, "geometry": [ { "lat": -1.30, "lon": 36.80 }, { "lat": -1.27, "lon": 36.85 } ] }
                    ] });
                    ("application/json", serde_json::to_vec(&json).expect("json"))
                } else {
                    ("text/plain", b"not found".to_vec())
                };
                // A conditional GET with our ETag gets a 304, as the real buckets do.
                let revalidated = req.lines().any(|l| {
                    l.to_ascii_lowercase().starts_with("if-none-match:") && l.contains("\"t1\"")
                });
                let (status, body) = match (ctype, revalidated) {
                    ("text/plain", _) => ("404 Not Found", body),
                    (_, true) => ("304 Not Modified", Vec::new()),
                    _ => ("200 OK", body),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nETag: \"t1\"\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn fetch_dem_places_a_georeferenced_asset_records_the_notice_and_feeds_the_heightmap_node() {
        let base = tile_server();
        let dir = tempfile::tempdir().expect("tmp");
        let reg = crate::ops::Registry::with_builtins();
        let ctx = Context::default();
        let call = |op: &str, params: serde_json::Value| {
            reg.invoke(&ctx, crate::envelope::Request::new(op, params))
        };
        let r = call(
            "project.create",
            serde_json::json!({ "path": dir.path().join("Geo.jdproj"), "name": "Geo" }),
        );
        assert_eq!(r.status, crate::envelope::Status::Ok, "{:?}", r.error);
        let r = call(
            "geo.fetch_dem",
            serde_json::json!({ "bbox": { "south": -1.30, "west": 36.80, "north": -1.25, "east": 36.86 }, "res_m": 30, "base_url": format!("{base}/t") }),
        );
        assert_eq!(r.status, crate::envelope::Status::Ok, "{:?}", r.error);
        let res = r.result.expect("result");
        assert_eq!(res["crs"], "EPSG:3857");
        assert_eq!(res["asset"], "dem-terrarium-m1.275_36.830.tif");
        assert!(res["fetch"]["requests"].as_u64().unwrap() >= 9);
        assert_eq!(res["fetch"]["cache_hits"], 0);
        let path = dir
            .path()
            .join("Geo.jdproj/assets/dem-terrarium-m1.275_36.830.tif");
        assert!(path.is_file());
        let tags = jd_terrain::geotiff_tags(&path)
            .expect("tags")
            .expect("geo tags");
        assert_eq!(tags.epsg, Some(3857));
        assert!((tags.pixel_scale.unwrap()[0] - 38.2185).abs() < 0.01);
        // The notice is on the project, journaled, and deduplicated.
        let a = call("geo.attributions", serde_json::json!({}));
        let list = a.result.unwrap();
        assert_eq!(list["attribution"].as_array().unwrap().len(), 1);
        assert!(
            list["text"]
                .as_str()
                .unwrap()
                .contains("registry.opendata.aws/terrain-tiles")
        );
        let r2 = call(
            "geo.fetch_dem",
            serde_json::json!({ "bbox": { "south": -1.30, "west": 36.80, "north": -1.25, "east": 36.86 }, "res_m": 30, "base_url": format!("{base}/t"), "overwrite": true }),
        );
        assert_eq!(r2.status, crate::envelope::Status::Ok, "{:?}", r2.error);
        assert_eq!(
            r2.result.unwrap()["fetch"]["cache_hits"],
            res["fetch"]["requests"],
            "second fetch: every tile from the cache"
        );
        assert_eq!(
            call("geo.attributions", serde_json::json!({}))
                .result
                .unwrap()["attribution"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        // Same name without overwrite is refused; offline with the cache still works.
        assert_eq!(call("geo.fetch_dem", serde_json::json!({ "bbox": { "south": -1.30, "west": 36.80, "north": -1.25, "east": 36.86 }, "base_url": format!("{base}/t") })).status, crate::envelope::Status::Error);
        let off = call(
            "geo.fetch_dem",
            serde_json::json!({ "bbox": { "south": -1.30, "west": 36.80, "north": -1.25, "east": 36.86 }, "base_url": format!("{base}/t"), "name": "off.tif", "offline": true }),
        );
        assert_eq!(off.status, crate::envelope::Status::Ok, "{:?}", off.error);
        let miss = call(
            "geo.fetch_dem",
            serde_json::json!({ "bbox": { "south": 10.0, "west": 10.0, "north": 10.05, "east": 10.05 }, "base_url": format!("{base}/t"), "name": "miss.tif", "offline": true }),
        );
        assert_eq!(miss.error.map(|e| e.code), Some(ErrorCode::E_NETWORK));
        // A host off the allowlist is refused before any request.
        let bad = call(
            "geo.fetch_dem",
            serde_json::json!({ "bbox": { "south": -1.30, "west": 36.80, "north": -1.25, "east": 36.86 }, "base_url": "https://evil.example.com/t", "name": "evil.tif" }),
        );
        assert_eq!(bad.error.map(|e| e.code), Some(ErrorCode::E_NETWORK));
        // The world is placed in the DEM's CRS at its origin, so the node lands it in metres.
        let set = call(
            "project.set_settings",
            serde_json::json!({ "crs": "EPSG:3857", "origin": [res["origin"][0].as_f64().unwrap(), res["origin"][1].as_f64().unwrap()] }),
        );
        assert_eq!(set.status, crate::envelope::Status::Ok, "{:?}", set.error);
        // The heightmap node reads the asset; the test tiles encode 1000 + 0.01 × (tile row × 256 + y),
        // and this bbox sits on tile rows 2061–2063 at z12, so the heights are ~6276–6284 m.
        let n = call(
            "graph.add_node",
            serde_json::json!({ "kind": "jd.input.heightmap@1", "params": { "path": "dem-terrarium-m1.275_36.830.tif", "georeference": true } }),
        );
        assert_eq!(n.status, crate::envelope::Status::Ok, "{:?}", n.error);
        let id = n.result.unwrap()["id"].as_str().unwrap().to_string();
        let out = call("graph.list", serde_json::json!({})).result.unwrap();
        let height_out = out["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"].as_str().unwrap().starts_with("jd.output"))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(call("graph.connect", serde_json::json!({ "from": id, "from_port": "out", "to": height_out, "to_port": "in" })).status, crate::envelope::Status::Ok);
        let ev = call("graph.evaluate", serde_json::json!({}));
        assert_eq!(ev.status, crate::envelope::Status::Ok, "{:?}", ev.error);
        let st = call("terrain.stats", serde_json::json!({ "layer": "height" }))
            .result
            .unwrap();
        let (lo, hi) = (
            st["stats"]["min"].as_f64().unwrap(),
            st["stats"]["max"].as_f64().unwrap(),
        );
        assert!(lo > 6270.0 && hi < 6290.0 && hi - lo > 1.0, "{lo}..{hi}");
        // Exports carry ATTRIBUTION.txt.
        let ex = call(
            "terrain.export_heightmap",
            serde_json::json!({ "layer": "height", "path": "h.exr", "format": "exr" }),
        );
        assert_eq!(ex.status, crate::envelope::Status::Ok, "{:?}", ex.error);
        let txt = std::fs::read_to_string(dir.path().join("Geo.jdproj/exports/ATTRIBUTION.txt"))
            .expect("ATTRIBUTION.txt");
        assert!(txt.contains("AWS Terrain Tiles"));
        // Features.
        let fr = call(
            "geo.fetch_features",
            serde_json::json!({ "center": [36.83, -1.28], "radius_m": 2000, "kinds": ["roads", "waterways"], "base_url": format!("{base}/api/interpreter") }),
        );
        assert_eq!(fr.status, crate::envelope::Status::Ok, "{:?}", fr.error);
        let fres = fr.result.unwrap();
        assert_eq!(fres["counts"]["roads"], 1);
        assert_eq!(fres["counts"]["waterways"], 1);
        assert!(
            dir.path()
                .join("Geo.jdproj/assets")
                .join(fres["asset"].as_str().unwrap())
                .is_file()
        );
        assert_eq!(
            call("geo.attributions", serde_json::json!({}))
                .result
                .unwrap()["attribution"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        // Reopen: the notices survive through project.json.
        call("project.save", serde_json::json!({}));
        let ctx2 = Context::default();
        let r = reg.invoke(
            &ctx2,
            crate::envelope::Request::new(
                "project.open",
                serde_json::json!({ "path": dir.path().join("Geo.jdproj") }),
            ),
        );
        assert_eq!(r.status, crate::envelope::Status::Ok);
        let a = reg.invoke(
            &ctx2,
            crate::envelope::Request::new("geo.attributions", serde_json::json!({})),
        );
        assert_eq!(
            a.result.unwrap()["attribution"].as_array().unwrap().len(),
            2
        );
    }
}
