//! Widgets shared with the desktop UI's look.
//!
//! [`Range`] is the Lightroom-style slider (`docs/research/research-log/2026-10-26-lightroom-slider.md`),
//! drawn exactly like the desktop's `.jd-range` (`apps/desktop/src/index.css`): a 2 px track in
//! `bg-3`, the fill in `fg-1`, a 14 px ring handle (2 px border, panel-coloured centre and a 3 px
//! panel-coloured halo that opens a gap in the track) whose border thickens to 5 px while pressed.
//! Click or drag sets the value, double-click resets it, the arrow keys step.

use std::ops::RangeInclusive;

use eframe::egui::{self, Key, Rect, Response, Sense, Stroke, Ui, Widget, WidgetInfo, pos2, vec2};

use crate::theme::color;

const HANDLE: f32 = 14.0;
const HEIGHT: f32 = 16.0;

/// A slider over `range` with an optional reset value and fill origin.
pub struct Range<'a> {
    value: &'a mut f64,
    range: RangeInclusive<f64>,
    step: f64,
    reset: Option<f64>,
    fill_from: Option<f64>,
    width: f32,
    label: String,
}

impl<'a> Range<'a> {
    /// A slider of `value` over `range`; `label` names it for accessibility (the visible label is
    /// the row's, as in the desktop's `Field`).
    pub fn new(value: &'a mut f64, range: RangeInclusive<f64>, label: &str) -> Self {
        Self {
            value,
            range,
            step: 0.0,
            reset: None,
            fill_from: None,
            width: 112.0,
            label: label.to_owned(),
        }
    }

    /// Quantise to this step (0 = continuous).
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// Double-click returns to this value.
    pub fn reset(mut self, value: f64) -> Self {
        self.reset = Some(value);
        self
    }

    /// Fill from this value instead of the minimum (ranges centred on a default).
    pub fn fill_from(mut self, value: f64) -> Self {
        self.fill_from = Some(value);
        self
    }

    /// Width in points (the desktop's `w-28` is 112).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    fn snap(&self, v: f64) -> f64 {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let v = if self.step > 0.0 {
            lo + ((v - lo) / self.step).round() * self.step
        } else {
            v
        };
        v.clamp(lo, hi)
    }
}

impl Widget for Range<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, mut resp) =
            ui.allocate_exact_size(vec2(self.width, HEIGHT), Sense::click_and_drag());
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let span = (hi - lo).max(f64::EPSILON);
        // The handle's centre travels between the track's ends inset by its radius, as in WebKit.
        let track = Rect::from_min_max(
            pos2(rect.left() + HANDLE / 2.0, rect.center().y - 1.0),
            pos2(rect.right() - HANDLE / 2.0, rect.center().y + 1.0),
        );
        let x_of = |v: f64| track.left() + ((v - lo) / span) as f32 * track.width();
        let before = *self.value;

        if resp.double_clicked() {
            if let Some(r) = self.reset {
                *self.value = r;
            }
        } else if let Some(p) = resp.interact_pointer_pos()
            && (resp.dragged() || resp.clicked())
        {
            let t = f64::from(((p.x - track.left()) / track.width()).clamp(0.0, 1.0));
            *self.value = self.snap(lo + t * span);
        }
        if resp.has_focus() {
            let step = if self.step > 0.0 {
                self.step
            } else {
                span / 100.0
            };
            let (l, r) = ui.input(|i| {
                (
                    i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::ArrowUp),
                )
            });
            if l {
                *self.value = self.snap(*self.value - step);
            }
            if r {
                *self.value = self.snap(*self.value + step);
            }
        }
        if *self.value != before {
            resp.mark_changed();
        }

        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            let alpha = if ui.is_enabled() { 1.0 } else { 0.4 };
            let c = |c: egui::Color32| c.gamma_multiply(alpha);
            p.rect_filled(track, 1.0, c(color::BG3));
            let (a, b) = {
                let (x0, x1) = (
                    x_of(self.fill_from.unwrap_or(lo).clamp(lo, hi)),
                    x_of(*self.value),
                );
                (x0.min(x1), x0.max(x1))
            };
            p.rect_filled(
                Rect::from_x_y_ranges(a..=b, track.y_range()),
                1.0,
                c(color::FG1),
            );
            let centre = pos2(x_of(*self.value), rect.center().y);
            let pressed = resp.is_pointer_button_down_on() || resp.dragged();
            let ring = if pressed || resp.hovered() {
                color::FG0
            } else {
                color::FG1
            };
            let border = if pressed { 5.0 } else { 2.0 };
            // Halo (the gap in the track), the panel-coloured centre, then the ring.
            p.circle_filled(centre, HANDLE / 2.0 + 3.0, color::BG1);
            p.circle_filled(centre, HANDLE / 2.0, color::BG1);
            p.circle_stroke(
                centre,
                HANDLE / 2.0 - border / 2.0,
                Stroke::new(border, c(ring)),
            );
            if resp.has_focus() {
                p.circle_stroke(centre, HANDLE / 2.0 + 4.0, Stroke::new(2.0, color::ACCENT));
            }
        }
        let (enabled, value, label) = (ui.is_enabled(), *self.value, self.label.clone());
        resp.widget_info(|| WidgetInfo::slider(enabled, value, &label));
        resp
    }
}
