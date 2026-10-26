//! The Lightroom-style slider: click and drag set the value, double-click resets, arrow keys step,
//! and it is an accessible slider with its name and value.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use jd_studio::widgets::Range;

fn slider(start: f64) -> Harness<'static, f64> {
    Harness::builder()
        .with_size([200.0, 40.0])
        // 60 fps: the default quarter-second steps are longer than a double-click.
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(jd_studio::theme::install)
        .build_ui_state(
            |ui, v: &mut f64| {
                ui.add(Range::new(v, 0.0..=359.0, "Azimuth").step(1.0).reset(140.0));
            },
            start,
        )
}

/// Hover, press, release — as a mouse does (kittest's `drop_at` also removes the cursor).
fn click_at(h: &Harness<'static, f64>, pos: egui::Pos2) {
    h.hover_at(pos);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
}

#[test]
fn click_sets_double_click_resets_and_arrows_step() {
    let mut h = slider(140.0);
    let node = h.get_by_role_and_label(egui::accesskit::Role::Slider, "Azimuth");
    let r = node.rect();
    // A click at the right end of the track is the maximum (the handle travels inset by its radius).
    let right = egui::pos2(r.right() - 7.0, r.center().y);
    h.drag_at(right);
    h.run();
    h.drop_at(right);
    h.run();
    assert_eq!(*h.state(), 359.0);
    let left = egui::pos2(r.left() + 7.0 + (r.width() - 14.0) * 0.25, r.center().y);
    h.drag_at(left);
    h.run();
    h.drop_at(left);
    h.run();
    assert!((*h.state() - 90.0).abs() <= 1.0, "{}", h.state());
    // Double-click: back to the default. Half a second of quiet first, so the earlier clicks do
    // not chain into a triple click (egui counts clicks within 0.3 s).
    h.run_steps(30);
    for _ in 0..2 {
        click_at(&h, left);
        h.step();
    }
    h.run();
    assert_eq!(*h.state(), 140.0);
    // Focus it and step with the keys.
    h.get_by_role_and_label(egui::accesskit::Role::Slider, "Azimuth")
        .focus();
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert_eq!(*h.state(), 142.0);
    h.key_press(egui::Key::ArrowLeft);
    h.run();
    assert_eq!(*h.state(), 141.0);
}

/// `JD_WIDGET_CAPTURE=<png> cargo test -p jd-studio --test widgets -- --ignored`: the slider at 2×
/// on a panel, to compare with the desktop's `.jd-range`.
#[test]
#[ignore = "writes a comparison image"]
fn capture_slider() {
    let out = std::env::var("JD_WIDGET_CAPTURE").expect("JD_WIDGET_CAPTURE=<path.png>");
    let mut h = Harness::builder()
        .with_size([240.0, 70.0])
        .with_pixels_per_point(2.0)
        .wgpu()
        .with_context_setup(jd_studio::theme::install)
        .build_ui_state(
            |ui, v: &mut [f64; 3]| {
                egui::Frame::new()
                    .fill(jd_studio::theme::color::BG1)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        for (i, (label, range)) in [
                            ("Azimuth 140°", 0.0..=359.0),
                            ("Exposure 1", 0.25..=4.0),
                            ("Bloom 0", 0.0..=1.0),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.add_space(20.0);
                                let r = Range::new(&mut v[i], range, label);
                                ui.add(if i == 1 { r.fill_from(1.0) } else { r });
                            });
                        }
                    });
            },
            [140.0, 1.0, 0.0],
        );
    h.run();
    h.render().expect("render").save(&out).expect("save");
}
