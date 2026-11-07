//! The Game panel (Play in Studio, ADR 0010 §4): the running game, drawn by its Player from
//! `Workspace.CurrentCamera`, filling the panel edge to edge. Play opens it in front of the Viewport and Stop closes
//! it; it is not in the View menu and never saved in the layout. The panel notes its size in physical pixels and shows
//! the texture the shell draws the game into (`app::game`).

use egui::emath::GuiRounding;
use egui::{Align2, FontId, Sense, Ui};

use super::ids;
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::current_colors;
use crate::theme::icons::Icon;
use crate::theme::tokens::text;

/// The Game panel.
#[derive(Clone, Copy, Debug)]
pub struct Game;

impl Panel for Game {
    fn id(&self) -> &'static str {
        ids::GAME
    }

    fn title(&self) -> &'static str {
        "Game"
    }

    fn icon(&self) -> Icon {
        Icon::PanelGame
    }

    fn zone(&self) -> Zone {
        Zone::Centre
    }

    fn context(&self) -> Context {
        Context::Editor
    }

    fn open_by_default(&self) -> bool {
        false
    }

    fn transient(&self) -> bool {
        true
    }

    fn ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        let p = current_colors(ui.ctx()).palette;
        let pixels_per_point = ui.ctx().pixels_per_point();
        // Edge to edge, on whole pixels, as the Viewport.
        let rect = ui
            .max_rect()
            .expand(crate::theme::tokens::space::S4)
            .round_to_pixels(pixels_per_point);
        ui.set_clip_rect(rect);
        let note = match cx.editor.play.as_mut() {
            Some(session) => {
                let view = &mut session.view;
                let size = (rect.size() * pixels_per_point).round();
                view.rect = Some(rect);
                view.pixels = Some((size.x.max(1.0) as u32, size.y.max(1.0) as u32));
                view.shown = true;
                match view.texture {
                    Some(texture) => {
                        // At its own size while a resize is under way (#311).
                        super::draw_render(ui, texture, view.texture_pixels, view.texture_limit, rect);
                        None
                    }
                    None => Some(
                        view.error
                            .clone()
                            .unwrap_or_else(|| "Starting the game\u{2026}".to_owned()),
                    ),
                }
            }
            None => Some("Press Play to run the game here.".to_owned()),
        };
        if let Some(note) = note {
            ui.painter().rect_filled(rect, 0.0, p.bg);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                note,
                FontId::proportional(text::BODY),
                p.subtle,
            );
        }
        ui.allocate_rect(rect, Sense::hover());
    }
}
