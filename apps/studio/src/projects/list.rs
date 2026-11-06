//! `projects.toml` in Studio's config folder: the projects the user has opened, most recent first (ADR 0009,
//! Screens).
//!
//! ```toml
//! version = 1
//!
//! [[project]]
//! path = "C:\\Users\\me\\Documents\\Metallic\\My Game.mtproj"
//! name = "My Game"
//! last_opened = 1762430400   # seconds since 1970-01-01 UTC
//! thumbnail = "..."          # optional, for later; kept as it is
//! ```
//!
//! - A missing file is an empty list; nothing is written until the list changes.
//! - A write goes to `projects.toml.tmp` and is renamed over the file, so a crash never leaves half a list.
//! - **A file that is not TOML is never overwritten:** it is renamed to `projects.toml.bak` (or `.bak.2`, ...), the
//!   list starts empty, and [`ProjectList::take_notice`] says so once. An entry without a path is skipped; a value of
//!   the wrong kind takes its default.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use metallic_engine::metallic_platform::{AppId, PlatformError, UserPaths};
use toml::{Table, Value};

use crate::settings::store::{backup_path, write_atomically};

/// The list's file name in Studio's config folder.
pub const PROJECTS_FILE: &str = "projects.toml";

/// The list's format version, written as `version`.
pub const FORMAT_VERSION: i64 = 1;

/// The first lines of every file written.
const HEADER: &str = "# Metallic Studio's projects list. Studio rewrites this file when you open, add or remove a \
                      project.\n# last_opened is in seconds since 1970-01-01 UTC.\n";

/// Where `app`'s projects list lives for the current user: `projects.toml` in its config folder.
pub fn projects_path(app: &AppId) -> Result<PathBuf, PlatformError> {
    Ok(UserPaths::for_app(app)?.config_dir().join(PROJECTS_FILE))
}

/// One project in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectEntry {
    /// The `.mtproj` file.
    pub path: PathBuf,
    /// The project's name, as its manifest said when it was last opened.
    pub name: String,
    /// When it was last opened, in seconds since 1970-01-01 UTC; `None` if never.
    pub last_opened: Option<u64>,
    /// Its thumbnail image, once Studio makes them (kept as the file says until then).
    pub thumbnail: Option<PathBuf>,
    /// Whether its file was missing when last looked at. Not saved.
    pub missing: bool,
}

impl ProjectEntry {
    /// An entry for the project at `path`, called `name`, never opened.
    pub fn new(path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            last_opened: None,
            thumbnail: None,
            missing: false,
        }
    }

    /// When it was last opened.
    pub fn last_opened_at(&self) -> Option<SystemTime> {
        self.last_opened
            .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)))
    }
}

/// The projects list, the file it came from, and what to tell the user about that file.
#[derive(Debug, Default)]
pub struct ProjectList {
    /// The file; `None` keeps the list in memory only.
    path: Option<PathBuf>,
    /// Whether the file may be written: not when it could not be read, or a damaged one could not be moved aside.
    writable: bool,
    /// The projects, most recently opened first.
    entries: Vec<ProjectEntry>,
    /// What the user should be told about the file, until it is taken.
    notice: Option<String>,
}

impl ProjectList {
    /// A list that lives in memory only: empty, never saved.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Reads the list at `path`. Never fails: see the module's documentation for a missing or damaged file. Each
    /// project's file is looked at, to mark the missing ones.
    pub fn load(path: PathBuf) -> Self {
        let mut list = Self {
            path: Some(path.clone()),
            writable: true,
            ..Self::default()
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return list,
            Err(error) => {
                log::warn!("projects: could not read {}: {error}", path.display());
                list.writable = false;
                list.notice = Some(format!(
                    "Studio could not read its projects list ({error}), so the list is empty and will not be saved \
                     this time. The file is {}.",
                    path.display()
                ));
                return list;
            }
        };
        let parsed = String::from_utf8(bytes)
            .map_err(|_| "it is not UTF-8 text".to_owned())
            .and_then(|text| text.parse::<Table>().map_err(|error| error.message().to_owned()));
        match parsed {
            Ok(document) => {
                list.entries = entries_of(&document);
                list.refresh_missing();
            }
            Err(reason) => list.set_aside(&path, &reason),
        }
        list
    }

    /// Moves a damaged file aside, so saving the list never overwrites it.
    fn set_aside(&mut self, path: &Path, reason: &str) {
        let Some(backup) = backup_path(path) else {
            self.writable = false;
            return;
        };
        let name = backup.file_name().map_or_else(
            || backup.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        match std::fs::rename(path, &backup) {
            Ok(()) => {
                log::warn!(
                    "projects: {} is damaged ({reason}); kept as {}, starting with an empty list",
                    path.display(),
                    backup.display()
                );
                self.notice = Some(format!(
                    "Studio's projects list was damaged ({reason}), so the list starts empty. Your projects are \
                     untouched: add them again with Add. The old list was kept as {name}, next to the new one."
                ));
            }
            Err(error) => {
                log::warn!(
                    "projects: {} is damaged ({reason}) and could not be moved aside: {error}",
                    path.display()
                );
                self.writable = false;
                self.notice = Some(format!(
                    "Studio's projects list was damaged ({reason}) and could not be kept aside ({error}), so the \
                     list is empty and will not be saved this time. The file is {}.",
                    path.display()
                ));
            }
        }
    }

    /// The projects, most recently opened first.
    pub fn entries(&self) -> &[ProjectEntry] {
        &self.entries
    }

    /// The project at `path`, if it is in the list.
    pub fn get(&self, path: &Path) -> Option<&ProjectEntry> {
        self.entries.iter().find(|entry| entry.path == path)
    }

    /// The file, unless the list lives in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// What the user should be told about the list's file; only the first call gets it.
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Records that the project at `path`, called `name`, was opened at `now`: it is added if new, and moves to the
    /// front.
    pub fn opened(&mut self, path: &Path, name: &str, now: SystemTime) {
        let mut entry = match self.entries.iter().position(|entry| entry.path == path) {
            Some(index) => self.entries.remove(index),
            None => ProjectEntry::new(path, name),
        };
        if !name.is_empty() {
            name.clone_into(&mut entry.name);
        }
        entry.last_opened = now.duration_since(UNIX_EPOCH).ok().map(|since| since.as_secs());
        entry.missing = false;
        self.entries.insert(0, entry);
    }

    /// Adds `entry` at the end, unless its path is already listed. Returns whether it was added.
    pub fn push(&mut self, entry: ProjectEntry) -> bool {
        if self.get(&entry.path).is_some() {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// Takes the project at `path` off the list (its file is left alone). Returns whether it was listed.
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.path != path);
        self.entries.len() != before
    }

    /// Marks the project at `path` as missing, or not.
    pub fn set_missing(&mut self, path: &Path, missing: bool) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.path == path) {
            entry.missing = missing;
        }
    }

    /// Looks at every project's file again and marks the missing ones.
    pub fn refresh_missing(&mut self) {
        for entry in &mut self.entries {
            entry.missing = !entry.path.is_file();
        }
    }

    /// Writes the list, unless it lives in memory or its file must not be written. Returns whether it wrote.
    pub fn save(&self) -> io::Result<bool> {
        let Some(path) = self.path.as_deref().filter(|_| self.writable) else {
            return Ok(false);
        };
        write_atomically(path, &format!("{HEADER}{}", document_of(&self.entries)))?;
        Ok(true)
    }

    /// [`ProjectList::save`], logging a failure: the list is in memory either way.
    pub fn save_or_log(&self) {
        if let Err(error) = self.save() {
            log::warn!("projects: could not save the list: {error}");
        }
    }
}

/// The entries of a parsed `projects.toml`, without duplicates. An entry without a path is skipped and logged.
fn entries_of(document: &Table) -> Vec<ProjectEntry> {
    let mut entries: Vec<ProjectEntry> = Vec::new();
    let Some(projects) = document.get("project") else {
        return entries;
    };
    let Some(projects) = projects.as_array() else {
        log::warn!("projects: `project` is not a list of tables; ignored");
        return entries;
    };
    for project in projects {
        let Some(table) = project.as_table() else {
            log::warn!("projects: an entry is not a table; skipped");
            continue;
        };
        let Some(path) = table
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
        else {
            log::warn!("projects: an entry has no path; skipped");
            continue;
        };
        let path = PathBuf::from(path);
        if entries.iter().any(|entry| entry.path == path) {
            continue;
        }
        let name = table
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| name_from_path(&path));
        entries.push(ProjectEntry {
            name,
            last_opened: table
                .get("last_opened")
                .and_then(Value::as_integer)
                .and_then(|seconds| u64::try_from(seconds).ok()),
            thumbnail: table
                .get("thumbnail")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            missing: false,
            path,
        });
    }
    entries
}

/// The file's document for `entries`.
fn document_of(entries: &[ProjectEntry]) -> Table {
    let mut document = Table::new();
    document.insert("version".into(), Value::Integer(FORMAT_VERSION));
    let projects = entries
        .iter()
        .map(|entry| {
            let mut table = Table::new();
            table.insert("path".into(), Value::String(entry.path.to_string_lossy().into_owned()));
            table.insert("name".into(), Value::String(entry.name.clone()));
            if let Some(seconds) = entry.last_opened.and_then(|seconds| i64::try_from(seconds).ok()) {
                table.insert("last_opened".into(), Value::Integer(seconds));
            }
            if let Some(thumbnail) = &entry.thumbnail {
                table.insert(
                    "thumbnail".into(),
                    Value::String(thumbnail.to_string_lossy().into_owned()),
                );
            }
            Value::Table(table)
        })
        .collect();
    document.insert("project".into(), Value::Array(projects));
    document
}

/// A project's name from its file name (`My Game.mtproj` is "My Game").
pub fn name_from_path(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.display().to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty folder of its own for `test`, and the list's path in it.
    fn file(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-studio-projects-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(PROJECTS_FILE)
    }

    #[test]
    fn a_missing_file_is_an_empty_list() {
        let path = file("missing");
        let list = ProjectList::load(path.clone());
        assert!(list.entries().is_empty());
        assert!(!path.exists(), "nothing is written by reading");
    }

    #[test]
    fn the_list_saves_and_loads_back_most_recent_first() {
        let path = file("round-trip");
        let dir = path.parent().unwrap().to_path_buf();
        let real = dir.join("Real.mtproj");
        std::fs::write(&real, "not really a zip, but there").unwrap();
        let mut list = ProjectList::load(path.clone());
        let at = |seconds| UNIX_EPOCH + Duration::from_secs(seconds);
        list.opened(&dir.join("Gone.mtproj"), "Gone", at(1_000));
        list.opened(&real, "Real", at(2_000));
        let mut later = ProjectEntry::new(dir.join("Later.mtproj"), "Later");
        later.thumbnail = Some(dir.join("later.png"));
        assert!(list.push(later));
        assert!(!list.push(ProjectEntry::new(&real, "Twice")), "a path is listed once");
        assert!(list.save().unwrap());
        assert!(!path.with_extension("toml.tmp").exists());

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Metallic Studio's projects list."), "{text}");
        assert!(
            text.contains("[[project]]") && text.contains("last_opened = 2000"),
            "{text}"
        );
        let again = ProjectList::load(path);
        let names: Vec<_> = again.entries().iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["Real", "Gone", "Later"]);
        assert_eq!(again.entries()[0].last_opened, Some(2_000));
        assert_eq!(again.entries()[0].last_opened_at(), Some(at(2_000)));
        assert_eq!(again.entries()[2].last_opened, None);
        assert_eq!(again.entries()[2].thumbnail, Some(dir.join("later.png")));
        // Missing files are marked on load.
        let missing: Vec<_> = again.entries().iter().map(|entry| entry.missing).collect();
        assert_eq!(missing, [false, true, true]);
    }

    #[test]
    fn opening_again_moves_a_project_to_the_front_and_remove_leaves_its_file() {
        let path = file("reopen");
        let dir = path.parent().unwrap().to_path_buf();
        let a = dir.join("A.mtproj");
        std::fs::write(&a, "a").unwrap();
        let mut list = ProjectList::load(path);
        list.opened(&a, "A", UNIX_EPOCH + Duration::from_secs(10));
        list.opened(&dir.join("B.mtproj"), "B", UNIX_EPOCH + Duration::from_secs(20));
        list.opened(&a, "A renamed", UNIX_EPOCH + Duration::from_secs(30));
        assert_eq!(list.entries().len(), 2);
        assert_eq!(list.entries()[0].name, "A renamed");
        assert_eq!(list.entries()[0].last_opened, Some(30));

        assert!(list.remove(&a));
        assert!(!list.remove(&a));
        assert!(a.is_file(), "removing from the list never deletes the project");
        assert_eq!(list.entries().len(), 1);
    }

    #[test]
    fn a_corrupt_file_is_kept_as_bak_and_the_list_starts_empty() {
        let path = file("corrupt");
        let damaged = "[[project]\npath = ";
        std::fs::write(&path, damaged).unwrap();
        let mut list = ProjectList::load(path.clone());
        assert!(list.entries().is_empty());
        let notice = list.take_notice().expect("the user is told");
        assert!(notice.contains("projects.toml.bak"), "{notice}");
        assert!(list.take_notice().is_none(), "told once");
        assert_eq!(
            std::fs::read_to_string(path.with_extension("toml.bak")).unwrap(),
            damaged
        );
        assert!(!path.exists());

        // The next save writes a fresh list beside the backup.
        list.opened(Path::new("x.mtproj"), "X", SystemTime::now());
        assert!(list.save().unwrap());
        assert_eq!(ProjectList::load(path.clone()).entries().len(), 1);
        assert_eq!(
            std::fs::read_to_string(path.with_extension("toml.bak")).unwrap(),
            damaged
        );
    }

    #[test]
    fn bad_entries_are_skipped_and_wrong_kinds_take_defaults() {
        let path = file("entries");
        std::fs::write(
            &path,
            "version = 1\n[[project]]\nname = \"no path\"\n[[project]]\npath = \"Kept.mtproj\"\nlast_opened = \
             \"yesterday\"\n[[project]]\npath = \"Kept.mtproj\"\n",
        )
        .unwrap();
        let list = ProjectList::load(path);
        assert_eq!(list.entries().len(), 1);
        let entry = &list.entries()[0];
        assert_eq!(entry.name, "Kept", "named after its file");
        assert_eq!(entry.last_opened, None);
        assert!(entry.missing);
    }

    #[test]
    fn an_in_memory_list_is_never_written() {
        let mut list = ProjectList::in_memory();
        list.opened(Path::new("a.mtproj"), "A", SystemTime::now());
        assert!(!list.save().unwrap());
        assert!(list.path().is_none());
    }
}
