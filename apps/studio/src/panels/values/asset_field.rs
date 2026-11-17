//! A field naming a project asset, as Godot's resource slots have it: a text field and, at its right, a button that
//! opens a window to pick one. It is a part's Material in the Properties panel (#739), and each texture map of the
//! material editor (#762), which shows a small preview of the picture at its left. The material's is described here;
//! a picture's is the same, its name the file's without the extension, and a name typed is the picture of that name.
//!
//! - **The field** shows the material's name, without `.material`. Focused, it shows the material file's whole path,
//!   which can be typed or pasted; Enter (or leaving it) takes what it names: a material file's path gives that
//!   asset's id, a material's name (`Concrete`) is kept as the name, and nothing is no material. Anything else leaves
//!   the value as it was, as Escape does.
//! - **Completions** (#760): once something is typed, a list under the field offers the materials whose name or path
//!   holds it, those whose name starts with it first. Up and Down pick one out, Enter takes the one picked out (or
//!   what is typed, when it names a material, else the first), Tab takes the one picked out or the first, and a click
//!   takes the one clicked.
//! - **The window** lists the project's materials, searched by name or path, under None: a click picks one out, a
//!   double-click or Select chooses it, Cancel or Escape closes. A button beside the search shows them as a grid of
//!   their balls instead, and back (#761); the next window opens the way the last was left.
//! - A material dragged from the Assets panel and let go on the field is chosen, as on any file picker (#676).

use egui::{
    Align2, CornerRadius, FontId, Id, Key, Modal, Modifiers, Rect, Role, Sense, Stroke, TextEdit, Ui, WidgetInfo, pos2,
    vec2,
};
use metallic_engine::metallic_core::Widget;
use metallic_engine::metallic_project::material::{BUILTIN_NAMES, material_name};

use super::{Files, NO_FILE, field_height, labelled, paint_thumbnail, want_picture};
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{radius, shadow, size, space, text};
use crate::theme::{ButtonKind, button, current_colors, fonts};

/// The window's list: how many rows it shows before it scrolls.
const LIST_ROWS: f32 = 12.0;
/// The window's width.
const WINDOW_WIDTH: f32 = 460.0;
/// The most completions offered at once (#760).
const COMPLETIONS: usize = 8;
/// The narrowest the completions' list is, however narrow the field.
const COMPLETIONS_WIDTH: f32 = 240.0;
/// The side of a tile of the window's grid (#761), its name below it.
const TILE: f32 = 84.0;

/// What a field names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A material: its value is the file's asset id or a material's name (#739).
    Material,
    /// A picture (#762): its value is the file's asset id.
    Picture,
}

impl Kind {
    /// The files it takes.
    fn widget(self) -> Widget {
        match self {
            Kind::Material => Widget::AssetId {
                extensions: &["material"],
            },
            Kind::Picture => Widget::AssetId {
                extensions: &["png", "jpg", "jpeg"],
            },
        }
    }

    /// The name the file at `path` is shown by: its file name without the extension.
    fn name(self, path: &str) -> &str {
        match self {
            Kind::Material => material_name(path),
            Kind::Picture => {
                let file = path.rsplit_once('/').map_or(path, |(_, file)| file);
                file.rsplit_once('.').map_or(file, |(stem, _)| stem)
            }
        }
    }

    /// `path` without its extension.
    fn stem(self, path: &str) -> &str {
        path.rsplit_once('.')
            .filter(|(_, extension)| !extension.contains('/'))
            .map_or(path, |(stem, _)| stem)
    }

    /// The window's title, and its tooltip on the field's button.
    fn title(self) -> &'static str {
        match self {
            Kind::Material => "Choose a material",
            Kind::Picture => "Choose a picture",
        }
    }

    /// The window's search's hint.
    fn search(self) -> &'static str {
        match self {
            Kind::Material => "Search the materials",
            Kind::Picture => "Search the pictures",
        }
    }

    /// A file's icon, until its thumbnail is drawn.
    fn icon(self) -> Icon {
        match self {
            Kind::Material => Icon::AssetMaterial,
            Kind::Picture => Icon::AssetImage,
        }
    }
}

/// Where the window keeps whether it shows a grid (#761), for the next window of `kind`.
fn grid_view(kind: Kind) -> Id {
    Id::unique(("metallic.asset_field.grid", kind == Kind::Picture))
}

/// One file the field or the window can name: its path and its asset id.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Choice {
    path: String,
    id: String,
}

/// The project's files of `kind`, by path.
fn choices(files: &Files, kind: Kind) -> Vec<Choice> {
    files
        .members
        .iter()
        .filter(|(path, _)| kind.widget().takes_file(path))
        .filter_map(|(path, id)| {
            Some(Choice {
                path: path.clone(),
                id: id.clone()?,
            })
        })
        .collect()
}

/// What the field shows for the value `current`: the file's name, the name a value gives, or nothing.
fn short(kind: Kind, choices: &[Choice], current: &str) -> String {
    match choices.iter().find(|choice| choice.id == current) {
        Some(choice) => kind.name(&choice.path).to_owned(),
        None => current.to_owned(),
    }
}

/// What the focused field shows for `current`: the file's path, the path of the project's file of the name a value
/// gives, or that name when the project has none.
fn long(kind: Kind, choices: &[Choice], current: &str) -> String {
    choices
        .iter()
        .find(|choice| choice.id == current)
        .or_else(|| choices.iter().find(|choice| kind.name(&choice.path) == current))
        .map_or_else(|| current.to_owned(), |choice| choice.path.clone())
}

/// The value `typed` names: a file's id (by its path, with its extension or not), or `""` for nothing; for a material
/// a material's name, kept as the name, and for a picture the id of the one picture of that name. `None` when it
/// names nothing.
fn value_of(kind: Kind, choices: &[Choice], typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Some(String::new());
    }
    let path = |choice: &&Choice| choice.path == typed || kind.stem(&choice.path) == typed;
    if let Some(choice) = choices.iter().find(path) {
        return Some(choice.id.clone());
    }
    let mut named = choices.iter().filter(|choice| kind.name(&choice.path) == typed);
    match kind {
        Kind::Material => (named.next().is_some() || BUILTIN_NAMES.contains(&typed)).then(|| typed.to_owned()),
        Kind::Picture => match (named.next(), named.next()) {
            (Some(only), None) => Some(only.id.clone()),
            _ => None,
        },
    }
}

/// The files offered for `typed` (#760): those whose name or path holds it, ignoring case, at most [`COMPLETIONS`].
/// Those whose name starts with it come first, then those whose path does, each by name.
fn completions<'a>(kind: Kind, choices: &'a [Choice], typed: &str) -> Vec<&'a Choice> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(u8, String, &Choice)> = choices
        .iter()
        .filter_map(|choice| {
            let name = kind.name(&choice.path).to_lowercase();
            let path = choice.path.to_lowercase();
            let rank = if name.starts_with(&typed) {
                0
            } else if path.starts_with(&typed) {
                1
            } else if name.contains(&typed) || path.contains(&typed) {
                2
            } else {
                return None;
            };
            Some((rank, name, choice))
        })
        .collect();
    found.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    found
        .into_iter()
        .take(COMPLETIONS)
        .map(|(_, _, choice)| choice)
        .collect()
}

/// What is typed in the focused field: the text, whether it was changed since the field was focused (completions
/// show only then), and the completion picked out with Up and Down.
#[derive(Clone, Debug, Default)]
struct Typing {
    text: String,
    edited: bool,
    picked: Option<usize>,
}

/// The window's state while it is open.
#[derive(Clone, Debug, Default)]
struct Window {
    /// The search.
    query: String,
    /// The row picked out: a material's id, or `""` for None.
    picked: Option<String>,
}

/// The field of `kind` labelled `label`, whose value is `current` (an asset id, a material's name, or nothing),
/// `width` wide; a picture's with its preview at its left. Returns the value chosen: an asset id, a material's name,
/// or `""` for none.
pub(super) fn asset_field(
    ui: &mut Ui,
    files: &Files,
    kind: Kind,
    label: &str,
    current: &str,
    width: f32,
    mixed: bool,
) -> Option<String> {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let choices = choices(files, kind);
    let id = ui.make_persistent_id(("metallic.properties.asset", label));
    let field_id = id.with("field");
    let window_id = id.with("window");
    let focused = ui.memory(|memory| memory.has_focus(field_id));
    let shown = if mixed {
        String::new()
    } else {
        short(kind, &choices, current)
    };
    // Focused, the field holds what is typed, from the whole path on.
    let mut typing = if focused {
        ui.data_mut(|data| data.get_temp::<Typing>(id))
            .unwrap_or_else(|| Typing {
                text: if mixed {
                    String::new()
                } else {
                    long(kind, &choices, current)
                },
                ..Typing::default()
            })
    } else {
        Typing {
            text: shown.clone(),
            ..Typing::default()
        }
    };
    // The completions shown since the last frame, and Up and Down moving through them before the field sees them.
    let offered = if focused && typing.edited {
        completions(kind, &choices, &typing.text)
    } else {
        Vec::new()
    };
    if !offered.is_empty() {
        let last = offered.len() - 1;
        if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            typing.picked = Some(typing.picked.map_or(0, |picked| (picked + 1).min(last)));
        }
        if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            typing.picked = typing.picked.and_then(|picked| picked.checked_sub(1));
        }
    }
    let mut typed = typing.text.clone();
    let height = field_height(ui);
    let button_side = height;
    // A picture's preview, at the field's left (#762).
    let preview = kind == Kind::Picture;
    let preview_room = if preview { height + space::S2 } else { 0.0 };
    let field_width = (width - button_side - space::S1 - preview_room).max(0.0);
    let mut chosen = None;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S1;
        if preview {
            let (rect, _) = ui.allocate_exact_size(vec2(height, height), Sense::hover());
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::TILE), p.surface);
            if let Some(choice) = choices.iter().find(|choice| choice.id == current) {
                want_picture(ui.ctx(), &choice.path);
                if let Some(thumbnail) = files.picture(&choice.path) {
                    paint_thumbnail(ui.painter(), thumbnail, rect);
                }
            }
            ui.add_space(space::S2 - space::S1);
        }
        let hint = if mixed { "Mixed" } else { NO_FILE };
        let response = ui.add_sized(
            [field_width, height],
            TextEdit::singleline(&mut typed)
                .id(field_id)
                .hint_text(hint)
                .frame(egui::Frame::NONE)
                .margin(vec2(0.0, 2.0))
                .text_color(p.text),
        );
        labelled(&response, Role::TextInput, label);
        if response.changed() {
            typing.edited = true;
            typing.picked = None;
        }
        if let Some(choice) = choices.iter().find(|choice| choice.id == current) {
            response.clone().on_hover_text(choice.path.as_str());
        }
        if response.has_focus() && !focused {
            // Just focused: the whole path, all of it selected, from the next frame on.
            let path = if mixed {
                String::new()
            } else {
                long(kind, &choices, current)
            };
            if let Some(mut state) = TextEdit::load_state(ui.ctx(), field_id) {
                let all = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(path.chars().count()),
                );
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), field_id);
            }
            ui.data_mut(|data| {
                data.insert_temp(
                    id,
                    Typing {
                        text: path,
                        ..Typing::default()
                    },
                )
            });
            ui.ctx().request_repaint();
        } else if response.has_focus() {
            typing.text = typed.clone();
            ui.data_mut(|data| data.insert_temp(id, typing.clone()));
        } else if focused {
            ui.data_mut(|data| data.remove::<Typing>(id));
            let (cancelled, entered, tabbed) = ui.input(|input| {
                (
                    input.key_pressed(Key::Escape),
                    input.key_pressed(Key::Enter),
                    input.key_pressed(Key::Tab),
                )
            });
            // Enter takes the completion picked out, else what is typed when it names one, else the first; Tab the
            // one picked out or the first.
            let completed = match (typing.picked, value_of(kind, &choices, &typed)) {
                (Some(picked), _) if entered || tabbed => offered.get(picked),
                (None, None) if entered => offered.first(),
                (None, _) if tabbed => offered.first(),
                _ => None,
            };
            let value = match completed {
                Some(choice) => Some(choice.id.clone()),
                None => value_of(kind, &choices, &typed),
            };
            if !cancelled
                && let Some(value) = value
                && (value != current || mixed)
            {
                chosen = Some(value);
            }
        }
        // The button that opens the window.
        let (rect, open) = ui.allocate_exact_size(vec2(button_side, height), Sense::click());
        open.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Choose {label}")));
        if open.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::TILE), p.surface_hover);
        }
        let ink = if open.hovered() { p.text } else { p.muted };
        icons::paint_at(
            ui.painter(),
            rect.center(),
            Align2::CENTER_CENTER,
            Icon::AssetFolder,
            text::SMALL,
            ink,
        );
        if open.on_hover_text(kind.title()).clicked() {
            let picked = (!mixed).then(|| current.to_owned());
            ui.data_mut(|data| {
                data.insert_temp(
                    window_id,
                    Window {
                        query: String::new(),
                        picked,
                    },
                )
            });
        }
        response
    });
    // The completions, under the field, while it is focused (a click on one makes it let go of the focus this frame,
    // and the click is the completion's).
    if focused && !offered.is_empty() {
        let field = row.inner.rect;
        if let Some(clicked) = completion_list(ui, id.with("completions"), kind, field, &offered, typing.picked)
            && let Some(choice) = offered.get(clicked)
        {
            chosen = Some(choice.id.clone()).filter(|value| value != current || mixed);
        }
    }
    // A file dragged from the Assets panel and let go on the field (#676).
    let response = &row.inner;
    let fits = |drag: &crate::panels::assets::AssetDrag| match drag.paths.as_slice() {
        [path] => choices
            .iter()
            .find(|choice| &choice.path == path)
            .map(|choice| choice.id.clone()),
        _ => None,
    };
    if response
        .dnd_hover_payload::<crate::panels::assets::AssetDrag>()
        .is_some_and(|drag| fits(&drag).is_some())
    {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::ZERO,
            Stroke::new(2.0, colors.accent),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(drag) = response.dnd_release_payload::<crate::panels::assets::AssetDrag>()
        && let Some(value) = fits(&drag)
    {
        chosen = Some(value);
    }
    if let Some(picked) = window(ui.ctx(), window_id, kind, &choices, files) {
        chosen = Some(picked).filter(|picked| picked != current || mixed);
    }
    chosen
}

/// The window of `kind`, while it is open: returns the value chosen when it closes with one. `files` has the
/// thumbnails drawn.
fn window(ctx: &egui::Context, id: Id, kind: Kind, choices: &[Choice], files: &Files) -> Option<String> {
    let mut state = ctx.data(|data| data.get_temp::<Window>(id))?;
    let p = current_colors(ctx).palette;
    let frame = egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(size::BORDER, p.border))
        .corner_radius(CornerRadius::same(radius::DIALOG))
        .shadow(shadow::MENU)
        .inner_margin(space::S6);
    let (mut choose, mut cancel) = (false, false);
    let modal = Modal::new(id.with("modal")).frame(frame).show(ctx, |ui| {
        ui.set_width(WINDOW_WIDTH);
        ui.label(
            egui::RichText::new(kind.title())
                .font(fonts::named(fonts::HEADING, text::TITLE))
                .color(p.text),
        );
        ui.add_space(space::S4);
        let grid = ctx.data(|data| data.get_temp::<bool>(grid_view(kind))).unwrap_or(false);
        let mut switch = false;
        crate::panels::search_box_then(ui, &mut state.query, kind.search(), size::ROW, |ui| {
            let (icon, label) = if grid {
                (Icon::UiListView, "Show as a list")
            } else {
                (Icon::UiGridView, "Show as a grid")
            };
            switch = icon_button(ui, icon, label, size::ROW).clicked();
        });
        if switch {
            ctx.data_mut(|data| data.insert_temp(grid_view(kind), !grid));
        }
        ui.add_space(space::S3);
        let query = state.query.trim().to_lowercase();
        let shown: Vec<&Choice> = choices
            .iter()
            .filter(|choice| query.is_empty() || choice.path.to_lowercase().contains(&query))
            .collect();
        egui::ScrollArea::vertical()
            .id_salt(id.with("list"))
            .max_height(size::ROW * LIST_ROWS)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let none = std::iter::once((NO_FILE, None, String::new()));
                let files_shown = shown
                    .iter()
                    .map(|choice| (kind.name(&choice.path), Some(choice.path.as_str()), choice.id.clone()));
                let items = |ui: &mut Ui| {
                    for (index, (name, path, value)) in none.chain(files_shown).enumerate() {
                        let on = state.picked.as_deref() == Some(value.as_str());
                        let response = if grid {
                            tile(ui, files, kind.icon(), name, path, on)
                        } else {
                            row(ui, Role::Row, kind.icon(), index, name, path, on)
                        };
                        if response.clicked() {
                            state.picked = Some(value.clone());
                        }
                        if response.double_clicked() {
                            state.picked = Some(value);
                            choose = true;
                        }
                    }
                };
                if grid {
                    ui.spacing_mut().item_spacing = vec2(space::S2, space::S2);
                    ui.horizontal_wrapped(items);
                } else {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    items(ui);
                }
            });
        ui.add_space(space::S5);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(state.picked.is_some(), |ui| {
                choose |= button(ui, ButtonKind::Primary, "Select").clicked();
            });
            cancel = button(ui, ButtonKind::Secondary, "Cancel").clicked();
        });
        if ui.input(|input| input.key_pressed(Key::Enter)) && state.picked.is_some() {
            choose = true;
        }
    });
    if modal.should_close() {
        cancel = true;
    }
    if choose || cancel {
        ctx.data_mut(|data| data.remove::<Window>(id));
        ctx.request_repaint();
        return state.picked.filter(|_| choose);
    }
    ctx.data_mut(|data| data.insert_temp(id, state));
    None
}

/// A square button `side` across of `icon`, `label` for screen readers and in its tooltip.
fn icon_button(ui: &mut Ui, icon: Icon, label: &str, side: f32) -> egui::Response {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::TILE), p.surface_hover);
    }
    let ink = if response.hovered() { p.text } else { p.muted };
    icons::paint_at(
        ui.painter(),
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        text::BODY,
        ink,
    );
    response.on_hover_text(label)
}

/// A tile of the window's grid (#761): the file's thumbnail (its `icon` until the thumbnail is drawn) over its name,
/// picked out when `on`. None's has its name alone.
fn tile(ui: &mut Ui, files: &Files, icon: Icon, name: &str, path: Option<&str>, on: bool) -> egui::Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(TILE, TILE + size::ROW), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::GridCell, true, on, name));
    if ui.is_rect_visible(rect) {
        let fill = if on {
            p.surface_hover
        } else if response.hovered() {
            p.surface
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(radius::TILE), fill);
        let picture = Rect::from_min_size(rect.min, vec2(TILE, TILE)).shrink(space::S3);
        if let Some(path) = path {
            want_picture(ui.ctx(), path);
            match files.picture(path) {
                Some(thumbnail) => paint_thumbnail(ui.painter(), thumbnail, picture),
                None => icons::paint(
                    ui.painter(),
                    icon,
                    picture.shrink(picture.width() / 4.0),
                    icon.color(&colors),
                ),
            }
        }
        let ink = if on { p.text } else { p.muted };
        let label = Rect::from_min_max(pos2(rect.left(), rect.bottom() - size::ROW), rect.max);
        ui.painter().with_clip_rect(label.shrink2(vec2(space::S1, 0.0))).text(
            label.center(),
            Align2::CENTER_CENTER,
            name,
            FontId::proportional(text::SMALL),
            ink,
        );
    }
    response
}

/// The completions `offered` for the field at `field`, in a list under it, the `picked`th picked out. Returns the one
/// clicked.
fn completion_list(
    ui: &Ui,
    id: Id,
    kind: Kind,
    field: Rect,
    offered: &[&Choice],
    picked: Option<usize>,
) -> Option<usize> {
    let p = current_colors(ui.ctx()).palette;
    let frame = egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(size::BORDER, p.border))
        .corner_radius(CornerRadius::same(radius::ITEM))
        .shadow(shadow::MENU)
        .inner_margin(space::S1);
    let mut clicked = None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(field.left_bottom() + vec2(0.0, space::S1))
        .show(ui.ctx(), |ui| {
            frame.show(ui, |ui| {
                ui.set_width(field.width().max(COMPLETIONS_WIDTH));
                ui.spacing_mut().item_spacing.y = 0.0;
                for (index, choice) in offered.iter().enumerate() {
                    let name = kind.name(&choice.path);
                    let on = picked == Some(index);
                    let path = Some(choice.path.as_str());
                    if row(ui, Role::ListBoxOption, kind.icon(), index, name, path, on).clicked() {
                        clicked = Some(index);
                    }
                }
            });
        });
    clicked
}

/// One row of a list, the `index`th, for screen readers a `role`: the file's `icon`, its name and its path, picked out
/// when `on`.
fn row(
    ui: &mut Ui,
    role: Role,
    file_icon: Icon,
    index: usize,
    name: &str,
    path: Option<&str>,
    on: bool,
) -> egui::Response {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW), Sense::click());
    response.widget_info(|| WidgetInfo::selected(role, true, on, name));
    if ui.is_rect_visible(rect) {
        let fill = if on {
            p.surface_hover
        } else if response.hovered() {
            p.surface
        } else {
            p.rows().fill(index)
        };
        ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
        let icon = Rect::from_min_size(
            pos2(rect.left() + space::S3, rect.center().y - text::BODY / 2.0),
            vec2(text::BODY, text::BODY),
        );
        if path.is_some() {
            icons::paint(ui.painter(), file_icon, icon, file_icon.color(&colors));
        }
        let name_x = icon.right() + space::S3;
        let ink = if on { p.text } else { p.muted };
        let galley = ui
            .painter()
            .layout_no_wrap(name.to_owned(), FontId::proportional(text::BODY), ink);
        let name_width = galley.size().x;
        ui.painter()
            .galley(pos2(name_x, rect.center().y - galley.size().y / 2.0), galley, ink);
        if let Some(path) = path {
            let clip = Rect::from_min_max(pos2(name_x + name_width, rect.top()), rect.max);
            ui.painter().with_clip_rect(clip).text(
                pos2(name_x + name_width + space::S4, rect.center().y),
                Align2::LEFT_CENTER,
                path,
                FontId::proportional(text::SMALL),
                p.subtle,
            );
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some() -> Vec<Choice> {
        vec![
            Choice {
                path: "Assets/Materials/Concrete.material".into(),
                id: "aa".into(),
            },
            Choice {
                path: "Assets/Rust.material".into(),
                id: "bb".into(),
            },
        ]
    }

    #[test]
    fn the_field_shows_the_name_and_focused_the_path() {
        let choices = some();
        assert_eq!(short(Kind::Material, &choices, "bb"), "Rust");
        assert_eq!(long(Kind::Material, &choices, "bb"), "Assets/Rust.material");
        // A value naming a material shows the name, and focused the project's file of that name.
        assert_eq!(short(Kind::Material, &choices, "Concrete"), "Concrete");
        assert_eq!(
            long(Kind::Material, &choices, "Concrete"),
            "Assets/Materials/Concrete.material"
        );
        assert_eq!(
            long(Kind::Material, &choices, "Neon"),
            "Neon",
            "a built-in the project has no file of"
        );
        assert_eq!(short(Kind::Material, &choices, ""), "");
    }

    #[test]
    fn what_is_typed_names_a_file_a_name_or_nothing() {
        let choices = some();
        assert_eq!(
            value_of(Kind::Material, &choices, "Assets/Rust.material").as_deref(),
            Some("bb")
        );
        assert_eq!(
            value_of(Kind::Material, &choices, " Assets/Rust ").as_deref(),
            Some("bb"),
            "without .material"
        );
        assert_eq!(
            value_of(Kind::Material, &choices, "Concrete").as_deref(),
            Some("Concrete")
        );
        assert_eq!(
            value_of(Kind::Material, &choices, "Neon").as_deref(),
            Some("Neon"),
            "a built-in name"
        );
        assert_eq!(value_of(Kind::Material, &choices, "").as_deref(), Some(""));
        assert_eq!(value_of(Kind::Material, &choices, "Assets/Nope.material"), None);
    }

    #[test]
    fn completions_put_names_that_start_with_the_text_first() {
        let mut choices = some();
        choices.push(Choice {
            path: "Assets/Concrete Rough.material".into(),
            id: "cc".into(),
        });
        choices.push(Choice {
            path: "Assets/Old/Wet Concrete.material".into(),
            id: "dd".into(),
        });
        let names = |typed: &str| -> Vec<&str> {
            completions(Kind::Material, &choices, typed)
                .into_iter()
                .map(|choice| material_name(&choice.path))
                .collect()
        };
        assert_eq!(names("conc"), ["Concrete", "Concrete Rough", "Wet Concrete"]);
        assert_eq!(names("assets/old"), ["Wet Concrete"], "a path's start");
        assert_eq!(names("RUST"), ["Rust"], "whatever the case");
        assert!(names("  ").is_empty() && names("glass").is_empty());
    }

    #[test]
    fn a_picture_is_named_by_its_file_and_a_name_gives_its_id() {
        let choices = vec![
            Choice {
                path: "Assets/Textures/bark.png".into(),
                id: "aa".into(),
            },
            Choice {
                path: "Assets/moss.v2.jpg".into(),
                id: "bb".into(),
            },
            Choice {
                path: "Assets/Old/moss.v2.png".into(),
                id: "cc".into(),
            },
        ];
        let kind = Kind::Picture;
        assert_eq!(short(kind, &choices, "aa"), "bark");
        assert_eq!(short(kind, &choices, "bb"), "moss.v2", "only the last extension goes");
        assert_eq!(long(kind, &choices, "aa"), "Assets/Textures/bark.png");
        assert_eq!(
            value_of(kind, &choices, "Assets/Textures/bark.png").as_deref(),
            Some("aa")
        );
        assert_eq!(value_of(kind, &choices, "Assets/Textures/bark").as_deref(), Some("aa"));
        assert_eq!(
            value_of(kind, &choices, "bark").as_deref(),
            Some("aa"),
            "the one picture of that name"
        );
        assert_eq!(value_of(kind, &choices, "moss.v2"), None, "two of that name");
        assert_eq!(value_of(kind, &choices, "Concrete"), None, "no built-in pictures");
        assert_eq!(value_of(kind, &choices, "").as_deref(), Some(""));
        let found: Vec<&str> = completions(kind, &choices, "mo")
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(found, ["bb", "cc"]);
    }
}
