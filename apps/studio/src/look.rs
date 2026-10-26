//! The desktop UI's building blocks in egui — `apps/desktop/src/index.css` (`.btn`,
//! `.btn-primary`, `.jd-panel`, `.jd-section`) and `App.tsx` (`Field`, `inputCls`, the toolbar's
//! icon buttons) — so Studio and the Editor look the same (owner rule, ADR-0017).

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, ImageSource, Layout, Margin, Response, RichText,
    Sense, Shadow, Stroke, StrokeKind, Ui, WidgetInfo, accesskit::Role, pos2, vec2,
};

use crate::theme::{self, color};

/// `border-white/5`.
pub const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(13, 13, 13, 13);
/// `border-white/10`.
pub const HAIRLINE_HOVER: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26);

/// `.jd-panel`: a floating card (`rounded-xl bg-jd-bg-1`, a soft shadow, a `white/5` ring).
pub fn card() -> Frame {
    Frame::new()
        .fill(color::BG1)
        .corner_radius(CornerRadius::same(12))
        .stroke(Stroke::new(1.0, HAIRLINE))
        .shadow(Shadow {
            offset: [0, 10],
            blur: 30,
            spread: 0,
            color: Color32::from_black_alpha(115),
        })
        .inner_margin(Margin::same(12))
}

/// A side column: the card scrolling its content, with the card's right padding inside the
/// scroll area as the desktop's `overflow-y-auto` panel has it, so the overlay scroll bar sits in
/// that gutter and never covers the right-aligned controls.
pub fn scroll_card(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    card()
        .inner_margin(Margin {
            right: 0,
            ..Margin::same(12)
        })
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            egui::ScrollArea::vertical()
                .id_salt(id)
                .auto_shrink(false)
                .show(ui, |ui| {
                    Frame::new()
                        .inner_margin(Margin {
                            right: 12,
                            ..Margin::ZERO
                        })
                        .show(ui, add);
                });
        });
}

/// `.jd-section`: a chevron (`▾`, `text-[10px] text-jd-fg-2`) and a semibold 12.5 px title.
pub fn section(ui: &mut Ui, title: &str) {
    let title = crate::i18n::tr(ui.ctx(), title);
    let title = title.as_str();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(vec2(8.0, 12.0), Sense::hover());
        let c = r.center();
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                pos2(c.x - 3.5, c.y - 1.75),
                pos2(c.x + 3.5, c.y - 1.75),
                pos2(c.x, c.y + 2.25),
            ],
            color::FG2,
            Stroke::NONE,
        ));
        ui.label(
            RichText::new(title)
                .font(theme::semibold(12.5))
                .color(color::FG0),
        );
    });
    ui.add_space(8.0);
}

/// `Field`: the label (`text-xs text-jd-fg-1`) on the left, the control on the right, `py-1`.
pub fn field(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(color::FG1));
        ui.with_layout(Layout::right_to_left(Align::Center), add);
    });
    ui.add_space(2.0);
}

/// `inputCls`: a right-aligned text field `w-28` (112 px) on `bg-jd-bg-2`.
pub fn input(ui: &mut Ui, value: &mut String, width: f32, hint: &str, name: &str) -> Response {
    let r = ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(width)
            .align(egui::Align2::RIGHT_CENTER)
            .font(egui::FontId::proportional(12.0))
            .hint_text(hint),
    );
    let name = name.to_owned();
    r.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, &name));
    r
}

/// An icon from `assets/icons/` (lucide, rendered by `scripts/studio-icons.mjs`).
pub fn icon(name: &str) -> ImageSource<'static> {
    macro_rules! icons {
        ($($n:literal),* $(,)?) => {
            match name {
                $($n => egui::include_image!(concat!("../assets/icons/", $n, ".svg")),)*
                _ => egui::include_image!("../assets/icons/circle.svg"),
            }
        };
    }
    icons!(
        "arrow-down",
        "arrow-up",
        "chevron-down",
        "chevron-right",
        "eye",
        "eye-off",
        "plus",
        "search",
        "trash-2",
        "file-plus",
        "folder-open",
        "save",
        "file-input",
        "download",
        "undo-2",
        "redo-2",
        "wand-2",
        "play",
        "camera",
        "maximize-2",
        "folder-search",
        "folder-plus",
        "upload",
        "globe",
        "workflow",
        "panel-left-close",
        "panel-left-open",
        "home",
        "box",
        "brush",
        "route",
        "grid-3x-3",
        "horizon",
        "mountain",
        "layers",
        "blend",
        "thermometer",
        "droplets",
        "git-branch",
        "snowflake",
        "wind",
        "pickaxe",
        "flame",
        "flashlight",
    )
}

/// `.btn` (or `.btn-primary` when `primary`): an optional 14 px icon and 12 px text.
pub fn button(
    ui: &mut Ui,
    icon_name: Option<&str>,
    text: &str,
    primary: bool,
    enabled: bool,
) -> Response {
    let fg = if primary { Color32::WHITE } else { color::FG0 };
    let label = RichText::new(text).size(12.0).color(fg);
    let mut b = match icon_name {
        Some(i) => egui::Button::image_and_text(
            egui::Image::new(icon(i))
                .fit_to_exact_size(vec2(14.0, 14.0))
                .tint(fg),
            label,
        ),
        None => egui::Button::new(label),
    };
    if primary {
        b = b.fill(color::PRIMARY).stroke(Stroke::NONE);
    }
    ui.add_enabled(enabled, b)
}

/// The toolbar's icon buttons: a 15 px icon in `fg-1`, `p-1.5`, `hover:bg-jd-bg-3`; the
/// tooltip is also the accessible name.
pub fn icon_button(ui: &mut Ui, icon_name: &str, tip: &str, enabled: bool) -> Response {
    icon_button_tinted(ui, icon_name, tip, enabled, color::FG1)
}

/// [`icon_button`] in another resting colour (the layer strip's `text-jd-accent`).
pub fn icon_button_tinted(
    ui: &mut Ui,
    icon_name: &str,
    tip: &str,
    enabled: bool,
    rest: Color32,
) -> Response {
    let size = vec2(27.0, 27.0);
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        if hovered {
            ui.painter().rect_filled(rect, 6.0, color::BG3);
        }
        let tint = match (enabled, hovered) {
            (false, _) => color::FG1.gamma_multiply(0.3),
            (true, true) => color::FG0,
            (true, false) => rest,
        };
        egui::Image::new(icon(icon_name)).tint(tint).paint_at(
            ui,
            egui::Rect::from_center_size(rect.center(), vec2(15.0, 15.0)),
        );
    }
    let tip_owned = tip.to_owned();
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &tip_owned));
    resp.on_hover_text(tip)
}

/// The toolbar's separators (`mx-1 h-4 w-px bg-jd-line`).
pub fn toolbar_separator(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 16.0), Sense::hover());
    ui.painter()
        .vline(r.center().x, r.y_range(), Stroke::new(1.0, color::LINE));
}

/// A rounded frame around a rect (the viewport's `rounded-xl ring-1 ring-white/5`).
pub fn ring(ui: &Ui, rect: egui::Rect) {
    ui.painter()
        .rect_stroke(rect, 12.0, Stroke::new(1.0, HAIRLINE), StrokeKind::Inside);
}

/// The desktop's checkbox (`accent-color: primary` on the native control): a 13 px rounded box,
/// filled blue with a white tick when on, a hairline square when off. `name` is its accessible
/// label (the row's text).
pub fn checkbox(ui: &mut Ui, on: &mut bool, name: &str) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if *on {
            p.rect_filled(rect, 3.0, color::PRIMARY);
            let c = rect.center();
            p.add(egui::Shape::line(
                vec![
                    c + vec2(-3.5, 0.0),
                    c + vec2(-1.0, 2.5),
                    c + vec2(3.5, -2.5),
                ],
                Stroke::new(1.8, Color32::WHITE),
            ));
        } else {
            p.rect_filled(rect, 3.0, color::BG2);
            p.rect_stroke(rect, 3.0, Stroke::new(1.0, color::FG2), StrokeKind::Inside);
        }
    }
    let (state, name) = (*on, name.to_owned());
    resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, state, &name));
    resp
}

/// The desktop's `btn btn-primary w-full justify-center`: a full-width primary button with its
/// icon and text centred (egui's button keeps its content at the left when stretched).
pub fn wide_primary(ui: &mut Ui, icon_name: &str, text: &str, enabled: bool) -> Response {
    wide_button(ui, icon_name, text, true, enabled)
}

/// A full-width button (`btn w-full justify-center`, or with `btn-primary`) with its icon and
/// text centred.
pub fn wide_button(
    ui: &mut Ui,
    icon_name: &str,
    text: &str,
    primary: bool,
    enabled: bool,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), sense);
    if ui.is_rect_visible(rect) {
        let alpha = if enabled { 1.0 } else { 0.4 };
        let hovered = resp.hovered() && enabled;
        let (fill, stroke, fg) = if primary {
            let fill = if hovered {
                color::ACCENT
            } else {
                color::PRIMARY
            };
            (fill, Stroke::NONE, Color32::WHITE)
        } else {
            let w = &ui.visuals().widgets;
            let v = if hovered { &w.hovered } else { &w.inactive };
            (v.weak_bg_fill, v.bg_stroke, color::FG0)
        };
        ui.painter().rect(
            rect,
            4.0,
            fill.gamma_multiply(alpha),
            stroke,
            StrokeKind::Inside,
        );
        let galley = ui.painter().layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(12.0),
            fg.gamma_multiply(alpha),
        );
        let w = 14.0 + 6.0 + galley.size().x;
        let left = rect.center().x - w / 2.0;
        egui::Image::new(icon(icon_name))
            .tint(fg.gamma_multiply(alpha))
            .paint_at(
                ui,
                egui::Rect::from_center_size(pos2(left + 7.0, rect.center().y), vec2(14.0, 14.0)),
            );
        ui.painter().galley(
            pos2(left + 20.0, rect.center().y - galley.size().y / 2.0),
            galley,
            fg,
        );
    }
    let name = text.to_owned();
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &name));
    resp
}

/// A field committed when it loses focus (the desktop's `defaultValue` + `onBlur`): it shows
/// `current` until edited, keeps the typed text while focused, and returns it on leaving if it
/// differs from `current`.
pub fn commit_input(
    ui: &mut Ui,
    current: &str,
    width: f32,
    hint: &str,
    name: &str,
    tip: &str,
) -> Option<String> {
    let id = ui.make_persistent_id(("commit-input", name));
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| current.to_owned());
    let mut r = input(ui, &mut text, width, hint, name);
    if !tip.is_empty() {
        r = r.on_hover_text(tip);
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        return (text.trim() != current).then(|| text.trim().to_owned());
    }
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    }
    None
}
