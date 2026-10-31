//! The Watch pane: expressions evaluated in the selected frame each time the game pauses.
use eframe::egui::{self, RichText};

use super::DebugPanel;
use crate::editor::EditorState;
use crate::panel;
use crate::tokens::ui_tokens;

/// Draws the watch list and the field that adds to it.
pub fn ui(ui: &mut egui::Ui, panel_state: &mut DebugPanel, state: &mut EditorState, paused: bool) {
    panel::header(ui, "Watch", |_| {});
    let t = ui_tokens();
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("debug_watch")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for i in 0..panel_state.watches.len() {
                // Taken for the row and put back: no copy each frame.
                let expr = std::mem::take(&mut panel_state.watches[i]);
                ui.horizontal(|ui| {
                    if ui.small_button("×").on_hover_text("Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.label(RichText::new(&expr).font(t.mono()));
                    let value = if paused {
                        panel_state.value(state, &expr)
                    } else {
                        Err("not paused".into())
                    };
                    match value {
                        Ok(v) => ui.label(RichText::new(format!("= {v}")).font(t.mono())),
                        Err(e) => ui.label(RichText::new(&*e).color(if paused {
                            t.error
                        } else {
                            t.text_dim
                        })),
                    };
                });
                panel_state.watches[i] = expr;
            }
            let r = ui.add(
                egui::TextEdit::singleline(&mut panel_state.new_watch)
                    .hint_text("Add expression")
                    .font(t.mono())
                    .desired_width(f32::INFINITY),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let expr = panel_state.new_watch.trim().to_string();
                if !expr.is_empty() {
                    panel_state.watches.push(expr);
                }
                panel_state.new_watch.clear();
                r.request_focus();
            }
        });
    if let Some(i) = remove {
        panel_state.watches.remove(i);
    }
}
