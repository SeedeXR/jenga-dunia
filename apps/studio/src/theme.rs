//! Studio's look: Host Grotesk (OFL, `assets/fonts/`) and the desktop app's dark tokens
//! (`apps/desktop/src/index.css`, `--color-jd-*`), so the two windows read as one product.

use eframe::egui::{self, Color32, FontData, FontFamily, FontId, TextStyle};
use eframe::epaint::text::{
    FontInsert, FontPriority, FontTweak, InsertFontFamily, VariationCoords,
};

const HOST_GROTESK: &[u8] = include_bytes!("../assets/fonts/HostGrotesk-wght.ttf");
/// The family name of Host Grotesk at weight 600 (`font-semibold`).
const SEMIBOLD: &str = "semibold";

/// Host Grotesk semibold at `size` points (headings, the brand name).
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// The `--color-jd-*` tokens. A test checks them against the CSS so the two cannot drift.
pub mod color {
    use super::Color32;
    /// `--color-jd-bg-0`: page, text fields, viewport background.
    pub const BG0: Color32 = Color32::from_rgb(0x0c, 0x0f, 0x15);
    /// `--color-jd-bg-1`: panels.
    pub const BG1: Color32 = Color32::from_rgb(0x15, 0x1a, 0x24);
    /// `--color-jd-bg-2`: cards, windows.
    pub const BG2: Color32 = Color32::from_rgb(0x1d, 0x23, 0x31);
    /// `--color-jd-bg-3`: buttons, scrollbars.
    pub const BG3: Color32 = Color32::from_rgb(0x28, 0x30, 0x43);
    /// `--color-jd-line`: separators.
    pub const LINE: Color32 = Color32::from_rgb(0x2b, 0x33, 0x46);
    /// `--color-jd-fg-0`: primary text.
    pub const FG0: Color32 = Color32::from_rgb(0xee, 0xf1, 0xf7);
    /// `--color-jd-fg-1`: secondary text.
    pub const FG1: Color32 = Color32::from_rgb(0xbe, 0xc6, 0xd5);
    /// `--color-jd-fg-2`: hints.
    pub const FG2: Color32 = Color32::from_rgb(0x8a, 0x94, 0xa9);
    /// `--color-jd-accent`.
    pub const ACCENT: Color32 = Color32::from_rgb(0x6b, 0x8c, 0xff);
    /// `--color-jd-primary`.
    pub const PRIMARY: Color32 = Color32::from_rgb(0x3f, 0x63, 0xf2);
    /// `--color-jd-warn`.
    pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xc4, 0x51);
    /// `--color-jd-error`.
    pub const ERROR: Color32 = Color32::from_rgb(0xff, 0x7b, 0x72);
}

/// Install the font and the dark palette on a context (the app and the test harness).
pub fn install(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
    ctx.add_font(FontInsert::new(
        "host-grotesk",
        FontData::from_static(HOST_GROTESK),
        vec![InsertFontFamily {
            family: FontFamily::Proportional,
            priority: FontPriority::Highest,
        }],
    ));
    // The same variable font at wght 600 as its own family; the regular face backs it up for
    // anything it lacks.
    ctx.add_font(FontInsert::new(
        "host-grotesk-semibold",
        FontData::from_static(HOST_GROTESK).tweak(FontTweak {
            coords: VariationCoords::new([(b"wght", 600.0)]),
            ..Default::default()
        }),
        vec![InsertFontFamily {
            family: FontFamily::Name(SEMIBOLD.into()),
            priority: FontPriority::Highest,
        }],
    ));
    ctx.add_font(FontInsert::new(
        "host-grotesk-fallback",
        FontData::from_static(HOST_GROTESK),
        vec![InsertFontFamily {
            family: FontFamily::Name(SEMIBOLD.into()),
            priority: FontPriority::Lowest,
        }],
    ));
    // The desktop's sizes: 13 px body, 12 px controls and the log (`text-xs`).
    ctx.all_styles_mut(|style| {
        use FontFamily::{Monospace, Proportional};
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(12.5, FontFamily::Name(SEMIBOLD.into())),
            ),
            (TextStyle::Body, FontId::new(13.0, Proportional)),
            (TextStyle::Button, FontId::new(12.0, Proportional)),
            (TextStyle::Small, FontId::new(12.0, Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, Monospace)),
        ]
        .into();
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    });
    let mut v = egui::Visuals::dark();
    v.panel_fill = color::BG1;
    v.window_fill = color::BG2;
    v.faint_bg_color = color::BG2;
    v.extreme_bg_color = color::BG0;
    v.hyperlink_color = color::ACCENT;
    v.warn_fg_color = color::WARN;
    v.error_fg_color = color::ERROR;
    v.selection.bg_fill = color::PRIMARY;
    v.selection.stroke.color = color::ACCENT;
    v.widgets.noninteractive.bg_stroke.color = color::LINE;
    v.widgets.noninteractive.fg_stroke.color = color::FG1;
    // `.btn`: bg-2 with a white/5 hairline; hover bg-3 and white/10; rounded-md.
    let hair = egui::Color32::from_rgba_premultiplied(13, 13, 13, 13);
    let hair_hover = egui::Color32::from_rgba_premultiplied(26, 26, 26, 26);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = egui::CornerRadius::same(6);
        w.expansion = 0.0;
    }
    v.widgets.inactive.weak_bg_fill = color::BG2;
    v.widgets.inactive.bg_fill = color::BG2;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, hair);
    v.widgets.inactive.fg_stroke.color = color::FG0;
    v.widgets.hovered.weak_bg_fill = color::BG3;
    v.widgets.hovered.bg_fill = color::BG3;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, hair_hover);
    v.widgets.hovered.fg_stroke.color = color::FG0;
    // Text fields (`inputCls`): bg-2, focus border accent.
    v.text_edit_bg_color = Some(color::BG2);
    v.widgets.active.weak_bg_fill = color::PRIMARY;
    v.widgets.active.bg_fill = color::PRIMARY;
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::Theme::Dark);
}

#[cfg(test)]
mod tests {
    use super::color::*;

    #[test]
    fn tokens_match_the_desktop_css() {
        let css = include_str!("../../desktop/src/index.css");
        for (name, c) in [
            ("bg-0", BG0),
            ("bg-1", BG1),
            ("bg-2", BG2),
            ("bg-3", BG3),
            ("line", LINE),
            ("fg-0", FG0),
            ("fg-1", FG1),
            ("fg-2", FG2),
            ("accent", ACCENT),
            ("primary", PRIMARY),
            ("warn", WARN),
            ("error", ERROR),
        ] {
            let hex = format!(
                "--color-jd-{name}: #{:02x}{:02x}{:02x};",
                c.r(),
                c.g(),
                c.b()
            );
            assert!(css.contains(&hex), "index.css lacks `{hex}`");
        }
    }
}
