//! Environment diagnostics (`jd diag env`, MCP `diag.env`).

use serde::{Deserialize, Serialize};

/// Presence of an optional external tool on `PATH`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ToolStatus {
    /// Tool name.
    pub name: String,
    /// Resolved path, if found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Snapshot of the host environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EnvInfo {
    /// Jenga Dunia version.
    pub app_version: String,
    /// OS name and version.
    pub os: String,
    /// CPU architecture.
    pub arch: String,
    /// Logical CPU count.
    pub cpus: usize,
    /// Total physical memory in bytes.
    pub memory_total_bytes: u64,
    /// Currently available memory in bytes.
    pub memory_available_bytes: u64,
    /// Current working directory.
    pub cwd: String,
    /// Optional external tools (validators, DCCs) detected on PATH.
    pub tools: Vec<ToolStatus>,
}

const OPTIONAL_TOOLS: &[&str] = &[
    "blender",
    "usdchecker",
    "usdcat",
    "usdzip",
    "gltf-validator",
    "gltf-transform",
    "toktx",
];

fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
        // Windows: honour PATHEXT so npm `.cmd` shims and `.bat` wrappers are found too.
        if cfg!(windows) {
            let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
            for ext in exts.split(';').filter(|e| !e.is_empty()) {
                let with_ext = dir.join(format!("{name}{}", ext.to_ascii_lowercase()));
                if with_ext.is_file() {
                    return Some(with_ext.display().to_string());
                }
            }
        }
    }
    None
}

/// Collect environment information. Never fails; unknown values are reported as such.
pub fn env_info() -> EnvInfo {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    EnvInfo {
        app_version: crate::APP_VERSION.to_owned(),
        os: format!(
            "{} {}",
            sysinfo::System::name().unwrap_or_else(|| std::env::consts::OS.to_owned()),
            sysinfo::System::os_version().unwrap_or_default()
        )
        .trim()
        .to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        cpus: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        memory_total_bytes: sys.total_memory(),
        memory_available_bytes: sys.available_memory(),
        cwd: std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        tools: OPTIONAL_TOOLS
            .iter()
            .map(|t| ToolStatus {
                name: (*t).to_owned(),
                path: which(t),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{BenchParams, BenchScene, bench};
    #[test]
    fn bench_s1_runs_small_and_is_deterministic() {
        let p = BenchParams {
            scene: BenchScene::S1,
            size_px: Some(64),
        };
        let a = bench(&p).expect("bench");
        let b = bench(&p).expect("bench");
        assert_eq!(a.stages.len(), 4);
        assert!(a.stages.iter().all(|s| s.ms >= 0.0));
        assert_eq!(a.size_px, 64);
        assert_eq!(a.content_hash, b.content_hash, "same scene, same hash");
        assert!(
            bench(&BenchParams {
                scene: BenchScene::S1,
                size_px: Some(4)
            })
            .is_err()
        );
    }

    #[test]
    fn env_info_reports_sane_values() {
        let e = super::env_info();
        assert!(e.cpus >= 1);
        assert!(e.memory_total_bytes > 0);
        assert_eq!(e.tools.len(), super::OPTIONAL_TOOLS.len());
    }
}

/// Benchmark scenes (`docs/spec/performance-targets.md`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BenchScene {
    /// S1-small-interactive: fBm + ridged noise, add, thermal erosion (50 steps) — the preview path.
    #[default]
    S1,
}

/// `diag.bench` params.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BenchParams {
    /// Scene to run.
    #[serde(default)]
    pub scene: BenchScene,
    /// Square size in pixels (S1 default 1024).
    #[serde(default)]
    pub size_px: Option<u32>,
}

/// One timed stage.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BenchStage {
    /// Stage name.
    pub name: String,
    /// Wall time in milliseconds.
    pub ms: f64,
}

/// `diag.bench` result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BenchReport {
    /// Scene.
    pub scene: BenchScene,
    /// Square size in pixels.
    pub size_px: u32,
    /// Stages in order.
    pub stages: Vec<BenchStage>,
    /// Sum of the stages, milliseconds.
    pub total_ms: f64,
    /// Highest resident set size of this process observed between stages, bytes (None when the
    /// platform does not report it).
    pub peak_rss_bytes: Option<u64>,
    /// `debug` or `release` — debug numbers are not comparable with the spec's targets.
    pub profile: &'static str,
    /// Content hash of the final field (determinism check across runs and machines).
    pub content_hash: String,
}

fn rss_bytes() -> Option<u64> {
    let pid = sysinfo::get_current_pid().ok()?;
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory())
}

/// Run a benchmark scene with the real kernels and report per-stage wall time + peak RSS.
pub fn bench(p: &BenchParams) -> jd_types::JdResult<BenchReport> {
    use jd_terrain::{FractalKind, NoiseKind, NoiseParams, generate_noise, thermal_erode};
    let size = p.size_px.unwrap_or(1024);
    if !(16..=16384).contains(&size) {
        return Err(jd_types::JdError::invalid(
            "size_px",
            "must be within 16..=16384",
        ));
    }
    let world = jd_types::WorldSettings {
        extent_m: [size as f64 * 4.0, size as f64 * 4.0],
        resolution_px: [size, size],
        tile_size: 512,
        halo: 4,
        origin: [0.0, 0.0],
        crs: None,
        seed: 7,
    };
    let spacing = [4.0f32, 4.0f32];
    let mut stages = Vec::new();
    let mut peak = rss_bytes();
    let mut stage =
        |name: &str, f: &mut dyn FnMut() -> jd_types::JdResult<()>| -> jd_types::JdResult<()> {
            let t = std::time::Instant::now();
            f()?;
            stages.push(BenchStage {
                name: name.to_owned(),
                ms: t.elapsed().as_secs_f64() * 1000.0,
            });
            peak = peak.max(rss_bytes());
            Ok(())
        };
    let base = NoiseParams {
        seed: 7,
        feature_size_m: size as f32,
        octaves: 8,
        height_range_m: [0.0, 800.0],
        ..Default::default()
    };
    let mut fbm = None;
    stage("noise.fbm", &mut || {
        fbm = Some(generate_noise(
            &world,
            &NoiseParams {
                kind: NoiseKind::OpenSimplex2,
                fractal: FractalKind::Fbm,
                ..base.clone()
            },
        )?);
        Ok(())
    })?;
    let mut ridged = None;
    stage("noise.ridged", &mut || {
        ridged = Some(generate_noise(
            &world,
            &NoiseParams {
                fractal: FractalKind::Ridged,
                seed: 11,
                height_range_m: [0.0, 400.0],
                ..base.clone()
            },
        )?);
        Ok(())
    })?;
    let (Some(mut sum), Some(r)) = (fbm, ridged) else {
        return Err(jd_types::JdError::new(
            jd_types::ErrorCode::E_INTERNAL,
            "bench stages out of order",
        ));
    };
    stage("math.add", &mut || {
        let (a, b) = (sum.to_vec(), r.to_vec());
        let w = sum.width();
        sum.fill_with(|x, y| {
            let i = (y * w + x) as usize;
            a[i] + b[i]
        });
        Ok(())
    })?;
    let mut eroded = None;
    stage("erosion.thermal×50", &mut || {
        eroded = Some(thermal_erode(&sum, spacing, 35.0, 50, 0.25, None)?);
        Ok(())
    })?;
    let Some(eroded) = eroded else {
        return Err(jd_types::JdError::new(
            jd_types::ErrorCode::E_INTERNAL,
            "bench stages out of order",
        ));
    };
    let total_ms = stages.iter().map(|s| s.ms).sum();
    Ok(BenchReport {
        scene: p.scene,
        size_px: size,
        stages,
        total_ms,
        peak_rss_bytes: peak,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        content_hash: eroded.content_hash().to_string(),
    })
}
