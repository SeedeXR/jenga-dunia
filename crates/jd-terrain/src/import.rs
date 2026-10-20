//! Foreign heightmaps (no Jenga Dunia sidecar): PNG 8/16-bit, EXR, TIFF (f32 or u16), RAW r16/r32.
//! Decoded to f32 metres and resampled (bilinear) onto the project's world grid so any DEM or
//! engine heightmap can drive a graph.
//!
//! Metres come from `range_m` (quantised sources) or the file values (float sources). GeoTIFF
//! files contribute their nodata value (`GDAL_NODATA`) and ground geometry (pixel scale, tie point)
//! through [`geotiff_tags`]; nodata cells are filled with the lowest valid height.
//!
//! ponytail: no projections / CRS transforms (those arrive with `jd-geo`); nodata fill is a flat
//! floor, nearest-valid inpainting when real DEM voids show up.

use std::path::Path;

use crate::Heightfield;
use jd_types::{ErrorCode, JdError, JdResult, WorldSettings};
use serde::{Deserialize, Serialize};

/// How to interpret a foreign file.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForeignImport {
    /// Height range the full 16-bit span maps to (quantised PNG/TIFF/r16); ignored for float data.
    #[serde(default)]
    pub range_m: Option<[f32; 2]>,
    /// Width and height in pixels — required for headerless RAW files.
    #[serde(default)]
    pub size_px: Option<[u32; 2]>,
    /// Row 0 of the file is north (true, the usual image convention) or south (the field convention).
    #[serde(default = "yes")]
    pub north_up: bool,
    /// Sample value meaning "no data" (filled with the lowest valid height). Default: the GeoTIFF
    /// `GDAL_NODATA` tag when present, otherwise none; non-finite samples are always treated as nodata.
    #[serde(default)]
    pub nodata: Option<f32>,
}

/// Georeferencing read from a GeoTIFF (what the file says, no reprojection).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GeoTags {
    /// Ground size of one pixel, [x, y], in the file's CRS units (`ModelPixelScaleTag`).
    pub pixel_scale: Option<[f64; 2]>,
    /// Raster → model tie point `[i, j, k, x, y, z]` (`ModelTiepointTag`, first point).
    pub tiepoint: Option<[f64; 6]>,
    /// `GDAL_NODATA` value.
    pub nodata: Option<f32>,
}

/// GeoTIFF tags of a `.tif`/`.tiff`; `None` for other extensions or when the file carries none.
pub fn geotiff_tags(path: &Path) -> JdResult<Option<GeoTags>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if !matches!(ext.as_deref(), Some("tif" | "tiff")) {
        return Ok(None);
    }
    use tiff::tags::Tag;
    let mut dec = tiff::decoder::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?))
        .map_err(|e| corrupt("TIFF read failed", e))?;
    let f64s = |dec: &mut tiff::decoder::Decoder<_>, tag: Tag| -> JdResult<Option<Vec<f64>>> {
        match dec.find_tag(tag).map_err(|e| corrupt("TIFF tag", e))? {
            Some(_) => Ok(Some(
                dec.get_tag_f64_vec(tag)
                    .map_err(|e| corrupt("TIFF tag", e))?,
            )),
            None => Ok(None),
        }
    };
    let pixel_scale =
        f64s(&mut dec, Tag::ModelPixelScaleTag)?.and_then(|v| (v.len() >= 2).then(|| [v[0], v[1]]));
    let tiepoint = f64s(&mut dec, Tag::ModelTiepointTag)?
        .and_then(|v| (v.len() >= 6).then(|| [v[0], v[1], v[2], v[3], v[4], v[5]]));
    let nodata = match dec
        .find_tag(Tag::GdalNodata)
        .map_err(|e| corrupt("TIFF tag", e))?
    {
        Some(_) => dec
            .get_tag_ascii_string(Tag::GdalNodata)
            .ok()
            .and_then(|s| s.trim().trim_end_matches('\0').parse::<f32>().ok()),
        None => None,
    };
    if pixel_scale.is_none() && tiepoint.is_none() && nodata.is_none() {
        return Ok(None);
    }
    Ok(Some(GeoTags {
        pixel_scale,
        tiepoint,
        nodata,
    }))
}

/// Replace nodata / non-finite samples with the lowest valid height. Errors when nothing is valid.
fn fill_nodata(data: &mut [f32], nodata: Option<f32>) -> JdResult<usize> {
    let is_void = |v: f32| !v.is_finite() || nodata.is_some_and(|n| v == n);
    let floor = data
        .iter()
        .copied()
        .filter(|v| !is_void(*v))
        .fold(f32::INFINITY, f32::min);
    if !floor.is_finite() {
        return Err(corrupt("heightmap", "every sample is nodata"));
    }
    let mut n = 0;
    for v in data.iter_mut() {
        if is_void(*v) {
            *v = floor;
            n += 1;
        }
    }
    Ok(n)
}
fn yes() -> bool {
    true
}

fn corrupt(what: &str, e: impl std::fmt::Display) -> JdError {
    JdError::new(ErrorCode::E_FORMAT_CORRUPT, format!("{what}: {e}"))
}

fn dequant(q: &[u16], range: [f32; 2]) -> Vec<f32> {
    let k = (range[1] - range[0]) / 65535.0;
    q.iter().map(|&v| range[0] + v as f32 * k).collect()
}

/// Decode a file to (samples row-major, width, height). Quantised sources need `range_m`.
/// Nodata samples (see [`ForeignImport::nodata`]) are filled with the lowest valid height.
pub fn read_foreign(path: &Path, opts: &ForeignImport) -> JdResult<(Vec<f32>, u32, u32)> {
    let (mut data, w, h) = decode(path, opts)?;
    let nodata = match opts.nodata {
        Some(n) => Some(n),
        None => geotiff_tags(path)?.and_then(|t| t.nodata),
    };
    fill_nodata(&mut data, nodata)?;
    Ok((data, w, h))
}

fn decode(path: &Path, opts: &ForeignImport) -> JdResult<(Vec<f32>, u32, u32)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let need_range = || {
        opts.range_m.ok_or_else(|| {
            JdError::invalid(
                "range_m",
                "quantised heightmaps need the height range the 16-bit span maps to",
            )
            .next("Pass range_m: [min_m, max_m] (e.g. the engine's terrain height).")
        })
    };
    match ext.as_str() {
        "png" => {
            let img = image::open(path)
                .map_err(|e| corrupt("PNG read failed", e))?
                .into_luma16();
            let (w, h) = img.dimensions();
            Ok((dequant(img.as_raw(), need_range()?), w, h))
        }
        "exr" => {
            use exr::prelude::*;
            let img =
                read_all_flat_layers_from_file(path).map_err(|e| corrupt("EXR read failed", e))?;
            let layer = img
                .layer_data
                .first()
                .ok_or_else(|| corrupt("EXR", "no layers"))?;
            let ch = layer
                .channel_data
                .list
                .iter()
                .find(|c| c.name.eq("Y") || c.name.eq("R"))
                .or_else(|| layer.channel_data.list.first())
                .ok_or_else(|| corrupt("EXR", "no channels"))?;
            let size = layer.size;
            Ok((
                ch.sample_data.values_as_f32().collect(),
                size.0 as u32,
                size.1 as u32,
            ))
        }
        "tif" | "tiff" => {
            let mut dec =
                tiff::decoder::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?))
                    .map_err(|e| corrupt("TIFF read failed", e))?;
            let (w, h) = dec
                .dimensions()
                .map_err(|e| corrupt("TIFF read failed", e))?;
            let data = match dec
                .read_image()
                .map_err(|e| corrupt("TIFF read failed", e))?
            {
                tiff::decoder::DecodingResult::F32(v) => v,
                tiff::decoder::DecodingResult::U16(v) => dequant(&v, need_range()?),
                tiff::decoder::DecodingResult::U8(v) => {
                    let r = need_range()?;
                    v.iter()
                        .map(|&b| r[0] + (b as f32 / 255.0) * (r[1] - r[0]))
                        .collect()
                }
                _ => {
                    return Err(corrupt(
                        "TIFF",
                        "unsupported sample type (use f32, u16 or u8 grayscale)",
                    ));
                }
            };
            Ok((data, w, h))
        }
        "r16" | "raw" | "r32" => {
            let [w, h] = opts.size_px.ok_or_else(|| {
                JdError::invalid("size_px", "headerless RAW needs size_px: [width, height]")
            })?;
            let bytes = std::fs::read(path)?;
            let data = if ext == "r32" {
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| f32::from_le_bytes(*c))
                    .collect::<Vec<_>>()
            } else {
                let q: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                dequant(&q, need_range()?)
            };
            if data.len() != (w as usize) * (h as usize) {
                return Err(corrupt(
                    "RAW",
                    format!("{} samples for a {w}×{h} size", data.len()),
                ));
            }
            Ok((data, w, h))
        }
        other => Err(JdError::new(
            ErrorCode::E_FORMAT_UNSUPPORTED,
            format!("unsupported heightmap extension `{other}`"),
        )
        .next("Use .png, .exr, .tif/.tiff, .r16 or .r32.")),
    }
}

/// Bilinearly resample row-major `data` (w × h) onto the world grid. Field row 0 is south, so a
/// north-up file (the image convention) is flipped. Non-finite samples are rejected.
pub fn resample_to_world(
    data: &[f32],
    w: u32,
    h: u32,
    north_up: bool,
    world: &WorldSettings,
) -> JdResult<Heightfield> {
    if w == 0 || h == 0 || data.len() != (w as usize) * (h as usize) {
        return Err(corrupt("heightmap", "size does not match sample count"));
    }
    if data.iter().any(|v| !v.is_finite()) {
        return Err(JdError::invalid("in", "heightmap has non-finite samples"));
    }
    let mut out = Heightfield::from_world(world)?;
    let (ow, oh) = (out.width(), out.height());
    let at = |x: u32, y: u32| {
        // Field rows run south → north; a north-up file (row 0 = north) is flipped.
        let yy = if north_up { h - 1 - y } else { y };
        data[(yy * w + x) as usize]
    };
    out.fill_with(|x, y| {
        // Pixel-centre mapping: output centre → source coordinates.
        let sx = ((x as f32 + 0.5) / ow as f32) * w as f32 - 0.5;
        let sy = ((y as f32 + 0.5) / oh as f32) * h as f32 - 0.5;
        let (x0, y0) = (
            sx.floor().clamp(0.0, (w - 1) as f32) as u32,
            sy.floor().clamp(0.0, (h - 1) as f32) as u32,
        );
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = (
            (sx - x0 as f32).clamp(0.0, 1.0),
            (sy - y0 as f32).clamp(0.0, 1.0),
        );
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(res: u32) -> WorldSettings {
        WorldSettings {
            extent_m: [1000.0, 1000.0],
            resolution_px: [res, res],
            tile_size: 64,
            halo: 2,
            ..Default::default()
        }
    }

    #[test]
    fn png16_with_range_resamples_onto_the_world() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("dem.png");
        // 32×32 ramp: 0 at x = 0 … 65535 at x = 31.
        let img = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_fn(32, 32, |x, _| {
            image::Luma([(x as f32 / 31.0 * 65535.0) as u16])
        });
        img.save(&p).expect("png");
        let opts = ForeignImport {
            range_m: Some([100.0, 300.0]),
            ..Default::default()
        };
        let (data, w, h) = read_foreign(&p, &opts).expect("read");
        assert_eq!((w, h), (32, 32));
        let hf = resample_to_world(&data, w, h, true, &world(64)).expect("resample");
        assert_eq!((hf.width(), hf.height()), (64, 64));
        assert!(
            (hf.get(0, 10) - 100.0).abs() < 4.0,
            "west edge ≈ min: {}",
            hf.get(0, 10)
        );
        assert!(
            (hf.get(63, 10) - 300.0).abs() < 4.0,
            "east edge ≈ max: {}",
            hf.get(63, 10)
        );
        assert!(
            (hf.get(32, 10) - 200.0).abs() < 6.0,
            "middle ≈ mean: {}",
            hf.get(32, 10)
        );
        assert!(
            read_foreign(&p, &ForeignImport::default()).is_err(),
            "quantised needs range_m"
        );
    }

    #[test]
    fn quantised_tiff_and_raw_use_range_and_size() {
        let dir = tempfile::tempdir().expect("tmp");
        let t = dir.path().join("q.tif");
        let q: Vec<u16> = (0..8 * 4).map(|i| (i % 8) as u16 * 9362).collect();
        tiff::encoder::TiffEncoder::new(std::fs::File::create(&t).expect("file"))
            .expect("enc")
            .write_image::<tiff::encoder::colortype::Gray16>(8, 4, &q)
            .expect("write");
        assert!(
            read_foreign(&t, &ForeignImport::default()).is_err(),
            "u16 needs range_m"
        );
        let (d, w, h) = read_foreign(
            &t,
            &ForeignImport {
                range_m: Some([0.0, 70.0]),
                ..Default::default()
            },
        )
        .expect("read");
        assert_eq!((w, h), (8, 4));
        assert!((d[7] - 70.0 * 65534.0 / 65535.0).abs() < 1e-2 && d[0] == 0.0);
        let r = dir.path().join("r.r32");
        let mut bytes = Vec::new();
        for v in [1.5f32, 2.5, 3.5, 4.5, 5.5, 6.5] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&r, &bytes).expect("write");
        assert!(
            read_foreign(&r, &ForeignImport::default()).is_err(),
            "raw needs size_px"
        );
        let (d, w, h) = read_foreign(
            &r,
            &ForeignImport {
                size_px: Some([3, 2]),
                ..Default::default()
            },
        )
        .expect("read");
        assert_eq!((w, h, d[5]), (3, 2, 6.5));
        assert!(
            read_foreign(
                &r,
                &ForeignImport {
                    size_px: Some([4, 2]),
                    ..Default::default()
                }
            )
            .is_err(),
            "size mismatch"
        );
    }

    #[test]
    fn geotiff_tags_are_read_and_nodata_is_filled_with_the_floor() {
        use tiff::encoder::{TiffEncoder, colortype::Gray32Float};
        use tiff::tags::Tag;
        let dir = tempfile::tempdir().expect("tmp");
        let t = dir.path().join("dem.tif");
        let mut data: Vec<f32> = (0..32).map(|i| 100.0 + i as f32).collect();
        data[5] = -9999.0;
        data[17] = f32::NAN;
        {
            let mut enc = TiffEncoder::new(std::fs::File::create(&t).expect("file")).expect("enc");
            let mut img = enc.new_image::<Gray32Float>(8, 4).expect("img");
            img.encoder()
                .write_tag(Tag::ModelPixelScaleTag, &[30.0f64, 30.0, 0.0][..])
                .expect("scale");
            img.encoder()
                .write_tag(
                    Tag::ModelTiepointTag,
                    &[0.0f64, 0.0, 0.0, 500_000.0, 4_000_000.0, 0.0][..],
                )
                .expect("tie");
            img.encoder()
                .write_tag(Tag::GdalNodata, "-9999")
                .expect("nodata");
            img.rows_per_strip(4).expect("rps");
            img.write_data(&data).expect("data");
        }
        let tags = geotiff_tags(&t).expect("tags").expect("some");
        assert_eq!(tags.pixel_scale, Some([30.0, 30.0]));
        assert_eq!(
            tags.tiepoint.map(|p| [p[3], p[4]]),
            Some([500_000.0, 4_000_000.0])
        );
        assert_eq!(tags.nodata, Some(-9999.0));
        // The tag's nodata and the NaN are both filled with the lowest valid height (100 m).
        let (v, w, h) = read_foreign(
            &t,
            &ForeignImport {
                north_up: false,
                ..Default::default()
            },
        )
        .expect("read");
        assert_eq!((w, h), (8, 4));
        assert_eq!(v[5], 100.0);
        assert_eq!(v[17], 100.0);
        assert_eq!(v[31], 131.0);
        // An explicit nodata wins over the tag.
        let (v2, ..) = read_foreign(
            &t,
            &ForeignImport {
                north_up: false,
                nodata: Some(131.0),
                ..Default::default()
            },
        )
        .expect("read");
        assert_eq!(
            v2[31], -9999.0,
            "131 is void now and the floor is that −9999 sample"
        );
        assert_eq!(
            v2[5], -9999.0,
            "the tag value is a normal sample when nodata is explicit"
        );
        // Everything void is an error, not a silent flat field.
        let mut enc =
            TiffEncoder::new(std::fs::File::create(dir.path().join("void.tif")).expect("file"))
                .expect("enc");
        enc.write_image::<Gray32Float>(2, 2, &[f32::NAN; 4])
            .expect("img");
        assert!(read_foreign(&dir.path().join("void.tif"), &ForeignImport::default()).is_err());
        // Plain PNGs carry no tags.
        assert_eq!(geotiff_tags(Path::new("x.png")).expect("png"), None);
    }

    #[test]
    fn tiff_f32_round_trips_and_south_up_flips_rows() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("dem.tif");
        let data: Vec<f32> = (0..16 * 8).map(|i| (i / 16) as f32 * 10.0).collect(); // rows 0..7 → 0..70 m
        let f = std::fs::File::create(&p).expect("file");
        tiff::encoder::TiffEncoder::new(f)
            .expect("enc")
            .write_image::<tiff::encoder::colortype::Gray32Float>(16, 8, &data)
            .expect("write");
        let (back, w, h) = read_foreign(&p, &ForeignImport::default()).expect("read");
        assert_eq!((w, h), (16, 8));
        assert_eq!(back, data);
        let north = resample_to_world(&back, w, h, true, &world(8)).expect("north");
        let south = resample_to_world(&back, w, h, false, &world(8)).expect("south");
        // File rows 0..7 rise 0→70 m. north_up: file row 0 is north → field row 0 (south) is the
        // LAST file row (high); south-up keeps file order.
        assert!(
            north.get(4, 0) > north.get(4, 7),
            "north-up file: low rows land north"
        );
        assert!(south.get(4, 0) < south.get(4, 7));
        assert_eq!(
            read_foreign(&dir.path().join("x.bmp"), &ForeignImport::default())
                .expect_err("ext")
                .code,
            ErrorCode::E_FORMAT_UNSUPPORTED
        );
    }
}
