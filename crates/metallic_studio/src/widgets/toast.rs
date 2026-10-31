//! Toasts (research doc 3.7): short notes in the bottom-right corner, 20 px from the edges.
//!
//! A toast is a floating plate (`bg-hover`, level 3 shadow, top highlight), 300 to 420 px wide,
//! with a status icon, the text and an optional action. It enters over `t-slow` (easing out, from
//! 8 px lower), stays 4 s, and leaves over `t-base` (easing in). Errors stay until dismissed. At most
//! three show at once; a fourth pushes the oldest out.

use eframe::egui::{
    self, Align, Align2, Area, CornerRadius, Frame, Id, Layout, Margin, Order, RichText, Stroke,
    Ui, emath::easing, vec2,
};

use crate::theme::{self, fonts, icons};
use crate::widgets::Button;

/// How long a non-error toast stays, in seconds.
pub const STAY: f64 = 4.0;
/// Most toasts shown at once.
pub const MAX: usize = 3;
const MIN_WIDTH: f32 = 300.0;
const MAX_WIDTH: f32 = 420.0;
const EDGE: f32 = 20.0;
const GAP: f32 = 8.0;

/// The kind of note, which picks the icon and whether it times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Something finished.
    Success,
    /// Something to know.
    Info,
    /// Something failed; stays until dismissed.
    Error,
}

/// What the creator did with a toast this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastEvent {
    /// The toast's action button was pressed; carries the toast's `key`.
    Action(String),
}

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    tone: Tone,
    text: String,
    action: Option<(String, String)>,
    born: Option<f64>,
    leaving: Option<f64>,
    height: f32,
}

/// The toast stack of one screen.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
}

impl Toasts {
    /// Shows a note.
    pub fn push(&mut self, tone: Tone, text: impl Into<String>) {
        self.push_inner(tone, text.into(), None);
    }

    /// Shows a note with an action button `label`; pressing it returns
    /// [`ToastEvent::Action`] with `key` from [`Self::show`] and dismisses the toast.
    pub fn push_with_action(
        &mut self,
        tone: Tone,
        text: impl Into<String>,
        label: impl Into<String>,
        key: impl Into<String>,
    ) {
        self.push_inner(tone, text.into(), Some((label.into(), key.into())));
    }

    fn push_inner(&mut self, tone: Tone, text: String, action: Option<(String, String)>) {
        // The same note twice in a row only restarts its timer.
        if let Some(last) = self
            .items
            .iter_mut()
            .rev()
            .find(|t| t.leaving.is_none())
            .filter(|t| t.text == text && t.tone == tone)
        {
            last.born = None;
            return;
        }
        self.items.push(Toast {
            id: self.next,
            tone,
            text,
            action,
            born: None,
            leaving: None,
            height: 0.0,
        });
        self.next += 1;
        let staying = self.items.iter().filter(|t| t.leaving.is_none()).count();
        if staying > MAX
            && let Some(oldest) = self.items.iter_mut().find(|t| t.leaving.is_none())
        {
            oldest.leaving = Some(f64::NEG_INFINITY);
        }
    }

    /// Starts the newest toast's exit. Returns false when there was none (Esc uses this).
    pub fn dismiss_newest(&mut self) -> bool {
        match self.items.iter_mut().rev().find(|t| t.leaving.is_none()) {
            Some(t) => {
                t.leaving = Some(f64::NEG_INFINITY);
                true
            }
            None => false,
        }
    }

    /// The texts of the toasts still showing, oldest first (for tests).
    pub fn texts(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter(|t| t.leaving.is_none())
            .map(|t| t.text.as_str())
            .collect()
    }

    /// Draws the stack and runs the timers. Returns an action the creator pressed, if any.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<ToastEvent> {
        let now = ctx.input(|i| i.time);
        let mut event = None;
        let mut bottom = EDGE;
        // Newest at the bottom; older ones stack upwards.
        for toast in self.items.iter_mut().rev() {
            let born = *toast.born.get_or_insert(now);
            if toast.tone != Tone::Error && toast.leaving.is_none() && now - born >= STAY {
                toast.leaving = Some(now);
            }
            if toast.leaving == Some(f64::NEG_INFINITY) {
                toast.leaving = Some(now);
            }
            let enter = ((now - born) as f32 / theme::T_SLOW).clamp(0.0, 1.0);
            let exit = toast
                .leaving
                .map_or(0.0, |t| ((now - t) as f32 / theme::T_BASE).clamp(0.0, 1.0));
            let opacity = easing::cubic_out(enter) * (1.0 - easing::cubic_in(exit));
            let slide = 8.0 * (1.0 - easing::cubic_out(enter)) + 4.0 * easing::cubic_in(exit);
            let area = Area::new(Id::unique("toast").with(toast.id))
                .order(Order::Foreground)
                .anchor(Align2::RIGHT_BOTTOM, vec2(-EDGE, -bottom + slide))
                .interactable(exit == 0.0);
            let shown = area.show(ctx, |ui| {
                ui.multiply_opacity(opacity);
                draw(ui, toast)
            });
            toast.height = shown.response.rect.height();
            match shown.inner {
                Some(Pressed::Action) => {
                    if let Some((_, key)) = &toast.action {
                        event = Some(ToastEvent::Action(key.clone()));
                    }
                    toast.leaving.get_or_insert(now);
                }
                Some(Pressed::Dismiss) => {
                    toast.leaving.get_or_insert(now);
                }
                None => {}
            }
            bottom += toast.height + GAP;
            if enter < 1.0 || exit > 0.0 {
                ctx.request_repaint();
            }
        }
        let before = self.items.len();
        self.items.retain(|t| {
            t.leaving.is_none_or(|left| {
                left == f64::NEG_INFINITY || (now - left) as f32 <= theme::T_BASE
            })
        });
        if self.items.len() != before {
            ctx.request_repaint();
        }
        if let Some(next) = self
            .items
            .iter()
            .filter(|t| t.tone != Tone::Error && t.leaving.is_none())
            .filter_map(|t| t.born)
            .map(|born| born + STAY - now)
            .reduce(f64::min)
        {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(next.max(0.0)));
        }
        event
    }
}

enum Pressed {
    Action,
    Dismiss,
}

fn draw(ui: &mut Ui, toast: &Toast) -> Option<Pressed> {
    let (icon, colour) = match toast.tone {
        Tone::Success => (icons::CHECK_CIRCLE, theme::SUCCESS),
        Tone::Info => (icons::INFO, theme::INFO),
        Tone::Error => (icons::WARNING_CIRCLE, theme::ERROR),
    };
    let mut pressed = None;
    let frame = Frame::new()
        .fill(theme::BG_HOVER)
        .corner_radius(CornerRadius::same(theme::R_MD))
        .shadow(theme::SHADOW_POP)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_width(MIN_WIDTH - 28.0);
            ui.set_max_width(MAX_WIDTH - 28.0);
            ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
            // Lay the text out first so the row's height is known and icon, text and buttons
            // share one centre line. Room is left for the buttons on the right.
            let buttons = if toast.action.is_some() { 120.0 } else { 0.0 }
                + if toast.tone == Tone::Error { 32.0 } else { 0.0 };
            let text_w = (MAX_WIDTH - 28.0 - 16.0 - 20.0 - buttons).max(120.0);
            let mut job = egui::text::LayoutJob::single_section(
                toast.text.clone(),
                egui::TextFormat {
                    font_id: fonts::body(),
                    color: theme::TEXT_1,
                    line_height: Some(18.0),
                    ..Default::default()
                },
            );
            job.wrap.max_width = text_w;
            let text = ui.painter().layout_job(job);
            let row_h = text.size().y.max(28.0);
            let row_w = ui.available_width();
            ui.allocate_ui_with_layout(
                vec2(row_w, row_h),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_min_height(row_h);
                    ui.label(
                        RichText::new(icon)
                            .font(fonts::icon_fill(16.0))
                            .color(colour),
                    );
                    ui.label(text);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if toast.tone == Tone::Error
                            && ui
                                .add(Button::ghost("").icon(icons::X).alt("Dismiss").compact())
                                .clicked()
                        {
                            pressed = Some(Pressed::Dismiss);
                        }
                        if let Some((label, _)) = &toast.action
                            && ui.add(Button::ghost(label).compact()).clicked()
                        {
                            pressed = Some(Pressed::Action);
                        }
                    });
                },
            );
        });
    let rect = frame.response.rect;
    let r = f32::from(theme::R_MD);
    ui.painter().hline(
        rect.left() + r..=rect.right() - r,
        rect.top() + 0.5,
        Stroke::new(1.0, theme::HIGHLIGHT),
    );
    pressed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_most_three_stay_and_the_oldest_leaves_first() {
        let mut t = Toasts::default();
        for i in 0..4 {
            t.push(Tone::Success, format!("note {i}"));
        }
        assert_eq!(t.texts(), ["note 1", "note 2", "note 3"]);
    }

    #[test]
    fn a_repeated_note_is_not_stacked_twice() {
        let mut t = Toasts::default();
        t.push(Tone::Error, "gone");
        t.push(Tone::Error, "gone");
        assert_eq!(t.texts(), ["gone"]);
    }

    #[test]
    fn dismissing_takes_the_newest_first() {
        let mut t = Toasts::default();
        t.push(Tone::Info, "a");
        t.push_with_action(Tone::Error, "b", "Remove", "key");
        assert!(t.dismiss_newest());
        assert_eq!(t.texts(), ["a"]);
        assert!(t.dismiss_newest());
        assert!(!t.dismiss_newest());
    }

    #[test]
    fn notes_time_out_but_errors_stay() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let mut t = Toasts::default();
        t.push(Tone::Success, "done");
        t.push(Tone::Error, "failed");
        for time in [0.0, 1.0, STAY + 0.1, STAY + 1.0] {
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                t.show(ui.ctx());
            })
            .textures_delta
            .clear();
        }
        assert_eq!(t.texts(), ["failed"]);
    }
}
