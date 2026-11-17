//! The image preview (#681): a project picture in a tab of its own, `image:<asset id>`, opened from the Assets panel.
//! It shows the picture and its facts; it is not an editor.

use std::collections::HashMap;

use metallic_engine::metallic_project::AssetId;
use metallic_engine::metallic_project::material::{Material, is_material};

use super::EditorState;

/// The image preview's panel id, the first part of its tab's id.
pub const PANEL: &str = "image";

/// The largest side a preview's texture is decoded at.
const LARGEST: u32 = 2048;

/// The tab id of the picture `id`'s preview.
pub fn pane(id: AssetId) -> String {
    format!("{PANEL}:{id}")
}

/// The picture an image tab's id names.
pub fn image_of(pane: &str) -> Option<AssetId> {
    pane.strip_prefix(PANEL)?.strip_prefix(':')?.parse().ok()
}

/// A picture as its preview shows it: its texture and its size in pixels.
#[derive(Clone)]
pub struct Shown {
    /// The picture, fitted to [`LARGEST`].
    pub texture: egui::TextureHandle,
    /// Its own width and height.
    pub size: (u32, u32),
}

impl std::fmt::Debug for Shown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shown")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// The previews open, and those asked for.
#[derive(Debug, Default)]
pub struct ImagePreviews {
    /// Each picture read, `None` for one that could not be.
    shown: HashMap<AssetId, Option<Shown>>,
    opening: Vec<AssetId>,
}

impl ImagePreviews {
    /// The tabs asked for since the last call.
    pub fn take_opening(&mut self) -> Vec<AssetId> {
        std::mem::take(&mut self.opening)
    }
}

impl EditorState {
    /// Opens the picture at `path` in its preview tab (#681).
    pub fn open_image(&mut self, path: &str) -> Result<(), String> {
        let id = self
            .project
            .assets()
            .id(path)
            .ok_or_else(|| format!("{path} has no asset id yet."))?;
        self.images.opening.push(id);
        Ok(())
    }

    /// The picture `id` as its preview shows it, read the first time.
    pub fn image_preview(&mut self, ctx: &egui::Context, id: AssetId) -> Option<Shown> {
        if let Some(shown) = self.images.shown.get(&id) {
            return shown.clone();
        }
        let mut reader = self.project.asset_reader();
        // A pixelated picture shows crisp (#781).
        let options = if self.project.assets().pixelated_id(id) {
            egui::TextureOptions::NEAREST
        } else {
            egui::TextureOptions::LINEAR
        };
        let shown = reader.load_image(&id.to_string()).ok().map(|mut image| {
            let size = (image.width, image.height);
            image.fit(LARGEST);
            let picture =
                egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba);
            Shown {
                texture: ctx.load_texture(format!("metallic.image.{id}"), picture, options),
                size,
            }
        });
        self.images.shown.insert(id, shown.clone());
        shown
    }

    /// Draws the picture at `path` pixelated (nearest filtering) or smooth (#781): kept in the project's asset index,
    /// it shows in its thumbnails and preview at once, and the renderers read it and the materials that use it again.
    pub fn set_pixelated(&mut self, path: &str, on: bool) {
        let Some(id) = self.project.assets().id(path) else {
            return;
        };
        if self.project.assets().pixelated(path) == on {
            return;
        }
        self.project.set_pixelated(path, on);
        self.thumbnails.remove(&id);
        self.images.shown.remove(&id);
        for material in self.materials_using(id) {
            if let Some(material) = self.project.assets().id(&material) {
                self.materials.note_changed(material);
            }
        }
        self.materials.note_changed_picture(id);
    }

    /// The materials that use the picture `id`, by path, sorted.
    pub fn materials_using(&mut self, id: AssetId) -> Vec<String> {
        let materials: Vec<String> = self
            .project
            .members()
            .into_iter()
            .filter(|member| is_material(member))
            .map(str::to_owned)
            .collect();
        let mut using = Vec::new();
        for path in materials {
            let Ok(bytes) = self.project.read(&path) else {
                continue;
            };
            if let Ok(loaded) = Material::from_bytes(&bytes)
                && loaded.material.references().contains(&id)
            {
                using.push(path);
            }
        }
        using
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_names_its_picture() {
        let id = AssetId::from_u128(0x5);
        assert_eq!(image_of(&pane(id)), Some(id));
        assert_eq!(crate::dock::panel_of(&pane(id)), PANEL);
        assert_eq!(image_of("material:00000000000000000000000000000005"), None);
    }
}
