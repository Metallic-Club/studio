//! The Viewport panel: the engine's render of the open game, filling the panel edge to edge (see
//! [`crate::viewport`]). The panel notes its size in physical pixels and shows the texture the shell draws; until
//! there is one it shows the project's name.
//!
//! A left click selects what is under the pointer (#354, [`crate::viewport::pick`]): Ctrl (Cmd) or Shift adds or
//! removes it, a click on nothing clears the selection. Alt+click picks the part itself, even inside a model, and
//! on the same spot again the part behind it; Shift+Alt adds each (#482, [`crate::viewport::pick::PickCycle`]). The
//! selection is outlined in the accent colour, and the part under the pointer faintly. A drag selects too, as the
//! Select tool's mode says (#467, [`crate::viewport::select`]). Not while a game runs: the viewport shows the
//! editor's scene, and the panels the game's.

use std::time::Duration;

use egui::emath::GuiRounding;
use egui::{Align2, FontId, Painter, Rect, Sense, Stroke, Ui};
use metallic_engine::metallic_core::{DataModel, Id};
use metallic_engine::metallic_render::ShadingView;
use metallic_engine::perf::StatsViewer;

use super::ids;
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::icons::Icon;
use crate::theme::tokens::text;
use crate::theme::{current_colors, fonts};
use crate::viewport::index::PartIndex;
use crate::viewport::light_icons;
use crate::viewport::lines::Lines;
use crate::viewport::pick::{self, EDGES};
use crate::viewport::tools;
use crate::viewport::{SpeedChange, View, ViewportState};

/// How thick the selection's outline is, in points.
const OUTLINE: f32 = 1.5;
/// A box less than this many points across on screen is outlined as a dot (#597): its edges would lie on one another.
const TINY: f32 = 4.0;
/// Up to how many selected instances the outlines are anti-aliased (#597).
const SMOOTH_UP_TO: usize = 1_000;
/// How long the speed readout stays after the wheel last changed a speed (#385).
const SPEED_NOTE: Duration = Duration::from_millis(900);

/// The Viewport panel.
#[derive(Clone, Copy, Debug)]
pub struct Viewport;

/// Adds `id`'s outline box (a part's own, a model's round its parts), or a light's reach (#523), with `stroke` to `lines`. A
/// light's reach is drawn over everything when `on_top`, else hidden where parts are in front of it (#572).
fn outline(lines: &mut Lines, view: &View, dm: &DataModel, parts: &PartIndex, id: Id, stroke: Stroke, on_top: bool) {
    // A light's reach (#523): it has no box of its own.
    for line in light_icons::range_lines(dm, id) {
        let pieces = if on_top {
            vec![line]
        } else {
            light_icons::seen_pieces(dm, parts, view.pos, line, id)
        };
        for [a, b] in pieces {
            if let Some(line) = view.segment(a, b) {
                lines.segment(line, stroke);
            }
        }
    }
    // Parts and models only (#616): the Camera or a service would outline everything inside it.
    let Some(bounds) = pick::outline_box(dm, id).filter(|_| pick::is_outlined(dm, id)) else {
        return;
    };
    let corners = bounds.corners();
    match view.project_all(corners) {
        // In front of the camera: each corner turned once, the box left out when it is wholly off the panel.
        Some(screen) => {
            let area = Rect::from_points(&screen);
            if !lines.meets(area) {
                return;
            }
            if area.width() < TINY && area.height() < TINY {
                lines.dot(area.center(), stroke);
                return;
            }
            for (a, b) in EDGES {
                lines.segment([screen[a], screen[b]], stroke);
            }
        }
        // Partly behind it: each edge cut at the near plane.
        None => {
            for (a, b) in EDGES {
                if let Some(line) = view.segment(corners[a], corners[b]) {
                    lines.segment(line, stroke);
                }
            }
        }
    }
}

/// The Debug section of the Viewport's menu and the Game tab's (#492): the performance overlay's viewers (#479), as
/// the Player draws them, over the viewport and over a running game alike; Shift+F1 to F4 by default.
pub(super) fn debug_section(ui: &mut Ui, cx: &mut PanelContext<'_>) {
    crate::theme::section(ui, "metallic.viewport.menu.debug", "Debug", |ui| {
        indented(ui, |ui| {
            let stats = &mut cx.editor.viewport.stats;
            for viewer in StatsViewer::ALL {
                let mut on = stats.contains(viewer);
                if crate::theme::checkbox(ui, &mut on, viewer.name()).changed() {
                    stats.set(viewer, on);
                }
            }
        });
    });
}

/// A section's rows in the Viewport's menu (#477), in line with its label, past the chevron.
fn indented(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(crate::theme::tokens::text::BODY + crate::theme::tokens::space::S5);
        ui.vertical(add_contents);
    });
}

/// What the speed note says for `change` (#385).
fn speed_text(change: SpeedChange, speeds: &crate::settings::Viewport) -> String {
    let fly = crate::panels::properties::number(f64::from(speeds.fly_speed));
    match change {
        SpeedChange::Fly => format!("Fly speed  {fly} studs/s"),
        SpeedChange::Secondary => {
            let fast = crate::panels::properties::number(f64::from(speeds.fly_speed * speeds.secondary_speed));
            let times = crate::panels::properties::number(f64::from(speeds.secondary_speed));
            format!("Secondary speed  \u{D7}{times}  ({fast} studs/s)")
        }
    }
}

/// The camera speed the wheel just changed (#385), or the Select tool's mode just chosen (#467), whichever is newer,
/// in small plain text in the bottom-left corner for a moment, with a faint shadow to read on a bright sky, as Forge's.
fn note(
    painter: &Painter,
    rect: egui::Rect,
    viewport: &ViewportState,
    mode: crate::viewport::select::SelectMode,
    speeds: &crate::settings::Viewport,
) {
    let speed = viewport.speed_note.filter(|(_, when)| when.elapsed() < SPEED_NOTE);
    let chosen = viewport.mode_note.filter(|when| when.elapsed() < SPEED_NOTE);
    let text = match (speed, chosen) {
        (Some((change, said)), Some(when)) if said >= when => speed_text(change, speeds),
        (_, Some(_)) => format!("Select mode  {}", mode.label()),
        (Some((change, _)), None) => speed_text(change, speeds),
        (None, None) => return,
    };
    let at = rect.left_bottom() + egui::vec2(14.0, -12.0);
    let font = FontId::proportional(text::BODY);
    painter.text(
        at + egui::vec2(1.0, 1.0),
        Align2::LEFT_BOTTOM,
        &text,
        font.clone(),
        egui::Color32::from_black_alpha(140),
    );
    painter.text(
        at,
        Align2::LEFT_BOTTOM,
        text,
        font,
        egui::Color32::from_white_alpha(235),
    );
    painter.ctx().request_repaint_after(SPEED_NOTE);
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

    fn has_menu(&self) -> bool {
        true
    }

    /// What the viewport shows (#387), in sections that fold (#477).
    fn menu(&self, ui: &mut Ui, cx: &mut PanelContext<'_>) {
        ui.set_min_width(crate::theme::MENU_WIDTH);
        ui.spacing_mut().item_spacing.y = crate::theme::tokens::space::S2;
        crate::theme::section(ui, "metallic.viewport.menu.view", "View", |ui| {
            indented(ui, |ui| {
                crate::theme::checkbox(ui, &mut cx.settings.viewport.show_axes, "Axis gizmo");
                crate::theme::checkbox(ui, &mut cx.settings.viewport.show_light_icons, "Light icons");
                crate::theme::checkbox(
                    ui,
                    &mut cx.settings.viewport.light_lines_on_top,
                    "Light lines over everything",
                )
                .on_hover_text("Off, parts in front of a selected light's range lines hide them.");
            });
        });
        // How the parts are drawn (#478). The wireframe needs a device that draws lines.
        let wireframe = cx.editor.viewport.wireframe;
        crate::theme::section(ui, "metallic.viewport.menu.shading", "Shading", |ui| {
            indented(ui, |ui| {
                for view in ShadingView::ALL {
                    let enabled = view != ShadingView::Wireframe || wireframe;
                    let chosen = cx.settings.viewport.shading == view;
                    let response = crate::theme::radio(ui, chosen, enabled, view.name());
                    if response.clicked() {
                        cx.settings.viewport.shading = view;
                    }
                    if !enabled {
                        response.on_hover_text("This graphics device cannot draw lines.");
                    }
                }
            });
        });
        debug_section(ui, cx);
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
            // The increments, where they are on (#359).
            let editing = cx.settings.editing;
            let snap = tools::Snap {
                move_step: if editing.snap_moves { editing.move_step } else { 0.0 },
                turn_step: if editing.snap_turns { editing.turn_step } else { 0.0 },
            };
            // The index is lent to the tools, which hold the whole scene (#596).
            let parts = std::mem::take(&mut cx.editor.parts);
            let on_gizmo = tools::interact(ui, &response, &view, &mut cx.editor.scene(), &parts, &mut gizmo, snap);
            let editor = &mut *cx.editor;
            editor.parts = parts;
            // What a drag moved this frame, before what is under the pointer is picked.
            editor.refresh_parts();
            // The lights' icons (#506), clicked before what is behind them.
            let lights = if cx.settings.viewport.show_light_icons {
                editor
                    .viewport
                    .light_icons
                    .icons(&editor.dm, &editor.parts, &view)
                    .to_vec()
            } else {
                Vec::new()
            };
            let under = |at: egui::Pos2| {
                light_icons::icon_at(&lights, at)
                    .or_else(|| view.ray(at).and_then(|ray| pick::pick(&editor.dm, &editor.parts, ray)))
            };
            let hovered = response.hover_pos().filter(|_| !on_gizmo).and_then(under);
            let modifiers = ui.input(|input| input.modifiers);
            // Alt means "select": it passes through a gizmo's handle to what is behind (#482).
            // Picking a parent from the Parent row (#599): what a click would select (Alt: the part itself) is the
            // parent, and the selection stays.
            let picking = editor.explorer.parent_pick.is_some();
            if picking && response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            if picking && response.clicked() {
                let at = response.interact_pointer_pos();
                let target = if modifiers.alt {
                    at.and_then(|at| view.ray(at))
                        .and_then(|ray| pick::nearest_part(&editor.dm, &editor.parts, ray))
                        .map(|(part, _)| part)
                } else {
                    at.and_then(under)
                };
                if let Some(target) = target {
                    editor.scene().pick_parent(target);
                }
            } else if response.clicked() && (modifiers.alt || !on_gizmo) {
                let adding = modifiers.command || modifiers.shift;
                let at = response.interact_pointer_pos();
                if let Some(light) = at.and_then(|at| light_icons::icon_at(&lights, at)) {
                    // A light's icon (#506).
                    editor.viewport.pick_cycle.reset();
                    if adding {
                        editor.selection.toggle(light);
                    } else {
                        editor.select(Some(light));
                    }
                } else if modifiers.alt {
                    // The part itself, even inside a model, and on the same spot again the one behind it (#482).
                    let hits = at
                        .and_then(|at| view.ray(at))
                        .map(|ray| pick::parts_along(&editor.dm, &editor.parts, ray))
                        .unwrap_or_default();
                    let at = at.map_or([0.0; 2], |at| [at.x, at.y]);
                    match editor.viewport.pick_cycle.advance(at, &hits, pick::CYCLE_TOLERANCE) {
                        Some(id) if adding => {
                            editor.selection.add(id);
                            editor.explorer.reveal(&editor.dm, id);
                        }
                        Some(id) => editor.select(Some(id)),
                        None if adding => {}
                        None => editor.selection.clear(),
                    }
                } else {
                    editor.viewport.pick_cycle.reset();
                    match at.and_then(under) {
                        Some(id) if adding => editor.selection.toggle(id),
                        Some(id) => editor.select(Some(id)),
                        None if adding => {}
                        None => editor.selection.clear(),
                    }
                }
            }
            let painter = ui.painter().with_clip_rect(rect);
            let accent = current_colors(ui.ctx()).accent;
            let on_top = cx.settings.viewport.light_lines_on_top;
            // Every outline in one mesh (#597): thousands selected draw thousands of boxes.
            let mut lines = Lines::new(rect, ui.ctx().pixels_per_point());
            lines.reserve(editor.selection.len() * EDGES.len());
            // A selection this large is drawn with sharp lines, half the geometry (#597).
            lines.set_smooth(editor.selection.len() <= SMOOTH_UP_TO);
            // Under the pointer, faintly, while a tool is active (#466) or a parent is picked (#599).
            if let Some(id) = hovered.filter(|&id| (gizmo.tool.is_some() || picking) && !editor.selection.contains(id))
            {
                outline(
                    &mut lines,
                    &view,
                    &editor.dm,
                    &editor.parts,
                    id,
                    Stroke::new(1.0, accent.gamma_multiply(0.5)),
                    on_top,
                );
            }
            for &id in editor.selection.ids() {
                outline(
                    &mut lines,
                    &view,
                    &editor.dm,
                    &editor.parts,
                    id,
                    Stroke::new(OUTLINE, accent),
                    on_top,
                );
            }
            if !lines.is_empty() {
                painter.add(lines.into_shape());
            }
            light_icons::paint(
                &painter,
                &lights,
                editor.selection.ids(),
                hovered,
                &current_colors(ui.ctx()),
            );
            let revision = editor.parts.revision();
            tools::paint(&painter, &view, &editor.scene(), revision, &mut gizmo, snap);
            let gizmo_mode = gizmo.select_mode;
            editor.viewport.gizmo = gizmo;
            note(&painter, rect, &editor.viewport, gizmo_mode, &cx.settings.viewport);
        }
        // Which way the axes point (#386), unless the Viewport's menu turned it off (#387).
        if cx.settings.viewport.show_axes {
            let rotation = cx.editor.viewport.camera.rotation();
            crate::viewport::axes::paint(&ui.painter().with_clip_rect(rect), rect, rotation);
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
