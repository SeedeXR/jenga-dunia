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
}

impl HeightmapFormat {
    /// Conventional file extension.
    pub fn extension(self) -> &'static str {
        match self {
            HeightmapFormat::Exr => "exr",
            HeightmapFormat::Png16 => "png",
            HeightmapFormat::RawR16 => "r16",
            HeightmapFormat::RawR32 => "r32",
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
    hf.validate_finite()?;
    ensure_parent(path)?;
    let stats = hf.stats();
    let format = opts.format;
    let flip = opts.flip_y.unwrap_or(matches!(
        format,
        HeightmapFormat::Exr | HeightmapFormat::Png16
    ));
    let range = opts.range_m.unwrap_or([stats.min, stats.max]);
    if range[0] >= range[1] || range.iter().any(|v| !v.is_finite()) {
        return Err(JdError::invalid("range_m", "must be finite with min < max"));
    }
    let (w, h) = (hf.width(), hf.height());
    let data = rows(hf, flip);
    let bytes = match format {
        HeightmapFormat::Exr => {
            use exr::prelude::*;
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
            let q = quantise(&data, range);
            let img = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_raw(w, h, q)
                .ok_or_else(|| JdError::new(ErrorCode::E_INTERNAL, "PNG buffer size mismatch"))?;
            img.save_with_format(path, image::ImageFormat::Png)
                .map_err(|e| {
                    JdError::new(ErrorCode::E_SERIALIZE, format!("PNG write failed: {e}"))
                        .detail("path", path.display().to_string())
                })?;
            fs::metadata(path)?.len()
        }
        HeightmapFormat::RawR16 => {
            let q = quantise(&data, range);
            let mut buf = Vec::with_capacity(q.len() * 2);
            for v in q {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            write_all(path, &buf)?
        }
        HeightmapFormat::RawR32 => {
            let mut buf = Vec::with_capacity(data.len() * 4);
            for v in data {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            write_all(path, &buf)?
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

fn write_all(path: &Path, buf: &[u8]) -> JdResult<u64> {
    let mut f = fs::File::create(path)?;
    f.write_all(buf)?;
    f.sync_all()?;
    Ok(buf.len() as u64)
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
        for fmt in [HeightmapFormat::Exr, HeightmapFormat::RawR32] {
            let path = dir.path().join(format!("h.{}", fmt.extension()));
            let rep = export_heightmap(
                &hf,
                &path,
                &ExportOptions {
                    format: fmt,
                    flip_y: None,
                    range_m: None,
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
            },
        )
        .expect_err("nan");
        assert_eq!(err.code, ErrorCode::E_NUMERIC_UNSTABLE);
    }
}
