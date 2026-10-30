//! The "Recent games" list, stored as a small TOML file in the user's config folder.
//!
//! Loading never fails: a missing or corrupt file gives an empty list, and a failed save is only
//! logged, so a broken config can never stop Studio from starting.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many recent games are remembered.
pub const MAX_RECENT: usize = 10;

/// One remembered game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProject {
    /// Display name.
    pub name: String,
    /// The game folder.
    pub path: PathBuf,
}

#[derive(Default, Serialize, Deserialize)]
struct RecentFile {
    #[serde(default)]
    project: Vec<RecentProject>,
}

/// The list of recent games, newest first, backed by one file.
#[derive(Debug, Clone, Default)]
pub struct RecentProjects {
    file: Option<PathBuf>,
    items: Vec<RecentProject>,
}

/// Where Studio keeps its settings: env `METALLIC_STUDIO_CONFIG_DIR`, else `%APPDATA%\Metallic\Studio`,
/// else `$XDG_CONFIG_HOME/metallic/studio` or `~/.config/metallic/studio`. `None` if nothing is known.
pub fn config_dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var("METALLIC_STUDIO_CONFIG_DIR") {
        return Some(dir);
    }
    if let Some(appdata) = var("APPDATA") {
        return Some(appdata.join("Metallic").join("Studio"));
    }
    if let Some(xdg) = var("XDG_CONFIG_HOME") {
        return Some(xdg.join("metallic").join("studio"));
    }
    var("HOME").map(|home| home.join(".config").join("metallic").join("studio"))
}

impl RecentProjects {
    /// Loads the list from `<config dir>/recent.toml`.
    pub fn load_default() -> Self {
        Self::load(config_dir().map(|d| d.join("recent.toml")))
    }

    /// Loads the list from `file` (or an empty, unsaved list when `None`). Never fails.
    pub fn load(file: Option<PathBuf>) -> Self {
        let items = file
            .as_deref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|text| match toml::from_str::<RecentFile>(&text) {
                Ok(parsed) => Some(parsed.project),
                Err(e) => {
                    log::warn!("ignoring unreadable recent games file: {}", e.message());
                    None
                }
            })
            .unwrap_or_default();
        Self { file, items }
    }

    /// The remembered games, newest first.
    pub fn items(&self) -> &[RecentProject] {
        &self.items
    }

    /// Moves (or adds) a game to the top, trims the list and saves it.
    pub fn record(&mut self, name: &str, path: &Path) {
        self.items.retain(|p| p.path != path);
        self.items.insert(
            0,
            RecentProject {
                name: name.to_owned(),
                path: path.to_path_buf(),
            },
        );
        self.items.truncate(MAX_RECENT);
        self.save();
    }

    /// Writes the list; failures are logged, never returned.
    fn save(&self) {
        let Some(file) = &self.file else { return };
        let body = RecentFile {
            project: self.items.clone(),
        };
        let result = toml::to_string(&body)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                if let Some(parent) = file.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::write(file, text).map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            log::warn!("could not save recent games: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_empty_list() {
        let dir = tempfile::tempdir().expect("tmp");
        assert!(
            RecentProjects::load(Some(dir.path().join("nope.toml")))
                .items()
                .is_empty()
        );
    }

    #[test]
    fn corrupt_file_gives_empty_list() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("recent.toml");
        fs::write(&file, "[[project]\nname = ").expect("write");
        assert!(RecentProjects::load(Some(file)).items().is_empty());
    }

    #[test]
    fn record_dedupes_orders_and_round_trips() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("sub").join("recent.toml");
        let mut recent = RecentProjects::load(Some(file.clone()));
        recent.record("A", Path::new("/games/a"));
        recent.record("B", Path::new("/games/b"));
        recent.record("A again", Path::new("/games/a"));
        let names: Vec<_> = recent.items().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["A again", "B"]);
        let reloaded = RecentProjects::load(Some(file));
        assert_eq!(reloaded.items(), recent.items());
    }

    #[test]
    fn list_is_capped() {
        let mut recent = RecentProjects::load(None);
        for i in 0..(MAX_RECENT + 5) {
            recent.record("g", Path::new(&format!("/games/{i}")));
        }
        assert_eq!(recent.items().len(), MAX_RECENT);
    }
}
