//! The loading splash (ADR 0009, Screens): a 720 × 420 borderless window with 12 px rounded corners, a light neutral
//! panel holding the Metallic mark, the wordmark, the version, and the real startup step with "n of N" over a 2 px
//! accent bar.
//!
//! The splash is the main window in disguise: the shell opens the window at [`SIZE`], borderless, and turns the same
//! window into the main one when the splash is done. It is always the dark mode with the default accent, because it
//! shows before the settings are read.

use std::time::{Duration, Instant};

use egui::{Align, CornerRadius, Layout, Rect, RichText, Stroke, TextureHandle, UiBuilder, Vec2, vec2};

use crate::theme::tokens::{self, Accent, Motion, radius, size, text};
use crate::theme::{fonts, rounded_outline};

/// The splash window's size, in points.
pub const SIZE: Vec2 = vec2(720.0, 420.0);

/// The splash stays at least this long once shown, so it never flashes. Nothing ever waits on made-up work.
pub const MIN_TIME: Duration = Duration::from_millis(700);

/// The emblem panel's width.
const PANEL_WIDTH: f32 = 272.0;
/// The mark's size on the panel, in points.
pub const EMBLEM: f32 = 144.0;

/// Where the startup is, as the splash shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status<'a> {
    /// The steps' names, in order.
    pub steps: &'a [&'static str],
    /// How many are done.
    pub done: usize,
}

impl Status<'_> {
    /// The step to name (the running one, or the last once all are done) and its "n of N".
    pub fn label(&self) -> (&'static str, String) {
        let total = self.steps.len();
        let shown = self.done.min(total.saturating_sub(1));
        let name = self.steps.get(shown).copied().unwrap_or("");
        (name, format!("{} of {total}", (shown + 1).min(total)))
    }

    /// The share of the steps done, 0 to 1.
    pub fn fraction(&self) -> f32 {
        if self.steps.is_empty() {
            1.0
        } else {
            self.done.min(self.steps.len()) as f32 / self.steps.len() as f32
        }
    }
}

/// The splash's own state: when it was first shown.
#[derive(Debug, Default)]
pub struct Splash {
    shown_at: Option<Instant>,
}

impl Splash {
    /// A splash not shown yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the splash may go at `now`: the steps are done and it has shown for [`MIN_TIME`]. The time counts from
    /// the first frame it was drawn in, not from launch.
    pub fn finished(&self, steps_done: bool, now: Instant) -> bool {
        steps_done
            && self
                .shown_at
                .is_some_and(|shown| now.saturating_duration_since(shown) >= MIN_TIME)
    }

    /// Draws the splash filling `ui` (the window is [`SIZE`]) at `now`.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        now: Instant,
        status: Status<'_>,
        emblem: Option<&TextureHandle>,
        motion: Motion,
    ) {
        let shown = *self.shown_at.get_or_insert(now);
        // Arrives: fades in, easing out. Reduced motion: no fade.
        let t = if motion.slow > 0.0 {
            (now.saturating_duration_since(shown).as_secs_f32() / motion.slow).min(1.0)
        } else {
            1.0
        };
        if t < 1.0 {
            ui.multiply_opacity(egui::emath::easing::cubic_out(t));
            ui.ctx().request_repaint();
        }
        let p = &tokens::DARK;
        let accent = Accent::default().on(true);
        let rect = Rect::from_center_size(ui.max_rect().center(), SIZE);
        let painter = ui.painter().clone();
        let r = radius::PANEL;
        painter.add(egui::Shape::convex_polygon(
            rounded_outline(rect, f32::from(r), 8),
            p.bg,
            Stroke::NONE,
        ));

        // The emblem panel: one flat light grey field, so the dark mark reads.
        let panel = Rect::from_min_size(rect.min, vec2(PANEL_WIDTH, rect.height()));
        let corners = CornerRadius {
            nw: r,
            sw: r,
            ..CornerRadius::ZERO
        };
        painter.rect_filled(panel, corners, p.action);
        if let Some(emblem) = emblem {
            let at = Rect::from_center_size(panel.center(), Vec2::splat(EMBLEM));
            egui::Image::new((emblem.id(), Vec2::splat(EMBLEM)))
                .alt_text("Metallic")
                .paint_at(ui, at);
        }
        // A 1 px border around the body, as the website's panels have.
        let body_edge = Rect::from_min_max(egui::pos2(panel.right(), rect.top()), rect.max).shrink(0.5);
        let edge = Stroke::new(size::BORDER, p.border);
        painter.hline(
            body_edge.left()..=body_edge.right() - f32::from(r),
            body_edge.top(),
            edge,
        );
        painter.hline(
            body_edge.left()..=body_edge.right() - f32::from(r),
            body_edge.bottom(),
            edge,
        );
        painter.vline(
            body_edge.right(),
            body_edge.top() + f32::from(r)..=body_edge.bottom() - f32::from(r),
            edge,
        );

        // The body. Text goes in as labels, so screen readers (and the UI tests) read what the splash says.
        let body = Rect::from_min_max(
            egui::pos2(panel.right() + 40.0, rect.top() + 44.0),
            egui::pos2(rect.right() - 40.0, rect.bottom() - 36.0),
        );
        let line_height = text::WORDMARK + 4.0;
        let mut y = body.top();
        for (word, colour) in [("Metallic", p.text), ("Studio", p.muted)] {
            let line = Rect::from_min_size(egui::pos2(body.left(), y), vec2(body.width(), line_height));
            label(
                ui,
                line,
                Layout::left_to_right(Align::Center),
                RichText::new(word)
                    .font(fonts::named(fonts::HEADING, text::WORDMARK))
                    .extra_letter_spacing(text::WORDMARK_TRACKING)
                    .color(colour),
            );
            y += line_height;
        }
        let version = Rect::from_min_size(egui::pos2(body.left(), y + 12.0), vec2(body.width(), 18.0));
        label(
            ui,
            version,
            Layout::left_to_right(Align::Center),
            RichText::new(format!("Version {}", crate::VERSION))
                .size(text::SMALL)
                .color(p.subtle),
        );

        // The status: the step and its count above a 2 px track, whose fill eases to the share done.
        let track = Rect::from_min_max(egui::pos2(body.left(), body.bottom() - 2.0), body.right_bottom());
        let line = Rect::from_min_max(
            egui::pos2(body.left(), track.top() - 10.0 - 20.0),
            egui::pos2(body.right(), track.top() - 10.0),
        );
        let (step, count) = status.label();
        label(
            ui,
            line,
            Layout::left_to_right(Align::Center),
            RichText::new(step).size(text::BODY).color(p.muted),
        );
        label(
            ui,
            line,
            Layout::right_to_left(Align::Center),
            RichText::new(count).size(text::SMALL).color(p.subtle),
        );
        painter.rect_filled(track, 1.0, p.surface);
        let fill =
            ui.ctx()
                .animate_value_with_time(egui::Id::unique("splash.progress"), status.fraction(), motion.slow);
        if fill > 0.0 {
            let done = Rect::from_min_size(track.min, vec2(track.width() * fill.clamp(0.0, 1.0), track.height()));
            painter.rect_filled(done, 1.0, accent);
        }
    }
}

/// One line of the splash's text, placed in `rect`.
fn label(ui: &mut egui::Ui, rect: Rect, layout: Layout, text: RichText) {
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.add(egui::Label::new(text).selectable(false).extend());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEPS: [&str; 3] = ["one", "two", "three"];

    #[test]
    fn the_status_names_the_running_step_and_counts() {
        let status = |done| Status { steps: &STEPS, done };
        assert_eq!(status(0).label(), ("one", "1 of 3".to_owned()));
        assert_eq!(status(0).fraction(), 0.0);
        assert_eq!(status(1).label(), ("two", "2 of 3".to_owned()));
        assert_eq!(status(2).label().1, "3 of 3");
        // Done: the last step stays named, and the bar is full.
        assert_eq!(status(3).label(), ("three", "3 of 3".to_owned()));
        assert_eq!(status(3).fraction(), 1.0);
        assert_eq!(status(9).fraction(), 1.0);
        let none = Status { steps: &[], done: 0 };
        assert_eq!(none.label(), ("", "0 of 0".to_owned()));
        assert_eq!(none.fraction(), 1.0);
    }

    #[test]
    fn the_splash_waits_for_the_steps_and_its_minimum_time() {
        let start = Instant::now();
        let mut splash = Splash::new();
        assert!(
            !splash.finished(true, start + MIN_TIME * 2),
            "never shown: the time has not started"
        );
        splash.shown_at = Some(start);
        assert!(!splash.finished(true, start));
        assert!(!splash.finished(true, start + MIN_TIME - Duration::from_millis(1)));
        assert!(!splash.finished(false, start + MIN_TIME * 10), "steps still running");
        assert!(splash.finished(true, start + MIN_TIME));
        // A clock that went backwards never finishes it early.
        assert!(!splash.finished(true, start.checked_sub(Duration::from_secs(1)).unwrap()));
    }
}
