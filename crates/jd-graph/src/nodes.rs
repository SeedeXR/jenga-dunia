//! Built-in node kinds (Phase 2 initial set).

use std::collections::BTreeMap;
use std::sync::Arc;

use jd_terrain::{Heightfield, NoiseParams, generate_noise};
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

fn schema<T: schemars::JsonSchema>() -> Json {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Json::Null)
}

fn params<T: for<'de> Deserialize<'de>>(p: &Map<String, Json>) -> JdResult<T> {
    serde_json::from_value(Json::Object(p.clone()))
        .map_err(|e| JdError::invalid("params", e.to_string()))
}

fn field<'a>(inputs: &'a BTreeMap<String, Value>, name: &str) -> JdResult<&'a Arc<Heightfield>> {
    inputs
        .get(name)
        .ok_or_else(|| JdError::invalid(name, "input not connected"))?
        .field()
}

fn out(name: &str, hf: Heightfield) -> BTreeMap<String, Value> {
    BTreeMap::from([(name.to_owned(), Value::Field(Arc::new(hf)))])
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

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BlurParams {
    /// Box-blur radius in pixels (1..=4; bounded by the tile halo).
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

/// Tile-local box blur that reads neighbours through the halo (ADR-0005). The radius is
/// clamped to the field's halo so a tile never reads outside its padded storage.
fn box_blur(a: &Heightfield, radius: u32) -> JdResult<Heightfield> {
    let r = radius.min(a.halo()).max(1) as i64;
    let mut outf = Heightfield::new(a.width(), a.height(), a.tile_size(), a.halo())?;
    let tiles_in = a.tiles();
    let ts = a.tile_size();
    let (w, h) = (a.width(), a.height());
    let results: Vec<Vec<f32>> = tiles_in
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
    // Write interiors back in tile order (edge tiles beyond the field are clamped by fill_with).
    let (tx, _) = a.tile_counts();
    outf.fill_with(|x, y| {
        let (tcx, tcy) = (x / ts, y / ts);
        let t = &results[(tcy * tx + tcx) as usize];
        let _ = (w, h);
        t[((y % ts) * ts + (x % ts)) as usize]
    });
    Ok(outf)
}

/// Register every built-in node kind.
pub fn register_all(r: &mut Registry) {
    r.register(NodeDef::new(
        "jd.noise.fbm@1",
        "Fractal noise heightfield (OpenSimplex2/Perlin/… × fBm/ridged/ping-pong), parameters in metres",
        vec![],
        vec![port("out", PortType::Heightfield, false)],
        schema::<NoiseParams>(),
        Some(0),
        |ctx: &EvalCtx, p, _| {
            let np: NoiseParams = params(p)?;
            Ok(out("out", generate_noise(&ctx.lod_world, &np)?))
        },
    ));
    r.register(NodeDef::new(
        "jd.math.add@1",
        "a + b",
        vec![
            port("a", PortType::Heightfield, true),
            port("b", PortType::Heightfield, true),
        ],
        vec![port("out", PortType::Heightfield, false)],
        schema::<()>(),
        Some(0),
        |_, _, i| {
            Ok(out(
                "out",
                map2(field(i, "a")?, field(i, "b")?, |a, b| a + b)?,
            ))
        },
    ));
    r.register(NodeDef::new(
        "jd.math.multiply@1",
        "a × b (use a 0..1 mask as b to attenuate)",
        vec![
            port("a", PortType::Heightfield, true),
            port("b", PortType::Heightfield, true),
        ],
        vec![port("out", PortType::Heightfield, false)],
        schema::<()>(),
        Some(0),
        |_, _, i| {
            Ok(out(
                "out",
                map2(field(i, "a")?, field(i, "b")?, |a, b| a * b)?,
            ))
        },
    ));
    r.register(NodeDef::new(
        "jd.math.scale@1",
        "in × factor",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        schema::<ScaleParams>(),
        Some(0),
        |_, p, i| {
            let sp: ScaleParams = params(p)?;
            Ok(out("out", map1(field(i, "in")?, |v| v * sp.factor)?))
        },
    ));
    r.register(NodeDef::new(
        "jd.math.remap@1",
        "Linear remap of [from0, from1] to [to0, to1], optional clamp",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        schema::<RemapParams>(),
        Some(0),
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
    r.register(NodeDef::new(
        "jd.filter.blur@1",
        "Box blur (tile-local, reads through the halo)",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        schema::<BlurParams>(),
        Some(4),
        |_, p, i| {
            let bp: BlurParams = params(p)?;
            if !(1..=4).contains(&bp.radius) {
                return Err(JdError::invalid("radius", "must be within 1..=4"));
            }
            Ok(out("out", box_blur(field(i, "in")?, bp.radius)?))
        },
    ));
    r.register(NodeDef::new(
        "jd.output.height@1",
        "Publishes its input as a named heightfield layer",
        vec![port("in", PortType::Heightfield, true)],
        vec![port("out", PortType::Heightfield, false)],
        schema::<OutputParams>(),
        Some(0),
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
        let result = outs[&(o.clone(), "out".into())].field().expect("field");
        let stats = result.stats();
        assert_eq!(stats.non_finite, 0);
        assert!(stats.max <= 200.0 + 1e-3 && stats.min >= -1e-3, "{stats:?}");
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
        let small = outs4[&(o, "out".into())].field().expect("field");
        assert_eq!((small.width(), small.height()), (32, 24));
    }

    #[test]
    fn blur_through_halo_matches_naive_reference() {
        let mut hf = Heightfield::new(100, 70, 32, 4).expect("hf");
        hf.fill_with(|x, y| ((x * 7 + y * 13) % 50) as f32);
        let blurred = box_blur(&hf, 2).expect("blur");
        // Naive reference with edge clamping.
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
    }

    #[test]
    fn validation_reports_missing_inputs_type_mismatch_and_unknown_kinds() {
        let reg = Registry::with_builtins();
        let cache = Cache::default();
        let ev = Evaluator::new(&reg, &cache);
        let mut g = Graph::default();
        let s = add(&mut g, "jd.math.scale@1", serde_json::json!({}));
        assert_eq!(
            ev.validate(&g).expect_err("missing").code,
            ErrorCode::E_GRAPH_MISSING_INPUT
        );
        let n = add(&mut g, "jd.noise.fbm@1", serde_json::json!({}));
        connect(&mut g, &n, "nope", &s, "in");
        assert_eq!(
            ev.validate(&g).expect_err("bad port").code,
            ErrorCode::E_INPUT_INVALID
        );
        g.edges.clear();
        connect(&mut g, &n, "out", &s, "in");
        ev.validate(&g).expect("valid");
        add(&mut g, "jd.unknown@9", serde_json::json!({}));
        assert_eq!(
            ev.validate(&g).expect_err("unknown").code,
            ErrorCode::E_NODE_UNKNOWN
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
    }
}
