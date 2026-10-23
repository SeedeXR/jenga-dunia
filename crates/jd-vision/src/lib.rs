//! Monocular depth from a photograph as a height field, through ONNX Runtime (CPU). Two model
//! families, told apart by the ONNX graph's output names:
//!
//! - **MoGe-2** (Microsoft, MIT; weights `Ruicheng/moge-2-vit{s,b,l}-normal-onnx`, MIT). The
//!   graph is the raw `forward()`: `image` (1, 3, H, W) float32 RGB in 0..1 — the encoder
//!   normalises with the ImageNet mean/std itself — and `num_tokens` (int64 scalar, 1200..3600;
//!   more = finer, slower) in; `points` (1, H, W, 3) affine-invariant camera points, `normal`,
//!   `mask` (sigmoid probability) and `metric_scale` (already exp'd) out. The post-processing
//!   MoGe's `infer()` does in Python is done here: the focal length (relative to half the image
//!   diagonal) and the z shift are recovered by fitting the pinhole projection of the points to
//!   the view-plane grid (`recover_focal_shift`), the shift is added to z, and the metric scale
//!   turns the shifted z into depth in metres.
//! - **Depth Anything 3 metric** (ByteDance Seed, Apache-2.0; `depth-anything/DA3METRIC-LARGE`,
//!   the community ONNX export `Heliosoph/da3metric-large-onnx`, Apache-2.0). `image`
//!   (1, 3, 504, 504) ImageNet-normalised in; `depth` (1, 1, 504, 504) *canonical* depth — as
//!   seen by a camera with a 300 px focal length — and `sky` (≥ 0.5 = sky) out. The official
//!   `apply_metric_scaling` is `depth × focal_px / 300`, so metres need the camera's horizontal
//!   field of view (`fov_x_deg`); without it the depth stays canonical and `metric` is false.
//!
//! The height field is the negated depth normalised to 0..1 over the pixels the model trusts
//! (near = high). One image at a time, no batching.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use jd_types::{ErrorCode, JdError, JdResult};

/// Which model family the ONNX graph turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// MoGe-2: affine points + mask + metric scale.
    MoGe2,
    /// Depth Anything 3 metric: canonical depth + sky.
    Da3Metric,
}

impl ModelKind {
    /// The name the ops report: `moge2` or `da3-metric`.
    pub fn as_str(self) -> &'static str {
        match self {
            ModelKind::MoGe2 => "moge2",
            ModelKind::Da3Metric => "da3-metric",
        }
    }
}

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
    /// Depth per pixel, row 0 = the top; metres when `metric`, else the model's own units
    /// (DA3 canonical depth without a field of view); masked-out pixels are `NaN`.
    pub depth_m: Vec<f32>,
    /// Whether `depth_m` is in metres.
    pub metric: bool,
    /// Horizontal field of view in degrees: recovered from the points (MoGe-2) or the one
    /// given (DA3 needs it; `None` when it had none).
    pub fov_x_deg: Option<f32>,
    /// Where `fov_x_deg` came from: `recovered` (MoGe's fit), `given`, `camera` (DA3's camera
    /// head) or `none`.
    pub fov_source: &'static str,
    /// MoGe-2: focal length relative to half the image diagonal, and the z shift added to the
    /// affine points; `None` for DA3.
    pub focal_shift: Option<(f32, f32)>,
    /// Share of pixels the model masked out, 0..1.
    pub masked_share: f32,
    /// MoGe-2's metric-scale estimate (metres per unit of shifted z); 1 for DA3.
    pub metric_scale: f32,
    /// The model family.
    pub model: ModelKind,
    /// Milliseconds spent loading the model (0 when the session was already resident).
    pub load_ms: u64,
    /// Milliseconds spent in the session.
    pub infer_ms: u64,
}

/// One model resident at a time: the session (1.3 GB for ViT-L) is kept across calls and
/// replaced when a different model path comes in.
static SESSION: OnceLock<Mutex<Option<(PathBuf, ort::session::Session)>>> = OnceLock::new();
/// The DA3 camera model (`Heliosoph/da3-base-onnx`), resident beside the depth model.
static CAMERA: OnceLock<Mutex<Option<(PathBuf, ort::session::Session)>>> = OnceLock::new();

fn ort_err<E: std::fmt::Display>(what: &'static str) -> impl Fn(E) -> JdError {
    move |e| JdError::new(ErrorCode::E_INTERNAL, format!("onnxruntime {what}: {e}"))
}

/// Open an ONNX session on the CPU (see [`depth_from_image`] for why not CoreML).
fn open_session(path: &Path) -> JdResult<ort::session::Session> {
    // ONNX Runtime allows one environment per process; every session shares it.
    static ENV: OnceLock<ort::environment::Environment> = OnceLock::new();
    let env = match ENV.get() {
        Some(e) => e.clone(),
        None => {
            let e = ort::init().build().map_err(ort_err("init"))?;
            ENV.get_or_init(|| e).clone()
        }
    };
    ort::session::Session::builder(&env)
        .map_err(ort_err("builder"))?
        .with_auto_device(ort::session::builder::AutoDevicePolicy::PreferCPU)
        .map_err(ort_err("device policy"))?
        .with_intra_threads(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        )
        .map_err(ort_err("threads"))?
        .commit_from_file(path)
        .map_err(ort_err("load"))
}

/// DA3's camera head from the `da3-base` export: `intrinsics` (1, 1, 3, 3) at the 504² grid →
/// `(fx, fy)` in pixels (the principal point is fixed at 252, 252). The input is the same
/// ImageNet-normalised square the metric model sees.
fn da3_intrinsics(camera_model: &Path, chw: Vec<f32>, px: u32) -> JdResult<(f32, f32)> {
    let abs = std::path::absolute(camera_model)?;
    if !abs.is_file() {
        return Err(JdError::new(ErrorCode::E_DEPENDENCY_MISSING, format!("camera model not found: {}", abs.display()))
            .next("Download `model.onnx` from huggingface.co/Heliosoph/da3-base-onnx (Apache-2.0) and pass its path as `camera_model`."));
    }
    let slot = CAMERA.get_or_init(|| Mutex::new(None));
    let mut guard = slot
        .lock()
        .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "camera session lock poisoned"))?;
    if guard.as_ref().is_none_or(|(p, _)| *p != abs) {
        let session = open_session(&abs)?;
        if !session.outputs().iter().any(|o| o.name() == "intrinsics") {
            return Err(JdError::invalid(
                "camera_model",
                "the graph has no `intrinsics` output (the DA3 metric export has no camera head; use the da3-base export)",
            ));
        }
        *guard = Some((abs.clone(), session));
    }
    let session = &mut guard
        .as_mut()
        .ok_or_else(|| JdError::new(ErrorCode::E_INTERNAL, "no camera session"))?
        .1;
    let t = ort::value::Tensor::from_array(([1usize, 3, px as usize, px as usize], chw))
        .map_err(ort_err("input"))?;
    let outputs = session
        .run(ort::inputs!["image" => t])
        .map_err(ort_err("run"))?;
    let (_, k) = outputs["intrinsics"]
        .try_extract_tensor::<f32>()
        .map_err(ort_err("intrinsics"))?;
    if k.len() != 9 || !(k[0] > 0.0 && k[4] > 0.0) {
        return Err(JdError::new(
            ErrorCode::E_INTERNAL,
            format!("unexpected DA3 intrinsics {k:?}"),
        ));
    }
    Ok((k[0], k[4]))
}

/// DA3's fixed network input.
const DA3_PX: u32 = 504;
/// DA3's canonical camera: the focal length, in pixels, its depth is expressed for.
const DA3_CANONICAL_FOCAL_PX: f32 = 300.0;

/// Run the model on `image` with `model` (a `.onnx` file), resizing the long side to `max_px`
/// first (MoGe works on a 14 px token grid of about `num_tokens` tokens whatever the input
/// size; DA3 always sees 504×504 and its maps are resampled back to the `max_px` grid).
/// `fov_x_deg`: the camera's horizontal field of view when known — MoGe then recovers the shift
/// only, DA3 needs it for metres; without it, `camera_model` (the DA3 base export, which has
/// the camera head the metric export lacks) estimates the intrinsics from the same input. CPU only: with ONNX Runtime 1.31 the CoreML provider claims
/// 221 of MoGe's 222 nodes and then fails to compute them ("Unable to compute the prediction",
/// zero-sized slices in the encoder) — and ort's default device policy would register it on
/// macOS anyway — so the session asks for the CPU outright.
pub fn depth_from_image(
    image: &Path,
    model: &Path,
    max_px: u32,
    num_tokens: i64,
    fov_x_deg: Option<f32>,
    camera_model: Option<&Path>,
) -> JdResult<Depth> {
    if !(518..=4096).contains(&max_px) {
        return Err(JdError::invalid("max_px", "must be within 518..=4096"));
    }
    if !(1200..=3600).contains(&num_tokens) {
        return Err(JdError::invalid("num_tokens", "must be within 1200..=3600"));
    }
    if let Some(f) = fov_x_deg
        && !(f > 1.0 && f < 179.0)
    {
        return Err(JdError::invalid(
            "fov_x_deg",
            "must be within 1..179 degrees",
        ));
    }
    if !model.is_file() {
        return Err(JdError::new(ErrorCode::E_DEPENDENCY_MISSING, format!("model not found: {}", model.display()))
            .next("Download `model.onnx` from huggingface.co/Ruicheng/moge-2-vitl-normal-onnx (MIT) or huggingface.co/Heliosoph/da3metric-large-onnx (Apache-2.0) and pass its path."));
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
    let model_abs = std::path::absolute(model)?;
    let slot = SESSION.get_or_init(|| Mutex::new(None));
    let mut guard = slot
        .lock()
        .map_err(|_| JdError::new(ErrorCode::E_INTERNAL, "vision session lock poisoned"))?;
    let mut load_ms = 0;
    if guard.as_ref().is_none_or(|(p, _)| *p != model_abs) {
        let t0 = std::time::Instant::now();
        let session = open_session(&model_abs)?;
        *guard = Some((model_abs.clone(), session));
        load_ms = t0.elapsed().as_millis() as u64;
    }
    let session = &mut guard
        .as_mut()
        .ok_or_else(|| JdError::new(ErrorCode::E_INTERNAL, "no vision session"))?
        .1;
    let has = |name: &str| session.outputs().iter().any(|o| o.name() == name);
    let kind = if has("points") && has("metric_scale") {
        ModelKind::MoGe2
    } else if has("depth") && has("sky") {
        ModelKind::Da3Metric
    } else {
        let names: Vec<&str> = session.outputs().iter().map(|o| o.name()).collect();
        return Err(JdError::invalid(
            "model",
            format!(
                "not a MoGe-2 (`points`, `metric_scale`) or DA3 metric (`depth`, `sky`) graph: outputs {names:?}"
            ),
        ));
    };
    let n = (w * h) as usize;
    // (1, 3, H, W) planar RGB; MoGe wants 0..1 at the working size, DA3 ImageNet-normalised at 504².
    let (fw, fh) = match kind {
        ModelKind::MoGe2 => (w, h),
        ModelKind::Da3Metric => (DA3_PX, DA3_PX),
    };
    let fed = image::imageops::resize(&img, fw, fh, image::imageops::FilterType::Triangle);
    let fn_ = (fw * fh) as usize;
    let mut chw = vec![0.0f32; 3 * fn_];
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    for (i, px) in fed.pixels().enumerate() {
        for c in 0..3 {
            let v = px.0[c] as f32 / 255.0;
            chw[c * fn_ + i] = match kind {
                ModelKind::MoGe2 => v,
                ModelKind::Da3Metric => (v - MEAN[c]) / STD[c],
            };
        }
    }
    let chw_camera =
        (kind == ModelKind::Da3Metric && fov_x_deg.is_none() && camera_model.is_some())
            .then(|| chw.clone());
    let image_t = ort::value::Tensor::from_array(([1usize, 3, fh as usize, fw as usize], chw))
        .map_err(ort_err("input"))?;
    let t0 = std::time::Instant::now();
    // depth per pixel of the w×h grid (model units), trusted flag, and the metric facts.
    let (mut depth, mut trusted, metric_scale, fov_x_deg, focal_shift, fov_source) = match kind {
        ModelKind::MoGe2 => {
            let tokens_t = ort::value::Tensor::from_array(([] as [usize; 0], vec![num_tokens]))
                .map_err(ort_err("input"))?;
            let outputs = session
                .run(ort::inputs!["image" => image_t, "num_tokens" => tokens_t])
                .map_err(ort_err("run"))?;
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
            let trusted: Vec<bool> = (0..n)
                .map(|i| mask[i] > 0.5 && (0..3).all(|c| points[i * 3 + c].is_finite()))
                .collect();
            let given_focal = fov_x_deg.map(|f| span_x(w, h) / (f.to_radians() / 2.0).tan());
            let (focal, shift) = recover_focal_shift(points, &trusted, w, h, given_focal);
            let scale = scale_v.first().copied().unwrap_or(1.0);
            let depth: Vec<f32> = (0..n)
                .map(|i| (points[i * 3 + 2] + shift) * scale)
                .collect();
            let fov = 2.0 * (span_x(w, h) / focal).atan().to_degrees();
            let source = if fov_x_deg.is_some() {
                "given"
            } else {
                "recovered"
            };
            (
                depth,
                trusted,
                scale,
                Some(fov),
                Some((focal, shift)),
                source,
            )
        }
        ModelKind::Da3Metric => {
            let outputs = session
                .run(ort::inputs!["image" => image_t])
                .map_err(ort_err("run"))?;
            let (dshape, canon) = outputs["depth"]
                .try_extract_tensor::<f32>()
                .map_err(ort_err("depth"))?;
            let (_, sky) = outputs["sky"]
                .try_extract_tensor::<f32>()
                .map_err(ort_err("sky"))?;
            let want = (DA3_PX * DA3_PX) as usize;
            if canon.len() != want || sky.len() != want {
                let dims: Vec<i64> = dshape.iter().copied().collect();
                return Err(JdError::new(
                    ErrorCode::E_INTERNAL,
                    format!("unexpected DA3 output shape {dims:?} (want 1×1×{DA3_PX}×{DA3_PX})"),
                ));
            }
            // Official `apply_metric_scaling`: depth × mean(fx, fy) / 300, with the focal
            // lengths in pixels of the 504² network input. The photograph was stretched to a
            // square, so fx comes from the horizontal field of view and fy from the vertical
            // one of the original aspect ratio.
            let (scale, fov, source) = match fov_x_deg {
                Some(f) => {
                    let tx = (f.to_radians() / 2.0).tan();
                    let ty = tx * ih as f32 / iw as f32;
                    let fx = 0.5 * DA3_PX as f32 / tx;
                    let fy = 0.5 * DA3_PX as f32 / ty;
                    ((fx + fy) / 2.0 / DA3_CANONICAL_FOCAL_PX, Some(f), "given")
                }
                None => match (camera_model, chw_camera) {
                    (Some(cm), Some(data)) => {
                        let (fx, fy) = da3_intrinsics(cm, data, DA3_PX)?;
                        let fov = 2.0 * (DA3_PX as f32 / 2.0 / fx).atan().to_degrees();
                        (
                            (fx + fy) / 2.0 / DA3_CANONICAL_FOCAL_PX,
                            Some(fov),
                            "camera",
                        )
                    }
                    _ => (1.0, None, "none"),
                },
            };
            let depth_lr: Vec<f32> = canon.iter().map(|d| d * scale).collect();
            let depth = resample_bilinear(&depth_lr, DA3_PX, DA3_PX, w, h);
            let sky_hr = resample_bilinear(sky, DA3_PX, DA3_PX, w, h);
            let trusted: Vec<bool> = (0..n)
                .map(|i| sky_hr[i] < 0.5 && depth[i].is_finite())
                .collect();
            (depth, trusted, 1.0, fov, None, source)
        }
    };
    let infer_ms = t0.elapsed().as_millis() as u64;
    let metric = match kind {
        ModelKind::MoGe2 => true,
        ModelKind::Da3Metric => fov_x_deg.is_some(),
    };
    // A depth at or behind the camera is nothing the model should say; drop it (MoGe's infer()
    // does the same after the shift).
    for i in 0..n {
        if trusted[i] && (!depth[i].is_finite() || depth[i] <= 0.0) {
            trusted[i] = false;
        }
    }
    let n_trusted = trusted.iter().filter(|&&t| t).count();
    if n_trusted < n / 100 {
        return Err(JdError::new(
            ErrorCode::E_INTERNAL,
            format!("the model trusts {n_trusted} of {n} pixels"),
        )
        .next("Try a photograph with visible ground."));
    }
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for i in (0..n).filter(|&i| trusted[i]) {
        lo = lo.min(depth[i]);
        hi = hi.max(depth[i]);
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
            if trusted[i] {
                1.0 - (depth[i] - lo) / (hi - lo)
            } else {
                f32::NAN
            }
        })
        .collect();
    for i in 0..n {
        if !trusted[i] {
            depth[i] = f32::NAN;
        }
    }
    Ok(Depth {
        width: w,
        height: h,
        height01,
        depth_m: depth,
        metric,
        fov_x_deg,
        fov_source,
        focal_shift,
        masked_share: 1.0 - n_trusted as f32 / n as f32,
        metric_scale,
        model: kind,
        load_ms,
        infer_ms,
    })
}

/// Half the normalised view-plane width: MoGe's UV grid spans ±width/diagonal horizontally
/// and ±height/diagonal vertically, so a focal length relative to half the diagonal makes the
/// horizontal field of view `2·atan(span_x / focal)`.
fn span_x(w: u32, h: u32) -> f32 {
    let a = w as f32 / h as f32;
    a / (1.0 + a * a).sqrt()
}

/// MoGe's view-plane coordinate of pixel column `j` of `w` (row `i` of `h` likewise with the
/// vertical span): the grid runs from −span·(w−1)/w to +span·(w−1)/w at the pixel centres.
fn view_uv(j: u32, w: u32, span: f32) -> f32 {
    span * (2.0 * j as f32 - w as f32 + 1.0) / w as f32
}

/// Recover MoGe's focal length (relative to half the image diagonal) and the z shift of an
/// affine-invariant point map: `min Σ |focal · xy / (z + shift) − uv|²` over the trusted
/// pixels, after a mask-aware nearest downsample to 64×64 (MoGe's `recover_focal_shift`). With
/// `focal` given only the shift is fitted. Returns `(1, 0)` when fewer than two pixels are
/// trusted.
///
/// ponytail: MoGe runs scipy's Levenberg–Marquardt from shift 0; the focal is closed-form for
/// any shift, so this is a one-dimensional fit — a geometric scan over the feasible shifts
/// (every `z + shift` positive) narrowed by golden-section search does the same job without
/// a solver dependency.
pub fn recover_focal_shift(
    points: &[f32],
    trusted: &[bool],
    w: u32,
    h: u32,
    focal: Option<f32>,
) -> (f32, f32) {
    let sx = span_x(w, h);
    let sy = 1.0 / (1.0 + (w as f32 / h as f32).powi(2)).sqrt();
    // Mask-aware nearest downsample: each 64×64 cell takes the trusted pixel nearest its centre
    // inside its footprint, or nothing.
    let (tw, th) = (w.min(64), h.min(64));
    let (cw, ch) = (w as f32 / tw as f32, h as f32 / th as f32);
    let mut uv: Vec<[f32; 2]> = Vec::with_capacity((tw * th) as usize);
    let mut xyz: Vec<[f32; 3]> = Vec::with_capacity((tw * th) as usize);
    for ti in 0..th {
        for tj in 0..tw {
            let (cx, cy) = ((tj as f32 + 0.5) * cw, (ti as f32 + 0.5) * ch);
            let (j0, j1) = (
                ((cx - cw / 2.0).floor().max(0.0)) as u32,
                ((cx + cw / 2.0).ceil() as u32).min(w - 1),
            );
            let (i0, i1) = (
                ((cy - ch / 2.0).floor().max(0.0)) as u32,
                ((cy + ch / 2.0).ceil() as u32).min(h - 1),
            );
            let mut best: Option<(f32, usize)> = None;
            for i in i0..=i1 {
                for j in j0..=j1 {
                    let idx = (i * w + j) as usize;
                    if !trusted[idx] {
                        continue;
                    }
                    let (dx, dy) = (j as f32 + 0.5 - cx, i as f32 + 0.5 - cy);
                    if dx.abs() > cw / 2.0 + 1e-4 || dy.abs() > ch / 2.0 + 1e-4 {
                        continue;
                    }
                    let d = dx * dx + dy * dy;
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, idx));
                    }
                }
            }
            if let Some((_, idx)) = best {
                let (i, j) = (idx as u32 / w, idx as u32 % w);
                uv.push([view_uv(j, w, sx), view_uv(i, h, sy)]);
                xyz.push([points[idx * 3], points[idx * 3 + 1], points[idx * 3 + 2]]);
            }
        }
    }
    if uv.len() < 2 {
        return (1.0, 0.0);
    }
    // Residual sum of squares for a shift, with the closed-form focal unless one is given.
    let focal_for = |shift: f32| -> f32 {
        if let Some(f) = focal {
            return f;
        }
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for (p, u) in xyz.iter().zip(&uv) {
            let d = (p[2] + shift) as f64;
            let (px, py) = (p[0] as f64 / d, p[1] as f64 / d);
            num += px * u[0] as f64 + py * u[1] as f64;
            den += px * px + py * py;
        }
        if den > 0.0 { (num / den) as f32 } else { 1.0 }
    };
    let cost = |shift: f32| -> f64 {
        let f = focal_for(shift) as f64;
        xyz.iter()
            .zip(&uv)
            .map(|(p, u)| {
                let d = (p[2] + shift) as f64;
                let (ex, ey) = (
                    f * p[0] as f64 / d - u[0] as f64,
                    f * p[1] as f64 / d - u[1] as f64,
                );
                ex * ex + ey * ey
            })
            .sum()
    };
    // Feasible shifts keep every depth positive: shift > −min z. Scan t = shift + min z over
    // a geometric grid (1e-3 .. 1e3 in units of the point cloud), then narrow.
    let zmin = xyz.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
    let t_of = |k: usize| 1e-3f32 * 1e6f32.powf(k as f32 / 120.0);
    let mut best_k = 0;
    let mut best_c = f64::INFINITY;
    for k in 0..=120 {
        let c = cost(t_of(k) - zmin);
        if c < best_c {
            best_c = c;
            best_k = k;
        }
    }
    let (mut a, mut b) = (t_of(best_k.saturating_sub(1)), t_of((best_k + 1).min(120)));
    let phi = 0.618_034f32;
    let (mut c1, mut c2) = (b - phi * (b - a), a + phi * (b - a));
    let (mut f1, mut f2) = (cost(c1 - zmin), cost(c2 - zmin));
    for _ in 0..60 {
        if f1 < f2 {
            b = c2;
            c2 = c1;
            f2 = f1;
            c1 = b - phi * (b - a);
            f1 = cost(c1 - zmin);
        } else {
            a = c1;
            c1 = c2;
            f1 = f2;
            c2 = a + phi * (b - a);
            f2 = cost(c2 - zmin);
        }
    }
    let shift = (a + b) / 2.0 - zmin;
    (focal_for(shift), shift)
}

/// Bilinear resample of a `sw`×`sh` map to `dw`×`dh` (pixel-centre aligned).
fn resample_bilinear(src: &[f32], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<f32> {
    let at = |x: u32, y: u32| src[(y.min(sh - 1) * sw + x.min(sw - 1)) as usize];
    let mut out = Vec::with_capacity((dw * dh) as usize);
    for y in 0..dh {
        let fy = ((y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5).max(0.0);
        let (y0, ty) = (fy.floor() as u32, fy.fract());
        for x in 0..dw {
            let fx = ((x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5).max(0.0);
            let (x0, tx) = (fx.floor() as u32, fx.fract());
            let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
            let bot = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
            out.push(top * (1.0 - ty) + bot * ty);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pinhole camera with a known focal and a known z shift, seen through MoGe's affine
    /// convention, comes back with both numbers.
    #[test]
    fn focal_and_shift_are_recovered_from_a_synthetic_point_map() {
        let (w, h) = (96u32, 64u32);
        let (f_true, s_true) = (1.3f32, 0.8f32);
        let sx = span_x(w, h);
        let sy = 1.0 / (1.0 + (w as f32 / h as f32).powi(2)).sqrt();
        let mut points = Vec::with_capacity((w * h * 3) as usize);
        let mut trusted = Vec::with_capacity((w * h) as usize);
        for i in 0..h {
            for j in 0..w {
                let z = 2.0 + 0.6 * (j as f32 * 0.2).sin() + 0.3 * (i as f32 * 0.3).cos();
                let (u, v) = (view_uv(j, w, sx), view_uv(i, h, sy));
                points.extend_from_slice(&[u * z / f_true, v * z / f_true, z - s_true]);
                trusted.push(i > 8); // a sky band the model would mask
            }
        }
        let (f, s) = recover_focal_shift(&points, &trusted, w, h, None);
        assert!((f - f_true).abs() < 1e-2, "focal {f} vs {f_true}");
        assert!((s - s_true).abs() < 1e-2, "shift {s} vs {s_true}");
        // With the focal given, the shift alone is fitted.
        let (f2, s2) = recover_focal_shift(&points, &trusted, w, h, Some(f_true));
        assert_eq!(f2, f_true);
        assert!((s2 - s_true).abs() < 1e-2, "shift {s2} vs {s_true}");
        // Too few pixels: the identity.
        let none = vec![false; (w * h) as usize];
        assert_eq!(recover_focal_shift(&points, &none, w, h, None), (1.0, 0.0));
    }

    #[test]
    fn bilinear_resample_keeps_a_ramp() {
        let src: Vec<f32> = (0..4).flat_map(|_| (0..4).map(|x| x as f32)).collect();
        let out = resample_bilinear(&src, 4, 4, 8, 2);
        assert_eq!(out.len(), 16);
        assert!(out[0] <= out[7] && (out[7] - 3.0).abs() < 1e-6, "{out:?}");
    }

    /// The model-backed tests share the one resident session slot, so they take turns.
    static TURN: Mutex<()> = Mutex::new(());

    fn hill_png() -> PathBuf {
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
        path
    }

    /// Needs the 1.3 GB MoGe-2 ONNX model: `JD_MOGE_MODEL=/path/to/model.onnx`; skipped otherwise.
    #[test]
    fn a_synthetic_hill_photograph_comes_back_as_a_relief() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(model) = std::env::var("JD_MOGE_MODEL") else {
            eprintln!("JD_MOGE_MODEL unset: skipping");
            return;
        };
        let path = hill_png();
        let d = depth_from_image(&path, Path::new(&model), 518, 1200, None, None).expect("depth");
        assert_eq!(d.model, ModelKind::MoGe2);
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
        // Metric: a field of view was recovered and every trusted depth is a positive metre.
        assert!(d.metric);
        let fov = d.fov_x_deg.expect("fov");
        assert!((10.0..170.0).contains(&fov), "fov {fov}");
        let (focal, _shift) = d.focal_shift.expect("focal/shift");
        assert!(focal > 0.0);
        let dm = d.depth_m[(150 * 256 + 128) as usize];
        assert!(dm.is_finite() && dm > 0.0, "{dm}");
        assert!(d.infer_ms > 0 && d.load_ms > 0);
        // The session stays resident: a second call loads nothing; a given fov pins it.
        let again =
            depth_from_image(&path, Path::new(&model), 518, 1200, Some(60.0), None).expect("again");
        assert_eq!(again.load_ms, 0);
        assert!((again.fov_x_deg.expect("fov") - 60.0).abs() < 1e-3);
        assert!(
            depth_from_image(&path, Path::new("/nonexistent.onnx"), 518, 1200, None, None).is_err()
        );
    }

    /// Needs the DA3 metric ONNX model: `JD_DA3_MODEL=/path/to/model.onnx`; skipped otherwise.
    #[test]
    fn the_da3_metric_graph_is_recognised_and_scaled_by_the_field_of_view() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(model) = std::env::var("JD_DA3_MODEL") else {
            eprintln!("JD_DA3_MODEL unset: skipping");
            return;
        };
        let path = hill_png();
        let canon =
            depth_from_image(&path, Path::new(&model), 518, 1200, None, None).expect("depth");
        assert_eq!(canon.model, ModelKind::Da3Metric);
        assert!(!canon.metric && canon.fov_x_deg.is_none());
        assert_eq!((canon.width, canon.height), (256, 256));
        let metric = depth_from_image(&path, Path::new(&model), 518, 1200, Some(60.0), None)
            .expect("metric");
        assert!(metric.metric);
        // 60° horizontal on a square: fx = fy = 252 / tan 30° = 436.5 px → × 436.5 / 300.
        let i = (150 * 256 + 128) as usize;
        let ratio = metric.depth_m[i] / canon.depth_m[i];
        assert!((ratio - 436.48 / 300.0).abs() < 1e-2, "ratio {ratio}");
        assert!(metric.height01[i].is_finite());
        // The camera head of the da3-base export estimates the field of view when none is given.
        if let Ok(cam) = std::env::var("JD_DA3_CAMERA_MODEL") {
            let est = depth_from_image(
                &path,
                Path::new(&model),
                518,
                1200,
                None,
                Some(Path::new(&cam)),
            )
            .expect("camera");
            assert!(est.metric && est.fov_source == "camera");
            let fov = est.fov_x_deg.expect("fov");
            assert!((10.0..170.0).contains(&fov), "fov {fov}");
            let ratio = est.depth_m[i] / canon.depth_m[i];
            assert!(
                ratio > 0.1 && ratio < 10.0,
                "scaled by the estimated focal: {ratio}"
            );
        } else {
            eprintln!("JD_DA3_CAMERA_MODEL unset: skipping the camera head");
        }
    }
}
