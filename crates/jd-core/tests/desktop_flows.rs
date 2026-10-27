//! Engine behaviour the desktop's end-to-end specs checked through its panels, kept when those
//! panels moved to Studio (ADR-0017 P5; memory/p5-retire-map-2026-10-27.md): the look and the
//! settings through save → reopen, the debris and sand layers, painted density's clamped strokes,
//! the cache GC report, still exports kept under exports/, scatter following the timeline.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use jd_core::{Context, Registry, Request};
use serde_json::{Value, json};

struct Engine {
    reg: Registry,
    ctx: Context,
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
}

impl Engine {
    /// A fresh 64² project with the starter graph.
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("w.jdproj");
        let e = Self {
            reg: Registry::with_builtins(),
            ctx: Context::default(),
            path: path.clone(),
            _dir: dir,
        };
        e.ok(
            "project.create",
            json!({ "path": path, "name": "W", "world": { "resolution_px": [64, 64], "extent_m": [640.0, 640.0], "tile_size": 64, "halo": 4, "origin": [0, 0], "seed": 7 } }),
        );
        e
    }

    fn call(&self, op: &str, params: Value) -> jd_core::Response {
        self.reg.invoke(&self.ctx, Request::new(op, params))
    }

    fn ok(&self, op: &str, params: Value) -> Value {
        let r = self.call(op, params);
        assert!(r.error.is_none(), "{op}: {:?}", r.error);
        r.result.unwrap_or(Value::Null)
    }

    fn reopen(&self) -> Value {
        self.ok("project.save", json!({}));
        self.ok("project.close", json!({}));
        self.ok("project.open", json!({ "path": self.path }))
    }

    /// The node feeding the output that publishes `layer`.
    fn feed_of(&self, layer: &str) -> Value {
        let g = self.ok("graph.list", json!({}));
        let nodes = g["nodes"].as_array().unwrap();
        let out = nodes
            .iter()
            .find(|n| {
                n["kind"].as_str().unwrap().starts_with("jd.output.")
                    && n["params"]["layer"] == layer
            })
            .unwrap_or_else(|| panic!("no output for {layer}"));
        let e = g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["to"]["node"] == out["id"])
            .expect("the output's input");
        nodes
            .iter()
            .find(|n| n["id"] == e["from"]["node"])
            .cloned()
            .unwrap()
    }
}

#[test]
fn the_look_is_saved_in_scene_json_and_comes_back_on_reopen() {
    let e = Engine::new();
    let sun = json!({ "azimuth_deg": 200.0, "elevation_deg": 35.0 });
    let scatter = json!({ "mask": "height", "spacing_m": 25.0, "seed": 3 });
    let materials = json!([{ "mask": null, "albedo": [0.5, 0.4, 0.3], "roughness": 0.9 }]);
    e.ok(
        "project.set_look",
        json!({ "materials": materials, "sun": sun, "scatter": scatter }),
    );
    let opened = e.reopen();
    let scene: Value =
        serde_json::from_slice(&std::fs::read(e.path.join("scene.json")).unwrap()).unwrap();
    for look in [&scene["look"], &opened["look"]] {
        assert_eq!(look["sun"]["azimuth_deg"], 200.0, "{look}");
        assert_eq!(look["scatter"]["spacing_m"], 25.0, "{look}");
        assert_eq!(look["materials"][0]["roughness"], 0.9, "{look}");
    }
}

#[test]
fn settings_origin_crs_and_wind_survive_save_and_reopen() {
    let e = Engine::new();
    e.ok(
        "project.set_settings",
        json!({ "autosave_interval_s": 30, "origin": [500000.0, 4100000.0], "crs": "EPSG:32637", "wind": { "azimuth_deg": 90.0, "speed_mps": 12.0 } }),
    );
    let opened = e.reopen();
    let on_disk: Value =
        serde_json::from_slice(&std::fs::read(e.path.join("project.json")).unwrap()).unwrap();
    for p in [&opened, &on_disk] {
        let world = &p["world"];
        assert_eq!(world["crs"], "EPSG:32637", "{p}");
        assert_eq!(world["origin"], json!([500000.0, 4100000.0]), "{p}");
        assert_eq!(world["wind"]["azimuth_deg"], 90.0, "{p}");
    }
    assert_eq!(opened["settings"]["autosave_interval_s"], 30);
    assert_eq!(on_disk["settings"]["autosave_interval_s"], 30);
}

#[test]
fn debris_and_sand_splice_in_front_of_the_height() {
    let e = Engine::new();
    let debris = e.ok("graph.add_simulation", json!({ "which": "debris" }));
    assert_eq!(debris["published"], json!(["debris"]));
    assert_eq!(e.feed_of("height")["kind"], "jd.erosion.debris@1");
    assert_eq!(e.feed_of("debris")["kind"], "jd.erosion.debris@1");
    let info = e.ok("project.info", json!({}));
    assert!(
        info["look"]["materials"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["mask"] == "debris"),
        "{}",
        info["look"]
    );
    // Sand goes in front of the height too: the chain now ends at the dunes.
    let sand = e.ok("graph.add_simulation", json!({ "which": "sand" }));
    assert_eq!(sand["published"], json!([]));
    assert_eq!(e.feed_of("height")["kind"], "jd.erosion.aeolian@1");
    let r = e.ok("graph.evaluate", json!({ "lod": 0 }));
    assert!(r["layers"].get("debris").is_some(), "{r}");
}

#[test]
fn painted_density_gets_a_clamped_strokes_node_in_front_of_its_mask() {
    let e = Engine::new();
    e.ok("graph.add_simulation", json!({ "which": "snow" }));
    let added = e.ok(
        "graph.sculpt_target",
        json!({ "layer": "snow", "kind": "strokes", "clamp": [0.0, 1.0] }),
    );
    let strokes = e.feed_of("snow");
    assert_eq!(strokes["id"], added["id"]);
    assert_eq!(strokes["kind"], "jd.sculpt.strokes@1");
    assert_eq!(strokes["params"]["clamp"], json!([0.0, 1.0]));
    // Asked again, the same node: one strokes node per layer.
    let again = e.ok(
        "graph.sculpt_target",
        json!({ "layer": "snow", "kind": "strokes", "clamp": [0.0, 1.0] }),
    );
    assert_eq!(again["id"], added["id"]);
}

#[test]
fn cache_gc_reports_what_it_kept_under_the_projects_limit() {
    let e = Engine::new();
    e.ok("graph.evaluate", json!({ "lod": 0 }));
    let limit = e.ok("project.info", json!({}))["settings"]["cache_limit_bytes"].clone();
    let gc = e.ok("project.cache_gc", json!({}));
    assert_eq!(gc["limit_bytes"], limit);
    for k in ["blobs", "bytes", "evicted_blobs", "evicted_bytes"] {
        assert!(gc[k].is_u64(), "{k}: {gc}");
    }
    assert!(gc["bytes"].as_u64().unwrap() <= limit.as_u64().unwrap());
}

#[test]
fn stills_are_written_under_exports_and_nowhere_else() {
    use base64::Engine as _;
    let e = Engine::new();
    let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nstill");
    let r = e.ok(
        "project.write_export",
        json!({ "path": "stills/view_01.png", "bytes_base64": png }),
    );
    let written = e.path.join("exports/stills/view_01.png");
    assert_eq!(std::fs::read(&written).unwrap(), b"\x89PNG\r\n\x1a\nstill");
    assert_eq!(std::path::Path::new(r["path"].as_str().unwrap()), written);
    for bad in ["../escape.png", "/tmp/jd-escape.png", ""] {
        let r = e.call(
            "project.write_export",
            json!({ "path": bad, "bytes_base64": png }),
        );
        assert_eq!(
            r.error.expect(bad).code.as_str(),
            "E_INPUT_INVALID",
            "{bad}"
        );
    }
    assert!(!e.path.join("escape.png").exists());
}

#[test]
fn scatter_follows_the_timeline_when_asked() {
    let e = Engine::new();
    e.ok("graph.evaluate", json!({ "lod": 0 }));
    let count = |t: f64| {
        e.ok("graph.set_time", json!({ "time": t }));
        let r = e.ok(
            "terrain.scatter",
            json!({ "mask": "height", "spacing_m": 20.0, "seed": 1, "follow_time": true }),
        );
        r["count"].as_u64().unwrap()
    };
    let (start, end) = (count(0.0), count(1.0));
    assert_eq!(start, 0, "nothing has grown at the start of the timeline");
    assert!(end > 100, "{end} at the end of the timeline");
    // Without follow_time the timeline is ignored: the full population at any time.
    e.ok("graph.set_time", json!({ "time": 0.0 }));
    let fixed = e.ok(
        "terrain.scatter",
        json!({ "mask": "height", "spacing_m": 20.0, "seed": 1 }),
    );
    assert_eq!(fixed["count"].as_u64().unwrap(), end);
}
