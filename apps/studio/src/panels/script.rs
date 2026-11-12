//! The script editor's panel (#528): one tab per script ([`crate::code`]), its code in the code field
//! ([`crate::code::view`]) in the Code Editor's settings (#529).
//!
//! While typing a name, a `.` or `:`, or the string of `GetService("` and the like, a list of what fits opens under the
//! cursor (#531): Up and Down choose, Tab or Enter takes, Escape closes, and a click takes one too. A name of the API
//! under the pointer says what it is.
//!
//! Ctrl+wheel over it, and Ctrl+plus, minus and 0, change the code's size (#541): the Code Editor's font size.
//!
//! Ctrl+F opens its find bar and Ctrl+H the bar with its replace row (#554, [`crate::code::find_bar`]); F3 and
//! Shift+F3 go to the next match and the one before.

use egui::text::CCursor;
use egui::text_edit::TextEditState;
use egui::{Align, Id, Key, Layout, Modifiers, Order, RichText, ScrollArea, Ui, vec2};

use crate::code::analysis::Diagnostic;
use crate::code::complete::{self, Completion};
use crate::code::find::{self, FindBar};
use crate::code::find_bar::{self, Action, Count};
use crate::theme::current_colors;
use crate::theme::tokens::{space, text};

use super::empty_note;
use crate::code::{self, PANEL};
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::icons::Icon;

/// The script editor.
#[derive(Clone, Copy, Debug)]
pub struct ScriptEditor;

impl Panel for ScriptEditor {
    fn id(&self) -> &'static str {
        PANEL
    }

    fn title(&self) -> &'static str {
        "Script"
    }

    fn icon(&self) -> Icon {
        Icon::ExplorerScript
    }

    fn zone(&self) -> Zone {
        // Beside the viewport, as a script opens in Roblox Studio, until one is docked elsewhere.
        Zone::Centre
    }

    fn context(&self) -> Context {
        Context::Editor
    }

    fn open_by_default(&self) -> bool {
        false
    }

    fn transient(&self) -> bool {
        // A tab per script, opened from the Explorer, and not kept in the layout.
        true
    }

    fn ui(&self, ui: &mut Ui, _cx: &mut PanelContext<'_>) {
        empty_note(ui, "Double-click a script in the Explorer to open it.");
    }

    fn pane_title(&self, pane: &str, cx: &PanelContext<'_>) -> String {
        code::script_of(pane)
            .and_then(|script| cx.editor.dm.name(script).ok())
            .map_or_else(|| self.title().to_owned(), str::to_owned)
    }

    fn pane_ui(&self, ui: &mut Ui, cx: &mut PanelContext<'_>, pane: &str) {
        let Some(script) = code::script_of(pane).filter(|&script| code::in_game(&cx.editor.dm, script)) else {
            empty_note(ui, "This script is no longer in the game.");
            return;
        };
        let member = code::source(&cx.editor.dm, script);
        if member.is_empty() {
            empty_note(
                ui,
                "This script has no file. Double-click it in the Explorer to give it one.",
            );
            return;
        }
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|input| input.pointer.any_pressed()) {
            cx.editor.scripts.focused = Some(script);
        }
        let focus = cx.editor.scripts.take_focus(script);
        let editor = &mut *cx.editor;
        // What is wrong with the code (#530), checked again only when a file changed.
        let (analysis, diagnostics) = editor.scripts.check(&mut editor.project, &editor.dm, script, &member);
        let id = field_id(script);
        if focus {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        let popup_id = id.with("completion");
        let mut popup: Popup = ui.ctx().data(|data| data.get_temp(popup_id)).unwrap_or_default();
        let focused = ui.memory(|memory| memory.has_focus(id));
        if focused {
            editor.scripts.focused = Some(script);
        }
        // The list's keys, before the field sees them (#531).
        let mut taken = false;
        if popup.open && focused {
            let at = code::view::cursor(ui.ctx(), id)
                .map(|cursor| code::view::byte_of(&analysis.source, cursor))
                .unwrap_or_default();
            let completion = editor
                .scripts
                .with_env(&mut editor.project, &editor.dm, script, |env| {
                    complete::complete(&analysis, at, env)
                })
                .filter(|completion| !completion.items.is_empty());
            match completion {
                Some(completion) => {
                    let count = completion.items.len();
                    let key = |key: Key| ui.input_mut(|input| input.consume_key(Modifiers::NONE, key));
                    if key(Key::ArrowDown) {
                        popup.selected = (popup.selected + 1) % count;
                    }
                    if key(Key::ArrowUp) {
                        popup.selected = (popup.selected + count - 1) % count;
                    }
                    // What is typed is already the suggestion (`then`, `end`): Tab and Enter are the field's.
                    let typed = analysis.source.get(completion.replace.clone()).unwrap_or_default();
                    let done = completion
                        .items
                        .get(popup.selected.min(count - 1))
                        .is_some_and(|item| item.label == typed);
                    let pressed = |key: Key| ui.input(|input| input.key_pressed(key));
                    if (done && (pressed(Key::Tab) || pressed(Key::Enter))) || key(Key::Escape) {
                        popup.open = false;
                    } else if key(Key::Tab) || key(Key::Enter) {
                        let text = editor.scripts.text(&mut editor.project, &member);
                        take(ui.ctx(), id, text, &completion, popup.selected.min(count - 1));
                        popup.open = false;
                        taken = true;
                    }
                }
                None => popup.open = false,
            }
        }
        // Ctrl+wheel and Ctrl+plus or minus zoom the code (#541).
        let over = ui.rect_contains_pointer(ui.max_rect());
        code::view::zoom(ui, id, &mut cx.settings.code, over);
        // The find bar (#554): just asked for, its query is the selection when that is on one line.
        let asked = editor.scripts.find.asked == Some(script);
        if asked {
            editor.scripts.find.asked = None;
            let (text, find) = editor.scripts.text_and_find(&mut editor.project, &member);
            if let Some(range) = selected_bytes(ui.ctx(), id, text) {
                if !range.is_empty() && !text[range.clone()].contains('\n') {
                    find.query.text = text[range.clone()].to_owned();
                }
                find.at.insert(script, range);
            }
        }
        if editor.scripts.find.is_open(script)
            && focused
            && !popup.open
            && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            editor.scripts.find.close(script);
        }
        let find_open = editor.scripts.find.is_open(script);
        let finder = if find_open {
            editor.scripts.find.finder()
        } else {
            Ok(None)
        };
        let find_at = editor.scripts.find.at.get(&script).cloned();
        let text = editor.scripts.text(&mut editor.project, &member);
        // While the list or the find bar shows, Escape is theirs: the field keeps the keyboard.
        let output = code::view::show(
            ui,
            id,
            text,
            &cx.settings.code,
            &diagnostics,
            finder
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .map(|finder| (finder, find_at.clone())),
            popup.open || find_open,
        );
        let mut changed = output.changed || taken;
        if find_open {
            let step = editor
                .scripts
                .find
                .step
                .take_if(|(asked, _)| *asked == script)
                .map(|(_, forward)| forward);
            let (text, find) = editor.scripts.text_and_find(&mut editor.project, &member);
            // While the code has the keyboard, the search is where its selection is.
            if output.response.has_focus()
                && let Some(range) = selected_bytes(ui.ctx(), id, text)
            {
                find.at.insert(script, range);
            }
            let mut at = find.at.get(&script).cloned().unwrap_or(0..0);
            let count = count(text, &finder, &at);
            let mut actions = find_bar::show(ui, id.with("find"), output.view, find, &count, asked);
            match step {
                Some(true) => actions.push(Action::Next),
                Some(false) => actions.push(Action::Previous),
                None => {}
            }
            changed |= find_actions(ui.ctx(), id, text, find, &actions, &mut at);
            if actions.contains(&Action::Close) {
                find.close(script);
                // The code gets the keyboard with the match gone to selected.
                select_bytes(ui.ctx(), id, text, at.clone());
                ui.memory_mut(|memory| memory.request_focus(id));
            }
            find.at.insert(script, at);
        }
        if changed {
            let text = editor.scripts.text(&mut editor.project, &member);
            let bytes = text.clone().into_bytes();
            editor.scripts.edited();
            if let Err(error) = editor.project.write(&member, bytes) {
                log::warn!("scripts: could not keep {member}: {error}");
            }
        }
        // What was typed opens the list, or keeps it open (#531).
        if output.changed {
            let typed = ui.input(|input| {
                input.events.iter().rev().find_map(|event| match event {
                    egui::Event::Text(text) => text.chars().next_back(),
                    _ => None,
                })
            });
            match typed {
                Some(c) if c == '_' || c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '"' | '\'') => {
                    if !popup.open {
                        popup.selected = 0;
                    }
                    popup.open = true;
                }
                Some(_) => popup.open = false,
                None => {}
            }
        }
        // A click on the list takes the keyboard from the field for a moment: the list stays, at the field's cursor.
        let over_list = ui
            .ctx()
            .memory(|memory| memory.area_rect(popup_id))
            .is_some_and(|rect| {
                ui.input(|input| input.pointer.interact_pos())
                    .is_some_and(|at| rect.contains(at))
            });
        let cursor = output.cursor.or_else(|| {
            if over_list {
                code::view::cursor(ui.ctx(), id)
            } else {
                None
            }
        });
        if cursor.is_none() || taken {
            popup.open = false;
        }
        let (analysis, diagnostics) = if changed {
            editor.scripts.check(&mut editor.project, &editor.dm, script, &member)
        } else {
            (analysis, diagnostics)
        };
        if popup.open
            && let Some(cursor) = cursor
        {
            let at = code::view::byte_of(&analysis.source, cursor);
            let completion = editor
                .scripts
                .with_env(&mut editor.project, &editor.dm, script, |env| {
                    complete::complete(&analysis, at, env)
                })
                .filter(|completion| !completion.items.is_empty());
            match completion {
                Some(completion) => {
                    popup.selected = popup.selected.min(completion.items.len() - 1);
                    let place = output.galley_pos
                        + output
                            .galley
                            .pos_from_cursor(CCursor::new(cursor))
                            .left_bottom()
                            .to_vec2();
                    if let Some(chosen) = list(ui, popup_id, place, &completion, &mut popup.selected) {
                        let text = editor.scripts.text(&mut editor.project, &member);
                        take(ui.ctx(), id, text, &completion, chosen);
                        popup.open = false;
                        ui.memory_mut(|memory| memory.request_focus(id));
                        let bytes = text.clone().into_bytes();
                        editor.scripts.edited();
                        if let Err(error) = editor.project.write(&member, bytes) {
                            log::warn!("scripts: could not keep {member}: {error}");
                        }
                    }
                }
                None => popup.open = false,
            }
        }
        ui.ctx().data_mut(|data| data.insert_temp(popup_id, popup));
        // Ctrl and the pointer over a name that is declared somewhere: underlined; clicked, it goes there (#532).
        let ctrl = ui.input(|input| input.modifiers.command);
        if ctrl && let Some(at) = output.hovered {
            let target = editor.scripts.with_env(&mut editor.project, &editor.dm, script, |env| {
                code::definition::definition(&analysis, at, env)
            });
            if let Some(target) = target
                && let Some(token) = analysis.token_at(at).and_then(|index| analysis.tokens.get(index))
            {
                let accent = current_colors(ui.ctx()).accent;
                code::view::underline(ui.painter(), &output, &analysis.source, token.start..token.end, accent);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if output.response.clicked() {
                    go_to(ui.ctx(), editor, script, target);
                }
            }
        }
        // What a name of the API is, under the pointer (#531), where no problem is.
        if !popup.open
            && let Some(at) = output.hovered
            && !over_problem(&diagnostics, at)
            && !ui.input(|input| input.modifiers.command)
        {
            let hover = editor.scripts.with_env(&mut editor.project, &editor.dm, script, |env| {
                complete::hover(&analysis, at, env)
            });
            if let Some(hover) = hover {
                let p = current_colors(ui.ctx()).palette;
                output.response.clone().on_hover_ui_at_pointer(|ui| {
                    ui.set_max_width(420.0);
                    ui.label(
                        RichText::new(&hover.title)
                            .font(code::view::font(&cx.settings.code).clone())
                            .color(p.text),
                    );
                    if !hover.doc.is_empty() {
                        ui.add_space(space::S2);
                        ui.label(RichText::new(&hover.doc).size(text::SMALL).color(p.muted));
                    }
                });
            }
        }
    }
}

/// Shows `target` (#532): the cursor there, in this script's field or, opening its tab, another's.
fn go_to(
    ctx: &egui::Context,
    editor: &mut crate::editor::EditorState,
    script: code::ScriptId,
    target: code::definition::Target,
) {
    use code::definition::Target;
    let (other, range) = match target {
        Target::Here(range) => (script, range),
        Target::Script(other, range) => (other, range),
    };
    if other != script {
        editor.open_script(other);
    }
    let member = code::source(&editor.dm, other);
    let text = editor.scripts.text(&mut editor.project, &member);
    let at = text[..range.start.min(text.len())].chars().count();
    code::view::reveal(ctx, field_id(other), at);
}

/// What is selected in the field `id` showing `text`, as bytes.
fn selected_bytes(ctx: &egui::Context, id: Id, text: &str) -> Option<std::ops::Range<usize>> {
    let chars = code::view::selection(ctx, id)?;
    Some(code::view::byte_of(text, chars.start)..code::view::byte_of(text, chars.end))
}

/// Selects the bytes `range` of `text` in the field `id`, scrolling to them (#554).
fn select_bytes(ctx: &egui::Context, id: Id, text: &str, range: std::ops::Range<usize>) {
    let start = text[..range.start.min(text.len())].chars().count();
    let end = text[..range.end.min(text.len())].chars().count();
    code::view::select(ctx, id, start..end);
}

/// How many matches the find bar's query `finder` has in `text`, and which is the one at `at`.
fn count(text: &str, finder: &Result<Option<find::Finder>, String>, at: &std::ops::Range<usize>) -> Count {
    match finder {
        Err(why) => Count::Error(why.clone()),
        Ok(None) => Count::Empty,
        Ok(Some(finder)) => {
            let matches = finder.matches(text);
            let current = find::current(&matches, at);
            Count::Found {
                current,
                total: matches.len(),
            }
        }
    }
}

/// Does what the find bar asked (#554) in the field `id` showing `text`, the search being `at` (bytes): goes to a
/// match, or replaces one or all, `at` following. Returns whether `text` changed.
fn find_actions(
    ctx: &egui::Context,
    id: Id,
    text: &mut String,
    find: &mut FindBar,
    actions: &[Action],
    at: &mut std::ops::Range<usize>,
) -> bool {
    let mut changed = false;
    let go = |text: &str, range: std::ops::Range<usize>, at: &mut std::ops::Range<usize>| {
        select_bytes(ctx, id, text, range.clone());
        *at = range;
    };
    for action in actions {
        // Made again after a toggle or the query changed.
        let Ok(Some(finder)) = find.finder() else {
            continue;
        };
        let matches = finder.matches(text);
        let selection = at.clone();
        let target = match action {
            Action::Changed => find::next(&matches, &(selection.start..selection.start)),
            Action::Next => find::next(&matches, &selection),
            Action::Previous => find::previous(&matches, &selection),
            Action::Replace => match find::current(&matches, &selection) {
                Some(index) => {
                    let range = matches[index].clone();
                    let with = finder.replacement(text, range.clone(), &find.replace);
                    text.replace_range(range.clone(), &with);
                    changed = true;
                    let after = range.start + with.len();
                    let matches = finder.matches(text);
                    match find::next(&matches, &(after..after)) {
                        Some(next) => go(text, matches[next].clone(), at),
                        None => go(text, after..after, at),
                    }
                    continue;
                }
                None => find::next(&matches, &selection),
            },
            Action::ReplaceAll => {
                if !matches.is_empty() {
                    *text = finder.replace_all(text, &matches, &find.replace);
                    changed = true;
                    let start = selection.start.min(text.len());
                    let start = (0..=start).rev().find(|&byte| text.is_char_boundary(byte)).unwrap_or(0);
                    go(text, start..start, at);
                }
                continue;
            }
            Action::Close => continue,
        };
        if let Some(index) = target {
            go(text, matches[index].clone(), at);
        }
    }
    changed
}

/// Whether byte `at` is in a problem's range: its message shows there instead.
fn over_problem(diagnostics: &[Diagnostic], at: usize) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.range.start <= at && at < diagnostic.range.end.max(diagnostic.range.start + 1))
}

/// The completion list's state, kept per field.
#[derive(Clone, Copy, Debug, Default)]
struct Popup {
    /// Whether it shows.
    open: bool,
    /// The suggestion chosen.
    selected: usize,
}

/// How many suggestions show at once.
const ROWS: usize = 8;

/// Puts the suggestion `chosen` of `completion` in `text`, in place of the word typed, the cursor after it.
fn take(ctx: &egui::Context, id: Id, text: &mut String, completion: &Completion, chosen: usize) {
    let Some(item) = completion.items.get(chosen) else {
        return;
    };
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    let start = text[..completion.replace.start.min(text.len())].chars().count();
    let end = text[..completion.replace.end.min(text.len())].chars().count();
    code::view::replace_chars(ctx, id, &mut state, text, start..end, &item.label);
}

/// Draws the suggestions with their top-left at `place`, `selected` lit and its description under them. Returns the
/// one clicked.
fn list(ui: &Ui, id: Id, place: egui::Pos2, completion: &Completion, selected: &mut usize) -> Option<usize> {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let mut clicked = None;
    egui::Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(place + vec2(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(320.0);
                let row = 22.0;
                ScrollArea::vertical()
                    .id_salt(id.with("rows"))
                    .max_height(row * ROWS as f32)
                    .show(ui, |ui| {
                        for (index, item) in completion.items.iter().enumerate() {
                            let chosen = index == *selected;
                            let response = ui
                                .allocate_ui_with_layout(
                                    vec2(ui.available_width(), row),
                                    Layout::left_to_right(Align::Center),
                                    |ui| {
                                        ui.set_min_size(vec2(ui.available_width(), row));
                                        if chosen {
                                            ui.painter().rect_filled(ui.max_rect(), 3.0, p.surface_hover);
                                        }
                                        ui.add_space(space::S2);
                                        ui.label(
                                            RichText::new(item.kind.letter())
                                                .monospace()
                                                .size(text::SMALL)
                                                .color(colors.accent),
                                        );
                                        ui.add_space(space::S2);
                                        ui.label(RichText::new(&item.label).monospace().color(p.text));
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            ui.add_space(space::S2);
                                            let detail: String = item.detail.chars().take(28).collect();
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(detail).size(text::SMALL).color(p.subtle),
                                                )
                                                .truncate(),
                                            );
                                        });
                                    },
                                )
                                .response
                                .interact(egui::Sense::click());
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(egui::Role::ListItem, true, chosen, &item.label)
                            });
                            if chosen {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                clicked = Some(index);
                            }
                        }
                    });
                if let Some(item) = completion.items.get(*selected)
                    && (!item.doc.is_empty() || !item.detail.is_empty())
                {
                    ui.separator();
                    if !item.detail.is_empty() {
                        ui.label(RichText::new(&item.detail).monospace().size(text::SMALL).color(p.text));
                    }
                    if !item.doc.is_empty() {
                        ui.label(RichText::new(&item.doc).size(text::SMALL).color(p.muted));
                    }
                }
            });
        });
    if let Some(index) = clicked {
        *selected = index;
    }
    clicked
}

/// The id of `script`'s code field.
pub fn field_id(script: code::ScriptId) -> Id {
    Id::unique("metallic.script").with(script)
}
