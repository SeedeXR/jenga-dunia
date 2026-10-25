//! Deterministic noise generators (`docs/research/algorithms.md` §1).
//!
//! Backed by `fastnoise-lite` (MIT): OpenSimplex2/2S, Perlin, Value, ValueCubic,
//! Cellular with fBm / ridged / ping-pong fractals and domain warp. Output is a
//! pure function of `(seed, params, world x, world y)`, so tiling and thread
//! count cannot change results (ADR-0007). Coordinates are in **metres** so a
//! parameter means the same thing at every resolution (lesson from Gaea/WM:
//! `docs/research/gaea-world-machine.md` D1.6).

use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};
use jd_types::{JdError, JdResult, WorldSettings};
use serde::{Deserialize, Serialize};

use crate::heightfield::Heightfield;

/// Base noise lattice.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum NoiseKind {
    /// OpenSimplex2 (fast, isotropic). Default.
    #[default]
    OpenSimplex2,
    /// OpenSimplex2S (smoother).
    OpenSimplex2S,
    /// Improved Perlin.
    Perlin,
    /// Value noise.
    Value,
    /// Cubic value noise.
    ValueCubic,
    /// Cellular / Worley (F1 distance).
    Cellular,
}

/// Fractal combinator.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum FractalKind {
    /// Single octave.
    None,
    /// Fractal Brownian motion. Default.
    #[default]
    Fbm,
    /// Ridged multifractal (mountain ridges).
    Ridged,
    /// Ping-pong (terraced/billowy).
    PingPong,
}

/// Parameters of a noise heightfield generator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoiseParams {
    /// Lattice.
    #[serde(default)]
    pub kind: NoiseKind,
    /// Fractal combinator.
    #[serde(default)]
    pub fractal: FractalKind,
    /// Seed (combined with the world seed by the caller when desired).
    #[serde(default)]
    pub seed: i32,
    /// Feature size in metres (wavelength of the first octave).
    #[serde(default = "default_feature_size_m")]
    pub feature_size_m: f32,
    /// Octave count (1–16).
    #[serde(default = "default_octaves")]
    pub octaves: u8,
    /// Frequency multiplier per octave.
    #[serde(default = "default_lacunarity")]
    pub lacunarity: f32,
    /// Amplitude multiplier per octave.
    #[serde(default = "default_gain")]
    pub gain: f32,
    /// Output height range in metres: noise ∈ [-1,1] maps to [min, max].
    #[serde(default = "default_height_range")]
    pub height_range_m: [f32; 2],
    /// Domain warp: the sampling coordinates are bent by a second noise before the lattice is
    /// read, which turns straight ridges and round cells into the swirled, folded shapes of
    /// real relief (World Creator's distortion filter, Gaea's warp). Unset = none.
    #[serde(default)]
    pub warp: Option<DomainWarp>,
    /// Offset of the sampling origin in metres.
    #[serde(default)]
    pub offset_m: [f32; 2],
}

fn default_feature_size_m() -> f32 {
    1024.0
}
fn default_octaves() -> u8 {
    6
}
fn default_lacunarity() -> f32 {
    2.0
}
fn default_gain() -> f32 {
    0.5
}
fn default_height_range() -> [f32; 2] {
    [0.0, 500.0]
}

impl Default for NoiseParams {
    fn default() -> Self {
        Self {
            kind: NoiseKind::OpenSimplex2,
            fractal: FractalKind::Fbm,
            seed: 0,
            feature_size_m: default_feature_size_m(),
            octaves: default_octaves(),
            lacunarity: default_lacunarity(),
            gain: default_gain(),
            height_range_m: default_height_range(),
            warp: None,
            offset_m: [0.0, 0.0],
        }
    }
}

impl NoiseParams {
    /// Validate ranges before any work is done.
    pub fn validate(&self) -> JdResult<()> {
        if self.feature_size_m <= 0.0 || !self.feature_size_m.is_finite() {
            return Err(JdError::invalid(
                "feature_size_m",
                "must be a finite positive number of metres",
            ));
        }
        if !(1..=16).contains(&self.octaves) {
            return Err(JdError::invalid("octaves", "must be within 1..=16"));
        }
        if self.lacunarity <= 0.0
            || !self.lacunarity.is_finite()
            || self.gain < 0.0
            || !self.gain.is_finite()
        {
            return Err(JdError::invalid(
                "lacunarity|gain",
                "must be finite and positive",
            ));
        }
        if self.height_range_m.iter().any(|v| !v.is_finite())
            || self.height_range_m[0] >= self.height_range_m[1]
        {
            return Err(JdError::invalid(
                "height_range_m",
                "must be finite with min < max",
            ));
        }
        Ok(())
    }

    fn build(&self) -> FastNoiseLite {
        let mut n = FastNoiseLite::with_seed(self.seed);
        n.set_noise_type(Some(match self.kind {
            NoiseKind::OpenSimplex2 => NoiseType::OpenSimplex2,
            NoiseKind::OpenSimplex2S => NoiseType::OpenSimplex2S,
            NoiseKind::Perlin => NoiseType::Perlin,
            NoiseKind::Value => NoiseType::Value,
            NoiseKind::ValueCubic => NoiseType::ValueCubic,
            NoiseKind::Cellular => NoiseType::Cellular,
        }));
        n.set_fractal_type(Some(match self.fractal {
            FractalKind::None => FractalType::None,
            FractalKind::Fbm => FractalType::FBm,
            FractalKind::Ridged => FractalType::Ridged,
            FractalKind::PingPong => FractalType::PingPong,
        }));
        n.set_frequency(Some(1.0 / self.feature_size_m));
        n.set_fractal_octaves(Some(self.octaves as i32));
        n.set_fractal_lacunarity(Some(self.lacunarity));
        n.set_fractal_gain(Some(self.gain));
        n
    }
}

/// Domain warp settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DomainWarp {
    /// How far a sample point is displaced, metres (> 0).
    pub amp_m: f32,
    /// Wavelength of the displacement field, metres (> 0); larger folds the terrain in broader sweeps.
    pub feature_m: f32,
    /// Octaves of the displacement field (1..=8): more adds finer wrinkles to the folds.
    #[serde(default = "warp_octaves")]
    pub octaves: u8,
}
fn warp_octaves() -> u8 {
    3
}

impl DomainWarp {
    fn validate(&self) -> Result<(), String> {
        if !(self.amp_m > 0.0 && self.amp_m.is_finite())
            || !(self.feature_m > 0.0 && self.feature_m.is_finite())
        {
            return Err("warp.amp_m and warp.feature_m must be positive".into());
        }
        if !(1..=8).contains(&self.octaves) {
            return Err("warp.octaves must be within 1..=8".into());
        }
        Ok(())
    }
    /// The displacement field: progressive OpenSimplex2 domain warp of `amp_m`, seeded apart.
    fn build(&self, seed: i32) -> FastNoiseLite {
        let mut n = FastNoiseLite::with_seed(seed.wrapping_add(7919));
        n.set_domain_warp_type(Some(fastnoise_lite::DomainWarpType::OpenSimplex2));
        n.set_domain_warp_amp(Some(self.amp_m));
        n.set_frequency(Some(1.0 / self.feature_m));
        n.set_fractal_type(Some(FractalType::DomainWarpProgressive));
        n.set_fractal_octaves(Some(i32::from(self.octaves)));
        n
    }
}

/// Generate a heightfield for `world` from `params`. Sampling is in world metres:
/// pixel (x, y) → (origin + x·spacing, origin + y·spacing).
pub fn generate_noise(world: &WorldSettings, params: &NoiseParams) -> JdResult<Heightfield> {
    params.validate()?;
    if let Some(w) = &params.warp {
        w.validate().map_err(|m| JdError::invalid("warp", m))?;
    }
    world.validate().map_err(|m| JdError::invalid("world", m))?;
    let noise = params.build();
    let warp = params.warp.map(|w| w.build(params.seed));
    let [sx, sy] = world.spacing_m();
    let [ox, oy] = [
        world.origin[0] + params.offset_m[0] as f64,
        world.origin[1] + params.offset_m[1] as f64,
    ];
    let [lo, hi] = params.height_range_m;
    let half = 0.5 * (hi - lo);
    let mid = lo + half;
    let mut hf = Heightfield::from_world(world)?;
    hf.fill_with(|x, y| {
        let (mut wx, mut wy) = ((ox + x as f64 * sx) as f32, (oy + y as f64 * sy) as f32);
        if let Some(w) = &warp {
            (wx, wy) = w.domain_warp_2d(wx, wy);
        }
        let n = noise.get_noise_2d(wx, wy); // ∈ [-1, 1]
        mid + n * half
    });
    hf.validate_finite()?;
    Ok(hf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn small_world(res: u32, tile: u32) -> WorldSettings {
        WorldSettings {
            extent_m: [1024.0, 1024.0],
            resolution_px: [res, res],
            tile_size: tile,
            halo: 2,
            ..Default::default()
        }
    }

    #[test]
    fn domain_warp_bends_the_field_and_a_bad_warp_is_refused() {
        let w = small_world(128, 64);
        let plain = generate_noise(
            &w,
            &NoiseParams {
                seed: 5,
                ..Default::default()
            },
        )
        .expect("plain");
        let warped = generate_noise(
            &w,
            &NoiseParams {
                seed: 5,
                warp: Some(DomainWarp {
                    amp_m: 120.0,
                    feature_m: 400.0,
                    octaves: 3,
                }),
                ..Default::default()
            },
        )
        .expect("warped");
        assert_ne!(
            plain.content_hash(),
            warped.content_hash(),
            "the warp moves the samples"
        );
        let (ps, ws) = (plain.stats(), warped.stats());
        assert!(
            (ps.min - ws.min).abs() < 60.0 && (ps.max - ws.max).abs() < 60.0,
            "same height range: {ps:?} vs {ws:?}"
        );
        assert!(
            generate_noise(
                &w,
                &NoiseParams {
                    warp: Some(DomainWarp {
                        amp_m: 0.0,
                        feature_m: 400.0,
                        octaves: 3
                    }),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn same_seed_same_field_different_seed_different_field() {
        let w = small_world(128, 64);
        let p = NoiseParams {
            seed: 3,
            ..Default::default()
        };
        let a = generate_noise(&w, &p).expect("a");
        let b = generate_noise(&w, &p).expect("b");
        assert_eq!(a.content_hash(), b.content_hash());
        let c = generate_noise(&w, &NoiseParams { seed: 4, ..p }).expect("c");
        assert_ne!(a.content_hash(), c.content_hash());
    }

    #[test]
    fn tiling_and_thread_count_do_not_change_samples() {
        let p = NoiseParams {
            seed: 11,
            fractal: FractalKind::Ridged,
            ..Default::default()
        };
        let a = generate_noise(&small_world(200, 64), &p).expect("a");
        let b = generate_noise(&small_world(200, 128), &p).expect("b");
        assert_eq!(a.to_vec(), b.to_vec(), "tile size must not change results");
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("pool");
        let c = pool.install(|| generate_noise(&small_world(200, 64), &p).expect("c"));
        assert_eq!(
            a.to_vec(),
            c.to_vec(),
            "thread count must not change results"
        );
    }

    #[test]
    fn output_respects_height_range_and_is_finite() {
        let p = NoiseParams {
            height_range_m: [100.0, 300.0],
            ..Default::default()
        };
        let hf = generate_noise(&small_world(256, 128), &p).expect("hf");
        let s = hf.stats();
        assert_eq!(s.non_finite, 0);
        assert!(s.min >= 100.0 - 1e-3 && s.max <= 300.0 + 1e-3, "{s:?}");
        assert!(s.max - s.min > 20.0, "terrain should not be flat: {s:?}");
    }

    #[test]
    fn invalid_params_are_rejected_before_work() {
        assert!(
            NoiseParams {
                octaves: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            NoiseParams {
                feature_size_m: 0.0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            NoiseParams {
                height_range_m: [10.0, 10.0],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            NoiseParams {
                gain: f32::NAN,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(16))]
        #[test]
        fn any_valid_params_produce_finite_bounded_output(
            seed in any::<i32>(), octaves in 1u8..=8, gain in 0.1f32..0.9, feature in 50f32..4000.0,
            kind_idx in 0usize..6, fractal_idx in 0usize..4
        ) {
            let kinds = [NoiseKind::OpenSimplex2, NoiseKind::OpenSimplex2S, NoiseKind::Perlin, NoiseKind::Value, NoiseKind::ValueCubic, NoiseKind::Cellular];
            let fractals = [FractalKind::None, FractalKind::Fbm, FractalKind::Ridged, FractalKind::PingPong];
            let p = NoiseParams { seed, octaves, gain, feature_size_m: feature, kind: kinds[kind_idx], fractal: fractals[fractal_idx], ..Default::default() };
            let hf = generate_noise(&small_world(64, 64), &p).expect("generate");
            let s = hf.stats();
            prop_assert_eq!(s.non_finite, 0);
            prop_assert!(s.min >= -1e-3 && s.max <= 500.0 + 1e-3, "{:?}", s);
        }
    }
}
