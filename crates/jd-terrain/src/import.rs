//! Foreign heightmaps (no Jenga Dunia sidecar): PNG 8/16-bit, EXR, TIFF (f32 or u16), RAW r16/r32.
//! Decoded to f32 metres and resampled (bilinear) onto the project's world grid so any DEM or
//! engine heightmap can drive a graph.
//!
//! ponytail: metres come from `range_m` (quantised sources) or the file values (float sources);
//! GeoTIFF tags, projections and nodata handling arrive with `jd-geo`.

use std::path::Path;

use crate::Heightfield;
use jd_types::{ErrorCode, JdError, JdResult, WorldSettings};
use serde::Deserialize;

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
    /// Row 0 of the file is north (true, the usual image convention) or south.
    #[serde(default = "yes")]
    pub north_up: bool,
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
pub fn read_foreign(path: &Path, opts: &ForeignImport) -> JdResult<(Vec<f32>, u32, u32)> {
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

/// Bilinearly resample row-major `data` (w × h) onto the world grid, flipping rows when the
/// file's row 0 is south. Non-finite samples are rejected.
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
        let yy = if north_up { y } else { h - 1 - y };
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
        assert!(north.get(4, 0) < north.get(4, 7));
        assert!(south.get(4, 0) > south.get(4, 7));
        assert_eq!(
            read_foreign(&dir.path().join("x.bmp"), &ForeignImport::default())
                .expect_err("ext")
                .code,
            ErrorCode::E_FORMAT_UNSUPPORTED
        );
    }
}
