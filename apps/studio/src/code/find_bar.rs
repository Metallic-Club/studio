//! A script's find bar (#554), at the top right of its code as VS Code has it: the query with its match case, whole
//! word and regular expression toggles, how many matches there are and which is selected, previous, next and close;
//! and, opened by its arrow (or Ctrl+H), a row to replace the selected match or every one.
//!
//! Enter goes to the next match and Shift+Enter to the one before; in the replace field Enter replaces, and
//! Ctrl+Alt+Enter replaces all. Escape closes the bar, and Alt+C, Alt+W and Alt+R turn the toggles on and off.

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    Align, Color32, CornerRadius, FontFamily, FontId, Id, Key, Layout, Modifiers, Rect, Response, Role, Sense, Stroke,
    TextEdit, Ui, UiBuilder, WidgetInfo, pos2, vec2,
};

use super::find::FindBar;
use crate::theme::current_colors;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{space, text};

/// What the bar was asked to do this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The query or a toggle changed: go to the first match from the selection.
    Changed,
    /// Go to the next match.
    Next,
    /// Go to the match before.
    Previous,
    /// Replace the selected match, and go to the next.
    Replace,
    /// Replace every match.
    ReplaceAll,
    /// Close the bar.
    Close,
}

/// How many matches the query has, as the bar shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Count {
    /// Nothing is looked for.
    Empty,
    /// The pattern is not one: why.
    Error(String),
    /// `total` matches, the selected one `current` (from 0) if the selection is one.
    Found {
        /// The selected match.
        current: Option<usize>,
        /// How many there are.
        total: usize,
    },
}

impl Count {
    /// The words the bar shows.
    pub fn words(&self) -> String {
        match self {
            Count::Empty => String::new(),
            Count::Error(why) => why.clone(),
            Count::Found { total: 0, .. } => "No results".to_owned(),
            Count::Found {
                current: Some(current),
                total,
            } => format!("{} of {total}", current + 1),
            Count::Found { current: None, total } => format!("? of {total}"),
        }
    }
}

/// The bar's width, when the code is wide enough.
const WIDTH: f32 = 440.0;

/// The height of each of its rows.
const ROW: f32 = 26.0;

/// The side of its buttons.
const BUTTON: f32 = 22.0;

/// The width of the words saying how many matches there are.
const COUNT: f32 = 78.0;

/// How far it is from the top of the code and from its right (left of the scroll bar).
const INSET: f32 = 14.0;

/// The id of the bar's query field, for the bar `id`.
pub fn query_id(id: Id) -> Id {
    id.with("query")
}

/// The id of the bar's replace field, for the bar `id`.
pub fn replace_id(id: Id) -> Id {
    id.with("replace")
}

/// The rectangle the bar takes at the top right of `view`, the code's area, with its replace row when `replace`.
pub fn rect(view: Rect, replace: bool) -> Rect {
    let width = WIDTH.min(view.width() - 2.0 * INSET).max(0.0);
    let height = if replace { 2.0 * ROW } else { ROW } + 2.0 * space::S2;
    Rect::from_min_size(
        pos2(view.right() - INSET - width, view.top() + space::S2),
        vec2(width, height),
    )
}

/// Draws the bar `id` of `bar` at the top right of `view`, with the matches `count`. With `focus`, its query takes the
/// keyboard, all of it selected. Returns what it was asked to do.
pub fn show(ui: &mut Ui, id: Id, view: Rect, bar: &mut FindBar, count: &Count, focus: bool) -> Vec<Action> {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let mut actions = Vec::new();
    let query_id = query_id(id);
    let replace_id = replace_id(id);
    // Its keys, before its fields see them.
    let (in_query, in_replace) = ui.memory(|memory| (memory.has_focus(query_id), memory.has_focus(replace_id)));
    if in_query || in_replace {
        ui.input_mut(|input| {
            if input.consume_key(Modifiers::NONE, Key::Escape) {
                actions.push(Action::Close);
            }
            if input.consume_key(Modifiers::ALT, Key::C) {
                bar.query.case = !bar.query.case;
                actions.push(Action::Changed);
            }
            if input.consume_key(Modifiers::ALT, Key::W) {
                bar.query.word = !bar.query.word;
                actions.push(Action::Changed);
            }
            if input.consume_key(Modifiers::ALT, Key::R) {
                bar.query.regex = !bar.query.regex;
                actions.push(Action::Changed);
            }
            if in_query {
                if input.consume_key(Modifiers::SHIFT, Key::Enter) {
                    actions.push(Action::Previous);
                }
                if input.consume_key(Modifiers::NONE, Key::Enter) {
                    actions.push(Action::Next);
                }
            } else {
                if input.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::Enter) {
                    actions.push(Action::ReplaceAll);
                }
                if input.consume_key(Modifiers::NONE, Key::Enter) {
                    actions.push(Action::Replace);
                }
            }
        });
    }
    let rect = rect(view, bar.show_replace);
    if rect.width() < 4.0 * BUTTON {
        return actions;
    }
    // Its ground takes the clicks that miss its controls, so they do not reach the code under it.
    let _ground = ui.interact(rect, id.with("ground"), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect.translate(vec2(0.0, 2.0)), 4.0, Color32::from_black_alpha(40));
    painter.rect_filled(rect, 4.0, p.surface);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    let inner = rect.shrink(space::S2);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min))
            .id_salt(id.with("bar")),
    );
    let ui = &mut child;
    ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
    let field_width = (inner.width() - BUTTON - 3.0 * BUTTON - COUNT - 3.0 * BUTTON - 10.0 * 2.0).max(40.0);
    ui.allocate_ui_with_layout(vec2(inner.width(), ROW), Layout::left_to_right(Align::Center), |ui| {
        let (icon, words) = if bar.show_replace {
            (Icon::UiCollapse, "Hide replace")
        } else {
            (Icon::UiExpand, "Show replace")
        };
        if icon_button(ui, id.with("toggle"), icon, words, true).clicked() {
            bar.show_replace = !bar.show_replace;
        }
        let response = field(ui, query_id, &mut bar.query.text, "Find", field_width, count);
        if response.changed() {
            actions.push(Action::Changed);
        }
        if focus {
            focus_all(ui.ctx(), query_id, bar.query.text.chars().count());
        }
        if toggle(ui, id.with("case"), "Aa", "Match case (Alt+C)", &mut bar.query.case) {
            actions.push(Action::Changed);
        }
        if toggle(
            ui,
            id.with("word"),
            "ab",
            "Match whole word (Alt+W)",
            &mut bar.query.word,
        ) {
            actions.push(Action::Changed);
        }
        if toggle(
            ui,
            id.with("regex"),
            ".*",
            "Use regular expression (Alt+R)",
            &mut bar.query.regex,
        ) {
            actions.push(Action::Changed);
        }
        let (count_rect, _) = ui.allocate_exact_size(vec2(COUNT, ROW), Sense::hover());
        let ink = match count {
            Count::Error(_) => crate::code::view::severity_color(crate::code::analysis::Severity::Error, colors.dark),
            Count::Found { total: 0, .. } => {
                crate::code::view::severity_color(crate::code::analysis::Severity::Warning, colors.dark)
            }
            _ => p.muted,
        };
        ui.painter().text(
            count_rect.left_center() + vec2(space::S2, 0.0),
            egui::Align2::LEFT_CENTER,
            count.words(),
            FontId::proportional(text::SMALL),
            ink,
        );
        let any = matches!(count, Count::Found { total, .. } if *total > 0);
        if icon_button(ui, id.with("previous"), Icon::UiMoveUp, "Previous match", any)
            .on_hover_text("Previous match (Shift+Enter)")
            .clicked()
        {
            actions.push(Action::Previous);
        }
        if icon_button(ui, id.with("next"), Icon::UiMoveDown, "Next match", any)
            .on_hover_text("Next match (Enter)")
            .clicked()
        {
            actions.push(Action::Next);
        }
        if icon_button(ui, id.with("close"), Icon::UiClose, "Close find", true)
            .on_hover_text("Close (Escape)")
            .clicked()
        {
            actions.push(Action::Close);
        }
    });
    if bar.show_replace {
        ui.allocate_ui_with_layout(vec2(inner.width(), ROW), Layout::left_to_right(Align::Center), |ui| {
            ui.add_space(BUTTON + 2.0);
            field(ui, replace_id, &mut bar.replace, "Replace", field_width, &Count::Empty);
            let any = matches!(count, Count::Found { total, .. } if *total > 0);
            if text_button(ui, id.with("replace-one"), "Replace", "Replace", any)
                .on_hover_text("Replace (Enter)")
                .clicked()
            {
                actions.push(Action::Replace);
            }
            if text_button(ui, id.with("replace-all"), "All", "Replace all", any)
                .on_hover_text("Replace all (Ctrl+Alt+Enter)")
                .clicked()
            {
                actions.push(Action::ReplaceAll);
            }
        });
    }
    actions
}

/// A field of the bar: `label` for screen readers and as its hint, its border red while `count` is an error.
fn field(ui: &mut Ui, id: Id, value: &mut String, label: &str, width: f32, count: &Count) -> Response {
    let dark = current_colors(ui.ctx()).dark;
    let response = ui.add(
        TextEdit::singleline(value)
            .id(id)
            .hint_text(label)
            .font(FontId::monospace(text::BODY))
            // Escape is the bar's: it closes it, rather than only leaving the field.
            .event_filter(egui::EventFilter {
                horizontal_arrows: true,
                escape: true,
                ..egui::EventFilter::default()
            })
            .desired_width(width)
            .margin(vec2(space::S2, 3.0)),
    );
    if matches!(count, Count::Error(_)) {
        let red = crate::code::view::severity_color(crate::code::analysis::Severity::Error, dark);
        ui.painter()
            .rect_stroke(response.rect, 2.0, Stroke::new(1.0, red), egui::StrokeKind::Inside);
    }
    response.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, label));
    response
}

/// Gives the field `id` the keyboard with its `chars` characters all selected.
fn focus_all(ctx: &egui::Context, id: Id, chars: usize) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(chars))));
    state.store(ctx, id);
    ctx.memory_mut(|memory| memory.request_focus(id));
}

/// A square button of the bar showing `icon`, `label` for screen readers.
fn icon_button(ui: &mut Ui, id: Id, icon: Icon, label: &str, enabled: bool) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let (rect, _) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::hover());
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, id, sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    if enabled && response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), p.surface_hover);
    }
    let ink = if !enabled {
        p.subtle
    } else if response.hovered() {
        p.text
    } else {
        p.muted
    };
    icons::paint_at(
        ui.painter(),
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        text::BODY,
        ink,
    );
    response
}

/// A small button of the bar showing `words`, `label` for screen readers.
fn text_button(ui: &mut Ui, id: Id, words: &str, label: &str, enabled: bool) -> Response {
    let p = current_colors(ui.ctx()).palette;
    let font = FontId::proportional(text::SMALL);
    let width = ui.fonts_mut(|fonts| fonts.layout_no_wrap(words.to_owned(), font.clone(), p.text).size().x) + 12.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, BUTTON), Sense::hover());
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, id, sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    if enabled && response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), p.surface_hover);
    }
    let ink = if enabled { p.text } else { p.subtle };
    ui.painter()
        .text(rect.center(), egui::Align2::CENTER_CENTER, words, font, ink);
    response
}

/// A toggle of the bar showing `words` in code's letters, lit while `on`; `label` for screen readers and its tooltip.
/// Returns whether it was clicked.
fn toggle(ui: &mut Ui, id: Id, words: &str, label: &str, on: &mut bool) -> bool {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let (rect, _) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let selected = *on;
    response.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, selected, label));
    if *on {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), colors.accent.gamma_multiply(0.3));
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, colors.accent),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), p.surface_hover);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        words,
        FontId::new(text::SMALL, FontFamily::Monospace),
        if *on { p.text } else { p.muted },
    );
    let clicked = response.on_hover_text(label).clicked();
    if clicked {
        *on = !*on;
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_says_which_match_of_how_many() {
        assert_eq!(Count::Empty.words(), "");
        assert_eq!(
            Count::Found {
                current: None,
                total: 0
            }
            .words(),
            "No results"
        );
        assert_eq!(
            Count::Found {
                current: Some(2),
                total: 7
            }
            .words(),
            "3 of 7"
        );
        assert_eq!(
            Count::Found {
                current: None,
                total: 7
            }
            .words(),
            "? of 7"
        );
        assert_eq!(Count::Error("bad".into()).words(), "bad");
    }

    #[test]
    fn the_bar_sits_at_the_top_right_of_the_code() {
        let view = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
        let bar = rect(view, false);
        assert_eq!(bar.right(), view.right() - INSET);
        assert!(bar.top() > view.top() && bar.top() < view.top() + 10.0);
        assert_eq!(bar.width(), WIDTH);
        assert!(rect(view, true).height() > bar.height());
        // Narrow code: no wider than it.
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 600.0));
        assert!(rect(narrow, false).left() >= narrow.left());
    }
}
