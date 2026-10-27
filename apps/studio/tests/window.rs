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

mod common;
use common::*;

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
fn tasks_and_diagnostics_work_before_a_project_is_open() {
    let server = Server::bind(Registry::with_builtins(), Context::default()).expect("bind");
    let mut h = window(Arc::new(server));
    h.run();
    // The desktop's Tasks and Diagnostics sit outside its project block; the project's own
    // sections (Import, Settings) wait for one.
    h.get_by_label("no running jobs");
    h.get_by_label("Environment");
    assert!(h.query_by_label("Settings").is_none());
}

#[test]
fn the_editors_shortcuts_run_the_toolbar_commands() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    let logged =
        |h: &Harness<'static, StudioApp>, t: &str| h.state().log_lines().any(|l| l.text == t);
    assert!(!logged(&h, "project.save: ok"));
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::S);
    settle(&mut h, |a| {
        a.log_lines().any(|l| l.text == "project.save: ok")
    });
    // ⌘R evaluates the graph, as the toolbar's Evaluate.
    let before = h
        .state()
        .log_lines()
        .filter(|l| l.text == "graph.evaluate: ok")
        .count();
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::R);
    settle(&mut h, |a| {
        a.log_lines()
            .filter(|l| l.text == "graph.evaluate: ok")
            .count()
            > before
    });
}

#[test]
fn the_side_panels_collapse_to_a_strip_and_come_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    let wide = h.state().viewport.rect().width();
    h.get_by_label("Collapse the project panel to the edge")
        .click();
    h.get_by_label("Collapse the inspector to the edge").click();
    h.run();
    assert!(
        h.query_by_label("no running jobs").is_none(),
        "the left panel is still open"
    );
    // The viewport takes the room the panels gave up.
    assert!(h.state().viewport.rect().width() > wide + 400.0);
    h.get_by_label("Show the project panel").click();
    h.get_by_label("Show the inspector").click();
    h.run();
    h.get_by_label("no running jobs");
    assert!((h.state().viewport.rect().width() - wide).abs() < 1.0);
}

#[test]
fn detail_tiles_blend_over_the_maps_by_their_weights() {
    use jd_studio::viewport::{Detail, Image, Maps};
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.maps_px().is_some());
    let flat = |px: u32, rgba: [u8; 4]| Image {
        size: [px, px],
        rgba: rgba.repeat((px * px) as usize),
    };
    let maps = |details: Vec<Detail>, weight: u8| Maps {
        albedo: flat(64, [128, 128, 128, 255]),
        normal: flat(64, [128, 128, 255, 255]),
        roughness: flat(64, [255, 230, 0, 255]),
        emissive: None,
        details,
        detail_weights: Some(flat(64, [weight, 0, 0, 0])),
    };
    // A black-and-white checker tile, fully weighted: it must show.
    let checker = Image {
        size: [8, 8],
        rgba: (0..64)
            .flat_map(|i| {
                if (i % 8 + i / 8) % 2 == 0 {
                    [0u8, 0, 0, 255]
                } else {
                    [255, 255, 255, 255]
                }
            })
            .collect(),
    };
    let tile = Detail {
        albedo: checker,
        normal: flat(8, [128, 128, 255, 255]),
        tile_m: 64.0,
    };
    let shot = |h: &mut Harness<'static, StudioApp>, m: Maps| {
        h.state_mut().viewport.set_maps(Some(m));
        h.run_steps(3);
        let rect = h.state().viewport.rect();
        let img = render(h);
        let ppp = img.width() as f32 / 1200.0;
        image::imageops::crop_imm(
            &img,
            (rect.left() * ppp) as u32,
            (rect.top() * ppp) as u32,
            (rect.width() * ppp) as u32,
            (rect.height() * ppp) as u32,
        )
        .to_image()
    };
    let plain = shot(&mut h, maps(vec![tile.clone()], 0));
    let detailed = shot(&mut h, maps(vec![tile], 255));
    let changed = plain
        .pixels()
        .zip(detailed.pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0).any(|(x, y)| x.abs_diff(y) > 16))
        .count();
    assert!(
        changed * 10 > (plain.width() * plain.height()) as usize,
        "the weighted tile changed only {changed} of {} pixels",
        plain.width() * plain.height()
    );
}

#[test]
fn number_fields_step_with_the_arrow_keys() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    let spacing = |h: &Harness<'static, StudioApp>| {
        h.get_by_role_and_label(eframe::egui::accesskit::Role::TextInput, "Spacing (m)")
            .value()
    };
    assert_eq!(spacing(&h).as_deref(), Some("40"));
    h.get_by_role_and_label(eframe::egui::accesskit::Role::TextInput, "Spacing (m)")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(eframe::egui::accesskit::Role::TextInput, "Spacing (m)")
        .click();
    h.run();
    h.key_press(eframe::egui::Key::ArrowUp);
    h.run();
    assert_eq!(spacing(&h).as_deref(), Some("41"));
    h.key_press_modifiers(eframe::egui::Modifiers::SHIFT, eframe::egui::Key::ArrowDown);
    h.run();
    assert_eq!(spacing(&h).as_deref(), Some("31"));
}

/// e2e/jobs.spec.ts in Studio: a long evaluation shows in the header with Cancel, which stops
/// it with E_CANCELLED; Esc (outside a text field) aborts the next one too.
#[test]
fn a_running_job_shows_in_the_header_and_cancel_or_esc_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    // A very long hydraulic run in front of the output (the node's maximum iterations).
    let r = probe.invoke(Request::new(
        "graph.splice",
        json!({ "layer": "height", "kind": "jd.erosion.hydraulic@1", "params": { "iterations": 2000 } }),
    ));
    assert!(r.error.is_none(), "{:?}", r.error);
    settle(&mut h, |_| true);
    let cancelled = |a: &StudioApp| {
        a.log_lines()
            .filter(|l| l.text.contains("E_CANCELLED"))
            .count()
    };
    let running = |h: &mut Harness<'static, StudioApp>| {
        let until = Instant::now() + Duration::from_secs(30);
        while h.query_by_label("Esc to abort").is_none() {
            assert!(Instant::now() < until, "no job in the header");
            h.step();
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::R);
    running(&mut h);
    h.get(egui_kittest::kittest::by().label_contains("graph.evaluate · node "));
    h.get_all_by_label("Cancel").next().unwrap().click();
    settle(&mut h, |a| cancelled(a) == 1);
    assert!(
        h.query_by_label("Esc to abort").is_none(),
        "the job line stays after the cancel"
    );
    // Esc aborts the next one.
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::R);
    running(&mut h);
    h.key_press(eframe::egui::Key::Escape);
    settle(&mut h, |a| cancelled(a) == 2);
}

#[test]
fn the_still_button_writes_the_viewport_under_exports() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    h.get_by_label("Save the viewport as a still (exports/stills/)")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines().any(|l| l.text.starts_with("still: "))
    });
    let stills = dir.path().join("Kit.jdproj/exports/stills");
    let pngs: Vec<_> = std::fs::read_dir(&stills)
        .expect("exports/stills")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    assert_eq!(pngs.len(), 1, "{pngs:?}");
    let png = std::fs::read(&pngs[0]).unwrap();
    assert!(
        png.len() > 1024 && png.starts_with(b"\x89PNG"),
        "{} bytes",
        png.len()
    );
}

/// e2e/desert.spec.ts: the Desert plain preset fills the Noise section, and Generate keeps its
/// 0..40 m (the starter graph's range must not override it).
#[test]
fn the_desert_plain_preset_fills_the_noise_and_generate_keeps_its_range() {
    use eframe::egui::accesskit::Role;
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    tap(&mut h, Some(Role::ComboBox), "Noise preset");
    h.run();
    h.get_by_label("Desert plain 0..40 m").click();
    h.run();
    let field = |h: &Harness<'static, StudioApp>, label: &str| {
        h.get_by_role_and_label(Role::TextInput, label)
            .value()
            .unwrap_or_default()
    };
    assert_eq!(field(&h, "Feature size (m)"), "1200");
    assert_eq!(field(&h, "Octaves"), "3");
    // The Noise section's seed (Scatter has one too, higher up the column).
    let seed = || {
        egui_kittest::kittest::by()
            .role(Role::TextInput)
            .label("Seed")
    };
    h.get_all(seed()).last().unwrap().scroll_to_me();
    h.run();
    h.get_all(seed()).last().unwrap().focus();
    h.run();
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::A);
    h.get_all(seed()).last().unwrap().type_text("3");
    h.run();
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::G);
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("graph.evaluate: ok"))
    });
    settle(&mut h, |_| true);
    let layer = h.state().layer().unwrap().to_owned();
    let stats = probe
        .invoke(Request::new("terrain.stats", json!({ "layer": layer })))
        .result
        .unwrap()["stats"]
        .clone();
    let (lo, hi) = (
        stats["min"].as_f64().unwrap(),
        stats["max"].as_f64().unwrap(),
    );
    assert!(lo >= -0.01 && hi <= 40.01 && hi > 10.0, "{lo}..{hi} m");
}

/// e2e/diagnostics.spec.ts: the environment line, the cache GC report under the project's limit,
/// and the bench's stages.
#[test]
fn diagnostics_report_the_environment_the_cache_and_the_bench() {
    use egui_kittest::kittest::by;
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    for (button, op) in [
        ("Environment", "diag.env: ok"),
        ("Cache GC", "project.cache_gc: ok"),
        ("Bench 256²", "diag.bench: ok"),
    ] {
        h.get_by_label(button).scroll_to_me();
        h.run();
        h.get_by_label(button).click();
        h.run_steps(1);
        settle(&mut h, |a| a.log_lines().any(|l| l.text.starts_with(op)));
    }
    h.run();
    h.get(by().label_contains(" cpus · "));
    h.get(by().label_contains(" blobs · "));
    h.get(by().label_contains("limit 2048 MB"));
    h.get(by().label_contains("S1 256² "));
    h.get(by().label_contains("thermal×50"));
}

/// e2e/timeline.spec.ts: with Grow with timeline the population follows the project's time —
/// nobody at t = 0, more than a hundred by t = 1.
#[test]
fn scatter_grows_with_the_timeline() {
    use eframe::egui::accesskit::Role;
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    tap(&mut h, Some(Role::ComboBox), "Mask");
    h.run();
    h.get_by_label("height").click();
    h.run();
    tap(&mut h, Some(Role::CheckBox), "Grow with timeline");
    h.get_by_label("Place").scroll_to_me();
    h.run();
    h.get_by_label("Place").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("terrain.scatter: ok"))
    });
    assert_eq!(h.state().viewport.instance_count(), 0, "born at t = 0");
    // The Editor moves the timeline and evaluates (its live preview): Studio re-places.
    for (op, params) in [
        ("graph.set_time", json!({ "time": 1.0 })),
        ("graph.evaluate", json!({ "lod": 0 })),
    ] {
        let r = probe.invoke(Request::new(op, params));
        assert!(r.error.is_none(), "{op}: {:?}", r.error);
    }
    settle(&mut h, |a| a.viewport.instance_count() > 100);
}

/// e2e/terrain.spec.ts in Studio: the sun, sky, clouds and fog readouts follow their sliders; a
/// PNG export is 16-bit with W_EXPORT_LOSS; an EXR export comes back through Import as a
/// heightmap node.
#[test]
fn the_sky_readouts_follow_their_sliders_and_heightmaps_go_out_and_back() {
    use eframe::egui::accesskit::Role;
    use egui_kittest::kittest::by;
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.sky_px().is_some());
    let readout = |h: &Harness<'static, StudioApp>| h.state().viewport_readout();
    assert!(readout(&h).contains(" · sun 35°/140°"), "{}", readout(&h));
    assert!(readout(&h).contains(" · sky 512×256"), "{}", readout(&h));
    h.get(by().role(Role::Slider).label("Turbidity 3"));
    // A slider stepped by its keys: Elevation 35° → 12°, Clouds → 60 %, Fog → 50 %.
    let step = |h: &mut Harness<'static, StudioApp>, label: &str, key, n: usize| {
        h.get(by().role(Role::Slider).label(label)).scroll_to_me();
        h.run();
        h.get(by().role(Role::Slider).label(label)).focus();
        h.run();
        for _ in 0..n {
            h.key_press(key);
            h.run();
        }
    };
    step(&mut h, "Elevation 35°", eframe::egui::Key::ArrowLeft, 23);
    assert!(readout(&h).contains(" · sun 12°/140°"), "{}", readout(&h));
    step(&mut h, "Clouds 0 %", eframe::egui::Key::ArrowRight, 12);
    settle(&mut h, |a| {
        a.log_lines().any(|l| l.text.contains("clouds 60 %"))
    });
    step(&mut h, "Fog 0 %", eframe::egui::Key::ArrowRight, 10);
    assert!(readout(&h).contains(" · fog 50 %"), "{}", readout(&h));
    // PNG: 16-bit, with the precision warning in the log.
    let export = |h: &mut Harness<'static, StudioApp>, file: &str| {
        fill(h, "Export file", file);
        h.get_by_label("Export heightmap").click();
        h.run_steps(1);
        settle(h, |_| true);
    };
    export(&mut h, "h.png");
    assert!(
        h.state()
            .log_lines()
            .any(|l| l.text.contains("W_EXPORT_LOSS"))
    );
    let png = std::fs::read(dir.path().join("Kit.jdproj/exports/h.png")).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(png[24], 16, "IHDR bit depth");
    // EXR out, then in through Import: copied into assets/ and a heightmap node reading it.
    export(&mut h, "h.exr");
    let exr = dir.path().join("Kit.jdproj/exports/h.exr");
    assert!(exr.exists());
    fill(&mut h, "Import", &exr.display().to_string());
    h.get_by_label("Import this file (⌘I opens a dialog)")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("project.import_asset: ok"))
    });
    assert!(dir.path().join("Kit.jdproj/assets/h.exr").exists());
    let g = probe
        .invoke(Request::new("graph.list", json!({})))
        .result
        .unwrap();
    assert!(
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["kind"] == "jd.input.heightmap@1" && n["params"]["path"] == "h.exr"),
        "{g}"
    );
}

/// e2e/hydrology.spec.ts and lakes.spec.ts in Studio, the graph edited as the Editor would: a
/// sea-level node puts a sea at its level, its wind raises the waves the readout reports, the water
/// maps export; a fill node published as `lakes` shows standing water.
#[test]
fn the_sea_follows_its_node_and_lakes_show_their_water() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let ok = |op: &str, params: Value| {
        let r = probe.invoke(Request::new(op, params));
        assert!(r.error.is_none(), "{op}: {:?}", r.error);
        r.result.unwrap_or(Value::Null)
    };
    let readout = |h: &Harness<'static, StudioApp>| h.state().viewport_readout();
    let sea = ok(
        "graph.add_node",
        json!({ "kind": "jd.hydrology.sea_level@1", "params": {} }),
    )["id"]
        .clone();
    settle(&mut h, |a| a.viewport_readout().contains(" · sea 0 m"));
    ok(
        "graph.set_param",
        json!({ "id": sea, "key": "level_m", "value": 250.0 }),
    );
    settle(&mut h, |a| a.viewport_readout().contains(" · sea 250 m"));
    // The shoreline is drawn where the sea meets the ground (e2e/chrome.spec.ts), and the ruler
    // spans the shown layer's heights.
    assert!(readout(&h).contains(" · shore "), "{}", readout(&h));
    let layer = h.state().layer().unwrap().to_owned();
    let max = ok("terrain.stats", json!({ "layer": layer }))["stats"]["max"]
        .as_f64()
        .unwrap();
    let range = h.state().viewport.info().unwrap().range_m;
    assert!(
        (f64::from(range[1]) - max).abs() < 0.5,
        "ruler {range:?}, layer max {max}"
    );
    // A 20 m/s gale with unlimited fetch: a 9.8 m / 380 m Pierson–Moskowitz sea.
    ok(
        "graph.set_param",
        json!({ "id": sea, "key": "wind_speed_mps", "value": 20.0 }),
    );
    settle(&mut h, |a| {
        a.viewport_readout().contains(" · waves 9.8 m / 380 m")
    });
    h.get_by_label("Export water maps").scroll_to_me();
    // The waves animate, so the window never settles: step through the scroll's animation.
    h.run_steps(30);
    h.get_by_label("Export water maps").click();
    h.run_steps(1);
    let until = Instant::now() + Duration::from_secs(60);
    while !h
        .state()
        .log_lines()
        .any(|l| l.text.starts_with("ocean.bake: ok"))
    {
        assert!(
            Instant::now() < until,
            "no ocean.bake: {:?}",
            h.state()
                .log_lines()
                .map(|l| l.text.clone())
                .collect::<Vec<_>>()
        );
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        dir.path()
            .join("Kit.jdproj/exports/water/wave_normal.png")
            .exists()
    );
    // Lakes: noise → fill → an output publishing `lakes`, evaluated.
    let g = ok("graph.list", json!({}));
    let noise = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "jd.noise.fbm@1")
        .unwrap()["id"]
        .clone();
    let fill = ok(
        "graph.add_node",
        json!({ "kind": "jd.hydrology.fill@1", "params": {} }),
    )["id"]
        .clone();
    let out = ok(
        "graph.add_node",
        json!({ "kind": "jd.output.height@1", "params": { "layer": "lakes" } }),
    )["id"]
        .clone();
    // The sea node needs its terrain too, or the graph does not evaluate.
    ok(
        "graph.connect",
        json!({ "from": noise, "from_port": "out", "to": sea, "to_port": "in" }),
    );
    ok(
        "graph.connect",
        json!({ "from": noise, "from_port": "out", "to": fill, "to_port": "in" }),
    );
    ok(
        "graph.connect",
        json!({ "from": fill, "from_port": "lakes", "to": out, "to_port": "in" }),
    );
    ok("graph.evaluate", json!({ "lod": 0 }));
    settle(&mut h, |a| a.viewport_readout().contains(" cells"));
    let r = readout(&h);
    assert!(r.contains(" · lakes "), "{r}");
}

/// e2e/look.spec.ts: a project's saved look (sun, scatter, materials) is what the panels show
/// when it is opened.
#[test]
fn an_opened_project_shows_its_saved_look() {
    use eframe::egui::accesskit::Role;
    use egui_kittest::kittest::by;
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    for (op, params) in [
        (
            "project.set_look",
            json!({
                "materials": [{ "mask": null, "albedo": [0.5, 0.4, 0.3], "roughness": 0.9 }],
                "sun": { "azimuth_deg": 200.0, "elevation_deg": 35.0 },
                "scatter": { "mask": "height", "spacing_m": 25.0, "seed": 3 }
            }),
        ),
        ("project.save", json!({})),
        ("project.close", json!({})),
        (
            "project.open",
            json!({ "path": dir.path().join("Kit.jdproj") }),
        ),
    ] {
        let r = server.invoke(Request::new(op, params));
        assert!(r.error.is_none(), "{op}: {:?}", r.error);
    }
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    h.get(by().role(Role::Slider).label("Azimuth 200°"));
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Spacing (m)")
            .value()
            .as_deref(),
        Some("25")
    );
    h.get_by_label("Remove material 1");
}

/// e2e/tree.spec.ts: the layer strip's cone is a shape layer in the stack, removable, while the
/// generator under it is not.
#[test]
fn a_cone_from_the_strip_is_a_removable_shape_layer() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    h.get_by_label("Shape: a cone at the world's centre (jd.shape.primitive)")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("graph.splice: ok"))
    });
    let enabled = |h: &Harness<'static, StudioApp>, label: &str| {
        !format!("{:?}", h.get_by_label(label)).contains("disabled: true")
    };
    assert!(
        enabled(&h, "Remove shape.primitive"),
        "the cone can be removed"
    );
    assert!(!enabled(&h, "Remove Noise"), "the generator cannot");
    tap(
        &mut h,
        Some(eframe::egui::accesskit::Role::Button),
        "Remove shape.primitive",
    );
    let until = Instant::now() + Duration::from_secs(30);
    while h.query_by_label("Remove shape.primitive").is_some() {
        assert!(
            Instant::now() < until,
            "{:?}",
            h.state()
                .log_lines()
                .map(|l| l.text.clone())
                .collect::<Vec<_>>()
        );
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// e2e/graph.spec.ts: the Noise section mirrors the noise node, so an edit made in the Editor
/// shows here and Generate does not revert it.
#[test]
fn the_noise_section_follows_the_editors_edit_and_generate_keeps_it() {
    use eframe::egui::accesskit::Role;
    use egui_kittest::kittest::by;
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let noise = || {
        let g = probe
            .invoke(Request::new("graph.list", json!({})))
            .result
            .unwrap();
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == "jd.noise.fbm@1")
            .cloned()
            .unwrap()
    };
    let r = probe.invoke(Request::new(
        "graph.set_param",
        json!({ "id": noise()["id"], "key": "seed", "value": 9 }),
    ));
    assert!(r.error.is_none(), "{:?}", r.error);
    let seed = |h: &Harness<'static, StudioApp>| {
        h.get_all(by().role(Role::TextInput).label("Seed"))
            .last()
            .unwrap()
            .value()
            .unwrap_or_default()
    };
    // The Noise section is far down the right column: scroll to it.
    h.get_all(by().role(Role::TextInput).label("Seed"))
        .last()
        .unwrap()
        .scroll_to_me();
    h.run();
    // The edit reaches Studio as an event, then a graph re-read: wait for it (a single settle can
    // return before the re-read has started, as under the parallel gate run).
    let until = Instant::now() + Duration::from_secs(20);
    while seed(&h) != "9" {
        assert!(
            Instant::now() < until,
            "the Noise section shows the Editor's seed: {}",
            seed(&h)
        );
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::G);
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("graph.evaluate: ok"))
    });
    assert_eq!(
        noise()["params"]["seed"],
        9,
        "Generate kept the Editor's edit"
    );
}

/// e2e/graph.spec.ts: an unwired node pauses the live preview with an explanation, not an error.
#[test]
fn a_half_wired_graph_pauses_the_live_preview_without_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    // The Editor adds a blur node and has not wired it yet.
    let r = probe.invoke(Request::new(
        "graph.add_node",
        json!({ "kind": "jd.filter.blur@1", "params": {} }),
    ));
    assert!(r.error.is_none(), "{:?}", r.error);
    settle(&mut h, |_| true);
    // An edit here (a terrace from the strip) evaluates the preview, which waits on the blur.
    h.get_by_label("Filter: terraces (jd.filter.terrace)")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("live preview waiting: input `in`"))
    });
    assert!(
        !h.state()
            .log_lines()
            .any(|l| l.text.starts_with("graph.evaluate: E_")),
        "{:?}",
        h.state()
            .log_lines()
            .map(|l| l.text.clone())
            .collect::<Vec<_>>()
    );
}

/// e2e/paint.spec.ts: Paint density puts a 0..1 clamped strokes node in front of the mask's output
/// and arms the brush.
#[test]
fn paint_density_arms_a_clamped_brush_on_the_mask() {
    use eframe::egui::accesskit::Role;
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let r = probe.invoke(Request::new(
        "graph.add_simulation",
        json!({ "which": "snow" }),
    ));
    assert!(r.error.is_none(), "{:?}", r.error);
    let r = probe.invoke(Request::new("graph.evaluate", json!({ "lod": 0 })));
    assert!(r.error.is_none(), "{:?}", r.error);
    settle(&mut h, |a| a.layers().iter().any(|l| l == "snow"));
    tap(&mut h, Some(Role::ComboBox), "Mask");
    h.run();
    h.get_by_label("snow").click();
    h.run();
    tap(&mut h, Some(Role::Button), "Paint density");
    settle(&mut h, |a| a.viewport.tool.is_some());
    let g = probe
        .invoke(Request::new("graph.list", json!({})))
        .result
        .unwrap();
    let strokes = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "jd.sculpt.strokes@1")
        .cloned()
        .expect("a strokes node");
    assert_eq!(strokes["params"]["clamp"], json!([0.0, 1.0]), "{strokes}");
}

/// The status bar (plan P6): the last log line and the viewport readout; the log is closed until
/// the line is clicked.
#[test]
fn the_status_bar_carries_the_last_line_and_opens_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(engine(dir.path()));
    settle(&mut h, |a| a.viewport.info().is_some());
    let last = h.state().log_lines().last().unwrap().text.clone();
    h.get(egui_kittest::kittest::by().label_contains("viewport: wgpu · "));
    assert!(
        h.query_by_label(&last).is_none(),
        "the log is closed: the line is drawn in the status bar, not listed"
    );
    h.get_by_label("Show the log").click();
    h.run();
    h.get_by_label("Hide the log");
    // The log lists it now (its lines start with the local time).
    h.get(egui_kittest::kittest::by().label_contains(&last));
}

/// The floating viewport icons on a narrow window (plan P6): the labelled chips drop to icons and
/// nothing overlaps; the layer rail stays inside the viewport.
#[test]
fn the_viewport_icons_stay_apart_on_a_narrow_window() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let mut h = Harness::builder()
        .with_size([900.0, 560.0])
        .with_max_steps(64)
        .wgpu()
        .with_context_setup(jd_studio::theme::install)
        .build_eframe(move |cc| StudioApp::new(cc, server, Options::default()));
    settle(&mut h, |a| a.viewport.info().is_some());
    let vp = h.state().viewport.rect();
    let chips: Vec<eframe::egui::Rect> = [
        "Fit: back to the home view",
        "Fullscreen viewport (Esc leaves)",
        "Frame: draw the world as a block with side walls and a rim",
        "Grid tool: select cells of the world and set their height, World Creator style",
        "Horizon: continue the noise beyond the world's border as a coarse skirt (same seed, so it matches at the edge; erosion is not continued)",
        "Road: the next drag on the terrain becomes a spline node that flattens a road bed along it",
        "Sculpt: adds a stroke node before the output and paints with the brush",
        "Show the node graph (the Editor window)",
    ]
    .iter()
    // The header's toolbar has a fullscreen button too: take the one over the viewport.
    .map(|t| {
        h.get_all(egui_kittest::kittest::by().label(t))
            .map(|n| n.rect())
            .find(|r| r.top() > vp.top())
            .unwrap_or_else(|| panic!("no chip {t}"))
    })
    .collect();
    for (i, a) in chips.iter().enumerate() {
        assert!(
            vp.contains_rect(*a),
            "chip {i} leaves the viewport: {a:?} in {vp:?}"
        );
        for b in &chips[i + 1..] {
            assert!(!a.intersects(*b), "chips overlap: {a:?} {b:?}");
        }
    }
    // Icons only: the labelled chips are as narrow as the icon ones.
    assert!(chips[3..].iter().all(|r| r.width() <= 30.0), "{chips:?}");
    let rail = h
        .get_by_label("Filter: terraces (jd.filter.terrace)")
        .rect();
    assert!(vp.contains_rect(rail), "{rail:?} in {vp:?}");
}

#[test]
fn a_write_from_the_editor_refreshes_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let seen: Arc<std::sync::Mutex<Vec<Value>>> = Arc::default();
    let s = seen.clone();
    let editor = Client::connect(
        ClientConfig::new(server.socket(), server.token(), "editor"),
        Box::new(move |topic, data| {
            if topic == "selection" {
                s.lock().unwrap().push(data.clone());
            }
        }),
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
    // Selection sync: a node picked in the Editor is selected here …
    let r = editor
        .call(Request::new(
            "ui.select",
            json!({ "nodes": ["01JDGRAPH000000000000N01SE"] }),
        ))
        .expect("ui.select");
    assert!(r.error.is_none(), "{:?}", r.error);
    settle(&mut h, |a| {
        a.selected_node() == Some("01JDGRAPH000000000000N01SE")
    });
    // … and a layer row clicked here reaches the Editor as `selection` from gui-studio.
    tap(&mut h, None, "Noise");
    let until = Instant::now() + Duration::from_secs(10);
    while !seen
        .lock()
        .unwrap()
        .iter()
        .any(|d| d["source"] == "gui-studio")
    {
        assert!(
            Instant::now() < until,
            "no selection from Studio: {:?}",
            seen.lock().unwrap()
        );
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
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
    // 48 rendered frames: on the Windows runner's software rasteriser (WARP), sharing two cores
    // with the other window tests, this took over 120 s.
    let until = Instant::now() + Duration::from_secs(300);
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
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
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
    // A click on the line selects the cave node (the Editor's inspector follows).
    h.get(egui_kittest::kittest::by().label_contains(" km · "))
        .click();
    h.run();
    let picked = h
        .state()
        .selected_node()
        .map(str::to_owned)
        .expect("a node selected");
    let graph = probe
        .invoke(Request::new("graph.list", json!({})))
        .result
        .unwrap();
    let kind = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == picked.as_str())
        .map(|n| n["kind"].as_str().unwrap_or_default().to_owned());
    assert!(kind.is_some_and(|k| k.starts_with("jd.cave")), "{picked}");
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
    // An entrance's × takes it out of the node (e2e/caves.spec.ts).
    let n = h.state().cave_entrances(0).len();
    tap(
        &mut h,
        Some(eframe::egui::accesskit::Role::Button),
        &format!("Remove entrance {n} of cave 1"),
    );
    settle(&mut h, |a| a.cave_entrances(0).len() == n - 1);
    // Show caves off: the walls leave the viewport and its readout.
    tap(
        &mut h,
        Some(eframe::egui::accesskit::Role::CheckBox),
        "Show caves",
    );
    settle(&mut h, |a| a.viewport.cave_triangles() == 0);
    assert!(
        !h.state().viewport_readout().contains(" · caves "),
        "{}",
        h.state().viewport_readout()
    );
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
    // Its detail tile: the maps re-bake with it.
    h.get_by_label("Material 1 detail").scroll_to_me();
    h.run();
    h.get_by_label("Material 1 detail").click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.detail_tiles() == 1);
    // Its tile: 8 m by default; 4 m re-bakes (e2e/materials-detail.spec.ts).
    let tile = "Material 1 tile (m)";
    assert_eq!(
        h.get_by_role_and_label(eframe::egui::accesskit::Role::TextInput, tile)
            .value()
            .as_deref(),
        Some("8")
    );
    let bakes = |a: &StudioApp| {
        a.log_lines()
            .filter(|l| l.text.starts_with("terrain.textures: ok"))
            .count()
    };
    let before = bakes(h.state());
    fill(&mut h, tile, "4");
    h.key_press(eframe::egui::Key::Tab);
    h.run_steps(1);
    settle(&mut h, |a| bakes(a) > before);
    let until = Instant::now() + Duration::from_secs(20);
    while look()[0]["detail"]["tile_m"] != 4.0 {
        assert!(
            Instant::now() < until,
            "the tile was not written: {}",
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
    // A clip that states where and when it was shot: the place, the time and the sun; Use sun
    // puts the viewport's sun there (e2e/reference.spec.ts — the Namib, a June morning).
    let clip = dir.path().join("dunes.mp4");
    std::fs::write(&clip, namib_clip()).unwrap();
    fill(&mut h, "Reference file", &clip.display().to_string());
    h.get_by_label("Read").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .filter(|l| l.text.starts_with("media.metadata: ok"))
            .count()
            == 2
    });
    use egui_kittest::kittest::by;
    h.get(by().label_contains("2026-06-21T10:00:00Z"));
    h.get(by().label_contains("-24.7500, 15.3500"));
    h.get(by().label_contains("° from north, "));
    // The reference's Use sun (left column); the HDRI row has one too, on the right.
    h.get_all(by().label("Use sun"))
        .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .unwrap()
        .click();
    h.run();
    let degrees = |h: &Harness<'static, StudioApp>, name: &str| -> f64 {
        let n = h.get(
            by().role(eframe::egui::accesskit::Role::Slider)
                .label_contains(name),
        );
        let label = format!("{n:?}");
        let start = label.find(name).unwrap() + name.len();
        label[start..]
            .split('°')
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    };
    let (az, el) = (degrees(&h, "Azimuth "), degrees(&h, "Elevation "));
    assert!(
        az > 0.0 && az < 90.0,
        "a north-eastern morning sun in the southern winter: {az}"
    );
    assert!(el > 20.0 && el < 60.0, "{el}");
}

/// The reference spec's clip: an MP4 that states only where (Android `©xyz`) and when (`mvhd`,
/// 2026-06-21T10:00:00Z) it was shot.
fn namib_clip() -> Vec<u8> {
    let bx = |ty: &[u8], payload: &[u8]| {
        let mut b = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(ty);
        b.extend_from_slice(payload);
        b
    };
    let created: u32 = 3_864_880_800; // seconds since 1904
    let mut mvhd = vec![0u8; 4];
    for v in [created, created, 1000, 5000] {
        mvhd.extend_from_slice(&v.to_be_bytes());
    }
    mvhd.extend_from_slice(&[0u8; 80]);
    let mut xyz = vec![0u8, 18, 0x15, 0xc7];
    xyz.extend_from_slice(b"-24.7500+015.3500/");
    let mut moov = bx(b"mvhd", &mvhd);
    moov.extend(bx(b"udta", &bx(b"\xa9xyz", &xyz)));
    let mut out = bx(b"ftyp", b"isom\0\0\0\0isom");
    out.extend(bx(b"moov", &moov));
    out
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
    let field = |h: &Harness<'static, StudioApp>, label: &str| {
        h.get_by_role_and_label(eframe::egui::accesskit::Role::TextInput, label)
            .value()
            .unwrap_or_default()
    };
    for (label, value) in [
        ("Autosave (s)", "120"),
        ("Cache limit (MB)", "2048"),
        ("Memory budget (GB)", "4"),
    ] {
        assert_eq!(field(&h, label), value, "{label}");
    }
    // Out of range: the engine refuses, the log says why, the field shows the value again.
    fill(&mut h, "Cache limit (MB)", "8");
    h.key_press(eframe::egui::Key::Tab);
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("project.set_settings: E_INPUT_INVALID"))
    });
    h.run();
    assert_eq!(field(&h, "Cache limit (MB)"), "2048");
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
    // Spliced in once and selected, so the inspector shows it (e2e/road.spec.ts).
    settle(&mut h, |a| a.selected_node() == road["id"].as_str());
    assert_eq!(
        h.state()
            .log_lines()
            .filter(|l| l.text.starts_with("graph.splice: ok"))
            .count(),
        1
    );
    assert!(h.state().viewport.tool.is_none(), "one drag, one road");
}

/// e2e/sculpt.spec.ts, alphas.spec.ts and sculptlayer.spec.ts in Studio: the stroke's strength,
/// alpha and hardness, one undo step per stroke, Shift + wheel strength, the right button
/// painting nothing, lower = negative strength, the south half of the screen, height scale 0.
#[test]
fn the_brush_records_its_settings_undoes_and_its_layer_scales_like_the_desktop() {
    use eframe::egui::accesskit::Role;
    let dir = tempfile::tempdir().unwrap();
    let server = engine(dir.path());
    let probe = server.clone();
    let mut h = window(server);
    settle(&mut h, |a| a.viewport.info().is_some());
    let strokes = || -> Vec<Value> {
        let g = probe
            .invoke(Request::new("graph.list", json!({})))
            .result
            .unwrap();
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == "jd.sculpt.strokes@1")
            .and_then(|n| n["params"]["strokes"].as_array().cloned())
            .unwrap_or_default()
    };
    // The shown layer's maximum (a live preview publishes `height#lod<N>`), as the desktop's
    // layer-stats readout.
    // The full-resolution height's mean after evaluating the graph as it stands (the preview
    // the window shows is a coarser LOD, so its stats are not comparable with the start's).
    let mean = || {
        let r = probe.invoke(Request::new("graph.evaluate", json!({ "lod": 0 })));
        assert!(r.error.is_none(), "{:?}", r.error);
        probe
            .invoke(Request::new("terrain.stats", json!({ "layer": "height" })))
            .result
            .unwrap()["stats"]["mean"]
            .as_f64()
            .unwrap()
    };
    let base_mean = mean();
    h.get_by_label("Sculpt: adds a stroke node before the output and paints with the brush")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.tool.is_some());
    fill(&mut h, "Strength (m)", "120");
    h.get_by_role_and_label(
        Role::RadioButton,
        "Crater: a raised rim round a sunken bowl",
    )
    .click();
    h.run();
    let r = h.state().viewport.rect();
    let (a, b) = (
        r.center() - eframe::egui::vec2(60.0, 0.0),
        r.center() + eframe::egui::vec2(60.0, 0.0),
    );
    drag_on(&mut h, a, b);
    settle(&mut h, |_| strokes().len() == 1);
    let s = &strokes()[0];
    assert_eq!(
        (s["strength_m"].as_f64(), s["alpha"].as_str()),
        (Some(120.0), Some("crater")),
        "{s}"
    );
    for p in s["points"].as_array().unwrap() {
        let (u, v) = (p[0].as_f64().unwrap(), p[1].as_f64().unwrap());
        assert!(
            (0.0..=2048.0).contains(&u) && (0.0..=2048.0).contains(&v),
            "{p}"
        );
    }
    // The stroke changes the terrain.
    assert!(
        (mean() - base_mean).abs() > 0.01,
        "the stroke changed nothing"
    );
    // Each stroke is one undo step (⌥⌘Z).
    h.key_press_modifiers(
        eframe::egui::Modifiers::COMMAND | eframe::egui::Modifiers::ALT,
        eframe::egui::Key::Z,
    );
    settle(&mut h, |_| strokes().is_empty());
    // Shift + wheel over the brush lowers its strength; the right button orbits, painting nothing.
    h.hover_at(r.center());
    // Shift held across frames (the window reads the input's modifiers, not the event's).
    h.event(eframe::egui::Event::ModifiersChanged(
        eframe::egui::Modifiers::SHIFT,
    ));
    h.run_steps(1);
    h.event(eframe::egui::Event::MouseWheel {
        unit: eframe::egui::MouseWheelUnit::Point,
        delta: eframe::egui::vec2(0.0, -120.0),
        phase: eframe::egui::TouchPhase::Move,
        source: eframe::egui::MouseWheelSource::Unknown,
        modifiers: eframe::egui::Modifiers::SHIFT,
    });
    h.run_steps(3);
    h.event(eframe::egui::Event::ModifiersChanged(
        eframe::egui::Modifiers::NONE,
    ));
    h.run_steps(1);
    let strength: f64 = h
        .get_by_role_and_label(Role::TextInput, "Strength (m)")
        .value()
        .unwrap()
        .parse()
        .unwrap();
    assert!(strength < 120.0, "{strength}");
    right_drag(&mut h, a, b + eframe::egui::vec2(0.0, 10.0));
    h.run_steps(3);
    assert!(strokes().is_empty(), "the right button painted");
    // Lower: a negative strength; the hardness travels with the stroke; the lower half of the
    // screen is the south of the world (small v).
    h.get_by_role_and_label(Role::ComboBox, "Brush mode")
        .click();
    h.run();
    h.get_by_label("lower").click();
    h.run();
    fill(&mut h, "Strength (m)", "30");
    h.get(
        egui_kittest::kittest::by()
            .role(Role::Slider)
            .label_contains("Hardness "),
    )
    .focus();
    h.run();
    for _ in 0..10 {
        h.key_press(eframe::egui::Key::ArrowRight);
    }
    h.run();
    let low = eframe::egui::vec2(0.0, r.height() * 0.28);
    drag_on(
        &mut h,
        r.center() - eframe::egui::vec2(40.0, 0.0) + low,
        r.center() + eframe::egui::vec2(40.0, 0.0) + low,
    );
    settle(&mut h, |_| strokes().len() == 1);
    let s = &strokes()[0];
    assert_eq!(
        (
            s["mode"].as_str(),
            s["strength_m"].as_f64(),
            s["hardness"].as_f64()
        ),
        (Some("raise"), Some(-30.0), Some(0.5)),
        "{s}"
    );
    for p in s["points"].as_array().unwrap() {
        assert!(p[1].as_f64().unwrap() < 1024.0, "south half: {p}");
    }
    // The layer's Height scale 0 hides what its strokes did (sculptlayer.spec.ts).
    h.get_by_role_and_label(Role::ComboBox, "Brush mode")
        .click();
    h.run();
    h.get_by_label("raise").click();
    h.run();
    fill(&mut h, "Strength (m)", "400");
    drag_on(&mut h, a, b);
    settle(&mut h, |_| strokes().len() == 2);
    assert!(
        (mean() - base_mean).abs() > 0.5,
        "the 400 m stroke changed nothing"
    );
    h.get(egui_kittest::kittest::by().label_contains("area: whole terrain"));
    fill(&mut h, "Height scale", "0");
    h.key_press(eframe::egui::Key::Tab);
    h.run_steps(1);
    let scale = || -> Option<f64> {
        let g = probe
            .invoke(Request::new("graph.list", json!({})))
            .result
            .unwrap();
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == "jd.sculpt.strokes@1")?["params"]["height_scale"]
            .as_f64()
    };
    settle(&mut h, |_| scale() == Some(0.0));
    assert!(
        (mean() - base_mean).abs() < 0.01,
        "height scale 0 still shows the strokes"
    );
    // Push: one drag is a grab op whose vector is the drag (e2e/transforms.spec.ts). The column
    // scrolled down to the layer settings: back up to the brush's mode.
    h.get_by_role_and_label(Role::TextInput, "Strength (m)")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(Role::ComboBox, "Brush mode")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(Role::ComboBox, "Brush mode")
        .click();
    h.run();
    // The eighth of nine modes: scrolled into the popup's view first.
    h.get_by_label("push / pull").scroll_to_me();
    h.run();
    h.get_by_label("push / pull").click();
    h.run();
    h.get(egui_kittest::kittest::by().label_contains("(grab)"));
    drag_on(
        &mut h,
        r.center(),
        r.center() + eframe::egui::vec2(60.0, 0.0),
    );
    let push = || -> Option<Value> {
        let g = probe
            .invoke(Request::new("graph.list", json!({})))
            .result
            .unwrap();
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == "jd.sculpt.transforms@1")?["params"]["ops"]
            .as_array()?
            .last()
            .cloned()
    };
    settle(&mut h, |_| push().is_some_and(|o| o["kind"] == "push"));
    let op = push().unwrap();
    assert!(
        op["vector_m"][0].as_f64().unwrap() > 0.0,
        "a drag to the east: {op}"
    );
}

#[test]
fn the_grid_tool_selects_cells_and_levels_raises_and_transforms_them() {
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
    h.get_by_label(
        "Grid tool: select cells of the world and set their height, World Creator style",
    )
    .click();
    h.run();
    h.get_by_role_and_label(eframe::egui::accesskit::Role::ComboBox, "Cells per side");
    // A drag across the middle selects every cell it crosses.
    let r = h.state().viewport.rect();
    drag_on(
        &mut h,
        r.center() - eframe::egui::vec2(60.0, 0.0),
        r.center() + eframe::egui::vec2(60.0, 0.0),
    );
    h.run();
    let n = h.state().grid_selection().len();
    assert!(n >= 2, "{n} cells");
    h.get_by_label(&format!("{n} cells"));
    // A click on a selected cell toggles it off.
    let c = r.center() + eframe::egui::vec2(60.0, 0.0);
    drag_on(&mut h, c, c);
    h.run();
    assert_eq!(h.state().grid_selection().len(), n - 1);
    // The right button orbits without touching the selection (e2e/grid.spec.ts).
    right_drag(
        &mut h,
        r.center() - eframe::egui::vec2(0.0, 40.0),
        r.center() + eframe::egui::vec2(80.0, -20.0),
    );
    assert_eq!(h.state().grid_selection().len(), n - 1);
    // The full-resolution maximum after evaluating the graph as it stands.
    let max = || {
        let r = probe.invoke(Request::new("graph.evaluate", json!({ "lod": 0 })));
        assert!(r.error.is_none(), "{:?}", r.error);
        probe
            .invoke(Request::new("terrain.stats", json!({ "layer": "height" })))
            .result
            .unwrap()["stats"]["max"]
            .as_f64()
            .unwrap()
    };
    fill(&mut h, "Height (m)", "700");
    // Set height: one flatten stroke over the selected cells; Raise: one more.
    let button = eframe::egui::accesskit::Role::Button;
    tap(&mut h, Some(button), "Set height");
    settle(&mut h, |_| {
        node("jd.sculpt.strokes@1").is_some_and(|s| {
            s["params"]["strokes"]
                .as_array()
                .is_some_and(|a| a.len() == 1)
        })
    });
    let stroke = node("jd.sculpt.strokes@1").unwrap()["params"]["strokes"][0].clone();
    assert_eq!(
        (stroke["mode"].as_str(), stroke["shape"].as_str()),
        (Some("flatten"), Some("square"))
    );
    assert_eq!(stroke["points"].as_array().unwrap().len(), n - 1);
    // Square hard stamps of half a cell, levelled to the height: above the 100..600 m noise, the
    // terrain's maximum is exactly it.
    assert_eq!(
        (stroke["target_m"].as_f64(), stroke["amount"].as_f64()),
        (Some(700.0), Some(1.0)),
        "{stroke}"
    );
    assert_eq!(
        stroke["radius_m"].as_f64(),
        Some(2048.0 / 32.0 / 2.0),
        "{stroke}"
    );
    assert!((max() - 700.0).abs() < 1e-3);
    fill(&mut h, "Step (m)", "25");
    tap(&mut h, Some(button), "Raise");
    settle(&mut h, |_| {
        node("jd.sculpt.strokes@1").is_some_and(|s| {
            s["params"]["strokes"]
                .as_array()
                .is_some_and(|a| a.len() == 2)
        })
    });
    let raised = node("jd.sculpt.strokes@1").unwrap()["params"]["strokes"][1].clone();
    assert_eq!(
        (raised["mode"].as_str(), raised["strength_m"].as_f64()),
        (Some("raise"), Some(25.0)),
        "{raised}"
    );
    assert!((max() - 725.0).abs() < 1e-3);
    // Each grid operation is one undo step (⌥⌘Z): back to 700 m.
    h.key_press_modifiers(
        eframe::egui::Modifiers::COMMAND | eframe::egui::Modifiers::ALT,
        eframe::egui::Key::Z,
    );
    settle(&mut h, |_| {
        node("jd.sculpt.strokes@1").is_some_and(|s| {
            s["params"]["strokes"]
                .as_array()
                .is_some_and(|a| a.len() == 1)
        })
    });
    assert!((max() - 700.0).abs() < 1e-3);
    // Duplicate: the block copied one selection-width east, as a transforms op.
    tap(&mut h, Some(button), "Duplicate");
    settle(&mut h, |_| {
        node("jd.sculpt.transforms@1")
            .is_some_and(|t| t["params"]["ops"].as_array().is_some_and(|o| o.len() == 1))
    });
    let op = node("jd.sculpt.transforms@1").unwrap()["params"]["ops"][0].clone();
    assert_eq!(op["kind"], "duplicate");
    assert!(op["vector_m"][0].as_f64().unwrap() > 0.0, "{op}");
    // Scale (factor 0 flattens the block's relief), Rotate and Align: one op each, in order, all
    // in the one transforms node (e2e/transforms.spec.ts).
    fill(&mut h, "Factor", "0");
    for b in ["Scale", "Rotate", "Align"] {
        tap(&mut h, Some(button), b);
        settle(&mut h, |_| true);
    }
    let ops = node("jd.sculpt.transforms@1").unwrap()["params"]["ops"].clone();
    let kinds: Vec<_> = ops
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["duplicate", "scale", "rotate", "align"], "{ops}");
    assert_eq!(ops[1]["factor"], 0.0, "{ops}");
    tap(&mut h, Some(button), "Clear");
    h.run();
    assert!(h.state().grid_selection().is_empty());
}

/// `JD_STUDIO_CAPTURE=<png> cargo test -p jd-studio --test window -- --ignored capture`: the
/// window rendered offscreen (no screen capture involved) on a fixed project — 4096 m, 1024²,
/// seed 42, the starter graph — at 1440 × 900, for the user guide and visual checks.
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
    // Scatter (JD_STUDIO_SCATTER=1): `<capture>-scatter.png`.
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
    // A cave (JD_STUDIO_CAVE=1): `<capture>-cave.png`.
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
    // The Noise section (JD_STUDIO_NOISE=1): `<capture>-noise.png`.
    if std::env::var_os("JD_STUDIO_NOISE").is_some() {
        h.get_by_role_and_label(eframe::egui::accesskit::Role::Slider, "octave 1 strength")
            .scroll_to_me();
        h.run();
        let out = out.replace(".png", "-noise.png");
        render(&mut h).save(&out).expect("save png");
    }
    // The post effects (JD_STUDIO_POST=1): `<capture>-post.png`.
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
