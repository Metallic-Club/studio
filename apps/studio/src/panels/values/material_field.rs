//! A part's Material in the Properties panel (#739): a text field and, at its right, a button that opens a window to
//! pick one, as Godot's resource slots have it.
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
//!   double-click or Select chooses it, Cancel or Escape closes.
//! - A material dragged from the Assets panel and let go on the field is chosen, as on any file picker (#676).

use egui::{
    Align2, CornerRadius, FontId, Id, Key, Modal, Modifiers, Rect, Role, Sense, Stroke, TextEdit, Ui, WidgetInfo, pos2,
    vec2,
};
use metallic_engine::metallic_project::material::{BUILTIN_NAMES, material_name};

use super::{Files, NO_FILE, field_height, labelled};
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

/// One material the field or the window can name: its file's path and its asset id.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Choice {
    path: String,
    id: String,
}

/// The project's materials, by path.
fn choices(files: &Files, widget: metallic_engine::metallic_core::Widget) -> Vec<Choice> {
    files
        .members
        .iter()
        .filter(|(path, _)| widget.takes_file(path))
        .filter_map(|(path, id)| {
            Some(Choice {
                path: path.clone(),
                id: id.clone()?,
            })
        })
        .collect()
}

/// What the field shows for the value `current`: a material file's name, the name a value gives, or nothing.
fn short(choices: &[Choice], current: &str) -> String {
    match choices.iter().find(|choice| choice.id == current) {
        Some(choice) => material_name(&choice.path).to_owned(),
        None => current.to_owned(),
    }
}

/// What the focused field shows for `current`: the material file's path, the path of the project's material of the
/// name a value gives, or that name when the project has none.
fn long(choices: &[Choice], current: &str) -> String {
    choices
        .iter()
        .find(|choice| choice.id == current)
        .or_else(|| choices.iter().find(|choice| material_name(&choice.path) == current))
        .map_or_else(|| current.to_owned(), |choice| choice.path.clone())
}

/// The value `typed` names: a material file's id (by its path, `.material` or not), a material's name, or `""` for
/// nothing. `None` when it names no material.
fn value_of(choices: &[Choice], typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Some(String::new());
    }
    let path = |choice: &&Choice| {
        choice.path == typed || choice.path.strip_suffix(".material").is_some_and(|stem| stem == typed)
    };
    if let Some(choice) = choices.iter().find(path) {
        return Some(choice.id.clone());
    }
    let named = choices.iter().any(|choice| material_name(&choice.path) == typed);
    (named || BUILTIN_NAMES.contains(&typed)).then(|| typed.to_owned())
}

/// The materials offered for `typed` (#760): those whose name or path holds it, ignoring case, at most
/// [`COMPLETIONS`]. Those whose name starts with it come first, then those whose path does, each by name.
fn completions<'a>(choices: &'a [Choice], typed: &str) -> Vec<&'a Choice> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(u8, String, &Choice)> = choices
        .iter()
        .filter_map(|choice| {
            let name = material_name(&choice.path).to_lowercase();
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

/// The Material field of property `label`, whose value is `current` (an asset id, a name, or nothing), `width` wide.
/// Returns the value chosen: an asset id, a name, or `""` for none.
pub(super) fn material_field(
    ui: &mut Ui,
    files: &Files,
    widget: metallic_engine::metallic_core::Widget,
    label: &str,
    current: &str,
    width: f32,
    mixed: bool,
) -> Option<String> {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let choices = choices(files, widget);
    let id = ui.make_persistent_id(("metallic.properties.material", label));
    let field_id = id.with("field");
    let window_id = id.with("window");
    let focused = ui.memory(|memory| memory.has_focus(field_id));
    let shown = if mixed { String::new() } else { short(&choices, current) };
    // Focused, the field holds what is typed, from the whole path on.
    let mut typing = if focused {
        ui.data_mut(|data| data.get_temp::<Typing>(id))
            .unwrap_or_else(|| Typing {
                text: if mixed { String::new() } else { long(&choices, current) },
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
        completions(&choices, &typing.text)
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
    let field_width = (width - button_side - space::S1).max(0.0);
    let mut chosen = None;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::S1;
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
            let path = if mixed { String::new() } else { long(&choices, current) };
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
            let completed = match (typing.picked, value_of(&choices, &typed)) {
                (Some(picked), _) if entered || tabbed => offered.get(picked),
                (None, None) if entered => offered.first(),
                (None, _) if tabbed => offered.first(),
                _ => None,
            };
            let value = match completed {
                Some(choice) => Some(choice.id.clone()),
                None => value_of(&choices, &typed),
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
        if open.on_hover_text("Choose a material").clicked() {
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
        if let Some(clicked) = completion_list(ui, id.with("completions"), field, &offered, typing.picked)
            && let Some(choice) = offered.get(clicked)
        {
            chosen = Some(choice.id.clone()).filter(|value| value != current || mixed);
        }
    }
    // A material dragged from the Assets panel and let go on the field (#676).
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
    if let Some(picked) = window(ui.ctx(), window_id, &choices) {
        chosen = Some(picked).filter(|picked| picked != current || mixed);
    }
    chosen
}

/// The window, while it is open: returns the value chosen when it closes with one.
fn window(ctx: &egui::Context, id: Id, choices: &[Choice]) -> Option<String> {
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
            egui::RichText::new("Choose a material")
                .font(fonts::named(fonts::HEADING, text::TITLE))
                .color(p.text),
        );
        ui.add_space(space::S4);
        crate::panels::search_box(ui, &mut state.query, "Search the materials");
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
                ui.spacing_mut().item_spacing.y = 0.0;
                let none = std::iter::once((NO_FILE, None, String::new()));
                let materials = shown.iter().map(|choice| {
                    (
                        material_name(&choice.path),
                        Some(choice.path.as_str()),
                        choice.id.clone(),
                    )
                });
                for (index, (name, path, value)) in none.chain(materials).enumerate() {
                    let on = state.picked.as_deref() == Some(value.as_str());
                    let response = row(ui, Role::Row, index, name, path, on);
                    if response.clicked() {
                        state.picked = Some(value.clone());
                    }
                    if response.double_clicked() {
                        state.picked = Some(value);
                        choose = true;
                    }
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

/// The completions `offered` for the field at `field`, in a list under it, the `picked`th picked out. Returns the one
/// clicked.
fn completion_list(ui: &Ui, id: Id, field: Rect, offered: &[&Choice], picked: Option<usize>) -> Option<usize> {
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
                    let name = material_name(&choice.path);
                    let on = picked == Some(index);
                    if row(ui, Role::ListBoxOption, index, name, Some(&choice.path), on).clicked() {
                        clicked = Some(index);
                    }
                }
            });
        });
    clicked
}

/// One row of a list, the `index`th, for screen readers a `role`: the material's icon, its name and its path, picked
/// out when `on`.
fn row(ui: &mut Ui, role: Role, index: usize, name: &str, path: Option<&str>, on: bool) -> egui::Response {
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
            icons::paint(
                ui.painter(),
                Icon::AssetMaterial,
                icon,
                Icon::AssetMaterial.color(&colors),
            );
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
        assert_eq!(short(&choices, "bb"), "Rust");
        assert_eq!(long(&choices, "bb"), "Assets/Rust.material");
        // A value naming a material shows the name, and focused the project's file of that name.
        assert_eq!(short(&choices, "Concrete"), "Concrete");
        assert_eq!(long(&choices, "Concrete"), "Assets/Materials/Concrete.material");
        assert_eq!(long(&choices, "Neon"), "Neon", "a built-in the project has no file of");
        assert_eq!(short(&choices, ""), "");
    }

    #[test]
    fn what_is_typed_names_a_file_a_name_or_nothing() {
        let choices = some();
        assert_eq!(value_of(&choices, "Assets/Rust.material").as_deref(), Some("bb"));
        assert_eq!(
            value_of(&choices, " Assets/Rust ").as_deref(),
            Some("bb"),
            "without .material"
        );
        assert_eq!(value_of(&choices, "Concrete").as_deref(), Some("Concrete"));
        assert_eq!(value_of(&choices, "Neon").as_deref(), Some("Neon"), "a built-in name");
        assert_eq!(value_of(&choices, "").as_deref(), Some(""));
        assert_eq!(value_of(&choices, "Assets/Nope.material"), None);
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
            completions(&choices, typed)
                .into_iter()
                .map(|choice| material_name(&choice.path))
                .collect()
        };
        assert_eq!(names("conc"), ["Concrete", "Concrete Rough", "Wet Concrete"]);
        assert_eq!(names("assets/old"), ["Wet Concrete"], "a path's start");
        assert_eq!(names("RUST"), ["Rust"], "whatever the case");
        assert!(names("  ").is_empty() && names("glass").is_empty());
    }
}
