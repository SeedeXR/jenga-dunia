//! Built-in node kinds (Phase 2 initial set).

use std::collections::BTreeMap;
use std::sync::Arc;

use jd_terrain::{
    AeolianParams, CoastalParams, DropletParams, FlowMethod, ForeignImport, Heightfield,
    HydraulicParams, NoiseParams, SeabedParams, SnowParams, StreamPowerParams, Stroke,
    THERMAL_MAX_RATE, aeolian_transport, apply_strokes, carve_rivers, classify_seabed,
    coastal_erode, droplet_erode, fill_depressions, flow_accumulation_with, generate_noise,
    read_foreign, resample_to_world, sea_level, slope_mask, snow_cover, wetness_index_with,
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
}
fn rivers_threshold() -> f32 {
    0.55
}
fn rivers_depth() -> f32 {
    20.0
}

/// Tile-local box blur that reads neighbours through the halo (ADR-0005). Fails cleanly when
/// the field's halo is smaller than the radius — a tile must never read outside its padding.
fn box_blur(a: &Heightfield, radius: u32) -> JdResult<Heightfield> {
    if radius == 0 || radius > a.halo() {
        return Err(JdError::invalid(
            "radius",
            format!("blur radius {radius} exceeds the tile halo {}", a.halo()),
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
            let n = ((2 * r + 1) * (2 * r + 1)) as f32;
            for iy in 0..ts as i64 {
                for ix in 0..ts as i64 {
                    let mut sum = 0.0f32;
                    for dy in -r..=r {
                        for dx in -r..=r {
                            sum += t.get_padded(ix + dx, iy + dy);
                        }
                    }
                    data[(iy * ts as i64 + ix) as usize] = sum / n;
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
    r.register(NodeDef::new::<NoParams, _>(
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
    ));
    r.register(NodeDef::new::<NoParams, _>(
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
    ));
    r.register(NodeDef::new::<ScaleParams, _>(
        "jd.math.scale@1",
        "in × factor",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |_, p, i| {
            let sp: ScaleParams = params(p)?;
            Ok(out("out", map1(field(i, "in")?, |v| v * sp.factor)?))
        },
    ));
    r.register(NodeDef::new::<RemapParams, _>(
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
    ));
    r.register(NodeDef::new::<BlurParams, _>(
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
    ));
    r.register(NodeDef::new::<ThermalParams, _>(
        "jd.erosion.thermal@1",
        "Thermal erosion: material steeper than the talus angle slides downhill (mass-conserving Jacobi steps); an optional `hardness` field (0..1, e.g. from `jd.terrain.strata@1`) lets hard rock hold near-vertical faces and shed at a tenth of the rate",
        vec![port("in", PortType::Heightfield, true), port("hardness", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let tp: ThermalParams = params(p)?;
            if !(1..=1000).contains(&tp.iterations) {
                return Err(JdError::invalid("iterations", "must be within 1..=1000"));
            }
            let hardness = i.get("hardness").map(|v| v.as_field());
            Ok(out("out", jd_terrain::thermal_erode_with(
                    field(i, "in")?,
                    spacing_m(ctx), tp.talus_deg, tp.iterations, tp.rate, hardness.map(|k| &**k), Some(&ctx.cancel))?))
        },
    ).with_memory(8.0));
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
    ));
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
    ));
    r.register(NodeDef::new::<AeolianParams, _>(
        "jd.erosion.aeolian@1",
        "Wind (aeolian) transport after Werner's slab model: sand hops downwind, stops in wind shadows and on sand, and the angle of repose holds — dunes and ripples; optional `sand` mask (0..1) says what can move",
        vec![port("in", PortType::Heightfield, true), port("sand", PortType::Heightfield, false)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let ap: AeolianParams = params(&with_world_wind(ctx, p, &["wind_azimuth_deg"]))?;
            if !(1..=500).contains(&ap.iterations) {
                return Err(JdError::invalid("iterations", "must be within 1..=500"));
            }
            let sand = i.get("sand").map(|v| v.as_field());
            Ok(out("out", aeolian_transport(field(i, "in")?, spacing_m(ctx), sand.map(|s| &**s), &ap, Some(&ctx.cancel))?))
        },
    ).with_memory(12.0));
    r.register(NodeDef::new::<CoastalParams, _>(
        "jd.erosion.coastal@1",
        "Coastal erosion: waves cut the shore within `band_m` above `level_m` by its ocean exposure and lay the material on the adjacent sea floor — cliff retreat, wave-cut platforms, beaches; outputs the terrain and a `shore` mask of wave work",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false), port("shore", PortType::Heightfield, false)],
        |ctx, p, i| {
            let cp: CoastalParams = params(p)?;
            if !(1..=1000).contains(&cp.iterations) {
                return Err(JdError::invalid("iterations", "must be within 1..=1000"));
            }
            let (out_h, shore) = coastal_erode(field(i, "in")?, &cp, Some(&ctx.cancel))?;
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
        "Hydraulic erosion (virtual pipes / shallow water): rain flows downhill, dissolves, carries and deposits sediment; outputs terrain, water depth and suspended sediment; an optional `hardness` field (0..1) drops the dissolving rate to a tenth on hard rock",
        vec![port("in", PortType::Heightfield, true), port("hardness", PortType::Heightfield, false)],
        vec![
            port("out", PortType::Heightfield, false),
            port("water", PortType::Heightfield, false),
            port("sediment", PortType::Heightfield, false),
        ],
        |ctx, p, i| {
            let hp: HydraulicNodeParams = params(p)?;
            if !(1..=2000).contains(&hp.iterations) {
                return Err(JdError::invalid("iterations", "must be within 1..=2000"));
            }
            let hardness = i.get("hardness").map(|v| v.as_field());
            let r = jd_terrain::hydraulic_erode_with(
                field(i, "in")?,
                spacing_m(ctx),
                &HydraulicParams {
                    iterations: hp.iterations,
                    dt: hp.dt,
                    rain_rate: hp.rain_rate,
                    evaporation: hp.evaporation,
                    capacity_k: hp.capacity_k,
                    dissolve_k: hp.dissolve_k,
                    deposit_k: hp.deposit_k,
                    max_concentration: hp.max_concentration,
                    ..Default::default()
                }, hardness.map(|k| &**k), Some(&ctx.cancel))?;
            Ok(BTreeMap::from([
                ("out".to_owned(), Value::field(r.height)),
                ("water".to_owned(), Value::field(r.water)),
                ("sediment".to_owned(), Value::field(r.sediment)),
            ]))
        },
    ).with_memory(40.0));
    r.register(NodeDef::new::<SculptParams, _>(
        "jd.sculpt.strokes@1",
        "Brush strokes (raise/lower with smooth falloff) recorded as data: non-destructive, replayable, undo removes a whole stroke",
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
        "Load a heightmap from the project's assets/ (PNG 16-bit, EXR, TIFF, RAW, or a binary FBX mesh rasterised over its footprint) and resample it onto the world grid",
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
            let (data, w, h) = read_foreign(&file, &ip.import)?;
            Ok(out("out", resample_to_world(&data, w, h, ip.import.north_up, &ctx.lod_world)?))
        },
    ));
    r.register(NodeDef::new::<FillParams, _>(
        "jd.hydrology.fill@1",
        "Priority-flood depression filling: raises pits to their spill level so every cell drains; `lakes` = water depth of the filled basins",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false), port("lakes", PortType::Heightfield, false)],
        |_, p, i| {
            let fp: FillParams = params(p)?;
            let (filled, lakes) = fill_depressions(field(i, "in")?, fp.epsilon_m)?;
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
        "Classify the sea floor below `level_m`: 1 sand on the shelf above the wave base, 2 rock on steep slopes, 3 mud in deep still water, 4 reef on a shallow gentle shelf away from river sediment (optional `rivers` and `shore` masks); 0 = land",
        vec![
            port("in", PortType::Heightfield, true),
            port("rivers", PortType::Heightfield, false),
            port("shore", PortType::Heightfield, false),
        ],
        vec![port("seabed", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: SeabedParams = params(p)?;
            let rivers = i.get("rivers").map(|v| v.as_field());
            let shore = i.get("shore").map(|v| v.as_field());
            Ok(out("seabed", classify_seabed(field(i, "in")?, spacing_m(ctx), rivers.map(|r| &**r), shore.map(|s| &**s), &sp)?))
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
        "Sediment infill of the sea floor: sand and mud cells (from a `seabed` class map) rise towards the local mean over `smooth_m`, capped at `max_fill_m` and `level_m`; rock, reef and land are untouched",
        vec![
            port("in", PortType::Heightfield, true),
            port("seabed", PortType::Heightfield, true),
        ],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let bp: jd_terrain::BathymetryParams = params(p)?;
            Ok(out("out", jd_terrain::bathymetry(field(i, "in")?, spacing_m(ctx), field(i, "seabed")?, &bp)?))
        },
    ).with_memory(16.0));
    r.register(NodeDef::new::<ClassMaskParams, _>(
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
    ));
    r.register(NodeDef::new::<SeaLevelParams, _>(
        "jd.hydrology.sea_level@1",
        "Ocean mask (1 below `level_m` where connected to the border; inland basins stay dry) with a soft shoreline, and water `depth` in metres",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("ocean", PortType::Heightfield, false), port("depth", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: SeaLevelParams = params(&with_world_wind(ctx, p, &["wind_azimuth_deg", "wind_speed_mps"]))?;
            if !(sp.visibility_m > 0.0 && sp.visibility_m.is_finite()) {
                return Err(JdError::invalid("visibility_m", "must be positive"));
            }
            if sp.tint.iter().any(|c| !(0.0..=1.0).contains(c)) {
                return Err(JdError::invalid("tint", "components must be within 0..=1"));
            }
            // The wind sea is drawn, not evaluated; validate it here so a bad edit fails at the node.
            jd_terrain::wave_spectrum(&jd_terrain::WaveParams {
                wind_speed_mps: sp.wind_speed_mps,
                fetch_km: sp.fetch_km,
                wind_azimuth_deg: sp.wind_azimuth_deg,
            })?;
            let (ocean, depth) = sea_level(field(i, "in")?, sp.level_m, sp.shore_m)?;
            Ok(BTreeMap::from([("ocean".to_owned(), Value::field(ocean)), ("depth".to_owned(), Value::field(depth))]))
        },
    ));
    r.register(NodeDef::new::<RiversParams, _>(
        "jd.hydrology.rivers@1",
        "Carve river channels where flow accumulates (depressions filled first); outputs the carved terrain and a 0..1 river mask",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false), port("rivers", PortType::Heightfield, false)],
        |ctx, p, i| {
            let rp: RiversParams = params(p)?;
            let (carved, rivers) = carve_rivers(field(i, "in")?, spacing_m(ctx), rp.threshold, rp.depth_m, rp.method)?;
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
    r.register(NodeDef::new::<OutputParams, _>(
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
    ));
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
