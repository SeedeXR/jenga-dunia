//! The viewport's chrome, as the desktop draws it over its canvas (`App.tsx` `ViewportChrome`,
//! the tool buttons and the layer strip): fit / still / fullscreen / frame at the top left, the
//! compass strip with the heading and distance, the height ruler, the hints, the tool buttons at
//! the top right and the "add a layer" strip on the right edge. Positions, sizes and colours are
//! the desktop's Tailwind classes in points.

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Shape, Stroke, Ui, pos2, vec2,
};

use crate::look;
use crate::theme::{self, color};

/// What a click in the chrome asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Back to the home view.
    Fit,
    /// Save the frame to exports/stills/.
    Still,
    /// Fullscreen viewport on / off.
    Fullscreen,
    /// Framed block on / off.
    Frame,
    /// Show the node graph (the Editor window).
    Graph,
    /// Insert this node before the height output (the layer strip).
    Insert(usize),
}

/// The layer strip, top to bottom: test id, icon, tooltip, node kind (None: not ported yet).
pub const STRIP: [(&str, &str, &str, Option<&str>); 11] = [
    (
        "strip-cone",
        "mountain",
        "Shape: a cone at the world's centre (jd.shape.primitive)",
        Some("jd.shape.primitive@1"),
    ),
    (
        "strip-terrace",
        "layers",
        "Filter: terraces (jd.filter.terrace)",
        Some("jd.filter.terrace@1"),
    ),
    (
        "strip-blur",
        "blend",
        "Filter: smooth (jd.filter.blur)",
        Some("jd.filter.blur@1"),
    ),
    (
        "strip-thermal",
        "thermometer",
        "Simulation: thermal erosion (jd.erosion.thermal)",
        Some("jd.erosion.thermal@1"),
    ),
    (
        "strip-hydraulic",
        "droplets",
        "Simulation: hydraulic erosion (jd.erosion.hydraulic)",
        Some("jd.erosion.hydraulic@1"),
    ),
    (
        "strip-rivers",
        "git-branch",
        "Simulation: carve rivers (jd.hydrology.rivers)",
        Some("jd.hydrology.rivers@1"),
    ),
    ("strip-snow", "snowflake", "Simulation layer: snow", None),
    (
        "strip-sand",
        "wind",
        "Simulation layer: wind-blown sand",
        None,
    ),
    (
        "strip-debris",
        "pickaxe",
        "Simulation layer: rockfall debris",
        None,
    ),
    ("strip-lava", "flame", "Simulation layer: lava flow", None),
    (
        "strip-cave",
        "flashlight",
        "Cave: a karst cave under the terrain (jd.cave.system); other kinds in the Caves panel",
        None,
    ),
];

/// What the chrome shows.
pub struct State {
    /// Camera heading (degrees, the desktop's `−θ`).
    pub heading: f32,
    /// Camera distance (metres).
    pub distance_m: f32,
    /// The shown layer's height range, when there is one.
    pub range_m: Option<[f32; 2]>,
    /// Framed block on.
    pub frame: bool,
    /// A project is open and nothing is running (the desktop's `!s.project || s.busy`).
    pub enabled: bool,
    /// Still possible (a terrain is shown).
    pub can_still: bool,
}

/// `fill-jd-bg-1/80` and friends: a token at an opacity.
fn alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}

/// The desktop's `tickStep`: 1 / 2 / 5 × 10ⁿ giving 4–8 ticks over `span`.
pub fn tick_step(span: f32) -> f32 {
    let raw = span / 6.0;
    let pow = 10f32.powf(raw.max(1e-6).log10().floor());
    let m = raw / pow;
    (if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    }) * pow
}

/// The desktop's `fmt` for heights: `456 m`, `1.2 km`, `2 km`.
pub fn fmt_m(v: f32) -> String {
    if v.abs() >= 1000.0 {
        let km = v / 1000.0;
        if v % 1000.0 == 0.0 {
            format!("{km:.0} km")
        } else {
            format!("{km:.1} km")
        }
    } else {
        format!("{} m", v.round())
    }
}

/// Draw the chrome over `rect` and return the clicked action.
pub fn show(ui: &mut Ui, rect: Rect, st: &State) -> Option<Action> {
    let mut action = None;
    compass(ui, rect, st);
    if let Some(range) = st.range_m {
        ruler(ui, rect, range);
    }
    hints(ui, rect);
    // Top left (`top-2 left-2 gap-1`): fit, still, fullscreen, frame.
    let mut x = rect.left() + 8.0;
    for (icon, tip, enabled, act, on) in [
        (
            "home",
            "Fit: back to the home view",
            true,
            Action::Fit,
            false,
        ),
        (
            "camera",
            "Still: save this frame as exports/stills/view_<n>.png",
            st.can_still,
            Action::Still,
            false,
        ),
        (
            "maximize-2",
            "Fullscreen viewport (Esc leaves)",
            true,
            Action::Fullscreen,
            false,
        ),
        (
            "box",
            "Frame: draw the world as a block with side walls and a rim",
            true,
            Action::Frame,
            st.frame,
        ),
    ] {
        let r = Rect::from_min_size(pos2(x, rect.top() + 8.0), vec2(28.0, 24.0));
        if chip(ui, r, icon, None, tip, enabled, on).clicked() {
            action = Some(act);
        }
        x += 32.0;
    }
    // Top right: Graph, Sculpt (right-28), Road (right-52), Horizon (right-72), Grid (right-[23.5rem]).
    // ponytail: Sculpt, Road, Horizon and Grid are the next P3 slices; shown disabled until then.
    for (right, icon, label, tip, act) in [
        (
            8.0,
            "workflow",
            "Graph",
            "Show the node graph (the Editor window)",
            Some(Action::Graph),
        ),
        (
            112.0,
            "brush",
            "Sculpt",
            "Sculpt: adds a stroke node before the output and paints with the brush",
            None,
        ),
        (
            208.0,
            "route",
            "Road",
            "Road: the next drag on the terrain becomes a spline node that flattens a road bed along it",
            None,
        ),
        (
            288.0,
            "horizon",
            "Horizon",
            "Horizon: continue the noise beyond the world's border as a coarse skirt (same seed, so it matches at the edge; erosion is not continued)",
            None,
        ),
        (
            376.0,
            "grid-3x-3",
            "Grid",
            "Grid tool: select cells of the world and set their height, World Creator style",
            None,
        ),
    ] {
        let w =
            26.0 + ui.fonts_mut(|f| {
                f.layout_no_wrap(label.into(), FontId::proportional(12.0), Color32::WHITE)
                    .size()
                    .x
            }) + 8.0;
        let r = Rect::from_min_size(
            pos2(rect.right() - right - w, rect.top() + 8.0),
            vec2(w, 24.0),
        );
        let enabled = act.is_some() && (act == Some(Action::Graph) || st.enabled);
        if chip(ui, r, icon, Some(label), tip, enabled, false).clicked() {
            action = act;
        }
    }
    // The layer strip: a card at the right edge, vertically centred, accent icons.
    let n = STRIP.len() as f32;
    let h = n * 27.0 + (n - 1.0) * 2.0 + 8.0;
    let strip = Rect::from_min_size(
        pos2(rect.right() - 8.0 - 35.0, rect.center().y - h / 2.0),
        vec2(35.0, h),
    );
    ui.painter().add(look::card().inner_margin(0).paint(strip));
    for (k, (_, icon, tip, kind)) in STRIP.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(strip.left() + 4.0, strip.top() + 4.0 + k as f32 * 29.0),
            vec2(27.0, 27.0),
        );
        let enabled = st.enabled && kind.is_some();
        let resp = ui
            .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                look::icon_button_tinted(ui, icon, tip, enabled, color::ACCENT)
            })
            .inner;
        if resp.clicked() {
            action = Some(Action::Insert(k));
        }
    }
    action
}

/// A `.btn px-1.5` (icon) or `.btn` (icon and label) placed at `r`; `on` is `text-jd-accent`.
fn chip(
    ui: &mut Ui,
    r: Rect,
    icon: &str,
    label: Option<&str>,
    tip: &str,
    enabled: bool,
    on: bool,
) -> egui::Response {
    ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
        let fg = if on { color::ACCENT } else { color::FG0 };
        let image = egui::Image::new(look::icon(icon))
            .fit_to_exact_size(vec2(14.0, 14.0))
            .tint(fg);
        let b = match label {
            Some(l) => egui::Button::image_and_text(image, RichText::new(l).size(12.0).color(fg)),
            None => egui::Button::image(image),
        };
        let resp = ui.add_enabled(enabled, b.min_size(r.size()));
        let tip_owned = tip.to_owned();
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Button, enabled, &tip_owned)
        });
        resp.on_hover_text(tip)
    })
    .inner
}

/// The compass strip (`top-11`, centred): 180° of bearings across 240 points, the heading at the
/// centre mark, then `326° · 5.3 km`.
fn compass(ui: &Ui, rect: Rect, st: &State) {
    const W: f32 = 240.0;
    let p = ui.painter();
    let o = pos2(rect.center().x - W / 2.0, rect.top() + 44.0);
    p.rect_filled(
        Rect::from_min_size(o, vec2(W, 26.0)),
        4.0,
        alpha(color::BG1, 0.8),
    );
    let px =
        |deg: f32| W / 2.0 + (((deg - st.heading + 540.0).rem_euclid(360.0)) - 180.0) * W / 180.0;
    for i in 0..24 {
        let t = i as f32 * 15.0;
        let x = px(t);
        if !(2.0..=W - 2.0).contains(&x) {
            continue;
        }
        let major = i % 6 == 0;
        let top = if major { 15.0 } else { 19.0 };
        p.line_segment(
            [o + vec2(x, top), o + vec2(x, 26.0)],
            Stroke::new(1.0, color::FG2),
        );
        if major {
            let name = ["N", "E", "S", "W"][i / 6];
            let c = if i == 0 { color::ACCENT } else { color::FG0 };
            p.text(
                o + vec2(x, 12.0),
                Align2::CENTER_BOTTOM,
                name,
                theme::semibold(10.0),
                c,
            );
        }
    }
    p.add(Shape::convex_polygon(
        vec![
            o + vec2(W / 2.0 - 4.0, 26.0),
            o + vec2(W / 2.0 + 4.0, 26.0),
            o + vec2(W / 2.0, 19.0),
        ],
        color::ACCENT,
        Stroke::NONE,
    ));
    let d = st.distance_m;
    let label = format!(
        "{}° · {}",
        st.heading.round(),
        if d >= 1000.0 {
            format!("{:.1} km", d / 1000.0)
        } else {
            format!("{} m", d.round())
        }
    );
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(label, FontId::proportional(10.0), color::FG1));
    let at = pos2(rect.center().x - galley.size().x / 2.0, o.y + 28.0);
    p.rect_filled(
        Rect::from_min_size(at - vec2(4.0, 0.0), galley.size() + vec2(8.0, 0.0)),
        4.0,
        alpha(color::BG1, 0.7),
    );
    p.galley(at, galley, color::FG1);
}

/// The height ruler (`top-14 bottom-16 left-1 w-16`): ticks over the layer's range, zero marked,
/// negatives in the warning colour, `Max` above and `Min` below.
fn ruler(ui: &Ui, rect: Rect, [lo, hi]: [f32; 2]) {
    let p = ui.painter();
    let (top, bottom, left) = (rect.top() + 56.0, rect.bottom() - 64.0, rect.left() + 4.0);
    let line_x = left + 64.0 - 4.0;
    p.vline(
        line_x,
        top..=bottom,
        Stroke::new(1.0, alpha(color::FG2, 0.6)),
    );
    let span = (hi - lo).max(1e-6);
    let step = tick_step(span);
    let y = |v: f32| top + (1.0 - (v - lo) / span) * (bottom - top);
    let mut v = (lo / step).ceil() * step;
    let mut n = 0;
    while v <= hi + 1e-6 && n < 40 {
        let zero = v.abs() < 1e-6;
        let (len, c) = if zero {
            (12.0, color::FG0)
        } else {
            (8.0, color::FG2)
        };
        p.hline(line_x - len..=line_x, y(v), Stroke::new(1.0, c));
        let tc = if v < 0.0 {
            color::WARN
        } else if zero {
            color::FG0
        } else {
            color::FG2
        };
        p.text(
            pos2(line_x - len - 4.0, y(v)),
            Align2::RIGHT_CENTER,
            fmt_m(v),
            FontId::proportional(10.0),
            tc,
        );
        v += step;
        n += 1;
    }
    p.text(
        pos2(left, top - 20.0),
        Align2::LEFT_TOP,
        format!("Max {}", fmt_m(hi)),
        FontId::proportional(10.0),
        color::FG1,
    );
    p.text(
        pos2(left, bottom + 20.0),
        Align2::LEFT_BOTTOM,
        format!("Min {}", fmt_m(lo)),
        FontId::proportional(10.0),
        color::FG1,
    );
}

/// The hints (`bottom-2`, centred): key on the left, what it does on the right.
fn hints(ui: &Ui, rect: Rect) {
    let rows = [("Drag", "orbit"), ("Wheel", "zoom")];
    let font = FontId::proportional(10.0);
    let lay = |t: &str, c| ui.fonts_mut(|f| f.layout_no_wrap(t.into(), font.clone(), c));
    let keys: Vec<_> = rows.iter().map(|(k, _)| lay(k, color::FG1)).collect();
    let vals: Vec<_> = rows.iter().map(|(_, v)| lay(v, color::FG2)).collect();
    let kw = keys.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let vw = vals.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let row_h = keys.iter().map(|g| g.size().y).fold(0.0, f32::max);
    let size = vec2(
        12.0 + kw + 12.0 + vw + 12.0,
        4.0 + row_h * rows.len() as f32 + 4.0,
    );
    let o = Pos2::new(rect.center().x - size.x / 2.0, rect.bottom() - 8.0 - size.y);
    let p = ui.painter();
    p.rect_filled(Rect::from_min_size(o, size), 4.0, alpha(color::BG1, 0.8));
    for (i, (k, v)) in keys.into_iter().zip(vals).enumerate() {
        let y = o.y + 4.0 + row_h * i as f32;
        p.galley(pos2(o.x + 12.0, y), k, color::FG1);
        p.galley(pos2(o.x + 12.0 + kw + 12.0, y), v, color::FG2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruler_steps_and_labels_print_like_the_desktop() {
        assert_eq!(tick_step(418.0), 50.0);
        assert_eq!(tick_step(6.0), 1.0);
        assert_eq!(tick_step(30.0), 5.0);
        assert_eq!(fmt_m(456.4), "456 m");
        assert_eq!(fmt_m(2000.0), "2 km");
        assert_eq!(fmt_m(1260.0), "1.3 km");
        assert_eq!(fmt_m(-12.0), "-12 m");
    }
}
