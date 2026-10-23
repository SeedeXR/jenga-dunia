# 2026-10-23 — DA3 intrinsics: the metric model has no camera head; the base export's does

Question (owner block, stage 2): can DA3 supply its own field of view so `vision.depth` gives
metres without `fov_x_deg`? Sources read verbatim by a research agent and spot-checked: the
DA3 configs and model code on GitHub (`ByteDance-Seed/Depth-Anything-3`, `src/depth_anything_3/
configs/*.yaml`, `model/da3.py`, `model/dpt.py`, `model/dualdpt.py`, `model/cam_dec.py`,
`model/utils/transform.py`, `utils/ray_utils.py`, `utils/alignment.py`), the safetensors
headers of the HF checkpoints (HTTP range reads), and the Hugging Face model cards of the ONNX
exports.

| Fact | Evidence |
|------|----------|
| `da3metric-large` (and `da3mono-large`) define only `net` + a `DPT` head (`output_dim: 1`, a sky head); no `cam_enc`, no `cam_dec`, no ray head | `configs/da3metric-large.yaml`; local `/tmp/jd-scratch/da3/config.json` is identical; the checkpoint's 406 tensors hold no `cam_*` keys |
| `da3-base` / `da3-large` use `DualDPT` (`depth`, `depth_conf`, `ray`, `ray_conf`) **plus** `cam_enc` + `cam_dec` (437 tensors incl. `model.cam_dec.fc_fov.*`) | `configs/da3-base.yaml`; safetensors header |
| Camera decoder → intrinsics: `fov_h = pose_enc[7]`, `fov_w = pose_enc[8]` (radians); `fx = (W/2)/tan(fov_w/2)`, `fy = (H/2)/tan(fov_h/2)`, principal point `(W/2, H/2)` | `model/utils/transform.py` `pose_encoding_to_extri_intri` |
| Metres: `depth × ((fx + fy)/2) / 300` | `utils/alignment.py` `apply_metric_scaling`; README FAQ |
| Upstream's own metres-without-a-camera path (`DepthAnything3Nested`) runs the any-view net for intrinsics and scales the metric net's depth with them | `model/da3.py` |
| `Heliosoph/da3-base-onnx`: Apache-2.0 (DA3-BASE is Apache; the NC Large/Giant are not part of it); `image` (1, 3, 504, 504) ImageNet-normalised; outputs `depth`, `depth_conf`, `extrinsics` (1, 1, 3, 4, near-identity for one view), `intrinsics` (1, 1, 3, 3) "at the 504×504 input grid (principal point at 252, 252)"; `model.onnx` 412 925 807 B, `model_fp16.onnx` 207 235 996 B; opset 17, fp32 trace matching PyTorch to 4.1e-7 | model card (`README.md` in the repo), HF API |
| Other exports: `SpatialHub/depth-anything-3-onnx` (fork-traced, sentinel −1 camera inputs, repo tagged CC-BY-NC with mixed contents), `zerochocobo/DepthAnything3_ONNX` (depth-only, ships an NC Large file) | model cards / stubs read with `onnx` |

Decision: `vision.depth {camera_model}` runs the base export on the same 504² tensor the metric
model sees and reads `intrinsics`; `fov_x = 2·atan(252/fx)`, metres by the official formula.
The ray-head route (`get_extrinsic_from_camray`: weighted RANSAC homography + QL decomposition)
is not exported by anyone and brings no documented intrinsics gain; a local export of our own
would reproduce the Heliosoph file (Python ≤ 3.13, torch < 2.5, `depth-anything-3 0.1.1`).

## Runs (Apple M4, CPU, ONNX Runtime 1.31; both sessions resident, 2.8 GB peak RSS)

| Image | camera head fov_x | MoGe-2 recovered | truth | DA3 metres with the estimate |
|-------|------------------|------------------|-------|------------------------------|
| Blender render, camera 60° (960 × 640) | **57.3°** | 69.4° | 60° | 0.9–10.6 m (true 7 m to > 100 m) |
| Hetch Hetchy 1914 (1024 × 718) | 36.8° | 57.5° | unknown (a 1914 view camera) | 1.3–4.2 m (a valley 1 km wide) |

The camera head is the better field-of-view estimate on the known-camera render (2.7° off
against MoGe's 9.4°); the metric head's scale is still an order of magnitude short on scenes
without scale cues, as documented in `2026-10-22-metric-depth.md`. Inference 2.3–2.5 s for the
pair at 504² (the metric model alone was 1.3–1.5 s).
