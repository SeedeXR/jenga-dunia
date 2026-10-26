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
    assert_eq!(
        h.get_all_by_label("Kittest").count(),
        2,
        "top bar and Project panel"
    );
    h.get_by_label("Generate terrain");
    h.get_by_label("Open Editor");
    let info = h.state().viewport.info().unwrap().clone();
    assert_eq!((info.size, info.extent_m), ([256, 256], [2048.0, 2048.0]));

    let (share, before) = covered(&mut h);
    assert!(
        share > 0.25,
        "the terrain covers only {:.0} % of the viewport",
        share * 100.0
    );

    // Drag across the viewport: the camera orbits and the scene is rendered again.
    let (yaw, renders) = (h.state().viewport.camera.yaw, h.state().viewport.renders());
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
        h.state().viewport.camera.yaw < yaw - 0.5,
        "yaw {yaw} -> {}",
        h.state().viewport.camera.yaw
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

/// `JD_STUDIO_CAPTURE=<png> cargo test -p jd-studio --test window -- --ignored capture`: the
/// window rendered offscreen at 2× for the user guide (no screen capture involved).
#[test]
#[ignore = "writes a documentation image"]
fn capture_for_docs() {
    let out = std::env::var("JD_STUDIO_CAPTURE").expect("JD_STUDIO_CAPTURE=<path.png>");
    let dir = tempfile::tempdir().unwrap();
    let server = Server::bind(Registry::with_builtins(), Context::default()).expect("bind");
    let world = json!({ "resolution_px": [1024, 1024], "extent_m": [4096.0, 4096.0], "tile_size": 512, "halo": 0, "origin": [0.0, 0.0], "seed": 42 });
    for (op, params) in [
        (
            "project.create",
            json!({ "path": dir.path().join("Hills.jdproj"), "name": "Hills", "world": world }),
        ),
        (
            "terrain.generate",
            json!({ "noise": { "height_range_m": [40.0, 460.0] } }),
        ),
    ] {
        assert!(
            server.invoke(Request::new(op, params)).error.is_none(),
            "{op}"
        );
    }
    let server = Arc::new(server);
    let mut h = Harness::builder()
        .with_size([1440.0, 900.0])
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(jd_studio::theme::install)
        .build_eframe(move |cc| StudioApp::new(cc, server, Options::default()));
    settle(&mut h, |a| a.viewport.info().is_some());
    h.render().expect("render").save(&out).expect("save png");
}
