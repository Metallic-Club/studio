//! The code field (#528, #529): a script's text in the Code Editor's font and colours, with the line numbers down its
//! left, the cursor's line lit, whitespace shown if asked, and Tab and Enter as a code editor has them. It scrolls both
//! ways: code is not wrapped.

use std::sync::Arc;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    Align2, FontFamily, FontId, Galley, Id, Key, Modifiers, Pos2, Rect, Response, ScrollArea, Sense, Shape, Stroke,
    TextBuffer, TextEdit, Ui, UiBuilder, pos2, vec2,
};

use super::highlight;
use super::settings::{CodeEditor, Kind};
use crate::theme::tokens::space;
use crate::theme::{current_colors, fonts};

/// What the field did this frame.
pub struct ViewOutput {
    /// Whether the text was edited.
    pub changed: bool,
    /// The field's response.
    pub response: Response,
    /// The text as laid out.
    pub galley: Arc<Galley>,
    /// Where the galley's top-left corner is on the screen.
    pub galley_pos: Pos2,
    /// The cursor, as a character index, while the field has the keyboard.
    pub cursor: Option<usize>,
    /// The gutter's area on the screen.
    pub gutter: Rect,
}

/// The font of code in `settings`.
pub fn font(settings: &CodeEditor) -> FontId {
    FontId::new(settings.size, FontFamily::Name(fonts::CODE.into()))
}

/// Draws `text` as code filling `ui`, the field interacting as `id`.
pub fn show(ui: &mut Ui, id: Id, text: &mut String, settings: &CodeEditor) -> ViewOutput {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let font = font(settings);
    let row_height = settings.size * settings.line_spacing;
    let digit = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
    let mut changed = keys(ui, id, text, settings);
    let lines = text.split('\n').count();
    let digits = lines.to_string().len().max(2);
    let gutter_width = digits as f32 * digit + 2.0 * space::S4;
    let dark = colors.dark;
    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap: f32| -> Arc<Galley> {
        let job = highlight::layout_job(text.as_str(), settings, &font, dark);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let inner = ScrollArea::both()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let widest = layouter(ui, text, f32::INFINITY).size().x;
            let view = ui.available_size();
            let width = (widest + 4.0 * digit).max(view.x - gutter_width);
            let height = (lines as f32 * row_height).max(view.y);
            let top_left = ui.cursor().min;
            let gutter = Rect::from_min_size(top_left, vec2(gutter_width, height));
            let field = Rect::from_min_size(pos2(gutter.right(), top_left.y), vec2(width, height));
            ui.allocate_rect(gutter.union(field), Sense::hover());
            // Behind the text: the cursor's line, filled in once the field is laid out.
            let line_light = ui.painter().add(Shape::Noop);
            // Every row the view holds is the field's, so a click under the last line puts the cursor at the end.
            let rows = ((height / row_height).floor() as usize).max(lines);
            let output = ui
                .scope_builder(UiBuilder::new().max_rect(field), |ui| {
                    TextEdit::multiline(text)
                        .id(id)
                        .font(font.clone())
                        .code_editor()
                        .lock_focus(true)
                        .frame(egui::Frame::NONE)
                        .margin(vec2(space::S3, 0.0))
                        .desired_width(width)
                        .desired_rows(rows)
                        .layouter(&mut layouter)
                        .show(ui)
                })
                .inner;
            let focused = output.response.response.has_focus();
            let cursor = output.cursor_range.filter(|_| focused).map(|range| range.primary);
            let current = cursor.map(|cursor| output.galley.layout_from_cursor(cursor).row);
            let galley = &output.galley;
            let painter = ui.painter();
            if settings.highlight_line
                && let Some(row) = current.and_then(|row| galley.rows.get(row))
            {
                let top = output.galley_pos.y + row.pos.y;
                let band = Rect::from_min_max(pos2(field.left(), top), pos2(field.right(), top + row.rect().height()));
                painter.set(line_light, Shape::rect_filled(band, 0.0, p.text.gamma_multiply(0.05)));
            }
            if settings.show_whitespace {
                let ink = settings.palette(dark).color(Kind::Text).gamma_multiply(0.3);
                for row in &galley.rows {
                    let middle = output.galley_pos.y + row.pos.y + row.rect().height() / 2.0;
                    for glyph in &row.glyphs {
                        let left = output.galley_pos.x + row.pos.x + glyph.pos.x;
                        match glyph.chr {
                            ' ' => {
                                painter.circle_filled(pos2(left + glyph.advance_width / 2.0, middle), 1.0, ink);
                            }
                            '\t' => {
                                let (from, to) = (left + 2.0, left + glyph.advance_width - 2.0);
                                if to > from {
                                    painter.line_segment([pos2(from, middle), pos2(to, middle)], Stroke::new(1.0, ink));
                                    painter.line_segment(
                                        [pos2(to - 3.0, middle - 3.0), pos2(to, middle)],
                                        Stroke::new(1.0, ink),
                                    );
                                    painter.line_segment(
                                        [pos2(to - 3.0, middle + 3.0), pos2(to, middle)],
                                        Stroke::new(1.0, ink),
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            // The line numbers, the cursor's brighter.
            painter.rect_filled(gutter, 0.0, p.panel);
            for (index, row) in galley.rows.iter().enumerate() {
                let y = output.galley_pos.y + row.pos.y + row.rect().height() / 2.0;
                let ink = if current == Some(index) { p.text } else { p.subtle };
                painter.text(
                    pos2(gutter.right() - space::S4, y),
                    Align2::RIGHT_CENTER,
                    (index + 1).to_string(),
                    FontId::new(settings.size, FontFamily::Name(fonts::CODE.into())),
                    ink,
                );
            }
            (output, cursor.map(|cursor| cursor.index.0), gutter)
        })
        .inner;
    let (output, cursor, gutter) = inner;
    changed |= output.response.response.changed();
    ViewOutput {
        changed,
        response: output.response.response,
        galley: output.galley,
        galley_pos: output.galley_pos,
        cursor,
        gutter,
    }
}

/// The keys the field takes as a code editor, before it sees them: Tab puts in the indent of `settings` (when that
/// is spaces; a tab the field puts in itself), and Enter keeps the line's indentation. Returns whether `text` changed.
fn keys(ui: &Ui, id: Id, text: &mut String, settings: &CodeEditor) -> bool {
    if !ui.memory(|memory| memory.has_focus(id)) {
        return false;
    }
    let mut changed = false;
    if settings.insert_spaces && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)) {
        changed |= replace_selection(ui.ctx(), id, text, &settings.indent());
    }
    if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
        let indent = cursor(ui.ctx(), id).map_or_else(String::new, |at| indentation(text, at));
        changed |= replace_selection(ui.ctx(), id, text, &format!("\n{indent}"));
    }
    changed
}

/// The cursor of the field `id`, as a character index.
pub fn cursor(ctx: &egui::Context, id: Id) -> Option<usize> {
    TextEditState::load(ctx, id)?
        .cursor
        .char_range()
        .map(|range| range.primary.index.0)
}

/// The leading spaces and tabs of the line holding the character `at` of `text`.
pub fn indentation(text: &str, at: usize) -> String {
    let byte = byte_of(text, at);
    let start = text[..byte].rfind('\n').map_or(0, |newline| newline + 1);
    text[start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// The byte offset of the character `index` of `text` (its end, past the last).
pub fn byte_of(text: &str, index: usize) -> usize {
    text.char_indices().nth(index).map_or(text.len(), |(byte, _)| byte)
}

/// Puts `with` in place of what is selected in the field `id` (at its cursor, when nothing is), the cursor after it.
/// Returns whether there was a cursor.
pub fn replace_selection(ctx: &egui::Context, id: Id, text: &mut String, with: &str) -> bool {
    let Some(mut state) = TextEditState::load(ctx, id) else {
        return false;
    };
    let Some(range) = state.cursor.char_range() else {
        return false;
    };
    let (from, to) = (
        range.primary.index.0.min(range.secondary.index.0),
        range.primary.index.0.max(range.secondary.index.0),
    );
    replace_chars(ctx, id, &mut state, text, from..to, with);
    true
}

/// Puts `with` in place of the characters `range` of `text` in the field `id`, the cursor after it.
pub fn replace_chars(
    ctx: &egui::Context,
    id: Id,
    state: &mut TextEditState,
    text: &mut String,
    range: std::ops::Range<usize>,
    with: &str,
) {
    let (start, end) = (byte_of(text, range.start), byte_of(text, range.end));
    text.replace_range(start..end, with);
    let at = range.start + with.chars().count();
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
    state.clone().store(ctx, id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_s_indentation_and_byte_offsets() {
        let text = "a\n\t  b = 1\nc";
        assert_eq!(indentation(text, 6), "\t  ");
        assert_eq!(indentation(text, 0), "");
        assert_eq!(indentation(text, 12), "");
        assert_eq!(byte_of("héllo", 2), 3);
        assert_eq!(byte_of("hé", 9), 3);
    }
}
