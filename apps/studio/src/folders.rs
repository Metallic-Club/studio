//! Studio's folders on disk (#312): its own folder in the user's profile (`%APPDATA%\Metallic\Studio` on Windows)
//! with the icons in it, and the games folder in the user's documents (`Documents\Metallic`).
//!
//! On start ([`prepare`]), Studio renames a folder an older Studio made in lower case (`metallic`) to `Metallic`,
//! copies the icons built into it (#402) into its own folder (only the files missing there, so the user's edits are
//! kept; a folder in the older flat layout is first moved to the new one, #322), and makes the games folder, whose
//! games the projects page lists.

use std::collections::HashMap;

use std::path::{Path, PathBuf};

use metallic_engine::metallic_platform::{AppId, UserPaths};
use metallic_engine::metallic_project::PROJECT_EXTENSION;

use crate::STUDIO_APP_ID;
use crate::theme::icons::{Icon, builtin};

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
pub fn prepare() {
    if let Some(paths) = user_paths() {
        if cfg!(windows) {
            // The roaming and the local folders: `<root>\Metallic\Studio\config` and `<root>\Metallic\Studio\data`.
            for dir in [paths.config_dir(), paths.data_dir()] {
                if let Some(organization) = dir.ancestors().nth(2) {
                    capitalise(organization);
                }
            }
        }
        let to = paths.app_dir().join(ICONS);
        match install_icons(builtin::SHIPPED, &to) {
            Ok(0) => {}
            Ok(count) => log::info!("icons: copied {count} files to {}", to.display()),
            Err(error) => log::warn!("icons: could not copy them to {}: {error}", to.display()),
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

/// The shipped icons' files: each one's path in the icons folder (`/` between folders) and its bytes.
pub type Shipped = [(&'static str, &'static [u8])];

/// Installs the `shipped` icons into `to`: moves a folder in the older flat layout to the new one first
/// ([`migrate_flat_icons`]), then writes every file `to` does not have yet. Files already there are the user's and are
/// left as they are. Returns how many were written.
pub fn install_icons(shipped: &Shipped, to: &Path) -> std::io::Result<usize> {
    let migrated = migrate_flat_icons(shipped, to)?;
    let copied = write_missing(shipped, to)?;
    if let Some(manifest) = migrated {
        crate::settings::store::write_atomically(&to.join(builtin::MANIFEST), &manifest)?;
        log::info!("icons: moved {} to the new layout", to.display());
    } else {
        update_manifest(to)?;
    }
    Ok(copied)
}

/// Brings the user's `icons.json` in `to` up to date (#403): every icon and colour group Studio has now listed, in the
/// current layout, keeping what the user chose. Written only when that changes it; a file that cannot be read is
/// left alone (Studio uses its defaults, and says so).
pub fn update_manifest(to: &Path) -> std::io::Result<()> {
    let path = to.join(builtin::MANIFEST);
    let (Ok(text), Ok(theirs)) = (std::fs::read_to_string(&path), builtin::read_manifest(to)) else {
        return Ok(());
    };
    let current = builtin::manifest_json_keeping(Some(&theirs), &HashMap::new());
    if text.replace("\r\n", "\n") != current {
        crate::settings::store::write_atomically(&path, &current)?;
        log::info!("icons: brought {} up to date", path.display());
    }
    Ok(())
}

/// Clears an icons folder an older Studio made (`icons.json` format 1: every SVG at the top, #291) for the new layout
/// (#322): its SVGs and licence go, and the `icons.json` to write once the new files are in is returned. What the user
/// changed is kept: an icon pointed at a file of their own keeps it, and an SVG they edited in place stays where it
/// is, its icon pointed at it. `None`, touching nothing, when `to` is not such a folder.
pub fn migrate_flat_icons(shipped: &Shipped, to: &Path) -> std::io::Result<Option<String>> {
    let Ok(old) = builtin::read_manifest(to) else {
        return Ok(None);
    };
    if old.format >= 2 {
        return Ok(None);
    }
    let mut kept: HashMap<Icon, String> = HashMap::new();
    for (id, entry) in &old.icons {
        let Some(icon) = Icon::from_id(id) else {
            continue;
        };
        let flat = format!("{}.svg", icon.lucide());
        if entry.file != flat {
            // A file of the user's own.
            kept.insert(icon, entry.file.clone());
            continue;
        }
        let default = icon.default_file();
        let original = shipped
            .iter()
            .find(|(path, _)| *path == default)
            .map(|(_, bytes)| *bytes);
        let edited = match (std::fs::read(to.join(&flat)), original) {
            (Ok(theirs), Some(shipped)) => theirs != shipped,
            _ => false,
        };
        if edited {
            kept.insert(icon, flat);
        }
    }
    let keep: std::collections::HashSet<&str> = kept.values().map(String::as_str).collect();
    for icon in Icon::ALL {
        let flat = format!("{}.svg", icon.lucide());
        if !keep.contains(flat.as_str()) {
            remove_if_there(&to.join(flat))?;
        }
    }
    for file in ["LICENSE-Lucide.txt", "NOTICE.txt", builtin::MANIFEST] {
        remove_if_there(&to.join(file))?;
    }
    Ok(Some(builtin::manifest_json_keeping(None, &kept)))
}

/// Deletes the file at `path`, if there is one.
fn remove_if_there(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Writes every `shipped` file (sub-folders too) that `to` does not have yet. Returns how many were written.
fn write_missing(shipped: &Shipped, to: &Path) -> std::io::Result<usize> {
    let mut written = 0;
    for (path, bytes) in shipped {
        let target = to.join(path);
        if target.exists() {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, bytes)?;
        written += 1;
    }
    Ok(written)
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
        let to = dir.join("installed");
        let from: &Shipped = &[
            ("icons.json", b"{ shipped }"),
            ("box.svg", b"<svg/>"),
            ("extra/note.txt", b"x"),
        ];
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join("icons.json"), "{ the user's }").unwrap();
        assert_eq!(install_icons(from, &to).unwrap(), 2, "box.svg and extra/note.txt");
        assert_eq!(
            std::fs::read_to_string(to.join("icons.json")).unwrap(),
            "{ the user's }"
        );
        assert!(to.join("box.svg").is_file() && to.join("extra").join("note.txt").is_file());
        // Again: nothing left to copy, and a file that is not a manifest is left as it is.
        assert_eq!(install_icons(from, &to).unwrap(), 0);
        assert_eq!(
            std::fs::read_to_string(to.join("icons.json")).unwrap(),
            "{ the user's }"
        );
        // Into a folder that is not there yet: all of it, Studio's own icons too (#402).
        let fresh = dir.join("fresh");
        assert_eq!(install_icons(builtin::SHIPPED, &fresh).unwrap(), builtin::SHIPPED.len());
        assert!(fresh.join(Icon::ExplorerPart.default_file()).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_flat_folder_moves_to_the_new_layout_keeping_the_user_s_changes() {
        let dir = temp("flat");
        let to = dir.join("installed");
        std::fs::create_dir_all(&to).unwrap();
        let shipped = builtin::SHIPPED;
        let svg = |icon: Icon| builtin::shipped(&icon.default_file()).unwrap().to_vec();
        // What a format 1 Studio installed: every SVG at the top, as shipped...
        for icon in Icon::ALL {
            std::fs::write(to.join(format!("{}.svg", icon.lucide())), svg(*icon)).unwrap();
        }
        std::fs::write(to.join("LICENSE-Lucide.txt"), "ISC").unwrap();
        // ... but the part pointed at a file of the user's, and the folder icon edited in place.
        std::fs::write(to.join("mine.svg"), "<svg/>").unwrap();
        std::fs::write(to.join("folder.svg"), "<svg>edited</svg>").unwrap();
        let mut manifest = String::from(r#"{ "format": 1, "about": "", "icons": {"#);
        let entries: Vec<String> = Icon::ALL
            .iter()
            .map(|icon| {
                let file = if *icon == Icon::ExplorerPart {
                    "mine.svg".to_owned()
                } else {
                    format!("{}.svg", icon.lucide())
                };
                format!(r#""{}": {{ "file": "{file}" }}"#, icon.id())
            })
            .collect();
        manifest.push_str(&entries.join(","));
        manifest.push_str("} }");
        std::fs::write(to.join(builtin::MANIFEST), manifest).unwrap();

        install_icons(shipped, &to).unwrap();
        // Only the user's files are left at the top, beside icons.json and the new folders.
        let mut top: Vec<String> = std::fs::read_dir(&to)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        top.sort();
        assert_eq!(
            top,
            [
                "assets",
                "explorer",
                "folder.svg",
                "icons.json",
                "interface",
                "licenses",
                "mine.svg",
                "panels",
                "toolbar"
            ]
        );
        let new = builtin::read_manifest(&to).unwrap();
        assert_eq!(new.format, builtin::FORMAT);
        let file = |icon: Icon| new.icons[icon.id()].file.clone();
        assert_eq!(file(Icon::ExplorerPart), "mine.svg");
        assert_eq!(file(Icon::ExplorerFolder), "folder.svg");
        assert_eq!(file(Icon::ExplorerModel), Icon::ExplorerModel.default_file());
        assert!(to.join(Icon::ExplorerModel.default_file()).is_file());
        assert_eq!(new.icons[Icon::ExplorerPart.id()].color.as_deref(), Some("object"));
        // Migrated once: started again, nothing changes.
        assert_eq!(install_icons(shipped, &to).unwrap(), 0);
        assert_eq!(
            builtin::read_manifest(&to).unwrap().icons[Icon::ExplorerPart.id()].file,
            "mine.svg"
        );
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
