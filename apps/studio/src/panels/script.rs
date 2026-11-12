//! The script editor's panel (#528): one tab per script ([`crate::code`]), its code in the code field
//! ([`crate::code::view`]) in the Code Editor's settings (#529).

use egui::{Id, Ui};

use super::empty_note;
use crate::code::{self, PANEL};
use crate::dock::{Panel, PanelContext, Zone};
use crate::keymap::Context;
use crate::theme::icons::Icon;

/// The script editor.
#[derive(Clone, Copy, Debug)]
pub struct ScriptEditor;

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
        // What is wrong with the code (#530), checked again only when a file changed.
        let (_analysis, diagnostics) = editor.scripts.check(&mut editor.project, &editor.dm, script, &member);
        let text = editor.scripts.text(&mut editor.project, &member);
        let id = field_id(script);
        if focus {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        let output = code::view::show(ui, id, text, &cx.settings.code, &diagnostics);
        if output.changed {
            let bytes = text.clone().into_bytes();
            editor.scripts.edited();
            if let Err(error) = editor.project.write(&member, bytes) {
                log::warn!("scripts: could not keep {member}: {error}");
            }
        }
    }
}

/// The id of `script`'s code field.
pub fn field_id(script: code::ScriptId) -> Id {
    Id::unique("metallic.script").with(script)
}
