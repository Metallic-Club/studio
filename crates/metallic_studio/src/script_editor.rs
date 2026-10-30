//! Script tabs: a Luau text editor with line numbers and syntax colouring.
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
        self.open.push(OpenScript { id, buffer });
        self.current = Some(self.open.len() - 1);
    }

    /// Writes every changed buffer to its script (before Save and Play).
    pub fn flush(&mut self, state: &mut EditorState) {
        for s in &self.open {
            commit(state, s.id, &s.buffer);
        }
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
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.current.is_none(), "Scene")
                .clicked()
            {
                self.current = None;
            }
            for (i, s) in self.open.iter().enumerate() {
                let label = state.display_name(s.id);
                let dirty = match state.world.get_ref(s.id, "Source") {
                    Ok(Value::String(src)) => src != &s.buffer,
                    _ => false,
                };
                let text = if dirty { format!("{label} •") } else { label };
                if ui.selectable_label(self.current == Some(i), text).clicked() {
                    self.current = Some(i);
                }
                if ui.small_button("×").on_hover_text("Close").clicked() {
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
    pub fn editor(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let Some(i) = self.current else { return };
        let Some(s) = self.open.get_mut(i) else {
            self.current = None;
            return;
        };
        ui.label(
            RichText::new(state.world.full_name(s.id))
                .color(ui_tokens().text_dim)
                .small(),
        );
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = highlight(text.as_str());
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let lines = s.buffer.lines().count().max(1);
        let mut lost_focus = false;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let numbers: String = (1..=lines).map(|n| format!("{n:>4}\n")).collect();
                    ui.label(
                        RichText::new(numbers)
                            .font(ui_tokens().mono())
                            .color(ui_tokens().text_dim),
                    );
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut s.buffer)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(30)
                            .lock_focus(true)
                            .layouter(&mut layouter),
                    );
                    lost_focus = r.lost_focus();
                });
            });
        if lost_focus {
            let (id, text) = (s.id, s.buffer.clone());
            commit(state, id, &text);
        }
    }
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
