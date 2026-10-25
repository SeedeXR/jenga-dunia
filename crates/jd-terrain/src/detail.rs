//! Tileable detail maps for a material layer: a seeded, periodic value-noise relief baked into
//! an albedo tile (the layer's colour with its variation) and a tangent-space normal tile, both
//! wrapping seamlessly so an engine can repeat them every `tile_m` metres. The same relief is
//! what the macro maps (`bake_maps_with`, the GLB) carry, sampled by world position, so the
//! viewport, the exports and the engines agree on where the grain lies.
//!
//! ponytail: one procedural look (fractal value noise on a wrapped lattice); texture-library
//! swatches plug in later by replacing the tile, not the plumbing.

use jd_types::{JdError, JdResult};

/// The detail of one material layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detail {
    /// Metres per repeat of the tile.
    pub tile_m: f32,
    /// Normal-map strength 0..1 (0 = flat).
    pub bump: f32,
    /// Albedo variation 0..1 (0 = the flat colour).
    pub contrast: f32,
    /// Noise seed.
    pub seed: u64,
    /// Tile side in pixels (power of two is kindest to engines).
    pub px: u32,
}

impl Default for Detail {
    fn default() -> Self {
        Detail {
            tile_m: 8.0,
            bump: 0.5,
            contrast: 0.3,
            seed: 0,
            px: 512,
        }
    }
}

impl Detail {
    /// Range checks, naming layer `k` in the error.
    pub fn validate(&self, k: usize) -> JdResult<()> {
        if !(self.tile_m.is_finite() && self.tile_m > 0.0) {
            return Err(JdError::invalid(
                "materials",
                format!("layer {k}: detail.tile_m must be positive metres"),
            ));
        }
        if !(0.0..=1.0).contains(&self.bump) || !(0.0..=1.0).contains(&self.contrast) {
            return Err(JdError::invalid(
                "materials",
                format!("layer {k}: detail.bump and detail.contrast must be within 0..=1"),
            ));
        }
        if !(16..=2048).contains(&self.px) {
            return Err(JdError::invalid(
                "materials",
                format!("layer {k}: detail.px must be within 16..=2048"),
            ));
        }
        Ok(())
    }
}

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Lattice value in 0..1 at integer cell (i, j) of a `period`-cell torus.
fn lattice(seed: u64, period: u32, i: i64, j: i64) -> f32 {
    let p = period as i64;
    let (i, j) = (i.rem_euclid(p) as u64, j.rem_euclid(p) as u64);
    let h = mix(seed ^ mix(i.wrapping_mul(73_856_093) ^ j.wrapping_mul(19_349_663)));
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Periodic value noise in 0..1 at (u, v) in tile units (one tile = `period` cells).
fn periodic_noise(seed: u64, period: u32, u: f32, v: f32) -> f32 {
    let (x, y) = (u * period as f32, v * period as f32);
    let (x0, y0) = (x.floor(), y.floor());
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (ease(x - x0), ease(y - y0));
    let (i, j) = (x0 as i64, y0 as i64);
    let l = |di: i64, dj: i64| lattice(seed, period, i + di, j + dj);
    let a = l(0, 0) + (l(1, 0) - l(0, 0)) * tx;
    let b = l(0, 1) + (l(1, 1) - l(0, 1)) * tx;
    a + (b - a) * ty
}

/// The tile's relief in 0..1 at (u, v) in tile units: four octaves, periods 8, 16, 32, 64 cells
/// per tile, so every octave wraps and the tile is seamless by construction.
pub fn relief(seed: u64, u: f32, v: f32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut total = 0.0;
    for (k, period) in [8u32, 16, 32, 64].iter().enumerate() {
        sum += amp
            * periodic_noise(
                seed.wrapping_add(k as u64 * 0x9E37),
                *period,
                u.rem_euclid(1.0),
                v.rem_euclid(1.0),
            );
        total += amp;
        amp *= 0.5;
    }
    sum / total
}

/// A baked tile: PNG bytes of the albedo (sRGB) and the tangent-space normal, `px` square.
#[derive(Debug, Clone)]
pub struct DetailTile {
    /// The colour tile, sRGB PNG.
    pub albedo: Vec<u8>,
    /// The tangent-space normal tile, PNG.
    pub normal: Vec<u8>,
    /// Tile side, pixels.
    pub px: u32,
    /// Metres per repeat.
    pub tile_m: f32,
}

/// Bake the albedo and normal tiles for `d` in the layer's colour (sRGB 0..1).
pub fn bake_detail(d: &Detail, albedo: [f32; 3]) -> JdResult<DetailTile> {
    d.validate(0)?;
    let n = d.px;
    let mut alb = Vec::with_capacity((n * n * 3) as usize);
    let mut nrm = Vec::with_capacity((n * n * 3) as usize);
    let step = 1.0 / n as f32;
    for j in 0..n {
        for i in 0..n {
            let (u, v) = ((i as f32 + 0.5) * step, (j as f32 + 0.5) * step);
            let r = relief(d.seed, u, v);
            // Albedo: the colour, darkened and lightened by the relief around its mean.
            let f = 1.0 + d.contrast * (r - 0.5) * 2.0;
            for c in albedo {
                alb.push(((c * f).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            // Normal: finite differences on the wrapped tile; `bump` scales the slope.
            let dx = (relief(d.seed, u + step, v) - relief(d.seed, u - step, v)) / (2.0 * step);
            let dy = (relief(d.seed, u, v + step) - relief(d.seed, u, v - step)) / (2.0 * step);
            let s = d.bump * 0.25;
            let (nx, ny, nz) = (-dx * s, -dy * s, 1.0f32);
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            for c in [nx / len, ny / len, nz / len] {
                nrm.push(((c * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    Ok(DetailTile {
        albedo: png(&alb, n)?,
        normal: png(&nrm, n)?,
        px: n,
        tile_m: d.tile_m,
    })
}

fn png(rgb: &[u8], n: u32) -> JdResult<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut out,
        rgb,
        n,
        n,
        image::ExtendedColorType::Rgb8,
        image::ImageFormat::Png,
    )
    .map_err(|e| {
        JdError::new(
            jd_types::ErrorCode::E_INTERNAL,
            format!("PNG encode failed: {e}"),
        )
    })?;
    Ok(out.into_inner())
}

/// An image texture set for a material layer: a tileable albedo PNG (sRGB) and an optional
/// tangent-space normal PNG, repeated every `tile_m` metres — World Creator's texture sets. It
/// feeds the same [`DetailTile`] path as the procedural grain, so the viewport, the GLB bake
/// and the engine profiles need nothing new.
#[derive(Debug, Clone)]
pub struct TextureSet {
    /// Albedo, sRGB.
    pub albedo: image::RgbImage,
    /// Tangent-space normal (+Y up), or None for a flat tile.
    pub normal: Option<image::RgbImage>,
    /// Metres per repeat.
    pub tile_m: f32,
    /// Exported tile side in pixels (the images are resampled to it).
    pub px: u32,
}

impl TextureSet {
    /// Load the PNGs (`normal` optional) and validate.
    pub fn load(
        albedo: &std::path::Path,
        normal: Option<&std::path::Path>,
        tile_m: f32,
        px: u32,
    ) -> JdResult<Self> {
        let read = |path: &std::path::Path| -> JdResult<image::RgbImage> {
            image::open(path).map(|img| img.into_rgb8()).map_err(|e| {
                JdError::new(jd_types::ErrorCode::E_FORMAT_CORRUPT, format!("texture {}: {e}", path.display()))
                    .next("Texture sets are PNG files in the project's assets/ (import them with project.import_asset).")
            })
        };
        let set = Self {
            albedo: read(albedo)?,
            normal: normal.map(read).transpose()?,
            tile_m,
            px,
        };
        set.validate()?;
        Ok(set)
    }

    /// Positive repeat, a usable tile size, non-empty images.
    pub fn validate(&self) -> JdResult<()> {
        if !(self.tile_m > 0.0 && self.tile_m.is_finite()) {
            return Err(JdError::invalid("texture.tile_m", "must be positive"));
        }
        if !(16..=4096).contains(&self.px) {
            return Err(JdError::invalid("texture.px", "must be within 16..=4096"));
        }
        if self.albedo.width() == 0
            || self.albedo.height() == 0
            || self
                .normal
                .as_ref()
                .is_some_and(|n| n.width() == 0 || n.height() == 0)
        {
            return Err(JdError::invalid("texture", "images must not be empty"));
        }
        Ok(())
    }

    /// The albedo texel under a world position (nearest, wrapping every `tile_m`), sRGB 0..1.
    pub fn albedo_at(&self, x_m: f64, z_m: f64) -> [f32; 3] {
        let (w, h) = (self.albedo.width(), self.albedo.height());
        let u = (x_m / f64::from(self.tile_m)).rem_euclid(1.0);
        let v = (z_m / f64::from(self.tile_m)).rem_euclid(1.0);
        let px = self.albedo.get_pixel(
            ((u * f64::from(w)) as u32).min(w - 1),
            ((v * f64::from(h)) as u32).min(h - 1),
        );
        [
            f32::from(px[0]) / 255.0,
            f32::from(px[1]) / 255.0,
            f32::from(px[2]) / 255.0,
        ]
    }

    /// The set as an exportable tile at `px` (a flat normal when none was given).
    pub fn tile(&self) -> JdResult<DetailTile> {
        self.validate()?;
        let n = self.px;
        let alb =
            image::imageops::resize(&self.albedo, n, n, image::imageops::FilterType::Triangle);
        let nrm = match &self.normal {
            Some(img) => {
                image::imageops::resize(img, n, n, image::imageops::FilterType::Triangle).into_raw()
            }
            None => [128u8, 128, 255].repeat((n * n) as usize),
        };
        Ok(DetailTile {
            albedo: png(alb.as_raw(), n)?,
            normal: png(&nrm, n)?,
            px: n,
            tile_m: self.tile_m,
        })
    }
}

/// The relief's slope at a world position, in tile units per tile, for the macro bakes:
/// `(dr/du, dr/dv)` and the relief itself.
pub fn relief_at_world(d: &Detail, x_m: f64, z_m: f64) -> (f32, f32, f32) {
    let (u, v) = (
        (x_m / d.tile_m as f64) as f32,
        (z_m / d.tile_m as f64) as f32,
    );
    let e = 1.0 / 256.0;
    let r = relief(d.seed, u, v);
    let du = (relief(d.seed, u + e, v) - relief(d.seed, u - e, v)) / (2.0 * e);
    let dv = (relief(d.seed, u, v + e) - relief(d.seed, u, v - e)) / (2.0 * e);
    (r, du, dv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_texture_set_samples_by_world_position_and_exports_a_tile() {
        // Two columns: red on the west half of each repeat, blue on the east.
        let albedo = image::RgbImage::from_fn(2, 2, |x, _| {
            if x == 0 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let set = TextureSet {
            albedo,
            normal: None,
            tile_m: 10.0,
            px: 16,
        };
        assert_eq!(set.albedo_at(2.0, 3.0), [1.0, 0.0, 0.0]);
        assert_eq!(set.albedo_at(7.0, 3.0), [0.0, 0.0, 1.0]);
        assert_eq!(
            set.albedo_at(12.0, 3.0),
            [1.0, 0.0, 0.0],
            "wraps every tile_m"
        );
        let tile = set.tile().expect("tile");
        assert_eq!(tile.px, 16);
        let img = image::load_from_memory(&tile.albedo)
            .expect("png")
            .into_rgb8();
        assert_eq!((img.width(), img.height()), (16, 16));
        let nrm = image::load_from_memory(&tile.normal)
            .expect("png")
            .into_rgb8();
        assert_eq!(
            nrm.get_pixel(3, 3).0,
            [128, 128, 255],
            "flat normal when none is given"
        );
        assert!(
            TextureSet {
                px: 8,
                ..set.clone()
            }
            .tile()
            .is_err()
        );
    }

    #[test]
    fn the_tile_wraps_and_the_relief_varies() {
        let d = Detail {
            px: 64,
            ..Default::default()
        };
        let t = bake_detail(&d, [0.5, 0.4, 0.3]).unwrap();
        assert_eq!((t.px, t.tile_m), (64, 8.0));
        let img = image::load_from_memory(&t.albedo).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (64, 64));
        // Seamless: the relief at u and u+1 is identical, and the first and last columns are
        // neighbours (their difference is no larger than any interior neighbour pair's).
        for k in 0..16 {
            let v = k as f32 / 16.0;
            assert!((relief(7, 0.3, v) - relief(7, 1.3, v)).abs() < 1e-6);
        }
        let col = |x: u32| {
            (0..64)
                .map(|y| img.get_pixel(x, y).0[0] as i32)
                .collect::<Vec<_>>()
        };
        let seam: i32 = col(0)
            .iter()
            .zip(col(63))
            .map(|(a, b)| (a - b).abs())
            .max()
            .unwrap();
        let inner: i32 = col(31)
            .iter()
            .zip(col(32))
            .map(|(a, b)| (a - b).abs())
            .max()
            .unwrap();
        assert!(seam <= inner.max(4), "seam {seam} vs interior {inner}");
        let (lo, hi) = img
            .pixels()
            .fold((255u8, 0u8), |(lo, hi), p| (lo.min(p.0[0]), hi.max(p.0[0])));
        assert!(hi - lo > 20, "the albedo varies: {lo}..{hi}");
        // The normal tile points mostly out of the surface and is not flat.
        let nm = image::load_from_memory(&t.normal).unwrap().to_rgb8();
        let (zmin, xs) = nm.pixels().fold((255u8, 0u32), |(z, x), p| {
            (z.min(p.0[2]), x + (p.0[0] != 128) as u32)
        });
        assert!(zmin > 200 && xs > 1000, "z {zmin}, varied x {xs}");
        assert!(Detail { tile_m: 0.0, ..d }.validate(0).is_err());
        assert!(bake_detail(&Detail { bump: 2.0, ..d }, [0.0; 3]).is_err());
        // Determinism.
        assert_eq!(bake_detail(&d, [0.5, 0.4, 0.3]).unwrap().albedo, t.albedo);
    }
}
