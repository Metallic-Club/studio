//! The script editor (#528): a script's code in a tab of its own, edited in place.
//!
//! - **A tab per script.** A double click on a Script, LocalScript or ModuleScript in the Explorer opens its tab
//!   (`script:<index>:<generation>`, a tab per instance of the dock), titled with its name, where the last script tab
//!   was docked. Its panel is [`crate::panels::script`].
//! - **The code is the project's file.** A script's `Source` names a member of the project (`Scripts/Door.luau`);
//!   a script with none is given `Scripts/<Name>.luau` when it is opened. What is typed is written into the project
//!   as it is typed (in memory, as every unsaved edit is), so Play runs it and Save writes it. The editor keeps each
//!   file's text ([`Scripts`]) once it is read.
//! - **Renaming a script** takes a slow double click in the Explorer, the double click being the editor's.

pub mod highlight;
pub mod lexer;
pub mod settings;
pub mod view;

use std::collections::HashMap;

use metallic_engine::metallic_core::{DataModel, Id, Value};
use metallic_engine::metallic_project::Project;

use crate::editor::EditorState;
use crate::viewport::pick::is_a;

/// An instance that is a script.
pub type ScriptId = Id;

/// The script editor's panel id, the first part of a script tab's id.
pub const PANEL: &str = "script";

/// The folder a script with no `Source` is given its file in.
pub const FOLDER: &str = "Scripts";

/// The extension of a Luau file.
pub const EXTENSION: &str = ".luau";

/// What the script editor keeps: each open file's text, and the scripts asked to be opened.
#[derive(Debug, Default)]
pub struct Scripts {
    /// Each file read, by its project member, as edited.
    texts: HashMap<String, String>,
    /// Scripts whose tabs are asked for, for Studio to open after the frame.
    opening: Vec<Id>,
    /// The script tab clicked in last.
    pub focused: Option<Id>,
    /// The script whose field takes the keyboard next frame: one just opened.
    focus: Option<Id>,
}

impl Scripts {
    /// The text of `member`: as edited, or read from `project` the first time (empty for a file it lacks).
    pub fn text(&mut self, project: &mut Project, member: &str) -> &mut String {
        self.texts.entry(member.to_owned()).or_insert_with(|| {
            if project.contains(member) {
                project.read_text(member).unwrap_or_else(|error| {
                    log::warn!("scripts: could not read {member}: {error}");
                    String::new()
                })
            } else {
                String::new()
            }
        })
    }

    /// Takes the scripts whose tabs were asked for since the last call.
    pub fn take_opening(&mut self) -> Vec<Id> {
        std::mem::take(&mut self.opening)
    }

    /// Whether `script`'s field is to take the keyboard now (it was just opened); asked once.
    pub fn take_focus(&mut self, script: Id) -> bool {
        let take = self.focus == Some(script);
        if take {
            self.focus = None;
        }
        take
    }

    /// Asks for `script`'s tab, its field to take the keyboard.
    fn ask(&mut self, script: Id) {
        self.focus = Some(script);
        if !self.opening.contains(&script) {
            self.opening.push(script);
        }
    }
}

/// The tab id of `script`'s tab.
pub fn pane(script: Id) -> String {
    format!("{PANEL}:{}:{}", script.index(), script.generation())
}

/// The script a script tab's id names.
pub fn script_of(pane: &str) -> Option<Id> {
    let rest = pane.strip_prefix(PANEL)?.strip_prefix(':')?;
    let (index, generation) = rest.split_once(':')?;
    Some(Id::new(index.parse().ok()?, generation.parse().ok()?))
}

/// Whether `id` is in the game: an instance under its root (a deleted one is kept for undo, out of it).
pub fn in_game(dm: &DataModel, id: Id) -> bool {
    dm.contains(id)
        && dm
            .root()
            .is_some_and(|root| dm.is_descendant_of(id, root).unwrap_or(false))
}

/// Whether `id` is a script: a Script, LocalScript or ModuleScript (any `SourceContainer`).
pub fn is_script(dm: &DataModel, id: Id) -> bool {
    is_a(dm, id, "SourceContainer")
}

/// The project member `script`'s `Source` names; empty for none.
pub fn source(dm: &DataModel, script: Id) -> String {
    match dm.get_by_name(script, "Source") {
        Ok(Value::String(path)) => path.trim().to_owned(),
        _ => String::new(),
    }
}

/// What a new file holds: a ModuleScript's table, else a greeting, as Roblox Studio starts them.
fn starter(dm: &DataModel, script: Id) -> &'static str {
    if is_a(dm, script, "ModuleScript") {
        "local module = {}\n\nreturn module\n"
    } else {
        "print(\"Hello world!\")\n"
    }
}

/// A member for a new file of a script called `name`, in [`FOLDER`], that `taken` says is free: the name with
/// anything but letters, digits, `_` and `-` as `_`, and a number after it if the name is taken.
pub fn new_member(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut stem: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if stem.is_empty() {
        stem = "Script".to_owned();
    }
    let first = format!("{FOLDER}/{stem}{EXTENSION}");
    if !taken(&first) {
        return first;
    }
    (2..)
        .map(|n| format!("{FOLDER}/{stem}{n}{EXTENSION}"))
        .find(|member| !taken(member))
        .unwrap_or(first)
}

impl EditorState {
    /// Asks for the tab of the script `id` of the game the Explorer shows (#528). In a running game that is the
    /// editor's script with the same file. A script with no file is given one (one undo step), outside a game.
    pub fn open_script(&mut self, id: Id) {
        let id = match &self.play {
            None => id,
            Some(session) => {
                let member = source(session.player.engine().data_model(), id);
                let editors = self.dm.root().and_then(|root| self.dm.descendants(root).ok());
                let found = editors.and_then(|mut all| {
                    all.find(|&other| {
                        is_script(&self.dm, other) && !member.is_empty() && source(&self.dm, other) == member
                    })
                });
                match found {
                    Some(found) => found,
                    None => {
                        log::warn!("scripts: that script's file is not in the editor's game; stop the game to open it");
                        return;
                    }
                }
            }
        };
        if !is_script(&self.dm, id) {
            return;
        }
        let mut member = source(&self.dm, id);
        if member.is_empty() {
            if self.play.is_some() {
                log::warn!("scripts: a script is given its file outside a game; stop the game to open it");
                return;
            }
            let name = self.dm.name(id).unwrap_or_default().to_owned();
            let project = &self.project;
            member = new_member(&name, |member| project.contains(member));
            if let Err(error) = self
                .scene()
                .set_by_name(id, "Source", Value::String(member.clone().into()))
            {
                log::warn!("scripts: could not give {name} its file: {error}");
                return;
            }
        }
        if !self.project.contains(&member) {
            let text = starter(&self.dm, id);
            if let Err(error) = self.project.write(&member, text) {
                log::warn!("scripts: could not make {member}: {error}");
                return;
            }
        }
        self.scripts.ask(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_names_its_script() {
        let id = Id::new(12, 3);
        assert_eq!(pane(id), "script:12:3");
        assert_eq!(script_of(&pane(id)), Some(id));
        assert_eq!(crate::dock::panel_of(&pane(id)), PANEL);
        assert_eq!(script_of("script"), None);
        assert_eq!(script_of("viewport"), None);
        assert_eq!(script_of("script:x:1"), None);
    }

    #[test]
    fn a_new_file_is_named_after_its_script_and_free() {
        assert_eq!(new_member("Door", |_| false), "Scripts/Door.luau");
        assert_eq!(new_member("Open the door!", |_| false), "Scripts/Open_the_door_.luau");
        assert_eq!(new_member("  ", |_| false), "Scripts/Script.luau");
        let taken = ["Scripts/Door.luau", "Scripts/Door2.luau"];
        assert_eq!(
            new_member("Door", |member| taken.contains(&member)),
            "Scripts/Door3.luau"
        );
    }
}
