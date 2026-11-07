//! Studio's folders on disk (#312): its own folder in the user's profile (`%APPDATA%\Metallic\Studio` on Windows)
//! with the icons in it, and the games folder in the user's documents (`Documents\Metallic`).
//!
//! On start ([`prepare`]), Studio renames a folder an older Studio made in lower case (`metallic`) to `Metallic`,
//! copies the icons it ships beside its executable into its own folder (only the files missing there, so the user's
//! edits are kept), and makes the games folder, whose games the projects page lists.

use std::path::{Path, PathBuf};

use metallic_engine::metallic_platform::{AppId, UserPaths};
use metallic_engine::metallic_project::PROJECT_EXTENSION;

use crate::STUDIO_APP_ID;

/// The organization's folder, in the user's profile and in their documents.
pub const ORGANIZATION: &str = "Metallic";
/// The icons' folder in Studio's own folder.
pub const ICONS: &str = "icons";

/// Studio's per-user folders, if the user has them.
pub fn user_paths() -> Option<UserPaths> {
    AppId::parse(STUDIO_APP_ID)
        .and_then(|app| UserPaths::for_app(&app))
        .ok()
}

/// Where Studio keeps the icons the user may edit: `icons` in its own folder (`%APPDATA%\Metallic\Studio\icons`).
pub fn icons_folder() -> Option<PathBuf> {
    user_paths().map(|paths| paths.app_dir().join(ICONS))
}

/// Where games go by default, and whose games the projects page lists: `Metallic` in the user's documents, or in
/// their home folder when they have no documents folder.
pub fn games_folder() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let documents = home.join("Documents");
    if documents.is_dir() { documents } else { home }.join(ORGANIZATION)
}

/// Gets Studio's folders ready, once at start: the organization's folder named `Metallic`, the icons copied into
/// Studio's own folder, and the games folder made. Each step that fails is logged and the rest go on.
pub fn prepare(shipped_icons: Option<&Path>) {
    if let Some(paths) = user_paths() {
        if cfg!(windows) {
            // The roaming and the local folders: `<root>\Metallic\Studio\config` and `<root>\Metallic\Studio\data`.
            for dir in [paths.config_dir(), paths.data_dir()] {
                if let Some(organization) = dir.ancestors().nth(2) {
                    capitalise(organization);
                }
            }
        }
        if let Some(from) = shipped_icons {
            let to = paths.app_dir().join(ICONS);
            match install_icons(from, &to) {
                Ok(0) => {}
                Ok(count) => log::info!("icons: copied {count} files to {}", to.display()),
                Err(error) => log::warn!("icons: could not copy them to {}: {error}", to.display()),
            }
        }
    }
    let games = games_folder();
    if let Err(error) = std::fs::create_dir_all(&games) {
        log::warn!("projects: could not make {}: {error}", games.display());
    }
}

/// Renames the folder `wanted` names, if it exists under another letter case (`metallic` made by an older Studio),
/// to exactly that name. Windows folders ignore case, so this only changes how it is shown.
pub fn capitalise(wanted: &Path) {
    let (Some(parent), Some(name)) = (wanted.parent(), wanted.file_name()) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let found = entry.file_name();
        if found != name && found.to_string_lossy().eq_ignore_ascii_case(&name.to_string_lossy()) {
            // Through a name of its own: a rename that changes only the case is not one on every file system.
            let step = parent.join(format!("{}.renaming", name.to_string_lossy()));
            let renamed = std::fs::rename(entry.path(), &step).and_then(|()| std::fs::rename(&step, wanted));
            match renamed {
                Ok(()) => log::info!("folders: renamed {} to {}", entry.path().display(), wanted.display()),
                Err(error) => log::warn!("folders: could not rename {}: {error}", entry.path().display()),
            }
            return;
        }
    }
}

/// Copies every file in `from` (sub-folders too) that `to` does not have yet. Files already there are the user's and
/// are left as they are. Returns how many were copied.
pub fn install_icons(from: &Path, to: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut copied = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copied += install_icons(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// The games in `folder`: every `.mtproj` in it or in a folder directly inside it, by path.
pub fn find_games(folder: &Path) -> Vec<PathBuf> {
    let is_project = |path: &Path| {
        path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case(PROJECT_EXTENSION))
    };
    let mut games = Vec::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return games;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_project(&path) {
            games.push(path);
        } else if path.is_dir()
            && let Ok(inner) = std::fs::read_dir(&path)
        {
            games.extend(
                inner
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| is_project(path)),
            );
        }
    }
    games.sort();
    games
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own under the system's temporary folder, empty.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-folders-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn icons_are_copied_without_touching_the_user_s() {
        let dir = temp("icons");
        let (from, to) = (dir.join("shipped"), dir.join("installed"));
        std::fs::create_dir_all(from.join("extra")).unwrap();
        std::fs::write(from.join("icons.json"), "{ shipped }").unwrap();
        std::fs::write(from.join("box.svg"), "<svg/>").unwrap();
        std::fs::write(from.join("extra").join("note.txt"), "x").unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join("icons.json"), "{ the user's }").unwrap();
        assert_eq!(install_icons(&from, &to).unwrap(), 2, "box.svg and extra/note.txt");
        assert_eq!(
            std::fs::read_to_string(to.join("icons.json")).unwrap(),
            "{ the user's }"
        );
        assert!(to.join("box.svg").is_file() && to.join("extra").join("note.txt").is_file());
        // Again: nothing left to copy.
        assert_eq!(install_icons(&from, &to).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lowercase_folder_is_renamed() {
        let dir = temp("case");
        std::fs::create_dir_all(dir.join("metallic").join("Studio")).unwrap();
        capitalise(&dir.join("Metallic"));
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Metallic"]);
        assert!(dir.join("Metallic").join("Studio").is_dir(), "its contents kept");
        // Already right, or missing: nothing happens.
        capitalise(&dir.join("Metallic"));
        capitalise(&dir.join("Elsewhere"));
        assert!(dir.join("Metallic").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn games_are_found_in_the_folder_and_one_level_in() {
        let dir = temp("games");
        std::fs::write(dir.join("Racer.mtproj"), "").unwrap();
        std::fs::create_dir_all(dir.join("Space Race")).unwrap();
        std::fs::write(dir.join("Space Race").join("Space Race.mtproj"), "").unwrap();
        std::fs::create_dir_all(dir.join("deep").join("deeper")).unwrap();
        std::fs::write(dir.join("deep").join("deeper").join("Hidden.mtproj"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        let games = find_games(&dir);
        assert_eq!(
            games,
            [
                dir.join("Racer.mtproj"),
                dir.join("Space Race").join("Space Race.mtproj")
            ]
        );
        assert!(find_games(&dir.join("nowhere")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_folders_are_named_metallic() {
        assert_eq!(
            games_folder().file_name().and_then(|name| name.to_str()),
            Some("Metallic")
        );
        if let Some(icons) = icons_folder() {
            assert!(icons.ends_with(ICONS));
            if cfg!(windows) {
                // %APPDATA%\Metallic\Studio\icons, with a capital M.
                let parts: Vec<String> = icons
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect();
                assert_eq!(parts[parts.len() - 3..], ["Metallic", "Studio", "icons"]);
            }
        }
    }
}
