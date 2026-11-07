//! The Viewport panel: where the engine draws the open scene. A placeholder in this slice; the viewport issue (#217)
//! draws the scene into a texture of the panel's size here.

use egui::{Align2, FontId, Ui};

use super::{icon, ids};
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::tokens::text;
use crate::theme::{current_colors, fonts};

/// The Viewport panel.
#[derive(Clone, Copy, Debug)]
pub struct Viewport;

impl Panel for Viewport {
    fn id(&self) -> &'static str {
        ids::VIEWPORT
    }

    fn title(&self) -> &'static str {
        "Viewport"
    }

    fn icon(&self) -> char {
        icon::MONITOR
    }

    fn zone(&self) -> Zone {
        Zone::Centre
    }

    fn context(&self) -> Context {
        Context::Viewport
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        let p = current_colors(ui.ctx()).palette;
        // The panel's whole area, past the margin the dock leaves: the scene will fill it edge to edge.
        let rect = ui.max_rect().expand(crate::theme::tokens::space::S4);
        ui.painter().rect_filled(rect, 0.0, p.bg);
        let centre = rect.center();
        ui.painter().text(
            centre - egui::vec2(0.0, 14.0),
            Align2::CENTER_CENTER,
            cx.editor.name(),
            fonts::named(fonts::HEADING, text::TITLE),
            p.muted,
        );
        ui.painter().text(
            centre + egui::vec2(0.0, 14.0),
            Align2::CENTER_CENTER,
            "The scene's view will be drawn here.",
            FontId::proportional(text::BODY),
            p.subtle,
        );
        ui.allocate_rect(rect, egui::Sense::hover());
    }
}
