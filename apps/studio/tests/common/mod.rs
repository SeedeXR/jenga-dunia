//! Helpers the Studio window tests share (`window.rs`, `guide.rs`): an engine with a project, the
//! window under egui_kittest on a real wgpu renderer, settling on the engine's answers, rendering,
//! and the pointer / keyboard gestures the desktop specs used.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use jd_core::{Context, Registry, Request};
use jd_ipc::Server;
use jd_studio::{Options, StudioApp};
use serde_json::json;

/// An engine with a small project and a generated `height` layer.
pub fn engine(dir: &std::path::Path) -> Arc<Server> {
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

pub fn window(server: Arc<Server>) -> Harness<'static, StudioApp> {
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
pub fn settle(h: &mut Harness<'static, StudioApp>, done: impl Fn(&StudioApp) -> bool) {
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
pub fn render(h: &mut Harness<'static, StudioApp>) -> image::RgbaImage {
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
pub fn tap(
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
pub fn drag_on(h: &mut Harness<'static, StudioApp>, a: eframe::egui::Pos2, b: eframe::egui::Pos2) {
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

/// Select a text field's contents and type over them (the desktop specs' `fill`).
pub fn fill(h: &mut Harness<'static, StudioApp>, label: &str, text: &str) {
    use eframe::egui::accesskit::Role;
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run();
    h.key_press_modifiers(eframe::egui::Modifiers::COMMAND, eframe::egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    h.run();
}

/// A drag with the secondary button (the desktop's right-button orbit).
pub fn right_drag(
    h: &mut Harness<'static, StudioApp>,
    a: eframe::egui::Pos2,
    b: eframe::egui::Pos2,
) {
    let button = |pos, pressed| eframe::egui::Event::PointerButton {
        pos,
        button: eframe::egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    h.hover_at(a);
    h.run_steps(1);
    h.event(button(a, true));
    h.run_steps(1);
    for k in 1..=5 {
        h.hover_at(a + (b - a) * (k as f32 / 5.0));
        h.run_steps(1);
    }
    h.event(button(b, false));
    h.run_steps(1);
}
