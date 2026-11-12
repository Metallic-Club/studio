//! The code field (#528-#530): a script's text in the Code Editor's font and colours, with the line numbers down its
//! left (striped by line, #540), the cursor's line lit, whitespace shown if asked, and Tab and Enter as a code editor
//! has them. It scrolls both ways: code is not wrapped.
//!
//! Enter after a line that opens a block steps in and, when the block is not closed, puts in its `end` (#542,
//! [`super::blocks`]).
//!
//! Errors and warnings (#530) colour their line's number and draw a line beside it (#539), are underlined with a wave,
//! and have a mark in the scroll bar, which goes to their line when clicked; each says what is wrong under the pointer.

use std::sync::Arc;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    Align2, Color32, FontFamily, FontId, Galley, Id, Key, Modifiers, Painter, Pos2, Rect, Response, ScrollArea, Sense,
    Shape, Stroke, TextBuffer, TextEdit, Ui, UiBuilder, pos2, vec2,
};

use super::analysis::{Diagnostic, Severity};
use super::highlight;
use super::settings::{CodeEditor, Kind, SIZE};
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
    /// The byte of the text under the pointer, if it is over a character.
    pub hovered: Option<usize>,
    /// The area the code is shown in, on the screen.
    pub view: Rect,
}

/// The width of the scroll bar's marks.
const MARK_WIDTH: f32 = 4.0;

/// The font of code in `settings`.
pub fn font(settings: &CodeEditor) -> FontId {
    FontId::new(settings.size, FontFamily::Name(fonts::CODE.into()))
}

/// Zooms the code (#541): Ctrl+wheel with the pointer over the editor (`over`), and Ctrl+= (or Ctrl++), Ctrl+- and
/// Ctrl+0 while the field `id` has the keyboard, change the font size of `settings` (Ctrl+0 back to the default),
/// within [`SIZE`]. Being the setting, the size is kept with the Code Editor's settings. Returns whether it changed.
pub fn zoom(ui: &Ui, id: Id, settings: &mut CodeEditor, over: bool) -> bool {
    let before = settings.size;
    let mut size = before;
    if over {
        // Smooth: the wheel's steps come in over a few frames.
        size *= ui.input(|input| input.zoom_delta());
    }
    if ui.memory(|memory| memory.has_focus(id)) {
        ui.input_mut(|input| {
            if input.consume_key(Modifiers::COMMAND, Key::Equals) || input.consume_key(Modifiers::COMMAND, Key::Plus) {
                size = size.round() + 1.0;
            }
            if input.consume_key(Modifiers::COMMAND, Key::Minus) {
                size = size.round() - 1.0;
            }
            if input.consume_key(Modifiers::COMMAND, Key::Num0) {
                size = CodeEditor::default().size;
            }
        });
    }
    settings.size = size.clamp(*SIZE.start(), *SIZE.end());
    settings.size != before
}

/// The colour of `severity` on the theme (`dark` or light), as the Output's lines have it.
pub fn severity_color(severity: Severity, dark: bool) -> Color32 {
    use crate::panels::output::{ERROR, ERROR_LIGHT, WARNING, WARNING_LIGHT};
    match (severity, dark) {
        (Severity::Error, true) => ERROR,
        (Severity::Error, false) => ERROR_LIGHT,
        (Severity::Warning, true) => WARNING,
        (Severity::Warning, false) => WARNING_LIGHT,
    }
}

/// Asks the field `id` to put its cursor at character `at` and scroll to it, from its next frame.
pub fn reveal(ctx: &egui::Context, id: Id, at: usize) {
    ctx.data_mut(|data| data.insert_temp(id.with("reveal"), at));
    ctx.memory_mut(|memory| memory.request_focus(id));
    ctx.request_repaint();
}

/// Draws `text` as code filling `ui`, the field interacting as `id`, marking `diagnostics`. With `escape`, the field
/// keeps the keyboard on Escape (a list over it closes instead).
pub fn show(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    settings: &CodeEditor,
    diagnostics: &[Diagnostic],
    escape: bool,
) -> ViewOutput {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let dark = colors.dark;
    let font = font(settings);
    let row_height = settings.size * settings.line_spacing;
    let digit = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
    let mut changed = keys(ui, id, text, settings);
    let lines = text.split('\n').count();
    let digits = lines.to_string().len().max(2);
    let gutter_width = digits as f32 * digit + 2.0 * space::S4;
    // A place asked for: the cursor there, its line a third of the way down.
    let revealed = ui.ctx().data_mut(|data| data.remove_temp::<usize>(id.with("reveal")));
    let mut scroll = ScrollArea::both()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false]);
    if let Some(at) = revealed {
        let at = at.min(text.chars().count());
        if let Some(mut state) = TextEditState::load(ui.ctx(), id).or_else(|| Some(TextEditState::default())) {
            state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
            state.store(ui.ctx(), id);
        }
        let line = text.chars().take(at).filter(|c| *c == '\n').count();
        scroll = scroll.vertical_scroll_offset((line as f32 * row_height - ui.available_height() / 3.0).max(0.0));
    }
    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap: f32| -> Arc<Galley> {
        let job = highlight::layout_job(text.as_str(), settings, &font, dark);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let shown = scroll.show(ui, |ui| {
        let widest = layouter(ui, text, f32::INFINITY).size().x;
        let view = ui.available_size();
        let width = (widest + 4.0 * digit).max(view.x - gutter_width - MARK_WIDTH);
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
                    .event_filter(egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape,
                    })
                    .frame(egui::Frame::NONE)
                    .margin(vec2(space::S3, 0.0))
                    .desired_width(width)
                    .desired_rows(rows)
                    .layouter(&mut layouter)
                    .show(ui)
            })
            .inner;
        let response = output.response.response.clone();
        let focused = response.has_focus();
        let cursor = output.cursor_range.filter(|_| focused).map(|range| range.primary);
        let current = cursor.map(|cursor| output.galley.layout_from_cursor(cursor).row);
        let galley = Arc::clone(&output.galley);
        let galley_pos = output.galley_pos;
        let painter = ui.painter().clone();
        if settings.highlight_line
            && let Some(row) = current.and_then(|row| galley.rows.get(row))
        {
            let top = galley_pos.y + row.pos.y;
            let band = Rect::from_min_max(pos2(field.left(), top), pos2(field.right(), top + row.rect().height()));
            painter.set(line_light, Shape::rect_filled(band, 0.0, p.text.gamma_multiply(0.05)));
        }
        if settings.show_whitespace {
            whitespace(
                &painter,
                &galley,
                galley_pos,
                settings.palette(dark).color(Kind::Text).gamma_multiply(0.3),
            );
        }
        // The waves under what is wrong, worst last so it shows.
        let mut worst_first: Vec<&Diagnostic> = diagnostics.iter().collect();
        worst_first.sort_by_key(|diagnostic| diagnostic.severity);
        for diagnostic in &worst_first {
            wave(
                &painter,
                &galley,
                galley_pos,
                text,
                diagnostic,
                severity_color(diagnostic.severity, dark),
            );
        }
        // The line numbers, the cursor's brighter, a line with a problem in its colour.
        let mut line_severity: Vec<Option<Severity>> = vec![None; galley.rows.len()];
        for diagnostic in diagnostics {
            let line = text[..diagnostic.range.start.min(text.len())].matches('\n').count();
            if let Some(slot) = line_severity.get_mut(line) {
                *slot = (*slot).max(Some(diagnostic.severity));
            }
        }
        painter.rect_filled(gutter, 0.0, p.panel);
        let row_spans: Vec<(f32, f32)> = galley
            .rows
            .iter()
            .map(|row| (galley_pos.y + row.pos.y, row.rect().height()))
            .collect();
        for (rect, color) in stripes(gutter, &row_spans, p.rows()) {
            painter.rect_filled(rect, 0.0, color);
        }
        for (rect, color) in problem_lines(gutter, &row_spans, &line_severity, dark) {
            painter.rect_filled(rect, 0.0, color);
        }
        for (index, row) in galley.rows.iter().enumerate() {
            let y = galley_pos.y + row.pos.y + row.rect().height() / 2.0;
            let ink = match line_severity.get(index).copied().flatten() {
                Some(severity) => severity_color(severity, dark),
                None if current == Some(index) => p.text,
                None => p.subtle,
            };
            painter.text(
                pos2(gutter.right() - space::S4, y),
                Align2::RIGHT_CENTER,
                (index + 1).to_string(),
                font.clone(),
                ink,
            );
        }
        // A number with a problem tells it under the pointer.
        let gutter_response = ui.interact(gutter, id.with("gutter"), Sense::hover());
        if let Some(at) = gutter_response.hover_pos() {
            let line = ((at.y - galley_pos.y) / row_height).floor().max(0.0) as usize;
            let messages = messages_on_line(text, diagnostics, line);
            if !messages.is_empty() {
                gutter_response.on_hover_text(messages.join("\n"));
            }
        }
        // The byte under the pointer, when it is over text.
        let hovered = response
            .hover_pos()
            .filter(|at| galley.rect.translate(galley_pos.to_vec2()).expand(1.0).contains(*at))
            .map(|at| {
                let cursor = galley.cursor_from_pos(at - galley_pos);
                super::view::byte_of(text, cursor.index.0)
            });
        (
            response,
            galley,
            galley_pos,
            cursor.map(|cursor| cursor.index.0),
            hovered,
        )
    });
    let view = shown.inner_rect;
    let (response, galley, galley_pos, cursor, hovered) = shown.inner;
    changed |= response.changed();
    marks(ui, id, view, lines, text, diagnostics, dark);
    // What is wrong under the pointer.
    if let Some(at) = hovered {
        let here: Vec<&str> = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.range.start <= at && at < diagnostic.range.end.max(diagnostic.range.start + 1)
            })
            .map(|diagnostic| diagnostic.message.as_str())
            .collect();
        if !here.is_empty() {
            response.clone().on_hover_text_at_pointer(here.join("\n"));
        }
    }
    ViewOutput {
        changed,
        response,
        galley,
        galley_pos,
        cursor,
        hovered,
        view,
    }
}

/// The gutter's stripes (#540): each of its lines `rows` (top and height on the screen) across the gutter `gutter`,
/// alternately the two shades of the Explorer's and the Properties' rows, the first line darker. The code beside them
/// keeps its own ground.
fn stripes(gutter: Rect, rows: &[(f32, f32)], shades: crate::theme::tokens::RowShades) -> Vec<(Rect, Color32)> {
    rows.iter()
        .enumerate()
        .map(|(index, &(top, height))| {
            let rect = Rect::from_min_max(pos2(gutter.left(), top), pos2(gutter.right(), top + height));
            (rect, shades.fill(index))
        })
        .collect()
}

/// How thick the line beside the number of a line with a problem is (#539).
pub const PROBLEM_LINE: f32 = 2.0;

/// The lines beside the numbers of the lines with a problem (#539), for the gutter `gutter` whose lines are
/// `rows` (each its top and height on the screen) with the worst problem of each in `line_severity`: [`PROBLEM_LINE`]
/// wide, just inside the gutter's right edge (between the numbers and the code), in that problem's colour.
fn problem_lines(
    gutter: Rect,
    rows: &[(f32, f32)],
    line_severity: &[Option<Severity>],
    dark: bool,
) -> Vec<(Rect, Color32)> {
    let right = gutter.right() - space::S1;
    rows.iter()
        .zip(line_severity)
        .filter_map(|(&(top, height), severity)| {
            let severity = (*severity)?;
            let rect = Rect::from_min_max(pos2(right - PROBLEM_LINE, top), pos2(right, top + height));
            Some((rect, severity_color(severity, dark)))
        })
        .collect()
}

/// The messages of `diagnostics` on line `line` (from 0) of `text`.
fn messages_on_line(text: &str, diagnostics: &[Diagnostic], line: usize) -> Vec<String> {
    diagnostics
        .iter()
        .filter(|diagnostic| text[..diagnostic.range.start.min(text.len())].matches('\n').count() == line)
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
}

/// The marks of `diagnostics` along the scroll bar of the view `view`, each at its line's place in the code's `lines`;
/// a click on one goes to its line, and it tells what is wrong under the pointer.
fn marks(ui: &mut Ui, id: Id, view: Rect, lines: usize, text: &str, diagnostics: &[Diagnostic], dark: bool) {
    let mut by_line: Vec<(usize, Severity, usize)> = Vec::new();
    for diagnostic in diagnostics {
        let start = diagnostic.range.start.min(text.len());
        let line = text[..start].matches('\n').count();
        let character = text[..start].chars().count();
        match by_line.iter_mut().find(|(other, _, _)| *other == line) {
            Some(entry) => entry.1 = entry.1.max(diagnostic.severity),
            None => by_line.push((line, diagnostic.severity, character)),
        }
    }
    for (line, severity, character) in by_line {
        let y = view.top() + view.height() * (line as f32 + 0.5) / lines.max(1) as f32;
        let rect = Rect::from_center_size(pos2(view.right() - MARK_WIDTH / 2.0, y), vec2(MARK_WIDTH, 4.0));
        ui.painter().rect_filled(rect, 1.0, severity_color(severity, dark));
        let response = ui.interact(rect.expand2(vec2(2.0, 2.0)), id.with(("mark", line)), Sense::click());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Go to line {}", line + 1)));
        if response.clicked() {
            reveal(ui.ctx(), id, character);
        }
        let messages = messages_on_line(text, diagnostics, line);
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("Line {}: {}", line + 1, messages.join("\n")));
    }
}

/// A line under the bytes `range` of `text` (on its first line), as the field `output` laid it out: the name Ctrl+click
/// goes to the definition of (#532).
pub fn underline(painter: &Painter, output: &ViewOutput, text: &str, range: std::ops::Range<usize>, color: Color32) {
    let start = range.start.min(text.len());
    let end = range.end.clamp(start, text.len());
    let (from, to) = (text[..start].chars().count(), text[..end].chars().count());
    let left = output.galley.pos_from_cursor(CCursor::new(from));
    let right = output.galley.pos_from_cursor(CCursor::new(to));
    let y = output.galley_pos.y + left.bottom() - 1.0;
    painter.line_segment(
        [
            pos2(output.galley_pos.x + left.left(), y),
            pos2(output.galley_pos.x + right.left(), y),
        ],
        Stroke::new(1.0, color),
    );
}

/// A wave under the part of `diagnostic`'s range on its first line.
fn wave(painter: &Painter, galley: &Galley, galley_pos: Pos2, text: &str, diagnostic: &Diagnostic, color: Color32) {
    let start = diagnostic.range.start.min(text.len());
    let line_end = text[start..].find('\n').map_or(text.len(), |offset| start + offset);
    let end = diagnostic.range.end.clamp(start, line_end);
    let (from, to) = (text[..start].chars().count(), text[..end].chars().count());
    let left = galley.pos_from_cursor(CCursor::new(from));
    let right = galley.pos_from_cursor(CCursor::new(to.max(from)));
    let (x0, x1) = (
        galley_pos.x + left.left(),
        (galley_pos.x + right.left()).max(galley_pos.x + left.left() + 6.0),
    );
    let y = galley_pos.y + left.bottom() - 1.0;
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x < x1 {
        points.push(pos2(x, if up { y - 1.5 } else { y + 1.0 }));
        up = !up;
        x += 3.0;
    }
    points.push(pos2(x1, y));
    painter.add(Shape::line(points, Stroke::new(1.0, color)));
}

/// Faint dots for spaces and arrows for tabs.
fn whitespace(painter: &Painter, galley: &Galley, galley_pos: Pos2, ink: Color32) {
    for row in &galley.rows {
        let middle = galley_pos.y + row.pos.y + row.rect().height() / 2.0;
        for glyph in &row.glyphs {
            let left = galley_pos.x + row.pos.x + glyph.pos.x;
            match glyph.chr {
                ' ' => {
                    painter.circle_filled(pos2(left + glyph.advance_width / 2.0, middle), 1.0, ink);
                }
                '\t' => {
                    let (from, to) = (left + 2.0, left + glyph.advance_width - 2.0);
                    if to > from {
                        let stroke = Stroke::new(1.0, ink);
                        painter.line_segment([pos2(from, middle), pos2(to, middle)], stroke);
                        painter.line_segment([pos2(to - 3.0, middle - 3.0), pos2(to, middle)], stroke);
                        painter.line_segment([pos2(to - 3.0, middle + 3.0), pos2(to, middle)], stroke);
                    }
                }
                _ => {}
            }
        }
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
        changed |= enter(ui.ctx(), id, text, settings);
    }
    changed
}

/// Enter in the field `id`: after a line that opens a block, a line indented one step more and the block's `end` when
/// it is not closed (#542); otherwise a line with this one's indentation. Returns whether `text` changed.
fn enter(ctx: &egui::Context, id: Id, text: &mut String, settings: &CodeEditor) -> bool {
    let Some(mut state) = TextEditState::load(ctx, id) else {
        return false;
    };
    let Some(range) = state.cursor.char_range() else {
        return false;
    };
    let at = range.primary.index.0;
    if at == range.secondary.index.0
        && let Some(enter) = super::blocks::enter(text, byte_of(text, at), &settings.indent())
    {
        let from = text[..enter.replace.start].chars().count();
        let to = text[..enter.replace.end].chars().count();
        replace_chars(ctx, id, &mut state, text, from..to, &enter.with);
        let cursor = from + enter.with[..enter.cursor].chars().count();
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(cursor))));
        state.store(ctx, id);
        return true;
    }
    let indent = indentation(text, at);
    replace_selection(ctx, id, text, &format!("\n{indent}"))
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

    /// #540: the line numbers are striped by line, as the Explorer's and the Properties' rows, across the gutter only.
    #[test]
    fn the_line_numbers_are_striped() {
        let shades = crate::theme::Colors::default().palette.rows();
        let gutter = Rect::from_min_size(pos2(10.0, 100.0), vec2(40.0, 300.0));
        let rows = [(100.0, 20.0), (120.0, 20.0), (140.0, 20.0)];
        let stripes = stripes(gutter, &rows, shades);
        let fills: Vec<Color32> = stripes.iter().map(|(_, fill)| *fill).collect();
        assert_eq!(fills, [shades.darker, shades.brighter, shades.darker]);
        assert_ne!(shades.darker, shades.brighter);
        for ((rect, _), (top, height)) in stripes.iter().zip(rows) {
            assert_eq!((rect.left(), rect.right()), (gutter.left(), gutter.right()));
            assert_eq!((rect.top(), rect.height()), (top, height));
        }
    }

    /// #539: a line with an error or a warning has a line in its colour beside its number; the others none.
    #[test]
    fn a_line_with_a_problem_has_a_line_beside_its_number() {
        let gutter = Rect::from_min_size(pos2(10.0, 100.0), vec2(40.0, 300.0));
        let rows = [(100.0, 20.0), (120.0, 20.0), (140.0, 20.0)];
        let severities = [Some(Severity::Error), None, Some(Severity::Warning)];
        let lines = problem_lines(gutter, &rows, &severities, true);
        assert_eq!(lines.len(), 2);
        let (error, warning) = (lines[0], lines[1]);
        assert_eq!(error.0.width(), PROBLEM_LINE);
        assert_eq!((error.0.top(), error.0.bottom()), (100.0, 120.0));
        assert_eq!((warning.0.top(), warning.0.bottom()), (140.0, 160.0));
        assert!(error.0.right() <= gutter.right() && error.0.left() > gutter.right() - space::S4);
        assert_eq!(error.1, severity_color(Severity::Error, true));
        assert_eq!(warning.1, severity_color(Severity::Warning, true));
        let light = problem_lines(gutter, &rows, &severities, false);
        assert_eq!(light[0].1, severity_color(Severity::Error, false));
    }

    #[test]
    fn a_line_s_messages() {
        let text = "a\nb\nc";
        let diagnostics = [
            Diagnostic {
                range: 2..3,
                severity: Severity::Warning,
                message: "one".into(),
            },
            Diagnostic {
                range: 4..5,
                severity: Severity::Error,
                message: "two".into(),
            },
        ];
        assert_eq!(messages_on_line(text, &diagnostics, 1), ["one"]);
        assert_eq!(messages_on_line(text, &diagnostics, 2), ["two"]);
        assert!(messages_on_line(text, &diagnostics, 0).is_empty());
    }
}
