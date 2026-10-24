//! What a photo, a 360° panorama or a video says about where, when and how it was taken, and
//! where the sun stood — the facts a reference image can contribute to a world before anyone
//! guesses (`docs/research/media-metadata.md`).
//!
//! - **Photos** (JPEG, PNG, TIFF, HEIF, WebP): EXIF through `kamadak-exif` — capture time (GPS UTC
//!   first, then `DateTimeOriginal` + `OffsetTimeOriginal`), GPS position and altitude, the
//!   compass heading (`GPSImgDirection`, true or magnetic), focal length and sensor for the field
//!   of view, exposure. XMP Photo Sphere (`GPano:*`) marks equirectangular panoramas and gives
//!   their pose, from which the column facing north follows.
//! - **Videos** (MP4, MOV): an ISO BMFF box walk of `moov` only — `mvhd` creation time and
//!   duration, the track size, Android `©xyz` and Apple `mdta` location / facing keys, and
//!   Spherical Video V2 (`st3d`, `sv3d/proj/prhd`) or V1 (`uuid` XML) for 360° video.
//! - **Sun**: NOAA's solar position (Meeus; the `NOAA_Solar_Calculations_day` formulas), azimuth
//!   clockwise from true north and refracted elevation — checked against NOAA's own sheet (1e-6°)
//!   and the NREL SPA reference case (0.003°).
//!
//! ponytail: no GoPro GPMF tracks, no Matroska, no magnetic declination model (a magnetic heading
//! is reported as such, not corrected); add them when a file that needs them turns up.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use jd_types::{JdError, JdResult};
use schemars::JsonSchema;
use serde::Serialize;

/// Everything read from one file. Absent facts are `null`, never guessed.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct MediaMetadata {
    /// `photo`, `photo360`, `video` or `video360`.
    pub kind: String,
    /// Container: `jpeg`, `png`, `tiff`, `heif`, `webp`, `mp4` or `mov`.
    pub format: String,
    /// Displayed width and height in pixels (after the EXIF orientation).
    pub px: Option<[u32; 2]>,
    /// EXIF orientation 1..8 (6 and 8 are portrait shots stored sideways).
    pub orientation: Option<u16>,
    /// Capture instant, UTC, RFC 3339.
    pub captured_at_utc: Option<String>,
    /// Where the instant came from: `gps` (satellite UTC), `exif_offset` (camera clock + its UTC
    /// offset), `gpano_first_photo_date` (a photo sphere's first source photo, UTC),
    /// `exif_local_assumed_utc` (camera clock, no offset — may be hours off), `mvhd` or
    /// `quicktime_creationdate`.
    pub captured_at_source: Option<String>,
    /// The camera's UTC offset at capture, `±HH:MM`.
    pub local_offset: Option<String>,
    /// Position, WGS 84.
    pub gps: Option<Gps>,
    /// Compass direction the camera faced, degrees clockwise from north.
    pub heading_deg: Option<f64>,
    /// `true` or `magnetic` (a magnetic heading is not corrected for declination).
    pub heading_ref: Option<String>,
    /// Where the heading came from.
    pub heading_source: Option<String>,
    /// Camera pitch, degrees above the horizon (Apple video `direction.facing`).
    pub pitch_deg: Option<f64>,
    /// Horizontal field of view, degrees (flat photos only).
    pub fov_x_deg: Option<f64>,
    /// `sensor` (focal length + focal-plane resolution), `f35_diag` (35 mm equivalent by the
    /// diagonal) or null.
    pub fov_source: Option<String>,
    /// Camera make, model and lens.
    pub camera: Option<Camera>,
    /// Exposure settings.
    pub exposure: Option<Exposure>,
    /// Panorama / 360° video projection, when the file declares one.
    pub projection: Option<Projection>,
    /// Video duration, seconds.
    pub duration_s: Option<f64>,
    /// The sun at the capture place and instant (needs `gps` and `captured_at_utc`).
    pub sun: Option<Sun>,
    /// Things the file suggests but does not state (e.g. a 2:1 image with no panorama tags).
    pub hints: Vec<String>,
}

/// A WGS 84 position.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
pub struct Gps {
    /// Latitude, degrees north.
    pub lat: f64,
    /// Longitude, degrees east.
    pub lon: f64,
    /// Altitude above sea level, metres.
    pub alt_m: Option<f64>,
}

/// Camera identity and optics.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Camera {
    /// Manufacturer.
    pub make: Option<String>,
    /// Model.
    pub model: Option<String>,
    /// Real focal length, mm.
    pub focal_mm: Option<f64>,
    /// 35 mm equivalent focal length, mm.
    pub focal_35mm: Option<f64>,
}

/// Exposure settings.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Exposure {
    /// Shutter time, seconds.
    pub exposure_s: Option<f64>,
    /// Aperture f-number.
    pub f_number: Option<f64>,
    /// ISO sensitivity.
    pub iso: Option<u32>,
    /// Exposure value at ISO 100: log2(N²/t) − log2(ISO/100). Bright sunlight ≈ 15, overcast ≈ 12.
    pub ev100: Option<f64>,
}

/// A declared projection.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Projection {
    /// `equirectangular` or `cubemap`.
    pub r#type: String,
    /// Heading of the full panorama's centre column, degrees clockwise from north.
    pub pose_heading_deg: Option<f64>,
    /// Pitch, degrees.
    pub pose_pitch_deg: Option<f64>,
    /// Roll, degrees.
    pub pose_roll_deg: Option<f64>,
    /// Full panorama size (GPano), pixels.
    pub full_px: Option<[u32; 2]>,
    /// Left and top of the stored crop inside the full panorama, pixels.
    pub crop_left_top: Option<[u32; 2]>,
    /// Column of the stored image that faces true north (needs a heading).
    pub north_column_px: Option<f64>,
    /// `mono`, `top-bottom`, `left-right`, `custom` or `right-left` (video).
    pub stereo: Option<String>,
}

/// The sun's position.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
pub struct Sun {
    /// Azimuth, degrees clockwise from true north.
    pub azimuth_deg: f64,
    /// Elevation above the horizon with atmospheric refraction, degrees.
    pub elevation_deg: f64,
    /// True when the sun was below the horizon (night or twilight).
    pub below_horizon: bool,
}

fn err(path: &Path, msg: impl Into<String>) -> JdError {
    JdError::invalid("path", msg).detail("path", path.display().to_string())
}

/// Read the metadata of the photo or video at `path`.
pub fn read_metadata(path: &Path) -> JdResult<MediaMetadata> {
    let mut f = File::open(path).map_err(|e| err(path, format!("cannot open: {e}")))?;
    let mut head = [0u8; 16];
    let n = f
        .read(&mut head)
        .map_err(|e| err(path, format!("cannot read: {e}")))?;
    let head = &head[..n];
    let format = if head.starts_with(&[0xFF, 0xD8]) {
        "jpeg"
    } else if head.starts_with(b"\x89PNG") {
        "png"
    } else if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        "tiff"
    } else if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        "webp"
    } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
        match &head[8..12] {
            b"heic" | b"heix" | b"mif1" | b"msf1" | b"heim" | b"heis" | b"avif" => "heif",
            b"qt  " => "mov",
            _ => "mp4",
        }
    } else if head.len() >= 8 && matches!(&head[4..8], b"moov" | b"wide" | b"mdat" | b"free") {
        "mov"
    } else {
        return Err(err(
            path,
            "not a photo or video this reader knows (JPEG, PNG, TIFF, HEIF, WebP, MP4, MOV)",
        ));
    };
    f.seek(SeekFrom::Start(0))
        .map_err(|e| err(path, e.to_string()))?;
    let mut m = if matches!(format, "mp4" | "mov") {
        read_video(&mut f, path)?
    } else {
        read_photo(f, path)?
    };
    m.format = format.into();
    if let (Some(g), Some(t)) = (m.gps, m.captured_at_utc.as_deref().and_then(parse_rfc3339)) {
        let (az, el) = sun_position(t, g.lat, g.lon);
        m.sun = Some(Sun {
            azimuth_deg: az,
            elevation_deg: el,
            below_horizon: el <= 0.0,
        });
    }
    Ok(m)
}

// ---------------------------------------------------------------------------------------------
// Photos

fn read_photo(f: File, path: &Path) -> JdResult<MediaMetadata> {
    use exif::{In, Tag, Value};
    let mut m = MediaMetadata {
        kind: "photo".into(),
        ..Default::default()
    };
    let bytes = {
        let mut b = Vec::new();
        BufReader::new(f)
            .read_to_end(&mut b)
            .map_err(|e| err(path, e.to_string()))?;
        b
    };
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(&bytes))
        .ok();
    let field = |t: Tag| {
        exif.as_ref()
            .and_then(|e| e.get_field(t, In::PRIMARY))
            .map(|f| &f.value)
    };
    let num = |t: Tag| -> Option<f64> {
        match field(t)? {
            Value::Rational(v) => v.first().filter(|r| r.denom != 0).map(|r| r.to_f64()),
            Value::SRational(v) => v.first().filter(|r| r.denom != 0).map(|r| r.to_f64()),
            Value::Short(v) => v.first().map(|x| *x as f64),
            Value::Long(v) => v.first().map(|x| *x as f64),
            Value::Float(v) => v.first().map(|x| *x as f64),
            Value::Double(v) => v.first().copied(),
            _ => None,
        }
    };
    let text = |t: Tag| -> Option<String> {
        match field(t)? {
            Value::Ascii(v) => v
                .first()
                .map(|s| {
                    String::from_utf8_lossy(s)
                        .trim_matches(char::from(0))
                        .trim()
                        .to_owned()
                })
                .filter(|s| !s.is_empty()),
            _ => None,
        }
    };
    let dms = |t: Tag| -> Option<f64> {
        match field(t)? {
            Value::Rational(v) if v.len() >= 3 && v.iter().all(|r| r.denom != 0) => {
                Some(v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0)
            }
            _ => None,
        }
    };
    m.orientation = num(Tag::Orientation)
        .map(|o| o as u16)
        .filter(|o| (1..=8).contains(o));
    // Pixel size: decode the header where we can, else trust the EXIF dimensions.
    let stored = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .or_else(|| {
            Some((
                num(Tag::PixelXDimension)? as u32,
                num(Tag::PixelYDimension)? as u32,
            ))
        });
    let portrait = matches!(m.orientation, Some(5..=8));
    m.px = stored.map(|(w, h)| if portrait { [h, w] } else { [w, h] });

    // Where.
    if let (Some(lat), Some(lon)) = (dms(Tag::GPSLatitude), dms(Tag::GPSLongitude)) {
        let s = |t: Tag, neg: &str| {
            if text(t).is_some_and(|r| r.eq_ignore_ascii_case(neg)) {
                -1.0
            } else {
                1.0
            }
        };
        let below =
            matches!(field(Tag::GPSAltitudeRef), Some(Value::Byte(v)) if v.first() == Some(&1));
        let alt = num(Tag::GPSAltitude).map(|a| if below { -a } else { a });
        let (lat, lon) = (
            lat * s(Tag::GPSLatitudeRef, "S"),
            lon * s(Tag::GPSLongitudeRef, "W"),
        );
        if lat.abs() <= 90.0 && lon.abs() <= 180.0 && !(lat == 0.0 && lon == 0.0) {
            m.gps = Some(Gps {
                lat,
                lon,
                alt_m: alt,
            });
        }
    }
    // When: satellite UTC first, then the camera clock with its offset, then the bare clock.
    let gps_time = match (text(Tag::GPSDateStamp), field(Tag::GPSTimeStamp)) {
        (Some(d), Some(Value::Rational(t))) if t.len() >= 3 && t.iter().all(|r| r.denom != 0) => {
            let secs = t[0].to_f64() * 3600.0 + t[1].to_f64() * 60.0 + t[2].to_f64();
            parse_exif_date(&d)
                .map(|(y, mo, da)| days_from_civil(y, mo, da) as f64 * 86400.0 + secs)
        }
        _ => None,
    };
    m.local_offset = text(Tag::OffsetTimeOriginal).filter(|o| parse_offset(o).is_some());
    let clock = text(Tag::DateTimeOriginal)
        .or_else(|| text(Tag::DateTime))
        .and_then(|s| parse_exif_datetime(&s));
    let sub = text(Tag::SubSecTimeOriginal)
        .and_then(|s| format!("0.{s}").parse::<f64>().ok())
        .unwrap_or(0.0);
    let (t, src) = match (
        gps_time,
        clock,
        m.local_offset.as_deref().and_then(parse_offset),
    ) {
        (Some(t), _, _) => (Some(t), "gps"),
        (None, Some(c), Some(off)) => (Some(c + sub - off as f64), "exif_offset"),
        (None, Some(c), None) => (Some(c + sub), "exif_local_assumed_utc"),
        _ => (None, ""),
    };
    if let Some(t) = t {
        m.captured_at_utc = Some(rfc3339(t));
        m.captured_at_source = Some(src.into());
    }
    // Heading.
    if let Some(dir) = num(Tag::GPSImgDirection).filter(|d| (0.0..=360.0).contains(d)) {
        m.heading_deg = Some(dir.rem_euclid(360.0));
        m.heading_ref = Some(
            if text(Tag::GPSImgDirectionRef).is_some_and(|r| r.eq_ignore_ascii_case("M")) {
                "magnetic"
            } else {
                "true"
            }
            .into(),
        );
        m.heading_source = Some("GPSImgDirection".into());
    }
    // Camera and exposure.
    let cam = Camera {
        make: text(Tag::Make),
        model: text(Tag::Model),
        focal_mm: num(Tag::FocalLength).filter(|f| *f > 0.0),
        focal_35mm: num(Tag::FocalLengthIn35mmFilm).filter(|f| *f > 0.0),
    };
    if cam.make.is_some() || cam.model.is_some() || cam.focal_mm.is_some() {
        m.camera = Some(cam.clone());
    }
    let ex = Exposure {
        exposure_s: num(Tag::ExposureTime).filter(|t| *t > 0.0),
        f_number: num(Tag::FNumber).filter(|n| *n > 0.0),
        iso: num(Tag::PhotographicSensitivity)
            .map(|i| i as u32)
            .filter(|i| *i > 0),
        ev100: None,
    };
    if ex.exposure_s.is_some() || ex.f_number.is_some() {
        let ev = match (ex.f_number, ex.exposure_s, ex.iso) {
            (Some(n), Some(t), Some(iso)) => Some((n * n / t).log2() - (iso as f64 / 100.0).log2()),
            _ => None,
        };
        m.exposure = Some(Exposure { ev100: ev, ..ex });
    }

    // Panorama (XMP GPano).
    let xmp = find_xmp(&bytes);
    let g = |k: &str| xmp.and_then(|x| xmp_value(x, &format!("GPano:{k}")));
    let gn = |k: &str| g(k).and_then(|v| v.trim().parse::<f64>().ok());
    if g("ProjectionType").is_some_and(|p| p.trim().eq_ignore_ascii_case("equirectangular")) {
        m.kind = "photo360".into();
        let full = match (gn("FullPanoWidthPixels"), gn("FullPanoHeightPixels")) {
            (Some(w), Some(h)) => Some([w as u32, h as u32]),
            _ => m.px,
        };
        let crop = [
            gn("CroppedAreaLeftPixels").unwrap_or(0.0) as u32,
            gn("CroppedAreaTopPixels").unwrap_or(0.0) as u32,
        ];
        // GPano: the pose heading is the full panorama's centre column. With no pose, a camera
        // heading (GPSImgDirection) is the usual stand-in (they agree on the Ricoh Theta sample).
        let heading = gn("PoseHeadingDegrees").or(m.heading_deg);
        let north = match (heading, full) {
            (Some(hd), Some([fw, _])) => {
                Some((fw as f64 * (0.5 - hd / 360.0)).rem_euclid(fw as f64) - crop[0] as f64)
            }
            _ => None,
        };
        if gn("PoseHeadingDegrees").is_some() {
            m.heading_deg = gn("PoseHeadingDegrees");
            m.heading_ref = Some("true".into());
            m.heading_source = Some("GPano:PoseHeadingDegrees".into());
        }
        // A stitched sphere's EXIF clock is the stitch time in local time with no offset; GPano's
        // FirstPhotoDate is the first source photo, in UTC.
        if m.captured_at_source.as_deref() != Some("gps")
            && let Some(t) = g("FirstPhotoDate").and_then(parse_rfc3339)
        {
            m.captured_at_utc = Some(rfc3339(t));
            m.captured_at_source = Some("gpano_first_photo_date".into());
        }
        m.projection = Some(Projection {
            r#type: "equirectangular".into(),
            pose_heading_deg: gn("PoseHeadingDegrees"),
            pose_pitch_deg: gn("PosePitchDegrees"),
            pose_roll_deg: gn("PoseRollDegrees"),
            full_px: full,
            crop_left_top: Some(crop),
            north_column_px: north,
            stereo: None,
        });
    } else if let Some([w, h]) = m.px {
        let (fov, src) = fov_x(&m, &cam, exif.as_ref(), num(Tag::DigitalZoomRatio));
        m.fov_x_deg = fov;
        m.fov_source = src.map(Into::into);
        if w == 2 * h {
            m.hints.push(
                "2:1 image without Photo Sphere (GPano) tags: possibly an equirectangular panorama"
                    .into(),
            );
        }
    }
    Ok(m)
}

/// Horizontal field of view of a flat photo.
fn fov_x(
    m: &MediaMetadata,
    cam: &Camera,
    exif: Option<&exif::Exif>,
    zoom: Option<f64>,
) -> (Option<f64>, Option<&'static str>) {
    use exif::{In, Tag, Value};
    let [w, h] = m.px.unwrap_or([0, 0]);
    if w == 0 || h == 0 {
        return (None, None);
    }
    let portrait = matches!(m.orientation, Some(5..=8));
    let get = |t: Tag| {
        exif.and_then(|e| e.get_field(t, In::PRIMARY))
            .map(|f| &f.value)
    };
    let rat = |t: Tag| match get(t) {
        Some(Value::Rational(v)) => v
            .first()
            .filter(|r| r.denom != 0 && r.num != 0)
            .map(|r| r.to_f64()),
        _ => None,
    };
    let unit_mm = match get(Tag::FocalPlaneResolutionUnit).and_then(|v| v.get_uint(0)) {
        Some(3) => Some(10.0),
        Some(2) | None => Some(25.4),
        _ => None,
    };
    let zoom = zoom.filter(|z| *z > 1.0).unwrap_or(1.0);
    // Sensor route: the displayed width runs along the sensor's long side unless the shot is portrait.
    let (dim, res) = if portrait {
        (Tag::PixelYDimension, Tag::FocalPlaneYResolution)
    } else {
        (Tag::PixelXDimension, Tag::FocalPlaneXResolution)
    };
    if let (Some(f), Some(px), Some(r), Some(u)) = (
        cam.focal_mm,
        get(dim).and_then(|v| v.get_uint(0)),
        rat(res),
        unit_mm,
    ) {
        let sensor_mm = px as f64 / r * u;
        if sensor_mm > 0.5 && sensor_mm < 100.0 {
            return (
                Some((2.0 * (sensor_mm / (2.0 * f) / zoom).atan()).to_degrees()),
                Some("sensor"),
            );
        }
    }
    // 35 mm equivalent, defined by the diagonal (43.27 mm): half-width = 21.635 · W / √(W² + H²).
    if let Some(f35) = cam.focal_35mm {
        let half = 21.635 * w as f64 / ((w as f64).hypot(h as f64));
        return (
            Some((2.0 * (half / f35 / zoom).atan()).to_degrees()),
            Some("f35_diag"),
        );
    }
    (None, None)
}

/// The XMP packet inside a file (JPEG APP1, PNG iTXt, TIFF, WebP all carry it as plain text).
fn find_xmp(bytes: &[u8]) -> Option<&str> {
    let start = find(bytes, b"<x:xmpmeta")?;
    let end = find(&bytes[start..], b"</x:xmpmeta>")? + start + b"</x:xmpmeta>".len();
    std::str::from_utf8(&bytes[start..end]).ok()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `key="value"` (attribute form) or `<key>value</key>` (element form).
fn xmp_value<'a>(xmp: &'a str, key: &str) -> Option<&'a str> {
    if let Some(i) = xmp.find(&format!("{key}=\"")) {
        let s = &xmp[i + key.len() + 2..];
        return s.find('"').map(|e| &s[..e]);
    }
    let open = format!("<{key}>");
    let i = xmp.find(&open)? + open.len();
    let s = &xmp[i..];
    s.find('<').map(|e| &s[..e])
}

// ---------------------------------------------------------------------------------------------
// Videos (ISO BMFF / QuickTime)

const MAX_MOOV: u64 = 64 << 20;

/// (type, payload) of each box in `buf`.
fn boxes(buf: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 8 <= buf.len() {
        let size32 = u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as u64;
        let ty = [buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]];
        let (size, hdr) = match size32 {
            0 => ((buf.len() - i) as u64, 8),
            1 if i + 16 <= buf.len() => (
                u64::from_be_bytes(buf[i + 8..i + 16].try_into().unwrap_or([0; 8])),
                16,
            ),
            s => (s, 8),
        };
        if size < hdr as u64 || i as u64 + size > buf.len() as u64 {
            break;
        }
        out.push((ty, &buf[i + hdr..i + size as usize]));
        i += size as usize;
    }
    out
}

fn child<'a>(buf: &'a [u8], ty: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(buf)
        .into_iter()
        .find(|(t, _)| t == ty)
        .map(|(_, p)| p)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_video(f: &mut File, path: &Path) -> JdResult<MediaMetadata> {
    let mut m = MediaMetadata {
        kind: "video".into(),
        ..Default::default()
    };
    // Find `moov` among the top-level boxes without reading `mdat`.
    let len = f.metadata().map_err(|e| err(path, e.to_string()))?.len();
    let mut pos = 0u64;
    let mut moov = None;
    while pos + 8 <= len {
        let mut h = [0u8; 16];
        f.seek(SeekFrom::Start(pos))
            .map_err(|e| err(path, e.to_string()))?;
        let n = f.read(&mut h).map_err(|e| err(path, e.to_string()))?;
        if n < 8 {
            break;
        }
        let size32 = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        let (size, hdr) = match size32 {
            0 => (len - pos, 8),
            1 if n >= 16 => (
                u64::from_be_bytes(h[8..16].try_into().unwrap_or([0; 8])),
                16,
            ),
            s => (s, 8),
        };
        if size < hdr {
            break;
        }
        if &h[4..8] == b"moov" {
            if size > MAX_MOOV {
                return Err(err(
                    path,
                    format!("moov box of {size} bytes is larger than {MAX_MOOV}"),
                ));
            }
            let mut b = vec![0u8; (size - hdr) as usize];
            f.seek(SeekFrom::Start(pos + hdr))
                .map_err(|e| err(path, e.to_string()))?;
            f.read_exact(&mut b).map_err(|e| err(path, e.to_string()))?;
            moov = Some(b);
            break;
        }
        pos += size;
    }
    let moov = moov.ok_or_else(|| err(path, "no `moov` box: not a playable MP4/MOV"))?;
    // mvhd: creation time (s since 1904-01-01) and duration.
    if let Some(mvhd) = child(&moov, b"mvhd") {
        let (created, scale, dur) = if mvhd.first() == Some(&1) {
            (
                mvhd.get(4..12)
                    .map(|s| u64::from_be_bytes(s.try_into().unwrap_or([0; 8]))),
                be32(mvhd, 20),
                mvhd.get(24..32)
                    .map(|s| u64::from_be_bytes(s.try_into().unwrap_or([0; 8]))),
            )
        } else {
            (
                be32(mvhd, 4).map(u64::from),
                be32(mvhd, 12),
                be32(mvhd, 16).map(u64::from),
            )
        };
        if let (Some(s), Some(d)) = (scale.filter(|s| *s > 0), dur) {
            m.duration_s = Some(d as f64 / s as f64);
        }
        if let Some(c) = created.filter(|c| *c > 0) {
            m.captured_at_utc = Some(rfc3339(c as f64 - 2_082_844_800.0)); // 1904 → 1970
            m.captured_at_source = Some("mvhd".into());
        }
    }
    for (_, trak) in boxes(&moov).into_iter().filter(|(t, _)| t == b"trak") {
        // Spherical Video V1: an XML `uuid` box in the track.
        for (_, p) in boxes(trak).into_iter().filter(|(t, _)| t == b"uuid") {
            if p.len() > 16
                && p[..16]
                    == [
                        0xff, 0xcc, 0x82, 0x63, 0xf8, 0x55, 0x4a, 0x93, 0x88, 0x14, 0x58, 0x7a,
                        0x02, 0x52, 0x1f, 0xdd,
                    ]
            {
                let xml = String::from_utf8_lossy(&p[16..]);
                if xmp_value(&xml, "GSpherical:Spherical").is_some_and(|v| v.trim() == "true") {
                    m.kind = "video360".into();
                    m.projection = Some(Projection {
                        r#type: xmp_value(&xml, "GSpherical:ProjectionType")
                            .unwrap_or("equirectangular")
                            .trim()
                            .to_lowercase(),
                        pose_heading_deg: xmp_value(&xml, "GSpherical:InitialViewHeadingDegrees")
                            .and_then(|v| v.trim().parse().ok()),
                        stereo: xmp_value(&xml, "GSpherical:StereoMode")
                            .map(|s| s.trim().to_lowercase()),
                        ..Default::default()
                    });
                }
            }
        }
        let Some(stsd) = child(trak, b"mdia")
            .and_then(|b| child(b, b"minf"))
            .and_then(|b| child(b, b"stbl"))
            .and_then(|b| child(b, b"stsd"))
        else {
            continue;
        };
        // A video track: its sample entry is a VisualSampleEntry (78 bytes before its children).
        let Some((_, entry)) = stsd.get(8..).map(boxes).and_then(|v| v.into_iter().next()) else {
            continue;
        };
        let (Some(w), Some(h)) = (entry.get(24..26), entry.get(26..28)) else {
            continue;
        };
        let (w, h) = (
            u16::from_be_bytes([w[0], w[1]]) as u32,
            u16::from_be_bytes([h[0], h[1]]) as u32,
        );
        let is_video = child(trak, b"mdia")
            .and_then(|b| child(b, b"hdlr"))
            .is_some_and(|hd| hd.get(8..12) == Some(b"vide"));
        if !is_video || w == 0 {
            continue;
        }
        m.px.get_or_insert([w, h]);
        let kids = entry.get(78..).unwrap_or(&[]);
        let stereo = child(kids, b"st3d")
            .and_then(|b| b.get(4))
            .map(|s| match s {
                0 => "mono",
                1 => "top-bottom",
                2 => "left-right",
                3 => "custom",
                _ => "right-left",
            });
        if let Some(sv3d) = child(kids, b"sv3d") {
            let proj = child(sv3d, b"proj");
            let prhd = proj.and_then(|p| child(p, b"prhd"));
            let fixed = |at: usize| {
                prhd.and_then(|b| be32(b, at))
                    .map(|v| v as i32 as f64 / 65536.0)
            };
            m.kind = "video360".into();
            // prhd yaw is counter-clockwise about the up axis; a compass heading is clockwise.
            let yaw = fixed(4);
            m.projection = Some(Projection {
                r#type: if proj.and_then(|p| child(p, b"cbmp")).is_some() {
                    "cubemap"
                } else {
                    "equirectangular"
                }
                .into(),
                pose_heading_deg: yaw.map(|y| (-y).rem_euclid(360.0)),
                pose_pitch_deg: fixed(8),
                pose_roll_deg: fixed(12),
                full_px: Some([w, h]),
                stereo: stereo.map(Into::into),
                ..Default::default()
            });
        } else if let (Some(p), Some(s)) = (m.projection.as_mut(), stereo) {
            p.stereo = Some(s.into());
        }
    }
    // Location: Android `udta/©xyz`, Apple `meta` keys.
    if let Some(xyz) = child(&moov, b"udta").and_then(|u| child(u, b"\xa9xyz")) {
        // u16 length, u16 language, then an ISO 6709 string.
        if let Some(s) = xyz.get(4..).map(String::from_utf8_lossy) {
            m.gps = parse_iso6709(&s);
        }
    }
    if let Some(meta) = child(&moov, b"meta") {
        // QuickTime `meta` has no version/flags; the ISO one does.
        let meta = if child(meta, b"hdlr").is_some() {
            meta
        } else {
            meta.get(4..).unwrap_or(&[])
        };
        let keys: Vec<String> = child(meta, b"keys")
            .map(|k| {
                let mut out = Vec::new();
                let mut i = 8usize;
                while let Some(sz) = be32(k, i) {
                    let sz = sz as usize;
                    if sz < 8 || i + sz > k.len() {
                        break;
                    }
                    out.push(String::from_utf8_lossy(&k[i + 8..i + sz]).into_owned());
                    i += sz;
                }
                out
            })
            .unwrap_or_default();
        for (ty, item) in child(meta, b"ilst").map(boxes).unwrap_or_default() {
            let idx = u32::from_be_bytes(ty) as usize;
            let Some(key) = idx.checked_sub(1).and_then(|i| keys.get(i)) else {
                continue;
            };
            let Some(data) = child(item, b"data") else {
                continue;
            };
            let value = String::from_utf8_lossy(data.get(8..).unwrap_or(&[])).into_owned();
            match key.as_str() {
                "com.apple.quicktime.location.ISO6709" => m.gps = parse_iso6709(&value).or(m.gps),
                "com.apple.quicktime.creationdate" => {
                    if let Some(t) = parse_rfc3339(&value) {
                        m.captured_at_utc = Some(rfc3339(t));
                        m.captured_at_source = Some("quicktime_creationdate".into());
                    }
                }
                "com.apple.quicktime.direction.facing" => {
                    // "+20.34M/-5.3": heading (T true / M magnetic) / pitch.
                    let (hd, pitch) = value.split_once('/').unwrap_or((value.as_str(), ""));
                    let mag = hd.ends_with('M');
                    if let Ok(h) = hd.trim_end_matches(['M', 'T']).parse::<f64>() {
                        m.heading_deg = Some(h.rem_euclid(360.0));
                        m.heading_ref = Some(if mag { "magnetic" } else { "true" }.into());
                        m.heading_source = Some("com.apple.quicktime.direction.facing".into());
                    }
                    m.pitch_deg = pitch.trim().parse().ok();
                }
                "com.apple.quicktime.make" | "com.apple.quicktime.model" => {
                    let c = m.camera.get_or_insert_with(Camera::default);
                    if key.ends_with("make") {
                        c.make = Some(value.trim().into())
                    } else {
                        c.model = Some(value.trim().into())
                    }
                }
                _ => {}
            }
        }
    }
    if m.kind == "video" && m.px.is_some_and(|[w, h]| w == 2 * h) {
        m.hints.push(
            "2:1 video without spherical metadata (sv3d / V1 uuid): possibly equirectangular"
                .into(),
        );
    }
    Ok(m)
}

/// `+27.5916+086.5640+8850/` → lat, lon, altitude.
fn parse_iso6709(s: &str) -> Option<Gps> {
    let s = s.trim().trim_end_matches('/');
    let mut parts = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if (c == '+' || c == '-') && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
        }
        if c.is_ascii_digit() || matches!(c, '+' | '-' | '.') {
            cur.push(c);
        } else {
            break;
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    let (lat, lon) = (
        parts.first()?.parse::<f64>().ok()?,
        parts.get(1)?.parse::<f64>().ok()?,
    );
    (lat.abs() <= 90.0 && lon.abs() <= 180.0).then(|| Gps {
        lat,
        lon,
        alt_m: parts.get(2).and_then(|a| a.parse().ok()),
    })
}

// ---------------------------------------------------------------------------------------------
// Time

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn rfc3339(unix_s: f64) -> String {
    let secs = unix_s.floor() as i64;
    let (y, mo, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    let frac = unix_s - secs as f64;
    let ms = (frac * 1000.0).round() as i64;
    if ms > 0 && ms < 1000 {
        format!(
            "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z",
            s / 3600,
            s % 3600 / 60,
            s % 60
        )
    } else {
        format!(
            "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z",
            s / 3600,
            s % 3600 / 60,
            s % 60
        )
    }
}

/// `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM|±HHMM)` → unix seconds.
fn parse_rfc3339(s: &str) -> Option<f64> {
    let s = s.trim();
    let (date, rest) = (s.get(..10)?, s.get(11..)?);
    let (y, mo, d) = (
        date.get(..4)?.parse().ok()?,
        date.get(5..7)?.parse().ok()?,
        date.get(8..10)?.parse().ok()?,
    );
    let tz_at = rest.find(['Z', '+', '-']).unwrap_or(rest.len());
    let (clock, tz) = rest.split_at(tz_at);
    let mut it = clock.split(':');
    let (h, mi): (f64, f64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    let sec: f64 = it.next().map(|x| x.parse().ok()).unwrap_or(Some(0.0))?;
    let off = if tz.is_empty() || tz == "Z" {
        0
    } else {
        parse_offset(tz)?
    };
    Some(days_from_civil(y, mo, d) as f64 * 86_400.0 + h * 3600.0 + mi * 60.0 + sec - off as f64)
}

fn parse_exif_date(s: &str) -> Option<(i64, u32, u32)> {
    let s = s.trim();
    Some((
        s.get(..4)?.parse().ok()?,
        s.get(5..7)?.parse().ok()?,
        s.get(8..10)?.parse().ok()?,
    ))
}

/// `YYYY:MM:DD HH:MM:SS` as if UTC.
fn parse_exif_datetime(s: &str) -> Option<f64> {
    let (y, mo, d) = parse_exif_date(s)?;
    let t = s.trim().get(11..19)?;
    let (h, mi, se): (f64, f64, f64) = (
        t.get(..2)?.parse().ok()?,
        t.get(3..5)?.parse().ok()?,
        t.get(6..8)?.parse().ok()?,
    );
    (1..=12)
        .contains(&mo)
        .then(|| days_from_civil(y, mo, d) as f64 * 86_400.0 + h * 3600.0 + mi * 60.0 + se)
}

/// `±HH:MM` or `±HHMM` → seconds east of UTC.
fn parse_offset(s: &str) -> Option<i64> {
    let s = s.trim();
    let sign = match s.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let digits: String = s[1..].chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 4 {
        return None;
    }
    let (h, m): (i64, i64) = (digits[..2].parse().ok()?, digits[2..].parse().ok()?);
    (h <= 14 && m < 60).then_some(sign * (h * 3600 + m * 60))
}

// ---------------------------------------------------------------------------------------------
// Sun

/// NOAA solar position (Meeus, as in `NOAA_Solar_Calculations_day.ods`): azimuth degrees
/// clockwise from true north, elevation degrees with NOAA's refraction bands. Valid 1901–2099.
pub fn sun_position(unix_s: f64, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
    let (sin, cos, rad, deg) = (f64::sin, f64::cos, f64::to_radians, f64::to_degrees);
    let jd = unix_s / 86_400.0 + 2_440_587.5;
    let t = (jd - 2_451_545.0) / 36_525.0;
    let l0 = (280.46646 + t * (36_000.769_83 + t * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + t * (35_999.050_29 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let c = sin(rad(m)) * (1.914602 - t * (0.004817 + 0.000014 * t))
        + sin(rad(2.0 * m)) * (0.019993 - 0.000101 * t)
        + sin(rad(3.0 * m)) * 0.000289;
    let omega = 125.04 - 1934.136 * t;
    let lambda = l0 + c - 0.00569 - 0.00478 * sin(rad(omega));
    let eps0 = 23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let eps = eps0 + 0.00256 * cos(rad(omega));
    let decl = (sin(rad(eps)) * sin(rad(lambda))).asin();
    let y = (rad(eps) / 2.0).tan().powi(2);
    let eot_min = 4.0
        * deg(y * sin(2.0 * rad(l0)) - 2.0 * e * sin(rad(m))
            + 4.0 * e * y * sin(rad(m)) * cos(2.0 * rad(l0))
            - 0.5 * y * y * sin(4.0 * rad(l0))
            - 1.25 * e * e * sin(2.0 * rad(m)));
    let tst = (unix_s.rem_euclid(86_400.0) / 60.0 + eot_min + 4.0 * lon_deg).rem_euclid(1440.0);
    let ha = tst / 4.0 - 180.0;
    let phi = rad(lat_deg);
    let zen = (sin(phi) * sin(decl) + cos(phi) * cos(decl) * cos(rad(ha)))
        .clamp(-1.0, 1.0)
        .acos();
    let el = 90.0 - deg(zen);
    let te = rad(el).tan();
    let refr = if el > 85.0 {
        0.0
    } else if el > 5.0 {
        58.1 / te - 0.07 / te.powi(3) + 0.000086 / te.powi(5)
    } else if el > -0.575 {
        1735.0 + el * (-518.2 + el * (103.4 + el * (-12.79 + el * 0.711)))
    } else {
        -20.772 / te
    } / 3600.0;
    let a = deg(((sin(phi) * cos(zen) - sin(decl)) / (cos(phi) * sin(zen)))
        .clamp(-1.0, 1.0)
        .acos());
    let az = if ha > 0.0 {
        (a + 180.0).rem_euclid(360.0)
    } else {
        (540.0 - a).rem_euclid(360.0)
    };
    (az, el + refr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exif::{Field, In, Rational, Tag, Value};
    use std::io::Write;

    #[test]
    fn sun_matches_noaa_and_the_spa_reference() {
        // NOAA_Solar_Calculations_day.ods cached rows: lat 40, lon −105, 2010-06-21, tz −7.
        let d0 = days_from_civil(2010, 6, 21) as f64 * 86_400.0;
        let (az, el) = sun_position(d0 + 19.0 * 3600.0, 40.0, -105.0);
        assert!(
            (az - 178.533230037881).abs() < 1e-4 && (el - 73.4384793439627).abs() < 1e-4,
            "{az} {el}"
        );
        let (az, el) = sun_position(d0 + 25.0 * 3600.0, 40.0, -105.0);
        assert!(
            (az - 288.099272917031).abs() < 1e-4 && (el - 15.2132018370298).abs() < 1e-4,
            "{az} {el}"
        );
        // NREL SPA example: 2003-10-17 12:30:30 −07:00 → zenith 50.11162°, azimuth 194.340241°
        // (SPA adds parallax and pressure refraction; NOAA agrees to 0.01°).
        let t = parse_rfc3339("2003-10-17T12:30:30-07:00").expect("time");
        let (az, el) = sun_position(t, 39.742476, -105.1786);
        assert!(
            (az - 194.340241).abs() < 0.01 && ((90.0 - el) - 50.11162).abs() < 0.01,
            "{az} {}",
            90.0 - el
        );
    }

    #[test]
    fn calendar_round_trips() {
        for (y, m, d) in [
            (1970, 1, 1),
            (2000, 2, 29),
            (2026, 10, 24),
            (1904, 1, 1),
            (2099, 12, 31),
        ] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
        assert_eq!(days_from_civil(1904, 1, 1) * 86_400, -2_082_844_800);
        assert_eq!(
            rfc3339(parse_rfc3339("2026-10-24T20:15:30+03:00").expect("t")),
            "2026-10-24T17:15:30Z"
        );
        assert_eq!(parse_offset("-0530"), Some(-(5 * 3600 + 30 * 60)));
    }

    fn rat(v: f64, den: u32) -> Rational {
        Rational {
            num: (v * den as f64).round() as u32,
            denom: den,
        }
    }

    /// A small JPEG with the given EXIF fields and optional XMP spliced in after SOI.
    fn jpeg(fields: &[Field], xmp: Option<&str>, w: u32, h: u32) -> Vec<u8> {
        let mut img = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut img)
            .encode(
                &vec![128u8; (w * h * 3) as usize],
                w,
                h,
                image::ExtendedColorType::Rgb8,
            )
            .expect("encode");
        let mut tiff = std::io::Cursor::new(Vec::new());
        let mut wtr = exif::experimental::Writer::new();
        for f in fields {
            wtr.push_field(f);
        }
        wtr.write(&mut tiff, false).expect("exif");
        let mut out = img[..2].to_vec();
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff.into_inner());
        out.extend([0xFF, 0xE1]);
        out.extend(((app1.len() + 2) as u16).to_be_bytes());
        out.extend(app1);
        if let Some(x) = xmp {
            let mut seg = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
            seg.extend(x.as_bytes());
            out.extend([0xFF, 0xE1]);
            out.extend(((seg.len() + 2) as u16).to_be_bytes());
            out.extend(seg);
        }
        out.extend(&img[2..]);
        out
    }

    fn write_tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("jd-media-{}-{name}", std::process::id()));
        std::fs::File::create(&p)
            .and_then(|mut f| f.write_all(bytes))
            .expect("write");
        p
    }

    fn f(tag: Tag, value: Value) -> Field {
        Field {
            tag,
            ifd_num: In::PRIMARY,
            value,
        }
    }

    #[test]
    fn a_geotagged_photo_gives_place_time_heading_fov_and_sun() {
        // Mount Kenya, 2026-03-20 09:00 UTC (12:00 EAT): the equinox sun stands almost overhead.
        let fields = vec![
            f(Tag::GPSLatitudeRef, Value::Ascii(vec![b"S".to_vec()])),
            f(
                Tag::GPSLatitude,
                Value::Rational(vec![rat(0.0, 1), rat(9.0, 1), rat(3.6, 10)]),
            ),
            f(Tag::GPSLongitudeRef, Value::Ascii(vec![b"E".to_vec()])),
            f(
                Tag::GPSLongitude,
                Value::Rational(vec![rat(37.0, 1), rat(18.0, 1), rat(0.0, 1)]),
            ),
            f(Tag::GPSAltitude, Value::Rational(vec![rat(4985.0, 1)])),
            f(
                Tag::GPSDateStamp,
                Value::Ascii(vec![b"2026:03:20".to_vec()]),
            ),
            f(
                Tag::GPSTimeStamp,
                Value::Rational(vec![rat(9.0, 1), rat(0.0, 1), rat(0.0, 1)]),
            ),
            f(Tag::GPSImgDirectionRef, Value::Ascii(vec![b"T".to_vec()])),
            f(Tag::GPSImgDirection, Value::Rational(vec![rat(292.5, 10)])),
            f(
                Tag::DateTimeOriginal,
                Value::Ascii(vec![b"2026:03:20 12:00:00".to_vec()]),
            ),
            f(
                Tag::OffsetTimeOriginal,
                Value::Ascii(vec![b"+03:00".to_vec()]),
            ),
            f(Tag::Make, Value::Ascii(vec![b"Test".to_vec()])),
            f(Tag::FocalLength, Value::Rational(vec![rat(6.9, 10)])),
            f(Tag::FocalLengthIn35mmFilm, Value::Short(vec![26])),
            f(
                Tag::ExposureTime,
                Value::Rational(vec![Rational {
                    num: 1,
                    denom: 1000,
                }]),
            ),
            f(Tag::FNumber, Value::Rational(vec![rat(1.8, 10)])),
            f(Tag::PhotographicSensitivity, Value::Short(vec![50])),
        ];
        let p = write_tmp("kenya.jpg", &jpeg(&fields, None, 64, 48));
        let m = read_metadata(&p).expect("read");
        std::fs::remove_file(&p).ok();
        assert_eq!(
            (m.kind.as_str(), m.format.as_str(), m.px),
            ("photo", "jpeg", Some([64, 48]))
        );
        let g = m.gps.expect("gps");
        assert!(
            (g.lat + 0.151).abs() < 1e-9 && (g.lon - 37.3).abs() < 1e-9 && g.alt_m == Some(4985.0),
            "{g:?}"
        );
        assert_eq!(m.captured_at_utc.as_deref(), Some("2026-03-20T09:00:00Z"));
        assert_eq!(m.captured_at_source.as_deref(), Some("gps"));
        assert_eq!(
            (m.heading_deg, m.heading_ref.as_deref()),
            (Some(292.5), Some("true"))
        );
        // 26 mm equivalent on a 4:3 frame: half-width 21.635·0.8 = 17.31 mm → 67.3° (not 69.4° by the 36 mm width).
        let fov = m.fov_x_deg.expect("fov");
        assert!(
            (fov - 67.32).abs() < 0.05 && m.fov_source.as_deref() == Some("f35_diag"),
            "{fov}"
        );
        let ev = m.exposure.as_ref().and_then(|e| e.ev100).expect("ev");
        assert!(
            (ev - ((1.8f64 * 1.8 * 1000.0).log2() + 1.0)).abs() < 1e-9,
            "{ev}"
        );
        let sun = m.sun.expect("sun");
        assert!(sun.elevation_deg > 80.0 && !sun.below_horizon, "{sun:?}");
        // The camera clock with its offset gives the same instant when GPS time is absent.
        let no_gps_time: Vec<Field> = fields
            .into_iter()
            .filter(|x| !matches!(x.tag, Tag::GPSDateStamp | Tag::GPSTimeStamp))
            .collect();
        let p = write_tmp("clock.jpg", &jpeg(&no_gps_time, None, 64, 48));
        let m = read_metadata(&p).expect("read");
        std::fs::remove_file(&p).ok();
        assert_eq!(
            (
                m.captured_at_utc.as_deref(),
                m.captured_at_source.as_deref()
            ),
            (Some("2026-03-20T09:00:00Z"), Some("exif_offset"))
        );
    }

    #[test]
    fn a_photo_sphere_gives_its_north_column() {
        let xmp = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:GPano="http://ns.google.com/photos/1.0/panorama/" GPano:ProjectionType="equirectangular" GPano:PoseHeadingDegrees="292.5" GPano:FullPanoWidthPixels="128" GPano:FullPanoHeightPixels="64" GPano:CroppedAreaLeftPixels="0" GPano:CroppedAreaTopPixels="0" GPano:FirstPhotoDate="2016-06-23T09:47:04.992Z"/></rdf:RDF></x:xmpmeta>"#;
        let p = write_tmp(
            "pano.jpg",
            &jpeg(
                &[f(Tag::Make, Value::Ascii(vec![b"RICOH".to_vec()]))],
                Some(xmp),
                128,
                64,
            ),
        );
        let m = read_metadata(&p).expect("read");
        std::fs::remove_file(&p).ok();
        assert_eq!(m.kind, "photo360");
        assert_eq!(
            (
                m.captured_at_utc.as_deref(),
                m.captured_at_source.as_deref()
            ),
            (
                Some("2016-06-23T09:47:04.992Z"),
                Some("gpano_first_photo_date")
            )
        );
        let pr = m.projection.expect("projection");
        assert_eq!(
            (pr.r#type.as_str(), pr.pose_heading_deg, pr.full_px),
            ("equirectangular", Some(292.5), Some([128, 64]))
        );
        // Centre column (64) faces 292.5°; north is 67.5° clockwise of it = 24 px right → column 88.
        assert!((pr.north_column_px.expect("north") - 88.0).abs() < 1e-9);
        assert!(m.fov_x_deg.is_none(), "no flat field of view for a sphere");
        // Element-form XMP is read too.
        assert_eq!(
            xmp_value(
                "<GPano:ProjectionType>equirectangular</GPano:ProjectionType>",
                "GPano:ProjectionType"
            ),
            Some("equirectangular")
        );
    }

    fn bx(ty: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend(ty);
        v.extend(payload);
        v
    }

    #[test]
    fn a_360_video_gives_duration_place_and_projection() {
        // mvhd v0: version/flags, created, modified, timescale 1000, duration 37 300.
        let created = (days_from_civil(2026, 10, 24) + 24_107) as u32 * 86_400 + 12 * 3600; // since 1904
        let mut mvhd = vec![0u8; 4];
        mvhd.extend(created.to_be_bytes());
        mvhd.extend(created.to_be_bytes());
        mvhd.extend(1000u32.to_be_bytes());
        mvhd.extend(37_300u32.to_be_bytes());
        mvhd.extend([0u8; 80]);
        // Visual sample entry: 78 bytes (width 3840, height 1920 at 24..28) + st3d + sv3d.
        let mut entry = vec![0u8; 78];
        entry[24..26].copy_from_slice(&3840u16.to_be_bytes());
        entry[26..28].copy_from_slice(&1920u16.to_be_bytes());
        entry.extend(bx(b"st3d", &[0, 0, 0, 0, 0]));
        let mut prhd = vec![0u8; 4];
        for v in [-90.0f64, 5.0, 0.0] {
            prhd.extend(((v * 65536.0) as i32).to_be_bytes());
        }
        let proj = bx(
            b"proj",
            &[bx(b"prhd", &prhd), bx(b"equi", &[0u8; 20])].concat(),
        );
        entry.extend(bx(b"sv3d", &[bx(b"svhd", &[0u8; 5]), proj].concat()));
        let mut stsd = vec![0u8; 4];
        stsd.extend(1u32.to_be_bytes());
        stsd.extend(bx(b"avc1", &entry));
        let mut hdlr = vec![0u8; 8];
        hdlr.extend(b"vide");
        hdlr.extend([0u8; 13]);
        let stbl = bx(b"stbl", &bx(b"stsd", &stsd));
        let mdia = bx(b"mdia", &[bx(b"hdlr", &hdlr), bx(b"minf", &stbl)].concat());
        let trak = bx(b"trak", &mdia);
        let mut xyz = vec![0, 18, 0x15, 0xc7];
        xyz.extend(b"-01.2921+036.8219/");
        let udta = bx(b"udta", &bx(b"\xa9xyz", &xyz));
        let file = [
            bx(b"ftyp", b"isom\0\0\0\0isom"),
            bx(b"mdat", &[0u8; 64]),
            bx(b"moov", &[bx(b"mvhd", &mvhd), trak, udta].concat()),
        ]
        .concat();
        let p = write_tmp("v.mp4", &file);
        let m = read_metadata(&p).expect("read");
        std::fs::remove_file(&p).ok();
        assert_eq!(
            (m.kind.as_str(), m.format.as_str(), m.px),
            ("video360", "mp4", Some([3840, 1920]))
        );
        assert_eq!(m.duration_s, Some(37.3));
        assert_eq!(m.captured_at_utc.as_deref(), Some("2026-10-24T12:00:00Z"));
        let g = m.gps.expect("gps");
        assert!(
            (g.lat + 1.2921).abs() < 1e-9 && (g.lon - 36.8219).abs() < 1e-9 && g.alt_m.is_none()
        );
        let pr = m.projection.expect("projection");
        // prhd yaw −90° (counter-clockwise) → heading 90° (east).
        assert_eq!(
            (
                pr.r#type.as_str(),
                pr.pose_heading_deg,
                pr.pose_pitch_deg,
                pr.stereo.as_deref()
            ),
            ("equirectangular", Some(90.0), Some(5.0), Some("mono"))
        );
        assert!(m.sun.is_some(), "place + time → sun");
        // ISO 6709 with altitude.
        assert_eq!(
            parse_iso6709("+27.5916+086.5640+8850/"),
            Some(Gps {
                lat: 27.5916,
                lon: 86.564,
                alt_m: Some(8850.0)
            })
        );
    }

    #[test]
    fn unknown_files_are_a_clear_error() {
        let p = write_tmp("x.txt", b"hello world, not an image");
        let e = read_metadata(&p).expect_err("text");
        std::fs::remove_file(&p).ok();
        assert!(e.message.contains("not a photo or video"), "{}", e.message);
    }
}
