//! The Call Stack pane: the paused frames, innermost first. Clicking one selects it for
//! Variables, Watch and hover values, and shows its line in the script editor.
use eframe::egui::{self, RichText};
use metallic_script::Frame;

use crate::panel;
use crate::tokens::ui_tokens;

/// Draws `stack` with `selected` highlighted. Returns the frame clicked.
pub fn ui(ui: &mut egui::Ui, stack: &[Frame], selected: usize) -> Option<usize> {
    panel::header(ui, "Call Stack", |_| {});
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("debug_call_stack")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if stack.is_empty() {
                ui.label(RichText::new("Not paused").color(ui_tokens().text_dim));
            }
            for (i, f) in stack.iter().enumerate() {
                let text = RichText::new(format!("{}  {}:{}", f.function, f.chunk, f.line))
                    .font(ui_tokens().mono());
                if ui.selectable_label(i == selected, text).clicked() {
                    clicked = Some(i);
                }
            }
        });
    clicked
}
