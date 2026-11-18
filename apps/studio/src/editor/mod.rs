//! The editor (ADR 0009, Screens): the open project and everything its panels show and remember.
//!
//! Opening runs on a worker thread ([`open`]) behind a progress overlay; the result is an [`EditorState`], which the
//! dock's panels read and change through [`crate::dock::PanelContext`]. The layout itself is the dock's, not the
//! editor's, so it outlives the project.

pub mod assets;
pub mod edit;
pub mod folder_colors;
pub mod history;
pub mod images;
pub mod materials;
pub mod multi;
pub mod open;
pub mod picture_loader;
pub mod pictures;
pub mod play;
pub mod selection;
pub mod sounds;

use std::path::Path;
use std::sync::Arc;

use metallic_engine::MeshAssets;
use metallic_engine::metallic_core::{DataModel, Id};
use metallic_engine::metallic_project::{Project, SceneIds, save_scene};

use crate::logging::LogBuffer;
use crate::panels::explorer::ExplorerState;
use crate::panels::output::OutputState;
use crate::viewport::ViewportState;
pub use edit::{Clipboard, TreeEdit};
pub use history::History;
pub use open::{AssetRow, Opened, Opening, STEPS};
pub use play::{GameView, PlaySession, Scene};
pub use selection::Selection;

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
    /// The Assets browser: its open folder, selection and search (#676).
    pub browser: assets::AssetBrowser,
    /// The materials open in the material editor (#680).
    pub materials: materials::MaterialEditor,
    /// The Assets browser's folder colours (#741), kept in the project.
    pub folder_colors: folder_colors::FolderColors,
    /// The Assets panel's rendered thumbnails of materials and scenes (#681).
    pub pictures: pictures::Pictures,
    /// The pictures open in the image preview (#681).
    pub images: images::ImagePreviews,
    /// The sound previews (#831): what each shows and what plays.
    pub sounds: sounds::SoundPreviews,
    /// Small pictures of the project's images, by asset id, for the material editor's slots (#680); `None` for one that
    /// could not be read.
    pub thumbnails: std::collections::HashMap<metallic_engine::metallic_project::AssetId, Option<egui::TextureHandle>>,
    /// Decodes the thumbnails above off the UI thread (#771).
    pub picture_loader: picture_loader::PictureLoader,
    /// What each mesh's model holds (#806), read the first time its properties show; `None` for one that could not be.
    pub mesh_facts: std::collections::HashMap<metallic_engine::metallic_project::AssetId, Option<assets::MeshFacts>>,
    /// The mesh and texture loader the viewport's renderer is fed from, until the renderer takes it.
    pub mesh_assets: Option<MeshAssets>,
    /// What is selected (#347).
    pub selection: Selection,
    /// The editor's undo history (#348); a running game has its own.
    pub history: History,
    /// What was copied or cut (#351), in the editor's scene or a running game's.
    pub clipboard: Clipboard,
    /// The Explorer's expanded instances and search.
    pub explorer: ExplorerState,
    /// The Output panel's lines, filter and search.
    pub output: OutputState,
    /// The viewport's camera and texture.
    pub viewport: ViewportState,
    /// The game session, while one runs (Play in Studio, ADR 0010).
    pub play: Option<PlaySession>,
    /// The file or folder the Properties panel shows in place of the selected instances (#779), set as it draws.
    pub properties_asset: Option<String>,
    /// The parts and lights in the editor's Workspace by where they are (#596), for picking; [`EditorState::refresh_parts`]
    /// brings it up to date.
    pub parts: crate::viewport::index::PartIndex,
    /// The script editor's open files and requests (#528).
    pub scripts: crate::code::Scripts,
    /// The other users of a Team Create session (ADR 0011), as the viewport and the Explorer show them.
    pub presence: crate::team::presence::Shown,
    /// Whether Team Create changed the scene since the last save: others' edits are no step of this history.
    pub collab_changed: bool,
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
            mut project,
            dm,
            scene_ids,
            assets,
            mesh_assets,
        } = opened;
        let mut explorer = ExplorerState::new(&dm);
        // The folders' icon colours (#603), kept beside the scene.
        explorer.icon_colors = crate::panels::explorer::colors::IconColors::load(&mut project, &scene_ids);
        // The Assets browser's folder colours (#741).
        let folder_colors = folder_colors::FolderColors::load(&mut project);
        let viewport = ViewportState::new(&dm);
        // Picking meets a MeshPart by its triangles once they are read (#494).
        crate::viewport::shapes::register_meshes(dm.uid(), project.asset_reader());
        Self {
            project,
            dm,
            scene_ids,
            assets,
            browser: assets::AssetBrowser::default(),
            materials: materials::MaterialEditor::default(),
            folder_colors,
            pictures: pictures::Pictures::default(),
            images: images::ImagePreviews::default(),
            sounds: sounds::SoundPreviews::default(),
            thumbnails: std::collections::HashMap::new(),
            picture_loader: picture_loader::PictureLoader::default(),
            mesh_facts: std::collections::HashMap::new(),
            mesh_assets: Some(mesh_assets),
            selection: Selection::new(),
            history: History::new(Selection::new()),
            clipboard: Clipboard::default(),
            explorer,
            output: OutputState::new(log),
            viewport,
            play: None,
            properties_asset: None,
            scripts: crate::code::Scripts::default(),
            parts: crate::viewport::index::PartIndex::new(),
            presence: Default::default(),
            collab_changed: false,
        }
    }

    /// Brings [`EditorState::parts`] up to date with the editor's scene: what changed since the last refresh.
    pub fn refresh_parts(&mut self) {
        self.parts.update(&mut self.dm);
    }

    /// The part index of the scene the Viewport shows (#835): the running game's while one runs, else the editor's.
    pub fn shown_parts_mut(&mut self) -> &mut crate::viewport::index::PartIndex {
        match self.play.as_mut() {
            Some(session) => &mut session.parts,
            None => &mut self.parts,
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

    /// The history the Edit menu undoes: the running game's while one runs (live editing), else the editor's.
    pub fn history(&self) -> &History {
        self.play.as_ref().map_or(&self.history, |session| &session.history)
    }

    /// The game the panels show and its selection: the running game's while one runs, else the editor's.
    pub fn shown(&self) -> (&DataModel, &Selection) {
        match &self.play {
            Some(session) => (session.player.engine().data_model(), &session.selection),
            None => (&self.dm, &self.selection),
        }
    }

    /// Whether the project has changes not saved: in its files, or to its scene since it was saved or opened (#348).
    pub fn is_modified(&self) -> bool {
        self.project.is_modified() || !self.history.is_saved() || self.collab_changed
    }

    /// Selects `id` alone (or nothing), and shows it in the Explorer.
    pub fn select(&mut self, id: Option<Id>) {
        self.selection.set_only(id);
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
        // The folders' icon colours (#603), by the ids the scene was just saved with.
        self.explorer
            .icon_colors
            .save(&self.dm, &mut self.project, &self.scene_ids)?;
        self.project.save().map_err(|error| error.to_string())?;
        self.history.mark_saved();
        self.collab_changed = false;
        log::info!("projects: saved {}", self.project.path().display());
        Ok(())
    }
}
