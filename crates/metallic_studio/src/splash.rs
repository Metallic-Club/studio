//! The splash (research doc section 4.1): a 720 x 420 borderless window with the cut, a titanium
//! plate holding the emblem, the wordmark, and the real startup steps with a determinate bar.
//!
//! The splash is the main window in disguise: Studio starts borderless at splash size and
//! [`become_main_window`] turns it into the normal window once the start screen is ready. It is
//! always Graphite, because it shows before any setting is read.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Rect, Stroke, TextureHandle, Ui, Vec2, ViewportBuilder,
    ViewportCommand, text::LayoutJob, vec2,
};

use crate::theme::{self, fonts};
use crate::widgets::cut;

/// Splash window size in points.
pub const SIZE: Vec2 = vec2(720.0, 420.0);
/// The main window's size once the splash is gone.
pub const MAIN_SIZE: Vec2 = vec2(1280.0, 800.0);
/// The main window's minimum size.
pub const MAIN_MIN_SIZE: Vec2 = vec2(960.0, 600.0);

/// The startup steps, in order. Each one names work Studio really does (see `app.rs`).
pub const STEPS: [&str; 4] = [
    "Loading fonts and settings",
    "Starting the renderer",
    "Reading templates",
    "Opening the start screen",
];

/// The splash stays at least this long, so it never flashes; it never waits on fake work.
const MIN_TIME: Duration = Duration::from_millis(700);

const PLATE_WIDTH: f32 = 272.0;
const EMBLEM: f32 = 160.0;
const WORDMARK_SIZE: f32 = 44.0;
const WORDMARK_LINE: f32 = 46.0;
/// The plate's own edges: a brighter top on the light metal, a dark seam towards the body.
const PLATE_TOP: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 89);
const PLATE_SEAM: Color32 = Color32::from_rgba_unmultiplied_const(0, 0, 0, 64);

/// Splash progress: how many [`STEPS`] are done.
#[derive(Debug)]
pub struct Splash {
    /// When the splash was first painted (the minimum time counts from there, not from launch).
    since: Option<Instant>,
    done: usize,
    hold: bool,
}

impl Default for Splash {
    fn default() -> Self {
        Self::new()
    }
}

impl Splash {
    /// A splash with no step done yet.
    pub fn new() -> Self {
        Self {
            since: None,
            done: 0,
            hold: false,
        }
    }

    /// A splash that stays at `done` steps forever (for the `--scene splash` capture).
    pub fn held_at(done: usize) -> Self {
        Self {
            done: done.min(STEPS.len()),
            hold: true,
            ..Self::new()
        }
    }

    /// The index of the step to run next, if any is left.
    pub fn next_step(&self) -> Option<usize> {
        (!self.hold && self.done < STEPS.len()).then_some(self.done)
    }

    /// Marks the step returned by [`Self::next_step`] as done.
    pub fn step_done(&mut self) {
        self.done = (self.done + 1).min(STEPS.len());
    }

    /// Every step is done and the minimum time has passed.
    pub fn finished(&self) -> bool {
        !self.hold
            && self.done == STEPS.len()
            && self.since.is_some_and(|t| t.elapsed() >= MIN_TIME)
    }

    /// The label and "n of 4" text for the step shown (the running one, or the last).
    pub fn status(&self) -> (&'static str, String) {
        let shown = self.done.min(STEPS.len() - 1);
        (STEPS[shown], format!("{} of {}", shown + 1, STEPS.len()))
    }

    /// The share of steps done, 0 to 1.
    pub fn progress(&self) -> f32 {
        self.done as f32 / STEPS.len() as f32
    }

    /// Paints the splash centred in `ui`'s rectangle (the window is exactly [`SIZE`]).
    pub fn show(&mut self, ui: &mut Ui, emblem: Option<&TextureHandle>) {
        let since = *self.since.get_or_insert_with(Instant::now);
        if !self.hold {
            // Arrive: fade in over `t-splash`, easing out.
            let t = (since.elapsed().as_secs_f32() / theme::t_splash()).min(1.0);
            if t < 1.0 {
                ui.multiply_opacity(egui::emath::easing::cubic_out(t));
                ui.ctx().request_repaint();
            }
        }
        let rect = Rect::from_center_size(ui.max_rect().center(), SIZE);
        let painter = ui.painter();
        let radius = CornerRadius::same(theme::R_LG);
        cut::fill(painter, rect, radius, theme::CUT_LG, theme::bg_base());

        // The titanium plate: one flat field of light metal, so the dark emblem reads.
        let plate = Rect::from_min_size(rect.min, vec2(PLATE_WIDTH, rect.height()));
        let plate_radius = CornerRadius {
            nw: theme::R_LG,
            sw: theme::R_LG,
            ..CornerRadius::ZERO
        };
        painter.rect_filled(plate, plate_radius, theme::TITANIUM);
        let r = f32::from(theme::R_LG);
        painter.hline(
            plate.left() + r..=plate.right(),
            plate.top() + 0.5,
            Stroke::new(1.0, PLATE_TOP),
        );
        painter.vline(
            plate.right() - 0.5,
            plate.y_range(),
            Stroke::new(1.0, PLATE_SEAM),
        );
        painter.hline(
            plate.right()..=rect.right() - theme::CUT_LG,
            rect.top() + 0.5,
            Stroke::new(1.0, theme::highlight()),
        );
        if let Some(emblem) = emblem {
            let at = Rect::from_center_size(plate.center(), Vec2::splat(EMBLEM));
            egui::Image::new((emblem.id(), Vec2::splat(EMBLEM))).paint_at(ui, at);
        }

        let body = Rect::from_min_max(
            egui::pos2(plate.right() + 40.0, rect.top() + 44.0),
            egui::pos2(rect.right() - 40.0, rect.bottom() - 36.0),
        );
        let painter = ui.painter();
        let mut y = body.top();
        for (text, light, color) in [
            ("Metallic", false, theme::text_1()),
            ("Studio", true, theme::text_2()),
        ] {
            let galley = painter.layout_job(wordmark_line(text, light, color));
            painter.galley(egui::pos2(body.left(), y), galley, color);
            y += WORDMARK_LINE;
        }
        painter.text(
            egui::pos2(body.left(), y + 18.0),
            Align2::LEFT_TOP,
            version_line(),
            fonts::small(),
            theme::text_3(),
        );

        // Status: step name and count above a 2 px track; the fill eases to the done share.
        let track = Rect::from_min_max(
            egui::pos2(body.left(), body.bottom() - 2.0),
            body.right_bottom(),
        );
        let (label, count) = self.status();
        // The centre of the 18 px status line that ends 10 px above the track.
        let line = track.top() - 10.0 - 9.0;
        painter.text(
            egui::pos2(body.left(), line),
            Align2::LEFT_CENTER,
            label,
            fonts::body(),
            theme::text_2(),
        );
        painter.text(
            egui::pos2(body.right(), line),
            Align2::RIGHT_CENTER,
            count,
            fonts::small(),
            theme::text_3(),
        );
        painter.rect_filled(track, 1.0, theme::bg_active());
        let shown = ui.ctx().animate_value_with_time(
            egui::Id::unique("splash-progress"),
            self.progress(),
            theme::t_panel(),
        );
        if shown > 0.0 {
            let fill = Rect::from_min_size(
                track.min,
                vec2(track.width() * shown.clamp(0.0, 1.0), track.height()),
            );
            painter.rect_filled(fill, 1.0, theme::accent());
        }
    }
}

/// "Version 0.1.1, dev channel". Studio has no channel setting yet, so this is the default
/// channel builds update from.
fn version_line() -> String {
    format!(
        "Version {}, {} channel",
        env!("CARGO_PKG_VERSION"),
        crate::updater::Channel::default().as_str()
    )
}

/// One wordmark line: Archivo at width 125, 44/46, tracked in by 2 %.
fn wordmark_line(text: &str, light: bool, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: fonts::wordmark(WORDMARK_SIZE, light),
            extra_letter_spacing: -0.02 * WORDMARK_SIZE,
            line_height: Some(WORDMARK_LINE),
            color,
            ..Default::default()
        },
    );
    job
}

/// The splash look for the first window: borderless, see-through outside the shape, fixed size.
pub fn viewport(builder: ViewportBuilder) -> ViewportBuilder {
    builder
        .with_inner_size(SIZE)
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
}

/// Turns the splash window into the normal Studio window, centred where the splash was.
pub fn become_main_window(ctx: &egui::Context) {
    let centre = ctx.input(|i| i.viewport().outer_rect.map(|r| r.center()));
    ctx.send_viewport_cmd(ViewportCommand::Decorations(true));
    ctx.send_viewport_cmd(ViewportCommand::Resizable(true));
    ctx.send_viewport_cmd(ViewportCommand::InnerSize(MAIN_SIZE));
    ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(MAIN_MIN_SIZE));
    if let Some(centre) = centre {
        let top_left = centre - MAIN_SIZE / 2.0;
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(
            top_left.x.max(0.0),
            top_left.y.max(0.0),
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_advance_one_at_a_time_and_report_their_count() {
        let mut s = Splash::new();
        assert_eq!(s.next_step(), Some(0));
        assert_eq!(
            s.status(),
            ("Loading fonts and settings", "1 of 4".to_owned())
        );
        assert_eq!(s.progress(), 0.0);
        s.step_done();
        assert_eq!(s.next_step(), Some(1));
        assert_eq!(s.status().1, "2 of 4");
        assert_eq!(s.progress(), 0.25);
        for _ in 0..10 {
            s.step_done();
        }
        assert_eq!(s.next_step(), None);
        assert_eq!(s.progress(), 1.0);
        assert_eq!(
            s.status(),
            ("Opening the start screen", "4 of 4".to_owned())
        );
    }

    #[test]
    fn the_splash_waits_for_every_step_and_the_minimum_time() {
        let mut s = Splash::new();
        assert!(!s.finished());
        s.since = Instant::now().checked_sub(MIN_TIME);
        assert!(!s.finished());
        for _ in STEPS {
            s.step_done();
        }
        assert!(s.finished());
        // Never painted yet: the minimum time has not started.
        let mut fresh = Splash::new();
        for _ in STEPS {
            fresh.step_done();
        }
        assert!(!fresh.finished());
        fresh.since = Some(Instant::now());
        assert!(!fresh.finished());
    }

    #[test]
    fn a_held_splash_never_runs_steps_or_finishes() {
        let s = Splash::held_at(1);
        assert_eq!(s.next_step(), None);
        assert_eq!(s.status().0, "Starting the renderer");
        assert!(!s.finished());
    }
}
