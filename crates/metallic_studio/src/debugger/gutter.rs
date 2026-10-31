//! The script editor's gutter: line numbers, breakpoint dots and the paused-line arrow.
//!
//! Click toggles a breakpoint; right-click opens a menu to add a condition or a log message.
//! Hovering a name in the text while paused shows its value.
use eframe::egui::{
    self, Align2, Galley, Pos2, Rangef, Rect, Response, Sense, Shape, Stroke, Vec2,
};
use metallic_core::InstanceId;
use metallic_script::Breakpoint;

use super::DebugPanel;
use crate::editor::EditorState;
use crate::tokens::ui_tokens;

/// Width of the dot column left of the line numbers.
const DOT: f32 = 16.0;

/// The gutter's place and the script it belongs to, allocated before the text field.
pub struct Gutter {
    rect: Rect,
    response: Response,
    script: InstanceId,
}

/// Allocates the gutter for a script of `lines` lines, left of the text field. `height` is the
/// text field's height last frame, so wrapped lines (more rows than lines) stay inside it.
pub fn gutter(ui: &mut egui::Ui, script: InstanceId, lines: usize, height: f32) -> Gutter {
    let font = ui_tokens().mono();
    let (digit, row) = ui.fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
    let digits = (lines.max(999).ilog10() + 1) as f32;
    let size = Vec2::new(
        DOT + digit * digits + 8.0,
        (row * lines.max(30) as f32 + 8.0).max(height),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    Gutter {
        rect,
        response,
        script,
    }
}

/// One logical line on screen.
#[derive(Clone, Copy)]
pub struct LineRow {
    /// 1-based line number.
    pub line: u32,
    /// Screen rectangle of its first row, across the gutter and text.
    pub rect: Rect,
    /// Screen heights of all its rows (more than one when it wraps).
    pub span: Rangef,
}

/// Screen rows of each logical line of `galley` drawn at `pos`.
pub fn line_rows(galley: &Galley, pos: Pos2, left: f32) -> Vec<LineRow> {
    let mut out = Vec::new();
    let mut line = 1;
    let mut start = true;
    for row in &galley.rows {
        let r = row.rect().translate(pos.to_vec2());
        if start {
            out.push(LineRow {
                line,
                rect: Rect::from_x_y_ranges(left..=r.right().max(left + 1.0), r.y_range()),
                span: r.y_range(),
            });
        } else if let Some(last) = out.last_mut() {
            last.span.max = r.bottom();
        }
        start = row.ends_with_newline;
        if start {
            line += 1;
        }
    }
    out
}

impl Gutter {
    /// Paints numbers, dots and the paused arrow for `rows`, and handles clicks and the menu.
    pub fn show(
        self,
        ui: &mut egui::Ui,
        rows: &[LineRow],
        state: &mut EditorState,
        debug: &mut DebugPanel,
    ) {
        let t = ui_tokens();
        let font = t.mono();
        let painter = ui.painter_at(self.rect);
        let paused = debug.paused_line(self.script);
        let at = |p: Pos2| rows.iter().find(|r| r.span.contains(p.y)).map(|r| r.line);
        let hovered = self.response.hover_pos().and_then(at);
        let dot_x = self.rect.left() + DOT / 2.0;
        for r in rows {
            let y = r.rect.center().y;
            if r.span.max < self.rect.top() - 20.0 || y > self.rect.bottom() + 20.0 {
                continue;
            }
            let colour = if paused == Some(r.line) {
                t.text
            } else {
                t.text_dim
            };
            painter.text(
                Pos2::new(self.rect.right() - 4.0, y),
                Align2::RIGHT_CENTER,
                r.line.to_string(),
                font.clone(),
                colour,
            );
            let centre = Pos2::new(dot_x, y);
            let radius = (r.rect.height() * 0.3).clamp(3.0, 5.5);
            match state.debug.get(self.script, r.line) {
                Some(Breakpoint::Plain) => {
                    painter.circle_filled(centre, radius, t.error);
                }
                Some(Breakpoint::Condition(_)) => {
                    painter.circle_filled(centre, radius, t.error);
                    painter.circle_filled(centre, radius * 0.4, t.deep);
                }
                Some(Breakpoint::Log(_)) => {
                    let d = radius * 1.1;
                    painter.add(Shape::convex_polygon(
                        vec![
                            centre + Vec2::new(0.0, -d),
                            centre + Vec2::new(d, 0.0),
                            centre + Vec2::new(0.0, d),
                            centre + Vec2::new(-d, 0.0),
                        ],
                        t.error,
                        Stroke::NONE,
                    ));
                }
                None if hovered == Some(r.line) => {
                    painter.circle_filled(centre, radius, t.error.gamma_multiply(0.35));
                }
                None => {}
            }
            if paused == Some(r.line) {
                let h = radius * 1.2;
                painter.add(Shape::convex_polygon(
                    vec![
                        centre + Vec2::new(-h, -h),
                        centre + Vec2::new(h, 0.0),
                        centre + Vec2::new(-h, h),
                    ],
                    t.warning,
                    Stroke::new(1.0, t.deep),
                ));
            }
        }
        // The paused line's band across the text.
        if let Some(r) = paused.and_then(|l| rows.iter().find(|r| r.line == l)) {
            let band = Rect::from_x_y_ranges(
                self.rect.right()..=r.rect.right().max(ui.clip_rect().right()),
                r.span,
            );
            ui.painter()
                .rect_filled(band, 0.0, t.warning.gamma_multiply(0.12));
        }

        if self.response.clicked()
            && let Some(line) = self.response.interact_pointer_pos().and_then(at)
        {
            state.toggle_breakpoint(self.script, line);
        }
        let menu_id = self.response.id.with("line");
        if self.response.secondary_clicked()
            && let Some(line) = self.response.interact_pointer_pos().and_then(at)
        {
            ui.ctx().data_mut(|d| d.insert_temp(menu_id, line));
        }
        let script = self.script;
        self.response.context_menu(|ui| {
            let Some(line) = ui.ctx().data(|d| d.get_temp::<u32>(menu_id)) else {
                ui.close();
                return;
            };
            ui.label(egui::RichText::new(format!("Line {line}")).color(ui_tokens().text_dim));
            let has = state.debug.get(script, line).is_some();
            if ui
                .button(if has {
                    "Remove breakpoint"
                } else {
                    "Add breakpoint"
                })
                .clicked()
            {
                state.toggle_breakpoint(script, line);
                ui.close();
            }
            if ui.button("Condition…").clicked() {
                debug.edit_breakpoint(state, script, line, false);
                ui.close();
            }
            if ui.button("Log message…").clicked() {
                debug.edit_breakpoint(state, script, line, true);
                ui.close();
            }
        });
    }
}

/// The name or field chain under character `index` of `text` (`player.Character` when the
/// pointer is on `Character`), or `None` off a name. Keywords are not names.
pub fn name_at(text: &str, index: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    if !chars.get(index).copied().is_some_and(word) {
        return None;
    }
    let mut end = index;
    while chars.get(end).copied().is_some_and(word) {
        end += 1;
    }
    let mut start = index;
    while start > 0 && (word(chars[start - 1]) || chars[start - 1] == '.') {
        if chars[start - 1] == '.' && start >= 2 && chars[start - 2] == '.' {
            break;
        }
        start -= 1;
    }
    while start < end && (chars[start] == '.' || chars[start].is_ascii_digit()) {
        start += 1;
    }
    let s: String = chars[start..end].iter().collect();
    if s.is_empty()
        || s.split('.')
            .any(|p| p.is_empty() || crate::script_editor::is_keyword(p))
    {
        return None;
    }
    Some(s)
}

/// Shows the hovered name's value as a tooltip while paused, when `script` (the shown one) is
/// the selected frame's script.
pub fn hover(
    response: &Response,
    galley: &Galley,
    pos: Pos2,
    (script, text): (InstanceId, &str),
    state: &mut EditorState,
    debug: &mut DebugPanel,
) {
    if debug.paused_line(script).is_none() || !response.hovered() {
        return;
    }
    let Some(p) = response.hover_pos() else {
        return;
    };
    let cursor = galley.cursor_from_pos(p - pos);
    // Off the end of a line the cursor sits after its last character: only exact hits count.
    let Some(name) = name_at(text, cursor.index.0) else {
        return;
    };
    if let Some(value) = debug.hover_value(state, script, &name) {
        let tip = egui::RichText::new(format!("{name} = {value}"))
            .font(ui_tokens().mono())
            .color(ui_tokens().text);
        response.clone().on_hover_text_at_pointer(tip);
    }
}

#[cfg(test)]
mod tests {
    use super::name_at;

    #[test]
    fn hovered_names_follow_field_chains() {
        let src = "local hp = player.Character.Humanoid.Health -- end";
        let at = |needle: &str| src.find(needle).unwrap() + 1;
        assert_eq!(name_at(src, at("hp")).as_deref(), Some("hp"));
        assert_eq!(
            name_at(src, at("Humanoid")).as_deref(),
            Some("player.Character.Humanoid")
        );
        assert_eq!(name_at(src, at("player")).as_deref(), Some("player"));
        assert_eq!(name_at(src, at("local")), None, "keywords are not names");
        assert_eq!(name_at(src, at(" = ")), None);
        assert_eq!(name_at("a..b", 3).as_deref(), Some("b"));
        assert_eq!(name_at("x = 12", 4), None, "numbers are not names");
    }
}
