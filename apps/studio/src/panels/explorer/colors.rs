//! Folders' icon colours (#603): a colour of the user's choosing for a folder's icon in the Explorer. They are
//! Studio's, not the game's: scripts and Properties never see them. They are kept in the project as
//! `.studio/explorer.json` (the editor's own data, which is neither an asset nor packed into the game), by each
//! folder's stable id in the start scene ([`SceneIds`]), so they follow a folder through renames and moves; copies
//! made in Studio (duplicate, copy and paste) take the colour along.

use std::collections::{BTreeMap, HashMap};

use metallic_engine::metallic_core::{DataModel, Id};
use metallic_engine::metallic_project::{EDITOR_DATA, Project, SceneIds};
use serde::{Deserialize, Serialize};

/// The member the colours are kept in.
pub fn member() -> String {
    format!("{EDITOR_DATA}explorer.json")
}

/// The Explorer's icon colours, by instance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IconColors {
    colors: HashMap<Id, [u8; 3]>,
}

/// `.studio/explorer.json`.
#[derive(Debug, Default, Deserialize, Serialize)]
struct File {
    /// Each folder's colour as `#rrggbb`, by its id in the start scene.
    #[serde(default)]
    icon_colors: BTreeMap<u64, String>,
}

impl IconColors {
    /// `id`'s colour, if it has one of its own.
    pub fn get(&self, id: Id) -> Option<[u8; 3]> {
        self.colors.get(&id).copied()
    }

    /// Gives `id` `color`, or its class's own colour back with `None`.
    pub fn set(&mut self, id: Id, color: Option<[u8; 3]>) {
        match color {
            Some(color) => {
                self.colors.insert(id, color);
            }
            None => {
                self.colors.remove(&id);
            }
        }
    }

    /// Gives `copy`, a copy of `original` ([`DataModel::clone_tree`]), and everything in it the colours of what they
    /// are copies of: the two trees are walked together, in the same order.
    pub fn copy_tree(&mut self, dm: &DataModel, original: Id, copy: Id) {
        if self.colors.is_empty() {
            return;
        }
        let (Ok(originals), Ok(copies)) = (dm.descendants(original), dm.descendants(copy)) else {
            return;
        };
        let pairs: Vec<(Id, Id)> = std::iter::once((original, copy)).chain(originals.zip(copies)).collect();
        for (from, to) in pairs {
            if let Some(color) = self.get(from) {
                self.colors.insert(to, color);
            }
        }
    }

    /// The colours kept in `project`, for the instances `ids` gives its start scene's ids to. A file that cannot be
    /// read gives none, and says why.
    pub fn load(project: &mut Project, ids: &SceneIds) -> Self {
        let member = member();
        if !project.contains(&member) {
            return Self::default();
        }
        let file: File = match project.read_text(&member).map(|text| serde_json::from_str(&text)) {
            Ok(Ok(file)) => file,
            Ok(Err(error)) => {
                log::warn!("explorer: {member} could not be read, so folders have their usual colours: {error}");
                return Self::default();
            }
            Err(error) => {
                log::warn!("explorer: {member} could not be read, so folders have their usual colours: {error}");
                return Self::default();
            }
        };
        let colors = file
            .icon_colors
            .iter()
            .filter_map(|(&scene_id, hex)| Some((ids.instance(scene_id)?, parse_hex(hex)?)))
            .collect();
        Self { colors }
    }

    /// Writes the colours of the instances in `dm`'s game into `project` by their start scene ids (`ids`, as the scene
    /// was just saved with): an instance not in the scene (deleted, or on the clipboard) is left out. With no colours
    /// the member is removed.
    pub fn save(&self, dm: &DataModel, project: &mut Project, ids: &SceneIds) -> Result<(), String> {
        let root = dm.root();
        let icon_colors: BTreeMap<u64, String> = self
            .colors
            .iter()
            .filter(|(id, _)| root.is_some_and(|root| dm.is_descendant_of(**id, root).unwrap_or(false)))
            .filter_map(|(&id, &[r, g, b])| Some((ids.get(id)?, format!("#{r:02x}{g:02x}{b:02x}"))))
            .collect();
        let member = member();
        if icon_colors.is_empty() {
            project.remove(&member);
            return Ok(());
        }
        let text = serde_json::to_string_pretty(&File { icon_colors }).map_err(|error| error.to_string())?;
        project
            .write(&member, text.into_bytes())
            .map_err(|error| error.to_string())
    }
}

/// `#rrggbb` as a colour.
fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_takes_the_colours_of_what_it_copies() {
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        let folder = dm.registry().class_id("Folder").unwrap();
        let world = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let outer = dm.create(folder).unwrap();
        let inner = dm.create(folder).unwrap();
        let plain = dm.create(folder).unwrap();
        dm.set_parent(outer, Some(world)).unwrap();
        dm.set_parent(plain, Some(outer)).unwrap();
        dm.set_parent(inner, Some(outer)).unwrap();
        let mut colors = IconColors::default();
        colors.set(outer, Some([1, 2, 3]));
        colors.set(inner, Some([4, 5, 6]));
        let copy = dm.clone_tree(outer).unwrap().unwrap();
        colors.copy_tree(&dm, outer, copy);
        assert_eq!(colors.get(copy), Some([1, 2, 3]));
        let copied: Vec<Option<[u8; 3]>> = dm.children(copy).unwrap().map(|id| colors.get(id)).collect();
        assert_eq!(copied, [None, Some([4, 5, 6])]);
        colors.set(outer, None);
        assert_eq!(colors.get(outer), None);
        assert_eq!(parse_hex("#0a0b0c"), Some([10, 11, 12]));
        assert_eq!(parse_hex("0a0b0c"), None);
    }
}
