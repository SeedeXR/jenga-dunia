//! Heightmap export/import: EXR (f32), PNG16, RAW r16/r32 with a JSON sidecar.
//!
//! Conventions (`docs/architecture/08-io-formats.md`): the field's row 0 is the
//! south edge; image formats expect row 0 at the top (north), so image exports
//! are flipped by default (`flip_y = true`). Quantised formats (PNG16, r16) map
//! `[range.min, range.max]` to `[0, 65535]` and record the range in the sidecar
//! so the exact heights can be recovered. Nothing is silently lost: every export
//! returns a report with the range and any quantisation step.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use jd_types::{ErrorCode, JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::heightfield::Heightfield;

/// Supported heightmap encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeightmapFormat {
    /// OpenEXR, single `Y` channel, 32-bit float, lossless (ZIP). Exact.
    Exr,
    /// 16-bit grayscale PNG, normalised to the export range.
    Png16,
    /// Headerless little-endian u16, normalised to the export range (Unity / Unreal).
    RawR16,
    /// Headerless little-endian f32. Exact.
    RawR32,
    /// TIFF, single 32-bit float sample per pixel (GIS and DCC tools read it directly). Exact.
    Tiff32,
}

impl HeightmapFormat {
    /// Conventional file extension.
    pub fn extension(self) -> &'static str {
        match self {
            HeightmapFormat::Exr => "exr",
            HeightmapFormat::Png16 => "png",
            HeightmapFormat::RawR16 => "r16",
            HeightmapFormat::RawR32 => "r32",
            HeightmapFormat::Tiff32 => "tif",
        }
    }
    /// True when values are quantised to 16 bits.
    pub fn is_quantised(self) -> bool {
        matches!(self, HeightmapFormat::Png16 | HeightmapFormat::RawR16)
    }
}

/// Export options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportOptions {
    /// Encoding.
    pub format: HeightmapFormat,
    /// Flip rows so row 0 is north (image convention). Default true for images, false for RAW.
    #[serde(default)]
    pub flip_y: Option<bool>,
    /// Height range mapped to [0, 65535] for quantised formats; defaults to the field's min/max.
    #[serde(default)]
    pub range_m: Option<[f32; 2]>,
    /// For `tiff32`: write GeoTIFF tags (pixel scale, tie point, EPSG GeoKeys) when the caller
    /// supplies a [`GeoRef`]. Default true; the file is then always north-up.
    #[serde(default = "yes")]
    pub georeference: bool,
}
fn yes() -> bool {
    true
}

/// Where the field sits in the world, for GeoTIFF tags: the south-west corner, the sample
/// spacing and an optional `EPSG:<code>` CRS (anything else is written without GeoKeys).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GeoRef {
    /// World coordinates of the south-west corner (outer edge of sample (0, 0)).
    pub origin: [f64; 2],
    /// Sample spacing [x, y] in CRS units.
    pub spacing: [f64; 2],
    /// `EPSG:32637`, `EPSG:4326`, … or None.
    pub crs: Option<String>,
}

/// GeoKeyDirectory for an EPSG code: geographic codes (4000–4999) use GeographicTypeGeoKey,
/// everything else ProjectedCSTypeGeoKey; raster type is PixelIsArea.
fn geokeys(epsg: Option<u32>) -> Vec<u16> {
    let mut keys: Vec<[u16; 4]> = vec![[1025, 0, 1, 1]]; // GTRasterTypeGeoKey = RasterPixelIsArea
    if let Some(code) = epsg {
        let geographic = (4000..5000).contains(&code);
        keys.push([1024, 0, 1, if geographic { 2 } else { 1 }]); // GTModelTypeGeoKey
        keys.push([if geographic { 2048 } else { 3072 }, 0, 1, code as u16]);
    }
    keys.sort_by_key(|k| k[0]);
    let mut out = vec![1, 1, 0, keys.len() as u16];
    for k in keys {
        out.extend_from_slice(&k);
    }
    out
}

/// Parse `EPSG:<code>` (case-insensitive); None for anything else.
pub fn epsg_code(crs: Option<&str>) -> Option<u32> {
    let s = crs?.trim();
    let code = s
        .strip_prefix("EPSG:")
        .or_else(|| s.strip_prefix("epsg:"))?;
    code.parse::<u32>().ok().filter(|c| (1..=65535).contains(c))
}

/// What was written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExportReport {
    /// Written file.
    pub path: PathBuf,
    /// Sidecar JSON (always written) describing size, range and conventions.
    pub sidecar: PathBuf,
    /// Encoding.
    pub format: HeightmapFormat,
    /// Width, height in pixels.
    pub size_px: [u32; 2],
    /// Height range encoded (for quantised formats) or observed (exact formats).
    pub range_m: [f32; 2],
    /// Metres per quantisation step (0 for exact formats).
    pub quantisation_step_m: f32,
    /// Whether rows were flipped (row 0 = north).
    pub flip_y: bool,
    /// Bytes written to `path`.
    pub bytes: u64,
}

/// Sidecar schema (`<file>.jd-height.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Sidecar {
    /// Sidecar schema version.
    pub schema_version: u32,
    /// Encoding.
    pub format: HeightmapFormat,
    /// Width, height.
    pub size_px: [u32; 2],
    /// Encoded/observed range in metres.
    pub range_m: [f32; 2],
    /// Row 0 is north when true.
    pub flip_y: bool,
    /// Byte order of RAW formats.
    pub byte_order: String,
    /// Units note.
    pub units: String,
    /// Generator.
    pub generator: String,
    /// Sample spacing [x, y] in metres when the exporter knew the world (extent = size_px × spacing_m).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing_m: Option<[f64; 2]>,
}

fn ensure_parent(path: &Path) -> JdResult<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    Ok(())
}

fn rows(hf: &Heightfield, flip: bool) -> Vec<f32> {
    let (w, h) = (hf.width() as usize, hf.height() as usize);
    let mut v = hf.to_vec();
    if flip {
        for y in 0..h / 2 {
            let (a, b) = v.split_at_mut((h - 1 - y) * w);
            a[y * w..y * w + w].swap_with_slice(&mut b[..w]);
        }
    }
    v
}

/// Write `hf` to `path` in `opts.format` plus a JSON sidecar next to it.
pub fn export_heightmap(
    hf: &Heightfield,
    path: &Path,
    opts: &ExportOptions,
) -> JdResult<ExportReport> {
    export_heightmap_geo(hf, path, opts, None)
}

/// [`export_heightmap`] with a georeference: a `tiff32` export then carries GeoTIFF tags
/// (`ModelPixelScaleTag`, `ModelTiepointTag`, GeoKeys for an EPSG CRS) and is written north-up.
pub fn export_heightmap_geo(
    hf: &Heightfield,
    path: &Path,
    opts: &ExportOptions,
    geo: Option<&GeoRef>,
) -> JdResult<ExportReport> {
    hf.validate_finite()?;
    ensure_parent(path)?;
    let stats = hf.stats();
    let format = opts.format;
    let spacing_m = geo.map(|g| g.spacing); // every format's sidecar carries the spacing; only TIFF gets tags
    let geo = geo.filter(|_| opts.georeference && format == HeightmapFormat::Tiff32);
    if let Some(g) = geo
        && !(g.spacing[0] > 0.0 && g.spacing[1] > 0.0 && g.origin.iter().all(|v| v.is_finite()))
    {
        return Err(JdError::invalid(
            "geo",
            "spacing must be positive and the origin finite",
        ));
    }
    let flip = if geo.is_some() {
        true // GeoTIFF tie point refers to the top-left (north-west) pixel
    } else {
        opts.flip_y.unwrap_or(matches!(
            format,
            HeightmapFormat::Exr | HeightmapFormat::Png16 | HeightmapFormat::Tiff32
        ))
    };
    let range = opts.range_m.unwrap_or([stats.min, stats.max]);
    if range[0] >= range[1] || range.iter().any(|v| !v.is_finite()) {
        return Err(JdError::invalid("range_m", "must be finite with min < max"));
    }
    let (w, h) = (hf.width(), hf.height());
    // The 16-bit and raw formats are quantised / written row by row (a 16k² layer is 1 GiB; the
    // whole-field f32 copy is only made for the encoders that need one: EXR, TIFF).
    let bytes = match format {
        HeightmapFormat::Exr => {
            use exr::prelude::*;
            let data = rows(hf, flip);
            let channel = AnyChannel::new("Y", FlatSamples::F32(data));
            let layer = Layer::new(
                Vec2(w as usize, h as usize),
                LayerAttributes::named("height"),
                Encoding::SMALL_LOSSLESS,
                AnyChannels::sort(smallvec::smallvec![channel]),
            );
            Image::from_layer(layer)
                .write()
                .to_file(path)
                .map_err(|e| {
                    JdError::new(ErrorCode::E_SERIALIZE, format!("EXR write failed: {e}"))
                        .detail("path", path.display().to_string())
                })?;
            fs::metadata(path)?.len()
        }
        HeightmapFormat::Png16 => {
            // Streamed through the png crate: no u16 plane, no encoder-side copy of it.
            let png_err = |e: png::EncodingError| {
                JdError::new(ErrorCode::E_SERIALIZE, format!("PNG write failed: {e}"))
                    .detail("path", path.display().to_string())
            };
            let file = std::io::BufWriter::with_capacity(1 << 20, fs::File::create(path)?);
            let mut enc = png::Encoder::new(file, w, h);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Sixteen);
            let mut writer = enc.write_header().map_err(png_err)?;
            let mut stream = writer.stream_writer().map_err(png_err)?;
            let mut bytes = vec![0u8; 2 * w as usize];
            for_each_row_quantised(hf, flip, range, |q| {
                for (b, v) in bytes.as_chunks_mut::<2>().0.iter_mut().zip(q) {
                    *b = v.to_be_bytes();
                }
                stream.write_all(&bytes)
            })?;
            stream.finish().map_err(png_err)?;
            fs::metadata(path)?.len()
        }
        HeightmapFormat::RawR16 => {
            let mut out = std::io::BufWriter::with_capacity(1 << 20, fs::File::create(path)?);
            for_each_row_quantised(hf, flip, range, |q| {
                for v in q {
                    out.write_all(&v.to_le_bytes())?;
                }
                Ok(())
            })?;
            finish(out)?;
            fs::metadata(path)?.len()
        }
        HeightmapFormat::RawR32 => {
            let mut out = std::io::BufWriter::with_capacity(1 << 20, fs::File::create(path)?);
            let mut row = vec![0.0f32; w as usize];
            for y in 0..h {
                hf.row(if flip { h - 1 - y } else { y }, &mut row);
                for v in &row {
                    out.write_all(&v.to_le_bytes())?;
                }
            }
            finish(out)?;
            fs::metadata(path)?.len()
        }
        HeightmapFormat::Tiff32 => {
            let data = rows(hf, flip);
            {
                // No BufWriter: tiff writes strips in large chunks, and the encoder's final
                // directory is written on drop, so the file must be complete before we measure it.
                let file = fs::File::create(path)?;
                let tiff_err = |e: tiff::TiffError| {
                    JdError::new(ErrorCode::E_SERIALIZE, format!("TIFF write failed: {e}"))
                        .detail("path", path.display().to_string())
                };
                let mut enc = tiff::encoder::TiffEncoder::new(file).map_err(tiff_err)?;
                let mut img = enc
                    .new_image::<tiff::encoder::colortype::Gray32Float>(w, h)
                    .map_err(tiff_err)?;
                if let Some(g) = geo {
                    use tiff::tags::Tag;
                    // North-west corner of the raster: origin is the south-west corner.
                    let north = g.origin[1] + g.spacing[1] * h as f64;
                    img.encoder()
                        .write_tag(
                            Tag::ModelPixelScaleTag,
                            &[g.spacing[0], g.spacing[1], 0.0][..],
                        )
                        .map_err(tiff_err)?;
                    img.encoder()
                        .write_tag(
                            Tag::ModelTiepointTag,
                            &[0.0, 0.0, 0.0, g.origin[0], north, 0.0][..],
                        )
                        .map_err(tiff_err)?;
                    let keys = geokeys(epsg_code(g.crs.as_deref()));
                    img.encoder()
                        .write_tag(Tag::GeoKeyDirectoryTag, &keys[..])
                        .map_err(tiff_err)?;
                }
                img.rows_per_strip(h.max(1)).map_err(tiff_err)?;
                img.write_data(&data).map_err(tiff_err)?;
            }
            fs::metadata(path)?.len()
        }
    };
    let sidecar_path = sidecar_path(path);
    let sidecar = Sidecar {
        schema_version: 1,
        format,
        size_px: [w, h],
        range_m: range,
        flip_y: flip,
        byte_order: "little-endian".into(),
        units: "metres; pixel spacing per project world settings".into(),
        generator: format!("jenga-dunia {}", env!("CARGO_PKG_VERSION")),
        spacing_m,
    };
    fs::write(&sidecar_path, serde_json::to_vec_pretty(&sidecar)?)?;
    Ok(ExportReport {
        path: path.to_path_buf(),
        sidecar: sidecar_path,
        format,
        size_px: [w, h],
        range_m: range,
        quantisation_step_m: if format.is_quantised() {
            (range[1] - range[0]) / 65535.0
        } else {
            0.0
        },
        flip_y: flip,
        bytes,
    })
}

fn sidecar_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".jd-height.json");
    PathBuf::from(s)
}

fn quantise(data: &[f32], range: [f32; 2]) -> Vec<u16> {
    let scale = 65535.0 / (range[1] - range[0]);
    data.iter()
        .map(|&v| (((v - range[0]) * scale).round().clamp(0.0, 65535.0)) as u16)
        .collect()
}

/// [`quantise`] one row at a time straight from the field (top row first when `flip`), so a
/// 16-bit export never holds a whole plane: a 16k² layer would be 512 MB of u16 alone.
fn for_each_row_quantised(
    hf: &Heightfield,
    flip: bool,
    range: [f32; 2],
    mut f: impl FnMut(&[u16]) -> std::io::Result<()>,
) -> JdResult<()> {
    let (w, h) = (hf.width(), hf.height());
    let mut row = vec![0.0f32; w as usize];
    for y in 0..h {
        hf.row(if flip { h - 1 - y } else { y }, &mut row);
        f(&quantise(&row, range))?;
    }
    Ok(())
}

fn finish(out: std::io::BufWriter<fs::File>) -> JdResult<()> {
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    Ok(())
}

/// Read a heightmap written by [`export_heightmap`] (uses the sidecar for RAW/PNG ranges and flips).
/// Returns a field with the given tile size/halo.
pub fn import_heightmap(path: &Path, tile_size: u32, halo: u32) -> JdResult<Heightfield> {
    let sidecar_bytes = fs::read(sidecar_path(path)).map_err(|e| {
        JdError::new(ErrorCode::E_FORMAT_UNSUPPORTED, format!("missing sidecar for {}: {e}", path.display()))
            .next("Import of foreign heightmaps without a sidecar arrives with the io crate; for now use files exported by Jenga Dunia.")
    })?;
    let sidecar: Sidecar = serde_json::from_slice(&sidecar_bytes)?;
    let [w, h] = sidecar.size_px;
    let [lo, hi] = sidecar.range_m;
    let mut data: Vec<f32> = match sidecar.format {
        HeightmapFormat::Exr => {
            use exr::prelude::*;
            let img = read_all_flat_layers_from_file(path).map_err(|e| {
                JdError::new(ErrorCode::E_FORMAT_CORRUPT, format!("EXR read failed: {e}"))
            })?;
            let layer = img
                .layer_data
                .first()
                .ok_or_else(|| JdError::new(ErrorCode::E_FORMAT_CORRUPT, "EXR has no layers"))?;
            let ch = layer
                .channel_data
                .list
                .iter()
                .find(|c| c.name.eq("Y"))
                .or_else(|| layer.channel_data.list.first())
                .ok_or_else(|| JdError::new(ErrorCode::E_FORMAT_CORRUPT, "EXR has no channels"))?;
            ch.sample_data.values_as_f32().collect()
        }
        HeightmapFormat::Png16 => {
            let img = image::open(path)
                .map_err(|e| {
                    JdError::new(ErrorCode::E_FORMAT_CORRUPT, format!("PNG read failed: {e}"))
                })?
                .into_luma16();
            dequantise(img.as_raw(), lo, hi)
        }
        HeightmapFormat::RawR16 => {
            let bytes = fs::read(path)?;
            let q: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            dequantise(&q, lo, hi)
        }
        HeightmapFormat::RawR32 => fs::read(path)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect(),
        HeightmapFormat::Tiff32 => {
            let mut dec =
                tiff::decoder::Decoder::new(std::io::BufReader::new(fs::File::open(path)?))
                    .map_err(|e| {
                        JdError::new(
                            ErrorCode::E_FORMAT_CORRUPT,
                            format!("TIFF read failed: {e}"),
                        )
                    })?;
            match dec.read_image().map_err(|e| {
                JdError::new(
                    ErrorCode::E_FORMAT_CORRUPT,
                    format!("TIFF read failed: {e}"),
                )
            })? {
                tiff::decoder::DecodingResult::F32(v) => v,
                _ => {
                    return Err(JdError::new(
                        ErrorCode::E_FORMAT_CORRUPT,
                        "TIFF is not 32-bit float grayscale",
                    ));
                }
            }
        }
    };
    if data.len() != (w as usize) * (h as usize) {
        return Err(JdError::new(
            ErrorCode::E_FORMAT_CORRUPT,
            "sample count does not match sidecar size",
        )
        .detail("expected", (w as u64) * (h as u64))
        .detail("actual", data.len() as u64));
    }
    if sidecar.flip_y {
        let wu = w as usize;
        let hu = h as usize;
        for y in 0..hu / 2 {
            let (a, b) = data.split_at_mut((hu - 1 - y) * wu);
            a[y * wu..y * wu + wu].swap_with_slice(&mut b[..wu]);
        }
    }
    let mut hf = Heightfield::new(w, h, tile_size, halo)?;
    hf.fill_with(|x, y| data[(y * w + x) as usize]);
    Ok(hf)
}

fn dequantise(q: &[u16], lo: f32, hi: f32) -> Vec<f32> {
    let step = (hi - lo) / 65535.0;
    q.iter().map(|&v| lo + v as f32 * step).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geotiff_export_writes_scale_tiepoint_and_epsg_keys_north_up() {
        let mut hf = Heightfield::new(8, 4, 8, 0).expect("hf");
        hf.fill_with(|x, y| x as f32 + y as f32 * 10.0); // row 0 (south) is the lowest row
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("geo.tif");
        let geo = GeoRef {
            origin: [500_000.0, 4_000_000.0],
            spacing: [30.0, 30.0],
            crs: Some("EPSG:32637".into()),
        };
        let rep = export_heightmap_geo(
            &hf,
            &p,
            &ExportOptions {
                format: HeightmapFormat::Tiff32,
                flip_y: Some(false),
                range_m: None,
                georeference: true,
            },
            Some(&geo),
        )
        .expect("export");
        assert!(
            rep.flip_y,
            "georeferenced TIFFs are always north-up, whatever flip_y says"
        );
        let tags = crate::import::geotiff_tags(&p)
            .expect("tags")
            .expect("some");
        assert_eq!(tags.pixel_scale, Some([30.0, 30.0]));
        assert_eq!(
            tags.tiepoint.map(|t| [t[3], t[4]]),
            Some([500_000.0, 4_000_120.0]),
            "tie point is the north-west corner"
        );
        assert_eq!(tags.epsg, Some(32637));
        // Reading it back as a foreign heightmap (north_up) restores the field exactly.
        let (v, w, h) = crate::import::read_foreign(&p, &crate::import::ForeignImport::default())
            .expect("read");
        assert_eq!((w, h), (8, 4));
        assert_eq!(
            v[0], 30.0,
            "file row 0 is the north row (the world flip happens in resample_to_world)"
        );
        assert_eq!(v[3 * 8 + 7], 7.0, "last file row is the south row");
        // Geographic codes become GeographicTypeGeoKey; a non-EPSG CRS writes no CRS key.
        assert_eq!(
            geokeys(Some(4326))[4..],
            [1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326]
        );
        assert_eq!(geokeys(None), vec![1, 1, 0, 1, 1025, 0, 1, 1]);
        assert_eq!(epsg_code(Some("epsg:3857")), Some(3857));
        assert_eq!(epsg_code(Some("WGS84")), None);
        // georeference: false → plain TIFF, no tags.
        let plain = dir.path().join("plain.tif");
        export_heightmap_geo(
            &hf,
            &plain,
            &ExportOptions {
                format: HeightmapFormat::Tiff32,
                flip_y: None,
                range_m: None,
                georeference: false,
            },
            Some(&geo),
        )
        .expect("export");
        assert_eq!(crate::import::geotiff_tags(&plain).expect("tags"), None);
    }
    use crate::noise::{NoiseParams, generate_noise};
    use jd_types::WorldSettings;

    fn field() -> Heightfield {
        let w = WorldSettings {
            extent_m: [512.0, 384.0],
            resolution_px: [96, 70],
            tile_size: 64,
            halo: 2,
            ..Default::default()
        };
        generate_noise(
            &w,
            &NoiseParams {
                seed: 5,
                height_range_m: [-20.0, 180.0],
                ..Default::default()
            },
        )
        .expect("field")
    }

    #[test]
    fn exr_and_r32_round_trip_exactly() {
        let dir = tempfile::tempdir().expect("tmp");
        let hf = field();
        for fmt in [
            HeightmapFormat::Exr,
            HeightmapFormat::RawR32,
            HeightmapFormat::Tiff32,
        ] {
            let path = dir.path().join(format!("h.{}", fmt.extension()));
            let rep = export_heightmap(
                &hf,
                &path,
                &ExportOptions {
                    format: fmt,
                    flip_y: None,
                    range_m: None,
                    georeference: true,
                },
            )
            .expect("export");
            assert_eq!(rep.quantisation_step_m, 0.0);
            assert!(rep.sidecar.exists() && rep.bytes > 0);
            let back = import_heightmap(&path, 64, 0).expect("import");
            assert_eq!(back.to_vec(), hf.to_vec(), "{fmt:?} must be exact");
        }
    }

    #[test]
    fn png16_and_r16_round_trip_within_one_quantisation_step() {
        let dir = tempfile::tempdir().expect("tmp");
        let hf = field();
        for fmt in [HeightmapFormat::Png16, HeightmapFormat::RawR16] {
            let path = dir.path().join(format!("h.{}", fmt.extension()));
            let rep = export_heightmap(
                &hf,
                &path,
                &ExportOptions {
                    format: fmt,
                    flip_y: None,
                    range_m: Some([-20.0, 180.0]),
                    georeference: true,
                },
            )
            .expect("export");
            assert!(rep.quantisation_step_m > 0.0);
            let back = import_heightmap(&path, 64, 0).expect("import");
            let max_err = back
                .to_vec()
                .iter()
                .zip(hf.to_vec())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(
                max_err <= rep.quantisation_step_m * 0.51,
                "{fmt:?} max_err {max_err} step {}",
                rep.quantisation_step_m
            );
        }
        let r16 = fs::metadata(dir.path().join("h.r16")).expect("meta").len();
        assert_eq!(r16, 96 * 70 * 2, "r16 is headerless u16");
    }

    #[test]
    fn flip_convention_puts_north_on_row_zero_for_images() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut hf = Heightfield::new(4, 3, 64, 0).expect("hf");
        hf.fill_with(|_, y| y as f32); // south row = 0, north row = 2
        let path = dir.path().join("f.r32");
        export_heightmap(
            &hf,
            &path,
            &ExportOptions {
                format: HeightmapFormat::RawR32,
                flip_y: Some(true),
                range_m: None,
                georeference: true,
            },
        )
        .expect("export");
        let bytes = fs::read(&path).expect("read");
        let first = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(
            first, 2.0,
            "row 0 of the file is the north row when flipped"
        );
        let back = import_heightmap(&path, 64, 0).expect("import");
        assert_eq!(back.get(0, 0), 0.0, "import undoes the flip");
    }

    #[test]
    fn bad_range_and_nan_are_rejected() {
        let dir = tempfile::tempdir().expect("tmp");
        let hf = field();
        let err = export_heightmap(
            &hf,
            &dir.path().join("x.png"),
            &ExportOptions {
                format: HeightmapFormat::Png16,
                flip_y: None,
                range_m: Some([5.0, 5.0]),
                georeference: true,
            },
        )
        .expect_err("bad range");
        assert_eq!(err.code, ErrorCode::E_INPUT_INVALID);
        let mut nan = Heightfield::new(2, 2, 64, 0).expect("hf");
        nan.fill_with(|x, _| if x == 0 { f32::NAN } else { 1.0 });
        let err = export_heightmap(
            &nan,
            &dir.path().join("y.exr"),
            &ExportOptions {
                format: HeightmapFormat::Exr,
                flip_y: None,
                range_m: None,
                georeference: true,
            },
        )
        .expect_err("nan");
        assert_eq!(err.code, ErrorCode::E_NUMERIC_UNSTABLE);
    }
}
