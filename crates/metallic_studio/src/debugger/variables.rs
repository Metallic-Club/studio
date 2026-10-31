//! The Variables pane: locals, upvalues and globals of the selected frame as a tree. Tables and
//! Instances expand; a local's value can be changed (right-click, Set value).
use eframe::egui::{self, RichText};
use metallic_script::{Scope, VarNode};

use super::DebugPanel;
use crate::editor::EditorState;
use crate::panel;
use crate::tokens::ui_tokens;

const SCOPES: [(Scope, &str, u8); 3] = [
    (Scope::Locals, "Locals", 0),
    (Scope::Upvalues, "Upvalues", 1),
    (Scope::Globals, "Globals", 2),
];

/// Draws the three scopes of the selected frame.
pub fn ui(ui: &mut egui::Ui, panel_state: &mut DebugPanel, state: &mut EditorState, paused: bool) {
    panel::header(ui, "Variables", |_| {});
    egui::ScrollArea::vertical()
        .id_salt("debug_variables")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !paused {
                ui.label(RichText::new("Not paused").color(ui_tokens().text_dim));
                return;
            }
            let frame = panel_state.frame;
            for (scope, title, key) in SCOPES {
                let nodes = panel_state.scope(state, frame, scope, key);
                egui::CollapsingHeader::new(format!("{title} ({})", nodes.len()))
                    .id_salt(("debug_scope", key))
                    .default_open(scope == Scope::Locals)
                    .show(ui, |ui| {
                        for node in &nodes {
                            row(ui, panel_state, state, node, scope == Scope::Locals, 0);
                        }
                    });
            }
        });
    panel_state.set_local_window(ui.ctx(), state);
}

fn row(
    ui: &mut egui::Ui,
    panel_state: &mut DebugPanel,
    state: &mut EditorState,
    node: &VarNode,
    editable: bool,
    depth: usize,
) {
    let t = ui_tokens();
    let text = RichText::new(format!("{} = {}", node.name, node.display)).font(t.mono());
    match node.children {
        Some(handle) if depth < 16 => {
            egui::CollapsingHeader::new(text)
                .id_salt(("debug_var", handle))
                .show(ui, |ui| {
                    for child in panel_state.expand(state, handle) {
                        row(ui, panel_state, state, &child, false, depth + 1);
                    }
                });
        }
        _ => {
            let r = ui.label(text);
            if editable {
                r.context_menu(|ui| {
                    if ui.button("Set value…").clicked() {
                        panel_state.begin_set_local(&node.name, &node.display);
                        ui.close();
                    }
                });
            }
        }
    }
}
