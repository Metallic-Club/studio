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

use crate::theme::Tip as _;
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

/// The View section of the Viewport's and the Game tab's menus (#387, #834): what is drawn over the world.
pub(crate) fn view_section(ui: &mut Ui, cx: &mut PanelContext<'_>) {
    crate::theme::section(ui, "metallic.viewport.menu.view", "View", |ui| {
        indented(ui, |ui| {
            crate::theme::checkbox(ui, &mut cx.settings.viewport.show_axes, "Axis gizmo");
            crate::theme::checkbox(ui, &mut cx.settings.viewport.show_light_icons, "Light icons");
            crate::theme::checkbox(
                ui,
                &mut cx.settings.viewport.light_lines_on_top,
                "Light lines over everything",
            )
            .tip("Off, parts in front of a selected light's range lines hide them.");
        });
    });
}

/// The Shading section of the Viewport's and the Game tab's menus (#478, #834): how the parts are drawn. The
/// wireframe needs a device that draws lines.
pub(crate) fn shading_section(ui: &mut Ui, cx: &mut PanelContext<'_>) {
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
                    response.tip("This graphics device cannot draw lines.");
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
        SpeedChange::Fly => format!("Fly speed  {fly} units/s"),
        SpeedChange::Secondary => {
            let fast = crate::panels::properties::number(f64::from(speeds.fly_speed * speeds.secondary_speed));
            let times = crate::panels::properties::number(f64::from(speeds.secondary_speed));
            format!("Secondary speed  \u{D7}{times}  ({fast} units/s)")
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

    /// "Server" while a game runs (ADR 0011 §8): the tab shows the running game's world then, the Game tab a player's
    /// view of it.
    fn pane_title(&self, _pane: &str, cx: &PanelContext<'_>) -> String {
        if cx.editor.is_playing() { "Server" } else { self.title() }.to_owned()
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
        view_section(ui, cx);
        shading_section(ui, cx);
        // Where the wheel zooms (#775), as Settings has it too.
        crate::theme::section(ui, "metallic.viewport.menu.zoom", "Zoom", |ui| {
            indented(ui, |ui| {
                for target in crate::settings::ZoomTarget::ALL {
                    let chosen = cx.settings.viewport.zoom_to == target;
                    if crate::theme::radio(ui, chosen, true, target.label()).clicked() {
                        cx.settings.viewport.zoom_to = target;
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
        // The scene shown (#835): the editor's, or the running game's while one runs, whose selection and tools work
        // as the editor's do (the edits are the session's, dropped at Stop).
        {
            let playing = cx.editor.is_playing();
            let view = View::new(&cx.editor.viewport.camera, rect);
            // The tool's gizmo first: a click on it is not a selection (#355).
            let mut gizmo = std::mem::take(&mut cx.editor.viewport.gizmo);
            let mut light_cache = std::mem::take(&mut cx.editor.viewport.light_icons);
            let mut pick_cycle = std::mem::take(&mut cx.editor.viewport.pick_cycle);
            // The increments, where they are on (#359).
            let editing = cx.settings.editing;
            let snap = tools::Snap {
                move_step: if editing.snap_moves { editing.move_step } else { 0.0 },
                turn_step: if editing.snap_turns { editing.turn_step } else { 0.0 },
            };
            // The index is lent to the tools, which hold the whole scene (#596).
            let mut parts = std::mem::take(cx.editor.shown_parts_mut());
            let show_light_icons = cx.settings.viewport.show_light_icons;
            let on_top = cx.settings.viewport.light_lines_on_top;
            let accent = current_colors(ui.ctx()).accent;
            let painter = ui.painter().with_clip_rect(rect);
            let mut dropped = None;
            {
                let mut scene = cx.editor.scene();
                parts.update(scene.dm);
                let on_gizmo = tools::interact(ui, &response, &view, &mut scene, &parts, &mut gizmo, snap);
                // What a drag moved this frame, before what is under the pointer is picked.
                parts.update(scene.dm);
                // The lights' icons (#506), clicked before what is behind them.
                let lights = if show_light_icons {
                    light_cache.icons(scene.dm, &parts, &view).to_vec()
                } else {
                    Vec::new()
                };
                // A material dragged from the Assets panel and let go on a part gives it the material (#680): the
                // editor's scene only.
                if !playing
                    && let Some(drag) = response.dnd_release_payload::<crate::panels::assets::AssetDrag>()
                    && let [path] = drag.paths.as_slice()
                    && metallic_engine::metallic_project::material::is_material(path)
                    && let Some(at) = ui.input(|input| input.pointer.latest_pos())
                    && let Some((part, _)) = view.ray(at).and_then(|ray| pick::nearest_part(scene.dm, &parts, ray))
                {
                    dropped = Some((path.clone(), part));
                }
                let dm: &DataModel = scene.dm;
                let under = |at: egui::Pos2| {
                    light_icons::icon_at(dm, &lights, at)
                        .or_else(|| view.ray(at).and_then(|ray| pick::pick(dm, &parts, ray)))
                };
                let hovered = response.hover_pos().filter(|_| !on_gizmo).and_then(under);
                let modifiers = ui.input(|input| input.modifiers);
                // Alt means "select": it passes through a gizmo's handle to what is behind (#482).
                // Picking a parent from the Parent row (#599): what a click would select (Alt: the part itself) is the
                // parent, and the selection stays.
                let picking = scene.explorer.parent_pick.is_some();
                if picking && response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
                let mut clicked: Option<Option<Id>> = None;
                let mut parent: Option<Id> = None;
                // Alt with Shift or Ctrl adds the part cycled to, rather than toggling it (#482).
                let mut cycled = false;
                let adding = modifiers.command || modifiers.shift;
                if picking && response.clicked() {
                    let at = response.interact_pointer_pos();
                    parent = if modifiers.alt {
                        at.and_then(|at| view.ray(at))
                            .and_then(|ray| pick::nearest_part(dm, &parts, ray))
                            .map(|(part, _)| part)
                    } else {
                        at.and_then(under)
                    };
                } else if response.clicked() && (modifiers.alt || !on_gizmo) {
                    let at = response.interact_pointer_pos();
                    if let Some(light) = at.and_then(|at| light_icons::icon_at(dm, &lights, at)) {
                        // A light's icon (#506).
                        pick_cycle.reset();
                        clicked = Some(Some(light));
                    } else if modifiers.alt {
                        // The part itself, even inside a model, and on the same spot again the one behind it (#482).
                        let hits = at
                            .and_then(|at| view.ray(at))
                            .map(|ray| pick::parts_along(dm, &parts, ray))
                            .unwrap_or_default();
                        let at = at.map_or([0.0; 2], |at| [at.x, at.y]);
                        clicked = Some(pick_cycle.advance(at, &hits, pick::CYCLE_TOLERANCE));
                        cycled = true;
                    } else {
                        pick_cycle.reset();
                        clicked = Some(at.and_then(under));
                    }
                }
                if let Some(target) = parent {
                    scene.pick_parent(target);
                }
                match clicked {
                    Some(Some(id)) if adding => {
                        if scene.selection.contains(id) && !cycled {
                            scene.selection.toggle(id);
                        } else {
                            scene.selection.add(id);
                            scene.explorer.reveal(scene.dm, id);
                        }
                    }
                    Some(Some(id)) => {
                        scene.selection.set_only(Some(id));
                        scene.explorer.reveal(scene.dm, id);
                    }
                    Some(None) if !adding => scene.selection.clear(),
                    _ => {}
                }
                let dm: &DataModel = scene.dm;
                // Every outline in one mesh (#597): thousands selected draw thousands of boxes.
                let mut lines = Lines::new(rect, ui.ctx().pixels_per_point());
                lines.reserve(scene.selection.len() * EDGES.len());
                // A selection this large is drawn with sharp lines, half the geometry (#597).
                lines.set_smooth(scene.selection.len() <= SMOOTH_UP_TO);
                // Under the pointer, faintly, while a tool is active (#466) or a parent is picked (#599).
                if let Some(id) =
                    hovered.filter(|&id| (gizmo.tool.is_some() || picking) && !scene.selection.contains(id))
                {
                    let faint = Stroke::new(1.0, accent.gamma_multiply(0.5));
                    outline(&mut lines, &view, dm, &parts, id, faint, on_top);
                }
                for &id in scene.selection.ids() {
                    outline(&mut lines, &view, dm, &parts, id, Stroke::new(OUTLINE, accent), on_top);
                }
                if !lines.is_empty() {
                    painter.add(lines.into_shape());
                }
                light_icons::paint(
                    &painter,
                    &lights,
                    scene.selection.ids(),
                    hovered,
                    &current_colors(ui.ctx()),
                );
                tools::paint(&painter, &view, &scene, parts.revision(), &mut gizmo, snap);
            }
            let editor = &mut *cx.editor;
            *editor.shown_parts_mut() = parts;
            editor.viewport.light_icons = light_cache;
            editor.viewport.pick_cycle = pick_cycle;
            if let Some((path, part)) = dropped
                && let Some(id) = editor.project.assets().id(&path)
                && let Err(error) = editor.apply_material_to(id, part)
            {
                log::warn!("materials: {error}");
            }
            if !playing {
                // Team Create (ADR 0011): what the others have selected, in their colours, and where they look from.
                let mut lines = Lines::new(rect, ui.ctx().pixels_per_point());
                crate::team::viewport::outlines(&mut lines, &view, &editor.dm, &editor.presence);
                if !lines.is_empty() {
                    painter.add(lines.into_shape());
                }
                let now = std::time::Instant::now();
                crate::team::viewport::cameras(&painter, &view, &editor.presence, now);
                if editor.presence.gliding(now) {
                    ui.ctx().request_repaint();
                }
            }
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
            let refresh = crate::settings::display_refresh(ui.ctx());
            match crate::settings::frame_cap(&cx.settings.rendering, refresh) {
                Some(fps) => ui
                    .ctx()
                    .request_repaint_after(Duration::from_secs_f64(1.0 / f64::from(fps))),
                None => ui.ctx().request_repaint(),
            }
        }
    }
}

/// What the Game tab draws over a running game (#834, #835), from the game's own camera (`Workspace.CurrentCamera`),
/// as the View section of its menu says: the selection's outlines, the lights' icons and the axis gizmo. `view` is
/// that camera drawing into the tab's `rect`.
pub(crate) fn game_overlay(
    ui: &Ui,
    view: &View,
    dm: &DataModel,
    parts: &PartIndex,
    selection: &crate::editor::Selection,
    settings: &crate::settings::Viewport,
) {
    let rect = view.rect;
    let painter = ui.painter().with_clip_rect(rect);
    let colors = current_colors(ui.ctx());
    let mut lines = Lines::new(rect, ui.ctx().pixels_per_point());
    lines.reserve(selection.len() * EDGES.len());
    lines.set_smooth(selection.len() <= SMOOTH_UP_TO);
    for &id in selection.ids() {
        outline(
            &mut lines,
            view,
            dm,
            parts,
            id,
            Stroke::new(OUTLINE, colors.accent),
            settings.light_lines_on_top,
        );
    }
    if !lines.is_empty() {
        painter.add(lines.into_shape());
    }
    if settings.show_light_icons {
        let lights = light_icons::light_icons(dm, parts, view);
        light_icons::paint(&painter, &lights, selection.ids(), None, &colors);
    }
    if settings.show_axes {
        crate::viewport::axes::paint(&painter, rect, view.rot);
    }
}

/// The view of the running game's camera (`Workspace.CurrentCamera`: its `CFrame` and `FieldOfView`) drawing into
/// `rect`, or `None` when the game has no camera.
pub(crate) fn game_view(dm: &DataModel, rect: Rect) -> Option<View> {
    let workspace = dm.service(dm.registry().class_id("Workspace")?)?;
    let camera = match dm.get_by_name(workspace, "CurrentCamera").ok()? {
        metallic_engine::metallic_core::Value::Ref(Some(camera)) => camera,
        _ => return None,
    };
    let cframe = match dm.get_by_name(camera, "CFrame").ok()? {
        metallic_engine::metallic_core::Value::CFrame(cframe) => cframe,
        _ => return None,
    };
    let fov = match dm.get_by_name(camera, "FieldOfView").ok()? {
        metallic_engine::metallic_core::Value::Number(degrees) => (degrees as f32).to_radians(),
        _ => return None,
    };
    Some(View {
        pos: cframe.position(),
        rot: cframe.rotation(),
        fov,
        rect,
    })
}
