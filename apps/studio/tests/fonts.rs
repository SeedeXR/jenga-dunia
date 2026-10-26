//! Every character Studio's sources put on screen has a glyph: the non-ASCII characters of all
//! string literals under `src/` (labels, tooltips, the log) drawn in one label with Studio's fonts,
//! where kittest panics on a missing glyph instead of drawing a box.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::collections::BTreeSet;

use eframe::egui;
use egui_kittest::Harness;

/// Non-ASCII characters inside `"…"` literals (comments skipped; escapes are ASCII anyway).
fn ui_characters() -> String {
    let mut chars = BTreeSet::new();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(dir).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
            let mut inside = false;
            for c in line.chars() {
                if c == '"' {
                    inside = !inside;
                } else if inside && !c.is_ascii() {
                    chars.insert(c);
                }
            }
        }
    }
    chars.into_iter().collect()
}

#[test]
fn every_ui_character_has_a_glyph() {
    let text = ui_characters();
    assert!(text.contains('⟲') && text.contains('⌘'), "{text}");
    for family in [
        egui::FontFamily::Proportional,
        egui::FontFamily::Monospace,
        egui::FontFamily::Name("semibold".into()),
    ] {
        let text = text.clone();
        let mut h = Harness::builder()
            .with_size([1200.0, 80.0])
            .build_ui(move |ui| {
                jd_studio::theme::install(ui.ctx());
                ui.label(egui::RichText::new(text.as_str()).family(family.clone()));
            });
        h.run_steps(2);
    }
}
