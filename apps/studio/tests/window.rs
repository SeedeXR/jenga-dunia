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
use serde_json::{Value, json};

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
        // Engine replies land on worker threads and each asks for a repaint, so `run` may need
        // more than kittest's default 4 steps to settle on a slow runner (CI saw 10).
        .with_max_steps(64)
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

/// The window as an image. kittest waits 10 s for the GPU, for everything queued; on a software
/// rasteriser (WARP on the Windows runner) a test that re-rendered the viewport for dozens of
/// steps can queue more than that (CI: "PollError … Wait timed out"). The work keeps running, so
/// wait again rather than fail.
fn render(h: &mut Harness<'static, StudioApp>) -> image::RgbaImage {
    let mut last = String::new();
    for _ in 0..4 {
        match h.render() {
            Ok(img) => return img,
            Err(e) if e.contains("PollError") => last = e,
            Err(e) => panic!("render: {e}"),
        }
    }
    panic!("render: {last}")
}

/// Scroll a control into view (the side columns scroll), click it, and step once.
fn tap(
    h: &mut Harness<'static, StudioApp>,
    role: Option<eframe::egui::accesskit::Role>,
    label: &str,
) {
    let by = || {
        let b = egui_kittest::kittest::by().label(label);
        match role {
            Some(r) => b.role(r),
            None => b,
        }
    };
    h.get(by()).scroll_to_me();
    h.run();
    h.get(by()).click();
    h.run_steps(1);
}

/// A primary-button drag on the viewport from `a` to `b` (press, a few moves, release), as a
/// stroke with a ground tool.
fn drag_on(h: &mut Harness<'static, StudioApp>, a: eframe::egui::Pos2, b: eframe::egui::Pos2) {
    h.hover_at(a);
    h.run_steps(1);
    let button = |pos, pressed| eframe::egui::Event::PointerButton {
        pos,
        button: eframe::egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    h.event(button(a, true));
    h.run_steps(1);
    for k in 1..=6 {
        h.hover_at(a + (b - a) * (k as f32 / 6.0));
        h.run_steps(1);
    }
    h.event(button(b, false));
    h.run_steps(1);
}

/// Share of viewport pixels that are not the background colour.
fn covered(h: &mut Harness<'static, StudioApp>) -> (f32, Vec<[u8; 4]>) {
    let img = render(h);
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
    // The layout is stable: no column grows frame after frame (a widget asking for more than the
    // available width once made the left panel creep to its maximum, squeezing the viewport).
    let rect = h.state().viewport.rect();
    h.run_steps(30);
    assert_eq!(
        h.state().viewport.rect(),
        rect,
        "the viewport moved: a side column is growing"
    );

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

#[test]
fn the_sun_card_writes_the_look_and_toggles_the_maps() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| {
        a.viewport.info().is_some() && a.viewport.maps_px().is_some()
    });
    // Fog to 30 %: a keyboard edit on the slider (each arrow is one 0.05 step).
    h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, "Fog 0 %")
        .focus();
    h.run();
    for _ in 0..6 {
        h.key_press(eframe::egui::Key::ArrowRight);
        h.run();
    }
    // The desktop's 400 ms debounce, then the look on disk carries it.
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        let info = probe
            .invoke(Request::new("project.info", json!({})))
            .result
            .unwrap();
        if info["look"]["sun"]["fog"]
            .as_f64()
            .is_some_and(|f| (f - 0.3).abs() < 1e-6)
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "the look was not written: {}",
            info["look"]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(&mut h, |a| {
        a.log_lines().any(|l| l.text == "project.set_look: ok")
    });
    // An HDRI on the dome: bake one (sun from the west, 20° up), load it, take its sun, clear it.
    let exr = dir.path().join("west.exr");
    let r = probe.invoke(Request::new(
        "sky.bake",
        json!({ "sun_azimuth_deg": 270.0, "sun_elevation_deg": 20.0, "width": 256, "path": exr }),
    ));
    assert!(r.error.is_none(), "{:?}", r.error);
    // The field holds `sky.exr`; select-all-and-type is awkward in kittest, so set it directly.
    h.state_mut().set_hdri_path(&exr.display().to_string());
    h.get_by_label("Load").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("sky.load_hdri: ok"))
    });
    assert!(h.state().viewport_readout().contains(" · hdri"));
    h.get_by_label("Use sun").click();
    h.run_steps(2);
    // The brightest pixel of a 256-wide map sits within a degree of the baked sun.
    let readout = h.state().viewport_readout();
    assert!(
        ["sun 19°/270°", "sun 20°/270°", "sun 21°/270°"]
            .iter()
            .any(|t| readout.contains(t)),
        "{readout}"
    );
    h.get_by_label("×").click();
    h.run();
    assert!(!h.state().viewport_readout().contains(" · hdri"));
    // PBR off: the ramp colours (no maps); on again: the maps come back.
    h.get_by_role_and_label(eframe::egui::accesskit::Role::CheckBox, "Textured (PBR)")
        .click();
    h.run_steps(2);
    assert!(h.state().viewport.maps_px().is_none());
    h.get_by_role_and_label(eframe::egui::accesskit::Role::CheckBox, "Textured (PBR)")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.maps_px().is_some());
}

#[test]
fn the_post_sliders_darken_the_corners_bloom_and_expose() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    // The maps too: a bake landing between two measurements would change the shading under them.
    settle(&mut h, |a| {
        a.viewport.info().is_some() && a.viewport.maps_px().is_some()
    });
    // Mean brightness of the viewport's corner and centre boxes (a tenth of its width).
    let mean = |h: &mut Harness<'static, StudioApp>| {
        let img = render(h);
        let r = h.state().viewport.rect().shrink(2.0);
        let d = r.width() / 10.0;
        let boxed = |x0: f32, y0: f32| {
            let mut sum = 0.0;
            let mut n = 0.0;
            for y in (y0 as u32..(y0 + d) as u32).step_by(2) {
                for x in (x0 as u32..(x0 + d) as u32).step_by(2) {
                    let p = img.get_pixel(x, y).0;
                    sum += f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2]);
                    n += 3.0;
                }
            }
            sum / n
        };
        let c = r.center();
        (
            boxed(r.left(), r.bottom() - d),
            boxed(c.x - d / 2.0, c.y - d / 2.0),
        )
    };
    let nudge = |h: &mut Harness<'static, StudioApp>, label: &str, steps: usize| {
        h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, label)
            .focus();
        h.run();
        for _ in 0..steps {
            h.key_press(eframe::egui::Key::ArrowRight);
            h.run();
        }
    };
    let (_, centre) = mean(&mut h);
    assert!(!h.state().viewport_readout().contains(" · post"));
    // Exposure 1.5 (ACES from here on, as with any effect): the centre brightens.
    nudge(&mut h, "Exposure 1", 10);
    let (corner, centre2) = mean(&mut h);
    assert!(centre2 > centre * 1.05, "centre {centre} -> {centre2}");
    // Vignette 0.5: the corner darkens; the centre, inside its 0.35 radius, keeps its light.
    nudge(&mut h, "Vignette 0", 10);
    let (corner2, centre3) = mean(&mut h);
    // At the box (uv distance ≈ 0.64) the factor is ≈ 0.74 linear, ≈ 0.87 once sRGB-encoded.
    assert!(corner2 < corner * 0.9, "corner {corner} -> {corner2}");
    assert!(
        (centre3 - centre2).abs() < 1.0,
        "centre {centre2} -> {centre3}"
    );
    nudge(&mut h, "Bloom 0", 6);
    let readout = h.state().viewport_readout();
    assert!(
        readout.contains(" · post bloom 0.3 · vignette 0.5 · exposure 1.5"),
        "{readout}"
    );
    h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, "Bloom 0.3");
}

#[test]
fn the_camera_row_keys_records_and_plays_a_flythrough() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    let home = h.state().viewport.camera;
    // Two keys: the home view, then a turned and closer one.
    h.get_by_label("+ key").click();
    h.run();
    let mut turned = home;
    turned.theta += 1.0;
    turned.radius *= 0.5;
    h.state_mut().viewport.set_camera(turned);
    h.run();
    h.get_by_label("+ key").click();
    h.run();
    h.get_by_label("2 keys");
    // The desktop's key line: heading (from south towards east) · elevation · distance.
    h.get_by_label("326° · 33° up · 2.7 km");
    // Go to the first key.
    h.get_all_by_label("Go").next().unwrap().click();
    h.run_steps(1);
    assert_eq!(h.state().viewport.camera, home);
    // Record the default 48 frames: every one written, the camera back where it was.
    h.get_by_label("Record").click();
    let until = Instant::now() + Duration::from_secs(120);
    while !h
        .state()
        .log_lines()
        .any(|l| l.text.starts_with("flythrough:"))
    {
        assert!(Instant::now() < until, "the flythrough did not finish");
        h.step();
    }
    let line = h
        .state()
        .log_lines()
        .find(|l| l.text.starts_with("flythrough:"))
        .unwrap()
        .text
        .clone();
    assert!(line.starts_with("flythrough: 48/48 frames"), "{line}");
    let frames = std::fs::read_dir(dir.path().join("Kit.jdproj/exports/flythrough"))
        .unwrap()
        .count();
    assert_eq!(frames, 48);
    assert_eq!(h.state().viewport.camera, home);
    // Play: 4 s along the path, ending on the last key.
    h.get_by_label("Play").click();
    let until = Instant::now() + Duration::from_secs(20);
    // (the radius goes through log space, so equal up to rounding)
    let at_end = |c: jd_studio::viewport::Camera| {
        (c.theta - turned.theta).abs() < 1e-4 && (c.radius / turned.radius - 1.0).abs() < 1e-4
    };
    while !at_end(h.state().viewport.camera) {
        assert!(Instant::now() < until, "Play did not reach the last key");
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    // Remove one key, then drop the rest.
    h.get_by_label("Remove key 2").click();
    h.run();
    h.get_by_label("1 keys");
    h.get_by_label("Drop the flythrough keys").click();
    h.run();
    h.get_by_label("0 keys");
}

#[test]
fn the_scatter_section_places_saves_and_exports_instances() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    // Mask: the height layer (above 1 everywhere, so every 40 m cell keeps its candidate).
    h.get_by_role_and_label(eframe::egui::accesskit::Role::ComboBox, "Mask")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(eframe::egui::accesskit::Role::ComboBox, "Mask")
        .click();
    h.run();
    h.get_by_label("height").click();
    h.run();
    // The section is below the Sun card: scroll the right column to it, as a user would.
    h.get_by_label("Place").scroll_to_me();
    h.run();
    h.get_by_label("Place").click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.instance_count() > 0);
    let n = h.state().viewport.instance_count();
    // 2048 m at 40 m: 51 or 52 cells a side.
    assert!((2601..=2704).contains(&n), "{n} instances");
    let readout = h.state().viewport_readout();
    assert!(
        readout.contains(&format!(" · {} instances", thousands(n))),
        "{readout}"
    );
    h.get_by_label(&format!("{} instances", thousands(n)));
    // The look on disk carries the mask after the 400 ms debounce.
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        let info = probe
            .invoke(Request::new("project.info", json!({})))
            .result
            .unwrap();
        if info["look"]["scatter"]["mask"] == "height" {
            assert_eq!(info["look"]["scatter"]["spacing_m"], 40.0);
            break;
        }
        assert!(
            Instant::now() < until,
            "the scatter look was not written: {}",
            info["look"]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Export .glb: the proxy instanced into exports/height.glb.
    h.get_by_label("Export .glb").scroll_to_me();
    h.run();
    h.get_by_label("Export .glb").click();
    h.run_steps(1);
    // Place logged one `terrain.scatter: ok`; the export is the second.
    settle(&mut h, |a| {
        a.log_lines()
            .filter(|l| l.text.starts_with("terrain.scatter: ok"))
            .count()
            >= 2
    });
    assert!(dir.path().join("Kit.jdproj/exports/height.glb").exists());
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[test]
fn the_colour_picker_draws_a_layer_on_the_terrain_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    // A second published layer: snow beside the height.
    for (op, params) in [
        (
            "graph.add_simulation",
            json!({ "which": "snow", "max_height_m": 600.0 }),
        ),
        ("graph.evaluate", json!({ "lod": 0 })),
    ] {
        let r = server.invoke(Request::new(op, params));
        assert!(r.error.is_none(), "{op}: {:?}", r.error);
    }
    let mut h = window(server);
    settle(&mut h, |a| {
        a.viewport.maps_px().is_some() && a.layers().iter().any(|l| l == "snow")
    });
    let (_, before) = covered(&mut h);
    let colour = || eframe::egui::accesskit::Role::ComboBox;
    h.get_by_role_and_label(colour(), "colour").click();
    h.run();
    h.get_by_label("snow").click();
    h.run();
    settle(&mut h, |a| a.viewport.overlay_on());
    let readout = h.state().viewport_readout();
    assert!(readout.contains(" · colour snow"), "{readout}");
    // The overlay wins over the maps, as the desktop's readout shows.
    assert!(!readout.contains(" · pbr"), "{readout}");
    let (_, after) = covered(&mut h);
    let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    assert!(
        changed * 4 > before.len(),
        "only {changed} of {} pixels changed",
        before.len()
    );
    // Back to the height ramp: no overlay, the maps shade again.
    h.get_by_role_and_label(colour(), "colour").click();
    h.run();
    h.get_by_label("height ramp").click();
    h.run();
    assert!(!h.state().viewport.overlay_on());
    assert!(h.state().viewport.maps_px().is_some());
    assert!(!h.state().viewport_readout().contains(" · colour"));
}

#[test]
fn the_caves_section_adds_shows_x_rays_maps_and_exports_a_cave() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    // Add cave (karst, the first kind): the node goes in, the graph evaluates, the cave is meshed.
    h.get_by_label("Add cave").scroll_to_me();
    h.run();
    h.get_by_label("Add cave").click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.cave_triangles() > 0);
    let readout = h.state().viewport_readout();
    let tris = h.state().viewport.cave_triangles();
    assert!(
        readout.contains(&format!(" · caves {} tris", thousands(tris as usize))),
        "{readout}"
    );
    // The cave's line: `Karst (limestone) · 1.24 km · 2 entrances …`.
    h.get(egui_kittest::kittest::by().label_contains("Karst (limestone) · "))
        .scroll_to_me();
    h.get(egui_kittest::kittest::by().label_contains(" km · "));
    // Place: a click on the terrain adds an entrance there (the ground under the pointer).
    let probe_entrances = |h: &Harness<'static, StudioApp>| h.state().cave_entrances(0);
    let before = probe_entrances(&h);
    tap(
        &mut h,
        Some(eframe::egui::accesskit::Role::Button),
        "Place entrances of cave 1",
    );
    assert!(h.state().viewport.tool.is_some(), "the tool is armed");
    let c = h.state().viewport.rect().center();
    let ground = h.state().viewport.pick(c).expect("ground under the centre");
    h.hover_at(c);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(eframe::egui::Event::PointerButton {
            pos: c,
            button: eframe::egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    settle(&mut h, |a| a.cave_entrances(0).len() == before.len() + 1);
    let added = *probe_entrances(&h).last().unwrap();
    assert!(
        (added[0] - f64::from(ground[0])).abs() < 1.0
            && (added[1] - f64::from(ground[1])).abs() < 1.0,
        "{added:?} vs {ground:?}"
    );
    tap(
        &mut h,
        Some(eframe::egui::accesskit::Role::Button),
        "Place entrances of cave 1",
    );
    assert!(h.state().viewport.tool.is_none(), "placing stopped");
    // X-ray 20 %: the readout says so and the terrain turns see-through.
    let (_, before) = covered(&mut h);
    // Scrolled into view first: egui gives no focus to a widget clipped out of its column.
    h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, "X-ray 0 %")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, "X-ray 0 %")
        .focus();
    h.run();
    for _ in 0..4 {
        h.key_press(eframe::egui::Key::ArrowRight);
        h.run();
    }
    assert!(
        h.state().viewport_readout().contains(" · x-ray 20 %"),
        "{}",
        h.state().viewport_readout()
    );
    let (_, after) = covered(&mut h);
    let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    assert!(
        changed * 10 > before.len(),
        "only {changed} of {} pixels changed",
        before.len()
    );
    // Footprint: the cave_plan layer as the colour.
    h.get_by_label("Footprint").scroll_to_me();
    h.run();
    h.get_by_label("Footprint").click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.overlay_on());
    assert!(h.state().viewport_readout().contains(" · colour cave_plan"));
    h.get_by_label("Footprint on");
    // .glb: the walls and speleothems into exports/cave_1.glb.
    h.get_by_label(".glb").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("caves.export: ok"))
    });
    assert!(dir.path().join("Kit.jdproj/exports/cave_1.glb").exists());
}

#[test]
fn actions_and_simulation_layers_add_their_nodes_in_one_click() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let kinds = || -> Vec<(String, Value)> {
        let g = probe
            .invoke(Request::new("graph.list", json!({})))
            .result
            .unwrap();
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| (n["kind"].as_str().unwrap().to_owned(), n["params"].clone()))
            .collect()
    };
    // Flip X: a jd.filter.action node in front of the height output, evaluated.
    h.get_by_label("Flip X").scroll_to_me();
    h.run();
    h.get_by_label("Flip X").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines().any(|l| l.text == "graph.evaluate: ok")
    });
    assert!(
        kinds()
            .iter()
            .any(|(k, p)| k == "jd.filter.action@1" && p["action"] == "flip_x"),
        "{:?}",
        kinds()
    );
    // + snow: the snowpack beside the height, `snow` published and shown as the colour.
    h.get_by_label("+ snow").scroll_to_me();
    h.run();
    h.get_by_label("+ snow").click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.overlay_on());
    assert!(kinds().iter().any(|(k, _)| k == "jd.climate.snowpack@1"));
    let readout = h.state().viewport_readout();
    assert!(readout.contains(" · colour snow"), "{readout}");
    // No refresh ran between adding the node and evaluating it (a map bake would ask for the
    // `snow` mask before it is published): the log holds no engine error.
    let errors: Vec<_> = h
        .state()
        .log_lines()
        .filter(|l| l.text.contains(": E_"))
        .map(|l| l.text.clone())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn the_materials_section_edits_the_stack_and_rebakes_the_maps() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.maps_px().is_some());
    let look = || {
        probe
            .invoke(Request::new("project.info", json!({})))
            .result
            .unwrap()["look"]["materials"]
            .clone()
    };
    // + layer: a grey, rough layer everywhere; the maps bake with it and the look stores it.
    h.get_by_label("+ layer").scroll_to_me();
    h.run();
    h.get_by_label("+ layer").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines().any(|l| {
            l.text.ends_with("with 1 material layer") || l.text.contains("with 1 material layer in")
        })
    });
    let until = Instant::now() + Duration::from_secs(20);
    while look().as_array().map_or(0, Vec::len) != 1 {
        assert!(
            Instant::now() < until,
            "the material was not written: {}",
            look()
        );
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(look()[0]["roughness"], 0.9);
    assert!(look()[0]["mask"].is_null());
    // Its mask: the height layer.
    h.get_by_role_and_label(eframe::egui::accesskit::Role::ComboBox, "Material 1 mask")
        .click();
    h.run();
    h.get_by_label("height").click();
    h.run();
    let until = Instant::now() + Duration::from_secs(20);
    while look()[0]["mask"] != "height" {
        assert!(
            Instant::now() < until,
            "the mask was not written: {}",
            look()
        );
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    // Remove it: the stack is empty again.
    h.get_by_label("Remove material 1").scroll_to_me();
    h.run();
    h.get_by_label("Remove material 1").click();
    h.run_steps(1);
    let until = Instant::now() + Duration::from_secs(20);
    while look().as_array().map_or(1, Vec::len) != 0 {
        assert!(
            Instant::now() < until,
            "the removal was not written: {}",
            look()
        );
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_noise_section_generates_tunes_octaves_and_measures() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let noise = || -> Value {
        let g = probe
            .invoke(Request::new("graph.list", json!({})))
            .result
            .unwrap();
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"].as_str().unwrap().starts_with("jd.noise."))
            .unwrap()["params"]
            .clone()
    };
    let combo = eframe::egui::accesskit::Role::ComboBox;
    // Preset Ridged alpine, then Generate: the panel goes into the noise node, the graph evaluates.
    h.get_by_role_and_label(combo, "Noise preset")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(combo, "Noise preset").click();
    h.run();
    h.get_by_label("Ridged alpine 0..1200 m").click();
    h.run();
    h.get_by_label("Generate").scroll_to_me();
    h.run();
    h.get_by_label("Generate").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines().any(|l| l.text == "graph.evaluate: ok")
    });
    let p = noise();
    assert_eq!(
        (p["fractal"].as_str(), p["octaves"].as_u64()),
        (Some("ridged"), Some(7)),
        "{p}"
    );
    assert_eq!(p["height_range_m"], json!([0.0, 1200.0]));
    // Strength: one keyboard step up, applied on release.
    h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, "Strength")
        .focus();
    h.run();
    h.key_press(eframe::egui::Key::ArrowRight);
    h.run_steps(1);
    settle(&mut h, |_| {
        (noise()["strength"].as_f64().unwrap_or(1.0) - 1.05).abs() < 1e-9
    });
    // The octave bars are for fbm; back to Mountains (fbm, 6 octaves) and tune the first.
    h.get_by_role_and_label(combo, "Noise preset")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(combo, "Noise preset").click();
    h.run();
    h.get_by_label("Mountains 0..500 m").click();
    h.run();
    h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, "octave 1 strength")
        .focus();
    h.run();
    h.key_press(eframe::egui::Key::ArrowDown);
    h.run_steps(1);
    settle(&mut h, |_| {
        noise()["octave_weights"]
            .as_array()
            .is_some_and(|w| w.len() == 6)
    });
    assert!((noise()["octave_weights"][0].as_f64().unwrap() - 0.95).abs() < 1e-9);
    h.get_by_label("reset").click();
    h.run_steps(1);
    settle(&mut h, |_| noise()["octave_weights"].is_null());
    // The layer's stats, then Measure landform.
    h.get_by_label("min / max");
    h.get_by_label("Measure landform").scroll_to_me();
    h.run();
    h.get_by_label("Measure landform").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("terrain.measure: ok"))
    });
    h.get_by_label("hypsometric");
}

#[test]
fn the_export_section_writes_every_kind_of_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    let exports = dir.path().join("Kit.jdproj/exports");
    // Two timeline frames, not the default 24: each is a full evaluation, which takes CI runners
    // longer than the wait below.
    let frames = || {
        egui_kittest::kittest::by()
            .role(eframe::egui::accesskit::Role::TextInput)
            .label("Frames")
    };
    h.get(frames()).scroll_to_me();
    h.run();
    h.get(frames()).focus();
    h.run();
    for _ in 0..2 {
        h.key_press(eframe::egui::Key::Backspace);
    }
    h.get(frames()).type_text("2");
    h.run();
    for (button, expect) in [
        ("Export heightmap", "height.exr"),
        ("Export", "unity"),
        ("Bake", "sky.exr"),
        ("Export frames", "frames"),
    ] {
        // By role: the section's heading is labelled `Export` too.
        let by = || {
            egui_kittest::kittest::by()
                .role(eframe::egui::accesskit::Role::Button)
                .label(button)
        };
        h.get_all(by()).next().unwrap().scroll_to_me();
        h.run();
        h.get_all(by()).next().unwrap().click();
        h.run_steps(1);
        let until = Instant::now() + Duration::from_secs(30);
        while !exports.join(expect).exists() || h.state().busy() {
            assert!(
                Instant::now() < until,
                "{button}: no exports/{expect}; log: {:?}",
                h.state()
                    .log_lines()
                    .map(|l| l.text.clone())
                    .collect::<Vec<_>>()
            );
            h.step();
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    // The mesh row's Export (the second `Export`): terrain.glb.
    h.get_all(
        egui_kittest::kittest::by()
            .role(eframe::egui::accesskit::Role::Button)
            .label("Export"),
    )
    .nth(1)
    .unwrap()
    .click();
    h.run_steps(1);
    settle(&mut h, |_| exports.join("terrain.glb").exists());
    assert!(std::fs::read_dir(exports.join("frames")).unwrap().count() >= 2);
    let errors: Vec<_> = h
        .state()
        .log_lines()
        .filter(|l| l.text.contains(": E_"))
        .map(|l| l.text.clone())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// A loopback Terrarium server (`/t/{z}/{x}/{y}.png`, height 1000 m + 0.01 m per row), as the
/// core's and the desktop's e2e tests use; returns its base URL.
fn tile_server() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let target = req
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_owned();
            let ty: u32 = target
                .trim_end_matches(".png")
                .rsplit('/')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let mut img = image::RgbImage::new(256, 256);
            for (_, py, p) in img.enumerate_pixels_mut() {
                let v = 1000.0 + (ty * 256 + py) as f32 * 0.01 + 32768.0;
                let r = (v / 256.0).floor();
                *p = image::Rgb([r as u8, (v - r * 256.0).floor() as u8, 0]);
            }
            let mut body = Vec::new();
            img.write_to(
                &mut std::io::Cursor::new(&mut body),
                image::ImageFormat::Png,
            )
            .unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://127.0.0.1:{port}/t")
}

#[test]
fn real_world_fetches_a_dem_and_wires_it_into_the_height() {
    // The core reads the provider override from the environment, which a test may not set in
    // its own process (no unsafe): run this test again as a child with it set.
    const NAME: &str = "real_world_fetches_a_dem_and_wires_it_into_the_height";
    if std::env::var_os("JD_GEO_TERRARIUM_URL").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--test-threads=1"])
            .env("JD_GEO_TERRARIUM_URL", tile_server())
            .status()
            .unwrap();
        assert!(status.success(), "the child run failed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    h.get_by_label("Fetch the DEM").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines().any(|l| l.text == "graph.evaluate: ok")
    });
    let errors: Vec<_> = h
        .state()
        .log_lines()
        .filter(|l| l.text.contains(": E_"))
        .map(|l| l.text.clone())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    // A heightmap node now feeds the height output, and the world sits in the DEM's CRS.
    let g = probe
        .invoke(Request::new("graph.list", json!({})))
        .result
        .unwrap();
    let dem = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "jd.input.heightmap@1")
        .expect("a heightmap node");
    let out = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "jd.output.height@1")
        .unwrap();
    let feeds: Vec<_> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["to"]["node"] == out["id"])
        .collect();
    assert_eq!(feeds.len(), 1);
    assert_eq!(feeds[0]["from"]["node"], dem["id"]);
    let info = probe
        .invoke(Request::new("project.info", json!({})))
        .result
        .unwrap();
    assert!(
        info["world"]["crs"]
            .as_str()
            .is_some_and(|c| c.starts_with("EPSG:")),
        "{}",
        info["world"]
    );
}

#[test]
fn the_reference_section_reads_an_image() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    // A plain PNG: no GPS, no time — the sun needs both.
    let png = dir.path().join("ref.png");
    image::RgbImage::new(64, 48).save(&png).unwrap();
    h.get_by_label("Reference file").focus();
    h.run();
    h.get_by_label("Reference file")
        .type_text(&png.display().to_string());
    h.run();
    h.get_by_label("Read").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("media.metadata: ok"))
    });
    h.get_by_label("no GPS");
    h.get_by_label("needs place and time");
}

#[test]
fn settings_commit_on_leaving_language_tasks_and_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let settings = || {
        probe
            .invoke(Request::new("project.info", json!({})))
            .result
            .unwrap()["settings"]
            .clone()
    };
    let before = settings()["autosave_interval_s"].as_u64().unwrap();
    // Autosave: typed, then committed when the field is left (the desktop's onBlur).
    h.get_by_role_and_label(eframe::egui::accesskit::Role::TextInput, "Autosave (s)")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(eframe::egui::accesskit::Role::TextInput, "Autosave (s)")
        .focus();
    h.run();
    h.get_by_role_and_label(eframe::egui::accesskit::Role::TextInput, "Autosave (s)")
        .type_text("0");
    h.run();
    assert_eq!(
        settings()["autosave_interval_s"].as_u64().unwrap(),
        before,
        "not before leaving"
    );
    h.key_press(eframe::egui::Key::Tab);
    h.run_steps(1);
    settle(&mut h, |_| {
        settings()["autosave_interval_s"].as_u64() == Some(before * 10)
    });
    // Kiswahili: the section titles follow.
    h.get_by_role_and_label(eframe::egui::accesskit::Role::ComboBox, "Language")
        .click();
    h.run();
    h.get_by_label("Kiswahili").click();
    h.run();
    h.get_by_label("Mipangilio");
    h.get_by_label("Jua");
    // Tasks idle; Diagnostics' environment line.
    h.get_by_label("no running jobs");
    h.get_by_label("Environment").scroll_to_me();
    h.run();
    h.get_by_label("Environment").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines().any(|l| l.text.starts_with("diag.env: ok"))
    });
    h.get_by_label("host");
}

#[test]
fn the_layer_stack_adds_bypasses_reorders_and_removes() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let graph = || {
        probe
            .invoke(Request::new("graph.list", json!({})))
            .result
            .unwrap()
    };
    let node = |kind: &str| {
        graph()["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == kind)
            .cloned()
    };
    let feed = |to: &Value| {
        graph()["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["to"]["node"] == *to && e["to"]["port"] == "in")
            .map(|e| e["from"]["node"].clone())
    };
    let button = eframe::egui::accesskit::Role::Button;
    let out = node("jd.output.height@1").unwrap();
    // `+` splices the default kind (erosion.thermal), then a terrace on top of it.
    tap(
        &mut h,
        None,
        "Splice the chosen node in front of the height output",
    );
    settle(&mut h, |_| node("jd.erosion.thermal@1").is_some());
    tap(
        &mut h,
        Some(eframe::egui::accesskit::Role::ComboBox),
        "Layer kind",
    );
    h.run();
    // The kind list scrolls inside its popup.
    h.get_by_label("filter.terrace").scroll_to_me();
    h.run();
    h.get_by_label("filter.terrace").click();
    h.run();
    tap(
        &mut h,
        None,
        "Splice the chosen node in front of the height output",
    );
    settle(&mut h, |_| node("jd.filter.terrace@1").is_some());
    let (thermal, terrace) = (
        node("jd.erosion.thermal@1").unwrap()["id"].clone(),
        node("jd.filter.terrace@1").unwrap()["id"].clone(),
    );
    assert_eq!(
        (feed(&out["id"]), feed(&terrace)),
        (Some(terrace.clone()), Some(thermal.clone()))
    );
    // The eye bypasses the terrace.
    tap(&mut h, Some(button), "Bypass filter.terrace");
    settle(&mut h, |_| {
        node("jd.filter.terrace@1").unwrap()["bypass"] == true
    });
    // Down: applied before the thermal; up: back on top.
    tap(&mut h, Some(button), "Move filter.terrace down");
    settle(&mut h, |_| feed(&out["id"]) == Some(thermal.clone()));
    assert_eq!(feed(&thermal), Some(terrace.clone()));
    tap(&mut h, Some(button), "Move filter.terrace up");
    settle(&mut h, |_| feed(&out["id"]) == Some(terrace.clone()));
    // The thermal sits right on the noise: down would feed a generator — refused, nothing moves.
    tap(&mut h, Some(button), "Move erosion.thermal down");
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("graph.swap_in_chain: E_"))
    });
    assert_eq!(feed(&out["id"]), Some(terrace.clone()));
    // The bin closes the gap.
    tap(&mut h, Some(button), "Remove filter.terrace");
    settle(&mut h, |_| node("jd.filter.terrace@1").is_none());
    assert_eq!(feed(&out["id"]), Some(thermal));
}

#[test]
fn sculpt_paints_strokes_and_transforms_and_the_road_lays_a_spline() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let node = |kind: &str| {
        let g = probe
            .invoke(Request::new("graph.list", json!({})))
            .result
            .unwrap();
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == kind)
            .cloned()
    };
    let button = eframe::egui::accesskit::Role::Button;
    // Sculpt: the strokes node goes in front of the output, the brush and its panel arm.
    h.get_by_label("Sculpt: adds a stroke node before the output and paints with the brush")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.tool.is_some());
    assert!(node("jd.sculpt.strokes@1").is_some());
    h.get_by_role_and_label(eframe::egui::accesskit::Role::ComboBox, "Brush mode");
    assert_eq!(h.state().viewport.tool, Some(150.0), "the brush radius");
    // A drag is one raise stroke.
    let r = h.state().viewport.rect();
    let (a, b) = (
        r.center() - eframe::egui::vec2(40.0, 0.0),
        r.center() + eframe::egui::vec2(40.0, 0.0),
    );
    drag_on(&mut h, a, b);
    settle(&mut h, |_| {
        node("jd.sculpt.strokes@1").is_some_and(|n| {
            n["params"]["strokes"]
                .as_array()
                .is_some_and(|s| s.len() == 1)
        })
    });
    let stroke = node("jd.sculpt.strokes@1").unwrap()["params"]["strokes"][0].clone();
    assert_eq!(
        (stroke["mode"].as_str(), stroke["radius_m"].as_f64()),
        (Some("raise"), Some(150.0)),
        "{stroke}"
    );
    assert!(stroke["points"].as_array().unwrap().len() >= 2, "{stroke}");
    // Ctrl / ⌘ + wheel over the brush sizes it (×1.1).
    h.hover_at(r.center());
    h.event(eframe::egui::Event::Zoom(1.2));
    h.run_steps(2);
    assert_eq!(h.state().viewport.tool, Some(165.0));
    // Rotate: the drag becomes an op of a transforms node.
    tap(
        &mut h,
        Some(eframe::egui::accesskit::Role::ComboBox),
        "Brush mode",
    );
    h.run();
    h.get_by_label("rotate").click();
    h.run();
    drag_on(&mut h, a, b);
    settle(&mut h, |_| {
        node("jd.sculpt.transforms@1")
            .is_some_and(|n| n["params"]["ops"].as_array().is_some_and(|o| o.len() == 1))
    });
    assert_eq!(
        node("jd.sculpt.transforms@1").unwrap()["params"]["ops"][0]["kind"],
        "rotate"
    );
    // Sculpt again: back to orbiting.
    h.get_by_role_and_label(
        button,
        "Sculpting: drag on the terrain to paint a stroke (click to stop)",
    )
    .click();
    h.run();
    assert!(h.state().viewport.tool.is_none());
    // Road: one drag lays a spline node, then the tool is off.
    h.get_by_role_and_label(button, "Road: the next drag on the terrain becomes a spline node that flattens a road bed along it").click();
    h.run();
    assert_eq!(h.state().viewport.tool, Some(6.0), "half the road's width");
    drag_on(&mut h, a, b);
    settle(&mut h, |_| node("jd.terrain.spline@1").is_some());
    let road = node("jd.terrain.spline@1").unwrap();
    assert_eq!(road["params"]["width_m"], 12.0);
    assert!(h.state().viewport.tool.is_none(), "one drag, one road");
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
        .with_max_steps(64)
        .wgpu()
        .with_context_setup(jd_studio::theme::install)
        .build_eframe(move |cc| StudioApp::new(cc, server, Options::default()));
    settle(&mut h, |a| {
        a.viewport.info().is_some()
            && a.viewport.maps_px().is_some()
            && a.viewport.sky_px().is_some()
    });
    render(&mut h).save(&out).expect("save png");
    // The horizon skirt (JD_STUDIO_HORIZON=1): `<capture>-horizon.png` beside it.
    if std::env::var_os("JD_STUDIO_HORIZON").is_some() {
        h.get_by_label("Horizon: continue the noise beyond the world's border as a coarse skirt (same seed, so it matches at the edge; erosion is not continued)").click();
        h.run_steps(1);
        settle(&mut h, |a| a.viewport.horizon_px().is_some());
        let out = out.replace(".png", "-horizon.png");
        render(&mut h).save(&out).expect("save png");
    }
    // Scatter (JD_STUDIO_SCATTER=1), as the desktop's JD_PARITY_SCATTER: `<capture>-scatter.png`.
    if std::env::var_os("JD_STUDIO_SCATTER").is_some() {
        let mask = || eframe::egui::accesskit::Role::ComboBox;
        h.get_by_role_and_label(mask(), "Mask").scroll_to_me();
        h.run();
        h.get_by_role_and_label(mask(), "Mask").click();
        h.run();
        h.get_by_label("height").click();
        h.run();
        h.get_by_label("Place").scroll_to_me();
        h.run();
        h.get_by_label("Place").click();
        h.run_steps(1);
        settle(&mut h, |a| a.viewport.instance_count() > 0);
        let out = out.replace(".png", "-scatter.png");
        render(&mut h).save(&out).expect("save png");
    }
    // A cave (JD_STUDIO_CAVE=1), as the desktop's JD_PARITY_CAVE: `<capture>-cave.png`.
    if std::env::var_os("JD_STUDIO_CAVE").is_some() {
        h.get_by_label("Add cave").scroll_to_me();
        h.run();
        h.get_by_label("Add cave").click();
        h.run_steps(1);
        settle(&mut h, |a| a.viewport.cave_triangles() > 0);
        h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, "X-ray 0 %")
            .focus();
        h.run();
        for _ in 0..4 {
            h.key_press(eframe::egui::Key::ArrowRight);
            h.run();
        }
        let out = out.replace(".png", "-cave.png");
        render(&mut h).save(&out).expect("save png");
    }
    // The Noise section (JD_STUDIO_NOISE=1), as the desktop's JD_PARITY_NOISE: `<capture>-noise.png`.
    if std::env::var_os("JD_STUDIO_NOISE").is_some() {
        h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, "octave 1 strength")
            .scroll_to_me();
        h.run();
        let out = out.replace(".png", "-noise.png");
        render(&mut h).save(&out).expect("save png");
    }
    // The post effects (JD_STUDIO_POST=1), as the desktop's JD_PARITY_POST: `<capture>-post.png`.
    if std::env::var_os("JD_STUDIO_POST").is_some() {
        h.state_mut().viewport.set_post(jd_studio::post::Post {
            exposure: 1.5,
            bloom: 0.6,
            vignette: 0.5,
        });
        h.run_steps(2);
        let out = out.replace(".png", "-post.png");
        render(&mut h).save(&out).expect("save png");
    }
}

/// `JD_STUDIO_CAPTURE=<dir>/studio-sea.png cargo test -p jd-studio --test window -- --ignored
/// capture_sea`: the desktop parity spec's sea project (a sea-level node at 180 m on the noise).
#[test]
#[ignore = "writes a parity image"]
fn capture_sea() {
    let out = std::env::var("JD_STUDIO_CAPTURE").expect("JD_STUDIO_CAPTURE=<path.png>");
    let dir = tempfile::tempdir().unwrap();
    let server = Server::bind(Registry::with_builtins(), Context::default()).expect("bind");
    let ok = |op: &str, params: serde_json::Value| {
        let r = server.invoke(Request::new(op, params));
        assert!(r.error.is_none(), "{op}: {:?}", r.error);
        r.result.unwrap()
    };
    let world = json!({ "resolution_px": [1024, 1024], "extent_m": [4096.0, 4096.0], "tile_size": 512, "halo": 4, "origin": [0, 0], "seed": 42 });
    ok(
        "project.create",
        json!({ "path": dir.path().join("Sea.jdproj"), "name": "Sea", "world": world }),
    );
    let g = ok("graph.list", json!({}));
    let noise = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"].as_str().unwrap().starts_with("jd.noise."))
        .unwrap()["id"]
        .clone();
    let sea = ok(
        "graph.add_node",
        json!({ "kind": "jd.hydrology.sea_level@1", "params": { "level_m": 180 } }),
    )["id"]
        .clone();
    ok(
        "graph.connect",
        json!({ "from": noise, "from_port": "out", "to": sea, "to_port": "in" }),
    );
    ok("graph.evaluate", json!({ "lod": 0 }));
    let server = Arc::new(server);
    let mut h = Harness::builder()
        .with_size([1440.0, 900.0])
        .with_max_steps(64)
        .wgpu()
        .with_context_setup(jd_studio::theme::install)
        .build_eframe(move |cc| StudioApp::new(cc, server, Options::default()));
    settle(&mut h, |a| {
        a.viewport.maps_px().is_some()
            && a.viewport.sky_px().is_some()
            && a.viewport.sea_info().is_some()
    });
    render(&mut h).save(&out).expect("save png");
}
