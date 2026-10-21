//! Monocular depth from a photograph as a height field: MoGe-2 (Microsoft, MIT; weights
//! `Ruicheng/moge-2-vit{s,b,l}-normal-onnx`, MIT) run through ONNX Runtime. The ONNX graph is
//! the raw `forward()`: `image` (1, 3, H, W) float32 RGB in 0..1 — the encoder normalises with
//! the ImageNet mean/std itself — and `num_tokens` (int64 scalar, 1200..3600; more = finer,
//! slower) in, `points` (1, H, W, 3) affine-invariant camera points, `normal`, `mask` (sigmoid
//! probability) and `metric_scale` out. The height field is the negated z of the points (near
//! = high), normalised to 0..1 over the masked pixels: relative depth up to an affine map.
//!
//! ponytail: no focal/shift recovery (MoGe's `infer()` post-processing, which turns the affine
//! points into metric depth) — a photograph of a landscape yields a relief, not metres; the
//! `metric_scale` output is returned for the day that lands. One image at a time, no batching.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use jd_types::{ErrorCode, JdError, JdResult};

/// What the model saw and said.
#[derive(Debug, Clone)]
pub struct Depth {
    /// Width of the field (the image as it was fed, ≤ `max_px` on the long side).
    pub width: u32,
    /// Height of the field.
    pub height: u32,
    /// Relative height 0..1 (near = 1), row 0 = the top of the photograph; masked-out pixels
    /// (sky, nothing) are `NaN`.
    pub height01: Vec<f32>,
    /// Share of pixels the model masked out, 0..1.
    pub masked_share: f32,
    /// The model's metric scale estimate (dimensionless until focal/shift recovery exists).
    pub metric_scale: f32,
    /// Milliseconds spent loading the model (0 when the session was already resident).
    pub load_ms: u64,
    /// Milliseconds spent in the session.
    pub infer_ms: u64,
}

/// One model resident at a time: the session (1.3 GB for ViT-L) is kept across calls and
/// replaced when a different model path comes in.
static SESSION: OnceLock<Mutex<Option<(PathBuf, ort::session::Session)>>> = OnceLock::new();

/// Run MoGe-2 on `image` with `model` (a `.onnx` file), resizing the long side to `max_px`
/// first (the model works on a 14 px token grid of about `num_tokens` tokens whatever the
/// input size; 1024 keeps the field useful without a 4k inference). CPU only: with ONNX
/// Runtime 1.31 the CoreML provider claims 221 of the model's 222 nodes and then fails to compute
/// them ("Unable to compute the prediction", zero-sized slices in the encoder) — and ort's default
/// device policy would register it on macOS anyway — so the session asks for the CPU outright.
pub fn depth_from_image(
    image: &Path,
    model: &Path,
    max_px: u32,
    num_tokens: i64,
) -> JdResult<Depth> {
    if !(518..=4096).contains(&max_px) {
        return Err(JdError::invalid("max_px", "must be within 518..=4096"));
    }
    if !(1200..=3600).contains(&num_tokens) {
        return Err(JdError::invalid("num_tokens", "must be within 1200..=3600"));
    }
    if !model.is_file() {
        return Err(JdError::new(ErrorCode::E_DEPENDENCY_MISSING, format!("model not found: {}", model.display()))
            .next("Download `model.onnx` from huggingface.co/Ruicheng/moge-2-vitl-normal-onnx (MIT) and pass its path."));
    }
    let img = image::open(image)
        .map_err(|e| JdError::invalid("image", format!("cannot read {}: {e}", image.display())))?
        .to_rgb8();
    let (iw, ih) = img.dimensions();
    let scale = (max_px as f32 / iw.max(ih) as f32).min(1.0);
    let (w, h) = (
        ((iw as f32 * scale) as u32).max(14),
        ((ih as f32 * scale) as u32).max(14),
    );
    let img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    // (1, 3, H, W), planar RGB in 0..1.
    let n = (w * h) as usize;
    let mut chw = vec![0.0f32; 3 * n];
    for (i, px) in img.pixels().enumerate() {
        for c in 0..3 {
            chw[c * n + i] = px.0[c] as f32 / 255.0;
        }
    }
    fn ort_err<E: std::fmt::Display>(what: &'static str) -> impl Fn(E) -> JdError {
        move |e| JdError::new(ErrorCode::E_INTERNAL, format!("onnxruntime {what}: {e}"))
    }
    let model_abs = std::path::absolute(model)?;
    let slot = SESSION.get_or_init(|| Mutex::new(None));
    let mut guard = slot
        .lock()
        .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "vision session lock poisoned"))?;
    let mut load_ms = 0;
    if guard.as_ref().is_none_or(|(p, _)| *p != model_abs) {
        let t0 = std::time::Instant::now();
        let env = ort::init().build().map_err(ort_err("init"))?;
        // ort's default device policy registers CoreML on macOS whatever the session asks for;
        // CoreML cannot compute this model (see above), so the CPU is chosen explicitly.
        let session = ort::session::Session::builder(&env)
            .map_err(ort_err("builder"))?
            .with_auto_device(ort::session::builder::AutoDevicePolicy::PreferCPU)
            .map_err(ort_err("device policy"))?
            .with_intra_threads(
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4),
            )
            .map_err(ort_err("threads"))?
            .commit_from_file(&model_abs)
            .map_err(ort_err("load"))?;
        *guard = Some((model_abs.clone(), session));
        load_ms = t0.elapsed().as_millis() as u64;
    }
    let session = &mut guard
        .as_mut()
        .ok_or_else(|| JdError::new(ErrorCode::E_INTERNAL, "no vision session"))?
        .1;
    let image_t = ort::value::Tensor::from_array(([1usize, 3, h as usize, w as usize], chw))
        .map_err(ort_err("input"))?;
    let tokens_t = ort::value::Tensor::from_array(([] as [usize; 0], vec![num_tokens]))
        .map_err(ort_err("input"))?;
    let t0 = std::time::Instant::now();
    let outputs = session
        .run(ort::inputs!["image" => image_t, "num_tokens" => tokens_t])
        .map_err(ort_err("run"))?;
    let infer_ms = t0.elapsed().as_millis() as u64;
    let (pshape, points) = outputs["points"]
        .try_extract_tensor::<f32>()
        .map_err(ort_err("points"))?;
    let (_, mask) = outputs["mask"]
        .try_extract_tensor::<f32>()
        .map_err(ort_err("mask"))?;
    let (_, scale_v) = outputs["metric_scale"]
        .try_extract_tensor::<f32>()
        .map_err(ort_err("metric_scale"))?;
    let dims: Vec<i64> = pshape.iter().copied().collect();
    if dims.len() != 4 || dims[3] != 3 || dims[1] as u32 != h || dims[2] as u32 != w {
        return Err(JdError::new(
            ErrorCode::E_INTERNAL,
            format!("unexpected `points` shape {dims:?} for a {w}×{h} image"),
        ));
    }
    if mask.len() != n {
        return Err(JdError::new(
            ErrorCode::E_INTERNAL,
            format!(
                "unexpected `mask` length {} for a {w}×{h} image",
                mask.len()
            ),
        ));
    }
    // Relative height: −z over the pixels the model trusts, min..max → 0..1.
    let z: Vec<f32> = (0..n).map(|i| points[i * 3 + 2]).collect();
    let trusted = |i: usize| mask[i] > 0.5 && z[i].is_finite();
    let n_trusted = (0..n).filter(|&i| trusted(i)).count();
    if n_trusted < n / 100 {
        return Err(JdError::new(
            ErrorCode::E_INTERNAL,
            format!("the model trusts {n_trusted} of {n} pixels"),
        )
        .next("Try a photograph with visible ground."));
    }
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for i in (0..n).filter(|&i| trusted(i)) {
        lo = lo.min(z[i]);
        hi = hi.max(z[i]);
    }
    if hi <= lo {
        return Err(JdError::new(
            ErrorCode::E_INTERNAL,
            "the model saw a flat plane: no relief to map",
        )
        .next("Try a photograph with visible relief."));
    }
    let height01 = (0..n)
        .map(|i| {
            if trusted(i) {
                1.0 - (z[i] - lo) / (hi - lo)
            } else {
                f32::NAN
            }
        })
        .collect();
    Ok(Depth {
        width: w,
        height: h,
        height01,
        masked_share: 1.0 - n_trusted as f32 / n as f32,
        metric_scale: scale_v.first().copied().unwrap_or(1.0),
        load_ms,
        infer_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the 1.3 GB MoGe-2 ONNX model: `JD_MOGE_MODEL=/path/to/model.onnx`; skipped otherwise.
    #[test]
    fn a_synthetic_hill_photograph_comes_back_as_a_relief() {
        let Ok(model) = std::env::var("JD_MOGE_MODEL") else {
            eprintln!("JD_MOGE_MODEL unset: skipping");
            return;
        };
        // A shaded dome on a flat plane: lighter towards the top (far), a bright blob in the
        // middle — the model should see something nearer in the middle than at the edges.
        let dir = std::env::temp_dir().join("jd-vision-test");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("hill.png");
        let mut img = image::RgbImage::new(256, 256);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let dx = x as f32 - 128.0;
            let dy = y as f32 - 150.0;
            let r = (dx * dx + dy * dy).sqrt();
            let dome = (1.0 - r / 90.0).clamp(0.0, 1.0);
            let sky = (1.0 - y as f32 / 256.0) * 0.4;
            let v = ((0.35 + 0.5 * dome + sky) * 255.0).clamp(0.0, 255.0) as u8;
            *p = image::Rgb([v, (v as f32 * 0.9) as u8, (v as f32 * 0.7) as u8]);
        }
        img.save(&path).expect("png");
        let d = depth_from_image(&path, Path::new(&model), 518, 1200).expect("depth");
        assert_eq!((d.width, d.height), (256, 256));
        assert_eq!(d.height01.len(), 256 * 256);
        let at = |x: u32, y: u32| d.height01[(y * 256 + x) as usize];
        let centre = at(128, 150);
        let edge = at(10, 150);
        assert!(centre.is_finite() && edge.is_finite(), "{centre} {edge}");
        assert!(
            centre > edge,
            "the dome reads nearer (higher) than the edge: {centre} vs {edge}"
        );
        assert!(d.infer_ms > 0 && d.load_ms > 0);
        // The session stays resident: a second call loads nothing.
        let again = depth_from_image(&path, Path::new(&model), 518, 1200).expect("again");
        assert_eq!(again.load_ms, 0);
        assert!(depth_from_image(&path, Path::new("/nonexistent.onnx"), 518, 1200).is_err());
    }
}
