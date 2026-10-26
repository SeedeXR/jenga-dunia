//! The Studio window under egui_kittest with a real wgpu renderer: panels, the viewport's
//! pixels, camera input, accessibility, and a write from another client refreshing the view.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use jd_core::{Context, Registry, Request};
use jd_ipc::{Client, ClientConfig, Server};
use jd_studio::{Options, StudioApp};
use serde_json::json;

/// An engine with a small project and a generated `height` layer.
fn engine(dir: &std::path::Path) -> Arc<Server> {
    let server = Server::bind(Registry::with_builtins(), Context::default()).expect("bind");
    let world = json!({ "resolution_px": [256, 256], "extent_m": [2048.0, 2048.0], "tile_size": 256, "halo": 0, "origin": [0.0, 0.0], "seed": 7 });
    for (op, params) in [
        (
            "project.create",
            json!({ "path": dir.join("Kit.jdproj"), "name": "Kittest", "world": world }),
        ),
        (
            "terrain.generate",
            json!({ "noise": { "height_range_m": [100.0, 600.0] } }),
        ),
    ] {
        let r = server.invoke(Request::new(op, params));
        assert!(r.error.is_none(), "{op}: {:?}", r.error);
    }
    Arc::new(server)
}

fn window(server: Arc<Server>) -> Harness<'static, StudioApp> {
    Harness::builder()
        .with_size([1200.0, 800.0])
        .wgpu()
        .with_context_setup(jd_studio::theme::install)
        .build_eframe(move |cc| StudioApp::new(cc, server, Options::default()))
}

/// Step until `done` holds (the engine answers on worker threads).
fn settle(h: &mut Harness<'static, StudioApp>, done: impl Fn(&StudioApp) -> bool) {
    let until = Instant::now() + Duration::from_secs(60);
    while !(done(h.state()) && !h.state().busy()) {
        assert!(Instant::now() < until, "the window did not settle");
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    h.run_steps(2);
}

/// Share of viewport pixels that are not the background colour.
fn covered(h: &mut Harness<'static, StudioApp>) -> (f32, Vec<[u8; 4]>) {
    let img = h.render().expect("render");
    let r = h.state().viewport.rect();
    let bg = jd_studio::theme::color::BG0;
    let mut px = Vec::new();
    for y in (r.top() as u32 + 1..r.bottom() as u32 - 1).step_by(4) {
        for x in (r.left() as u32 + 1..r.right() as u32 - 1).step_by(4) {
            px.push(img.get_pixel(x, y).0);
        }
    }
    let off = px.iter().filter(|p| {
        (i32::from(p[0]) - i32::from(bg.r())).abs()
            + (i32::from(p[1]) - i32::from(bg.g())).abs()
            + (i32::from(p[2]) - i32::from(bg.b())).abs()
            > 24
    });
    (off.count() as f32 / px.len() as f32, px)
}

#[test]
fn the_window_shows_the_project_renders_its_terrain_and_orbits() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    assert_eq!(h.state().layer(), Some("height"));
    // The desktop's header readout and toolbar (a tooltip is the button's accessible name).
    h.get_by_label("Kittest — 256² · 2048 m");
    h.get_by_label("Generate terrain (⌘G)");
    h.get_by_label("Open the node editor");
    let info = h.state().viewport.info().unwrap().clone();
    assert_eq!((info.size, info.extent_m), ([256, 256], [2048.0, 2048.0]));

    let (share, before) = covered(&mut h);
    assert!(
        share > 0.25,
        "the terrain covers only {:.0} % of the viewport",
        share * 100.0
    );

    // Drag across the viewport: the camera orbits and the scene is rendered again.
    let (theta, renders) = (
        h.state().viewport.camera.theta,
        h.state().viewport.renders(),
    );
    let c = h.state().viewport.rect().center();
    h.drag_at(c);
    h.step();
    for dx in [20.0, 60.0, 120.0] {
        h.hover_at(c + eframe::egui::vec2(dx, 0.0));
        h.step();
    }
    h.drop_at(c + eframe::egui::vec2(120.0, 0.0));
    h.run_steps(2);
    assert!(
        h.state().viewport.camera.theta < theta - 0.5,
        "theta {theta} -> {}",
        h.state().viewport.camera.theta
    );
    assert!(h.state().viewport.renders() > renders);
    let (_, after) = covered(&mut h);
    let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    assert!(
        changed * 10 > before.len(),
        "only {changed} of {} pixels changed",
        before.len()
    );
}

#[test]
fn a_write_from_the_editor_refreshes_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let editor = Client::connect(
        ClientConfig::new(server.socket(), server.token(), "editor"),
        Box::new(|_, _| {}),
        Box::new(|_| {}),
    )
    .expect("connect");
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let r = editor
        .call(Request::new(
            "terrain.generate",
            json!({ "layer": "ridges" }),
        ))
        .expect("call");
    assert!(r.error.is_none(), "{:?}", r.error);
    settle(&mut h, |a| a.layers().iter().any(|l| l == "ridges"));
    // The selection stays where the user left it.
    assert_eq!(h.state().layer(), Some("height"));
}

#[test]
fn the_viewport_chrome_inserts_layers_frames_and_fits() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    // The compass and the ruler read the camera and the layer, as the desktop's.
    h.get_by_label("Fit: back to the home view");
    assert!(h.state().viewport.frame);
    // Frame off and on again: the block and the readout follow.
    h.get_by_label("Frame: draw the world as a block with side walls and a rim")
        .click();
    h.run_steps(2);
    assert!(!h.state().viewport.frame);
    h.get_by_label("Frame: draw the world as a block with side walls and a rim")
        .click();
    h.run_steps(2);
    assert!(h.state().viewport.frame);
    // Horizon: the noise continued over 3 × the world as a 256² skirt; off again removes it.
    h.get_by_label("Horizon: continue the noise beyond the world's border as a coarse skirt (same seed, so it matches at the edge; erosion is not continued)").click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.horizon_px().is_some());
    assert_eq!(h.state().viewport.horizon_px(), Some(256));
    h.get_by_label("Horizon: continue the noise beyond the world's border as a coarse skirt (same seed, so it matches at the edge; erosion is not continued)").click();
    h.run_steps(2);
    assert_eq!(h.state().viewport.horizon_px(), None);
    // Orbit away, then Fit: back to θ 0.6.
    h.state_mut().viewport.camera.theta = 2.0;
    h.get_by_label("Fit: back to the home view").click();
    h.run_steps(2);
    assert_eq!(h.state().viewport.camera.theta, 0.6);
    // The layer strip splices a terrace filter in front of the height output and re-evaluates.
    h.get_by_label("Filter: terraces (jd.filter.terrace)")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| !a.busy());
    let g = probe
        .invoke(Request::new("graph.list", json!({})))
        .result
        .unwrap();
    let nodes = g["nodes"].as_array().unwrap();
    let terrace = nodes
        .iter()
        .find(|n| n["kind"] == "jd.filter.terrace@1")
        .expect("terrace node");
    let output = nodes
        .iter()
        .find(|n| n["kind"].as_str().unwrap().starts_with("jd.output.height@"))
        .unwrap();
    let edges = g["edges"].as_array().unwrap();
    assert!(
        edges
            .iter()
            .any(|e| e["from"]["node"] == terrace["id"] && e["to"]["node"] == output["id"]),
        "{edges:?}"
    );
    assert!(
        edges
            .iter()
            .any(|e| e["to"]["node"] == terrace["id"] && e["to"]["port"] == "in"),
        "{edges:?}"
    );
    assert!(
        h.state()
            .log_lines()
            .any(|l| l.text == "graph.evaluate: ok")
    );
    // Snow goes through the core's shared op: a snowpack beside the height, `snow` published
    // and its material in the look — the same graph the desktop's button builds.
    h.get_by_label("Simulation layer: snow").click();
    h.run_steps(1);
    settle(&mut h, |a| !a.busy());
    assert!(
        h.state()
            .log_lines()
            .any(|l| l.text == "graph.add_simulation: ok")
    );
    let info = probe
        .invoke(Request::new("project.info", json!({})))
        .result
        .unwrap();
    assert_eq!(info["look"]["materials"][0]["mask"], "snow");
    assert!(
        h.state().layers().iter().any(|l| l.starts_with("snow")),
        "{:?}",
        h.state().layers()
    );
}

/// `JD_STUDIO_CAPTURE=<png> cargo test -p jd-studio --test window -- --ignored capture`: the
/// window rendered offscreen (no screen capture involved) on the project the desktop's
/// `e2e/parity-capture.spec.ts` builds — same world, seed and starter graph — at the same
/// 1440 × 900, for side-by-side parity checks and the user guide.
#[test]
#[ignore = "writes a parity / documentation image"]
fn capture_for_docs() {
    let out = std::env::var("JD_STUDIO_CAPTURE").expect("JD_STUDIO_CAPTURE=<path.png>");
    let dir = tempfile::tempdir().unwrap();
    let server = Server::bind(Registry::with_builtins(), Context::default()).expect("bind");
    // The desktop's New: world 4096 m, 1024 px, tile 512, halo 4, seed 42; then Generate.
    let world = json!({ "resolution_px": [1024, 1024], "extent_m": [4096.0, 4096.0], "tile_size": 512, "halo": 4, "origin": [0, 0], "seed": 42 });
    for (op, params) in [
        (
            "project.create",
            json!({ "path": dir.path().join("Hills.jdproj"), "name": "Hills", "world": world }),
        ),
        ("graph.evaluate", json!({ "lod": 0 })),
    ] {
        let r = server.invoke(Request::new(op, params));
        assert!(r.error.is_none(), "{op}: {:?}", r.error);
    }
    let server = Arc::new(server);
    let mut h = Harness::builder()
        .with_size([1440.0, 900.0])
        .wgpu()
        .with_context_setup(jd_studio::theme::install)
        .build_eframe(move |cc| StudioApp::new(cc, server, Options::default()));
    settle(&mut h, |a| {
        a.viewport.info().is_some()
            && a.viewport.maps_px().is_some()
            && a.viewport.sky_px().is_some()
    });
    h.render().expect("render").save(&out).expect("save png");
    // The horizon skirt (JD_STUDIO_HORIZON=1): `<capture>-horizon.png` beside it.
    if std::env::var_os("JD_STUDIO_HORIZON").is_some() {
        h.get_by_label("Horizon: continue the noise beyond the world's border as a coarse skirt (same seed, so it matches at the edge; erosion is not continued)").click();
        h.run_steps(1);
        settle(&mut h, |a| a.viewport.horizon_px().is_some());
        let out = out.replace(".png", "-horizon.png");
        h.render().expect("render").save(&out).expect("save png");
    }
}
