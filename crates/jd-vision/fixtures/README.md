# jd-vision fixtures

## `vicunas-altiplano.jpg` — a real photograph with geometric ground truth

Vicuñas on a flat Altiplano playa (Andes). Photo by Persnickety Prints on Unsplash
(`unsplash.com/photos/rKY-BDYokVE`), downscaled 4× from the 6016 × 4016 original to 1504 × 1004,
JPEG quality 88. Licence: the [Unsplash License](https://unsplash.com/license) — "an irrevocable,
nonexclusive, worldwide copyright license to download, copy, modify, distribute, perform, and use
images from Unsplash for free, including for commercial purposes, without permission from or
attributing the photographer or Unsplash"; it may not be sold without significant modification
or compiled into a competing service (VERIFIED, read 2026-10-26). Credited here anyway.

The file carries no EXIF (no focal length, no camera height), so the ground truth comes from the
scene itself. Two adult vicuñas stand on flat ground; for two animals of the same size on a plane,
`shoulder_px = k · (feet_row − horizon_row)` with the same `k`, which fixes the eye-level horizon
row and the ratio of their distances without knowing the focal length or the camera height.
Measured by eye on 4× zooms of the original (rows from the top of this 1504 × 1004 fixture):

| Feature | Row | Column |
|---|---|---|
| Near adult: front hooves on the ground | 619.9 | 677 |
| Near adult: withers (back over the front legs) | 571.1 | 683 |
| Far adult: front hooves on the ground | 588.75 | 957 |
| Far adult: withers | 555.25 | 964 |
| Eye-level horizon (solved) | 520.4 (515.3–525.0 for ±3 original px on a shoulder) | — |

Derived: the far adult is **1.455× as far** as the near one (±3 %), whatever the lens; shoulder
height ÷ camera height = 0.49. Vicuña shoulder height is reported as 75–85 cm (Wikipedia), 86–96 cm
(Ultimate Ungulate) and about 90 cm (Britannica) — REPORTED, sources disagree — so the camera stood
**1.53–1.96 m** above the ground (1.76 m for an 86 cm shoulder): a standing photographer. The third
vicuña (grazing, at column ~1005) looks like a juvenile and is not used.

`cargo test -p jd-vision` checks the depth models against these numbers when the model files are
present (`JD_MOGE_MODEL`, `JD_DA3_MODEL`, `JD_DA3_CAMERA_MODEL`); `scripts/pre-push.sh` sets them
from `~/models/` when the files are there.
