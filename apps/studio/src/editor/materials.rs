//! The material editor's state (#680, ADR 0012): the materials open in tabs, what each looks like as edited, each
//! tab's own undo steps and preview angle, and the changes the renderers have not followed yet.
//!
//! A material's tab is `material:<asset id>`. An edit writes the material's file into the project at once (in memory,
//! as every edit is until Save) and notes its id ([`MaterialEditor::take_changed`]): the viewport's renderer and the
//! preview's read it again, so the preview and every part using the material change as the edit is made.

use std::collections::HashMap;

use metallic_engine::metallic_core::{Id, Value};
use metallic_engine::metallic_project::AssetId;
use metallic_engine::metallic_project::material::{Material, material_name};

use super::EditorState;
use crate::viewport::pick::is_a;

/// The material editor's panel id, the first part of a material tab's id.
pub const PANEL: &str = "material";

/// The tab id of the material `id`'s tab.
pub fn pane(id: AssetId) -> String {
    format!("{PANEL}:{id}")
}

/// The material a material tab's id names.
pub fn material_of(pane: &str) -> Option<AssetId> {
    pane.strip_prefix(PANEL)?.strip_prefix(':')?.parse().ok()
}

/// How many undo steps a tab keeps.
const UNDO_LIMIT: usize = 100;

/// One open material.
#[derive(Clone, Debug)]
pub struct OpenMaterial {
    /// As edited (and as its file says).
    pub material: Material,
    /// What it was before each edit, newest last.
    undo: Vec<Material>,
    /// What undo took back, newest last.
    redo: Vec<Material>,
    /// The preview's turn about the vertical axis and its tilt, in radians.
    pub yaw: f32,
    /// See `yaw`.
    pub pitch: f32,
}

/// The material editor's state.
#[derive(Debug, Default)]
pub struct MaterialEditor {
    open: HashMap<AssetId, OpenMaterial>,
    /// Tabs asked for, for Studio to open after the frame.
    opening: Vec<AssetId>,
    /// Materials whose file changed since the renderers last looked.
    changed: Vec<AssetId>,
    /// The tab whose preview is drawn next, its size in pixels, and the texture it was drawn into.
    pub preview: Option<(AssetId, (u32, u32))>,
    /// The preview's texture, once drawn, and the material it shows.
    pub preview_texture: Option<(AssetId, egui::TextureId)>,
}

impl MaterialEditor {
    /// The open material `id`, if its tab was opened.
    pub fn get(&self, id: AssetId) -> Option<&OpenMaterial> {
        self.open.get(&id)
    }

    /// The open material `id`, to turn its preview.
    pub fn get_mut(&mut self, id: AssetId) -> Option<&mut OpenMaterial> {
        self.open.get_mut(&id)
    }

    /// The tabs asked for since the last call.
    pub fn take_opening(&mut self) -> Vec<AssetId> {
        std::mem::take(&mut self.opening)
    }

    /// The materials whose file changed since the last call.
    pub fn take_changed(&mut self) -> Vec<AssetId> {
        std::mem::take(&mut self.changed)
    }

    /// Notes that the material `id`'s file changed (for the renderers).
    pub fn note_changed(&mut self, id: AssetId) {
        if !self.changed.contains(&id) {
            self.changed.push(id);
        }
    }

    /// Whether the tab of `id` has an edit to undo, and one to redo.
    pub fn can_undo_redo(&self, id: AssetId) -> (bool, bool) {
        self.open
            .get(&id)
            .map_or((false, false), |open| (!open.undo.is_empty(), !open.redo.is_empty()))
    }

    /// Forgets the tab of `id` (closed, or its file gone).
    pub fn close(&mut self, id: AssetId) {
        self.open.remove(&id);
    }
}

impl EditorState {
    /// The project's material `id`, read from its file.
    pub fn read_material(&mut self, id: AssetId) -> Result<Material, String> {
        let path = self
            .project
            .assets()
            .path(id)
            .ok_or_else(|| format!("The project has no material {id}."))?
            .to_owned();
        let bytes = self.project.read(&path).map_err(|error| error.to_string())?;
        Material::from_bytes(&bytes)
            .map(|loaded| loaded.material)
            .map_err(|error| format!("{path}: {error}"))
    }

    /// Opens the material at `path` in its tab (#680).
    pub fn open_material(&mut self, path: &str) -> Result<(), String> {
        let id = self
            .project
            .assets()
            .id(path)
            .ok_or_else(|| format!("{path} has no asset id yet."))?;
        if !self.materials.open.contains_key(&id) {
            let material = self.read_material(id)?;
            self.materials.open.insert(
                id,
                OpenMaterial {
                    material,
                    undo: Vec::new(),
                    redo: Vec::new(),
                    yaw: 0.6,
                    pitch: 0.35,
                },
            );
        }
        self.materials.opening.push(id);
        Ok(())
    }

    /// The name of the material `id`: its file's name without `.material`.
    pub fn material_title(&self, id: AssetId) -> Option<String> {
        self.project
            .assets()
            .path(id)
            .map(|path| material_name(path).to_owned())
    }

    /// Makes `material` the material `id`, an undo step of its tab: written into its file at once, and every part and
    /// preview using it follows.
    pub fn edit_material(&mut self, id: AssetId, material: Material) -> Result<(), String> {
        let Some(open) = self.materials.open.get_mut(&id) else {
            return Err("That material is not open.".into());
        };
        if open.material == material {
            return Ok(());
        }
        let before = std::mem::replace(&mut open.material, material);
        open.undo.push(before);
        if open.undo.len() > UNDO_LIMIT {
            open.undo.remove(0);
        }
        open.redo.clear();
        self.write_material(id)
    }

    /// Takes back the tab of `id`'s last edit (`redo`: does again what was taken back). Returns whether there was one.
    pub fn undo_material(&mut self, id: AssetId, redo: bool) -> bool {
        let Some(open) = self.materials.open.get_mut(&id) else {
            return false;
        };
        let (from, to) = if redo {
            (&mut open.redo, &mut open.undo)
        } else {
            (&mut open.undo, &mut open.redo)
        };
        let Some(material) = from.pop() else {
            return false;
        };
        to.push(std::mem::replace(&mut open.material, material));
        if let Err(error) = self.write_material(id) {
            log::warn!("materials: {error}");
        }
        true
    }

    /// Writes the open material `id` into its file and notes the change.
    fn write_material(&mut self, id: AssetId) -> Result<(), String> {
        let (Some(open), Some(path)) = (self.materials.open.get(&id), self.project.assets().path(id)) else {
            return Err("That material's file is gone.".into());
        };
        let path = path.to_owned();
        let text = open.material.to_json();
        self.project.write(&path, text).map_err(|error| error.to_string())?;
        self.materials.note_changed(id);
        Ok(())
    }

    /// Gives every selected part the material `id` (#703: the button on a material's tile), as one undo
    /// step of the scene. Returns how many parts took it; none when nothing selected is a part.
    pub fn apply_material(&mut self, id: AssetId) -> usize {
        let value = Value::String(id.to_string().into());
        let mut scene = self.scene();
        let parts: Vec<Id> = scene
            .selection
            .ids()
            .iter()
            .copied()
            .filter(|&part| is_a(scene.dm, part, "BasePart"))
            .collect();
        let mut applied = 0;
        for part in parts {
            match scene.set_by_name(part, "Material", value.clone()) {
                Ok(()) => applied += 1,
                Err(error) => log::warn!("materials: could not apply the material: {error}"),
            }
        }
        applied
    }

    /// Whether a part is selected to give a material to (#703: the button on a material's tile).
    pub fn can_apply_material(&mut self) -> bool {
        let scene = self.scene();
        scene.selection.ids().iter().any(|&id| is_a(scene.dm, id, "BasePart"))
    }

    /// Gives the part `part` the material `id` (#680: a material dropped on it in the viewport), as an undo step.
    pub fn apply_material_to(&mut self, id: AssetId, part: Id) -> Result<(), String> {
        let mut scene = self.scene();
        if !is_a(scene.dm, part, "BasePart") {
            return Err("Only a part takes a material.".into());
        }
        scene
            .set_by_name(part, "Material", Value::String(id.to_string().into()))
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_names_its_material() {
        let id = AssetId::from_u128(0xabc);
        assert_eq!(material_of(&pane(id)), Some(id));
        assert_eq!(crate::dock::panel_of(&pane(id)), PANEL);
        assert_eq!(material_of("material:nope"), None);
        assert_eq!(material_of("script:1:2"), None);
    }
}
