//! The editor (ADR 0009, Screens): the open project and everything its panels show and remember.
//!
//! Opening runs on a worker thread ([`open`]) behind a progress overlay; the result is an [`EditorState`], which the
//! dock's panels read and change through [`crate::dock::PanelContext`]. The layout itself is the dock's, not the
//! editor's, so it outlives the project.

pub mod open;

use std::path::Path;
use std::sync::Arc;

use metallic_engine::MeshAssets;
use metallic_engine::metallic_core::{DataModel, Id};
use metallic_engine::metallic_project::{Project, SceneIds, save_scene};

use crate::logging::LogBuffer;
use crate::panels::explorer::ExplorerState;
use crate::panels::output::OutputState;
pub use open::{AssetRow, Opened, Opening, STEPS};

/// The open project, its game, and the panels' state.
pub struct EditorState {
    /// The project.
    pub project: Project,
    /// The game: the start scene, as loaded.
    pub dm: DataModel,
    /// The start scene's instance ids, kept so a save writes the same ids back.
    pub scene_ids: SceneIds,
    /// The asset index, by path, as the Assets panel lists it.
    pub assets: Vec<AssetRow>,
    /// The mesh and texture loader the viewport's renderer is fed from (#217).
    pub mesh_assets: MeshAssets,
    /// The selected instance.
    pub selection: Option<Id>,
    /// The Explorer's expanded instances and search.
    pub explorer: ExplorerState,
    /// The Output panel's lines, filter and search.
    pub output: OutputState,
}

impl std::fmt::Debug for EditorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorState")
            .field("project", &self.project)
            .field("selection", &self.selection)
            .finish_non_exhaustive()
    }
}

impl EditorState {
    /// The editor of a project just opened, its Output reading `log`.
    pub fn new(opened: Opened, log: Arc<LogBuffer>) -> Self {
        let Opened {
            project,
            dm,
            scene_ids,
            assets,
            mesh_assets,
        } = opened;
        let explorer = ExplorerState::new(&dm);
        Self {
            project,
            dm,
            scene_ids,
            assets,
            mesh_assets,
            selection: None,
            explorer,
            output: OutputState::new(log),
        }
    }

    /// The project's file.
    pub fn path(&self) -> &Path {
        self.project.path()
    }

    /// The project's name.
    pub fn name(&self) -> &str {
        &self.project.manifest().name
    }

    /// Selects `id` (or nothing), and shows it in the Explorer.
    pub fn select(&mut self, id: Option<Id>) {
        self.selection = id;
        if let Some(id) = id {
            self.explorer.reveal(&self.dm, id);
        }
    }

    /// Saves the project (#187, #190): the game's scene into the start scene's member, then the archive, crash-safely.
    pub fn save(&mut self) -> Result<(), String> {
        let scene = self.project.manifest().start_scene.clone();
        let saved = save_scene(&self.dm, &mut self.scene_ids).map_err(|error| error.to_string())?;
        for warning in &saved.warnings {
            log::warn!("scene: {scene}: {warning}");
        }
        self.project
            .write(&scene, saved.text.into_bytes())
            .map_err(|error| error.to_string())?;
        self.project.save().map_err(|error| error.to_string())?;
        log::info!("projects: saved {}", self.project.path().display());
        Ok(())
    }
}
