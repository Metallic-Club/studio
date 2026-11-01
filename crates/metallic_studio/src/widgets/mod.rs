//! Studio's own widgets on top of egui: the cut, plates, buttons and keycaps (research doc
//! sections 3.3, 3.4 and 3.7), the start screen's cards, segmented filter and toasts, and the
//! number fields of Properties ([`number`]).
//! [`components`] draws the basic ones in every state for design review (`--scene components`).

pub mod button;
pub mod card;
pub mod components;
pub mod cut;
pub mod number;
pub mod plate;
pub mod segmented;
pub mod toast;

use std::sync::Arc;

use eframe::egui::{Color32, FontId, Galley, Painter, text::LayoutJob};

pub use button::{Button, Kind, ShowAs, keycap};
pub use card::Card;
pub use plate::Plate;
pub use segmented::segmented;
pub use toast::{Toasts, Tone};

/// Lays `text` out on one line, cut with "…" so it fits in `width`.
pub fn truncated(
    painter: &Painter,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = eframe::egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    painter.layout_job(job)
}
