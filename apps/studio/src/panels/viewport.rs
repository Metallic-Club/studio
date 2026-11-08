//! The Viewport panel: the engine's render of the open game, filling the panel edge to edge (see
//! [`crate::viewport`]). The panel notes its size in physical pixels and shows the texture the shell draws; until
//! there is one it shows the project's name.
//!
//! A left click selects what is under the pointer (#354, [`crate::viewport::pick`]): Ctrl (Cmd) or Shift adds or
//! removes it, a click on nothing clears the selection, and Alt is left to the camera's orbit. The selection is
//! outlined in the accent colour, and the part under the pointer faintly. Not while a game runs: the viewport shows
//! the editor's scene, and the panels the game's.

use std::time::Duration;

use egui::emath::GuiRounding;
use egui::{Align2, FontId, Painter, Sense, Stroke, Ui};
use metallic_engine::metallic_core::{DataModel, Id};

use super::ids;
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::icons::Icon;
use crate::theme::tokens::text;
use crate::theme::{current_colors, fonts};
use crate::viewport::View;
use crate::viewport::pick::{self, EDGES};
use crate::viewport::tools;

/// How thick the selection's outline is, in points.
const OUTLINE: f32 = 1.5;

/// The Viewport panel.
#[derive(Clone, Copy, Debug)]
pub struct Viewport;

/// Draws `id`'s outline box (a part's own, a model's round its parts) with `stroke`.
fn outline(painter: &Painter, view: &View, dm: &DataModel, id: Id, stroke: Stroke) {
    let Some(bounds) = pick::outline_box(dm, id) else {
        return;
    };
    let corners = bounds.corners();
    for (a, b) in EDGES {
        if let Some(line) = view.segment(corners[a], corners[b]) {
            painter.line_segment(line, stroke);
        }
    }
}

impl Panel for Viewport {
    fn id(&self) -> &'static str {
        ids::VIEWPORT
    }

    fn title(&self) -> &'static str {
        "Viewport"
    }

    fn icon(&self) -> Icon {
        Icon::PanelViewport
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
            // During a resize this is the old size's frame, drawn at its own size (#311): never stretched, never black.
            Some(texture) => {
                super::draw_render(ui, texture, viewport.texture_pixels, viewport.texture_limit, rect);
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
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        if !cx.editor.is_playing() {
            let view = View::new(&cx.editor.viewport.camera, rect);
            // The tool's gizmo first: a click on it is not a selection (#355).
            let mut gizmo = std::mem::take(&mut cx.editor.viewport.gizmo);
            let snap = tools::Snap::default();
            let on_gizmo = tools::interact(ui, &response, &view, &mut cx.editor.scene(), &mut gizmo, snap);
            let editor = &mut *cx.editor;
            let under = |at| view.ray(at).and_then(|ray| pick::pick(&editor.dm, ray));
            let hovered = response.hover_pos().filter(|_| !on_gizmo).and_then(under);
            let modifiers = ui.input(|input| input.modifiers);
            if response.clicked() && !modifiers.alt && !on_gizmo {
                let picked = response.interact_pointer_pos().and_then(under);
                match picked {
                    Some(id) if modifiers.command || modifiers.shift => editor.selection.toggle(id),
                    Some(id) => editor.select(Some(id)),
                    None if modifiers.command || modifiers.shift => {}
                    None => editor.selection.clear(),
                }
            }
            let painter = ui.painter().with_clip_rect(rect);
            let accent = current_colors(ui.ctx()).accent;
            if let Some(id) = hovered.filter(|&id| !editor.selection.contains(id)) {
                outline(
                    &painter,
                    &view,
                    &editor.dm,
                    id,
                    Stroke::new(1.0, accent.gamma_multiply(0.5)),
                );
            }
            for &id in editor.selection.ids() {
                outline(&painter, &view, &editor.dm, id, Stroke::new(OUTLINE, accent));
            }
            tools::paint(&painter, &view, &editor.scene(), &gizmo);
            editor.viewport.gizmo = gizmo;
        }
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
