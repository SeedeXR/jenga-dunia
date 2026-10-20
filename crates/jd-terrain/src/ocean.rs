//! Wind-sea spectrum: what waves a wind of a given speed raises over a given fetch, as the numbers
//! a Gerstner surface needs (`docs/research/algorithms.md`, "Water surface": Gerstner in the
//! viewport, FFT ocean later).
//!
//! Deep-water, fetch-limited growth after JONSWAP (Hasselmann et al. 1973) in its U10 form, capped
//! at the fully developed Pierson–Moskowitz (1964) sea: with χ = gF/U² the significant height is
//! Hs = 1.6·10⁻³ √χ U²/g and the peak period Tp = 0.2857 χ^⅓ U/g; a fully developed sea reaches
//! Hs ≈ 0.0246 U² and Tp ≈ 0.78 U (U in m/s, U10 ≈ U19.5/1.07). Wavelength from the deep-water
//! dispersion relation L = g Tp²/2π. These are the textbook forecasting curves (SPM 1984 without
//! its wind-stress factor, which over-predicts strong winds); a 20 m/s wind over 100 km gives
//! Hs 3.2 m / Tp 7.9 s, fully developed 9.8 m / 15.6 s.
//!
//! ponytail: three Gerstner components (peak, half, fifth wavelength) spread ±30° about the wind;
//! a seeded directional spectrum with dozens of components is the upgrade when the surface needs
//! to look less regular.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

const G: f32 = 9.81;

/// Wind-sea settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaveParams {
    /// Wind speed 10 m above the sea, m/s (≥ 0; 0 = flat calm): 3 ripples, 8 moderate sea, 20 gale.
    #[serde(default = "d_wind")]
    pub wind_speed_mps: f32,
    /// Fetch, km: how far the wind has blown over open water (> 0). Omit for a fully developed sea.
    #[serde(default)]
    pub fetch_km: Option<f32>,
    /// Where the wind blows FROM, degrees clockwise from north; waves travel the opposite way.
    #[serde(default = "d_azimuth")]
    pub wind_azimuth_deg: f32,
}

fn d_wind() -> f32 {
    8.0
}
fn d_azimuth() -> f32 {
    270.0
}

impl Default for WaveParams {
    fn default() -> Self {
        Self {
            wind_speed_mps: d_wind(),
            fetch_km: None,
            wind_azimuth_deg: d_azimuth(),
        }
    }
}

/// One trochoidal (Gerstner) wave.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WaveComponent {
    /// Crest height above the mean surface, m.
    pub amplitude_m: f32,
    /// Crest-to-crest distance, m.
    pub wavelength_m: f32,
    /// Travel direction, degrees clockwise from north.
    pub direction_deg: f32,
    /// Deep-water phase speed √(g L / 2π), m/s.
    pub speed_mps: f32,
    /// Phase offset, radians.
    pub phase: f32,
}

/// The wind sea: bulk statistics plus the Gerstner components that draw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WaveSpectrum {
    /// Significant wave height Hs (mean of the highest third), m.
    pub significant_height_m: f32,
    /// Peak period Tp, s.
    pub peak_period_s: f32,
    /// Peak wavelength, m.
    pub wavelength_m: f32,
    /// Whether the fetch was long enough for the sea to be fully developed for this wind.
    pub fully_developed: bool,
    /// Depth (m) shallower than which waves of this height break (H/d = 0.78): the surf zone.
    pub breaking_depth_m: f32,
    /// Gerstner components, largest first.
    pub components: Vec<WaveComponent>,
}

/// The wind sea raised by `p`.
pub fn wave_spectrum(p: &WaveParams) -> JdResult<WaveSpectrum> {
    let u = p.wind_speed_mps;
    if !(u >= 0.0 && u.is_finite()) {
        return Err(JdError::invalid("wind_speed_mps", "must be ≥ 0"));
    }
    if let Some(f) = p.fetch_km
        && !(f > 0.0 && f.is_finite())
    {
        return Err(JdError::invalid("fetch_km", "must be positive"));
    }
    if !p.wind_azimuth_deg.is_finite() {
        return Err(JdError::invalid("wind_azimuth_deg", "must be finite"));
    }
    // Fully developed Pierson–Moskowitz sea.
    let (hs_fd, tp_fd) = (0.0246 * u * u, 0.78 * u);
    let (mut hs, mut tp, mut fully) = (hs_fd, tp_fd, true);
    if let Some(f) = p.fetch_km
        && u > 0.0
    {
        let chi = G * f * 1000.0 / (u * u);
        let hs_f = 1.6e-3 * chi.sqrt() * u * u / G;
        let tp_f = 0.2857 * chi.cbrt() * u / G;
        if hs_f < hs_fd {
            (hs, tp, fully) = (hs_f, tp_f.min(tp_fd), false);
        }
    }
    let wavelength = G * tp * tp / (2.0 * std::f32::consts::PI);
    let travel = (p.wind_azimuth_deg + 180.0).rem_euclid(360.0);
    let component = |share: f32, l_div: f32, spread: f32, phase: f32| {
        let l = wavelength / l_div;
        WaveComponent {
            amplitude_m: hs * share,
            wavelength_m: l,
            direction_deg: (travel + spread).rem_euclid(360.0),
            speed_mps: (G * l / (2.0 * std::f32::consts::PI)).sqrt(),
            phase,
        }
    };
    let components = if hs > 0.0 {
        vec![
            component(0.4, 1.0, 0.0, 0.0),
            component(0.2, 2.2, 25.0, 1.3),
            component(0.1, 5.0, -35.0, 2.9),
        ]
    } else {
        Vec::new()
    };
    Ok(WaveSpectrum {
        significant_height_m: hs,
        peak_period_s: tp,
        wavelength_m: wavelength,
        fully_developed: fully,
        breaking_depth_m: hs / 0.78,
        components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gale_fetch_limited_and_fully_developed_match_the_forecasting_curves() {
        // 20 m/s over 100 km: the JONSWAP curves give Hs ≈ 3.2 m, Tp ≈ 7.9 s.
        let s = wave_spectrum(&WaveParams {
            wind_speed_mps: 20.0,
            fetch_km: Some(100.0),
            wind_azimuth_deg: 270.0,
        })
        .expect("spectrum");
        assert!(
            (s.significant_height_m - 3.23).abs() < 0.05,
            "Hs {}",
            s.significant_height_m
        );
        assert!(
            (s.peak_period_s - 7.85).abs() < 0.05,
            "Tp {}",
            s.peak_period_s
        );
        assert!(!s.fully_developed);
        assert_eq!(s.components.len(), 3);
        assert_eq!(
            s.components[0].direction_deg, 90.0,
            "a west wind sends waves east"
        );
        // Unlimited fetch: Pierson–Moskowitz, Hs ≈ 9.8 m, Tp ≈ 15.6 s, L ≈ 380 m.
        let fd = wave_spectrum(&WaveParams {
            wind_speed_mps: 20.0,
            fetch_km: None,
            wind_azimuth_deg: 270.0,
        })
        .expect("fd");
        assert!(fd.fully_developed);
        assert!((fd.significant_height_m - 9.84).abs() < 0.05);
        assert!(
            (fd.wavelength_m - 380.0).abs() < 2.0,
            "L {}",
            fd.wavelength_m
        );
        assert!((fd.breaking_depth_m - 12.6).abs() < 0.1);
        // Steepness stays well below the Gerstner cusp limit k·A < 1 for every component.
        for c in &fd.components {
            let ka = 2.0 * std::f32::consts::PI / c.wavelength_m * c.amplitude_m;
            assert!(ka < 0.2, "k·A {ka}");
        }
        // A 5 000 km fetch is more than a 20 m/s sea needs: it reports fully developed.
        assert!(
            wave_spectrum(&WaveParams {
                wind_speed_mps: 20.0,
                fetch_km: Some(5000.0),
                wind_azimuth_deg: 0.0,
            })
            .expect("long fetch")
            .fully_developed
        );
        // Calm: no components; bad inputs are refused.
        assert!(
            wave_spectrum(&WaveParams {
                wind_speed_mps: 0.0,
                ..WaveParams::default()
            })
            .expect("calm")
            .components
            .is_empty()
        );
        assert!(
            wave_spectrum(&WaveParams {
                wind_speed_mps: -1.0,
                ..WaveParams::default()
            })
            .is_err()
        );
        assert!(
            wave_spectrum(&WaveParams {
                fetch_km: Some(0.0),
                ..WaveParams::default()
            })
            .is_err()
        );
    }
}
