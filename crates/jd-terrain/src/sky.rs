//! Physical sky: Preetham, Shirley & Smits 1999, "A practical analytic model for daylight"
//! (`docs/research/algorithms.md`, Sky). The sky's luminance and chromaticity are Perez
//! distributions whose five coefficients are linear in the turbidity, scaled by a zenith value
//! fitted to the sun's zenith angle — the whole model is a page of published formulae, so it is
//! clean-room by construction (ADR-0010). Hošek–Wilkie 2012 is the better model but ships as
//! coefficient tables under their own licence; it is the upgrade if a user asks.
//!
//! Output: linear-sRGB radiance for any direction, and an equirectangular HDRI (OpenEXR, RGB
//! f32) plus a tonemapped PNG preview — VUE's "sky export as HDRI" (`docs/research/
//! vue-features.md`) for engines and renderers. The sun is a disc of 0.27° at `sun_radiance`;
//! below the horizon the horizon colour fades to a fifth of itself within a few degrees.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

/// Sky settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkyParams {
    /// Sun azimuth, degrees clockwise from north.
    #[serde(default = "d_azimuth")]
    pub sun_azimuth_deg: f32,
    /// Sun elevation above the horizon, degrees (0, 90].
    #[serde(default = "d_elevation")]
    pub sun_elevation_deg: f32,
    /// Turbidity: 2 = clear mountain air, 3 = clear, 6 = hazy, 10 = thick haze (2..=10).
    #[serde(default = "d_turbidity")]
    pub turbidity: f32,
    /// Scale from kcd/m² to the written radiance (> 0): 0.05 puts a clear zenith near 0.5.
    #[serde(default = "d_exposure")]
    pub exposure: f32,
    /// Radiance of the 0.27° sun disc after exposure (≥ 0; 0 hides the disc). The default is the
    /// real sun — ≈ 1.6·10⁹ cd/m², i.e. 1.6·10⁶ kcd/m² × the default exposure — so an HDRI lights a
    /// scene with the right sun-to-sky balance; in the map the disc's energy is spread over the
    /// pixel that holds it when the pixel is larger than the disc.
    #[serde(default = "d_sun")]
    pub sun_radiance: f32,
}

fn d_azimuth() -> f32 {
    140.0
}
fn d_elevation() -> f32 {
    35.0
}
fn d_turbidity() -> f32 {
    3.0
}
fn d_exposure() -> f32 {
    0.05
}
fn d_sun() -> f32 {
    80_000.0
}

impl Default for SkyParams {
    fn default() -> Self {
        Self {
            sun_azimuth_deg: d_azimuth(),
            sun_elevation_deg: d_elevation(),
            turbidity: d_turbidity(),
            exposure: d_exposure(),
            sun_radiance: d_sun(),
        }
    }
}

/// Perez distribution coefficients for one channel.
#[derive(Debug, Clone, Copy)]
struct Perez {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
}

impl Perez {
    fn f(&self, cos_theta: f32, gamma: f32) -> f32 {
        let cos_theta = cos_theta.max(0.01);
        (1.0 + self.a * (self.b / cos_theta).exp())
            * (1.0 + self.c * (self.d * gamma).exp() + self.e * gamma.cos().powi(2))
    }
}

/// The evaluated model for one sun position and turbidity.
#[derive(Debug, Clone)]
pub struct Sky {
    params: SkyParams,
    sun: [f32; 3],
    theta_s: f32,
    perez_y: Perez,
    perez_x: Perez,
    perez_yc: Perez,
    zenith: [f32; 3], // Y (kcd/m²), x, y
}

/// Direction of (azimuth clockwise from north, elevation) in the terrain frame: x east, y up, z south.
fn direction(azimuth_deg: f32, elevation_deg: f32) -> [f32; 3] {
    let (az, el) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
    [el.cos() * az.sin(), el.sin(), -el.cos() * az.cos()]
}

impl Sky {
    /// Build the model (validates the parameters).
    pub fn new(p: &SkyParams) -> JdResult<Self> {
        if !(0.0 < p.sun_elevation_deg && p.sun_elevation_deg <= 90.0) {
            return Err(JdError::invalid(
                "sun_elevation_deg",
                "must be within (0, 90]",
            ));
        }
        if !(2.0..=10.0).contains(&p.turbidity) {
            return Err(JdError::invalid("turbidity", "must be within 2..=10"));
        }
        if !(p.exposure > 0.0 && p.exposure.is_finite()) {
            return Err(JdError::invalid("exposure", "must be positive"));
        }
        if !(p.sun_radiance >= 0.0 && p.sun_radiance.is_finite()) || !p.sun_azimuth_deg.is_finite()
        {
            return Err(JdError::invalid("sun_radiance", "must be finite and ≥ 0"));
        }
        let t = p.turbidity;
        let theta_s = (90.0 - p.sun_elevation_deg).to_radians();
        // Zenith luminance (kcd/m²) and chromaticity: Preetham 1999, appendix A.2.
        let chi = (4.0 / 9.0 - t / 120.0) * (std::f32::consts::PI - 2.0 * theta_s);
        let y_z = (4.0453 * t - 4.9710) * chi.tan() - 0.2155 * t + 2.4192;
        let ts = [theta_s.powi(3), theta_s.powi(2), theta_s, 1.0];
        let tt = [t * t, t, 1.0];
        let m_x = [
            [0.00166, -0.00375, 0.00209, 0.0],
            [-0.02903, 0.06377, -0.03202, 0.00394],
            [0.11693, -0.21196, 0.06052, 0.25886],
        ];
        let m_y = [
            [0.00275, -0.00610, 0.00317, 0.0],
            [-0.04214, 0.08970, -0.04153, 0.00516],
            [0.15346, -0.26756, 0.06670, 0.26688],
        ];
        let zen = |m: &[[f32; 4]; 3]| -> f32 {
            (0..3)
                .map(|i| tt[i] * (0..4).map(|j| m[i][j] * ts[j]).sum::<f32>())
                .sum()
        };
        Ok(Self {
            params: p.clone(),
            sun: direction(p.sun_azimuth_deg, p.sun_elevation_deg),
            theta_s,
            perez_y: Perez {
                a: 0.1787 * t - 1.4630,
                b: -0.3554 * t + 0.4275,
                c: -0.0227 * t + 5.3251,
                d: 0.1206 * t - 2.5771,
                e: -0.0670 * t + 0.3703,
            },
            perez_x: Perez {
                a: -0.0193 * t - 0.2592,
                b: -0.0665 * t + 0.0008,
                c: -0.0004 * t + 0.2125,
                d: -0.0641 * t - 0.8989,
                e: -0.0033 * t + 0.0452,
            },
            perez_yc: Perez {
                a: -0.0167 * t - 0.2608,
                b: -0.0950 * t + 0.0092,
                c: -0.0079 * t + 0.2102,
                d: -0.0441 * t - 1.6537,
                e: -0.0109 * t + 0.0529,
            },
            zenith: [y_z.max(0.0), zen(&m_x), zen(&m_y)],
        })
    }

    /// Linear-sRGB radiance (after `exposure`) towards the unit direction `d` (x east, y up, z south).
    pub fn radiance(&self, d: [f32; 3]) -> [f32; 3] {
        let cos_theta = d[1];
        let cos_gamma =
            (d[0] * self.sun[0] + d[1] * self.sun[1] + d[2] * self.sun[2]).clamp(-1.0, 1.0);
        let gamma = cos_gamma.acos();
        // Sun disc: 0.27° radius.
        if self.params.sun_radiance > 0.0 && gamma < 0.27f32.to_radians() && cos_theta > 0.0 {
            return [self.params.sun_radiance; 3];
        }
        let horizon = cos_theta <= 0.0;
        let ct = if horizon { 0.0 } else { cos_theta };
        let cos_ts = self.theta_s.cos();
        let ratio = |p: &Perez| p.f(ct, gamma) / p.f(cos_ts, self.theta_s);
        let big_y = self.zenith[0] * ratio(&self.perez_y);
        let x = self.zenith[1] * ratio(&self.perez_x);
        let y = self.zenith[2] * ratio(&self.perez_yc);
        // xyY → XYZ → linear sRGB (D65).
        let (big_x, big_z) = if y > 1e-6 {
            (x / y * big_y, (1.0 - x - y) / y * big_y)
        } else {
            (0.0, 0.0)
        };
        // Below the horizon: the horizon colour fades to a fifth over the first ~10° (aerial
        // perspective on the ground), so a viewport looking down still sees a pale band at the rim.
        let ground_fade = if horizon {
            0.2 + 0.8 * (8.0 * cos_theta).exp()
        } else {
            1.0
        };
        let scale = self.params.exposure * ground_fade;
        [
            (3.2406 * big_x - 1.5372 * big_y - 0.4986 * big_z).max(0.0) * scale,
            (-0.9689 * big_x + 1.8758 * big_y + 0.0415 * big_z).max(0.0) * scale,
            (0.0557 * big_x - 0.2040 * big_y + 1.0570 * big_z).max(0.0) * scale,
        ]
    }

    /// Equirectangular map (`width` × `width / 2`): row 0 is the zenith, column 0 looks north and
    /// longitude runs clockwise (east at a quarter width), the layout renderers expect.
    pub fn equirect(&self, width: u32) -> (u32, u32, Vec<[f32; 3]>) {
        let height = (width / 2).max(1);
        let mut px = Vec::with_capacity((width * height) as usize);
        for row in 0..height {
            let elevation = 90.0 - (row as f32 + 0.5) / height as f32 * 180.0;
            for col in 0..width {
                let azimuth = (col as f32 + 0.5) / width as f32 * 360.0;
                px.push(self.radiance(direction(azimuth, elevation)));
            }
        }
        // The sun disc is smaller than a pixel at any practical width: give the pixel that holds
        // the sun the disc's energy (radiance × Ω_disc / Ω_pixel) so the HDRI lights a scene right.
        if self.params.sun_radiance > 0.0 {
            let (az, el) = (
                self.params.sun_azimuth_deg.rem_euclid(360.0),
                self.params.sun_elevation_deg,
            );
            let col = ((az / 360.0 * width as f32) as u32).min(width - 1);
            let row = (((90.0 - el) / 180.0 * height as f32) as u32).min(height - 1);
            let omega_disc = std::f32::consts::PI * 0.27f32.to_radians().powi(2);
            let omega_px = (std::f32::consts::TAU / width as f32)
                * (std::f32::consts::PI / height as f32)
                * el.to_radians().cos().max(1e-3);
            let spread = (omega_disc / omega_px).min(1.0);
            px[(row * width + col) as usize] = [self.params.sun_radiance * spread; 3];
        }
        (width, height, px)
    }
}

/// Write the sky as an OpenEXR HDRI (RGB f32, equirectangular) and return `(width, height, zenith_rgb, horizon_rgb)`.
pub fn bake_sky_hdri(
    p: &SkyParams,
    width: u32,
    path: &std::path::Path,
) -> JdResult<(u32, u32, [f32; 3], [f32; 3])> {
    if !(16..=8192).contains(&width) || !width.is_multiple_of(2) {
        return Err(JdError::invalid(
            "width",
            "must be even and within 16..=8192",
        ));
    }
    let sky = Sky::new(p)?;
    let (w, h, px) = sky.equirect(width);
    use exr::prelude::*;
    write_rgb_file(path, w as usize, h as usize, |x, y| {
        let c = px[y * w as usize + x];
        (c[0], c[1], c[2])
    })
    .map_err(|e| {
        JdError::new(
            jd_types::ErrorCode::E_INTERNAL,
            format!("EXR write failed: {e}"),
        )
    })?;
    Ok((
        w,
        h,
        sky.radiance([0.0, 1.0, 0.0]),
        sky.radiance(direction(0.0, 0.5)),
    ))
}

/// Tonemapped 8-bit preview of the equirectangular sky (Reinhard, sRGB) as PNG bytes.
pub fn sky_preview_png(p: &SkyParams, width: u32) -> JdResult<Vec<u8>> {
    let sky = Sky::new(p)?;
    let (w, h, px) = sky.equirect(width);
    let mut img = image::RgbImage::new(w, h);
    for (i, c) in px.iter().enumerate() {
        let enc =
            |v: f32| (((v / (1.0 + v)).clamp(0.0, 1.0)).powf(1.0 / 2.2) * 255.0).round() as u8;
        img.put_pixel(
            i as u32 % w,
            i as u32 / w,
            image::Rgb([enc(c[0]), enc(c[1]), enc(c[2])]),
        );
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| {
            JdError::new(
                jd_types::ErrorCode::E_INTERNAL,
                format!("PNG encode failed: {e}"),
            )
        })?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_sky_is_blue_at_the_zenith_bright_around_the_sun_and_hazier_with_turbidity() {
        let clear = Sky::new(&SkyParams::default()).expect("sky");
        let zenith = clear.radiance([0.0, 1.0, 0.0]);
        assert!(
            zenith[2] > zenith[0] && zenith[2] > zenith[1],
            "blue zenith: {zenith:?}"
        );
        assert!(
            zenith.iter().all(|c| (0.05..5.0).contains(c)),
            "sane exposure: {zenith:?}"
        );
        // The aureole: 5° from the sun is brighter than the same elevation 90° away in azimuth.
        let near = clear.radiance(direction(140.0, 40.0));
        let far = clear.radiance(direction(230.0, 40.0));
        assert!(near[1] > far[1] * 1.3, "aureole {near:?} vs {far:?}");
        // The disc itself is the brightest thing in the sky; hiding it removes it.
        let sun = clear.radiance(direction(140.0, 35.0));
        assert_eq!(sun, [SkyParams::default().sun_radiance; 3]);
        // In the map the disc's energy lands in one pixel, scaled by its share of that pixel.
        let (w, h, px) = clear.equirect(256);
        let brightest = px.iter().map(|c| c[1]).fold(0.0f32, f32::max);
        assert!(
            brightest > 50.0 && brightest < SkyParams::default().sun_radiance,
            "sun pixel {brightest} in a {w}×{h} map"
        );
        let no_disc = Sky::new(&SkyParams {
            sun_radiance: 0.0,
            ..SkyParams::default()
        })
        .expect("sky");
        assert!(no_disc.radiance(direction(140.0, 35.0))[1] < 50.0);
        // Haze whitens the sky: the zenith's blue-to-red ratio drops with turbidity, and the sky is
        // more uniform (the zenith catches up with the far horizon).
        let hazy = Sky::new(&SkyParams {
            turbidity: 8.0,
            ..SkyParams::default()
        })
        .expect("sky");
        let blueness = |s: &Sky| {
            let z = s.radiance([0.0, 1.0, 0.0]);
            z[2] / z[0].max(1e-6)
        };
        assert!(
            blueness(&hazy) < blueness(&clear),
            "hazy {} vs clear {}",
            blueness(&hazy),
            blueness(&clear)
        );
        let contrast = |s: &Sky| {
            (s.radiance(direction(320.0, 2.0))[1] / s.radiance([0.0, 1.0, 0.0])[1] - 1.0).abs()
        };
        assert!(
            contrast(&hazy) < contrast(&clear) + 0.6,
            "hazy {} vs clear {}",
            contrast(&hazy),
            contrast(&clear)
        );
        // Below the horizon: a dim flat ground.
        let ground = clear.radiance([0.0, -0.5, 0.5]);
        assert!(ground[1] < clear.radiance(direction(0.0, 0.5))[1]);
        // The HDRI round-trips through OpenEXR with the right layout: the top row is the zenith.
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("sky.exr");
        let (w, h, z, _) = bake_sky_hdri(&SkyParams::default(), 64, &path).expect("exr");
        assert_eq!((w, h), (64, 32));
        let img = exr::prelude::read_first_rgba_layer_from_file(
            &path,
            |size, _| vec![[0.0f32; 3]; size.width() * size.height()],
            |px, pos, (r, g, b, _): (f32, f32, f32, f32)| px[pos.y() * 64 + pos.x()] = [r, g, b],
        )
        .expect("read");
        let top = img.layer_data.channel_data.pixels[0];
        assert!(
            (top[2] - z[2]).abs() < 0.05 * z[2].max(0.01),
            "zenith {top:?} vs {z:?}"
        );
        assert!(
            sky_preview_png(&SkyParams::default(), 64)
                .expect("png")
                .len()
                > 100
        );
        assert!(
            Sky::new(&SkyParams {
                sun_elevation_deg: 0.0,
                ..SkyParams::default()
            })
            .is_err()
        );
        assert!(
            Sky::new(&SkyParams {
                turbidity: 1.0,
                ..SkyParams::default()
            })
            .is_err()
        );
        assert!(bake_sky_hdri(&SkyParams::default(), 15, &path).is_err());
    }
}
