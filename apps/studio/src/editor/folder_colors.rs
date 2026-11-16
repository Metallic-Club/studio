//! Folders' colours in the Assets browser (#741): a colour of the user's choosing for a project folder's icon in the
//! tree, the tiles and the list, set in the folder's inspector. They are Studio's, not the game's. They are kept in the
//! project as `.studio/browser.json` (the editor's own data, as the Explorer's `.studio/explorer.json` is, #603), by
//! each folder's path; they follow a folder when it or a folder it is in is renamed or moved, go with it when it is
//! deleted, and come along on its copies.

use std::collections::BTreeMap;

use metallic_engine::metallic_project::{EDITOR_DATA, Project};
use serde::{Deserialize, Serialize};

use super::EditorState;

/// The member the colours are kept in.
pub fn member() -> String {
    format!("{EDITOR_DATA}browser.json")
}

/// The folders' colours, by path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderColors {
    colors: BTreeMap<String, [u8; 3]>,
}

/// `.studio/browser.json`.
#[derive(Debug, Default, Deserialize, Serialize)]
struct File {
    /// Each folder's colour as `#rrggbb`, by its path.
    #[serde(default)]
    folder_colors: BTreeMap<String, String>,
}

/// Whether `path` is `folder` or inside it.
fn under(path: &str, folder: &str) -> bool {
    path == folder || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

impl FolderColors {
    /// The colour of `folder`, if it has one of its own.
    pub fn get(&self, folder: &str) -> Option<[u8; 3]> {
        self.colors.get(folder).copied()
    }

    /// The colours kept in `project`. A file that cannot be read gives none, and says why.
    pub fn load(project: &mut Project) -> Self {
        let member = member();
        if !project.contains(&member) {
            return Self::default();
        }
        let file: File = match project.read_text(&member).map(|text| serde_json::from_str(&text)) {
            Ok(Ok(file)) => file,
            Ok(Err(error)) => {
                log::warn!("assets: {member} could not be read, so folders have their usual colours: {error}");
                return Self::default();
            }
            Err(error) => {
                log::warn!("assets: {member} could not be read, so folders have their usual colours: {error}");
                return Self::default();
            }
        };
        let colors = file
            .folder_colors
            .into_iter()
            .filter_map(|(folder, hex)| Some((folder, crate::panels::color_picker::parse_hex(&hex)?)))
            .collect();
        Self { colors }
    }

    /// Writes the colours into `project`; with none, the member goes.
    fn write(&self, project: &mut Project) -> Result<(), String> {
        let member = member();
        if self.colors.is_empty() {
            project.remove(&member);
            return Ok(());
        }
        let folder_colors = self
            .colors
            .iter()
            .map(|(folder, &rgb)| (folder.clone(), crate::panels::color_picker::hex(rgb)))
            .collect();
        let text = serde_json::to_string_pretty(&File { folder_colors }).map_err(|error| error.to_string())?;
        project.write(&member, text).map_err(|error| error.to_string())
    }

    /// Moves the colours of `from` and the folders in it to `to`. Returns whether any moved.
    fn moved(&mut self, from: &str, to: &str) -> bool {
        let moving: Vec<String> = self.colors.keys().filter(|path| under(path, from)).cloned().collect();
        for path in &moving {
            if let Some(color) = self.colors.remove(path) {
                self.colors.insert(format!("{to}{}", &path[from.len()..]), color);
            }
        }
        !moving.is_empty()
    }

    /// Forgets the colours of `path` and the folders in it. Returns whether any went.
    fn removed(&mut self, path: &str) -> bool {
        let before = self.colors.len();
        self.colors.retain(|folder, _| !under(folder, path));
        self.colors.len() != before
    }

    /// Gives `to`, a copy of `from`, and the folders in it the colours of what they are copies of. Returns whether
    /// any were given.
    fn copied(&mut self, from: &str, to: &str) -> bool {
        let copies: Vec<(String, [u8; 3])> = self
            .colors
            .iter()
            .filter(|(path, _)| under(path, from))
            .map(|(path, &color)| (format!("{to}{}", &path[from.len()..]), color))
            .collect();
        let any = !copies.is_empty();
        self.colors.extend(copies);
        any
    }
}

impl EditorState {
    /// The colour of the project folder `folder`'s icon, if it has one of its own (#741).
    pub fn folder_color(&self, folder: &str) -> Option<[u8; 3]> {
        self.folder_colors.get(folder)
    }

    /// Gives the folder `folder`'s icon `color`, or its usual colour back with `None` (#741), written into the project
    /// at once (in memory until Save, as every edit is).
    pub fn set_folder_color(&mut self, folder: &str, color: Option<[u8; 3]>) -> Result<(), String> {
        match color {
            Some(color) => {
                self.folder_colors.colors.insert(folder.to_owned(), color);
            }
            None => {
                self.folder_colors.colors.remove(folder);
            }
        }
        self.folder_colors.write(&mut self.project)
    }

    /// Makes the colours follow a file or folder moved from `from` to `to`.
    pub(super) fn folder_colors_moved(&mut self, from: &str, to: &str) {
        if self.folder_colors.moved(from, to)
            && let Err(error) = self.folder_colors.write(&mut self.project)
        {
            log::warn!("assets: {error}");
        }
    }

    /// Forgets the colours of the files and folders `paths`, deleted.
    pub(super) fn folder_colors_removed(&mut self, paths: &[String]) {
        let mut any = false;
        for path in paths {
            any |= self.folder_colors.removed(path);
        }
        if any && let Err(error) = self.folder_colors.write(&mut self.project) {
            log::warn!("assets: {error}");
        }
    }

    /// Gives the copy `to` of `from` the colours of `from`'s folders.
    pub(super) fn folder_colors_copied(&mut self, from: &str, to: &str) {
        if self.folder_colors.copied(from, to)
            && let Err(error) = self.folder_colors.write(&mut self.project)
        {
            log::warn!("assets: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some() -> FolderColors {
        let mut colors = FolderColors::default();
        colors.colors.insert("Assets".into(), [1, 2, 3]);
        colors.colors.insert("Assets/Trees".into(), [4, 5, 6]);
        colors.colors.insert("Assets Old".into(), [7, 8, 9]);
        colors
    }

    #[test]
    fn colours_follow_moves_deletes_and_copies() {
        let mut colors = some();
        assert!(colors.moved("Assets", "Art"));
        assert_eq!(colors.get("Art"), Some([1, 2, 3]));
        assert_eq!(colors.get("Art/Trees"), Some([4, 5, 6]));
        assert_eq!(
            colors.get("Assets Old"),
            Some([7, 8, 9]),
            "a folder that only starts the same"
        );
        assert!(colors.copied("Art", "Art 2"));
        assert_eq!(colors.get("Art 2/Trees"), Some([4, 5, 6]));
        assert!(colors.removed("Art"));
        assert_eq!(colors.get("Art/Trees"), None);
        assert_eq!(colors.get("Art 2"), Some([1, 2, 3]));
        assert!(!colors.removed("Nowhere"));
    }
}
