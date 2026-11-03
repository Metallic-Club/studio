//! What [`ScriptHost::start_game`](crate::ScriptHost::start_game) needs: the context that decides which script
//! classes run, and where script sources come from.

use std::collections::HashMap;
use std::sync::Arc;

use metallic_core::{ClassId, Registry};
use thiserror::Error;

/// Which side a host runs scripts for. It decides which script classes start; every context runs only scripts
/// inside services flagged `runs_scripts` (`Workspace`, `ServerScriptService`).
///
/// Client and server contexts arrive with networking (M10). Until then a game runs in one [`RunContext::Local`]
/// context, which runs both kinds; the parameter exists so M10 only adds the split.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RunContext {
    /// The server: `Script`s run, `LocalScript`s do not.
    Server,
    /// A player's client: `LocalScript`s run, `Script`s do not. (M10 adds the client-only containers, such as a
    /// player's GUI, where Roblox also runs them.)
    Client,
    /// A standalone game with no separate server: `Script`s and `LocalScript`s both run.
    Local,
}

impl RunContext {
    /// The name of the class whose instances (subclasses included) start in this context.
    pub const fn script_class(self) -> &'static str {
        match self {
            Self::Server => "Script",
            Self::Client => "LocalScript",
            Self::Local => "BaseScript",
        }
    }

    /// The class [`RunContext::script_class`] names, in `registry`.
    pub(crate) fn class_in(self, registry: &Registry) -> Option<ClassId> {
        registry.class_id(self.script_class())
    }
}

/// Why a source file could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("cannot load `{path}`: {message}")]
pub struct LoadError {
    /// The project-relative path that was asked for.
    pub path: String,
    /// What went wrong, for the output.
    pub message: String,
}

impl LoadError {
    /// The error for a path that names no file.
    pub fn not_found(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            message: "no such file in the project".to_owned(),
        }
    }
}

/// Where the host reads script sources from: a script's `Source` property is a project-relative path, such as
/// `Scripts/Door.luau`. Projects and files arrive in M4; until then tools and tests use [`MemorySources`].
pub trait SourceLoader {
    /// The text of the file at `path`.
    fn load(&self, path: &str) -> Result<Arc<str>, LoadError>;
}

/// Sources kept in memory by path: for tests, tools and games built in code.
#[derive(Clone, Debug, Default)]
pub struct MemorySources {
    files: HashMap<String, Arc<str>>,
}

impl MemorySources {
    /// No files.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) the file at `path`. Returns `self`, so files can be chained.
    pub fn with(mut self, path: &str, source: &str) -> Self {
        self.insert(path, source);
        self
    }

    /// Adds (or replaces) the file at `path`.
    pub fn insert(&mut self, path: &str, source: &str) {
        self.files.insert(path.to_owned(), Arc::from(source));
    }
}

impl SourceLoader for MemorySources {
    fn load(&self, path: &str) -> Result<Arc<str>, LoadError> {
        self.files.get(path).cloned().ok_or_else(|| LoadError::not_found(path))
    }
}
