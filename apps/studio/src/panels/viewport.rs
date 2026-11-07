//! The Viewport panel: the engine's render of the open game, filling the panel edge to edge (see
//! [`crate::viewport`]). The panel notes its size in physical pixels and shows the texture the shell draws; until
//! there is one it shows the project's name.

use std::time::Duration;

use egui::emath::GuiRounding;
use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2};

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
        let pixels_per_point = ui.ctx().pixels_per_point();
        // The panel's whole area, past the margin the dock leaves, on whole pixels so the texture is drawn one texel
        // to a pixel.
        let rect = ui
            .max_rect()
            .expand(crate::theme::tokens::space::S4)
            .round_to_pixels(pixels_per_point);
        // The dock clips a panel to inside its margin; the viewport is the one panel that fills it.
        ui.set_clip_rect(rect);
        let viewport = &mut cx.editor.viewport;
        let size = (rect.size() * pixels_per_point).round();
        viewport.rect = Some(rect);
        viewport.pixels = Some((size.x.max(1.0) as u32, size.y.max(1.0) as u32));
        viewport.shown = true;
        match viewport.texture {
            // During a resize this is the old size's texture, stretched: never a black frame.
            Some(texture) => {
                ui.painter().image(
                    texture,
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                ui.painter().rect_filled(rect, 0.0, p.bg);
                let centre = rect.center();
                ui.painter().text(
                    centre - egui::vec2(0.0, 14.0),
                    Align2::CENTER_CENTER,
                    &cx.editor.project.manifest().name,
                    fonts::named(fonts::HEADING, text::TITLE),
                    p.muted,
                );
                let note = viewport.error.as_deref().unwrap_or("Drawing the scene\u{2026}");
                ui.painter().text(
                    centre + egui::vec2(0.0, 14.0),
                    Align2::CENTER_CENTER,
                    note,
                    FontId::proportional(text::BODY),
                    p.subtle,
                );
            }
        }
        ui.allocate_rect(rect, Sense::hover());
        // Frame pacing: continuous while the viewport shows and the window has focus, at the frame cap if there is
        // one. Otherwise Studio waits for input or a change.
        if ui.input(|input| input.focused) {
            match cx.settings.rendering.frame_cap.filter(|cap| *cap > 0) {
                Some(fps) => ui
                    .ctx()
                    .request_repaint_after(Duration::from_secs_f64(1.0 / f64::from(fps))),
                None => ui.ctx().request_repaint(),
            }
        }
    }
}
