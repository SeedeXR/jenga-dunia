# 2026-10-26 — real-photo depth check: vicuñas on an Altiplano playa

Closes the open item "camera-height metric-depth scale ✔ (synthetic; a real-photo check needs a
photo with a known camera height)". The owner supplied a photograph (Unsplash, Persnickety Prints,
`rKY-BDYokVE`, Unsplash License — VERIFIED at unsplash.com/license 2026-10-26) without EXIF: no
focal length, no camera height. The ground truth comes from the scene instead.

## Ground truth from geometry

On a flat plane, an object's pixel height over the pixel distance from its feet to the eye-level
horizon is its height over the camera height: `S_px / (y_feet − y_h) = H / h`. Two adult vicuñas
of the same size give two such equations, so they fix the horizon row `y_h` and — for a pinhole
camera over a plane — the ratio of their distances, `d_far / d_near = (y_near − y_h) / (y_far − y_h)`,
with neither the focal length nor the camera height. Measured by eye on 4× zooms of the 6016 × 4016
original (`crates/jd-vision/fixtures/README.md` has every number):

| | Near adult | Far adult |
|---|---|---|
| Front hooves (row, original px) | 2479.5 | 2355 |
| Shoulder (withers → hooves) | 195 px | 134 px |

→ horizon row 2081.5 (2061–2100 for ±3 px on a shoulder), **distance ratio 1.455 (±3 %)**, shoulder ÷
camera height 0.49. Vicuña shoulder height is REPORTED as 75–85 cm (Wikipedia), 86–96 cm (Ultimate
Ungulate), about 90 cm (Britannica); 70–110 cm (Guinness) — sources disagree, so the camera stood
**1.53–1.96 m** high (a standing photographer). The third, grazing animal looks like a juvenile and
fits neither line; it is not used.

## The models against it (fixture 1504 × 1004, `max_px` 1504, 1 800 tokens, CPU)

| | MoGe-2 ViT-L | DA3 metric (focal from the DA3-base camera head) |
|---|---|---|
| Field of view | 77.4° (recovered) | 65.0° (camera head) |
| Depth at the near / far adult's feet | 20.5 / 26.2 m | 14.3 / 19.2 m |
| Far / near ratio (truth 1.455) | 1.277 (−12 %) | 1.344 (−8 %) |
| Camera height the depth implies, unaided (truth 1.53–1.96 m) | 2.17 m | 1.20 m |
| Ground-plane fit (lower image): implied height → scale for a 1.76 m prior | 2.72 m → × 0.65 | 0.95 m → × 1.85 |
| Shoulders with the 1.76 m prior (truth 0.75–0.96 m) | 0.69 / 0.61 m | 1.09 / 1.01 m |

## What it says

- On a real photograph with scale cues (animals, gravel, a horizon), both metric heads read the
  scene at **the right order of magnitude unaided** — MoGe-2 about 10–40 % far, DA3 20–40 % near.
  The earlier "an order of magnitude too close" (`2026-10-22-metric-depth.md`) came from a 1914
  valley photograph and a Blender render; it does not hold for an ordinary photograph.
- Both **compress distance**: the far animal reads 8–12 % too close relative to the near one.
- The **camera-height prior does not help here**: it fits the ground plane over the lower image
  (the near field, a few metres to ~10 m), where the models' scale differs by ~25 % from their
  scale at the animals (15–25 m); one factor cannot fix a depth error that grows with distance, so
  the prior turns each model's error into one of the other sign. It still helps where the model
  is far off (the synthetic test: ×10). A distance-dependent correction, or a prior placed on a
  known object rather than the ground, would be the next step.

## In the test pipeline

`crates/jd-vision/fixtures/vicunas-altiplano.jpg` (586 KB) and the test
`a_real_photograph_of_vicunas_on_a_playa_reads_at_the_right_depths` hold every model to: far /
near ratio within 15 % of 1.455, unaided camera height 1.0–3.0 m, a ground plane with > 90 %
inliers, primed shoulders 0.55–1.2 m and the two adults within 15 % of each other. The bounds pin
today's accuracy against regressions; they are not the truth. `scripts/pre-push.sh` exports
`JD_MOGE_MODEL`, `JD_DA3_MODEL`, `JD_DA3_CAMERA_MODEL` from `~/models/<moge|da3|da3base>/model.onnx`
when present, so the local gate runs it; CI has no models and skips it.
