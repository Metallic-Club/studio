//! The script editor's panel (#528): one tab per script ([`crate::code`]), its code in a monospace field with the
//! line numbers down its left, scrolling both ways (code is not wrapped).

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, FontId, Galley, Id, Rect, ScrollArea, Sense, TextBuffer, TextEdit, Ui, UiBuilder, pos2, vec2};

use super::empty_note;
use crate::code::{self, PANEL};
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::current_colors;
use crate::theme::icons::Icon;
use crate::theme::tokens::space;

/// The script editor.
#[derive(Clone, Copy, Debug)]
pub struct ScriptEditor;

/// The code's size, in points.
const FONT_SIZE: f32 = 13.0;

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
        let text = editor.scripts.text(&mut editor.project, &member);
        let id = Id::unique("metallic.script").with(script);
        if focus {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        if code_view(ui, id, text) {
            let bytes = text.clone().into_bytes();
            if let Err(error) = editor.project.write(&member, bytes) {
                log::warn!("scripts: could not keep {member}: {error}");
            }
        }
    }
}

/// Draws `text` as code with the line numbers down its left, filling `ui`, the field interacting as `id`. Returns
/// whether it was edited.
pub fn code_view(ui: &mut Ui, id: Id, text: &mut String) -> bool {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let font = FontId::monospace(FONT_SIZE);
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let digit = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
    let lines = text.split('\n').count().max(1);
    let digits = lines.to_string().len().max(2);
    let gutter_width = digits as f32 * digit + 2.0 * space::S4;
    let job = |text: &str| {
        let mut job = LayoutJob::single_section(
            text.to_owned(),
            TextFormat {
                font_id: font.clone(),
                color: p.text,
                ..TextFormat::default()
            },
        );
        // Code is not wrapped: a long line scrolls.
        job.wrap.max_width = f32::INFINITY;
        job
    };
    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap: f32| -> Arc<Galley> {
        ui.fonts_mut(|fonts| fonts.layout_job(job(text.as_str())))
    };
    let mut changed = false;
    ScrollArea::both()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let widest = ui.fonts_mut(|fonts| fonts.layout_job(job(text.as_str())).size().x);
            let view = ui.available_size();
            let width = (widest + 4.0 * digit).max(view.x - gutter_width);
            let height = (lines as f32 * row_height).max(view.y);
            let top_left = ui.cursor().min;
            let gutter = Rect::from_min_size(top_left, vec2(gutter_width, height));
            let field = Rect::from_min_size(pos2(gutter.right(), top_left.y), vec2(width, height));
            ui.allocate_rect(gutter.union(field), Sense::hover());
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
            changed = output.response.changed();
            // The line numbers, the cursor's brighter.
            let current = output
                .cursor_range
                .map(|range| output.galley.layout_from_cursor(range.primary).row);
            let painter = ui.painter();
            painter.rect_filled(gutter, 0.0, p.panel);
            for (index, row) in output.galley.rows.iter().enumerate() {
                let y = output.galley_pos.y + row.pos.y + row.rect().height() / 2.0;
                let ink = if current == Some(index) { p.text } else { p.subtle };
                painter.text(
                    pos2(gutter.right() - space::S4, y),
                    Align2::RIGHT_CENTER,
                    (index + 1).to_string(),
                    font.clone(),
                    ink,
                );
            }
        });
    changed
}
