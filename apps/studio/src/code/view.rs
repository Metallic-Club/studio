//! The code field (#528-#530): a script's text in the Code Editor's font and colours, with the line numbers down its
//! left (striped by line, #540), the cursor's line lit, whitespace shown if asked, and Tab and Enter as a code editor
//! has them. It scrolls both ways: code is not wrapped.
//!
//! A click under the last line puts the cursor at the end of the code, as VS Code does (#557).
//!
//! Alt+click adds a cursor, and the keys act at every one (#558, [`super::cursors`]); Shift+Alt with a click or a drag
//! selects a column, a cursor on each line (#559). Selected code dragged elsewhere moves there, Ctrl copying it (#560).
//!
//! Enter after a line that opens a block steps in and, when the block is not closed, puts in its `end` (#542,
//! [`super::blocks`]).
//!
//! A colour written in the code (`Color3.fromRGB(...)` and the like) has a swatch before it that opens the colour
//! picker (#545, [`super::swatch`]).
//!
//! Errors and warnings (#530) colour their line's number and draw a line beside it (#539), are underlined with a wave,
//! and have a mark in the scroll bar, which goes to their line when clicked; each says what is wrong under the pointer.

use std::ops::Range;
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    Align2, Color32, FontFamily, FontId, Galley, Id, Key, Modifiers, Painter, Pos2, Rect, Response, ScrollArea, Sense,
    Shape, Stroke, TextBuffer, TextEdit, Ui, UiBuilder, pos2, vec2,
};

use super::analysis::{Diagnostic, Severity};
use super::cursors::{self, Caret, Motion};
use super::find::Finder;
use super::settings::{CodeEditor, Kind, SIZE};
use super::{highlight, minimap, swatch};
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
    /// The part of `view` left of the minimap and the scroll bar, which the code is not under (#556).
    pub code: Rect,
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

/// Draws `text` as code filling `ui`, the field interacting as `id`, marking `diagnostics` and lighting the matches
/// `found` of the find bar (#554: its finder, and the match gone to, lit brighter). With `escape`, the field keeps the
/// keyboard on Escape (a list over it closes instead).
pub fn show(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    settings: &CodeEditor,
    diagnostics: &[Diagnostic],
    found: Option<(&Finder, Option<Range<usize>>)>,
    escape: bool,
) -> ViewOutput {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let dark = colors.dark;
    let font = font(settings);
    let row_height = settings.size * settings.line_spacing;
    let digit = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
    let mut changed = keys(ui, id, text, settings);
    // Several cursors (#558): the field keeps Escape, which goes back to one.
    let several = !extras(ui.ctx(), id).is_empty();
    // The field's own cursor before it sees a click: Alt+click keeps it as another.
    let before = TextEditState::load(ui.ctx(), id).and_then(|state| state.cursor.char_range());
    // An edit made for the field (Enter, Tab, an end put in, a suggestion taken) moves its cursor without the field
    // knowing, so it would not scroll there itself: the view follows the cursor (#543).
    let follow = ui
        .ctx()
        .data_mut(|data| data.remove_temp::<bool>(id.with("follow")))
        .unwrap_or(false);
    let lines = text.split('\n').count();
    let digits = lines.to_string().len().max(2);
    let gutter_width = digits as f32 * digit + 2.0 * space::S4;
    // A place asked for: the cursor there, its line a third of the way down.
    let revealed = ui.ctx().data_mut(|data| data.remove_temp::<usize>(id.with("reveal")));
    // A selection made for the field (a match found, #554): scrolled into view if it is out of it.
    let selected = ui.ctx().data_mut(|data| data.remove_temp::<usize>(id.with("show")));
    let mut scroll = ScrollArea::both()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false]);
    // An offset the minimap asked for last frame (#556).
    if let Some(offset) = ui
        .ctx()
        .data_mut(|data| data.remove_temp::<f32>(id.with("minimap-offset")))
    {
        scroll = scroll.vertical_scroll_offset(offset);
    }
    // The minimap, and the scroll bar right of it, cover the code's right: the code can scroll out from under them.
    let bar = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_outer_margin + 2.0;
    let reserve = if settings.minimap { minimap::WIDTH + bar } else { 0.0 };
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
        let width = (widest + 4.0 * digit + reserve).max(view.x - gutter_width - MARK_WIDTH);
        let height = (lines as f32 * row_height).max(view.y);
        let top_left = ui.cursor().min;
        let gutter = Rect::from_min_size(top_left, vec2(gutter_width, height));
        let field = Rect::from_min_size(pos2(gutter.right(), top_left.y), vec2(width, height));
        ui.allocate_rect(gutter.union(field), Sense::hover());
        // Behind the text: the cursor's line and the matches found, filled in once the field is laid out.
        let line_light = ui.painter().add(Shape::Noop);
        let found_light = ui.painter().add(Shape::Noop);
        let extra_light = ui.painter().add(Shape::Noop);
        // The field is as tall as its lines; the view under them is `below_last_line`'s (#557).
        let rows = lines;
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
                        escape: escape || several,
                    })
                    .frame(egui::Frame::NONE)
                    .margin(vec2(space::S3, 0.0))
                    .desired_width(width)
                    .desired_rows(rows)
                    .layouter(&mut layouter)
                    .show(ui)
            })
            .inner;
        below_last_line(ui, id, field, &output.galley, output.galley_pos, text);
        let response = output.response.response.clone();
        alt_click(ui, id, &response, &output.galley, output.galley_pos, before);
        let dropped = drag_and_drop(ui, id, text, &output.galley, output.galley_pos);
        column_select(ui, id, field, &output.galley, output.galley_pos, before);
        if response.has_focus() {
            let (selections, carets) = extra_shapes(ui, id, &output.galley, output.galley_pos);
            ui.painter().set(extra_light, Shape::Vec(selections));
            ui.painter().extend(carets);
        }
        let focused = response.has_focus();
        let cursor = output.cursor_range.filter(|_| focused).map(|range| range.primary);
        let current = cursor.map(|cursor| output.galley.layout_from_cursor(cursor).row);
        let galley = Arc::clone(&output.galley);
        let galley_pos = output.galley_pos;
        if follow && let Some(cursor) = cursor {
            let at = galley.pos_from_cursor(cursor).translate(galley_pos.to_vec2());
            ui.scroll_to_rect(at.expand2(vec2(2.0 * digit, row_height / 2.0)), None);
        }
        if let Some(at) = selected {
            let at = galley.pos_from_cursor(CCursor::new(at)).translate(galley_pos.to_vec2());
            if !ui.clip_rect().contains_rect(at) {
                ui.scroll_to_rect(at.expand2(vec2(4.0 * digit, 0.0)), Some(egui::Align::Center));
            }
        }
        let painter = ui.painter().clone();
        // Where each line starts, for the marks below: worked out once (#561).
        let line_index = Lines::new(text);
        // Found in the text as it is now, after this frame's typing.
        let (found, gone_to) = found.map_or((Vec::new(), None), |(finder, at)| (finder.matches(text), at));
        if !found.is_empty() {
            let ink = severity_color(Severity::Warning, dark);
            let mut shapes = Vec::new();
            for range in &found {
                let current = gone_to.as_ref() == Some(range);
                let chars = line_index.char_of(text, range.start)..line_index.char_of(text, range.end);
                for rect in char_rects(&galley, galley_pos, chars) {
                    if current {
                        shapes.push(Shape::rect_filled(rect, 2.0, ink.gamma_multiply(0.45)));
                        shapes.push(Shape::rect_stroke(
                            rect,
                            2.0,
                            Stroke::new(1.0, ink),
                            egui::StrokeKind::Inside,
                        ));
                    } else {
                        shapes.push(Shape::rect_filled(rect, 2.0, ink.gamma_multiply(0.22)));
                    }
                }
            }
            painter.set(found_light, Shape::Vec(shapes));
        }
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
                (&galley, galley_pos),
                text,
                &line_index,
                diagnostic,
                severity_color(diagnostic.severity, dark),
            );
        }
        // The line numbers, the cursor's brighter, a line with a problem in its colour.
        let mut line_severity: Vec<Option<Severity>> = vec![None; galley.rows.len()];
        for diagnostic in diagnostics {
            let line = line_index.line_of(diagnostic.range.start);
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
            let messages = messages_on_line(text, &line_index, diagnostics, line);
            if !messages.is_empty() {
                gutter_response.on_hover_text(messages.join("\n"));
            }
        }
        // A swatch before each colour written (#545): a click opens the colour picker, which writes the colour back.
        let mut picked = false;
        let side = swatch_side(settings);
        for found in swatch::find(text) {
            let chars = line_index.char_of(text, found.start);
            let at = galley
                .pos_from_cursor(CCursor::new(chars))
                .translate(galley_pos.to_vec2());
            let rect = Rect::from_center_size(
                pos2(at.left() - SWATCH_GAP - side / 2.0, at.center().y),
                vec2(side, side),
            );
            if let Some(rgb) = swatch_ui(ui, id.with(("swatch", found.start)), rect, &found) {
                let with = swatch::write(&found, &text[found.args.clone()], rgb);
                text.replace_range(found.args.clone(), &with);
                picked = true;
                // The colours after it have moved; they show again next frame.
                break;
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
            picked || dropped,
        )
    });
    let view = shown.inner_rect;
    let offset = shown.state.clamped_offset().y;
    let content_height = shown.content_size.y;
    let (response, galley, galley_pos, cursor, hovered, picked) = shown.inner;
    changed |= response.changed() || picked;
    let mut code = view;
    if settings.minimap && view.width() > 2.0 * minimap::WIDTH {
        let rect = Rect::from_min_max(
            pos2(view.right() - bar - minimap::WIDTH, view.top()),
            pos2(view.right() - bar, view.bottom()),
        );
        code.max.x = rect.left();
        let map = minimap::Map {
            rect,
            lines,
            row_height,
            view_height: view.height(),
            offset,
            max_offset: content_height - view.height(),
        };
        let (minimap_response, asked) = minimap::interact(ui, id.with("minimap"), &map);
        let active = minimap_response.hovered() || minimap_response.dragged();
        minimap::paint(
            ui.painter(),
            &map,
            &galley.job,
            usize::from(settings.tab_width),
            (p.panel, p.text),
            active,
        );
        ui.painter().vline(
            rect.left(),
            view.y_range(),
            Stroke::new(1.0, p.border.gamma_multiply(0.5)),
        );
        if let Some(asked) = asked {
            ui.ctx()
                .data_mut(|data| data.insert_temp(id.with("minimap-offset"), asked));
            ui.ctx().request_repaint();
        }
    }
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
        code,
    }
}

/// The view under the code's last line, in the field `field` of the field `id` showing `text` laid out as `galley` at
/// `galley_pos` (#557): a click there puts the cursor at the end of the code, as VS Code does (Shift+click selects to
/// there), and a drag from there selects back from the end to the pointer.
fn below_last_line(ui: &Ui, id: Id, field: Rect, galley: &Galley, galley_pos: Pos2, text: &str) {
    let bottom = galley_pos.y + galley.rect.bottom();
    let below = Rect::from_min_max(pos2(field.left(), bottom), field.max);
    if below.height() < 1.0 {
        return;
    }
    let response = ui
        .interact(below, id.with("below"), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Text);
    let anchor_id = id.with("below-anchor");
    let end = CCursor::new(text.chars().count());
    let pressed = response.hovered() && ui.input(|input| input.pointer.primary_pressed());
    let range = if pressed {
        let shift = ui.input(|input| input.modifiers.shift);
        let anchor = TextEditState::load(ui.ctx(), id)
            .and_then(|state| state.cursor.char_range())
            .filter(|_| shift)
            .map_or(end, |range| range.secondary);
        ui.ctx().data_mut(|data| data.insert_temp(anchor_id, anchor.index.0));
        Some(CCursorRange::two(anchor, end))
    } else if response.dragged()
        && let Some(at) = response.interact_pointer_pos()
    {
        let anchor = ui
            .ctx()
            .data(|data| data.get_temp::<usize>(anchor_id))
            .map_or(end, CCursor::new);
        let to = if at.y >= bottom {
            end
        } else {
            galley.cursor_from_pos(at - galley_pos)
        };
        Some(CCursorRange::two(anchor, to))
    } else {
        None
    };
    // The field gives up the keyboard when a click ends on another widget: it is given it back.
    if response.clicked() || response.drag_stopped() {
        ui.memory_mut(|memory| memory.request_focus(id));
    }
    if let Some(range) = range {
        let mut state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        state.cursor.set_char_range(Some(range));
        state.store(ui.ctx(), id);
        ui.memory_mut(|memory| memory.request_focus(id));
        ui.ctx().request_repaint();
    }
}

/// The rectangles on the screen covering the characters `range` of the code `galley` lays out at `galley_pos`: one per
/// row it spans, a line's end showing as a space's width.
pub(crate) fn char_rects(galley: &Galley, galley_pos: Pos2, range: Range<usize>) -> Vec<Rect> {
    let (from, to) = (CCursor::new(range.start), CCursor::new(range.end.max(range.start)));
    let (first, last) = (galley.layout_from_cursor(from).row, galley.layout_from_cursor(to).row);
    let a = galley.pos_from_cursor(from);
    let b = galley.pos_from_cursor(to);
    let space = galley
        .rows
        .first()
        .and_then(|row| row.glyphs.first())
        .map_or(4.0, |glyph| glyph.advance_width.max(4.0));
    let mut rects = Vec::new();
    for row in first..=last {
        let Some(placed) = galley.rows.get(row) else {
            break;
        };
        let line = placed.rect();
        let left = if row == first { a.left() } else { line.left() };
        let right = if row == last { b.left() } else { line.right() + space };
        rects.push(Rect::from_min_max(
            pos2(left, line.top()),
            pos2(right.max(left), line.bottom()),
        ));
    }
    rects
        .into_iter()
        .map(|rect| rect.translate(galley_pos.to_vec2()))
        .collect()
}

/// Where each line of a text starts (#561), so a byte's line and character index are found without counting from the
/// text's start each time: a script with a problem on every line drew slowly, every problem counting its way there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    /// Each line's first byte.
    starts: Vec<usize>,
    /// Each line's first character's index.
    chars: Vec<usize>,
}

impl Lines {
    /// The lines of `text`.
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        let mut chars = vec![0];
        for (index, (byte, c)) in text.char_indices().enumerate() {
            if c == '\n' {
                starts.push(byte + 1);
                chars.push(index + 1);
            }
        }
        Self { starts, chars }
    }

    /// The line (from 0) holding byte `byte`.
    pub fn line_of(&self, byte: usize) -> usize {
        self.starts.partition_point(|&start| start <= byte).saturating_sub(1)
    }

    /// The index of the character at byte `byte` of `text`, the text these lines are of (its end, past the last).
    pub fn char_of(&self, text: &str, byte: usize) -> usize {
        let mut byte = byte.min(text.len());
        while !text.is_char_boundary(byte) {
            byte -= 1;
        }
        let line = self.line_of(byte);
        let start = self.starts.get(line).copied().unwrap_or(0).min(byte);
        self.chars.get(line).copied().unwrap_or(0) + text[start..byte].chars().count()
    }
}

/// Selects the characters `chars` in the field `id` without taking the keyboard, and scrolls them into view on its next
/// frame (a match the find bar went to, #554).
pub fn select(ctx: &egui::Context, id: Id, chars: Range<usize>) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(chars.start),
        CCursor::new(chars.end),
    )));
    state.store(ctx, id);
    ctx.data_mut(|data| data.insert_temp(id.with("show"), chars.end));
    ctx.request_repaint();
}

/// Puts the field `id`'s selection from `tail` to its cursor `head` (characters) without scrolling or taking the
/// keyboard: where Team Create moved it when others' edits or an undo changed the text (#631).
pub fn place_selection(ctx: &egui::Context, id: Id, tail: usize, head: usize) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(tail), CCursor::new(head))));
    state.store(ctx, id);
}

/// The field `id`'s selection as its two ends, the other end then the cursor, in characters.
pub fn selection_ends(ctx: &egui::Context, id: Id) -> Option<(usize, usize)> {
    let range = TextEditState::load(ctx, id)?.cursor.char_range()?;
    Some((range.secondary.index.0, range.primary.index.0))
}

/// What is selected in the field `id`, as characters, first to last (empty at the cursor).
pub fn selection(ctx: &egui::Context, id: Id) -> Option<Range<usize>> {
    let range = TextEditState::load(ctx, id)?.cursor.char_range()?;
    let (a, b) = (range.primary.index.0, range.secondary.index.0);
    Some(a.min(b)..a.max(b))
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

/// The space between a colour's swatch and the code on either side (#545).
const SWATCH_GAP: f32 = 3.0;

/// The side of a colour's swatch (#545) for code in `settings`: a little smaller than its letters.
pub fn swatch_side(settings: &CodeEditor) -> f32 {
    (settings.size * 0.8).round()
}

/// The room a colour's swatch takes before it in the code (#545).
pub fn swatch_room(settings: &CodeEditor) -> f32 {
    swatch_side(settings) + 2.0 * SWATCH_GAP
}

/// The swatch of the colour `found` at `rect`, interacting as `id` (#545): a click opens Studio's colour picker.
/// Returns the colour picked, when it changed.
fn swatch_ui(ui: &mut Ui, id: Id, rect: Rect, found: &swatch::Swatch) -> Option<[u8; 3]> {
    let p = current_colors(ui.ctx()).palette;
    let response = ui.interact(rect, id, Sense::click());
    let label = format!("Colour {}", crate::panels::color_picker::hex(found.rgb));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    let [r, g, b] = found.rgb;
    ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
    let ink = if response.hovered() { p.text } else { p.border };
    ui.painter()
        .rect_stroke(rect, 2.0, Stroke::new(1.0, ink), egui::StrokeKind::Inside);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut rgb = found.rgb;
    let mut changed = false;
    egui::Popup::menu(&response)
        .id(id.with("popup"))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            changed = crate::panels::color_picker::picker(ui, id.with("picker"), &mut rgb);
        });
    changed.then_some(rgb)
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

/// The messages of `diagnostics` on line `line` (from 0) of `text`, whose lines are `lines`.
fn messages_on_line(text: &str, lines: &Lines, diagnostics: &[Diagnostic], line: usize) -> Vec<String> {
    diagnostics
        .iter()
        .filter(|diagnostic| lines.line_of(diagnostic.range.start.min(text.len())) == line)
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
}

/// The marks of `diagnostics` along the scroll bar of the view `view`, each at its line's place in the code's `lines`;
/// a click on one goes to its line, and it tells what is wrong under the pointer.
fn marks(ui: &mut Ui, id: Id, view: Rect, lines: usize, text: &str, diagnostics: &[Diagnostic], dark: bool) {
    let index = Lines::new(text);
    // Each line with a problem once, by line: its worst problem and where the first starts.
    let mut by_line: std::collections::BTreeMap<usize, (Severity, usize)> = std::collections::BTreeMap::new();
    for diagnostic in diagnostics {
        let start = diagnostic.range.start.min(text.len());
        let line = index.line_of(start);
        let entry = by_line
            .entry(line)
            .or_insert_with(|| (diagnostic.severity, index.char_of(text, start)));
        entry.0 = entry.0.max(diagnostic.severity);
    }
    for (line, (severity, character)) in by_line {
        let y = view.top() + view.height() * (line as f32 + 0.5) / lines.max(1) as f32;
        let rect = Rect::from_center_size(pos2(view.right() - MARK_WIDTH / 2.0, y), vec2(MARK_WIDTH, 4.0));
        ui.painter().rect_filled(rect, 1.0, severity_color(severity, dark));
        let response = ui.interact(rect.expand2(vec2(2.0, 2.0)), id.with(("mark", line)), Sense::click());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Go to line {}", line + 1)));
        if response.clicked() {
            reveal(ui.ctx(), id, character);
        }
        let messages = messages_on_line(text, &index, diagnostics, line);
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
fn wave(
    painter: &Painter,
    (galley, galley_pos): (&Galley, Pos2),
    text: &str,
    lines: &Lines,
    diagnostic: &Diagnostic,
    color: Color32,
) {
    let start = diagnostic.range.start.min(text.len());
    let line_end = text[start..].find('\n').map_or(text.len(), |offset| start + offset);
    let end = diagnostic.range.end.clamp(start, line_end);
    let (from, to) = (lines.char_of(text, start), lines.char_of(text, end));
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
    let several = !extras(ui.ctx(), id).is_empty();
    if several {
        return several_keys(ui, id, text, settings);
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

/// The field `id`'s cursors besides its own (#558).
pub fn extras(ctx: &egui::Context, id: Id) -> Vec<Caret> {
    ctx.data(|data| data.get_temp::<Vec<Caret>>(id.with("cursors")))
        .unwrap_or_default()
}

/// Keeps `extras` as the field `id`'s cursors besides its own.
fn set_extras(ctx: &egui::Context, id: Id, extras: Vec<Caret>) {
    ctx.data_mut(|data| data.insert_temp(id.with("cursors"), extras));
}

/// The field `id`'s cursors, its own first (#558).
pub fn carets(ctx: &egui::Context, id: Id) -> Vec<Caret> {
    let own = TextEditState::load(ctx, id)
        .and_then(|state| state.cursor.char_range())
        .map_or(Caret::default(), |range| Caret {
            anchor: range.secondary.index.0,
            head: range.primary.index.0,
        });
    let mut all = vec![own];
    all.extend(extras(ctx, id));
    all
}

/// Makes `carets` (the field's own first) the field `id`'s cursors.
pub fn set_carets(ctx: &egui::Context, id: Id, carets: &[Caret]) {
    let Some((own, extras)) = carets.split_first() else {
        return;
    };
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(own.anchor),
        CCursor::new(own.head),
    )));
    state.store(ctx, id);
    set_extras(ctx, id, extras.to_vec());
}

/// A click in the field `response` (#558), its code laid out as `galley` at `galley_pos`: with Alt, its cursor `before`
/// the click stays as another, and the field's own goes where the click is; without, the field goes back to one cursor.
fn alt_click(ui: &Ui, id: Id, response: &Response, galley: &Galley, galley_pos: Pos2, before: Option<CCursorRange>) {
    let (pressed, alt, shift, at) = ui.input(|input| {
        (
            input.pointer.primary_pressed(),
            input.modifiers.alt,
            input.modifiers.shift,
            input.pointer.interact_pos(),
        )
    });
    if !pressed || !response.hovered() || shift {
        return;
    }
    if !alt {
        set_extras(ui.ctx(), id, Vec::new());
        return;
    }
    let (Some(before), Some(at)) = (before, at) else {
        return;
    };
    // Where the field puts its cursor for the click, once it takes it.
    let clicked = galley.cursor_from_pos(at - galley_pos).index.0;
    let mut all = vec![
        Caret::at(clicked),
        Caret {
            anchor: before.secondary.index.0,
            head: before.primary.index.0,
        },
    ];
    all.extend(extras(ui.ctx(), id));
    cursors::merge(&mut all);
    set_carets(ui.ctx(), id, &all);
}

/// Dragging the selection of the field `id` (#560), its code `text` laid out as `galley` at `galley_pos`: let go
/// elsewhere, the selected code moves there (Ctrl copies it instead) and stays selected; a mark shows where it goes
/// while dragging. A click on the selection, or a drop on it, puts the cursor there. Returns whether `text` changed.
fn drag_and_drop(ui: &Ui, id: Id, text: &mut String, galley: &Galley, galley_pos: Pos2) -> bool {
    let carets = carets(ui.ctx(), id);
    let [caret] = carets.as_slice() else {
        return false;
    };
    let selection = caret.range();
    if selection.is_empty() || !ui.memory(|memory| memory.has_focus(id)) {
        return false;
    }
    // The selection's rows take the pointer from the field.
    let responses: Vec<Response> = char_rects(galley, galley_pos, selection.clone())
        .into_iter()
        .enumerate()
        .map(|(row, rect)| ui.interact(rect, id.with(("drag", row)), Sense::click_and_drag()))
        .collect();
    let copy = ui.input(|input| input.modifiers.command);
    let over = |at: Pos2| galley.cursor_from_pos(at - galley_pos).index.0;
    let mut changed = false;
    for response in &responses {
        if response.dragged() {
            ui.ctx().set_cursor_icon(if copy {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::Grabbing
            });
            if let Some(at) = ui.input(|input| input.pointer.latest_pos()) {
                let mark = galley
                    .pos_from_cursor(CCursor::new(over(at)))
                    .translate(galley_pos.to_vec2());
                ui.painter().line_segment(
                    [mark.center_top(), mark.center_bottom()],
                    Stroke::new(2.0, current_colors(ui.ctx()).accent),
                );
            }
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
        }
        if (response.drag_stopped() || response.clicked())
            && let Some(at) = ui.input(|input| input.pointer.latest_pos())
        {
            let to = over(at);
            match moved(text, selection.clone(), to, copy) {
                Some(moved) => {
                    set_carets(ui.ctx(), id, &[moved]);
                    ui.ctx().data_mut(|data| data.insert_temp(id.with("follow"), true));
                    changed = true;
                }
                // A click, or a drop on the selection itself: the cursor there.
                None => set_carets(ui.ctx(), id, &[Caret::at(to)]),
            }
        }
        if response.clicked() || response.drag_stopped() {
            // The field gives up the keyboard when a click ends on another widget: it is given it back.
            ui.memory_mut(|memory| memory.request_focus(id));
            ui.ctx().request_repaint();
        }
    }
    changed
}

/// The characters `selection` of `text` dropped at character `to` (#560): moved there, or with `copy` copied there.
/// Returns the code's new selection, or nothing when dropped inside the selection (which does nothing).
pub fn moved(text: &mut String, selection: Range<usize>, to: usize, copy: bool) -> Option<Caret> {
    if (selection.start..=selection.end).contains(&to) {
        return None;
    }
    let mut chars: Vec<char> = text.chars().collect();
    let to = to.min(chars.len());
    // A selection kept from before the text changed can reach past its end.
    let selection = selection.start.min(chars.len())..selection.end.min(chars.len());
    let piece: Vec<char> = chars[selection.clone()].to_vec();
    let length = piece.len();
    let at = if copy {
        to
    } else {
        chars.drain(selection.clone());
        if to > selection.end { to - length } else { to }
    };
    chars.splice(at..at, piece);
    *text = chars.into_iter().collect();
    Some(Caret {
        anchor: at,
        head: at + length,
    })
}

/// Shift+Alt with a click or a drag in the field `field` (#559), its code laid out as `galley` at `galley_pos`: a
/// cursor on each line of the box, each selecting the box's part of its line, as VS Code's column selection. A click's
/// box is from the field's cursor `before` it to the click; a drag's, from where it started to the pointer. While
/// Shift+Alt is held this takes the pointer from the field.
fn column_select(ui: &Ui, id: Id, field: Rect, galley: &Galley, galley_pos: Pos2, before: Option<CCursorRange>) {
    let from_id = id.with("column-from");
    let press_id = id.with("column-press");
    let held = ui.input(|input| input.modifiers.alt && input.modifiers.shift);
    let from: Option<Pos2> = ui.ctx().data(|data| data.get_temp(from_id));
    if !held && from.is_none() {
        return;
    }
    let response = ui
        .interact(field, id.with("column"), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Text);
    let (pressed, released, at) = ui.input(|input| {
        (
            input.pointer.primary_pressed(),
            input.pointer.primary_released(),
            input.pointer.interact_pos(),
        )
    });
    let mut from = from;
    if pressed && response.hovered() {
        // From the cursor there was; where the press is, when there was none.
        let start = before.map_or_else(
            || at.map_or(Pos2::ZERO, |at| (at - galley_pos).to_pos2()),
            |range| galley.pos_from_cursor(range.primary).left_center(),
        );
        let press = at.map_or(start, |at| (at - galley_pos).to_pos2());
        ui.ctx().data_mut(|data| {
            data.insert_temp(from_id, start);
            data.insert_temp(press_id, press);
        });
        from = Some(start);
    }
    // Dragged, the box is from where the drag started.
    let dragging = response.dragged() || response.drag_stopped();
    if dragging && let Some(press) = ui.ctx().data(|data| data.get_temp::<Pos2>(press_id)) {
        from = Some(press);
    }
    if let (Some(from), Some(at)) = (from, at)
        && ((pressed && response.hovered()) || dragging || released)
    {
        let carets = column(galley, from, (at - galley_pos).to_pos2());
        set_carets(ui.ctx(), id, &carets);
        ui.ctx().request_repaint();
    }
    if released {
        ui.ctx().data_mut(|data| {
            data.remove::<Pos2>(from_id);
            data.remove::<Pos2>(press_id);
        });
        // The field gives up the keyboard when a click ends on another widget: it is given it back.
        ui.memory_mut(|memory| memory.request_focus(id));
    }
}

/// The cursors of a column selection in the code `galley` from `from` to `to` (both in the galley's space): one per
/// line between them, selecting from `from`'s side to `to`'s, the line `to` is on first (the field's own).
pub fn column(galley: &Galley, from: Pos2, to: Pos2) -> Vec<Caret> {
    let row_at = |y: f32| {
        galley
            .rows
            .iter()
            .position(|row| y < row.rect().bottom())
            .unwrap_or(galley.rows.len().saturating_sub(1))
    };
    let (first, last) = (row_at(from.y), row_at(to.y));
    let rows: Vec<usize> = if first <= last {
        (first..=last).rev().collect()
    } else {
        (last..=first).collect()
    };
    rows.into_iter()
        .filter_map(|row| {
            let y = galley.rows.get(row)?.rect().center().y;
            Some(Caret {
                anchor: galley.cursor_from_pos(vec2(from.x, y)).index.0,
                head: galley.cursor_from_pos(vec2(to.x, y)).index.0,
            })
        })
        .collect()
}

/// The field `id`'s other cursors as shapes, laid out as `galley` at `galley_pos`: their selections, to go behind the
/// text, and their carets, over it.
fn extra_shapes(ui: &Ui, id: Id, galley: &Galley, galley_pos: Pos2) -> (Vec<Shape>, Vec<Shape>) {
    let visuals = ui.visuals();
    let mut selections = Vec::new();
    let mut carets = Vec::new();
    for caret in extras(ui.ctx(), id) {
        let range = caret.range();
        if !range.is_empty() {
            for rect in char_rects(galley, galley_pos, range) {
                selections.push(Shape::rect_filled(rect, 0.0, visuals.selection.bg_fill));
            }
        }
        let at = galley
            .pos_from_cursor(CCursor::new(caret.head))
            .translate(galley_pos.to_vec2());
        carets.push(Shape::line_segment(
            [at.center_top(), at.center_bottom()],
            visuals.text_cursor.stroke,
        ));
    }
    (selections, carets)
}

/// The field `id`'s keys while it has several cursors (#558): typing, deleting, Enter, Tab, pasting, copying, cutting
/// and the arrows at every cursor, before the field sees them; Escape goes back to one cursor, and so do undo, redo and
/// select all, which the field does. Returns whether `text` changed.
fn several_keys(ui: &Ui, id: Id, text: &mut String, settings: &CodeEditor) -> bool {
    let mut all = carets(ui.ctx(), id);
    let events = ui.input(|input| input.events.clone());
    let mut taken = vec![false; events.len()];
    let mut changed = false;
    let mut one = false;
    for (index, event) in events.iter().enumerate() {
        let edit = match event {
            egui::Event::Text(typed) => Some(cursors::Edit::Insert(typed.clone())),
            egui::Event::Paste(pasted) => Some(cursors::paste(pasted, all.len())),
            egui::Event::Copy | egui::Event::Cut => {
                ui.ctx().copy_text(cursors::selections(text, &all).join("\n"));
                matches!(event, egui::Event::Cut).then(|| cursors::Edit::Insert(String::new()))
            }
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                let word = modifiers.command;
                let motion = match key {
                    Key::ArrowLeft => Some(Motion::Left { word }),
                    Key::ArrowRight => Some(Motion::Right { word }),
                    Key::ArrowUp => Some(Motion::Up),
                    Key::ArrowDown => Some(Motion::Down),
                    Key::Home => Some(Motion::Home),
                    Key::End => Some(Motion::End),
                    _ => None,
                };
                if let Some(motion) = motion {
                    cursors::motion(text, &mut all, motion, modifiers.shift);
                    taken[index] = true;
                    continue;
                }
                match key {
                    Key::Backspace => Some(cursors::Edit::Backspace { word }),
                    Key::Delete => Some(cursors::Edit::Delete { word }),
                    Key::Enter => Some(cursors::Edit::Newline),
                    Key::Tab if !modifiers.shift => Some(cursors::Edit::Insert(if settings.insert_spaces {
                        settings.indent()
                    } else {
                        "\t".to_owned()
                    })),
                    Key::Escape => {
                        one = true;
                        taken[index] = true;
                        continue;
                    }
                    // Undo, redo and select all are the field's, with one cursor.
                    Key::Z | Key::Y | Key::A if modifiers.command => {
                        one = true;
                        continue;
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };
        if let Some(edit) = edit {
            cursors::edit(text, &mut all, &edit);
            changed = true;
        }
        taken[index] = true;
    }
    if one {
        all.truncate(1);
    }
    set_carets(ui.ctx(), id, &all);
    if taken.iter().any(|taken| *taken) {
        ui.input_mut(|input| {
            let mut at = 0;
            input.events.retain(|_| {
                let keep = !taken.get(at).copied().unwrap_or(false);
                at += 1;
                keep
            });
        });
    }
    if changed {
        ui.ctx().data_mut(|data| data.insert_temp(id.with("follow"), true));
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

/// Puts `with` in place of the characters `range` of `text` in the field `id`, the cursor after it, and has the view
/// follow the cursor there on its next frame.
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
    ctx.data_mut(|data| data.insert_temp(id.with("follow"), true));
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

    /// #560: a selection dropped elsewhere moves there, or with Ctrl is copied there, and stays selected; dropped in
    /// itself it does nothing.
    #[test]
    fn a_dropped_selection_moves_or_is_copied() {
        let mut text = "alpha beta gamma".to_owned();
        assert_eq!(moved(&mut text, 6..10, 16, false), Some(Caret { anchor: 12, head: 16 }));
        assert_eq!(text, "alpha  gammabeta");
        assert_eq!(moved(&mut text, 12..16, 0, false), Some(Caret { anchor: 0, head: 4 }));
        assert_eq!(text, "betaalpha  gamma");
        assert_eq!(moved(&mut text, 0..4, 2, false), None);
        assert_eq!(moved(&mut text, 0..4, 4, false), None);
        assert_eq!(text, "betaalpha  gamma");
        assert_eq!(moved(&mut text, 0..4, 10, true), Some(Caret { anchor: 10, head: 14 }));
        assert_eq!(text, "betaalpha beta gamma");
        let mut accents = "é b".to_owned();
        assert_eq!(moved(&mut accents, 0..1, 3, false), Some(Caret { anchor: 2, head: 3 }));
        assert_eq!(accents, " bé");
        let mut short = "ab".to_owned();
        assert_eq!(moved(&mut short, 1..9, 0, false), Some(Caret { anchor: 0, head: 1 }));
        assert_eq!(short, "ba");
    }

    /// #561: a byte's line and character index, from the lines' starts.
    #[test]
    fn the_lines_find_a_byte_s_line_and_character() {
        let text = "ab\nçd\n\nef";
        let lines = Lines::new(text);
        assert_eq!(lines.line_of(0), 0);
        assert_eq!(lines.line_of(2), 0);
        assert_eq!(lines.line_of(3), 1);
        assert_eq!(lines.line_of(7), 2);
        assert_eq!(lines.line_of(text.len()), 3);
        for byte in 0..=text.len() {
            if text.is_char_boundary(byte) {
                assert_eq!(lines.char_of(text, byte), text[..byte].chars().count(), "byte {byte}");
            }
        }
        // Inside a character: its start.
        assert_eq!(lines.char_of(text, 4), 3);
        assert_eq!(lines.char_of(text, 99), text.chars().count());
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
        assert_eq!(messages_on_line(text, &Lines::new(text), &diagnostics, 1), ["one"]);
        assert_eq!(messages_on_line(text, &Lines::new(text), &diagnostics, 2), ["two"]);
        assert!(messages_on_line(text, &Lines::new(text), &diagnostics, 0).is_empty());
    }
}
