//! The file dialogs the projects page asks for, behind a trait: the shell gives Studio the system's own
//! (`rfd`, in `app::shell`), and tests and screenshots a scripted one, so no test ever opens a real dialog.

use std::path::{Path, PathBuf};

/// Asks the user for a file or a folder. Each call blocks until the user answers; `None` is a cancel.
pub trait FileDialog {
    /// An existing `.mtproj` to add.
    fn pick_project(&mut self) -> Option<PathBuf>;
    /// A folder for a new project, starting in `start` when there is one.
    fn pick_folder(&mut self, start: Option<&Path>) -> Option<PathBuf>;
}

/// A dialog that never shows and always answers "cancelled": Studio's until the shell gives it the system's.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDialog;

impl FileDialog for NoDialog {
    fn pick_project(&mut self) -> Option<PathBuf> {
        None
    }

    fn pick_folder(&mut self, _start: Option<&Path>) -> Option<PathBuf> {
        None
    }
}

/// A dialog that gives the answers it was handed, in order, and records what it was asked.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct ScriptedDialog {
    answers: std::rc::Rc<std::cell::RefCell<ScriptState>>,
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct ScriptState {
    /// The answers still to give.
    pub answers: std::collections::VecDeque<Option<PathBuf>>,
    /// What it was asked, in order: "project" or "folder".
    pub asked: Vec<&'static str>,
}

#[cfg(test)]
impl ScriptedDialog {
    /// A dialog that gives `answers`, in order, then cancels.
    pub(crate) fn new(answers: impl IntoIterator<Item = Option<PathBuf>>) -> Self {
        let dialog = Self::default();
        dialog.answers.borrow_mut().answers.extend(answers);
        dialog
    }

    /// What it was asked, in order.
    pub(crate) fn asked(&self) -> Vec<&'static str> {
        self.answers.borrow().asked.clone()
    }

    fn answer(&mut self, question: &'static str) -> Option<PathBuf> {
        let mut state = self.answers.borrow_mut();
        state.asked.push(question);
        state.answers.pop_front().flatten()
    }
}

#[cfg(test)]
impl FileDialog for ScriptedDialog {
    fn pick_project(&mut self) -> Option<PathBuf> {
        self.answer("project")
    }

    fn pick_folder(&mut self, _start: Option<&Path>) -> Option<PathBuf> {
        self.answer("folder")
    }
}
