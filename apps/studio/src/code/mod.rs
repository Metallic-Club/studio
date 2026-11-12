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
//! - **Errors and warnings** (#530): each file is checked when it changes ([`Scripts::check`]), by the Luau compiler
//!   (syntax) and by the editor's analysis ([`analysis`]); the code field marks them.

pub mod analysis;
pub mod api;
pub mod blocks;
pub mod complete;
pub mod definition;
pub mod find;
pub mod find_bar;
pub mod highlight;
pub mod lexer;
pub mod settings;
pub mod swatch;
pub mod view;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

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
    /// Each file's analysis and diagnostics as last checked, by its member (#530).
    checked: HashMap<String, Checked>,
    /// Counts the edits to any file: a file is checked again when it or another changed (it may require another).
    edits: u64,
    /// The scripts' find bar (#554).
    pub find: find::FindBar,
    /// The search across every script (#555).
    pub search: find::Search,
}

/// A file's analysis and diagnostics, and what they were made from.
#[derive(Debug)]
struct Checked {
    /// The file's text's hash and the edit count when it was checked.
    key: (u64, u64),
    analysis: Rc<analysis::Analysis>,
    diagnostics: Rc<Vec<analysis::Diagnostic>>,
}

impl Scripts {
    /// The text of `member`: as edited, or read from `project` the first time (empty for a file it lacks).
    pub fn text(&mut self, project: &mut Project, member: &str) -> &mut String {
        read_text(&mut self.texts, project, member)
    }

    /// The text of `member` (as [`Scripts::text`]) and the find bar, together.
    pub fn text_and_find(&mut self, project: &mut Project, member: &str) -> (&mut String, &mut find::FindBar) {
        (read_text(&mut self.texts, project, member), &mut self.find)
    }

    /// The matches of `finder` in every script file of the game `dm` (#555), each file once, in the game's order.
    pub fn search(&mut self, project: &mut Project, dm: &DataModel, finder: &find::Finder) -> Vec<find::FileMatches> {
        let mut out: Vec<find::FileMatches> = Vec::new();
        for (script, member) in script_files(dm) {
            if out.iter().any(|file| file.member == member) {
                continue;
            }
            let text = read_text(&mut self.texts, project, &member);
            let matches: Vec<find::LineMatch> = finder
                .matches(text)
                .into_iter()
                .map(|range| find::line_match(text, range))
                .collect();
            if !matches.is_empty() {
                out.push(find::FileMatches {
                    name: dm.full_name(script).unwrap_or_default(),
                    member,
                    script,
                    matches,
                });
            }
        }
        out
    }

    /// Puts `text` in `member`'s place, keeping it in `project` as typed text is, and notes the edit.
    pub fn replace_text(&mut self, project: &mut Project, member: &str, text: String) {
        if let Err(error) = project.write(member, text.clone().into_bytes()) {
            log::warn!("scripts: could not keep {member}: {error}");
        }
        self.texts.insert(member.to_owned(), text);
        self.edited();
    }

    /// Notes that a file was edited: every file is checked again (#530).
    pub fn edited(&mut self) {
        self.edits += 1;
    }

    /// The analysis and diagnostics of `script`'s file `member` (#530), checked again only when a file changed: its
    /// syntax error by the Luau compiler, and the analysis' warnings.
    pub fn check(
        &mut self,
        project: &mut Project,
        dm: &DataModel,
        script: Id,
        member: &str,
    ) -> (Rc<analysis::Analysis>, Rc<Vec<analysis::Diagnostic>>) {
        let text = self.text(project, member).clone();
        let key = (hash(&text), self.edits);
        if let Some(checked) = self.checked.get(member).filter(|checked| checked.key == key) {
            return (Rc::clone(&checked.analysis), Rc::clone(&checked.diagnostics));
        }
        let code = Rc::new(analysis::Analysis::new(&text));
        let mut diagnostics = Vec::new();
        if let Err(error) = metallic_engine::metallic_luau::check(&text, member) {
            diagnostics.push(analysis::Diagnostic {
                range: line_range(&text, error.line.saturating_sub(1) as usize),
                severity: analysis::Severity::Error,
                message: error.message,
            });
        }
        let sources = Sources::new(&mut self.texts, project, dm);
        sources.with_env(Some(script), |env| diagnostics.extend(code.warnings(env)));
        let diagnostics = Rc::new(diagnostics);
        self.checked.insert(
            member.to_owned(),
            Checked {
                key,
                analysis: Rc::clone(&code),
                diagnostics: Rc::clone(&diagnostics),
            },
        );
        (code, diagnostics)
    }

    /// Runs `f` with the types' [`analysis::Env`] for `script`: the game `dm`, and the other scripts' code from this
    /// editor's texts and `project`.
    pub fn with_env<R>(
        &mut self,
        project: &mut Project,
        dm: &DataModel,
        script: Id,
        f: impl FnOnce(&analysis::Env<'_>) -> R,
    ) -> R {
        Sources::new(&mut self.texts, project, dm).with_env(Some(script), f)
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

/// Where the other scripts' code comes from, for `require` (#530): the editor's texts, or the project's files.
struct Sources<'a> {
    texts: RefCell<&'a mut HashMap<String, String>>,
    project: RefCell<&'a mut Project>,
    dm: &'a DataModel,
    /// Each ModuleScript's analysis, once made.
    modules: RefCell<HashMap<Id, Option<Rc<analysis::Analysis>>>>,
}

impl<'a> Sources<'a> {
    fn new(texts: &'a mut HashMap<String, String>, project: &'a mut Project, dm: &'a DataModel) -> Self {
        Self {
            texts: RefCell::new(texts),
            project: RefCell::new(project),
            dm,
            modules: RefCell::new(HashMap::new()),
        }
    }

    /// The analysis of the ModuleScript `id`'s code.
    fn module(&self, id: Id) -> Option<Rc<analysis::Analysis>> {
        if let Some(found) = self.modules.borrow().get(&id) {
            return found.clone();
        }
        let made = (is_a(self.dm, id, "ModuleScript"))
            .then(|| source(self.dm, id))
            .filter(|member| !member.is_empty())
            .map(|member| {
                let mut texts = self.texts.borrow_mut();
                let text = texts.entry(member.clone()).or_insert_with(|| {
                    let mut project = self.project.borrow_mut();
                    if project.contains(&member) {
                        project.read_text(&member).unwrap_or_default()
                    } else {
                        String::new()
                    }
                });
                Rc::new(analysis::Analysis::new(text))
            });
        self.modules.borrow_mut().insert(id, made.clone());
        made
    }

    fn with_env<R>(&self, script: Option<Id>, f: impl FnOnce(&analysis::Env<'_>) -> R) -> R {
        let module = |id: Id| self.module(id);
        let env = analysis::Env {
            dm: self.dm,
            script,
            module: &module,
        };
        f(&env)
    }
}

/// The text of `member` in `texts`: as edited, or read from `project` the first time (empty for a file it lacks).
fn read_text<'a>(texts: &'a mut HashMap<String, String>, project: &mut Project, member: &str) -> &'a mut String {
    texts.entry(member.to_owned()).or_insert_with(|| {
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

/// A hash of `text`, to tell whether it changed.
fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// The bytes of line `line` (from 0) of `text`, without its indentation: what a compiler error underlines.
pub fn line_range(text: &str, line: usize) -> std::ops::Range<usize> {
    let mut start = 0;
    for (index, piece) in text.split('\n').enumerate() {
        if index == line {
            let indent = piece.len() - piece.trim_start().len();
            let end = start + piece.trim_end().len();
            return (start + indent).min(end)..end;
        }
        start += piece.len() + 1;
    }
    text.len()..text.len()
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

/// Every script of the game `dm` that has a file, with its file, in the game's order.
pub fn script_files(dm: &DataModel) -> Vec<(Id, String)> {
    let Some(root) = dm.root() else {
        return Vec::new();
    };
    let Ok(all) = dm.descendants(root) else {
        return Vec::new();
    };
    all.filter(|&id| is_script(dm, id))
        .map(|id| (id, source(dm, id)))
        .filter(|(_, member)| !member.is_empty())
        .collect()
}

/// The editor's script a line of the Output was written from (#544): the one with its file, the one of its full name
/// first (two scripts can share a file).
pub fn script_at(dm: &DataModel, place: &crate::logging::ScriptPlace) -> Option<Id> {
    let root = dm.root()?;
    let mut found = None;
    for id in dm.descendants(root).ok()? {
        if is_script(dm, id) && source(dm, id) == place.file {
            if dm.full_name(id).is_ok_and(|name| name == place.script) {
                return Some(id);
            }
            found = found.or(Some(id));
        }
    }
    found
}

impl EditorState {
    /// Opens the tab of the script a line of the Output was written from, its cursor at the start of the line's code
    /// (#544). Returns whether the script was found.
    pub fn go_to_place(&mut self, ctx: &egui::Context, place: &crate::logging::ScriptPlace) -> bool {
        let Some(id) = script_at(&self.dm, place) else {
            return false;
        };
        self.open_own_script(id);
        let member = source(&self.dm, id);
        let text = self.scripts.text(&mut self.project, &member);
        let line = place.line.map_or(0, |line| line.saturating_sub(1) as usize);
        let start = line_range(text, line).start;
        let at = text[..start.min(text.len())].chars().count();
        view::reveal(ctx, crate::panels::script::field_id(id), at);
        true
    }

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
        self.open_own_script(id);
    }

    /// Asks for the tab of the editor's script `id`, giving it a file when it has none (outside a game).
    fn open_own_script(&mut self, id: Id) {
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
    fn a_line_s_range_leaves_its_indentation_out() {
        let text = "a\n  local x =\nend";
        assert_eq!(&text[line_range(text, 1)], "local x =");
        assert_eq!(&text[line_range(text, 2)], "end");
        assert_eq!(line_range(text, 9), text.len()..text.len());
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
