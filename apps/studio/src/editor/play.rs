//! Play in Studio (ADR 0010): a game session of the open project, run by an embedded [`Player`].
//!
//! - **Copy on Play.** [`EditorState::play`] saves the editor's scene and loads the text into a new game, the same
//!   round trip as a project file, and hands that copy to [`Player::hosted`] with the project's scripts. The editor's
//!   own data model is never touched by a session.
//! - **Discard on Stop.** [`EditorState::stop`] drops the Player. Nothing the game or a live edit did can reach the
//!   editor's scene, its selection or the saved project.
//! - **Frames.** The app runs [`EditorState::tick`] once per frame, before drawing, so the Game tab and the panels
//!   show the frame just run; the shell then draws the game into the Game tab's texture (`app::game`).

use std::sync::Arc;
use std::time::Duration;

use egui::{Rect, TextureId};
use metallic_engine::metallic_classes;
use metallic_engine::metallic_core::DataModel;
use metallic_engine::metallic_project::{SceneIds, load_scene, save_scene};
use metallic_engine::metallic_render::settings::GraphicsSettings;
use metallic_engine::{RunState, project_sources};
use metallic_player::Player;

use super::{EditorState, History, Selection};
use crate::logging::{LogBuffer, ScriptOutput};
use crate::panels::explorer::ExplorerState;

/// A game session: the Player running the copy of the scene, and the Game tab's view of it.
pub struct PlaySession {
    /// The game.
    pub player: Player<ScriptOutput>,
    /// The scene id of each of the copy's instances (the same ids as the editor's scene's): which instance of the
    /// game is which instance of the editor's scene.
    pub ids: SceneIds,
    /// The Game tab's view.
    pub view: GameView,
    /// A number no other session of this Studio had: the shell's texture belongs to one session.
    pub number: u64,
    /// The selection in the running game (live editing, ADR 0010 §5): the editor's carried in by scene id at Play.
    pub selection: Selection,
    /// The Explorer's state over the running game.
    pub explorer: ExplorerState,
    /// The undo history of the panels' edits of the running game (#348), dropped at Stop.
    pub history: History,
}

/// The scene the Explorer and Properties show and edit: the editor's, or the running game's while a session runs
/// (live editing, ADR 0010 §5). Each has its own selection and Explorer state, so Stop finds the editor's as they were.
pub struct Scene<'a> {
    /// The game.
    pub dm: &'a mut DataModel,
    /// What is selected in it.
    pub selection: &'a mut Selection,
    /// The Explorer's expanded instances and search over it.
    pub explorer: &'a mut ExplorerState,
    /// Its undo history (#348).
    pub history: &'a mut History,
    /// Whether it is the running game's.
    pub playing: bool,
}

impl std::fmt::Debug for PlaySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaySession")
            .field("number", &self.number)
            .field("state", &self.player.engine().state())
            .finish_non_exhaustive()
    }
}

/// The Game tab's side of a session: what the panel and the shell tell each other, as the Viewport's do
/// ([`crate::viewport::ViewportState`]).
#[derive(Clone, Debug, Default)]
pub struct GameView {
    /// The tab's area as last drawn, in points.
    pub rect: Option<Rect>,
    /// The tab's size as last drawn, in physical pixels: the size the game is drawn at.
    pub pixels: Option<(u32, u32)>,
    /// Whether the tab was drawn since the shell last drew the game: a tab behind another is not drawn into.
    pub shown: bool,
    /// The game's render, registered with the UI renderer, once the shell has drawn one.
    pub texture: Option<TextureId>,
    /// The size of the target that render is in, as last drawn: kept larger than the tab, whose frame fills its
    /// top-left corner (#330).
    pub texture_pixels: Option<(u32, u32)>,
    /// The largest side that target can grow to (the device's largest texture).
    pub texture_limit: u32,
    /// Why the game cannot be drawn, when it cannot.
    pub error: Option<String>,
}

impl EditorState {
    /// Whether a game session runs (paused or not).
    pub fn is_playing(&self) -> bool {
        self.play.is_some()
    }

    /// Whether the session's game is paused.
    pub fn is_paused(&self) -> bool {
        self.play
            .as_ref()
            .is_some_and(|session| session.player.engine().state() == RunState::Paused)
    }

    /// Starts a session: copies the scene as it is now (unsaved edits included) into a new game and starts a Player
    /// on it, its scripts from the project and their output into `log`, drawn with `graphics`. Does nothing when a
    /// session runs already. Returns why it could not start.
    pub fn play(&mut self, graphics: GraphicsSettings, log: Arc<LogBuffer>, number: u64) -> Result<(), String> {
        if self.play.is_some() {
            return Ok(());
        }
        let saved = save_scene(&self.dm, &mut self.scene_ids).map_err(|error| format!("the scene: {error}"))?;
        let mut game = metallic_classes::new_game().map_err(|error| error.to_string())?;
        let loaded = load_scene(&mut game, &saved.text).map_err(|error| format!("the scene: {error}"))?;
        let sources = project_sources(&mut self.project).map_err(|error| format!("the scripts: {error}"))?;
        let player =
            Player::hosted(game, sources, graphics, ScriptOutput::new(log)).map_err(|error| error.to_string())?;
        log::info!("play: started {}", self.name());
        // The selection comes along by scene id, revealed in the game's Explorer.
        let selection = Selection::of(
            self.selection
                .ids()
                .iter()
                .filter_map(|&id| self.scene_ids.get(id))
                .filter_map(|scene_id| loaded.ids.instance(scene_id)),
        );
        let dm = player.engine().data_model();
        let mut explorer = ExplorerState::new(dm);
        if let Some(id) = selection.active() {
            explorer.reveal(dm, id);
        }
        self.play = Some(PlaySession {
            player,
            ids: loaded.ids,
            view: GameView::default(),
            number,
            history: History::new(selection.clone()),
            selection,
            explorer,
        });
        Ok(())
    }

    /// The scene the Explorer and Properties show and edit: the running game's while a session runs, else the
    /// editor's (ADR 0010 §5).
    pub fn scene(&mut self) -> Scene<'_> {
        match self.play.as_mut() {
            Some(session) => Scene {
                dm: session.player.engine_mut().data_model_mut(),
                selection: &mut session.selection,
                explorer: &mut session.explorer,
                history: &mut session.history,
                playing: true,
            },
            None => Scene {
                dm: &mut self.dm,
                selection: &mut self.selection,
                explorer: &mut self.explorer,
                history: &mut self.history,
                playing: false,
            },
        }
    }

    /// Ends the session: the game is dropped, and the editor's scene is as it was before Play. Returns whether one
    /// ran.
    pub fn stop(&mut self) -> bool {
        let Some(mut session) = self.play.take() else {
            return false;
        };
        session.player.engine_mut().stop();
        log::info!("play: stopped");
        true
    }

    /// Pauses a running game, or resumes a paused one. Returns whether there was a session.
    pub fn toggle_pause(&mut self) -> bool {
        let Some(session) = self.play.as_mut() else {
            return false;
        };
        let engine = session.player.engine_mut();
        if !engine.pause() {
            engine.resume();
        }
        true
    }

    /// Runs one frame of one fixed step of a paused game. Returns whether it ran (not without a paused game).
    pub fn step(&mut self) -> bool {
        self.is_paused()
            && self
                .play
                .as_mut()
                .is_some_and(|session| session.player.engine_mut().step_frame().is_some())
    }

    /// Runs the session's next frame, `dt` after the last: input, scripts, simulation (a paused game only takes its
    /// input). Does nothing without a session.
    pub fn tick(&mut self, dt: Duration) {
        if let Some(session) = self.play.as_mut() {
            session.player.frame(dt);
        }
    }
}
