//! The browser's right side: the open folder's folders and files (or the search's results) as tiles that wrap with
//! the panel's width (#676). A tile is its icon and its name; it is selected by a click, opened by a double-click,
//! and dragged onto a folder to move it there.

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, CornerRadius, FontId, Id, Order, Rect, Role, ScrollArea, Sense, Stroke, TextFormat, Ui, WidgetInfo,
};
use egui::{pos2, vec2};

use super::{
    Act, AssetDrag, AssetKind, Keys, MISSING, MenuLine, context_menu, drop_target, dropped_on, file_size, rename_field,
};
use crate::editor::EditorState;
use crate::editor::assets::{Entry, EntryKind};
use crate::theme::current_colors;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, size, space, text};

/// A tile's width at scale 1.
const TILE_WIDTH: f32 = 84.0;
/// The space between tiles.
const GAP: f32 = space::S2;
/// A tile's icon at scale 1.
const ICON: f32 = 30.0;
/// The height of a tile's icon area, above its name, at scale 1.
const ICON_AREA: f32 = 44.0;
/// The height of a tile's name: two lines of small text, the same at every scale.
const NAME_AREA: f32 = 38.0;

/// A tile's measures at the browser's scale (#696, Ctrl and the wheel): its picture grows and shrinks, its name keeps
/// its size and gets the tile's width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Measures {
    /// The tile.
    pub tile: egui::Vec2,
    /// Its icon's size.
    pub icon: f32,
    /// Its picture's square, above its name.
    pub icon_area: f32,
}

impl Measures {
    /// The measures at `scale` ([`crate::editor::assets::AssetBrowser::scale`]).
    pub(crate) fn at(scale: f32) -> Self {
        let icon_area = (ICON_AREA * scale).round();
        Self {
            tile: vec2((TILE_WIDTH * scale).round().max(64.0), icon_area + NAME_AREA),
            icon: (ICON * scale).round(),
            icon_area,
        }
    }
}
/// The room between the tiles and the area's edges (#700), so none sits flat against them.
const PAD: f32 = space::S3;

/// The empty space behind the tiles or the rows (#676): a click there clears the selection, a right-click offers a new
/// folder or file, and what is dropped there goes into the open folder.
pub(super) struct Ground {
    response: egui::Response,
    folder: String,
    searching: bool,
}

impl Ground {
    /// The empty space of `area`, labelled "Assets" with `role` for screen readers (a grid, or a list, #699).
    pub(super) fn of(ui: &mut Ui, editor: &EditorState, area: Rect, role: Role, acts: &mut Vec<Act>) -> Self {
        let response = ui.interact(area, Id::unique("metallic.assets.ground"), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(role, true, "Assets"));
        let folder = editor.browser.open.clone();
        let searching = !editor.browser.query.trim().is_empty();
        let ground_menu = [
            super::new_submenu(!searching),
            MenuLine::Item(super::IMPORT_ITEM, None, !searching),
        ];
        match context_menu(&response, &ground_menu) {
            Some(super::IMPORT_ITEM) => acts.push(Act::Import(folder.clone())),
            Some(label) => {
                if let Some(new) = super::new_item(label) {
                    acts.push(Act::New(folder.clone(), new));
                }
            }
            None => {}
        }
        if response.clicked() && !editor.browser.selected.is_empty() {
            acts.push(Act::Click {
                path: String::new(),
                add: false,
                range: false,
            });
        }
        Self {
            response,
            folder,
            searching,
        }
    }

    /// What was dropped on the empty space goes into the open folder (not while a search shows files from anywhere).
    /// Taken after the tiles or rows, so a folder's under the pointer takes a drop first.
    pub(super) fn drop(&self, acts: &mut Vec<Act>) {
        if let Some(paths) = dropped_on(&self.response).filter(|_| !self.searching) {
            acts.push(Act::Move(paths, self.folder.clone()));
        }
    }

    /// When nothing is shown, says so (and why) and takes what is dropped. Returns whether it did.
    pub(super) fn empty(&self, ui: &mut Ui, editor: &EditorState, entries: &[Entry], acts: &mut Vec<Act>) -> bool {
        if !entries.is_empty() {
            return false;
        }
        let line = if self.searching {
            format!("Nothing is called \u{201C}{}\u{201D}.", editor.browser.query.trim())
        } else {
            "This folder is empty. Right-click to add a folder or a script.".to_owned()
        };
        crate::panels::empty_note(ui, &line);
        self.drop(acts);
        true
    }
}

/// Ctrl and the wheel over `area` scale the tiles or rows (#696); the wheel alone scrolls.
pub(super) fn zoom(ui: &Ui, editor: &mut EditorState, area: Rect) {
    if ui.rect_contains_pointer(area) {
        let zoom = ui.input(|input| input.zoom_delta());
        if zoom != 1.0 {
            editor.browser.zoom(zoom);
        }
    }
}

/// Draws the tiles.
pub(super) fn show(ui: &mut Ui, editor: &mut EditorState, keys: &Keys, acts: &mut Vec<Act>) {
    // Pictures decoded this frame: a few a frame, so a folder of large pictures does not stall one (#681).
    let mut decodes = 0;
    let entries = super::shown_entries(editor);
    let area = ui.available_rect_before_wrap();
    let ground = Ground::of(ui, editor, area, Role::Grid, acts);
    if ground.empty(ui, editor, &entries, acts) {
        return;
    }
    zoom(ui, editor, area);
    let measures = Measures::at(editor.browser.scale);
    let tile_size = measures.tile;
    let columns = (((area.width() - 2.0 * PAD + GAP) / (tile_size.x + GAP)).floor() as usize).max(1);
    let lines = entries.len().div_ceil(columns);
    ScrollArea::vertical()
        .id_salt("assets.tiles")
        .auto_shrink([false, false])
        .content_margin(egui::Margin::same(PAD as i8))
        .show_rows(ui, tile_size.y + GAP, lines, |ui, range| {
            for line in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
                    let start = line * columns;
                    for entry in &entries[start..(start + columns).min(entries.len())] {
                        tile(ui, editor, entry, measures, keys, acts, &mut decodes);
                    }
                });
                ui.add_space(GAP);
            }
        });
    ground.drop(acts);
    drag_chip(ui);
}

/// One tile: its icon on a fill when selected or under the pointer, its name under it (two lines at most).
fn tile(
    ui: &mut Ui,
    editor: &mut EditorState,
    entry: &Entry,
    measures: Measures,
    keys: &Keys,
    acts: &mut Vec<Act>,
    decodes: &mut usize,
) {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let selected = editor.browser.selected.contains(&entry.path);
    let renaming = editor.browser.renaming.as_deref() == Some(entry.path.as_str());
    let Measures {
        tile: tile_size,
        icon: icon_size,
        icon_area,
    } = measures;
    let (rect, response) = ui.allocate_exact_size(tile_size, Sense::click_and_drag());
    let name = entry.name().to_owned();
    response.widget_info(|| WidgetInfo::selected(Role::GridCell, true, selected, &name));
    let folder = entry.is_folder();
    let dropping = folder && drop_target(&response, &entry.path);
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            Some(p.surface_hover)
        } else if response.hovered() {
            Some(p.surface)
        } else {
            None
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, CornerRadius::same(radius::TILE), fill);
        }
        if dropping {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(radius::TILE),
                Stroke::new(2.0, colors.accent),
                egui::StrokeKind::Inside,
            );
        }
        let (icon, ink) = icon_of(entry, &colors);
        let centre = pos2(rect.center().x, rect.top() + space::S2 + icon_area / 2.0);
        let area = Rect::from_center_size(centre, vec2(icon_area, icon_area));
        // A picture, a material or a scene shows itself (#681); until its thumbnail is here, its kind's icon.
        if !thumbnail(ui, editor, entry, area, decodes) {
            icons::paint_at(ui.painter(), centre, Align2::CENTER_CENTER, icon, icon_size, ink);
        }
        let label = Rect::from_min_max(
            pos2(rect.left() + space::S1, rect.top() + space::S2 + icon_area),
            pos2(rect.right() - space::S1, rect.bottom() - space::S1),
        );
        if renaming {
            let field = Rect::from_min_size(label.min, vec2(label.width(), 20.0));
            let typed_name = if folder { entry.file_name() } else { entry.name() };
            if let Some(act) = rename_field(ui, field, &entry.path, typed_name, true) {
                acts.push(act);
            }
        } else {
            let ink = match entry.kind {
                EntryKind::Missing(_) => p.subtle,
                _ if selected => p.text,
                _ => p.muted,
            };
            let mut job =
                LayoutJob::single_section(name.clone(), TextFormat::simple(FontId::proportional(text::SMALL), ink));
            job.wrap = TextWrapping {
                max_width: label.width(),
                max_rows: 2,
                break_anywhere: true,
                overflow_character: Some('\u{2026}'),
            };
            job.halign = egui::Align::Center;
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(pos2(label.center().x, label.top()), galley, ink);
        }
    }
    behave(ui, editor, entry, rect, response, keys, acts);
}

/// What a tile or a list row does (#676, #699), drawn in `rect` with `response`: the button on its corner (#703,
/// #704), its tooltip, taking a drop (a folder's), being dragged, its right-click menu, and clicks.
pub(super) fn behave(
    ui: &mut Ui,
    editor: &mut EditorState,
    entry: &Entry,
    rect: Rect,
    response: egui::Response,
    keys: &Keys,
    acts: &mut Vec<Act>,
) {
    let selected = editor.browser.selected.contains(&entry.path);
    let renaming = editor.browser.renaming.as_deref() == Some(entry.path.as_str());
    let folder = entry.is_folder();
    let name = entry.name().to_owned();
    // A material's has a button at its top right while the pointer is on it, which gives the material to the
    // selected parts (#703); a script file's, one that adds a script running it to the selected objects (#704).
    let corner = match entry.kind {
        EntryKind::File(AssetKind::Material) => Some((
            Icon::UiApplyMaterial,
            format!("Apply {name} to the selected parts"),
            editor.can_apply_material(),
            Act::ApplyMaterial(entry.path.clone()),
        )),
        EntryKind::File(AssetKind::Script(_)) => Some((
            Icon::UiAdd,
            format!("Add {name} to the selected objects"),
            editor.can_add_script(),
            Act::AddScript(entry.path.clone()),
        )),
        _ => None,
    };
    let scale = editor.browser.scale;
    if let Some((icon, label, enabled, act)) = corner
        && corner_button(ui, rect, response.id.with("corner"), icon, &label, enabled, scale)
    {
        acts.push(act);
    }
    let response = response.on_hover_text(hover_text(editor, entry));
    if let Some(paths) = dropped_on(&response).filter(|_| folder) {
        acts.push(Act::Move(paths, entry.path.clone()));
    }
    // Dragging the tile drags the selection, or the tile alone when it is not selected.
    let dragged = if selected {
        editor.browser.selected.iter().cloned().collect()
    } else {
        vec![entry.path.clone()]
    };
    if !renaming && !matches!(entry.kind, EntryKind::Missing(_)) {
        response.dnd_set_drag_payload(AssetDrag { paths: dragged });
    }
    let present = !matches!(entry.kind, EntryKind::Missing(_));
    let mut menu = vec![
        MenuLine::Item("Open", None, folder),
        MenuLine::Item("Copy path", None, true),
        MenuLine::Separator,
        MenuLine::Item("Rename", keys.rename.clone(), present),
        MenuLine::Item("Duplicate", keys.duplicate.clone(), present),
        MenuLine::Item("Delete", keys.delete.clone(), present),
    ];
    // A folder's takes imports too (#707).
    if folder {
        menu.insert(1, MenuLine::Item(super::IMPORT_ITEM, None, true));
    }
    let targets: Vec<String> = if selected {
        editor.browser.selected.iter().cloned().collect()
    } else {
        vec![entry.path.clone()]
    };
    match context_menu(&response, &menu) {
        Some("Open") => acts.push(Act::Activate(entry.clone())),
        Some(super::IMPORT_ITEM) => acts.push(Act::Import(entry.path.clone())),
        Some("Rename") => acts.push(Act::StartRename(entry.path.clone())),
        Some("Duplicate") => acts.push(Act::Duplicate(targets)),
        Some("Delete") => acts.push(Act::Delete(targets)),
        Some("Copy path") => acts.push(Act::CopyPaths(targets)),
        _ => {
            let modifiers = ui.input(|input| input.modifiers);
            if response.double_clicked() && !renaming {
                acts.push(Act::Activate(entry.clone()));
            } else if response.clicked() || (response.secondary_clicked() && !selected) {
                acts.push(Act::Click {
                    path: entry.path.clone(),
                    add: modifiers.command && response.clicked(),
                    range: modifiers.shift && response.clicked(),
                });
            }
        }
    }
}

/// `entry`'s icon and its colour: its kind's, a folder's, or a warning for a missing file.
pub(super) fn icon_of(entry: &Entry, colors: &crate::theme::Colors) -> (Icon, egui::Color32) {
    match entry.kind {
        EntryKind::Folder => (Icon::AssetFolder, Icon::AssetFolder.color(colors)),
        EntryKind::File(kind) => (kind.icon(), kind.icon().color(colors)),
        EntryKind::Missing(_) => (Icon::UiWarning, colors.accent),
    }
}

/// The side of the button on a tile's corner (#703), at scale 1.
const CORNER: f32 = 20.0;
/// The smallest and the largest the button gets, however the tiles are scaled (#734).
const CORNER_SIDES: (f32, f32) = (14.0, 40.0);

/// The side of the corner button of `rect`, a tile or a row, at the browser's `scale` (#734): it grows and shrinks
/// with the tiles, within [`CORNER_SIDES`], and never outgrows a row.
fn corner_side(rect: Rect, scale: f32) -> f32 {
    (CORNER * scale)
        .round()
        .clamp(CORNER_SIDES.0, CORNER_SIDES.1)
        .min(rect.height() - 2.0)
}

/// A small button at the top right of the tile `rect` while the pointer is over the tile (#703): `icon` on a raised
/// fill, `label` for screen readers and in its tooltip; greyed out when not `enabled`; at the browser's `scale`.
/// Returns whether it was clicked. Made after the tile, so it takes the click rather than the tile.
fn corner_button(ui: &mut Ui, rect: Rect, id: Id, icon: Icon, label: &str, enabled: bool, scale: f32) -> bool {
    if !ui.rect_contains_pointer(rect) {
        return false;
    }
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let side = corner_side(rect, scale);
    // At the top of a tile; in the middle of a row too short for it to sit lower (#699).
    let top = if rect.height() < 2.0 * side {
        rect.center().y - side / 2.0
    } else {
        rect.top() + space::S1
    };
    let button = Rect::from_min_size(pos2(rect.right() - space::S1 - side, top), vec2(side, side));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(button, id, sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    let fill = if enabled && response.hovered() {
        p.surface_hover
    } else {
        p.surface
    };
    ui.painter().rect(
        button,
        CornerRadius::same(radius::TILE),
        fill,
        Stroke::new(size::BORDER, p.border),
        egui::StrokeKind::Inside,
    );
    let ink = match (enabled, response.hovered()) {
        (false, _) => p.text.gamma_multiply(0.48),
        (true, true) => colors.accent,
        (true, false) => p.text,
    };
    icons::paint_at(
        ui.painter(),
        button.center(),
        Align2::CENTER_CENTER,
        icon,
        (side * text::SMALL / CORNER).round(),
        ink,
    );
    response.on_hover_text(label).clicked()
}

/// What a tile's tooltip says: its path, and its kind and size (or that it is missing).
pub(super) fn hover_text(editor: &EditorState, entry: &Entry) -> String {
    match entry.kind {
        EntryKind::Folder => format!("{}\nFolder", entry.path),
        EntryKind::Missing(kind) => format!("{}\n{} \u{2022} {MISSING}", entry.path, kind.label()),
        EntryKind::File(kind) => {
            let size = editor
                .project
                .fingerprint(&entry.path)
                .map(|fingerprint| format!(" \u{2022} {}", file_size(fingerprint.size)))
                .unwrap_or_default();
            format!("{}\n{}{size}", entry.path, kind.label())
        }
    }
}

/// While tiles are dragged, a chip beside the pointer saying what is dragged.
pub(super) fn drag_chip(ui: &Ui) {
    let Some(drag) = egui::DragAndDrop::payload::<AssetDrag>(ui.ctx()) else {
        return;
    };
    let Some(pointer) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let label = match drag.paths.as_slice() {
        [one] => crate::editor::assets::file_name(one).to_owned(),
        many => format!("{} items", many.len()),
    };
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(Order::Tooltip, Id::unique("metallic.assets.drag")));
    let galley = painter.layout_no_wrap(label, FontId::proportional(text::SMALL), p.text);
    let rect = Rect::from_min_size(
        pointer + vec2(space::S6, space::S4),
        vec2(galley.size().x + 2.0 * space::S3, galley.size().y + 2.0 * space::S2),
    );
    painter.rect(
        rect,
        CornerRadius::same(radius::TILE),
        p.surface,
        Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(space::S3, space::S2), galley, p.text);
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
}

/// The share of the ball's outline its disc is cut a little inside, so no sky shows at its rim.
const BALL_INSET: f32 = 0.98;
/// The sides of the ball's disc.
const BALL_SIDES: usize = 64;

/// Paints the material picture `texture` filling `area` as its ball alone (#697): a disc of the picture inside the
/// ball's outline ([`crate::app::thumbnails::ball_radius`]), its rim faded out over a pixel so it is smooth.
pub(crate) fn paint_ball(painter: &egui::Painter, texture: egui::TextureId, area: Rect) {
    use egui::epaint::{Mesh, Vertex};
    let share = crate::app::thumbnails::ball_radius() * BALL_INSET;
    let radius = share * area.width();
    let half_pixel = 0.5 / painter.ctx().pixels_per_point();
    let centre = area.center();
    let middle = pos2(0.5, 0.5);
    let vertex = |distance: f32, angle: f32, color: egui::Color32| {
        let towards = egui::Vec2::angled(angle);
        Vertex {
            pos: centre + distance * towards,
            uv: middle + (distance / area.width()) * towards,
            color,
        }
    };
    let mut mesh = Mesh::with_texture(texture);
    mesh.vertices.push(vertex(0.0, 0.0, egui::Color32::WHITE));
    for side in 0..BALL_SIDES {
        let angle = std::f32::consts::TAU * side as f32 / BALL_SIDES as f32;
        mesh.vertices
            .push(vertex(radius - half_pixel, angle, egui::Color32::WHITE));
        mesh.vertices
            .push(vertex(radius + half_pixel, angle, egui::Color32::TRANSPARENT));
    }
    for side in 0..BALL_SIDES {
        let next = (side + 1) % BALL_SIDES;
        let (inner, outer) = (1 + 2 * side as u32, 2 + 2 * side as u32);
        let (next_inner, next_outer) = (1 + 2 * next as u32, 2 + 2 * next as u32);
        mesh.add_triangle(0, inner, next_inner);
        mesh.add_triangle(inner, outer, next_inner);
        mesh.add_triangle(next_inner, outer, next_outer);
    }
    painter.add(mesh);
}

/// How many pictures a frame decodes for their tiles.
const DECODES_A_FRAME: usize = 2;

/// Draws `entry`'s thumbnail in `area` when it has one (#681): a picture at its aspect ratio, a material on a sphere,
/// a scene from its camera. Asks for what is missing or old. Returns whether it drew one.
pub(super) fn thumbnail(ui: &Ui, editor: &mut EditorState, entry: &Entry, area: Rect, decodes: &mut usize) -> bool {
    use crate::editor::pictures::{Picture, Wanted};
    let EntryKind::File(kind) = entry.kind else {
        return false;
    };
    let Some(id) = editor.project.assets().id(&entry.path) else {
        return false;
    };
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match kind {
        AssetKind::Image => {
            if !editor.thumbnails.contains_key(&id) {
                if *decodes >= DECODES_A_FRAME {
                    ui.ctx().request_repaint();
                    return false;
                }
                *decodes += 1;
            }
            let Some(texture) = crate::panels::material::thumbnail(ui.ctx(), editor, id) else {
                return false;
            };
            let [width, height] = texture.size().map(|side| side.max(1) as f32);
            let scale = (area.width() / width).min(area.height() / height);
            let shown = Rect::from_center_size(area.center(), vec2(width * scale, height * scale));
            ui.painter().image(texture.id(), shown, full, egui::Color32::WHITE);
            true
        }
        AssetKind::Material | AssetKind::Scene => {
            let Ok(fingerprint) = editor.project.fingerprint(&entry.path) else {
                return false;
            };
            let drawn = editor.pictures.get(id, fingerprint);
            if drawn.is_none_or(|(_, fresh)| !fresh) {
                editor.pictures.want(Wanted {
                    id,
                    picture: if kind == AssetKind::Material {
                        Picture::Material
                    } else {
                        Picture::Scene
                    },
                    path: entry.path.clone(),
                    fingerprint,
                });
                ui.ctx().request_repaint();
            }
            let Some((texture, _)) = drawn else {
                return false;
            };
            if kind == AssetKind::Material {
                // The ball alone, no sky round it (#697).
                paint_ball(ui.painter(), texture, area);
            } else {
                ui.painter().image(texture, area, full, egui::Color32::WHITE);
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #697: a material's ball is a disc inside the ball's outline: nothing opaque outside it, and its picture read from
    /// the middle of the thumbnail outwards as far as the ball reaches.
    #[test]
    fn the_corner_button_scales_within_limits() {
        let tile = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0));
        assert_eq!(corner_side(tile, 1.0), CORNER);
        assert_eq!(corner_side(tile, 1.5), 30.0);
        assert_eq!(corner_side(tile, 0.1), CORNER_SIDES.0);
        assert_eq!(corner_side(tile, 9.0), CORNER_SIDES.1);
        // A row keeps it inside itself.
        let row = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 20.0));
        assert_eq!(corner_side(row, 2.0), 18.0);
    }

    #[test]
    fn a_ball_is_painted_as_a_disc_inside_its_outline() {
        let ctx = egui::Context::default();
        let area = Rect::from_min_size(pos2(10.0, 20.0), vec2(44.0, 44.0));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_ball(ui.painter(), egui::TextureId::User(7), area);
        });
        output.textures_delta.clear();
        let mesh = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Mesh(mesh) => Some(mesh.clone()),
                _ => None,
            })
            .expect("a mesh");
        assert_eq!(mesh.texture_id, egui::TextureId::User(7));
        let reach = crate::app::thumbnails::ball_radius();
        assert!((0.35..0.5).contains(&reach), "{reach}");
        let outline = reach * area.width();
        for vertex in &mesh.vertices {
            let distance = vertex.pos.distance(area.center());
            if vertex.color.a() > 0 {
                assert!(
                    distance <= outline,
                    "an opaque point outside the ball: {distance} > {outline}"
                );
            }
            assert!(vertex.uv.distance(pos2(0.5, 0.5)) <= reach + 0.01, "{:?}", vertex.uv);
        }
        assert!(
            mesh.vertices.iter().any(|vertex| vertex.color.a() == 0),
            "the rim fades out"
        );
    }
}
