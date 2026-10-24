# Image and video metadata, and the sun (2026-10-24)

> Research note behind `jd-media` and the `media.metadata` op. Implemented as recommended:
> `kamadak-exif` 0.6.1 for EXIF, a plain scan for the XMP packet (GPano attributes and elements), a
> hand-written ISO BMFF walk for video (`mvhd`, `©xyz`, Apple `mdta` keys, `st3d` / `sv3d` / V1
> `uuid`), and the NOAA sun formulas (tests: NOAA sheet rows and the SPA case). **Verified on real
> files:** a Samsung and a Pixel photo (UTC time, size and 35 mm focal length match `mdls`; the Pixel
> filename's UTC time matches to the millisecond; Samsung's GPS 0,0 when location is off is read as
> no position), two MOV/MP4 files (duration and creation time match `ffprobe`), and a CC BY-SA
> Android photo sphere from Wikimedia Commons (Cmglee, Monte Croce, Wikimania 2016): GPano read,
> `FirstPhotoDate` used as the UTC time (EXIF `DateTime` was local, two hours off), and the sun's
> column in the image lies **east** of the computed north column (108° vs the computed 131° at the
> Esino Lario venue) — the clockwise-right convention holds; the 23° gap is the tablet compass's
> pose heading. Not done: GoPro GPMF, magnetic declination, sun detection in the image to correct a
> compass heading (follow-up).

I found everything except who-writes-what on 360 camera exports, which is marked unverified below. The sun function is written and tested: it matches NOAA's own spreadsheet to within 1e-6° and the NREL SPA reference case to within 0.003°. Recommendation: no new metadata crate for photos, add `kamadak-exif` 0.6.1, and decide later whether `nom-exif` 3.8.0 earns its place for video. No repository files were changed; scratch work was kept outside the repo.

Tags used below: **[V]** = checked against the cited source or run here. **[U]** = unverified.

## 1. Which metadata matters, with exact tags

### EXIF
Quotes are from the Exif 2.31 spec (CIPA DC-008-2016 translation, https://people.cs.umass.edu/~liberato/courses/2017-spring-compsci365/files/DC-008-Translation-2016-E.pdf) and the ExifTool tag tables (https://exiftool.org/TagNames/EXIF.html, https://exiftool.org/TagNames/GPS.html).

- **DateTimeOriginal (0x9003)** [V]: "The format is "YYYY:MM:DD HH:MM:SS" with time shown in 24-hour format." It is local time with no zone.
- **OffsetTimeOriginal (0x9011)** [V]: "offset from UTC … including daylight saving time … of the time of DateTimeOriginal tag. The format … is "±HH:MM"."
- **SubSecTimeOriginal (0x9291)** [V]: fractional seconds.
- **GPS position** [V]:
  - GPSLatitude/GPSLongitude (0x2/0x4): "three RATIONAL values giving the degrees, minutes, and seconds."
  - GPSLatitudeRef 'N'/'S' and GPSLongitudeRef 'E'/'W' give the sign.
  - GPSAltitude (0x6) is in metres. GPSAltitudeRef (0x5): "If the altitude is below sea level, a value of 1 is given."
- **GPS time** [V]: GPSTimeStamp (0x7): "Indicates the time as UTC … hour, minute, and second." GPSDateStamp (0x1d) is "YYYY:mm:dd". Together they are an absolute UTC instant, independent of the camera clock.
- **Heading** [V]:
  - GPSImgDirectionRef (0x10): "'T' denotes true direction and 'M' is magnetic direction", Default 'T'.
  - GPSImgDirection (0x11): "direction of the image when it was captured … 0.00 to 359.99."
  - GPSDestBearing/Ref (0x18/0x17) use the same T/M convention. It is the bearing to the subject, so use it only as a fallback.
- **Focal length and sensor** [V]:
  - FocalLength (0x920a): "The actual focal length of the lens, in mm."
  - FocalLengthIn35mmFilm (0xa405): "equivalent focal length assuming a 35mm film camera, in mm. A value of 0 means the focal length is unknown." The spec also says "Digital zoom ratio is not taken into account."
  - FocalPlaneXResolution (0xa20e): "pixels in the image width (X) direction per FocalPlaneResolutionUnit." FocalPlaneResolutionUnit (0xa210): 2 = inches, 3 = cm.
- **Other** [V]: Make 0x010f, Model 0x0110, Orientation 0x0112 (values 1–8; 6 = "Rotate 90 CW", 8 = "Rotate 270 CW"), ExposureTime 0x829a, FNumber 0x829d, ISO 0x8827, ExposureCompensation 0x9204, PixelX/YDimension 0xa002/0xa003. XMP inside TIFF lives in tag 0x02bc.

### XMP Photo Sphere (GPano)
Source: https://developers.google.com/streetview/spherical-metadata [V]. Namespace `http://ns.google.com/photos/1.0/panorama/`.

- **ProjectionType** (required): "Google products currently support the value equirectangular only."
- **PoseHeadingDegrees**: "Compass heading, measured in degrees clockwise from North, for the center the image. Value must be >= 0 and < 360."
- **PosePitchDegrees**: "degrees above the horizon, for the center in the image."
- **PoseRollDegrees**: "As roll increases, the horizon rotates counterclockwise in the image."
- **Rotation**: "R = R_Z(-heading) * R_X(pitch) * R_Y(roll)", in a frame where "X is true east, and Y is true north". "a photo sphere with angles (0, 0, 0) will be oriented such that the center pixel faces due north."
- **InitialViewHeadingDegrees**: "heading angle of the initial view … not relative to the pano center." It is a viewer hint; ignore it for geometry.
- **Required size fields**: FullPanoWidthPixels, FullPanoHeightPixels, CroppedAreaImageWidthPixels, CroppedAreaImageHeightPixels, CroppedAreaLeftPixels, CroppedAreaTopPixels.

Real sample, Ricoh Theta S (https://old.imaging-resource.com/PRODS/ricoh-theta-s/EXIF/YR0010286.HTM) [V]: ProjectionType equirectangular, 5376×2688 with no crop, PoseHeadingDegrees 292.5 equal to GPSImgDirection 292.5 "True North", FocalLength 1.3 mm. Treat this file's 35 mm and focal tags as meaningless for 360 images.

### 360 video
- **Spherical Video V2** (https://github.com/google/spatial-media/blob/master/docs/spherical-video-v2-rfc.md) [V]:
  - `st3d` and `sv3d` sit inside the VisualSampleEntry, `moov/trak/mdia/minf/stbl/stsd/avc1|hvc1/…`, not at the top level.
  - `stereo_mode`: 0 = mono, 1 = top-bottom, 2 = left-right, 3 = custom, 4 = right-left.
  - `sv3d` holds `svhd` and `proj`. `proj` holds `prhd` and one of `equi` or `cbmp`.
  - `prhd` stores 16.16 fixed point. pose_yaw_degrees: "counter-clockwise rotation in degrees around the up vector". This is the opposite sense to GPano heading.
  - `equi` stores 0.32 crop fractions.
- **Spherical Video V1** (https://github.com/google/spatial-media/blob/master/docs/spherical-video-rfc.md) [V]: XML in `moov/trak/uuid` with UUID `ffcc8263-f855-4a93-8814-587a02521fdd`, namespace `http://ns.google.com/videos/1.0/spherical/`. Elements include Spherical, Stitched, ProjectionType, StereoMode, InitialViewHeadingDegrees. V1 is superseded by V2.

### Camera-native 360 formats
- **Insta360** `.insp` and `.insv` hold unstitched dual fisheye per secondary sources [U]. Only exports from Insta360 Studio are equirectangular.
- **GoPro MAX** `.360` holds two video tracks forming an EAC cubemap (https://www.trekview.org/blog/reverse-engineering-gopro-360-file-format-part-1/) [U, secondary].
- I did not find a primary source on whether either vendor's exports carry `sv3d` or GPano [U]. Plan: if a file is 2:1 with no metadata, report it as a "possible equirectangular" hint and let the user decide.

### Ordinary video metadata
- **mvhd creation time** [V, Apple QTFF]: "seconds since midnight, January 1, 1904, preferably using coordinated universal time (UTC)." ExifTool warns that cameras often store local time instead (https://exiftool.org/TagNames/QuickTime.html).
- **Apple keys** (Apple QuickTime metadata keys doc) [V]:
  - `com.apple.quicktime.location.ISO6709`: "Geographic point location by coordinates as defined in ISO 6709:2008. For example, "+27.5916+086.5640+8850/"."
  - `com.apple.quicktime.direction.facing`: "+20.34M/-5.3" means a heading of 20.34° magnetic, looking 5.3° down.
- **Android** writes `moov/udta/©xyz`. I confirmed this from the `nom-exif` source, which reads both keys [V].
- **GoPro GPMF** (https://github.com/gopro/gpmf-parser, MIT/Apache) [V]:
  - Stored in a `meta` track with a `gpmd` sample description.
  - GPS5 = "latitude, longitude, altitude (WGS 84), 2D ground speed, and 3D speed", about 18 Hz. GPSU = UTC.
  - GPS9 arrived with HERO11. "No GPS receiver in HERO12". GPS returns on HERO13.
  - The MAX section lists no GPS fields.
- **Cheap to read without ffmpeg**: everything above is a short box walk. GPMF needs a KLV decoder, which is about 100 lines. I'd skip GPMF until someone asks for it.

## 2. Crates
Our `deny.toml` allow-list matches what you listed, plus "Apache-2.0 WITH LLVM-exception", with `confidence-threshold = 0.8`. Data is from the crates.io API as of today.

| Crate | Latest | Licence | Formats | GPS | XMP | Notes |
|---|---|---|---|---|---|---|
| **image** (already locked, 0.25.10) | — | MIT/Apache | `exif_metadata()` / `xmp_metadata()` for JPEG, PNG, WebP, TIFF | raw bytes | **raw bytes** | Already a dependency of jd-vision with the `jpeg` feature. The zune-jpeg 0.5.15 underneath strips the `Exif\0\0` and XMP prefixes [V, read source]. |
| **kamadak-exif** | 0.6.1 (2024-11-06) | BSD-2-Clause | TIFF/RAW, JPEG, HEIF/HEIC/AVIF, PNG, WebP | yes, all tags | no | One dependency, `mutate_once` (BSD-2). Pure Rust. Its `Reader::read_raw(Vec<u8>)` takes `image`'s blob directly. Slow release cadence. |
| **nom-exif** | 3.8.0 (2026-09-07) | MIT via license-file (crates.io shows "non-standard") | JPEG, PNG, WebP, HEIC, AVIF, TIFF, CR3, RAF; MP4, MOV, 3GP, MKV, WebM | yes; video via `©xyz` and `location.ISO6709` | only scans for the Motion Photo flag | Adds nom 8, iso6709parse, and geo-types, none of them locked. Reads mvhd `CreateDate`. **No `sv3d`/`st3d`** [V, grep of source]. cargo-deny should detect MIT from the licence text [U]; otherwise it needs a `[[licenses.clarify]]` entry. |
| little_exif | 0.6.23 | MIT/Apache | PNG, JPEG, HEIF, JXL, TIFF, WebP | yes | no | Read/write. Pulls in brotli and quick-xml 0.37, a second quick-xml version. |
| rexif | 0.7.6 | MIT | JPEG, TIFF only | yes | no | Too narrow. |
| mp4 | 0.14.0 (2023) | MIT | MP4 | — | — | Stale. |
| mp4parse | 0.17.0 (2023) | MPL-2.0 | MP4 | — | — | Stale. |
| re_mp4 | 0.5.1 | MIT | MP4 | — | — | — |
| mp4-atom | 0.16.2 | MIT/Apache | MP4 | — | — | None of these four decode `sv3d`, `©xyz` or GPMF for us. |
| xmp_toolkit | 1.12.1 | MIT/Apache | — | — | yes | C++ bindings. Not suitable. |
| xmpkit | 0.1.6 | MIT/Apache | — | — | yes | Too new to rely on. |
| solar-positioning | 0.7.2 | MIT | — | — | — | Has SPA and Grena3. |
| spa | 0.5.1 (2024) | Apache-2.0 | — | — | — | — |

The exif crate itself is also called `exif`, but on crates.io that name is a 2016 libexif wrapper (MIT), so ignore it.

**Recommendation**
- **Photos**: `image` (locked) for the raw EXIF and XMP blobs, `kamadak-exif = "0.6.1"` to parse EXIF (BSD-2, one tiny dependency, also covers HEIC), and the already-locked `quick-xml` 0.42 or a plain attribute scan for the roughly six GPano fields.
  - GPano appears both as attributes (`GPano:X="…"`) and as elements, so handle both.
- **Video**: a hand-written ISO BMFF walk of about 150 lines covering `mvhd`, `udta/©xyz`, `meta/keys+ilst`, `stsd/*/sv3d/proj/prhd`, `st3d` and the V1 `uuid`. That covers every 360 case, which `nom-exif` cannot.
  - The alternative is to add `nom-exif` 3.8.0 for video date and GPS and still hand-write the `sv3d` part.
  - Lazy pick: hand-write, since `sv3d` has to be written either way.
- **Sun**: no crate. The function below is about 40 lines and verified. `solar-positioning` 0.7.2 (MIT) is the upgrade if we ever need 0.0003° accuracy.

## 3. Sun position

### Sources
- **NOAA** (https://gml.noaa.gov/grad/solcalc/calcdetails.html) [V]:
  - The calculator is "based on equations from Astronomical Algorithms, by Jean Meeus."
  - Sunrise and sunset are "theoretically accurate to within a minute for locations between +/- 72° latitude, and within 10 minutes outside."
  - The spreadsheets are "only valid for dates between 1901 and 2099."
  - Refraction is modelled piecewise by elevation band. The spreadsheet uses `-20.772/tan(h)` below −0.575°, while the page says −20.774.
  - The page notes the calculator "is no longer actively supported or maintained."
- **NREL SPA** (Reda & Andreas, Solar Energy 76(5) 577–589, 2004; NREL/TP-560-34302, rev. Jan 2008):
  - ±0.0003°, years −2000 to 6000 (via search snippet [V-secondary]).
  - nrel.gov and midcdmz.nrel.gov do not resolve in DNS from here, so the paper could not be fetched directly.

### Formula set
I took it verbatim from NOAA's `NOAA_Solar_Calculations_day.ods` cell formulas and implemented it in Rust. The file is a scratch file (now `crates/jd-media/src/lib.rs`, `sun_position`): `sun_position(unix_s, lat, lon) -> (azimuth° clockwise from north, refracted elevation°)`, f64.

Steps, with T = Julian century:
1. JD = unix/86400 + 2440587.5.
2. Mean longitude L0, mean anomaly M, orbital eccentricity e, equation of centre C.
3. Apparent longitude λ = L0 + C − 0.00569 − 0.00478·sin Ω.
4. Obliquity ε = ε0 + 0.00256·cos Ω. Declination δ = asin(sin ε · sin λ).
5. Equation of time from y = tan²(ε/2).
6. True solar time = UTC minutes + EoT + 4·lon. Hour angle = TST/4 ∓ 180.
7. Zenith from the cosine law. Azimuth from the spreadsheet's branch formula.
8. Add the NOAA refraction bands.

### Test vectors (all passing with `rustc --test`) [V]
- **NOAA's own spreadsheet**, cached values at lat 40, lon −105, 2010-06-21:
  - 19:00Z (noon, tz −7) → az 178.533230037881, el 73.4384793439627. We get 178.533230 / 73.438479.
  - 2010-06-22T01:00Z → az 288.099272917031, el 15.2132018370298. We get 288.099273 / 15.213202.
  - Tolerance 1e-4; the actual match is about 1e-6.
- **SPA reference case**, confirmed in pvlib's `tests/test_spa.py` (https://github.com/pvlib/pvlib-python/blob/main/tests/test_spa.py):
  - Inputs: 2003-10-17 12:30:30 MST, lat 39.742476, lon −105.1786, elevation 1830.14 m, 820 mbar, 11 °C, ΔT 67 s.
  - Expected: theta = 50.11162, Phi = 194.340241. Your quoted values are correct.
  - Our function gives zenith 50.108638 and azimuth 194.342583, so asserting within 0.01° holds. The remaining gap is SPA's topocentric parallax and pressure/temperature refraction.

## 4. Mapping metadata onto the product

### Horizontal FOV
- **What "35 mm equivalent" means**: the Exif spec says only "equivalent focal length assuming a 35mm film camera". The common definition is by diagonal (CIPA DCG-001, via https://en.wikipedia.org/wiki/35_mm_equivalent_focal_length) [V-secondary]. Whether each camera follows it is [U].
- **General formula** (diagonal-based, works for any aspect): `hfov = 2·atan( 21.635 · W/√(W²+H²) / f35 )`, where W and H are the displayed dimensions after applying Orientation.
  - For 3:2 this reduces exactly to your `2·atan(36/(2·f35))`.
  - For 4:3 the equivalent sensor width is 34.6 mm, not 36. At f35 = 26 that gives 67.3° instead of 69.4°.
  - **Portrait** (Orientation 5–8, or H > W): use the short side as W. With a 4:3 sensor the equivalent width is 25.96 mm.
- **Sensor route** (preferred when present): `sensor_w_mm = PixelXDimension / FocalPlaneXResolution × unit_mm` (inch = 25.4, cm = 10), then `hfov = 2·atan(sensor_w/(2·FocalLength))`.
- **Digital zoom**: divide tan(hfov/2) by DigitalZoomRatio when it is greater than 1.
- **Cropped images**: if the decoded width differs from PixelXDimension, flag the FOV as untrustworthy.
- Report which route was used in `fov_source`: `sensor` | `f35_diag` | `none`.

### Camera heading
- GPSImgDirection with Ref 'T' is a true bearing; use it directly.
- With Ref 'M', true = magnetic + declination. Declination needs a magnetic model such as WMM, which we don't have. So return `heading_ref: "magnetic"` and leave correction to the caller; don't fake it.
- Video: `com.apple.quicktime.direction.facing` (the "+20.34M/-5.3" form).

### North column in an equirectangular image
- GPano fixes only the centre column: the centre faces PoseHeadingDegrees, and (0,0,0) means the centre faces north.
- Assuming the usual layout where moving right in the image means turning clockwise (the spec does not state the pixel mapping) [U], the heading of full-pano column u is `PoseHeading + (u/FullPanoWidth − 0.5)·360`.
- So north is at `u_N = FullPanoWidth·(0.5 − PoseHeading/360) mod FullPanoWidth`, and in the image `x_N = u_N − CroppedAreaLeftPixels`.
- Video `prhd` yaw is counter-clockwise, so `heading = −yaw` [U as to sign in practice].
- Check: the Theta S sample has PoseHeading equal to GPSImgDirection, so one real photo with a visible sun will validate the sign.

### Driving the product from a 360 image
- **(a) Sky and sun**: compute the sun from capture time plus GPS with the function above. If either is missing, fall back to the peak brightness in the upper half of the equirectangular image: column gives azimuth via the north mapping, row gives elevation = 90 − 180·v/H.
  - `SkyParams.sun_elevation_deg` must be in (0, 90], so a night or below-horizon capture should return `sun.below_horizon: true` and not be sent to the sky op.
  - Sky azimuth convention is "degrees clockwise from north", which matches.
- **(b) DEM fetch**: `geo.fetch_dem` takes `center: [lon, lat]` (**longitude first**) plus `radius_m` (`crates/jd-core/src/geo.rs:28`). The op must swap the order from EXIF.
- **(c) Depth**: our perspective models cannot take an equirectangular image directly.
  - The standard approach is 360MonoDepth (Rey-Area, Yuan, Richardt, CVPR 2022, https://arxiv.org/abs/2111.15669): "monocular depth estimation from high-resolution 360° images using tangent images". Per-face estimates are recombined "with deformable multi-scale alignment followed by gradient-domain blending".
  - What we would need: a reprojection from the equirectangular image to N perspective crops with known FOV. That FOV goes into `vision.depth`'s `fov_x_deg`, which makes DA3 metric. Then a reprojection back, plus per-face scale alignment.
  - A minimal version is about 6 cube faces at 90° FOV with a median-scale match on the overlaps.
  - Native alternative: Depth Any Panoramas (arXiv 2512.16913, CVPR 2026). Weights are at `Insta360-Research/DAP-weights` as `model.pth`, licensed **CC BY-NC 4.0**, so not usable commercially and not ONNX.

## Proposed op: `media.metadata {path}`

```
{ kind: "photo"|"photo360"|"video"|"video360",
  width, height,                        // displayed, after Orientation
  orientation,                          // 1..8
  captured_at_utc,                      // GPS date+time > DateTimeOriginal+OffsetTimeOriginal > mvhd/creationdate
  captured_at_source: "gps"|"exif_offset"|"exif_local_assumed"|"mvhd",
  local_offset,                         // "+03:00" or null
  gps: {lat, lon, alt_m} | null,
  heading_deg, heading_ref: "true"|"magnetic"|null, heading_source: "GPSImgDirection"|"GPano"|"direction.facing",
  fov_x_deg, fov_source: "sensor"|"f35_diag"|"none",   // null for 360
  camera: {make, model, focal_mm, focal_35mm},
  exposure: {exposure_s, f_number, iso, ev100},        // ev100 = log2(N²/t) − log2(ISO/100)
  projection: {type: "equirectangular"|"cubemap"|null, pose_heading_deg, pose_pitch_deg, pose_roll_deg,
               full_w, full_h, crop_left, crop_top, stereo: "mono"|"top-bottom"|...|null, north_column_px},
  sun: {azimuth_deg, elevation_deg, below_horizon, source: "computed"|"none"},
  raw_tags_count }
```

**How it feeds the other ops**
- `vision.depth` gets `fov_x_deg`, only when the source is not `none`.
- `geo.fetch_dem` gets `center: [gps.lon, gps.lat]` plus `radius_m`.
- `sky.*` gets `sun_azimuth_deg` and `sun_elevation_deg`, only when `elevation_deg > 0`.

**Placement**: sun math and EXIF parsing in a new module, for example `jd-vision/src/media.rs` (it already has `image` with jpeg), and the op in `ops.rs`. Remember to regenerate the schemas and `ops.gen.ts` per the push-gating rule.

**Skipped**: GPMF tracks, magnetic declination, and Matroska. Add them when a real GoPro file or a magnetic-only device turns up.