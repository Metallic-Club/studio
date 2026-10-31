//! Script tabs: a Luau text editor with line numbers and syntax colouring.
//!
//! The gutter holds the script debugger's breakpoints (click toggles, right-click adds a
//! condition or log message) and the paused-line arrow; see [`crate::debugger`]. Breakpoints move
//! with their lines as lines are typed or deleted above them. During Play the text is read-only
//! but can still take focus and a cursor, for F9 and hover values.
//!
//! Each open script keeps a text buffer. The buffer is written to the script's `Source` as one
//! undo step when the editor loses focus, when its tab closes, and before Save, so typing does
//! not flood the undo history with one step per key.
use eframe::egui::{self, Color32, RichText, TextFormat, text::LayoutJob};
use metallic_core::{InstanceId, Value};

use crate::editor::EditorState;
use crate::tokens::ui_tokens;

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "continue", "export",
    "type",
];

struct OpenScript {
    id: InstanceId,
    buffer: String,
    /// The text as of the last edit, to move breakpoints with their lines on the next one.
    before: String,
    /// Lines in `buffer`.
    lines: usize,
    /// The text cursor's character index and line (1-based), for F9.
    cursor: Option<(usize, u32)>,
    /// The text field's height last frame (the gutter matches it).
    height: f32,
    /// A line to scroll into view on the next frame.
    scroll_to: Option<u32>,
}

/// The open script tabs.
#[derive(Default)]
pub struct ScriptTabs {
    open: Vec<OpenScript>,
    /// Index of the shown tab in `open`; `None` shows the 3D scene.
    pub current: Option<usize>,
}

impl ScriptTabs {
    /// No scripts open.
    pub fn new() -> ScriptTabs {
        ScriptTabs::default()
    }

    /// Opens (or switches to) `id`.
    pub fn open(&mut self, id: InstanceId, state: &EditorState) {
        if let Some(i) = self.open.iter().position(|s| s.id == id) {
            self.current = Some(i);
            return;
        }
        let buffer = match state.world.get_ref(id, "Source") {
            Ok(Value::String(s)) => s.clone(),
            _ => return,
        };
        self.open.push(OpenScript {
            id,
            lines: line_count(&buffer),
            before: buffer.clone(),
            buffer,
            cursor: None,
            height: 0.0,
            scroll_to: None,
        });
        self.current = Some(self.open.len() - 1);
    }

    /// Opens `id` and scrolls `line` into view (the debugger paused there).
    pub fn reveal(&mut self, id: InstanceId, line: u32, state: &EditorState) {
        self.open(id, state);
        if let Some(s) = self.current.and_then(|i| self.open.get_mut(i))
            && s.id == id
        {
            s.scroll_to = Some(line);
        }
    }

    /// Writes every changed buffer to its script (before Save and Play).
    pub fn flush(&mut self, state: &mut EditorState) {
        for s in &self.open {
            commit(state, s.id, &s.buffer);
        }
    }

    /// Shows the Scene tab (Play starting).
    pub fn show_scene(&mut self) {
        self.current = None;
    }

    /// Closes tabs whose script left the tree (deleted, or undone insert).
    pub fn prune(&mut self, state: &EditorState) {
        let before = self.open.len();
        self.open.retain(|s| state.is_live(s.id));
        if self.open.len() != before {
            self.current = None;
        }
    }

    /// Draws the tab strip. Returns true when the scene tab is shown.
    pub fn tab_strip(&mut self, ui: &mut egui::Ui, state: &mut EditorState) -> bool {
        let mut close = None;
        crate::panel::tab_strip(ui, |ui| {
            let r = crate::panel::tab(ui, "Scene", self.current.is_none(), false, false);
            if r.clicked {
                self.current = None;
            }
            for (i, s) in self.open.iter().enumerate() {
                let label = state.display_name(s.id);
                let dirty = match state.world.get_ref(s.id, "Source") {
                    Ok(Value::String(src)) => src != &s.buffer,
                    _ => false,
                };
                let r = crate::panel::tab(ui, &label, self.current == Some(i), dirty, true);
                if r.clicked {
                    self.current = Some(i);
                }
                if r.closed {
                    close = Some(i);
                }
            }
        });
        if let Some(i) = close {
            let s = self.open.remove(i);
            commit(state, s.id, &s.buffer);
            self.current = match self.current {
                Some(c) if c == i => None,
                Some(c) if c > i => Some(c - 1),
                c => c,
            };
        }
        self.current.is_none()
    }

    /// Draws the current script.
    pub fn editor(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut EditorState,
        debug: &mut crate::debugger::DebugPanel,
    ) {
        let Some(i) = self.current else { return };
        let Some(s) = self.open.get_mut(i) else {
            self.current = None;
            return;
        };
        // The path label also names the text field for screen readers.
        let title = ui.label(
            RichText::new(state.world.full_name(s.id))
                .color(ui_tokens().text_dim)
                .small(),
        );
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = highlight(text.as_str());
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        // Read-only while playing: the edit world cannot change until Stop.
        let editable = !state.play.is_playing();
        let mut lost_focus = false;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let gutter = crate::debugger::gutter(ui, s.id, s.lines, s.height);
                    // A `&str` buffer takes focus, a cursor and selection but no edits.
                    let mut read_only = s.buffer.as_str();
                    let text: &mut dyn egui::TextBuffer = if editable {
                        &mut s.buffer
                    } else {
                        &mut read_only
                    };
                    let out = egui::TextEdit::multiline(text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .lock_focus(true)
                        .layouter(&mut layouter)
                        .show(ui);
                    let r = out.response.response.labelled_by(title.id);
                    lost_focus = r.lost_focus();
                    s.height = r.rect.height();
                    if r.changed() {
                        state.shift_breakpoints(s.id, &s.before, &s.buffer);
                        s.before.clone_from(&s.buffer);
                        s.lines = line_count(&s.buffer);
                    }
                    if let Some(range) = out.cursor_range {
                        let at = range.primary.index.0;
                        if r.changed() || s.cursor.is_none_or(|(i, _)| i != at) {
                            let line = s.buffer.chars().take(at).filter(|&c| c == '\n').count();
                            s.cursor = Some((at, line as u32 + 1));
                        }
                    }
                    // F9's line: while editing, the shown script's cursor; during Play only
                    // while this editor has focus, so a game keeps its F9.
                    state.debug.cursor = s
                        .cursor
                        .filter(|_| editable || r.has_focus())
                        .map(|(_, line)| (s.id, line));
                    let left = r.rect.left();
                    let rows = crate::debugger::line_rows(&out.galley, out.galley_pos, left);
                    gutter.show(ui, &rows, state, debug);
                    crate::debugger::hover(
                        &r,
                        &out.galley,
                        out.galley_pos,
                        (s.id, &s.buffer),
                        state,
                        debug,
                    );
                    if let Some(line) = s.scroll_to.take()
                        && let Some(row) = rows.iter().find(|r| r.line == line)
                    {
                        ui.scroll_to_rect(row.rect, Some(egui::Align::Center));
                    }
                });
            });
        if lost_focus {
            let (id, text) = (s.id, s.buffer.clone());
            commit(state, id, &text);
        }
    }
}

/// Lines in `text` as the gutter numbers them (at least one).
fn line_count(text: &str) -> usize {
    text.lines().count().max(1)
}

/// True for a Luau keyword (not a name to show a value for).
pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

fn commit(state: &mut EditorState, id: InstanceId, text: &str) {
    if let Ok(Value::String(old)) = state.world.get(id, "Source")
        && old != text
    {
        state.set_property(&[id], "Source", Value::String(text.to_string()));
    }
}

/// Colours Luau source: keywords, strings, numbers and `--` comments.
pub fn highlight(src: &str) -> LayoutJob {
    let mut job = LayoutJob::default();
    let t = ui_tokens();
    let font = t.mono();
    let mut push = |text: &str, colour: Color32| {
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: colour,
                ..Default::default()
            },
        );
    };
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        if c == b'-' && bytes.get(i + 1) == Some(&b'-') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            push(&src[start..i], t.syntax_comment);
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != c && bytes[i] != b'\n' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            push(&src[start..i], t.syntax_string);
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            push(&src[start..i], t.syntax_number);
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            push(
                word,
                if KEYWORDS.contains(&word) {
                    t.syntax_keyword
                } else {
                    t.text
                },
            );
        } else {
            // Advance by one whole character (UTF-8 safe).
            let len = src[i..].chars().next().map_or(1, char::len_utf8);
            i += len;
            push(&src[start..i], ui_tokens().text);
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_covers_every_character() {
        let src = "local x = 5 -- five\nprint(\"hé\", x)";
        let job = highlight(src);
        assert_eq!(job.text, src);
        let colours: Vec<Color32> = job.sections.iter().map(|s| s.format.color).collect();
        let t = ui_tokens();
        for c in [
            t.syntax_keyword,
            t.syntax_string,
            t.syntax_comment,
            t.syntax_number,
        ] {
            assert!(colours.contains(&c));
        }
    }
}
