//! The user guide's figures, rendered from Studio offscreen (plan P6, S4):
//! `JD_GUIDE_OUT=<dir> cargo test -p jd-studio --test guide -- --ignored --test-threads 1` writes
//! `<dir>/raw/<name>.png` and `<dir>/<name>.annot.json` per figure, then
//! `node apps/desktop/scripts/annotate.mjs <dir>/*.annot.json` draws the callouts into
//! `<dir>/<name>.png`. The callouts are placed on the controls they name (their accessible label),
//! so a layout change moves them with it.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{Rect, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{Queryable, by};
use jd_core::{Context, Registry, Request};
use jd_ipc::Server;
use jd_studio::{Options, StudioApp};
use serde_json::{Value, json};

mod common;
use common::*;

const SIZE: [f32; 2] = [1440.0, 900.0];

/// A guide world: 4096 m at 1024², seed 42, generated — the desktop guide's.
fn world(dir: &std::path::Path) -> Arc<Server> {
    world_of(dir, 4096.0, 1024)
}

/// A generated world of `extent` metres at `res`² (the slow simulations run on smaller grids).
fn world_of(dir: &std::path::Path, extent: f64, res: u32) -> Arc<Server> {
    let server = Server::bind(Registry::with_builtins(), Context::default()).expect("bind");
    let world = json!({ "resolution_px": [res, res], "extent_m": [extent, extent], "tile_size": 512, "halo": 4, "origin": [0, 0], "seed": 42 });
    for (op, params) in [
        (
            "project.create",
            json!({ "path": dir.join("Hills.jdproj"), "name": "Hills", "world": world }),
        ),
        ("graph.evaluate", json!({ "lod": 0 })),
    ] {
        let r = server.invoke(Request::new(op, params));
        assert!(r.error.is_none(), "{op}: {:?}", r.error);
    }
    Arc::new(server)
}

fn studio(server: Arc<Server>) -> Harness<'static, StudioApp> {
    Harness::builder()
        .with_size(SIZE)
        .with_max_steps(64)
        .with_render_cursor(false)
        .wgpu()
        .with_context_setup(jd_studio::theme::install)
        .build_eframe(move |cc| StudioApp::new(cc, server, Options::default()))
}

/// Settle on a shown terrain with its maps and sky.
fn ready(h: &mut Harness<'static, StudioApp>) {
    settle(h, |a| {
        a.viewport.info().is_some()
            && a.viewport.maps_px().is_some()
            && a.viewport.sky_px().is_some()
    });
    h.run_steps(4);
}

/// A callout: the control (its accessible label, or a part of it) and the caption.
struct Callout(&'static str, &'static str);

/// Render the window as `name` with numbered callouts on the controls they name.
fn shot(h: &mut Harness<'static, StudioApp>, name: &str, callouts: &[Callout]) {
    let out = PathBuf::from(std::env::var("JD_GUIDE_OUT").expect("JD_GUIDE_OUT=<dir>"));
    std::fs::create_dir_all(out.join("raw")).unwrap();
    // Nothing running (no "working…" in the header), and a few frames for the camera and fades.
    settle(h, |_| true);
    h.run_steps(4);
    let img = render(h);
    let ppp = img.width() as f32 / SIZE[0];
    let raw = out.join("raw").join(format!("{name}.png"));
    img.save(&raw).unwrap();
    // The largest match: a field over its own label ("Path" names both).
    let rect = |label: &str| -> Rect {
        let exact: Vec<Rect> = h.query_all(by().label(label)).map(|n| n.rect()).collect();
        let all = if exact.is_empty() {
            h.query_all(by().label_contains(label))
                .map(|n| n.rect())
                .collect()
        } else {
            exact
        };
        all.into_iter()
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap_or_else(|| panic!("{name}: no control labelled {label:?}"))
    };
    let annotations: Vec<Value> = callouts
        .iter()
        .enumerate()
        .map(|(i, Callout(label, _what))| {
            let r = rect(label).expand(4.0);
            let [x, y, w, hgt] = [r.left(), r.top(), r.width(), r.height()].map(|v| (v * ppp).round());
            // The badge sits off the box's top-left corner, or inside the image when that corner is
            // at an edge. The callout's meaning is in the guide's numbered list, not on the image.
            let lx = if x < 36.0 { x + w + 18.0 } else { x - 18.0 };
            let ly = if y < 36.0 { y + hgt / 2.0 } else { y - 18.0 };
            json!({ "type": "box", "x": x, "y": y, "w": w, "h": hgt, "label": (i + 1).to_string(), "lx": lx, "ly": ly })
        })

        .collect();
    let spec =
        json!({ "src": raw, "out": out.join(format!("{name}.png")), "annotations": annotations });
    std::fs::write(
        out.join(format!("{name}.annot.json")),
        serde_json::to_vec_pretty(&spec).unwrap(),
    )
    .unwrap();
}

/// Scroll a column to a control and let the scroll settle.
fn show(h: &mut Harness<'static, StudioApp>, label: &str) {
    h.query_all(by().label(label))
        .next()
        .unwrap_or_else(|| panic!("no {label}"))
        .scroll_to_me();
    h.run_steps(8);
}

fn slider(h: &mut Harness<'static, StudioApp>, label: &str, key: eframe::egui::Key, n: usize) {
    h.get(by().role(Role::Slider).label_contains(label))
        .scroll_to_me();
    h.run_steps(4);
    h.get(by().role(Role::Slider).label_contains(label)).focus();
    h.run();
    for _ in 0..n {
        h.key_press(key);
        h.run();
    }
}

#[test]
#[ignore = "writes the user guide's figures (JD_GUIDE_OUT)"]
fn guide_basics() {
    // Empty: no project yet.
    let empty =
        Arc::new(Server::bind(Registry::with_builtins(), Context::default()).expect("bind"));
    let mut h = studio(empty);
    h.run_steps(8);
    shot(&mut h, "desktop-empty", &[]);

    let dir = tempfile::tempdir().unwrap();
    let mut h = studio(world(dir.path()));
    ready(&mut h);
    shot(&mut h, "desktop-generated", &[]);
    shot(
        &mut h,
        "tutorial-01-create-project",
        &[
            Callout("Path", "the project's folder (a .jdproj directory)"),
            Callout("New", "create it with the world below"),
            Callout("Generate terrain (⌘G)", "generate"),
            Callout("Show the log", "what ran, and the log behind it"),
        ],
    );
    show(&mut h, "Export heightmap");
    shot(
        &mut h,
        "tutorial-02-generated",
        &[
            Callout("viewport: wgpu", "the viewport readout"),
            Callout("Measure landform", "the shown layer's statistics"),
            Callout("Export heightmap", "export"),
        ],
    );

    // Settings, scrolled into view in the project column.
    show(&mut h, "Language");
    shot(
        &mut h,
        "desktop-settings",
        &[Callout(
            "Autosave (s)",
            "project settings, committed on leaving a field",
        )],
    );
    show(&mut h, "Project");

    // The project column collapsed to a strip: the viewport takes its room.
    h.get_by_label("Collapse the project panel to the edge")
        .click();
    h.run_steps(8);
    shot(
        &mut h,
        "desktop-docking",
        &[Callout("Show the project panel", "bring the panel back")],
    );
    h.get_by_label("Show the project panel").click();
    h.run_steps(8);

    // A low, warm sun.
    slider(&mut h, "Elevation", eframe::egui::Key::ArrowLeft, 23);
    settle(&mut h, |a| a.viewport.sky_px().is_some());
    h.run_steps(8);
    shot(
        &mut h,
        "desktop-sun",
        &[Callout(
            "Elevation 12°",
            "the sun's height; azimuth above it",
        )],
    );

    // The physical sky: orbit down to the horizon.
    let mut cam = h.state().viewport.camera;
    cam.phi = 1.42;
    h.state_mut().viewport.set_camera(cam);
    h.run_steps(8);
    shot(
        &mut h,
        "desktop-sky-dome",
        &[Callout("Turbidity", "haze of the physical sky")],
    );

    // Textured (PBR), back at the home view and a midday sun.
    h.get_by_label("Fit: back to the home view").click();
    slider(&mut h, "Elevation", eframe::egui::Key::ArrowRight, 23);
    settle(&mut h, |a| a.viewport.maps_px().is_some());
    h.run_steps(8);
    shot(
        &mut h,
        "desktop-pbr",
        &[Callout(
            "Textured (PBR)",
            "baked albedo, normal and roughness maps",
        )],
    );
}

/// An engine op that must succeed; its result.
fn ok(server: &Server, op: &str, params: Value) -> Value {
    let r = server.invoke(Request::new(op, params));
    assert!(r.error.is_none(), "{op}: {:?}", r.error);
    r.result.unwrap_or(Value::Null)
}

/// The first node of `kind` (its id).
fn node(server: &Server, kind: &str) -> Value {
    let g = ok(server, "graph.list", json!({}));
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == kind)
        .unwrap()["id"]
        .clone()
}

fn add(server: &Server, kind: &str, params: Value) -> Value {
    ok(
        server,
        "graph.add_node",
        json!({ "kind": kind, "params": params }),
    )["id"]
        .clone()
}

fn connect(server: &Server, from: &Value, port: &str, to: &Value, to_port: &str) {
    ok(
        server,
        "graph.connect",
        json!({ "from": from, "from_port": port, "to": to, "to_port": to_port }),
    );
}

/// Publish `from.port` as `layer` (a second output node).
fn publish(server: &Server, from: &Value, port: &str, layer: &str) {
    let out = add(server, "jd.output.height@1", json!({ "layer": layer }));
    connect(server, from, port, &out, "in");
}

/// Pick `option` in the combo box labelled `combo`.
fn choose(h: &mut Harness<'static, StudioApp>, combo: &str, option: &str) {
    h.get(by().role(Role::ComboBox).label(combo)).scroll_to_me();
    h.run_steps(4);
    h.get(by().role(Role::ComboBox).label(combo)).click();
    h.run();
    h.get_all(by().label(option)).last().unwrap().click();
    h.run_steps(2);
}

#[test]
#[ignore = "writes the user guide's figures (JD_GUIDE_OUT)"]
fn guide_hydrology() {
    let dir = tempfile::tempdir().unwrap();
    let server = world(dir.path());
    // noise → rivers → height; the rivers mask and a wetness index as their own layers.
    let rivers = ok(
        &server,
        "graph.splice",
        json!({ "layer": "height", "kind": "jd.hydrology.rivers@1", "params": {} }),
    )["id"]
        .clone();
    publish(&server, &rivers, "rivers", "rivers");
    let wet = add(&server, "jd.hydrology.wetness@1", json!({}));
    connect(&server, &rivers, "out", &wet, "in");
    publish(&server, &wet, "out", "wetness");
    ok(&server, "graph.evaluate", json!({ "lod": 0 }));
    let probe = server.clone();
    let mut h = studio(server);
    ready(&mut h);
    shot(&mut h, "desktop-rivers", &[Callout("layer", "")]);
    choose(&mut h, "colour", "rivers");
    settle(&mut h, |a| a.viewport.overlay_on());
    shot(&mut h, "desktop-colour-by-layer", &[Callout("colour", "")]);
    choose(&mut h, "colour", "height ramp");
    // Scatter along the rivers.
    choose(&mut h, "Mask", "rivers");
    fill(&mut h, "Spacing (m)", "20");
    show(&mut h, "Place");
    h.get_by_label("Place").click();
    settle(&mut h, |a| a.viewport.instance_count() > 0);
    shot(
        &mut h,
        "desktop-scatter",
        &[Callout("Mask", ""), Callout("Place", "")],
    );
    // Paint density on the wetness mask: the brush armed on a clamped strokes node.
    choose(&mut h, "Mask", "wetness");
    show(&mut h, "Paint density");
    h.get_by_label("Paint density").click();
    settle(&mut h, |a| a.viewport.tool.is_some());
    // The brush panel opened above the Scatter section: scroll its button back into view.
    show(&mut h, "Paint density");
    shot(
        &mut h,
        "desktop-paint-density",
        &[Callout("Paint density", ""), Callout("Sculpting:", "")],
    );
    h.get_by_label("Sculpting: drag on the terrain to paint a stroke (click to stop)")
        .click();
    h.run_steps(2);

    // A sea at 250 m on the carved terrain; then murky water; then a gale.
    let sea = add(
        &probe,
        "jd.hydrology.sea_level@1",
        json!({ "level_m": 250.0 }),
    );
    connect(&probe, &rivers, "out", &sea, "in");
    settle(&mut h, |a| a.viewport_readout().contains(" · sea 250 m"));
    shot(
        &mut h,
        "desktop-sea-level",
        &[Callout("viewport: wgpu", "")],
    );
    ok(
        &probe,
        "graph.set_param",
        json!({ "id": sea, "key": "visibility_m", "value": 4.0 }),
    );
    ok(
        &probe,
        "graph.set_param",
        json!({ "id": sea, "key": "tint", "value": [0.12, 0.42, 0.36] }),
    );
    // Down to the shore, close enough to read the water's colour and the surf.
    let mut cam = h.state().viewport.camera;
    (cam.radius, cam.phi) = (0.4 * cam.size, 1.2);
    h.state_mut().viewport.set_camera(cam);
    settle(&mut h, |_| true);
    shot(&mut h, "desktop-water-clarity", &[]);
    ok(
        &probe,
        "graph.set_param",
        json!({ "id": sea, "key": "wind_speed_mps", "value": 20.0 }),
    );
    settle(&mut h, |a| a.viewport_readout().contains(" · waves "));
    shot(&mut h, "desktop-waves", &[Callout("viewport: wgpu", "")]);
    h.get_by_label("Fit: back to the home view").click();

    // The seabed under that sea, coloured by class; then the bathymetry's filled floor.
    let seabed = add(&probe, "jd.biome.seabed@1", json!({ "level_m": 250.0 }));
    connect(&probe, &rivers, "out", &seabed, "in");
    publish(&probe, &seabed, "seabed", "seabed");
    ok(
        &probe,
        "graph.set_param",
        json!({ "id": sea, "key": "level_m", "value": 0.0 }),
    );
    ok(&probe, "graph.evaluate", json!({ "lod": 0 }));
    settle(&mut h, |a| a.layers().iter().any(|l| l == "seabed"));
    choose(&mut h, "colour", "seabed");
    settle(&mut h, |a| a.viewport.overlay_on());
    shot(&mut h, "desktop-seabed", &[Callout("colour", "")]);
    let bathy = add(&probe, "jd.ocean.bathymetry@1", json!({ "level_m": 250.0 }));
    connect(&probe, &rivers, "out", &bathy, "in");
    connect(&probe, &seabed, "seabed", &bathy, "seabed");
    publish(&probe, &bathy, "out", "floor");
    ok(&probe, "graph.evaluate", json!({ "lod": 0 }));
    settle(&mut h, |a| a.layers().iter().any(|l| l == "floor"));
    choose(&mut h, "layer", "floor");
    settle(&mut h, |a| a.layer() == Some("floor"));
    shot(&mut h, "desktop-bathymetry", &[Callout("layer", "")]);
}

#[test]
#[ignore = "writes the user guide's figures (JD_GUIDE_OUT)"]
fn guide_layers_and_project() {
    // Snow: a cover mask published beside the height, the terrain coloured by it.
    let dir = tempfile::tempdir().unwrap();
    let server = world(dir.path());
    let noise = node(&server, "jd.noise.fbm@1");
    let snow = add(&server, "jd.climate.snow@1", json!({}));
    connect(&server, &noise, "out", &snow, "in");
    publish(&server, &snow, "snow", "snow");
    ok(&server, "graph.evaluate", json!({ "lod": 0 }));
    let mut h = studio(server);
    ready(&mut h);
    choose(&mut h, "colour", "snow");
    settle(&mut h, |a| a.viewport.overlay_on());
    shot(&mut h, "desktop-snow", &[Callout("colour", "")]);

    // The layer stack: a terrace on the noise, and the snowpack beside the output.
    let dir = tempfile::tempdir().unwrap();
    let server = world(dir.path());
    ok(
        &server,
        "graph.splice",
        json!({ "layer": "height", "kind": "jd.filter.terrace@1", "params": {} }),
    );
    ok(&server, "graph.add_simulation", json!({ "which": "snow" }));
    ok(&server, "graph.evaluate", json!({ "lod": 0 }));
    let mut h = studio(server);
    ready(&mut h);
    show(&mut h, "Search layers");
    shot(
        &mut h,
        "desktop-stack",
        &[
            Callout("Layer kind", ""),
            Callout("Remove filter.terrace", ""),
        ],
    );

    // Tasks and diagnostics with their results.
    show(&mut h, "Environment");
    for (button, op) in [
        ("Environment", "diag.env: ok"),
        ("Cache GC", "project.cache_gc: ok"),
        ("Bench 256²", "diag.bench: ok"),
    ] {
        h.get_by_label(button).click();
        h.run_steps(1);
        settle(&mut h, |a| a.log_lines().any(|l| l.text.starts_with(op)));
    }
    show(&mut h, "Bench 256²");
    shot(
        &mut h,
        "desktop-diagnostics",
        &[Callout("no running jobs", ""), Callout("Environment", "")],
    );

    // Import: an EXR export back as a heightmap node.
    show(&mut h, "Export heightmap");
    h.get_by_label("Export heightmap").click();
    h.run_steps(1);
    settle(&mut h, |_| true);
    let exr = dir.path().join("Hills.jdproj/exports/height.exr");
    fill(&mut h, "Import", &exr.display().to_string());
    h.get_by_label("Import this file (⌘I opens a dialog)")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("project.import_asset: ok"))
    });
    show(&mut h, "Project");
    shot(
        &mut h,
        "desktop-import",
        &[Callout("Import", ""), Callout("Show the log", "")],
    );
}

/// Move the camera: `radius` as a share of the world, `phi` from straight up.
fn frame(h: &mut Harness<'static, StudioApp>, radius: f32, phi: f32) {
    let mut cam = h.state().viewport.camera;
    (cam.radius, cam.phi) = (radius * cam.size, phi);
    h.state_mut().viewport.set_camera(cam);
    h.run_steps(8);
}

#[test]
#[ignore = "writes the user guide's figures (JD_GUIDE_OUT)"]
fn guide_tools() {
    use eframe::egui::vec2;
    let dir = tempfile::tempdir().unwrap();
    let server = world(dir.path());
    let probe = server.clone();
    let mut h = studio(server);
    ready(&mut h);

    // Horizon: the skirt around the world, seen from low down.
    h.get(by().label_contains("Horizon:")).click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport_readout().contains("horizon"));
    frame(&mut h, 1.6, 1.42);
    settle(&mut h, |_| true);
    shot(&mut h, "desktop-horizon", &[Callout("Horizon:", "")]);
    h.get(by().label_contains("Horizon:")).click();
    h.get_by_label("Fit: back to the home view").click();
    h.run_steps(8);

    // Sculpt: the brush armed, one raise stroke across the middle.
    h.get_by_label("Sculpt: adds a stroke node before the output and paints with the brush")
        .click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.tool.is_some());
    let r = h.state().viewport.rect();
    drag_on(
        &mut h,
        r.center() - vec2(120.0, 20.0),
        r.center() + vec2(120.0, 20.0),
    );
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("graph.evaluate: ok"))
    });
    show(&mut h, "Brush mode");
    shot(
        &mut h,
        "tutorial-04-sculpt",
        &[Callout("Sculpting:", ""), Callout("Brush mode", "")],
    );
    h.get_by_label("Sculpting: drag on the terrain to paint a stroke (click to stop)")
        .click();
    h.run_steps(2);

    // Road: one drag lays the spline that flattens its bed, widened to 40 m to read at this scale.
    frame(&mut h, 0.7, 0.95);
    h.get(by().label_contains("Road:")).click();
    h.run();
    let r = h.state().viewport.rect();
    drag_on(
        &mut h,
        r.center() - vec2(260.0, -60.0),
        r.center() + vec2(240.0, -40.0),
    );
    settle(&mut h, |a| a.selected_node().is_some());
    let road = node(&probe, "jd.terrain.spline@1");
    ok(
        &probe,
        "graph.set_param",
        json!({ "id": road, "key": "width_m", "value": 40.0 }),
    );
    ok(&probe, "graph.evaluate", json!({ "lod": 0 }));
    settle(&mut h, |_| true);
    shot(&mut h, "desktop-road", &[Callout("Road:", "")]);
    h.get_by_label("Fit: back to the home view").click();
    h.run_steps(8);

    // Grid: a block of cells of the 16 × 16 grid levelled to 600 m, above the 0..500 m noise.
    h.get(by().label_contains("Grid tool:")).click();
    h.run();
    choose(&mut h, "Cells per side", "16");
    let r = h.state().viewport.rect();
    for dy in [-40.0, 0.0, 40.0] {
        drag_on(
            &mut h,
            r.center() + vec2(-50.0, dy),
            r.center() + vec2(50.0, dy),
        );
    }
    h.run();
    fill(&mut h, "Height (m)", "600");
    h.get_by_label("Set height").click();
    h.run_steps(1);
    settle(&mut h, |_| {
        ok(&probe, "graph.list", json!({}))["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| {
                n["kind"] == "jd.sculpt.strokes@1"
                    && n["params"]["strokes"]
                        .as_array()
                        .is_some_and(|s| s.iter().any(|s| s["shape"] == "square"))
            })
    });
    ok(&probe, "graph.evaluate", json!({ "lod": 0 }));
    let max = ok(&probe, "terrain.stats", json!({ "layer": "height" }))["stats"]["max"].clone();
    assert!(max.as_f64().is_some_and(|m| m > 599.0), "max {max}");
    settle(&mut h, |_| true);
    shot(
        &mut h,
        "desktop-grid",
        &[Callout("Cells per side", ""), Callout("Set height", "")],
    );
}

#[test]
#[ignore = "writes the user guide's figures (JD_GUIDE_OUT)"]
fn guide_layers_caves_and_materials() {
    // A karst cave under the hills, the terrain x-rayed to show it.
    let dir = tempfile::tempdir().unwrap();
    let mut h = studio(world_of(dir.path(), 2048.0, 512));
    ready(&mut h);
    show(&mut h, "Add cave");
    h.get_by_label("Add cave").click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.cave_triangles() > 0);
    slider(&mut h, "X-ray", eframe::egui::Key::ArrowRight, 15);
    show(&mut h, "Add cave");
    shot(
        &mut h,
        "desktop-caves",
        &[Callout("Add cave", ""), Callout("X-ray 75 %", "")],
    );

    // Simulation layers: snow, then lava from the summit, coloured by its heat.
    let dir = tempfile::tempdir().unwrap();
    let mut h = studio(world_of(dir.path(), 2048.0, 256));
    ready(&mut h);
    for which in ["+ snow", "+ lava"] {
        show(&mut h, which);
        h.get_by_label(which).click();
        h.run_steps(1);
        settle(&mut h, |a| a.viewport.overlay_on());
    }
    settle(&mut h, |a| a.layers().iter().any(|l| l == "heat"));
    choose(&mut h, "colour", "heat");
    settle(&mut h, |a| a.viewport.overlay_on());
    show(&mut h, "+ snow");
    shot(
        &mut h,
        "desktop-simlayers",
        &[Callout("+ snow", ""), Callout("colour", "")],
    );

    // Materials: a layer masked by the snow; then its detail tile at 4 m.
    let dir = tempfile::tempdir().unwrap();
    let server = world_of(dir.path(), 2048.0, 512);
    let probe = server.clone();
    let mut h = studio(server);
    ready(&mut h);
    show(&mut h, "+ snow");
    h.get_by_label("+ snow").click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.overlay_on());
    choose(&mut h, "colour", "height ramp");
    show(&mut h, "+ layer");
    h.get_by_label("+ layer").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.contains("with 1 material layer"))
    });
    choose(&mut h, "Material 1 mask", "snow");
    settle(&mut h, |_| {
        ok(&probe, "project.info", json!({}))["look"]["materials"][0]["mask"] == "snow"
    });
    settle(&mut h, |_| true);
    show(&mut h, "+ layer");
    shot(
        &mut h,
        "desktop-materials",
        &[Callout("Material 1 mask", ""), Callout("+ layer", "")],
    );
    show(&mut h, "Material 1 detail");
    h.get_by_label("Material 1 detail").click();
    h.run_steps(1);
    settle(&mut h, |a| a.viewport.detail_tiles() == 1);
    fill(&mut h, "Material 1 tile (m)", "4");
    h.key_press(eframe::egui::Key::Tab);
    h.run_steps(1);
    settle(&mut h, |_| true);
    frame(&mut h, 0.35, 1.1);
    settle(&mut h, |_| true);
    shot(
        &mut h,
        "desktop-materials-detail",
        &[
            Callout("Material 1 detail", ""),
            Callout("Material 1 tile (m)", ""),
        ],
    );

    // Timeline growth: a population placed on the height, grown to t = 1.
    let dir = tempfile::tempdir().unwrap();
    let server = world(dir.path());
    let probe = server.clone();
    let mut h = studio(server);
    ready(&mut h);
    choose(&mut h, "Mask", "height");
    show(&mut h, "Grow with timeline");
    h.get(by().role(Role::CheckBox).label("Grow with timeline"))
        .click();
    h.run();
    h.get_by_label("Place").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("terrain.scatter: ok"))
    });
    ok(&probe, "graph.set_time", json!({ "time": 1.0 }));
    ok(&probe, "graph.evaluate", json!({ "lod": 0 }));
    settle(&mut h, |a| a.viewport.instance_count() > 100);
    show(&mut h, "Grow with timeline");
    shot(
        &mut h,
        "desktop-timeline-growth",
        &[Callout("Grow with timeline", ""), Callout("Place", "")],
    );
}

#[test]
#[ignore = "writes the user guide's figures (JD_GUIDE_OUT)"]
fn guide_desert_and_network() {
    // A desert plain (the panel's preset), dunes blown over it and sinkholes between them.
    let dir = tempfile::tempdir().unwrap();
    let server = world_of(dir.path(), 2048.0, 512);
    let noise = node(&server, "jd.noise.fbm@1");
    let height = node(&server, "jd.output.height@1");
    for (key, value) in [
        ("feature_size_m", json!(1200)),
        ("octaves", json!(3)),
        ("gain", json!(0.5)),
        ("height_range_m", json!([0, 40])),
        ("seed", json!(3)),
    ] {
        ok(
            &server,
            "graph.set_param",
            json!({ "id": noise, "key": key, "value": value }),
        );
    }
    let dunes = add(&server, "jd.erosion.aeolian@1", json!({ "iterations": 60 }));
    let karst = add(
        &server,
        "jd.erosion.karst@1",
        json!({ "density_per_km2": 6, "max_slope_deg": 15 }),
    );
    connect(&server, &noise, "out", &dunes, "in");
    connect(&server, &dunes, "out", &karst, "in");
    connect(&server, &karst, "out", &height, "in");
    publish(&server, &karst, "sinkholes", "sinkholes");
    ok(&server, "graph.evaluate", json!({ "lod": 0 }));
    let mut h = studio(server);
    ready(&mut h);
    choose(&mut h, "colour", "sinkholes");
    settle(&mut h, |a| a.viewport.overlay_on());
    frame(&mut h, 0.5, 1.0);
    settle(&mut h, |_| true);
    shot(&mut h, "desktop-desert", &[Callout("colour", "")]);

    // A drainage network grown first, the terrain made from it, its rivers as a layer.
    let dir = tempfile::tempdir().unwrap();
    let server = world_of(dir.path(), 6400.0, 512);
    let noise = node(&server, "jd.noise.fbm@1");
    let height = node(&server, "jd.output.height@1");
    ok(&server, "graph.remove_node", json!({ "id": noise }));
    let net = add(
        &server,
        "jd.hydrology.network@1",
        json!({ "river_order": 4 }),
    );
    connect(&server, &net, "out", &height, "in");
    publish(&server, &net, "rivers", "rivers");
    ok(&server, "graph.evaluate", json!({ "lod": 0 }));
    let mut h = studio(server);
    ready(&mut h);
    choose(&mut h, "colour", "rivers");
    settle(&mut h, |a| a.viewport.overlay_on());
    shot(&mut h, "desktop-network", &[Callout("colour", "")]);
}

#[test]
#[ignore = "writes the user guide's figures (JD_GUIDE_OUT); fetches public Terrarium tiles"]
fn guide_real_world() {
    // The panel's default point: Nairobi, 2 km around, from AWS Terrain Tiles.
    let dir = tempfile::tempdir().unwrap();
    let mut h = studio(world_of(dir.path(), 2048.0, 512));
    ready(&mut h);
    h.get_by_label("Fetch the DEM").click();
    h.run_steps(1);
    settle(&mut h, |a| {
        a.log_lines()
            .any(|l| l.text.starts_with("graph.evaluate: ok") || l.text.contains(": E_"))
    });
    let errors: Vec<_> = h
        .state()
        .log_lines()
        .filter(|l| l.text.contains(": E_"))
        .map(|l| l.text.clone())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    settle(&mut h, |_| true);
    shot(&mut h, "desktop-geo", &[Callout("Fetch the DEM", "")]);
}
