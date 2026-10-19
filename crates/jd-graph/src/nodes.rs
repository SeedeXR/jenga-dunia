//! Built-in node kinds (Phase 2 initial set).

use std::collections::BTreeMap;
use std::sync::Arc;

use jd_terrain::{
    ForeignImport, Heightfield, HydraulicParams, NoiseParams, Stroke, THERMAL_MAX_RATE,
    apply_strokes, flow_accumulation, generate_noise, hydraulic_erode, read_foreign,
    resample_to_world, slope_mask, thermal_erode,
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
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FlowParams {
    /// Output ln(1 + area) / ln(1 + N) in 0..1 (true) or the raw upstream cell count (false).
    #[serde(default = "yes")]
    log_scale: bool,
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
        "Thermal erosion: material steeper than the talus angle slides downhill (mass-conserving Jacobi steps)",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let tp: ThermalParams = params(p)?;
            if !(1..=1000).contains(&tp.iterations) {
                return Err(JdError::invalid("iterations", "must be within 1..=1000"));
            }
            Ok(out("out", thermal_erode(
                    field(i, "in")?,
                    spacing_m(ctx), tp.talus_deg, tp.iterations, tp.rate)?))
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
        "Hydraulic erosion (virtual pipes / shallow water): rain flows downhill, dissolves, carries and deposits sediment; outputs terrain, water depth and suspended sediment",
        vec![port("in", PortType::Heightfield, true)],
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
            let r = hydraulic_erode(
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
                    ..Default::default()
                },
            )?;
            Ok(BTreeMap::from([
                ("out".to_owned(), Value::field(r.height)),
                ("water".to_owned(), Value::field(r.water)),
                ("sediment".to_owned(), Value::field(r.sediment)),
            ]))
        },
    ));
    r.register(NodeDef::new::<SculptParams, _>(
        "jd.sculpt.strokes@1",
        "Brush strokes (raise/lower with smooth falloff) recorded as data: non-destructive, replayable, undo removes a whole stroke",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let sp: SculptParams = params(p)?;
            Ok(out("out", apply_strokes(field(i, "in")?, spacing_m(ctx), &sp.strokes)?))
        },
    ));
    r.register(NodeDef::new::<FlowParams, _>(
        "jd.analysis.flow@1",
        "D8 flow accumulation: upstream drainage area per cell (log-scaled 0..1 mask by default) — rivers, wetness, erosion seeding",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        |ctx, p, i| {
            let fp: FlowParams = params(p)?;
            Ok(out("out", flow_accumulation(field(i, "in")?, spacing_m(ctx), fp.log_scale)?))
        },
    ));
    r.register(NodeDef::new::<InputParams, _>(
        "jd.input.heightmap@1",
        "Load a heightmap from the project's assets/ (PNG 16-bit, EXR, TIFF, RAW) and resample it onto the world grid",
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
