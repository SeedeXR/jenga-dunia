//! Built-in node kinds (Phase 2 initial set).

use std::collections::BTreeMap;
use std::sync::Arc;

use jd_terrain::{
    AeolianParams, CoastalParams, DropletParams, FlowMethod, ForeignImport, Heightfield,
    HydraulicParams, NoiseParams, SeabedParams, SnowParams, StreamPowerParams, Stroke,
    THERMAL_MAX_RATE, aeolian_transport_with, apply_strokes, droplet_erode, fill_depressions_with,
    flow_accumulation_with, generate_noise, resample_to_world, sea_level, slope_mask, snow_cover,
    wetness_index_with,
};
use jd_types::{JdError, JdResult};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::{Map, Value as Json};

use crate::eval::Value;
use crate::model::{Port, PortType};
use crate::registry::{EvalCtx, NodeDef, Registry};

fn port(name: &str, ty: PortType, required: bool) -> Port {
    Port {
        name: name.into(),
        ty,
        required,
    }
}

fn params<T: for<'de> Deserialize<'de>>(p: &Map<String, Json>) -> JdResult<T> {
    serde_json::from_value(Json::Object(p.clone()))
        .map_err(|e| JdError::invalid("params", e.to_string()))
}

/// The shared headless device, or the error a node reports when the machine has none.
fn gpu() -> JdResult<&'static jd_gpu::Gpu> {
    jd_gpu::Gpu::shared().ok_or_else(|| {
        JdError::new(
            jd_types::ErrorCode::E_DEPENDENCY_MISSING,
            "no GPU adapter on this machine",
        )
        .next("Set `engine: \"cpu\"` on the node.")
    })
}

/// The world's shared wind fills in the named `wind_*` parameters a node leaves unset, so one
/// setting drives dunes, yardangs, snow scour and the wind sea together (the node hash covers the
/// world settings, so a wind change re-evaluates them).
fn with_world_wind(ctx: &EvalCtx, p: &Map<String, Json>, keys: &[&str]) -> Map<String, Json> {
    let mut p = p.clone();
    for k in keys {
        let v = match *k {
            "wind_speed_mps" => ctx.lod_world.wind.speed_mps,
            _ => ctx.lod_world.wind.azimuth_deg,
        };
        p.entry(*k).or_insert(Json::from(v));
    }
    p
}

fn field<'a>(inputs: &'a BTreeMap<String, Value>, name: &str) -> JdResult<&'a Arc<Heightfield>> {
    Ok(inputs
        .get(name)
        .ok_or_else(|| JdError::invalid(name, "input not connected"))?
        .as_field())
}

fn out(name: &str, hf: Heightfield) -> BTreeMap<String, Value> {
    BTreeMap::from([(name.to_owned(), Value::field(hf))])
}

/// Elementwise map of one field into a new field of the same shape.
fn map1(a: &Heightfield, f: impl Fn(f32) -> f32 + Sync) -> JdResult<Heightfield> {
    let mut outf = Heightfield::new(a.width(), a.height(), a.tile_size(), a.halo())?;
    outf.fill_with(|x, y| f(a.get(x, y)));
    Ok(outf)
}

/// Elementwise combination of two fields (must match in size).
fn map2(
    a: &Heightfield,
    b: &Heightfield,
    f: impl Fn(f32, f32) -> f32 + Sync,
) -> JdResult<Heightfield> {
    if (a.width(), a.height()) != (b.width(), b.height()) {
        return Err(JdError::invalid("inputs", "field sizes differ")
            .detail("a", format!("{}x{}", a.width(), a.height()))
            .detail("b", format!("{}x{}", b.width(), b.height())));
    }
    let mut outf = Heightfield::new(a.width(), a.height(), a.tile_size(), a.halo())?;
    outf.fill_with(|x, y| f(a.get(x, y), b.get(x, y)));
    Ok(outf)
}

/// No parameters.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoParams {}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ThermalParams {
    /// Angle of repose in degrees (sand ≈ 30–35, scree ≈ 35–40).
    #[serde(default = "talus_default")]
    talus_deg: f32,
    /// Where the steps run: `cpu` (default, bit-reproducible) or `gpu` (`jd-gpu`, the same kernel in
    /// WGSL; ≈ 1e-7 relative to the CPU; fails with E_DEPENDENCY_MISSING when no adapter exists).
    #[serde(default)]
    engine: Engine,
    /// CPU only: erode in `tile_px` squares with a halo of `iterations` cells (bit-identical to the
    /// whole field, a tile's scratch instead of 12 B per cell — how a 16k² layer fits); 0 = whole field.
    #[serde(default)]
    tile_px: u32,
    /// Jacobi steps (1..=1000); more steps move material further.
    #[serde(default = "iterations_default")]
    iterations: u32,
    /// Fraction of the excess that moves per step, (0, 0.25]; 0.25 is the stable maximum.
    #[serde(default = "rate_default")]
    rate: f32,
}
fn talus_default() -> f32 {
    35.0
}

/// `jd.input.layer@1` parameters.
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayerInputParams {
    /// Published layer to read, e.g. `height` or `height#lod2`.
    #[serde(default = "layer_input_default")]
    name: String,
}
fn layer_input_default() -> String {
    "height".to_owned()
}

/// Which processor runs a kernel that exists in both forms.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
enum Engine {
    /// The rayon CPU kernel: deterministic bit for bit across machines.
    #[default]
    Cpu,
    /// The WGSL kernel on the headless wgpu device (`jd-gpu`); results agree with the CPU to ~1e-7.
    Gpu,
}
fn iterations_default() -> u32 {
    50
}
fn rate_default() -> f32 {
    THERMAL_MAX_RATE
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct TerraceParams {
    /// Step height in metres (> 0).
    #[serde(default = "terrace_step")]
    step_m: f32,
    /// 0 = hard steps, 1 = untouched; in between blends the step edge over that fraction of the riser.
    #[serde(default = "terrace_softness")]
    softness: f32,
    /// Shift the step boundaries by this many metres.
    #[serde(default)]
    offset_m: f32,
}
fn terrace_step() -> f32 {
    25.0
}
fn terrace_softness() -> f32 {
    0.2
}

/// Terrace a height: quantise to `step_m` with a smoothstep riser of width `softness · step`.
fn terrace(h: f32, step_m: f32, softness: f32, offset_m: f32) -> f32 {
    let t = (h - offset_m) / step_m;
    let base = t.floor();
    let frac = t - base;
    let riser = softness.clamp(0.0, 1.0).max(1e-6);
    // Flat tread, then a smoothstep riser occupying the last `riser` fraction of the step.
    let u = ((frac - (1.0 - riser)) / riser).clamp(0.0, 1.0);
    let s = u * u * (3.0 - 2.0 * u);
    offset_m + (base + s) * step_m
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct GeomorphonParams {
    /// How far to look along each direction, metres; 0 = 20 cells.
    #[serde(default)]
    search_m: f32,
    /// Lines of sight within this angle of horizontal count as flat, degrees [0, 45).
    #[serde(default = "flat_one_deg")]
    flat_deg: f32,
}
fn flat_one_deg() -> f32 {
    1.0
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CurvatureParams {
    /// Curvature (1/m) that maps to ±1; smaller values exaggerate.
    #[serde(default = "curvature_scale")]
    scale: f32,
    /// Stencil half-width in samples (1..=16): 1 reacts to every bump, larger radii read
    /// ridges and valleys at landscape scale.
    #[serde(default = "one_px")]
    radius_px: u32,
}
fn curvature_scale() -> f32 {
    0.01
}
fn one_px() -> u32 {
    1
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SlopeParams {
    /// Slope that maps to 1.0 in the mask (degrees, (0, 90]).
    #[serde(default = "ninety")]
    max_deg: f32,
}
fn ninety() -> f32 {
    90.0
}

/// Horizontal sample spacing (x, y) of the world at the current LOD, in metres.
fn spacing_m(ctx: &EvalCtx) -> [f32; 2] {
    let s = ctx.lod_world.spacing_m();
    [s[0] as f32, s[1] as f32]
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct HydraulicNodeParams {
    /// Simulation steps (1..=2000).
    #[serde(default = "hyd_iterations")]
    iterations: u32,
    /// Time step in seconds ((0, 1]; 0.02 is stable for the pipe model).
    #[serde(default = "hyd_dt")]
    dt: f32,
    /// Rain in metres of water per second.
    #[serde(default = "hyd_rain")]
    rain_rate: f32,
    /// Where the steps run: `cpu` (default, bit-reproducible) or `gpu` (`jd-gpu`, the same passes in
    /// WGSL; ≈ 1e-5 relative to the CPU after 100 steps; E_DEPENDENCY_MISSING without an adapter).
    #[serde(default)]
    engine: Engine,
    /// Evaporation per second (fraction of depth).
    #[serde(default = "hyd_evap")]
    evaporation: f32,
    /// Sediment capacity constant Kc.
    #[serde(default = "one_f")]
    capacity_k: f32,
    /// Dissolving constant Ks.
    #[serde(default = "half")]
    dissolve_k: f32,
    /// Deposition constant Kd.
    #[serde(default = "one_f")]
    deposit_k: f32,
    /// Suspended sediment cap as a fraction of water depth (excess settles the same step).
    #[serde(default = "hyd_cap")]
    max_concentration: f32,
}
fn hyd_cap() -> f32 {
    0.3
}
fn hyd_iterations() -> u32 {
    100
}
fn hyd_dt() -> f32 {
    0.02
}
fn hyd_rain() -> f32 {
    0.012
}
fn hyd_evap() -> f32 {
    0.015
}
fn half() -> f32 {
    0.5
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BlurParams {
    /// Box-blur radius in pixels (1..=4; must not exceed the world's tile halo).
    #[serde(default = "one")]
    radius: u32,
}
fn one() -> u32 {
    1
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemapParams {
    /// Input range mapped from.
    #[serde(default = "unit")]
    from: [f32; 2],
    /// Output range mapped to.
    #[serde(default = "unit")]
    to: [f32; 2],
    /// Clamp to the output range.
    #[serde(default)]
    clamp: bool,
}
fn unit() -> [f32; 2] {
    [0.0, 1.0]
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScaleParams {
    /// Multiplier.
    #[serde(default = "one_f")]
    factor: f32,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CurveKind {
    /// Smoothstep from `edge0` to `edge1`: 0 below, 1 above, an S between.
    Smoothstep,
    /// `((v − lo) / (hi − lo))^gamma` over [lo, hi]: < 1 lifts the mid-tones, > 1 darkens them.
    Gamma,
    /// `lo + hi − v`: flips a mask or a height range.
    Invert,
    /// 0 below `edge0`, 1 above `edge1` (a hard step when they are equal).
    Threshold,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum MorphMode {
    /// Each cell takes the highest value within the radius: ridges and islands grow, pits close.
    Inflate,
    /// Each cell takes the lowest value within the radius: ridges thin, valleys widen.
    Deflate,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct MorphParams {
    /// Grow or shrink.
    mode: MorphMode,
    /// Window radius in pixels (1..=4; must not exceed the world's tile halo).
    #[serde(default = "one")]
    radius: u32,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CurveParams {
    /// The curve.
    kind: CurveKind,
    /// Start of the transition (smoothstep, threshold) or the low end of the range (gamma, invert).
    #[serde(default)]
    edge0: f32,
    /// End of the transition or the high end of the range.
    #[serde(default = "one_f")]
    edge1: f32,
    /// Gamma exponent (> 0).
    #[serde(default = "one_f")]
    gamma: f32,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PatternParams {
    /// The design.
    kind: jd_terrain::Pattern,
    /// Repeat distance, metres (> 0).
    #[serde(default = "pattern_period")]
    period_m: f32,
    /// Dot radius, line or band width, metres (0 < width ≤ period).
    #[serde(default = "pattern_width")]
    width_m: f32,
    /// Rotation of the design, degrees clockwise from north.
    #[serde(default)]
    angle_deg: f32,
    /// Edge blur as a share of the width (0..1).
    #[serde(default)]
    softness: f32,
}
fn pattern_period() -> f32 {
    100.0
}
fn pattern_width() -> f32 {
    25.0
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AspectParams {
    /// Compass direction the light comes from, degrees clockwise from north (180 = a southern sun).
    #[serde(default = "south")]
    azimuth_deg: f32,
    /// How fast the mask saturates with slope (0..1): at 1 a 45° slope facing the light is fully 1.
    #[serde(default = "half_f")]
    strength: f32,
}
fn south() -> f32 {
    180.0
}
fn half_f() -> f32 {
    0.5
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct OcclusionParams {
    /// How far terrain can hide the sky, metres (0 = 20 cells).
    #[serde(default)]
    reach_m: f32,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct EdgeFadeParams {
    /// Width of the fade inside the border, metres (> 0).
    #[serde(default = "edge_width")]
    width_m: f32,
    /// Height the border fades to, metres.
    #[serde(default)]
    edge_m: f32,
}
fn edge_width() -> f32 {
    200.0
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct StampParams {
    /// Heightmap file in the project's assets/ (PNG 16-bit, EXR, TIFF, RAW; row 0 = north unless `north_up` is false).
    path: String,
    /// Row 0 of the file is north (the image convention) or south (the field convention).
    #[serde(default = "yes_b")]
    north_up: bool,
    /// Where, how big, how high and how it meets the terrain.
    #[serde(flatten)]
    place: jd_terrain::Placement,
}
fn yes_b() -> bool {
    true
}

/// The file named by a node's `path`, resolved and confined to the project's assets/.
fn asset_file(ctx: &EvalCtx, path: &str) -> JdResult<std::path::PathBuf> {
    let rel = std::path::Path::new(path);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(JdError::invalid("path", "must be a plain relative path inside assets/")
            .next("Copy the file into the project's assets/ directory (project.import_asset) and reference it by name."));
    }
    let Some(dir) = &ctx.assets_dir else {
        return Err(JdError::invalid(
            "path",
            "no project is open, so there is no assets/ directory to read from",
        ));
    };
    let file = std::fs::canonicalize(dir.join(rel))
        .map_err(|e| JdError::invalid("path", format!("cannot read `{path}`: {e}")))?;
    let base = std::fs::canonicalize(dir)
        .map_err(|e| JdError::invalid("path", format!("assets directory unavailable: {e}")))?;
    if !file.starts_with(&base) {
        return Err(JdError::invalid(
            "path",
            "file resolves outside the project's assets/ directory",
        ));
    }
    Ok(file)
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BlendParams {
    /// Opacity 0..1: how much of the blended result replaces the input (1 = all of it, 0 = the
    /// input passes through).
    #[serde(default = "one_f")]
    opacity: f32,
}

/// Pointwise over `a`, `b` and an optional third field of the same size.
fn map3(
    a: &Heightfield,
    b: &Heightfield,
    t: Option<&Heightfield>,
    f: impl Fn(f32, f32, f32) -> f32 + Sync,
) -> JdResult<Heightfield> {
    for (name, o) in [("b", Some(b)), ("t", t)] {
        if let Some(o) = o
            && (o.width(), o.height()) != (a.width(), a.height())
        {
            return Err(JdError::invalid(name, "field size differs from `in`"));
        }
    }
    let mut outf = Heightfield::new(a.width(), a.height(), a.tile_size(), a.halo())?;
    outf.fill_with(|x, y| f(a.get(x, y), b.get(x, y), t.map_or(1.0, |t| t.get(x, y))));
    Ok(outf)
}

fn blend_opacity(p: &Map<String, Json>) -> JdResult<f32> {
    let bp: BlendParams = params(p)?;
    if !(0.0..=1.0).contains(&bp.opacity) {
        return Err(JdError::invalid("opacity", "must be within 0..=1"));
    }
    Ok(bp.opacity)
}
fn one_f() -> f32 {
    1.0
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct OutputParams {
    /// Layer name the result is published under.
    #[serde(default = "height")]
    layer: String,
}
fn height() -> String {
    "height".into()
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SculptParams {
    /// Recorded brush strokes, applied in order on top of the input.
    #[serde(default)]
    strokes: Vec<Stroke>,
    /// Keep the result within [min, max] after the strokes — `[0, 1]` when painting a density or
    /// class mask instead of a height.
    #[serde(default)]
    clamp: Option<[f32; 2]>,
}

/// One biome rule: every given range must hold; a range on an input that is not connected
/// never matches. Rules are tried in order; the first match wins.
#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BiomeRule {
    /// Display name (e.g. "alpine meadow").
    name: String,
    /// Height band in metres [min, max).
    #[serde(default)]
    height_m: Option<[f32; 2]>,
    /// Slope band in 0..1 (the `jd.analysis.slope@1` scale, 1 = 90°) [min, max).
    #[serde(default)]
    slope: Option<[f32; 2]>,
    /// Wetness band in 0..1 (the `jd.hydrology.wetness@1` index) [min, max).
    #[serde(default)]
    wetness: Option<[f32; 2]>,
    /// Band on the optional `mask` input [min, max): any field — `lakes` from `jd.hydrology.fill@1` for salt crusts on playa floors, a painted mask, a corridor.
    #[serde(default)]
    mask: Option<[f32; 2]>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BiomeParams {
    /// Rules in priority order; class ids are 1-based positions, 0 = unclassified. Empty = a
    /// temperate default set (beach, grassland, forest, rock, snow).
    #[serde(default)]
    rules: Vec<BiomeRule>,
}

/// Every given range must be a proper half-open interval (refused before journaling).
fn validate_biome_rules(bp: &BiomeParams) -> JdResult<()> {
    for (k, rule) in bp.rules.iter().enumerate() {
        for (what, range) in [
            ("height_m", rule.height_m),
            ("slope", rule.slope),
            ("wetness", rule.wetness),
            ("mask", rule.mask),
        ] {
            if let Some([lo, hi]) = range
                && (lo >= hi || lo.is_nan() || hi.is_nan())
            {
                return Err(JdError::invalid(
                    "rules",
                    format!(
                        "rule {} ({}): {what} range must have min < max",
                        k + 1,
                        rule.name
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn default_biome_rules() -> Vec<BiomeRule> {
    let r = |name: &str, h: Option<[f32; 2]>, s: Option<[f32; 2]>| BiomeRule {
        name: name.to_owned(),
        height_m: h,
        slope: s,
        wetness: None,
        mask: None,
    };
    vec![
        r("rock", None, Some([0.6, 2.0])),
        r("snow", Some([900.0, f32::MAX]), None),
        r("beach", Some([f32::MIN, 5.0]), None),
        r("grassland", Some([5.0, 300.0]), None),
        r("forest", Some([300.0, 900.0]), None),
    ]
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ClassMaskParams {
    /// Class id to select (1-based rule position from `jd.biome.classify@1`).
    id: u32,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FlowParams {
    /// Output ln(1 + area) / ln(1 + N) in 0..1 (true) or the raw upstream cell count (false).
    #[serde(default = "yes")]
    log_scale: bool,
    /// Routing: `d8` (single steepest neighbour), `dinf` (Tarboton facets, smoother streams) or `mfd` (Freeman, most diffuse).
    #[serde(default)]
    method: FlowMethod,
}
fn yes() -> bool {
    true
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct InputParams {
    /// File inside the project's `assets/` directory (png, exr, tif/tiff, r16, r32).
    path: String,
    /// How to read it (height range for quantised files, size for RAW, row order).
    #[serde(flatten)]
    import: ForeignImport,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FillParams {
    /// Extra rise per step across filled flats so they still drain (metres; 0 = perfectly flat lakes).
    #[serde(default = "fill_eps")]
    epsilon_m: f32,
}
fn fill_eps() -> f32 {
    0.001
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SeaLevelParams {
    /// Sea level in metres (world heights).
    #[serde(default)]
    level_m: f32,
    /// Shoreline softness: the ocean mask fades from 1 to 0 over this many metres above the level.
    #[serde(default = "shore_m")]
    shore_m: f32,
    /// Water clarity as the depth (metres) at which the bottom fades out: ~30 for crystal-clear
    /// tropical water, ~15 open ocean, ~3 for a murky estuary. Drives the viewport's Beer–Lambert
    /// absorption per colour channel and the baked water maps; the ocean mask itself ignores it.
    #[serde(default = "visibility_m")]
    visibility_m: f32,
    /// Colour of the suspended matter that scatters back out of the water (RGB 0..1): blue-green
    /// plankton, brown silt, green algae. Dominates as the water gets murkier.
    #[serde(default = "tint")]
    tint: [f32; 3],
    /// Wind speed 10 m above the sea, m/s (0 = calm): 3 ripples, 8 a moderate sea, 20 a gale. Unset
    /// → the world's shared wind. Sets
    /// the wave height and length the viewport animates (`ocean.waves`: JONSWAP growth capped at
    /// the fully developed Pierson–Moskowitz sea); the ocean mask ignores it.
    #[serde(default = "wind_speed_mps")]
    wind_speed_mps: f32,
    /// Fetch, km: how far the wind has blown over open water. Omit for a fully developed sea.
    #[serde(default)]
    fetch_km: Option<f32>,
    /// Where the wind blows FROM, degrees clockwise from north; waves travel the opposite way. Unset
    /// → the world's shared wind.
    #[serde(default = "wind_azimuth_deg")]
    wind_azimuth_deg: f32,
    /// Bend the waves over the shelf (refraction): the front slows in water shallower than half
    /// its length and turns towards the shore, so rays converge on headlands and spread in bays;
    /// `exposure` is multiplied by that focus (up to 2 at a headland). The wavelength is the wind
    /// sea's (`wind_speed_mps`, `fetch_km`); a 16 s swell bends from 200 m down, a 3 s chop from 7 m.
    #[serde(default)]
    refraction: bool,
    /// Width of the coastal band, metres (> 0), within which the surface `current` bends from the
    /// wind's direction to run alongshore.
    #[serde(default = "current_band_m")]
    current_band_m: f32,
}
fn current_band_m() -> f32 {
    2000.0
}
fn wind_speed_mps() -> f32 {
    8.0
}
fn wind_azimuth_deg() -> f32 {
    270.0
}
fn shore_m() -> f32 {
    2.0
}
fn visibility_m() -> f32 {
    15.0
}
fn tint() -> [f32; 3] {
    [0.1, 0.35, 0.4]
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct WetnessParams {
    /// Flow routing for the drainage area: `d8` (channel-like streaks), `dinf` (smoother) or `mfd` (diffuse moisture).
    #[serde(default)]
    method: FlowMethod,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RiversParams {
    /// Log-scaled flow accumulation (0..1) above which a cell is a river channel.
    #[serde(default = "rivers_threshold")]
    threshold: f32,
    /// Incision depth at the largest rivers, metres.
    #[serde(default = "rivers_depth")]
    depth_m: f32,
    /// Flow routing used to find channels: `d8`, `dinf` (fewer parallel-line artefacts) or `mfd` (wide, diffuse).
    #[serde(default)]
    method: FlowMethod,
    /// Springs: points in layer metres (east from the west edge, north from the south edge) where a
    /// river starts regardless of the local catchment — each pours `spring_area_m2` of upstream
    /// area in (a karst resurgence, a river entering from beyond the world's edge).
    #[serde(default)]
    springs: Vec<[f32; 2]>,
    /// Upstream area each spring contributes, m² (default 1 km²).
    #[serde(default = "spring_area")]
    spring_area_m2: f32,
}
fn spring_area() -> f32 {
    1.0e6
}
fn rivers_threshold() -> f32 {
    0.55
}
fn rivers_depth() -> f32 {
    20.0
}

/// Tile-local box blur that reads neighbours through the halo (ADR-0005). Fails cleanly when
/// the field's halo is smaller than the radius — a tile must never read outside its padding.
/// A tile-local window filter: `reduce` sees the (2r+1)² values around each cell (through the
/// halo) and returns the cell's new value — the mean is a box blur, max / min inflate / deflate,
/// the median denoises.
fn window_filter(
    a: &Heightfield,
    radius: u32,
    reduce: impl Fn(&mut [f32]) -> f32 + Sync,
) -> JdResult<Heightfield> {
    if radius == 0 || radius > a.halo() {
        return Err(JdError::invalid(
            "radius",
            format!("filter radius {radius} exceeds the tile halo {}", a.halo()),
        )
        .why("Tile-local filters can only read as far as the halo around each tile.")
        .next("Lower the radius or raise `world.halo` in project.json."));
    }
    let r = radius as i64;
    let mut outf = Heightfield::new(a.width(), a.height(), a.tile_size(), a.halo())?;
    let ts = a.tile_size();
    let results: Vec<Vec<f32>> = a
        .tiles()
        .par_iter()
        .map(|t| {
            let mut data = vec![0.0f32; (ts * ts) as usize];
            let mut window = Vec::with_capacity(((2 * r + 1) * (2 * r + 1)) as usize);
            for iy in 0..ts as i64 {
                for ix in 0..ts as i64 {
                    window.clear();
                    for dy in -r..=r {
                        for dx in -r..=r {
                            window.push(t.get_padded(ix + dx, iy + dy));
                        }
                    }
                    data[(iy * ts as i64 + ix) as usize] = reduce(&mut window);
                }
            }
            data
        })
        .collect();
    let (tx, _) = a.tile_counts();
    outf.fill_with(|x, y| {
        let t = &results[((y / ts) * tx + x / ts) as usize];
        t[((y % ts) * ts + (x % ts)) as usize]
    });
    Ok(outf)
}

fn box_blur(a: &Heightfield, radius: u32) -> JdResult<Heightfield> {
    window_filter(a, radius, |w| w.iter().sum::<f32>() / w.len() as f32)
}

/// The median of a window (partial sort; NaN-free fields by construction).
fn median(w: &mut [f32]) -> f32 {
    let mid = w.len() / 2;
    let (_, m, _) = w.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    *m
}

/// Register every built-in node kind.
pub fn register_all(r: &mut Registry) {
    r.register(NodeDef::new::<NoiseParams, _>(
        "jd.noise.fbm@1",
        "Fractal noise heightfield (OpenSimplex2/Perlin/… × fBm/ridged/ping-pong), parameters in metres",
        vec![],
        vec![port("out", PortType::Heightfield, false)],
        |ctx: &EvalCtx, p, _| {
            let np: NoiseParams = params(p)?;
            Ok(out("out", generate_noise(&ctx.lod_world, &np)?))
        },
    ));
    r.register(NodeDef::new::<jd_terrain::NetworkParams, _>(
        "jd.hydrology.network@1",
        "Hydrology-first terrain: a seeded drainage network is grown from the river mouths first and the heights are built along it (headwaters steep, big rivers flat), so every cell drains by construction; with `valley_relief_m` every river carries a valley cross-section (`valley_shape`: 1 V, 2 U, 0.5 canyon notch) blended into the heights within `valley_width_m` of it — `out` heights, `rivers` (Strahler order ≥ `river_order`, 0..1) and `order`",
        vec![],
        vec![port("out", PortType::Heightfield, false), port("rivers", PortType::Heightfield, false), port("order", PortType::Heightfield, false)],
        |ctx: &EvalCtx, p, _| {
            let np: jd_terrain::NetworkParams = params(p)?;
            let w = &ctx.lod_world;
            let (out_h, rivers, order) = jd_terrain::hydrology_first(w.resolution_px[0], w.resolution_px[1], w.tile_size, w.halo, spacing_m(ctx), &np)?;
            Ok(BTreeMap::from([("out".to_owned(), Value::field(out_h)), ("rivers".to_owned(), Value::field(rivers)), ("order".to_owned(), Value::field(order))]))
        },
    ).with_memory(50.0));
    r.register(NodeDef::new::<LayerInputParams, _>(
        "jd.input.layer@1",
        "Read a layer the project has already published (`height`, `height#lod2`, …) and resample it onto the current grid — the gate pattern: erode at a coarse lod, then add the fine detail at lod 0 (the layer's content hash is part of the cache key)",
        vec![],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, _| {
            let lp: LayerInputParams = params(p)?;
            let load = ctx.layers.as_ref().ok_or_else(|| {
                JdError::invalid("name", "no project is open, so there are no published layers to read")
            })?;
            let hf = load(&lp.name)?.ok_or_else(|| {
                JdError::invalid("name", format!("no published layer named `{}`", lp.name))
                    .next("Evaluate the graph (at the lod that publishes it) first; coarse layers are named `<layer>#lod<N>`.")
            })?;
            let w = &ctx.lod_world;
            if (hf.width(), hf.height()) == (w.resolution_px[0], w.resolution_px[1]) {
                return Ok(BTreeMap::from([("out".to_owned(), Value::from_arc(hf.clone()))]));
            }
            Ok(out("out", resample_to_world(&hf.to_vec(), hf.width(), hf.height(), false, w)?))
        },
    ));
    r.register(
        NodeDef::new::<NoParams, _>(
            "jd.math.add@1",
            "a + b",
            vec![
                port("a", PortType::Heightfield, true),
                port("b", PortType::Heightfield, true),
            ],
            vec![port("out", PortType::Heightfield, false)],
            |_, _, i| {
                Ok(out(
                    "out",
                    map2(field(i, "a")?, field(i, "b")?, |a, b| a + b)?,
                ))
            },
        )
        .local(|p| {
            let _ = p;
            0
        }),
    );
    // Curves (World Creator's Curve / Smoothstep / Step / Invert effects) and pattern masks.
    r.register(
        NodeDef::new::<CurveParams, _>(
            "jd.math.curve@1",
            "Reshape a mask or height: `smoothstep` from edge0 to edge1, `gamma` over [edge0, edge1], `invert` within [edge0, edge1], or a `threshold` step — the curve, smoothstep, step and invert effects of a distribution",
            vec![port("in", PortType::Heightfield, true)],
            vec![port("out", PortType::Heightfield, false)],
            |_, p, i| {
                let cp: CurveParams = params(p)?;
                if !(cp.gamma > 0.0 && cp.gamma.is_finite()) || !cp.edge0.is_finite() || !cp.edge1.is_finite() {
                    return Err(JdError::invalid("gamma", "gamma must be positive and the edges finite"));
                }
                let (e0, e1) = (cp.edge0, cp.edge1);
                let span = e1 - e0;
                Ok(out(
                    "out",
                    map1(field(i, "in")?, move |v| match cp.kind {
                        CurveKind::Smoothstep => {
                            let t = if span.abs() > 0.0 { ((v - e0) / span).clamp(0.0, 1.0) } else if v >= e0 { 1.0 } else { 0.0 };
                            t * t * (3.0 - 2.0 * t)
                        }
                        CurveKind::Gamma => {
                            let t = if span.abs() > 0.0 { ((v - e0) / span).clamp(0.0, 1.0) } else { 0.0 };
                            e0 + t.powf(cp.gamma) * span
                        }
                        CurveKind::Invert => e0 + e1 - v,
                        CurveKind::Threshold => {
                            if v < e0.min(e1) { 0.0 } else if v >= e0.max(e1) { 1.0 } else { (v - e0.min(e1)) / span.abs() }
                        }
                    })?,
                ))
            },
        )
        .local(|_| 0),
    );
    r.register(NodeDef::new::<PatternParams, _>(
        "jd.pattern@1",
        "A 0..1 pattern mask in world metres over the input's footprint — `dots`, `grid`, `stripes` or `checker` with `period_m`, `width_m`, `angle_deg` and edge `softness` — for scatter rows, field patterns, paved grids and stamp masks",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let pp: PatternParams = params(p)?;
            Ok(out("out", jd_terrain::pattern_mask(field(i, "in")?, spacing_m(ctx), pp.kind, pp.period_m, pp.width_m, pp.angle_deg, pp.softness)?))
        },
    ).with_memory(4.0));
    // Masks and filters from World Creator's distribution and filter lists.
    r.register(NodeDef::new::<AspectParams, _>(
        "jd.analysis.aspect@1",
        "Sun-direction / aspect mask 0..1: 1 where the slope faces the light from `azimuth_deg`, 0 where it faces away, 0.5 on flat ground — snow that melts on the sunny side, moss on the shaded one, World Creator's Sun Direction distribution",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let ap: AspectParams = params(p)?;
            Ok(out("out", jd_terrain::aspect_mask(field(i, "in")?, spacing_m(ctx), ap.azimuth_deg, ap.strength)?))
        },
    ).with_memory(4.0));
    r.register(NodeDef::new::<OcclusionParams, _>(
        "jd.analysis.occlusion@1",
        "Sky occlusion 0..1 from the highest terrain seen along 8 directions within `reach_m`: 0 on an open plain, high at the bottom of a gorge — an ambient-occlusion, cavity and shelter mask (1 − it is the sky view)",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let op: OcclusionParams = params(p)?;
            let sp = spacing_m(ctx);
            let reach = if op.reach_m > 0.0 { op.reach_m } else { 20.0 * sp[0].max(sp[1]) };
            Ok(out("out", jd_terrain::occlusion_mask(field(i, "in")?, sp, reach)?))
        },
    ).with_memory(4.0));
    r.register(NodeDef::new::<EdgeFadeParams, _>(
        "jd.filter.edge_fade@1",
        "Border fade: within `width_m` of the world's edge the heights blend smoothly to `edge_m`, so the terrain tiles or sits in an engine without a cliff at its border (World Creator's Zero Edge / Border Blend)",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let ep: EdgeFadeParams = params(p)?;
            Ok(out("out", jd_terrain::edge_fade(field(i, "in")?, spacing_m(ctx), ep.width_m, ep.edge_m)?))
        },
    ).with_memory(4.0));
    // Placed shapes: World Creator's stamp and primitive layers.
    r.register(NodeDef::new::<jd_terrain::PolygonParams, _>(
        "jd.shape.polygon@1",
        "A polygon layer: a closed outline (`points` in layer metres) takes `height_m` through `op` — `lerp` flattens a pad, `add` lifts, `max` / `min` clip — fading to nothing over `falloff_m` outside the outline, at `opacity` (World Creator's polygon shape)",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let pp: jd_terrain::PolygonParams = params(p)?;
            Ok(out("out", jd_terrain::place_polygon(field(i, "in")?, spacing_m(ctx), &pp)?))
        },
    ).with_memory(4.0));
    r.register(NodeDef::new::<jd_terrain::PrimitiveParams, _>(
        "jd.shape.primitive@1",
        "A placed primitive — cone, dome, plateau, ramp or crater — at `center_m`, `size_m` and `rotation_deg`, its 0..1 shape scaled to `height_range_m`, faded over `falloff_m` at the edge, and combined with the terrain by `op` (add, max to raise it out, min to carve it in, lerp to overwrite) at `opacity`; `ratio` sets a plateau's flat top or a crater's rim radius, `floor` a crater's floor",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let pp: jd_terrain::PrimitiveParams = params(p)?;
            Ok(out("out", jd_terrain::place_primitive(field(i, "in")?, spacing_m(ctx), &pp)?))
        },
    ).with_memory(4.0));
    r.register(NodeDef::new::<StampParams, _>(
        "jd.shape.stamp@1",
        "A heightmap stamp from the project's assets/ set down at `center_m`, `size_m` and `rotation_deg` — its own range normalised to 0..1 and scaled to `height_range_m`, faded over `falloff_m`, combined with the terrain by `op` at `opacity`; a mountain from a photo's relief, a crater, a city plinth placed where you want it",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: StampParams = params(p)?;
            let file = asset_file(ctx, &sp.path)?;
            let opts = jd_terrain::ForeignImport { north_up: sp.north_up, ..Default::default() };
            let (data, w, h) = jd_terrain::read_foreign(&file, &opts)?;
            Ok(out("out", jd_terrain::place_stamp(field(i, "in")?, spacing_m(ctx), &sp.place, &data, w, h)?))
        },
    ).with_memory(8.0));
    // Blend operations of a layer stack: the chain comes in on `in`, the other layer on `b`.
    for (kind, desc, op) in [
        (
            "jd.math.min@1",
            "min(in, b): keeps the lower of the chain and another layer — carve a stamp or a valley floor into the terrain; `opacity` blends the result over the input",
            f32::min as fn(f32, f32) -> f32,
        ),
        (
            "jd.math.max@1",
            "max(in, b): keeps the higher of the chain and another layer — raise a stamp, a plateau or a dune field onto the terrain; `opacity` blends the result over the input",
            f32::max as fn(f32, f32) -> f32,
        ),
    ] {
        r.register(
            NodeDef::new::<BlendParams, _>(
                kind,
                desc,
                vec![
                    port("in", PortType::Heightfield, true),
                    port("b", PortType::Heightfield, true),
                ],
                vec![port("out", PortType::Heightfield, false)],
                move |_, p, i| {
                    let k = blend_opacity(p)?;
                    Ok(out(
                        "out",
                        map2(field(i, "in")?, field(i, "b")?, |a, b| {
                            a + (op(a, b) - a) * k
                        })?,
                    ))
                },
            )
            .local(|_| 0),
        );
    }
    r.register(
        NodeDef::new::<BlendParams, _>(
            "jd.math.lerp@1",
            "in + (b − in) × t × opacity: blends another layer over the chain where the optional 0..1 `t` mask says (everywhere when unwired) — World Creator's layer overwrite with a mask and an opacity",
            vec![
                port("in", PortType::Heightfield, true),
                port("b", PortType::Heightfield, true),
                port("t", PortType::Heightfield, false),
            ],
            vec![port("out", PortType::Heightfield, false)],
            |_, p, i| {
                let k = blend_opacity(p)?;
                let t = i.get("t").map(|v| v.as_field());
                Ok(out("out", map3(field(i, "in")?, field(i, "b")?, t.map(|t| &**t), |a, b, t| a + (b - a) * t.clamp(0.0, 1.0) * k)?))
            },
        )
        .local(|_| 0),
    );
    r.register(
        NodeDef::new::<NoParams, _>(
            "jd.math.multiply@1",
            "a × b (use a 0..1 mask as b to attenuate)",
            vec![
                port("a", PortType::Heightfield, true),
                port("b", PortType::Heightfield, true),
            ],
            vec![port("out", PortType::Heightfield, false)],
            |_, _, i| {
                Ok(out(
                    "out",
                    map2(field(i, "a")?, field(i, "b")?, |a, b| a * b)?,
                ))
            },
        )
        .local(|p| {
            let _ = p;
            0
        }),
    );
    r.register(
        NodeDef::new::<ScaleParams, _>(
            "jd.math.scale@1",
            "in × factor",
            vec![port("in", PortType::Heightfield, true)],
            vec![port("out", PortType::Heightfield, false)],
            |_, p, i| {
                let sp: ScaleParams = params(p)?;
                Ok(out("out", map1(field(i, "in")?, |v| v * sp.factor)?))
            },
        )
        .local(|p| {
            let _ = p;
            0
        }),
    );
    r.register(
        NodeDef::new::<RemapParams, _>(
            "jd.math.remap@1",
            "Linear remap of [from0, from1] to [to0, to1], optional clamp",
            vec![port("in", PortType::Heightfield, true)],
            vec![port("out", PortType::Heightfield, false)],
            |_, p, i| {
                let rp: RemapParams = params(p)?;
                let span = rp.from[1] - rp.from[0];
                if span == 0.0 || !span.is_finite() {
                    return Err(JdError::invalid(
                        "from",
                        "range must be non-empty and finite",
                    ));
                }
                let k = (rp.to[1] - rp.to[0]) / span;
                let (lo, hi) = (rp.to[0].min(rp.to[1]), rp.to[0].max(rp.to[1]));
                Ok(out(
                    "out",
                    map1(field(i, "in")?, |v| {
                        let y = rp.to[0] + (v - rp.from[0]) * k;
                        if rp.clamp { y.clamp(lo, hi) } else { y }
                    })?,
                ))
            },
        )
        .local(|p| {
            let _ = p;
            0
        }),
    );
    r.register(
        NodeDef::new::<BlurParams, _>(
            "jd.filter.blur@1",
            "Box blur (tile-local, reads through the halo; radius ≤ world.halo)",
            vec![port("in", PortType::Heightfield, true)],
            vec![port("out", PortType::Heightfield, false)],
            |_, p, i| {
                let bp: BlurParams = params(p)?;
                if !(1..=4).contains(&bp.radius) {
                    return Err(JdError::invalid("radius", "must be within 1..=4"));
                }
                Ok(out("out", box_blur(field(i, "in")?, bp.radius)?))
            },
        )
        .local(|p| {
            let _ = p;
            p.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32
        }),
    );
    r.register(
        NodeDef::new::<MorphParams, _>(
            "jd.filter.inflate@1",
            "Morphological inflate / deflate: each cell takes the highest (or lowest) value within `radius` pixels — grows or shrinks masks and landforms, closes pits or thins ridges (World Creator's Inflate / Expand and Shrink; tile-local, radius ≤ world.halo)",
            vec![port("in", PortType::Heightfield, true)],
            vec![port("out", PortType::Heightfield, false)],
            |_, p, i| {
                let mp: MorphParams = params(p)?;
                if !(1..=4).contains(&mp.radius) {
                    return Err(JdError::invalid("radius", "must be within 1..=4"));
                }
                let f = field(i, "in")?;
                Ok(out("out", match mp.mode {
                    MorphMode::Inflate => window_filter(f, mp.radius, |w| w.iter().copied().fold(f32::NEG_INFINITY, f32::max))?,
                    MorphMode::Deflate => window_filter(f, mp.radius, |w| w.iter().copied().fold(f32::INFINITY, f32::min))?,
                }))
            },
        )
        .local(|p| p.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32),
    );
    r.register(
        NodeDef::new::<BlurParams, _>(
            "jd.filter.denoise@1",
            "Median filter: removes single-cell spikes and pits (DEM noise, import artefacts) while keeping edges that a blur would soften (World Creator's Denoise; tile-local, radius ≤ world.halo)",
            vec![port("in", PortType::Heightfield, true)],
            vec![port("out", PortType::Heightfield, false)],
            |_, p, i| {
                let bp: BlurParams = params(p)?;
                if !(1..=4).contains(&bp.radius) {
                    return Err(JdError::invalid("radius", "must be within 1..=4"));
                }
                Ok(out("out", window_filter(field(i, "in")?, bp.radius, median)?))
            },
        )
        .local(|p| p.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32),
    );
    r.register(NodeDef::new::<ThermalParams, _>(
        "jd.erosion.thermal@1",
        "Thermal erosion: material steeper than the talus angle slides downhill (mass-conserving Jacobi steps); an optional `hardness` field (0..1, e.g. from `jd.terrain.strata@1`) lets hard rock hold near-vertical faces and shed at a tenth of the rate; `engine: gpu` runs the same kernel on the headless GPU; `tile_px` erodes the CPU field tile by tile (exact, a tile's scratch) for very large worlds",
        vec![port("in", PortType::Heightfield, true), port("hardness", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let tp: ThermalParams = params(p)?;
            if !(1..=1000).contains(&tp.iterations) {
                return Err(JdError::invalid("iterations", "must be within 1..=1000"));
            }
            let hardness = i.get("hardness").map(|v| v.as_field());
            let eroded = match tp.engine {
                Engine::Cpu if tp.tile_px > 0 => jd_terrain::thermal_erode_tiled(
                    field(i, "in")?,
                    spacing_m(ctx), tp.talus_deg, tp.iterations, tp.rate, hardness.map(|k| &**k), tp.tile_px, Some(&ctx.cancel))?,
                Engine::Cpu => jd_terrain::thermal_erode_with(
                    field(i, "in")?,
                    spacing_m(ctx), tp.talus_deg, tp.iterations, tp.rate, hardness.map(|k| &**k), Some(&ctx.cancel))?,
                Engine::Gpu => {
                    let gpu = gpu()?;
                    gpu.thermal_erode(field(i, "in")?, spacing_m(ctx), tp.talus_deg, tp.iterations, tp.rate, hardness.map(|k| &**k), Some(&ctx.cancel))?
                }
            };
            Ok(out("out", eroded))
        },
    ).with_memory_fn(|p| if p.get("tile_px").and_then(|v| v.as_u64()).unwrap_or(0) > 0 { 0.5 } else { 8.0 }).local(|p| p.get("iterations").and_then(|v| v.as_u64()).map(|v| v as u32).unwrap_or_else(iterations_default)));
    r.register(NodeDef::new::<DropletParams, _>(
        "jd.erosion.droplet@1",
        "Particle (droplet) hydraulic erosion after Beyer: seeded droplets follow the slope with inertia, erode through a brush and deposit as they slow — gullies and fans; outputs the terrain and the signed `delta`",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false), port("delta", PortType::Heightfield, false)],
        |ctx, p, i| {
            let dp: DropletParams = params(p)?;
            let r = droplet_erode(field(i, "in")?, spacing_m(ctx), &dp, Some(&ctx.cancel))?;
            Ok(BTreeMap::from([("out".to_owned(), Value::field(r.terrain)), ("delta".to_owned(), Value::field(r.delta))]))
        },
    ).with_memory(8.0));
    r.register(NodeDef::new::<StreamPowerParams, _>(
        "jd.erosion.stream_power@1",
        "Fluvial (stream-power) erosion: channels incise in proportion to drainage area^m × slope, implicit and unconditionally stable; outputs the eroded terrain and an `incision` depth map; an optional `hardness` field (0..1) drops the erodibility to a tenth on hard rock",
        vec![port("in", PortType::Heightfield, true), port("hardness", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false), port("incision", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: StreamPowerParams = params(p)?;
            let input = field(i, "in")?;
            let hardness = i.get("hardness").map(|v| v.as_field());
            let eroded = jd_terrain::stream_power_erode_with(input, spacing_m(ctx), &sp, hardness.map(|k| &**k), Some(&ctx.cancel))?;
            let mut incision = eroded.clone();
            incision.fill_with(|x, y| (input.get(x, y) - eroded.get(x, y)).max(0.0));
            Ok(BTreeMap::from([("out".to_owned(), Value::field(eroded)), ("incision".to_owned(), Value::field(incision))]))
        },
    ).with_memory(28.0));
    r.register(NodeDef::new::<TerraceParams, _>(
        "jd.filter.terrace@1",
        "Terraces: quantise heights into steps of `step_m` with a soft riser — mesas, rice paddies, strata",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |_, p, i| {
            let tp: TerraceParams = params(p)?;
            if !(tp.step_m > 0.0 && tp.step_m.is_finite()) {
                return Err(JdError::invalid("step_m", "must be positive"));
            }
            if !(0.0..=1.0).contains(&tp.softness) {
                return Err(JdError::invalid("softness", "must be within 0..=1"));
            }
            if tp.softness >= 1.0 {
                return Ok(out("out", (**field(i, "in")?).clone()));
            }
            Ok(out("out", map1(field(i, "in")?, |h| terrace(h, tp.step_m, tp.softness, tp.offset_m))?))
        },
    )
    // Local: the kernel reads `radius` cells around a cell, so a change re-runs on its tiles only.
    .local(|p| p.get("radius").and_then(|v| v.as_u64()).unwrap_or(1) as u32));
    r.register(NodeDef::new::<GeomorphonParams, _>(
        "jd.analysis.geomorphon@1",
        "Landform class per cell (geomorphons, Jasiewicz & Stepinski 2013): 1 flat, 2 peak, 3 ridge, 4 shoulder, 5 spur, 6 slope, 7 hollow, 8 footslope, 9 valley, 10 pit (0 on the border) from lines of sight up to `search_m` along the 8 compass directions — feed `jd.mask.class@1` for ridge, valley or footslope masks",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let gp: GeomorphonParams = params(p)?;
            let spacing = spacing_m(ctx);
            let search = if gp.search_m > 0.0 { gp.search_m } else { 20.0 * spacing[0].max(spacing[1]) };
            Ok(out("out", jd_terrain::geomorphons(field(i, "in")?, spacing, search, gp.flat_deg)?))
        },
    ).with_memory(8.0));
    r.register(NodeDef::new::<CurvatureParams, _>(
        "jd.analysis.curvature@1",
        "Curvature (Laplacian of height, 1/m) mapped to −1..1: positive on ridges and convex breaks, negative in valleys and hollows — for rock exposure, soil and sediment masks",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let cp: CurvatureParams = params(p)?;
            if !(cp.scale > 0.0 && cp.scale.is_finite()) {
                return Err(JdError::invalid("scale", "must be positive"));
            }
            if !(1..=16).contains(&cp.radius_px) {
                return Err(JdError::invalid("radius_px", "must be within 1..=16"));
            }
            let h = field(i, "in")?;
            let [sx, sy] = spacing_m(ctx);
            let r = cp.radius_px as i64;
            let (dx, dy) = (sx * r as f32, sy * r as f32);
            let (w, hh) = (h.width(), h.height());
            let mut o = Heightfield::new(w, hh, h.tile_size(), h.halo())?;
            o.fill_with(|x, y| {
                let g = |xx: i64, yy: i64| h.get(xx.clamp(0, w as i64 - 1) as u32, yy.clamp(0, hh as i64 - 1) as u32);
                let (x, y) = (x as i64, y as i64);
                // f64 for the second differences: f32 cancellation on ~500 m heights leaves 1e-3 noise.
                let c = g(x, y) as f64;
                let d2x = (g(x + r, y) as f64 - 2.0 * c + g(x - r, y) as f64) / (dx as f64 * dx as f64);
                let d2y = (g(x, y + r) as f64 - 2.0 * c + g(x, y - r) as f64) / (dy as f64 * dy as f64);
                // Convex (ridge) = height falls away on both sides = negative Laplacian → +1.
                (-(d2x + d2y) / cp.scale as f64).clamp(-1.0, 1.0) as f32
            });
            Ok(out("out", o))
        },
    )
    // Local: the kernel reads `radius` cells around a cell, so a change re-runs on its tiles only.
    .local(|p| p.get("radius_px").and_then(|v| v.as_u64()).unwrap_or(1) as u32));
    r.register(NodeDef::new::<AeolianParams, _>(
        "jd.erosion.aeolian@1",
        "Wind (aeolian) transport after Werner's slab model: sand hops downwind, stops in wind shadows and on sand, and the angle of repose holds — dunes and ripples; optional `sand` mask (0..1) says what can move, optional `vegetation` mask (0..1) holds it (stabilised and parabolic dunes); a second wind (`wind2_azimuth_deg`, `wind2_share`) makes linear and star dunes",
        vec![port("in", PortType::Heightfield, true), port("sand", PortType::Heightfield, false), port("vegetation", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let ap: AeolianParams = params(&with_world_wind(ctx, p, &["wind_azimuth_deg"]))?;
            if !(1..=500).contains(&ap.iterations) {
                return Err(JdError::invalid("iterations", "must be within 1..=500"));
            }
            let sand = i.get("sand").map(|v| v.as_field());
            let vegetation = i.get("vegetation").map(|v| v.as_field());
            Ok(out("out", aeolian_transport_with(field(i, "in")?, spacing_m(ctx), sand.map(|s| &**s), vegetation.map(|v| &**v), &ap, Some(&ctx.cancel))?))
        },
    ).with_memory(12.0));
    r.register(NodeDef::new::<CoastalParams, _>(
        "jd.erosion.coastal@1",
        "Coastal erosion: waves cut the shore within `band_m` above `level_m` by its ocean exposure and lay the material on the adjacent sea floor — cliff retreat, wave-cut platforms, beaches; `drift` > 0 adds fetch (windward shores erode more) and longshore drift (deposits move downwind: spits, bars) from the wind; outputs the terrain and a `shore` mask of wave work; an optional `exposure` field (the sea-level node's wave exposure, 0..2) scales the wave work so sheltered bays keep their shore; `budget_m3` caps the volume the waves move over the whole run",
        vec![port("in", PortType::Heightfield, true), port("exposure", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false), port("shore", PortType::Heightfield, false)],
        |ctx, p, i| {
            let cp: CoastalParams = params(&with_world_wind(ctx, p, &["wind_azimuth_deg"]))?;
            if !(1..=1000).contains(&cp.iterations) {
                return Err(JdError::invalid("iterations", "must be within 1..=1000"));
            }
            let energy = i.get("exposure").map(|v| v.as_field());
            let (out_h, shore) = jd_terrain::coastal_erode_budget(field(i, "in")?, spacing_m(ctx), &cp, energy.map(|e| &**e), Some(&ctx.cancel))?;
            Ok(BTreeMap::from([("out".to_owned(), Value::field(out_h)), ("shore".to_owned(), Value::field(shore))]))
        },
    ).with_memory(16.0));
    r.register(NodeDef::new::<SnowParams, _>(
        "jd.climate.snow@1",
        "Snow cover 0..1 from height alone: a soft snow line, retention that falls with slope, sun-facing slopes melt, windward slopes scour — for biome rules, scatter and colour overlays",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("snow", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: SnowParams = params(&with_world_wind(ctx, p, &["wind_azimuth_deg"]))?;
            Ok(out("snow", snow_cover(field(i, "in")?, spacing_m(ctx), &sp)?))
        },
    ));
    r.register(NodeDef::new::<jd_terrain::SnowpackParams, _>(
        "jd.climate.snowpack@1",
        "Snowpack over `steps`: snow falls where it is cold enough and holds on gentle ground, a degree-day melt follows the lapse rate and the sun, wind drifts it to the lee, slopes above `avalanche_slope_deg` slide a share downhill each step — `depth` in metres of snow and `cover` 0..1; key `temperature_c` over the timeline for a season",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("depth", PortType::Heightfield, false), port("cover", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: jd_terrain::SnowpackParams = params(&with_world_wind(ctx, p, &["wind_azimuth_deg"]))?;
            let (depth, cover) = jd_terrain::snowpack(field(i, "in")?, spacing_m(ctx), &sp)?;
            Ok(BTreeMap::from([("depth".to_owned(), Value::field(depth)), ("cover".to_owned(), Value::field(cover))]))
        },
    ).with_memory(16.0));
    r.register(NodeDef::new::<jd_terrain::RainParams, _>(
        "jd.climate.rain@1",
        "Orographic rain: moisture rides the wind across the field; air holds less water the higher it climbs (`moisture_scale_m`), so windward slopes condense the excess as rain, descending air rains nothing (the shadow), flat ground takes a drizzle (`base_per_km`) and dry air re-moistens over `recovery_km`, and with `delay_km` the condensed water is carried that far downwind before it falls (onto the crest and over it) — `rain` 0..1 (the field's 99th percentile at 1; feed `jd.erosion.hydraulic@1`'s `rain` or a biome rule), `snowfall` (the share that falls where the lapsed `temperature_c` is below `snow_below_c`; key the temperature for a season) and `moisture` 0..1, the relative humidity of the air leaving each cell",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("rain", PortType::Heightfield, false), port("snowfall", PortType::Heightfield, false), port("moisture", PortType::Heightfield, false)],
        |ctx, p, i| {
            let rp: jd_terrain::RainParams = params(&with_world_wind(ctx, p, &["wind_azimuth_deg"]))?;
            let (rain, snowfall, moisture) = jd_terrain::orographic_rain(field(i, "in")?, spacing_m(ctx), &rp)?;
            Ok(BTreeMap::from([("rain".to_owned(), Value::field(rain)), ("snowfall".to_owned(), Value::field(snowfall)), ("moisture".to_owned(), Value::field(moisture))]))
        },
    )
    // Scratch: rain, vapour, cloud and humidity fields plus the percentile copy — 20 B/cell.
    .with_memory(20.0));
    r.register(NodeDef::new::<SlopeParams, _>(
        "jd.analysis.slope@1",
        "Slope mask 0..1 (gradient angle / max_deg) — feed into multiply to attenuate by steepness",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: SlopeParams = params(p)?;
            Ok(out(
                "out",
                slope_mask(field(i, "in")?, spacing_m(ctx), sp.max_deg)?,
            ))
        },
    ));
    r.register(NodeDef::new::<HydraulicNodeParams, _>(
        "jd.erosion.hydraulic@1",
        "Hydraulic erosion (virtual pipes / shallow water): rain flows downhill, dissolves, carries and deposits sediment; outputs terrain, water depth and suspended sediment; an optional `hardness` field (0..1) drops the dissolving rate to a tenth on hard rock; an optional `rain` field (0..1, e.g. `jd.climate.rain@1`) scales `rain_rate` per cell; an optional `drains` mask (> 0.5, e.g. a sea or river-mouth mask) lets water and its sediment leave the field there; an optional `soil` field (metres of loose material on the rock) is stripped at the full rate before the rock underneath and receives every deposit, and comes back as the `soil` output; `engine: gpu` runs the same passes on the headless GPU",
        vec![port("in", PortType::Heightfield, true), port("hardness", PortType::Heightfield, false), port("rain", PortType::Heightfield, false), port("drains", PortType::Heightfield, false), port("soil", PortType::Heightfield, false)],
        vec![
            port("out", PortType::Heightfield, false),
            port("water", PortType::Heightfield, false),
            port("sediment", PortType::Heightfield, false),
            port("soil", PortType::Heightfield, false),
        ],
        |ctx, p, i| {
            let hp: HydraulicNodeParams = params(p)?;
            if !(1..=2000).contains(&hp.iterations) {
                return Err(JdError::invalid("iterations", "must be within 1..=2000"));
            }
            let hardness = i.get("hardness").map(|v| v.as_field());
            let rain = i.get("rain").map(|v| v.as_field());
            let drains = i.get("drains").map(|v| v.as_field());
            let soil = i.get("soil").map(|v| v.as_field());
            let hp2 = HydraulicParams {
                    iterations: hp.iterations,
                    dt: hp.dt,
                    rain_rate: hp.rain_rate,
                    evaporation: hp.evaporation,
                    capacity_k: hp.capacity_k,
                    dissolve_k: hp.dissolve_k,
                    deposit_k: hp.deposit_k,
                    max_concentration: hp.max_concentration,
                    ..Default::default()
                };
            let r = match hp.engine {
                Engine::Cpu => jd_terrain::hydraulic_erode_layered(field(i, "in")?, spacing_m(ctx), &hp2, hardness.map(|k| &**k), rain.map(|k| &**k), drains.map(|k| &**k), soil.map(|k| &**k), Some(&ctx.cancel))?,
                Engine::Gpu => gpu()?.hydraulic_erode_layered(field(i, "in")?, spacing_m(ctx), &hp2, hardness.map(|k| &**k), rain.map(|k| &**k), drains.map(|k| &**k), soil.map(|k| &**k), Some(&ctx.cancel))?,
            };
            Ok(BTreeMap::from([
                ("out".to_owned(), Value::field(r.height)),
                ("water".to_owned(), Value::field(r.water)),
                ("sediment".to_owned(), Value::field(r.sediment)),
                ("soil".to_owned(), Value::field(r.soil)),
            ]))
        },
    ).with_memory(48.0));
    r.register(NodeDef::new::<SculptParams, _>(
        "jd.sculpt.strokes@1",
        "Brush strokes recorded as data — raise/lower, smooth, flatten (to the height under the first point or `target_m`), with a soft or hard (`hardness`) edge, round along the polyline or `square` stamps at each point (grid selections): non-destructive, replayable, undo removes a whole stroke",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: SculptParams = params(p)?;
            let painted = apply_strokes(field(i, "in")?, spacing_m(ctx), &sp.strokes)?;
            let Some([lo, hi]) = sp.clamp else {
                return Ok(out("out", painted));
            };
            if !(lo <= hi && lo.is_finite() && hi.is_finite()) {
                return Err(JdError::invalid("clamp", "must be [min, max] with min ≤ max"));
            }
            let mut clamped = painted.clone();
            clamped.fill_with(|x, y| painted.get(x, y).clamp(lo, hi));
            Ok(out("out", clamped))
        },
    ).with_memory(12.0));
    r.register(NodeDef::new::<FlowParams, _>(
        "jd.analysis.flow@1",
        "D8 flow accumulation: upstream drainage area per cell (log-scaled 0..1 mask by default) — rivers, wetness, erosion seeding",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let fp: FlowParams = params(p)?;
            Ok(out("out", flow_accumulation_with(field(i, "in")?, spacing_m(ctx), fp.log_scale, fp.method)?))
        },
    ).with_memory(20.0));
    r.register(NodeDef::new::<InputParams, _>(
        "jd.input.heightmap@1",
        "Load a heightmap from the project's assets/ (PNG 16-bit, EXR, TIFF, RAW, or a binary FBX mesh rasterised over its footprint) and resample it onto the world grid — a GeoTIFF with a tie point is placed by it and reprojected when its EPSG code differs from the world's CRS (`georeference: false` stretches instead)",
        vec![],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, _| {
            let ip: InputParams = params(p)?;
            let rel = std::path::Path::new(&ip.path);
            if rel.is_absolute() || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                return Err(JdError::invalid("path", "must be a plain relative path inside assets/")
                    .next("Copy the file into the project's assets/ directory (project.import_asset) and reference it by name."));
            }
            let Some(dir) = &ctx.assets_dir else {
                return Err(JdError::invalid("path", "no project is open, so there is no assets/ directory to read from"));
            };
            // Resolve symlinks and make sure the real file still lives under assets/.
            let file = std::fs::canonicalize(dir.join(rel)).map_err(|e| JdError::invalid("path", format!("cannot read `{}`: {e}", ip.path)))?;
            let base = std::fs::canonicalize(dir).map_err(|e| JdError::invalid("path", format!("assets directory unavailable: {e}")))?;
            if !file.starts_with(&base) {
                return Err(JdError::invalid("path", "file resolves outside the project's assets/ directory"));
            }
            let (data, w, h, tags) = jd_terrain::read_foreign_with_tags(&file, &ip.import)?;
            // A GeoTIFF that says where it is goes where it says (reprojected when its CRS differs
            // from the world's); everything else is stretched over the world.
            let placed = match tags {
                Some(tags) if ip.import.georeference && tags.pixel_scale.is_some() && tags.tiepoint.is_some() => {
                    jd_terrain::resample_to_world_geo(&data, w, h, &tags, &ctx.lod_world)?
                }
                _ => resample_to_world(&data, w, h, ip.import.north_up, &ctx.lod_world)?,
            };
            Ok(out("out", placed))
        },
    ));
    r.register(NodeDef::new::<FillParams, _>(
        "jd.hydrology.fill@1",
        "Priority-flood depression filling: raises pits to their spill level so every cell drains; `lakes` = water depth of the filled basins. Optional `drains` mask (> 0.5): underground outlets such as karst sinkholes — no lake forms there and nothing spills past them",
        vec![port("in", PortType::Heightfield, true), port("drains", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false), port("lakes", PortType::Heightfield, false)],
        |_, p, i| {
            let fp: FillParams = params(p)?;
            let drains = i.get("drains").map(|v| v.as_field());
            let (filled, lakes) = fill_depressions_with(field(i, "in")?, fp.epsilon_m, drains.map(|d| &**d))?;
            Ok(BTreeMap::from([("out".to_owned(), Value::field(filled)), ("lakes".to_owned(), Value::field(lakes))]))
        },
    ).with_memory(24.0));
    r.register(NodeDef::new::<BiomeParams, _>(
        "jd.biome.classify@1",
        "Classify every cell into a biome by ordered rules on height, slope (0..1), wetness (0..1) and an optional `mask` field (e.g. `lakes` depth → salt crust); outputs the class id map (1-based rule index, 0 = none)",
        vec![
            port("height", PortType::Heightfield, true),
            port("slope", PortType::Heightfield, false),
            port("wetness", PortType::Heightfield, false),
            port("mask", PortType::Heightfield, false),
        ],
        vec![port("biome", PortType::Heightfield, false)],
        |_, p, i| {
            let bp: BiomeParams = params(p)?;
            validate_biome_rules(&bp)?;
            let rules = if bp.rules.is_empty() { default_biome_rules() } else { bp.rules };
            let h = field(i, "height")?;
            let slope = i.get("slope").map(|v| v.as_field());
            let wet = i.get("wetness").map(|v| v.as_field());
            let mask = i.get("mask").map(|v| v.as_field());
            for (name, f) in [("slope", slope), ("wetness", wet), ("mask", mask)] {
                if let Some(f) = f
                    && (f.width(), f.height()) != (h.width(), h.height())
                {
                    return Err(JdError::invalid(name, "field sizes differ from `height`"));
                }
            }
            let within = |v: f32, r: [f32; 2]| v >= r[0] && v < r[1];
            let mut out_f = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
            out_f.fill_with(|x, y| {
                let hv = h.get(x, y);
                for (k, rule) in rules.iter().enumerate() {
                    let ok_h = rule.height_m.is_none_or(|r| within(hv, r));
                    let ok_s = match (rule.slope, slope) {
                        (None, _) => true,
                        (Some(r), Some(f)) => within(f.get(x, y), r),
                        (Some(_), None) => false,
                    };
                    let band = |want: Option<[f32; 2]>, f: Option<&Arc<Heightfield>>| match (want, f) {
                        (None, _) => true,
                        (Some(r), Some(f)) => within(f.get(x, y), r),
                        (Some(_), None) => false,
                    };
                    if ok_h && ok_s && band(rule.wetness, wet) && band(rule.mask, mask) {
                        return (k + 1) as f32;
                    }
                }
                0.0
            });
            Ok(out("biome", out_f))
        },
    ).with_check(validate_biome_rules));
    r.register(NodeDef::new::<SeabedParams, _>(
        "jd.biome.seabed@1",
        "Classify the sea floor below `level_m`: 1 sand on the shelf above the wave base, 2 rock on steep slopes, 3 mud in deep still water, 4 reef on a shallow gentle shelf away from river sediment (optional `rivers` and `shore` masks); 0 = land; an optional `exposure` field (the sea-level node's wave exposure) lets mud settle sooner in sheltered water; an optional `current` field (the sea-level node's drift direction) carries the river and shore sediment downstream — the reach stretches with the current and shrinks against it",
        vec![
            port("in", PortType::Heightfield, true),
            port("rivers", PortType::Heightfield, false),
            port("shore", PortType::Heightfield, false),
            port("exposure", PortType::Heightfield, false),
            port("current", PortType::Heightfield, false),
        ],
        vec![port("seabed", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: SeabedParams = params(p)?;
            let rivers = i.get("rivers").map(|v| v.as_field());
            let shore = i.get("shore").map(|v| v.as_field());
            let exposure = i.get("exposure").map(|v| v.as_field());
            let current = i.get("current").map(|v| v.as_field());
            Ok(out("seabed", jd_terrain::classify_seabed_current(field(i, "in")?, spacing_m(ctx), rivers.map(|r| &**r), shore.map(|s| &**s), exposure.map(|e| &**e), current.map(|c| &**c), &sp)?))
        },
    ).with_memory(8.0));
    r.register(NodeDef::new::<jd_terrain::StrataParams, _>(
        "jd.terrain.strata@1",
        "Layered rock hardness (0..1): repeating hard-cap / soft-bed cycles of `layer_m`, tilted by `dip_deg` towards `dip_azimuth_deg`, bed surfaces undulating by `warp_m` — feed it to the `hardness` input of thermal and stream-power erosion for mesas, buttes and stepped canyon walls; re-evaluate after the surface moves",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("hardness", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: jd_terrain::StrataParams = params(p)?;
            Ok(out("hardness", jd_terrain::strata_hardness(field(i, "in")?, spacing_m(ctx), &sp)?))
        },
    ).with_memory(4.0));
    r.register(NodeDef::new::<jd_terrain::YardangParams, _>(
        "jd.erosion.yardang@1",
        "Wind abrasion: sand-laden wind from `wind_azimuth_deg` sandblasts the faces that meet it and spares the sheltered lee, streamlining ridges into keels along the wind — yardangs; optional `hardness` lets caprock keels survive; mass leaves the field as dust",
        vec![port("in", PortType::Heightfield, true), port("hardness", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let yp: jd_terrain::YardangParams = params(&with_world_wind(ctx, p, &["wind_azimuth_deg"]))?;
            let hardness = i.get("hardness").map(|v| v.as_field());
            Ok(out("out", jd_terrain::wind_abrasion(field(i, "in")?, spacing_m(ctx), hardness.map(|k| &**k), &yp, Some(&ctx.cancel))?))
        },
    ).with_memory(8.0));
    r.register(NodeDef::new::<jd_terrain::DebrisParams, _>(
        "jd.erosion.debris@1",
        "Rockfall debris: cliffs steeper than `source_slope_deg` shed `release_m` of rock as seeded blocks that roll down the fall line until the ground is gentler than the angle of repose and the energy line back to the cliff is flatter than `reach_deg` (Fahrböschung) — talus cones and boulder fields at the slope foot, big blocks running furthest (`sorting`); optional 0..1 `source` mask; outputs the terrain, a 0..1 `debris` cover mask for boulder scatter and the mean block `size` in metres",
        vec![port("in", PortType::Heightfield, true), port("source", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false), port("debris", PortType::Heightfield, false), port("size", PortType::Heightfield, false)],
        |ctx, p, i| {
            let dp: jd_terrain::DebrisParams = params(p)?;
            let source = i.get("source").map(|v| v.as_field());
            let r = jd_terrain::rockfall(field(i, "in")?, spacing_m(ctx), source.map(|s| &**s), &dp, Some(&ctx.cancel))?;
            Ok(BTreeMap::from([
                ("out".to_owned(), Value::field(r.out)),
                ("debris".to_owned(), Value::field(r.debris)),
                ("size".to_owned(), Value::field(r.size)),
            ]))
        },
    // Scratch: surface, block count, size sum, source mask (16 B/cell), the source list (≤ 12 B/cell)
    // and the three outputs (12 B/cell).
    ).with_memory(40.0));
    r.register(NodeDef::new::<jd_terrain::LavaParams, _>(
        "jd.volcanic.lava@1",
        "Lava flows (MAGFLOW / GPUFLOW cellular automaton): `vents` (layer metres, m³/s; empty = one summit vent) effuse for `eruption_s`; the lava spreads as a Bingham fluid whose viscosity and yield strength rise as it cools by radiation, and freezes into new ground below `solid_temp_k`; `duration_s` of simulated time (keyable on the timeline). Outputs the terrain with the lava on it, the `lava` thickness in metres and a 0..1 `heat` glow of the still-molten lava for emissive materials",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false), port("lava", PortType::Heightfield, false), port("heat", PortType::Heightfield, false)],
        |ctx, p, i| {
            let lp: jd_terrain::LavaParams = params(p)?;
            let r = jd_terrain::lava_flow(field(i, "in")?, spacing_m(ctx), &lp, Some(&ctx.cancel))?;
            Ok(BTreeMap::from([
                ("out".to_owned(), Value::field(r.out)),
                ("lava".to_owned(), Value::field(r.lava)),
                ("heat".to_owned(), Value::field(r.heat)),
            ]))
        },
    // Scratch: terrain, ground, thickness and temperature twice (24 B/cell), eight fluxes (32 B/cell)
    // and the three outputs (12 B/cell).
    ).with_memory(68.0));
    r.register(NodeDef::new::<jd_terrain::SplineParams, _>(
        "jd.terrain.spline@1",
        "A road, path, canal or dyke along a polyline (`points` in layer metres): the corridor of `width_m` is flattened onto the line's smoothed grade, cut `profile_m` below it or raised above it, with a `falloff_m` shoulder; outputs the terrain and a 0..1 corridor `mask` for materials and scatter",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false), port("mask", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: jd_terrain::SplineParams = params(p)?;
            let (out_h, mask) = jd_terrain::apply_spline(field(i, "in")?, spacing_m(ctx), &sp)?;
            Ok(BTreeMap::from([("out".to_owned(), Value::field(out_h)), ("mask".to_owned(), Value::field(mask))]))
        },
    ).with_memory(12.0));
    r.register(NodeDef::new::<jd_terrain::KarstParams, _>(
        "jd.erosion.karst@1",
        "Karst sinkholes (dolines): seeded bowl or collapse depressions on flat, soluble ground — `density_per_km2`, radius and depth ranges, `max_slope_deg`, `min_spacing_m`; optional `soluble` mask (0..1); outputs the carved terrain and a `sinkholes` floor mask",
        vec![
            port("in", PortType::Heightfield, true),
            port("soluble", PortType::Heightfield, false),
        ],
        vec![port("out", PortType::Heightfield, false), port("sinkholes", PortType::Heightfield, false)],
        |ctx, p, i| {
            let kp: jd_terrain::KarstParams = params(p)?;
            let soluble = i.get("soluble").map(|v| v.as_field());
            let (out_h, mask, _) = jd_terrain::karst_sinkholes(field(i, "in")?, spacing_m(ctx), soluble.map(|s| &**s), &kp)?;
            Ok(BTreeMap::from([("out".to_owned(), Value::field(out_h)), ("sinkholes".to_owned(), Value::field(mask))]))
        },
    ).with_memory(16.0));
    r.register(NodeDef::new::<jd_terrain::BathymetryParams, _>(
        "jd.ocean.bathymetry@1",
        "Sediment infill of the sea floor: sand and mud cells (from a `seabed` class map) rise towards the local mean over `smooth_m`, capped at `max_fill_m` and `level_m`; rock, reef and land are untouched. An optional `supply` mask (the coastal node's `shore`, a river mask, their sum) makes the sediment come from those cells and thin away from them over `reach_m`; `budget_m3` caps the volume laid down",
        vec![
            port("in", PortType::Heightfield, true),
            port("seabed", PortType::Heightfield, true),
            port("supply", PortType::Heightfield, false),
        ],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let bp: jd_terrain::BathymetryParams = params(p)?;
            let supply = i.get("supply").map(|v| v.as_field());
            Ok(out("out", jd_terrain::bathymetry_with(field(i, "in")?, spacing_m(ctx), field(i, "seabed")?, supply.map(|s| &**s), &bp, Some(&ctx.cancel))?))
        },
    // Scratch: the f64 summed-area table + mean + fill (16 B/cell) plus, with `supply` wired, the
    // mask, classes, distance, weight and heap (≈ 28 B/cell); the estimate cannot see the port, so
    // it charges the supplied case.
    ).with_memory(44.0));
    r.register(
        NodeDef::new::<ClassMaskParams, _>(
            "jd.mask.class@1",
            "0/1 mask of the cells whose class id equals `id` (feed a `jd.biome.classify@1` map)",
            vec![port("classes", PortType::Heightfield, true)],
            vec![port("out", PortType::Heightfield, false)],
            |_, p, i| {
                let cp: ClassMaskParams = params(p)?;
                let id = cp.id as f32;
                Ok(out(
                    "out",
                    map1(field(i, "classes")?, |v| {
                        if (v - id).abs() < 0.5 { 1.0 } else { 0.0 }
                    })?,
                ))
            },
        )
        .local(|p| {
            let _ = p;
            0
        }),
    );
    r.register(NodeDef::new::<SeaLevelParams, _>(
        "jd.hydrology.sea_level@1",
        "Ocean mask (1 below `level_m` where connected to the border; inland basins stay dry) with a soft shoreline, water `depth` in metres, and the wave `exposure` 0..1 of every water cell — the open-water fetch upwind of it (the wind and `fetch_km` of the node or the world), low in the lee of islands and headlands; with `refraction` the waves bend over the shelf and the exposure is focused on headlands (up to 2, which the coastal and seabed nodes honour) and spread in bays; `current` is the surface drift's direction (degrees clockwise from north the water flows towards): the wind offshore, bent alongshore within `current_band_m` of land — feed `jd.biome.seabed@1` so sediment travels downstream",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("ocean", PortType::Heightfield, false), port("depth", PortType::Heightfield, false), port("exposure", PortType::Heightfield, false), port("current", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: SeaLevelParams = params(&with_world_wind(ctx, p, &["wind_azimuth_deg", "wind_speed_mps"]))?;
            if !(sp.visibility_m > 0.0 && sp.visibility_m.is_finite()) {
                return Err(JdError::invalid("visibility_m", "must be positive"));
            }
            if !(sp.current_band_m > 0.0 && sp.current_band_m.is_finite()) {
                return Err(JdError::invalid("current_band_m", "must be positive"));
            }
            if sp.tint.iter().any(|c| !(0.0..=1.0).contains(c)) {
                return Err(JdError::invalid("tint", "components must be within 0..=1"));
            }
            // The wind sea is drawn, not evaluated; validate it here so a bad edit fails at the node.
            let sea = jd_terrain::wave_spectrum(&jd_terrain::WaveParams {
                wind_speed_mps: sp.wind_speed_mps,
                fetch_km: sp.fetch_km,
                wind_azimuth_deg: sp.wind_azimuth_deg,
            })?;
            let (ocean, depth) = sea_level(field(i, "in")?, sp.level_m, sp.shore_m)?;
            let mut exposure = jd_terrain::wave_exposure(&ocean, spacing_m(ctx), sp.wind_azimuth_deg, sp.fetch_km.unwrap_or(50.0))?; // no fetch limit set: a 50 km open-coast scale
            if sp.refraction {
                let focus = jd_terrain::wave_refraction(&ocean, &depth, spacing_m(ctx), sp.wind_azimuth_deg, sea.wavelength_m, Some(&ctx.cancel))?;
                exposure = map2(&exposure, &focus, |e, k| e * k)?;
            }
            let current = jd_terrain::surface_current(&ocean, spacing_m(ctx), sp.wind_azimuth_deg, sp.current_band_m, Some(&ctx.cancel))?;
            Ok(BTreeMap::from([("ocean".to_owned(), Value::field(ocean)), ("depth".to_owned(), Value::field(depth)), ("exposure".to_owned(), Value::field(exposure)), ("current".to_owned(), Value::field(current))]))
        },
    )
    // Scratch: the flood (height copy, sea flags, reach) ≈ 9 B/cell plus the current's distance
    // to land, water flags and heap ≈ 13; the refraction march adds arrival time, slowness, flags,
    // ray direction, spacing integral, order and its heap ≈ 25 B/cell (its own scratch is freed first).
    .with_memory_fn(|p| if p.get("refraction").and_then(|v| v.as_bool()).unwrap_or(false) { 36.0 } else { 22.0 }));
    r.register(NodeDef::new::<RiversParams, _>(
        "jd.hydrology.rivers@1",
        "Carve river channels where flow accumulates (depressions filled first), plus any `springs` you place (points that pour `spring_area_m2` of catchment in, so a river starts there); outputs the carved terrain and a 0..1 river mask",
        vec![port("in", PortType::Heightfield, true), port("drains", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false), port("rivers", PortType::Heightfield, false)],
        |ctx, p, i| {
            let rp: RiversParams = params(p)?;
            let drains = i.get("drains").map(|v| v.as_field());
            let h = field(i, "in")?;
            let sp = spacing_m(ctx);
            let springs = if rp.springs.is_empty() {
                None
            } else {
                if !(rp.spring_area_m2 > 0.0 && rp.spring_area_m2.is_finite()) {
                    return Err(JdError::invalid("spring_area_m2", "must be positive"));
                }
                let cells = rp.spring_area_m2 / (sp[0] * sp[1]);
                let mut src = Heightfield::new(h.width(), h.height(), h.tile_size(), h.halo())?;
                src.fill_with(|_, _| 0.0);
                for [u, v] in &rp.springs {
                    if !(u.is_finite() && v.is_finite()) {
                        return Err(JdError::invalid("springs", "spring positions must be finite"));
                    }
                    let x = ((u / sp[0]).floor().max(0.0) as u32).min(h.width() - 1);
                    let y = ((v / sp[1]).floor().max(0.0) as u32).min(h.height() - 1);
                    src.set(x, y, src.get(x, y) + cells);
                }
                Some(src)
            };
            let (carved, rivers) = jd_terrain::carve_rivers_sourced(h, sp, rp.threshold, rp.depth_m, rp.method, drains.map(|d| &**d), springs.as_ref())?;
            Ok(BTreeMap::from([("out".to_owned(), Value::field(carved)), ("rivers".to_owned(), Value::field(rivers))]))
        },
    ).with_memory(32.0));
    r.register(NodeDef::new::<WetnessParams, _>(
        "jd.hydrology.wetness@1",
        "Topographic wetness index ln(a / tan β) normalised to 0..1 — wet valley floors high, steep ridges low (biome/moisture masks); `method` picks d8 / dinf / mfd routing",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let wp: WetnessParams = params(p)?;
            Ok(out("out", wetness_index_with(field(i, "in")?, spacing_m(ctx), wp.method)?))
        },
    ).with_memory(24.0));
    r.register(
        NodeDef::new::<OutputParams, _>(
            "jd.output.height@1",
            "Publishes its input as a named heightfield layer",
            vec![port("in", PortType::Heightfield, true)],
            vec![port("out", PortType::Heightfield, false)],
            |_, p, i| {
                let op: OutputParams = params(p)?;
                if op.layer.trim().is_empty() {
                    return Err(JdError::invalid("layer", "layer name must not be empty"));
                }
                Ok(BTreeMap::from([(
                    "out".to_owned(),
                    i.get("in")
                        .cloned()
                        .ok_or_else(|| JdError::invalid("in", "not connected"))?,
                )]))
            },
        )
        .aliasing_input(),
    );
}

/// Id of the starter graph's noise node.
pub const STARTER_NOISE_ID: &str = "01JDGRAPH000000000000N01SE";
/// Id of the starter graph's output node.
pub const STARTER_OUTPUT_ID: &str = "01JDGRAPH0000000000HE1GHT0";

/// The graph every new project starts with: one fBm noise node feeding a `height` output.
/// Deterministic ids so templates and tests are stable.
pub fn starter() -> crate::model::Graph {
    use crate::model::{Edge, Graph, Node, NodeKind, PortRef, UiMeta};
    use jd_types::NodeId;
    // Crockford base32 (no I, L, O, U); `starter_ids_are_stable` guards against a typo turning
    // these into random ids.
    let noise: NodeId = STARTER_NOISE_ID.parse().unwrap_or_default();
    let output: NodeId = STARTER_OUTPUT_ID.parse().unwrap_or_default();
    let mut g = Graph::default();
    g.nodes.insert(
        noise.clone(),
        Node {
            kind: NodeKind("jd.noise.fbm@1".into()),
            params: serde_json::json!({ "seed": 0, "fractal": "fbm", "feature_size_m": 1024.0, "octaves": 6, "height_range_m": [0.0, 500.0] })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            bypass: false,
            ui: UiMeta { position: [0.0, 0.0], label: Some("Noise".into()) },
        },
    );
    g.nodes.insert(
        output.clone(),
        Node {
            kind: NodeKind("jd.output.height@1".into()),
            params: serde_json::json!({ "layer": "height" })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            bypass: false,
            ui: UiMeta {
                position: [320.0, 0.0],
                label: Some("Height".into()),
            },
        },
    );
    g.edges.push(Edge {
        from: PortRef {
            node: noise,
            port: "out".into(),
        },
        to: PortRef {
            node: output,
            port: "in".into(),
        },
    });
    g
}

/// Layer name declared by an output node (`jd.output.height@1`).
pub fn output_layer_name(params: &Map<String, Json>) -> String {
    params
        .get("layer")
        .and_then(Json::as_str)
        .unwrap_or("height")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{Cache, Evaluator};
    use crate::model::{Edge, Graph, Node, NodeKind, PortRef, UiMeta};
    use jd_types::{ErrorCode, NodeId, WorldSettings};

    fn world() -> WorldSettings {
        WorldSettings {
            extent_m: [256.0, 256.0],
            resolution_px: [128, 96],
            tile_size: 64,
            halo: 4,
            ..Default::default()
        }
    }

    fn add(g: &mut Graph, kind: &str, p: Json) -> NodeId {
        let id = NodeId::new();
        g.nodes.insert(
            id.clone(),
            Node {
                kind: NodeKind(kind.into()),
                params: p.as_object().cloned().unwrap_or_default(),
                bypass: false,
                ui: UiMeta::default(),
            },
        );
        id
    }

    fn connect(g: &mut Graph, a: &NodeId, ap: &str, b: &NodeId, bp: &str) {
        g.edges.push(Edge {
            from: PortRef {
                node: a.clone(),
                port: ap.into(),
            },
            to: PortRef {
                node: b.clone(),
                port: bp.into(),
            },
        });
    }

    #[test]
    fn a_local_node_reruns_on_the_changed_tiles_only_and_matches_the_whole_run() {
        // 512 m / 256² at tile 64 (16 tiles, halo 4): noise → sculpt → blur(2) and curvature(2). A
        // single 20 m stroke in the middle of tile (0, 0) dirties that tile alone; the warm
        // evaluator re-runs blur and curvature on it (plus halo) and the result is bit-identical
        // to a cold evaluation of the same graph.

        let reg = Registry::with_builtins();
        let world = WorldSettings {
            extent_m: [512.0, 512.0],
            resolution_px: [256, 256],
            tile_size: 64,
            halo: 4,
            ..Default::default()
        };
        let build = |stroke: bool| {
            let mut g = Graph::default();
            let noise = add(
                &mut g,
                "jd.noise.fbm@1",
                serde_json::json!({ "seed": 3, "height_range_m": [0.0, 200.0], "feature_size_m": 80.0 }),
            );
            let strokes = if stroke {
                serde_json::json!({ "strokes": [{ "points": [[64.0, 64.0]], "radius_m": 20.0, "strength_m": 30.0 }] })
            } else {
                serde_json::json!({ "strokes": [] })
            };
            let sculpt = add(&mut g, "jd.sculpt.strokes@1", strokes);
            let blur = add(
                &mut g,
                "jd.filter.blur@1",
                serde_json::json!({ "radius": 2 }),
            );
            let curv = add(
                &mut g,
                "jd.analysis.curvature@1",
                serde_json::json!({ "radius_px": 2 }),
            );
            connect(&mut g, &noise, "out", &sculpt, "in");
            connect(&mut g, &sculpt, "out", &blur, "in");
            connect(&mut g, &sculpt, "out", &curv, "in");
            (g, blur, curv)
        };
        // Same node ids in both graphs (the memo is per node): edit `plain`'s sculpt params.
        let (plain, blur, curv) = build(false);
        let mut edited = plain.clone();
        for node in edited.nodes.values_mut() {
            if node.kind.0 == "jd.sculpt.strokes@1" {
                node.params = serde_json::json!({ "strokes": [{ "points": [[64.0, 64.0]], "radius_m": 20.0, "strength_m": 30.0 }] })
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
            }
        }
        let cache = Cache::default();
        let ev = Evaluator::new(&reg, &cache);
        ev.evaluate_with(&plain, &world, 0, None).expect("first");
        let (warm, report) = ev.evaluate_with(&edited, &world, 0, None).expect("warm");
        let (cold, _) = Evaluator::new(&reg, &Cache::default())
            .evaluate_with(&edited, &world, 0, None)
            .expect("cold");
        for id in [&blur, &curv] {
            let r = report.nodes.iter().find(|n| &n.node == id).expect("report");
            assert_eq!(
                r.partial_tiles,
                Some(1),
                "{}: one tile re-run, got {:?}",
                r.kind,
                r.partial_tiles
            );
            let a = warm[&(id.clone(), "out".into())].as_field().to_vec();
            let b = cold[&(id.clone(), "out".into())].as_field().to_vec();
            assert_eq!(a, b, "{}: the spliced result equals the whole run", r.kind);
        }
        // A third evaluation with nothing changed is served from the cache outright.
        let (_, again) = ev.evaluate_with(&edited, &world, 0, None).expect("again");
        assert!(again.nodes.iter().all(|n| n.cached));
    }

    #[test]
    fn inflate_grows_a_spike_and_the_median_removes_it_while_a_blur_spreads_it() {
        let mut h = Heightfield::new(32, 32, 32, 2).expect("hf");
        h.fill_with(|x, y| if (x, y) == (16, 16) { 10.0 } else { 0.0 });
        let grown = window_filter(&h, 1, |w| {
            w.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        })
        .expect("inflate");
        let high = (0..32)
            .flat_map(|y| (0..32).map(move |x| (x, y)))
            .filter(|&(x, y)| grown.get(x, y) == 10.0)
            .count();
        assert_eq!(high, 9, "a 3×3 block");
        let shrunk = window_filter(&grown, 1, |w| {
            w.iter().copied().fold(f32::INFINITY, f32::min)
        })
        .expect("deflate");
        assert_eq!(shrunk.get(16, 16), 10.0);
        assert_eq!(
            shrunk.get(15, 16),
            0.0,
            "open = inflate then deflate restores the spike"
        );
        let clean = window_filter(&h, 1, median).expect("median");
        assert_eq!(
            clean.get(16, 16),
            0.0,
            "a lone spike is noise to the median"
        );
        let blurred = box_blur(&h, 1).expect("blur");
        assert!((blurred.get(16, 16) - 10.0 / 9.0).abs() < 1e-5);
        assert!(
            window_filter(&h, 3, median).is_err(),
            "radius past the halo"
        );
    }

    #[test]
    fn curves_reshape_a_ramp() {
        let reg = Registry::with_builtins();
        let mut g = Graph::default();
        let a = add(
            &mut g,
            "jd.noise.fbm@1",
            serde_json::json!({ "seed": 1, "height_range_m": [0.0, 1.0], "feature_size_m": 64.0 }),
        );
        let sm = add(
            &mut g,
            "jd.math.curve@1",
            serde_json::json!({ "kind": "smoothstep", "edge0": 0.25, "edge1": 0.75 }),
        );
        let inv = add(
            &mut g,
            "jd.math.curve@1",
            serde_json::json!({ "kind": "invert", "edge0": 0.0, "edge1": 1.0 }),
        );
        let th = add(
            &mut g,
            "jd.math.curve@1",
            serde_json::json!({ "kind": "threshold", "edge0": 0.5, "edge1": 0.5 }),
        );
        for n in [&sm, &inv, &th] {
            connect(&mut g, &a, "out", n, "in");
        }
        let cache = Cache::default();
        let ev = Evaluator::new(&reg, &cache);
        let (outs, _) = ev.evaluate_with(&g, &world(), 0, None).expect("eval");
        let f = |id: &NodeId| outs[&(id.clone(), "out".into())].as_field().to_vec();
        let (av, smv, invv, thv) = (f(&a), f(&sm), f(&inv), f(&th));
        for i in 0..av.len() {
            let t = ((av[i] - 0.25) / 0.5).clamp(0.0, 1.0);
            assert!((smv[i] - t * t * (3.0 - 2.0 * t)).abs() < 1e-5);
            assert!((invv[i] - (1.0 - av[i])).abs() < 1e-5);
            assert_eq!(thv[i], if av[i] >= 0.5 { 1.0 } else { 0.0 });
        }
        let bad = add(
            &mut g,
            "jd.math.curve@1",
            serde_json::json!({ "kind": "gamma", "gamma": 0.0 }),
        );
        connect(&mut g, &a, "out", &bad, "in");
        assert!(ev.evaluate_with(&g, &world(), 0, None).is_err());
    }

    #[test]
    fn blend_nodes_take_the_lower_the_higher_or_a_masked_mix_scaled_by_opacity() {
        let reg = Registry::with_builtins();
        let mut g = Graph::default();
        let a = add(
            &mut g,
            "jd.noise.fbm@1",
            serde_json::json!({ "seed": 1, "height_range_m": [0.0, 100.0], "feature_size_m": 64.0 }),
        );
        // Constant fields: the noise scaled to zero, then remapped to one value.
        let zero = add(
            &mut g,
            "jd.math.scale@1",
            serde_json::json!({ "factor": 0.0 }),
        );
        connect(&mut g, &a, "out", &zero, "in");
        let b = add(
            &mut g,
            "jd.math.remap@1",
            serde_json::json!({ "from": [0.0, 1.0], "to": [50.0, 50.0] }),
        ); // a flat 50 m
        let t = add(
            &mut g,
            "jd.math.remap@1",
            serde_json::json!({ "from": [0.0, 1.0], "to": [0.25, 0.25] }),
        ); // a constant mask
        connect(&mut g, &zero, "out", &b, "in");
        connect(&mut g, &zero, "out", &t, "in");
        let lo = add(&mut g, "jd.math.min@1", serde_json::json!({}));
        let hi = add(
            &mut g,
            "jd.math.max@1",
            serde_json::json!({ "opacity": 0.5 }),
        );
        let mix = add(&mut g, "jd.math.lerp@1", serde_json::json!({}));
        let plain = add(
            &mut g,
            "jd.math.lerp@1",
            serde_json::json!({ "opacity": 0.0 }),
        );
        for (n, has_t) in [(&lo, false), (&hi, false), (&mix, true), (&plain, false)] {
            connect(&mut g, &a, "out", n, "in");
            connect(&mut g, &b, "out", n, "b");
            if has_t {
                connect(&mut g, &t, "out", n, "t");
            }
        }
        let cache = Cache::default();
        let ev = Evaluator::new(&reg, &cache);
        let (outs, _) = ev.evaluate_with(&g, &world(), 0, None).expect("eval");
        let f = |id: &NodeId| outs[&(id.clone(), "out".into())].as_field().to_vec();
        let (av, lov, hiv, mixv, plainv) = (f(&a), f(&lo), f(&hi), f(&mix), f(&plain));
        for i in 0..av.len() {
            assert!(
                (lov[i] - av[i].min(50.0)).abs() < 1e-4,
                "min at {i}: {} vs {}",
                lov[i],
                av[i]
            );
            assert!(
                (hiv[i] - (av[i] + (av[i].max(50.0) - av[i]) * 0.5)).abs() < 1e-4,
                "max at half opacity at {i}"
            );
            assert!(
                (mixv[i] - (av[i] + (50.0 - av[i]) * 0.25)).abs() < 1e-4,
                "lerp by the 0.25 mask at {i}"
            );
            assert_eq!(plainv[i], av[i], "opacity 0 passes the input through");
        }
        let bad = add(
            &mut g,
            "jd.math.min@1",
            serde_json::json!({ "opacity": 1.5 }),
        );
        connect(&mut g, &a, "out", &bad, "in");
        connect(&mut g, &b, "out", &bad, "b");
        assert!(ev.evaluate_with(&g, &world(), 0, None).is_err());
    }

    #[test]
    fn terrace_quantises_with_soft_risers_and_curvature_signs_ridges_and_valleys() {
        // Terrace: a ramp 0..100 m with 25 m steps, softness 0.2 → treads are flat, risers span 20 % of a step.
        assert_eq!(terrace(10.0, 25.0, 0.2, 0.0), 0.0);
        assert_eq!(
            terrace(19.0, 25.0, 0.2, 0.0),
            0.0,
            "tread holds until the riser starts at 80 %"
        );
        assert!(
            (terrace(22.5, 25.0, 0.2, 0.0) - 12.5).abs() < 1e-4,
            "riser midpoint sits mid-step"
        );
        assert!((terrace(25.0, 25.0, 0.2, 0.0) - 25.0).abs() < 1e-4);
        assert_eq!(terrace(60.0, 25.0, 0.0001, 0.0), 50.0, "hard steps");
        assert_eq!(
            terrace(55.0, 25.0, 0.2, 10.0),
            35.0,
            "offset shifts the step grid (steps at 10, 35, 60)"
        );
        // Curvature through the graph: a paraboloid bowl is concave (−), a dome convex (+), a plane 0.
        let reg = Registry::with_builtins();
        let run = |h: fn(u32, u32) -> f32| -> Vec<f32> {
            let mut f = Heightfield::new(128, 96, 64, 4).expect("hf");
            f.fill_with(h);
            let def = reg
                .get(&NodeKind("jd.analysis.curvature@1".into()))
                .expect("def");
            let ctx = EvalCtx {
                lod_world: world(),
                assets_dir: None,
                cancel: Default::default(),
                layers: None,
            };
            let params = serde_json::json!({ "scale": 0.01 })
                .as_object()
                .cloned()
                .unwrap_or_default();
            let outs = def
                .evaluate(
                    &ctx,
                    &params,
                    &BTreeMap::from([("in".to_owned(), Value::field(f))]),
                )
                .expect("eval");
            outs["out"].as_field().to_vec()
        };
        // world(): 256 m over 128 × 96 px → 2 m × 2.67 m cells (anisotropic on purpose). Bowl: z = 0.01·r² → Laplacian 0.04 → −0.04/0.01 = −1 (clamped) in the interior.
        let bowl = run(|x, y| {
            0.01 * (((x as f32 - 64.0) * 2.0).powi(2)
                + ((y as f32 - 48.0) * (256.0 / 96.0)).powi(2))
        });
        assert!(
            (bowl[48 * 128 + 64] + 1.0).abs() < 1e-3,
            "bowl centre is concave: {}",
            bowl[48 * 128 + 64]
        );
        let dome = run(|x, y| {
            -0.001
                * (((x as f32 - 64.0) * 2.0).powi(2) + ((y as f32 - 48.0) * (256.0 / 96.0)).powi(2))
        });
        assert!(
            (dome[48 * 128 + 64] - 0.4).abs() < 1e-3,
            "dome: Laplacian −0.004 → +0.4: {}",
            dome[48 * 128 + 64]
        );
        let plane = run(|x, y| 3.0 * x as f32 + 2.0 * y as f32);
        for y in 1..95u32 {
            for x in 1..127u32 {
                let v = plane[(y * 128 + x) as usize];
                assert!(v.abs() < 1e-4, "a plane has no curvature: ({x},{y}) = {v}");
            }
        }
    }

    #[test]
    fn biome_rules_classify_in_order_and_skip_rules_on_missing_inputs() {
        let reg = Registry::with_builtins();
        let cache = Cache::with_capacity(0);
        let mut g = Graph::default();
        let noise = add(
            &mut g,
            "jd.noise.fbm@1",
            serde_json::json!({ "seed": 3, "height_range_m": [0.0, 1200.0], "feature_size_m": 40.0 }),
        );
        let slope = add(&mut g, "jd.analysis.slope@1", serde_json::json!({}));
        let biome = add(&mut g, "jd.biome.classify@1", serde_json::json!({}));
        let rock = add(&mut g, "jd.mask.class@1", serde_json::json!({ "id": 1 }));
        connect(&mut g, &noise, "out", &slope, "in");
        connect(&mut g, &noise, "out", &biome, "height");
        connect(&mut g, &biome, "biome", &rock, "classes");
        // Without the slope input, rule 1 (rock, slope-only) can never match → no cell is class 1.
        let ev = Evaluator::new(&reg, &cache);
        let (outs, _) = ev.evaluate_with(&g, &world(), 0, None).expect("eval");
        let classes = outs[&(biome.clone(), "biome".into())].as_field().to_vec();
        assert!(
            classes.iter().all(|c| *c != 1.0),
            "rock needs the slope input"
        );
        assert!(
            classes.contains(&4.0) && classes.contains(&5.0),
            "grassland and forest present"
        );
        let heights = outs[&(noise.clone(), "out".into())].as_field().to_vec();
        for (c, h) in classes.iter().zip(&heights) {
            let expect = if *h >= 900.0 {
                2.0
            } else if *h < 5.0 {
                3.0
            } else if *h < 300.0 {
                4.0
            } else {
                5.0
            };
            assert_eq!(*c, expect, "height {h} → class {c}");
        }
        // With slope connected, steep cells become rock (class 1) and the mask picks exactly them.
        connect(&mut g, &slope, "out", &biome, "slope");
        let (outs, _) = ev.evaluate_with(&g, &world(), 0, None).expect("eval");
        let classes = outs[&(biome.clone(), "biome".into())].as_field().to_vec();
        let steep = outs[&(slope.clone(), "out".into())].as_field().to_vec();
        let mask = outs[&(rock.clone(), "out".into())].as_field().to_vec();
        assert!(
            classes.contains(&1.0),
            "some cells are steep enough for rock"
        );
        for ((c, s), m) in classes.iter().zip(&steep).zip(&mask) {
            assert_eq!(*c == 1.0, *s >= 0.6, "slope {s} → rock? {c}");
            assert_eq!(*m == 1.0, *c == 1.0);
        }
        // A `mask` band works like the others: unwired it never matches, wired it takes the cells in band.
        let salt = add(
            &mut g,
            "jd.biome.classify@1",
            serde_json::json!({ "rules": [{ "name": "salt crust", "mask": [0.6, 2.0] }, { "name": "other" }] }),
        );
        connect(&mut g, &noise, "out", &salt, "height");
        let (outs, _) = ev.evaluate_with(&g, &world(), 0, None).expect("eval");
        let c = outs[&(salt.clone(), "biome".into())].as_field().to_vec();
        assert!(c.iter().all(|c| *c == 2.0), "no mask wired → never salt");
        connect(&mut g, &slope, "out", &salt, "mask");
        let (outs, _) = ev.evaluate_with(&g, &world(), 0, None).expect("eval");
        let c = outs[&(salt.clone(), "biome".into())].as_field().to_vec();
        for (c, s) in c.iter().zip(&steep) {
            assert_eq!(*c == 1.0, *s >= 0.6, "mask {s} → salt? {c}");
        }
        // Bad ranges are refused.
        let bad = add(
            &mut g,
            "jd.biome.classify@1",
            serde_json::json!({ "rules": [{ "name": "x", "height_m": [10.0, 5.0] }] }),
        );
        connect(&mut g, &noise, "out", &bad, "height");
        assert!(ev.evaluate_with(&g, &world(), 0, None).is_err());
    }

    #[test]
    fn chain_evaluates_deterministically_and_caches() {
        let reg = Registry::with_builtins();
        let cache = Cache::with_capacity(0);
        let mut g = Graph::default();
        let n = add(
            &mut g,
            "jd.noise.fbm@1",
            serde_json::json!({ "seed": 3, "height_range_m": [0.0, 100.0] }),
        );
        let s = add(
            &mut g,
            "jd.math.scale@1",
            serde_json::json!({ "factor": 2.0 }),
        );
        let b = add(
            &mut g,
            "jd.filter.blur@1",
            serde_json::json!({ "radius": 2 }),
        );
        let o = add(
            &mut g,
            "jd.output.height@1",
            serde_json::json!({ "layer": "height" }),
        );
        connect(&mut g, &n, "out", &s, "in");
        connect(&mut g, &s, "out", &b, "in");
        connect(&mut g, &b, "out", &o, "in");
        let ev = Evaluator::new(&reg, &cache);
        let (outs, rep) = ev.evaluate(&g, &world(), 0).expect("eval");
        assert_eq!(rep.nodes.len(), 4);
        assert!(rep.nodes.iter().all(|r| !r.cached));
        let result = outs[&(o.clone(), "out".into())].as_field();
        let stats = result.stats();
        assert_eq!(stats.non_finite, 0);
        assert!(stats.max <= 200.0 + 1e-3 && stats.min >= -1e-3, "{stats:?}");
        assert_eq!(
            (result.tile_size(), result.halo()),
            (64, 4),
            "lod 0 keeps the world's tiling"
        );
        // Second run: everything cached, identical output.
        let (outs2, rep2) = ev.evaluate(&g, &world(), 0).expect("eval");
        assert!(rep2.nodes.iter().all(|r| r.cached));
        assert_eq!(
            outs2[&(o.clone(), "out".into())].content_hash(),
            result.content_hash()
        );
        // Change a downstream param: only that node and its descendants recompute.
        g.nodes
            .get_mut(&b)
            .expect("b")
            .params
            .insert("radius".into(), serde_json::json!(1));
        let (_, rep3) = ev.evaluate(&g, &world(), 0).expect("eval");
        let by: BTreeMap<_, _> = rep3
            .nodes
            .iter()
            .map(|r| (r.node.clone(), r.cached))
            .collect();
        assert!(by[&n] && by[&s] && !by[&b] && !by[&o]);
        // LOD 2 produces a quarter-resolution field without touching the lod-0 cache entries.
        let (outs4, rep4) = ev.evaluate(&g, &world(), 2).expect("eval");
        assert!(rep4.nodes.iter().all(|r| !r.cached));
        let small = outs4[&(o, "out".into())].as_field();
        assert_eq!((small.width(), small.height()), (32, 24));
    }

    #[test]
    fn blur_through_halo_matches_naive_reference_and_rejects_oversized_radius() {
        let mut hf = Heightfield::new(100, 70, 32, 4).expect("hf");
        hf.fill_with(|x, y| ((x * 7 + y * 13) % 50) as f32);
        let blurred = box_blur(&hf, 2).expect("blur");
        let (w, h) = (100i64, 70i64);
        for &(x, y) in &[
            (0i64, 0i64),
            (31, 31),
            (32, 32),
            (33, 10),
            (99, 69),
            (50, 35),
        ] {
            let mut sum = 0.0;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    sum += hf.get(
                        (x + dx).clamp(0, w - 1) as u32,
                        (y + dy).clamp(0, h - 1) as u32,
                    );
                }
            }
            let expect = sum / 25.0;
            let got = blurred.get(x as u32, y as u32);
            assert!(
                (got - expect).abs() < 1e-3,
                "({x},{y}) got {got} expected {expect}"
            );
        }
        let no_halo = Heightfield::new(16, 16, 64, 0).expect("hf");
        assert_eq!(
            box_blur(&no_halo, 1).expect_err("halo 0").code,
            ErrorCode::E_INPUT_INVALID
        );
        assert_eq!(
            box_blur(&hf, 5).expect_err("radius > halo").code,
            ErrorCode::E_INPUT_INVALID
        );
    }

    #[test]
    fn validation_reports_missing_inputs_bad_ports_params_and_unknown_kinds() {
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let ev = Evaluator::new(&reg, &cache);
        let mut g = Graph::default();
        let s = add(&mut g, "jd.math.scale@1", serde_json::json!({}));
        ev.validate_structure(&g)
            .expect("structurally fine while unconnected");
        assert_eq!(
            ev.validate(&g).expect_err("missing").code,
            ErrorCode::E_GRAPH_MISSING_INPUT
        );
        let n = add(&mut g, "jd.noise.fbm@1", serde_json::json!({}));
        connect(&mut g, &n, "nope", &s, "in");
        assert_eq!(
            ev.validate_structure(&g).expect_err("bad port").code,
            ErrorCode::E_INPUT_INVALID
        );
        g.edges.clear();
        connect(&mut g, &n, "out", &s, "in");
        ev.validate(&g).expect("valid");
        g.nodes
            .get_mut(&s)
            .expect("s")
            .params
            .insert("factorr".into(), serde_json::json!(2));
        let err = ev.validate_structure(&g).expect_err("bad param");
        assert_eq!(err.code, ErrorCode::E_INPUT_INVALID);
        assert_eq!(
            err.details["node"],
            serde_json::json!(s.to_string()),
            "errors name the node"
        );
        g.nodes.get_mut(&s).expect("s").params.remove("factorr");
        add(&mut g, "jd.unknown@9", serde_json::json!({}));
        assert_eq!(
            ev.validate(&g).expect_err("unknown").code,
            ErrorCode::E_NODE_UNKNOWN
        );
    }

    #[test]
    fn layer_input_resamples_a_published_layer_and_only_outputs_skips_the_rest() {
        use std::sync::Mutex;
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        // A coarse 32 × 24 ramp (0..100 west → east) stands in for a layer published at lod 2.
        let coarse = Arc::new(Mutex::new({
            let mut c = Heightfield::new(32, 24, 64, 0).expect("c");
            c.fill_with(|x, _| x as f32 / 31.0 * 100.0);
            Arc::new(c)
        }));
        let src = coarse.clone();
        let loader: crate::registry::LayerLoader = Arc::new(move |name: &str| {
            Ok((name == "coarse#lod2").then(|| src.lock().expect("lock").clone()))
        });
        let mut g = Graph::default();
        let n = add(&mut g, "jd.noise.fbm@1", serde_json::json!({ "seed": 4 }));
        let heavy = add(
            &mut g,
            "jd.erosion.thermal@1",
            serde_json::json!({ "iterations": 10 }),
        );
        let o_coarse = add(
            &mut g,
            "jd.output.height@1",
            serde_json::json!({ "layer": "coarse" }),
        );
        let l = add(
            &mut g,
            "jd.input.layer@1",
            serde_json::json!({ "name": "coarse#lod2" }),
        );
        let o_fine = add(
            &mut g,
            "jd.output.height@1",
            serde_json::json!({ "layer": "fine" }),
        );
        connect(&mut g, &n, "out", &heavy, "in");
        connect(&mut g, &heavy, "out", &o_coarse, "in");
        connect(&mut g, &l, "out", &o_fine, "in");
        let ev = || {
            Evaluator::new(&reg, &cache)
                .with_layers(loader.clone())
                .only_outputs(Some(vec!["fine".into()]))
        };
        let (outs, rep) = ev().evaluate(&g, &world(), 0).expect("fine only");
        assert_eq!(
            rep.nodes.len(),
            2,
            "only the layer node and its output ran: {:?}",
            rep.nodes.iter().map(|n| n.kind.clone()).collect::<Vec<_>>()
        );
        let fine = outs[&(o_fine.clone(), "out".into())].as_field();
        assert_eq!(
            (fine.width(), fine.height()),
            (128, 96),
            "resampled onto the world grid"
        );
        assert!(
            fine.get(0, 10) < 3.0 && fine.get(127, 10) > 97.0,
            "the ramp survives: {} .. {}",
            fine.get(0, 10),
            fine.get(127, 10)
        );
        // A re-published coarse layer changes the key: the layer node is not served from cache.
        let mut c2 = Heightfield::new(32, 24, 64, 0).expect("c2");
        c2.fill_with(|x, _| 100.0 - x as f32 / 31.0 * 100.0);
        *coarse.lock().expect("lock") = Arc::new(c2);
        let (outs2, rep2) = ev().evaluate(&g, &world(), 0).expect("again");
        assert!(
            rep2.nodes.iter().all(|n| !n.cached),
            "nothing cached after the layer changed"
        );
        assert!(
            outs2[&(o_fine, "out".into())].as_field().get(0, 10) > 97.0,
            "the new ramp"
        );
        // Memory estimate honours the selection; an unknown output name is refused.
        let all = Evaluator::new(&reg, &cache)
            .estimate_bytes(&g, &world(), 0)
            .expect("all");
        let fine_only = ev().estimate_bytes(&g, &world(), 0).expect("fine");
        assert!(fine_only < all, "{fine_only} < {all}");
        assert!(
            Evaluator::new(&reg, &cache)
                .only_outputs(Some(vec!["nope".into()]))
                .evaluate(&g, &world(), 0)
                .is_err()
        );
    }

    #[test]
    fn memory_estimate_follows_tiling_and_ignores_aliased_outputs() {
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let mut g = Graph::default();
        let n = add(&mut g, "jd.noise.fbm@1", serde_json::json!({}));
        let t = add(
            &mut g,
            "jd.erosion.thermal@1",
            serde_json::json!({ "iterations": 10 }),
        );
        let o = add(
            &mut g,
            "jd.output.height@1",
            serde_json::json!({ "layer": "height" }),
        );
        connect(&mut g, &n, "out", &t, "in");
        connect(&mut g, &t, "out", &o, "in");
        let cells = 128u64 * 96;
        let whole = Evaluator::new(&reg, &cache)
            .estimate_bytes(&g, &world(), 0)
            .expect("est");
        // Two real fields (noise, thermal) at 4 B/cell, the output aliases, thermal scratch 8 B/cell.
        assert_eq!(whole, 2 * 4 * cells + 8 * cells);
        g.nodes
            .get_mut(&t)
            .expect("t")
            .params
            .insert("tile_px".into(), serde_json::json!(64));
        let tiled = Evaluator::new(&reg, &cache)
            .estimate_bytes(&g, &world(), 0)
            .expect("est");
        assert_eq!(
            tiled,
            2 * 4 * cells + cells / 2,
            "a tiled kernel declares half a byte per cell"
        );
    }

    #[test]
    fn starter_ids_are_stable_and_the_graph_evaluates() {
        let g = starter();
        let ids: Vec<String> = g.nodes.keys().map(ToString::to_string).collect();
        assert_eq!(
            ids,
            vec![STARTER_NOISE_ID.to_owned(), STARTER_OUTPUT_ID.to_owned()]
        );
        assert_eq!(starter(), g, "starter graph must be deterministic");
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let (outs, _) = Evaluator::new(&reg, &cache)
            .evaluate(&g, &world(), 2)
            .expect("starter evaluates");
        assert_eq!(outs.len(), 2);
    }

    #[test]
    fn thermal_gpu_engine_matches_the_cpu_engine_or_names_the_missing_adapter() {
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let mut g = Graph::default();
        let n = add(
            &mut g,
            "jd.noise.fbm@1",
            serde_json::json!({ "seed": 4, "feature_size_m": 64.0, "height_range_m": [0.0, 400.0] }),
        );
        let cpu = add(
            &mut g,
            "jd.erosion.thermal@1",
            serde_json::json!({ "iterations": 10 }),
        );
        let gpu = add(
            &mut g,
            "jd.erosion.thermal@1",
            serde_json::json!({ "iterations": 10, "engine": "gpu" }),
        );
        connect(&mut g, &n, "out", &cpu, "in");
        connect(&mut g, &n, "out", &gpu, "in");
        let r = Evaluator::new(&reg, &cache).evaluate(&g, &world(), 0);
        match (jd_gpu::Gpu::shared(), r) {
            (Some(_), Ok((outs, _))) => {
                let (a, b) = (
                    outs[&(cpu, "out".into())].as_field().to_vec(),
                    outs[&(gpu, "out".into())].as_field().to_vec(),
                );
                let worst = a
                    .iter()
                    .zip(&b)
                    .map(|(x, y)| (x - y).abs())
                    .fold(0.0f32, f32::max);
                assert!(
                    worst <= 400.0 * 1e-4,
                    "gpu engine agrees with cpu: {worst} m"
                );
            }
            (None, Err(e)) => assert_eq!(e.code, ErrorCode::E_DEPENDENCY_MISSING),
            (gpu, r) => panic!("adapter {:?} but {:?}", gpu.is_some(), r.map(|_| ())),
        }
    }

    #[test]
    fn erosion_and_slope_nodes_evaluate_at_world_scale() {
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let mut g = Graph::default();
        let n = add(
            &mut g,
            "jd.noise.fbm@1",
            serde_json::json!({ "seed": 4, "feature_size_m": 64.0, "height_range_m": [0.0, 400.0] }),
        );
        let e = add(
            &mut g,
            "jd.erosion.thermal@1",
            serde_json::json!({ "iterations": 10 }),
        );
        let s = add(&mut g, "jd.analysis.slope@1", serde_json::json!({}));
        connect(&mut g, &n, "out", &e, "in");
        connect(&mut g, &e, "out", &s, "in");
        let (outs, rep) = Evaluator::new(&reg, &cache)
            .evaluate(&g, &world(), 0)
            .expect("eval");
        assert_eq!(rep.nodes.len(), 3);
        let noise = outs[&(n.clone(), "out".into())].as_field();
        let eroded = outs[&(e, "out".into())].as_field();
        assert_ne!(
            noise.content_hash(),
            eroded.content_hash(),
            "steep noise must erode"
        );
        let (a, b) = (noise.stats(), eroded.stats());
        assert!(
            (a.mean - b.mean).abs() < 1e-2,
            "mass conserved: {} vs {}",
            a.mean,
            b.mean
        );
        assert!(
            b.max <= a.max + 1e-3 && b.min >= a.min - 1e-3,
            "erosion never exceeds the input range"
        );
        let mask = outs[&(s, "out".into())].as_field().stats();
        assert!(
            mask.min >= 0.0 && mask.max <= 1.0 && mask.max > 0.0,
            "{mask:?}"
        );
        // Hydraulic node: three outputs, terrain+sediment conserved, water and sediment ≥ 0.
        let hy = add(
            &mut g,
            "jd.erosion.hydraulic@1",
            serde_json::json!({ "iterations": 20 }),
        );
        connect(&mut g, &n, "out", &hy, "in");
        let (outs, _) = Evaluator::new(&reg, &cache)
            .evaluate(&g, &world(), 0)
            .expect("eval");
        let ht = outs[&(hy.clone(), "out".into())].as_field();
        let wt = outs[&(hy.clone(), "water".into())].as_field();
        let st = outs[&(hy, "sediment".into())].as_field();
        assert!(wt.stats().min >= 0.0 && st.stats().min >= 0.0);
        assert!(
            ((ht.stats().mean + st.stats().mean) - noise.stats().mean).abs() < 1e-3,
            "terrain+sediment mean preserved"
        );
        g.nodes
            .get_mut(&n)
            .expect("n")
            .params
            .insert("seed".into(), serde_json::json!(5));
        let (_, rep2) = Evaluator::new(&reg, &cache)
            .evaluate(&g, &world(), 0)
            .expect("eval");
        assert!(
            rep2.nodes.iter().all(|r| !r.cached),
            "upstream change invalidates the chain"
        );
    }

    #[test]
    fn sculpt_node_applies_strokes_in_layer_metres() {
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let mut g = Graph::default();
        let n = add(&mut g, "jd.noise.fbm@1", serde_json::json!({ "seed": 2 }));
        let z = add(
            &mut g,
            "jd.math.scale@1",
            serde_json::json!({ "factor": 0.0 }),
        ); // flat 0 m
        let s = add(
            &mut g,
            "jd.sculpt.strokes@1",
            serde_json::json!({ "strokes": [{ "points": [[65.0, 86.667]], "radius_m": 20.0, "strength_m": 50.0 }] }),
        );
        connect(&mut g, &n, "out", &z, "in");
        connect(&mut g, &z, "out", &s, "in");
        let (outs, _) = Evaluator::new(&reg, &cache)
            .evaluate(&g, &world(), 0)
            .expect("eval");
        let f = outs[&(s, "out".into())].as_field();
        // world(): 256 m over 128×96 px → 2 m × 2.667 m pixels; pixel (32, 32) is centred at (65, 86.667) m.
        assert!((f.get(32, 32) - 50.0).abs() < 0.5, "{}", f.get(32, 32));
        assert_eq!(f.get(100, 10), 0.0);
    }

    #[test]
    fn flow_node_marks_valleys() {
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let mut g = Graph::default();
        let n = add(
            &mut g,
            "jd.noise.fbm@1",
            serde_json::json!({ "seed": 8, "feature_size_m": 96.0 }),
        );
        let f = add(&mut g, "jd.analysis.flow@1", serde_json::json!({}));
        connect(&mut g, &n, "out", &f, "in");
        let (outs, _) = Evaluator::new(&reg, &cache)
            .evaluate(&g, &world(), 0)
            .expect("eval");
        let m = outs[&(f, "out".into())].as_field().stats();
        assert!(m.min > 0.0 && m.max <= 1.0 && m.max > m.min, "{m:?}");
    }

    #[test]
    fn input_node_reads_assets_and_refuses_escapes() {
        let dir = tempfile::tempdir().expect("tmp");
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(&assets).expect("dir");
        let img = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_fn(16, 16, |x, _| {
            image::Luma([(x * 4369) as u16])
        });
        img.save(assets.join("dem.png")).expect("png");
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let mut g = Graph::default();
        let i = add(
            &mut g,
            "jd.input.heightmap@1",
            serde_json::json!({ "path": "dem.png", "range_m": [0.0, 100.0] }),
        );
        let ev = Evaluator::new(&reg, &cache);
        let (outs, _) = ev
            .evaluate_with(&g, &world(), 0, Some(&assets))
            .expect("eval");
        let f = outs[&(i.clone(), "out".into())].as_field();
        assert_eq!(
            (f.width(), f.height()),
            (128, 96),
            "resampled onto the world grid"
        );
        assert!(f.get(2, 10) < 20.0 && f.get(125, 10) > 80.0);
        assert_eq!(
            ev.evaluate(&g, &world(), 0).expect_err("no project").code,
            ErrorCode::E_INPUT_INVALID
        );
        #[cfg(unix)]
        {
            std::fs::write(dir.path().join("outside.png"), b"x").expect("write");
            std::os::unix::fs::symlink(dir.path().join("outside.png"), assets.join("link.png"))
                .expect("symlink");
            g.nodes
                .get_mut(&i)
                .expect("i")
                .params
                .insert("path".into(), serde_json::json!("link.png"));
            assert_eq!(
                ev.evaluate_with(&g, &world(), 0, Some(&assets))
                    .expect_err("symlink escape")
                    .code,
                ErrorCode::E_INPUT_INVALID
            );
        }
        g.nodes
            .get_mut(&i)
            .expect("i")
            .params
            .insert("path".into(), serde_json::json!("../dem.png"));
        assert_eq!(
            ev.evaluate_with(&g, &world(), 0, Some(&assets))
                .expect_err("escape")
                .code,
            ErrorCode::E_INPUT_INVALID
        );
    }

    #[test]
    fn bypass_passes_input_through() {
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let mut g = Graph::default();
        let n = add(&mut g, "jd.noise.fbm@1", serde_json::json!({ "seed": 1 }));
        let s = add(
            &mut g,
            "jd.math.scale@1",
            serde_json::json!({ "factor": 5.0 }),
        );
        connect(&mut g, &n, "out", &s, "in");
        g.nodes.get_mut(&s).expect("s").bypass = true;
        let (outs, _) = Evaluator::new(&reg, &cache)
            .evaluate(&g, &world(), 1)
            .expect("eval");
        assert_eq!(
            outs[&(s, "out".into())].content_hash(),
            outs[&(n, "out".into())].content_hash()
        );
        // A bypassed node still needs the input it forwards.
        g.edges.clear();
        let err = Evaluator::new(&reg, &cache)
            .validate(&g)
            .expect_err("nothing to forward");
        assert_eq!(err.code, ErrorCode::E_GRAPH_MISSING_INPUT);
    }
}
