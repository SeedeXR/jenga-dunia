# 2026-10-22 — metric depth: MoGe-2 focal/shift recovery in Rust, Depth Anything 3 metric

Closes the `jd-vision` follow-up (todo: "DA3 and metric depth (focal/shift recovery)") and
evidence-register item 8's DA3 half. Sources read verbatim (curl of the raw files, kept under
`/tmp/jd-scratch/moge/src/`): `microsoft/MoGe` `moge/utils/geometry_numpy.py`
(`normalized_view_plane_uv_numpy`, `solve_optimal_focal_shift`, `solve_optimal_shift`,
`recover_focal_shift_numpy`), `moge/model/v2.py` (`infer()`), `EasternJournalist/utils3d`
`utils3d/numpy/maps.py` (`masked_nearest_resize`), `ByteDance-Seed/Depth-Anything-3`
`src/depth_anything_3/utils/alignment.py` (`apply_metric_scaling`), the MoGe `docs/onnx.md` page
and the `Heliosoph/da3metric-large-onnx` model card (HF API: licence `apache-2.0`, files
`model.onnx`, `model_fp16.onnx`, `config.json`, `LICENSE.txt`).

## What the references say

| Fact | Source | Used as |
|------|--------|---------|
| The ONNX graph is the raw `forward()`; `infer()`'s focal/shift recovery and reprojection "cannot be exported to ONNX" | MoGe `docs/onnx.md` | why the port exists |
| `forward()` returns `mask.sigmoid()` and `metric_scale.exp()` | `v2.py` | the ONNX outputs are already probabilities / metres-per-unit |
| UV grid: `span_x = a/√(1+a²)`, `span_y = 1/√(1+a²)`, `u = linspace(−span_x·(W−1)/W, +…, W)` | `geometry_numpy.py` L33–45 | `view_uv`, `span_x` |
| `min |focal·xy/(z+shift) − uv|`: closed-form focal `Σ(xy_proj·uv)/Σ|xy_proj|²`, shift by `least_squares(x0=0, ftol=1e-3, method='lm')` | L82–115 | `recover_focal_shift` (cost identical; the 1-D search replaces LM) |
| Downsample to 64×64 with `masked_nearest_resize`: each target pixel takes the nearest valid source pixel inside its footprint | `recover_focal_shift_numpy` L118–141, `maps.py` L551–611 | the subsample loop |
| `infer()`: `points[..., 2] += shift`, `mask &= z > 0`, `depth = z`, `points *= metric_scale`, `depth *= metric_scale`; with `fov_x` given, `focal = span_x / tan(fov_x/2)` and the shift alone is solved | `v2.py` L286–307 | `depth_m`, the `fov_x_deg` input, `fov = 2·atan(span_x/focal)` |
| `apply_metric_scaling(depth, intrinsics, 300)`: `depth × ((fx + fy)/2) / 300` | DA3 `alignment.py` L118–133 | the DA3 metres |
| DA3 ONNX: `image` (1,3,504,504) ImageNet-normalised; `depth` (1,1,504,504) canonical "as seen through a 300 px focal length"; `sky ≥ 0.5` = sky | Heliosoph model card | the DA3 branch |

## Checks

- Unit test: a pinhole camera (focal 1.3, shift 0.8) seen through MoGe's affine convention, with a
  masked sky band, comes back to 1e-2 on both numbers; with the focal given, the shift alone.
- The Rust solver against MoGe's own scipy solver on the *same* ONNX outputs
  (`/tmp/jd-scratch/fovcheck/ref.py`, onnxruntime 1.31 + scipy in a scratch venv, the reference
  `solve_optimal_focal_shift` verbatim on an identical 64² subsample):

  | Image | Rust fov_x / depth_m | scipy LM fov_x / depth_m |
  |-------|----------------------|--------------------------|
  | Hetch Hetchy 1914 (1024 × 718) | 57.54° / 5.53–18.61 m | 57.57° / 5.51–18.72 m |
  | Blender EEVEE render, camera **60°** horizontal (960 × 640, `fovcheck/render.py`) | 69.40° / 3.6–80.9 m | 69.34° / 3.55–78.87 m |

  The port matches the reference; the 60° → 69° gap and the 5–19 m reading of a kilometre-wide
  valley are the model's estimates (its metric head learnt scenes with scale cues).
- `vision.depth` on the photograph: `fov_x_deg` 57.5, `depth_m` [5.5, 18.6], `metric_scale`
  6.96; with `fov_x_deg: 55` given, [6.1, 19.2].

## DA3 metric runs (`Heliosoph/da3metric-large-onnx` fp32, 1.34 GB; `JD_DA3_MODEL` test passes: the
graph is recognised by `depth` + `sky`, a 60° field of view scales the canonical depth by
436.5/300 to 1e-2)

| Image | `fov_x_deg` | `depth_m` | masked | inference | notes |
|-------|-------------|-----------|--------|-----------|-------|
| Hetch Hetchy (1024 × 718) | none | 0.52–1.66 (canonical) | 2.6 % | 1.45 s, 2.5 GB peak | `metric: false`; `metric: true` refused (`E_INPUT_INVALID`) |
| Hetch Hetchy | 57.5 (MoGe's estimate) | 0.97–3.08 m | 2.6 % | 1.33 s | |
| Blender render, camera 60° | 60 | 1.07–12.6 m | 32 % (sky) | 1.35 s | true range ≈ 7 m (bottom edge) to > 100 m |

The canonical depth grows with distance (`da3ref.py`: column 252, rows 500 → 200: 0.65, 0.74,
0.89, 1.20, 2.08, 2.59, 4.35), so it is depth, not disparity, and the scaling is the official
`× mean(fx, fy) / 300`; the model still reads the synthetic scene an order of magnitude too
close, as MoGe-2 reads the valley. Both metric heads were trained on scenes with scale cues;
neither recovers a landscape's scale from one picture, and the docs say so.
