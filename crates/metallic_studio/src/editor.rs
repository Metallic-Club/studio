//! The editing model of one open game: world, selection, history, tool settings, Output and Play.
//!
//! [`EditorState`] holds no UI. Panels ([`crate::explorer`], [`crate::properties`],
//! [`crate::viewport`], [`crate::output`]) read it and change it only through its methods, which
//! route every edit through [`History`](crate::commands::History). Building tools (#213 onward)
//! use the same entry points:
//!
//! - [`EditorState::apply`] for a one-shot edit (Insert, Delete, a typed property value).
//! - [`EditorState::record`] after a live-preview gesture that already wrote the world (gizmo
//!   drags, Properties scrubbing): one gesture, one undo step.
//! - [`EditorState::selection`] for what to act on; [`EditorState::tool`],
//!   [`EditorState::snapping`] and [`EditorState::space`] for the header settings.
//!
//! After any history change the selection drops ids that are no longer in the tree, and the undo
//! name flashes in the status bar for 2 s.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use metallic_core::{InstanceId, Value, Vec3, World, load_scene, save_scene};

use crate::StudioError;
use crate::commands::{Command, CommandError, History};
use crate::output::{Level, OutputSink, Tag};
use crate::play::{PlayController, PlayStart};
use crate::selection::{Selection, is_detached};
use crate::transform::Aabb;

/// How long an undo or save note stays in the status bar.
pub const NOTE_TIME: Duration = Duration::from_secs(2);

/// Classes offered by Insert (Explorer right-click and the Insert menu).
pub const INSERTABLE: &[&str] = &[
    "Part",
    "SpawnLocation",
    "Checkpoint",
    "Script",
    "LocalScript",
    "Folder",
    "Model",
    "RemoteEvent",
    "ScreenGui",
    "TextLabel",
];

/// The active viewport tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Click to select, drag on empty space for a box.
    Select,
    /// Move arrows and plane squares.
    #[default]
    Move,
    /// Rotate rings.
    Rotate,
    /// Scale face handles.
    Scale,
}

impl Tool {
    /// Name in the header.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Move => "Move",
            Tool::Rotate => "Rotate",
            Tool::Scale => "Scale",
        }
    }
}

/// Snapping settings (header chips).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapping {
    /// Snapping on (Ctrl inverts it during a drag).
    pub enabled: bool,
    /// Move and scale increment in studs.
    pub move_step: f64,
    /// Rotate increment in degrees.
    pub rotate_step: f64,
}

impl Default for Snapping {
    fn default() -> Self {
        Snapping {
            enabled: true,
            move_step: 1.0,
            rotate_step: 15.0,
        }
    }
}

/// Gizmo axes: world or the active part's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Space {
    /// World axes.
    #[default]
    World,
    /// The active part's axes.
    Local,
}

/// A one-off request from a command to the UI (the UI owns the camera and text fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequest {
    /// Frame the selection in the viewport (F).
    FrameSelection,
    /// Frame everything (Home).
    FrameAll,
    /// Start renaming the active item in Explorer (F2).
    Rename(InstanceId),
    /// Open a script in the editor tab.
    OpenScript(InstanceId),
    /// Run a registered action (context menus).
    RunAction(&'static str),
    /// Open the Settings dialog (Ctrl+,).
    OpenSettings,
    /// Close the game, asking first when it has unsaved changes (Ctrl+W, like File > Close game).
    CloseGame,
}

/// Everything about one open game except the UI.
pub struct EditorState {
    /// The edit world.
    pub world: World,
    /// Selected instances.
    pub selection: Selection,
    /// Undo and redo.
    pub history: History,
    /// The game folder.
    pub project_dir: PathBuf,
    /// The scene file being edited.
    pub scene_path: PathBuf,
    /// Game name (title bar).
    pub name: String,
    /// Active tool.
    pub tool: Tool,
    /// Snapping settings.
    pub snapping: Snapping,
    /// Gizmo space.
    pub space: Space,
    /// Output handle.
    pub output: OutputSink,
    /// Play modes and the running session.
    pub play: PlayController,
    /// When false, nothing is written to disk (screenshot mode).
    pub can_save: bool,
    /// Where Insert puts a new part: the surface under the cursor, updated by the viewport.
    pub insert_point: Vec3,
    requests: Vec<UiRequest>,
    note: Option<(String, Instant)>,
}

impl EditorState {
    /// Wraps a world.
    pub fn new(
        world: World,
        project_dir: &Path,
        scene_path: &Path,
        name: &str,
        output: OutputSink,
    ) -> EditorState {
        EditorState {
            world,
            selection: Selection::new(),
            history: History::new(),
            project_dir: project_dir.to_path_buf(),
            scene_path: scene_path.to_path_buf(),
            name: name.to_string(),
            tool: Tool::default(),
            snapping: Snapping::default(),
            space: Space::default(),
            output,
            play: PlayController::new(),
            can_save: true,
            insert_point: Vec3::new(0.0, 0.5, 0.0),
            requests: Vec::new(),
            note: None,
        }
    }

    /// Opens a game folder: reads `metallic.toml` and its main scene.
    pub fn open(project_dir: &Path, output: OutputSink) -> Result<EditorState, StudioError> {
        let config =
            metallic_runtime::ProjectConfig::read(project_dir).map_err(|e| StudioError::Scene {
                path: project_dir.to_path_buf(),
                message: e.to_string(),
            })?;
        let scene_path = project_dir.join(&config.main_scene);
        let world = load_scene(&scene_path).map_err(|e| StudioError::Scene {
            path: scene_path.clone(),
            message: e.to_string(),
        })?;
        output.push(
            Tag::Studio,
            Level::Info,
            format!("Opened {} ({})", config.name, config.main_scene.display()),
        );
        Ok(EditorState::new(
            world,
            project_dir,
            &scene_path,
            &config.name,
            output,
        ))
    }

    /// Runs a command as one undo step.
    pub fn apply(&mut self, cmd: Command) -> Result<(), CommandError> {
        if self.play.is_playing() {
            return Err(CommandError::Nothing("edit while playing"));
        }
        let result = self.history.apply(cmd, &mut self.world).map(|_| ());
        if let Err(e) = &result {
            self.output.push(Tag::Studio, Level::Warning, e.to_string());
        }
        self.after_history_change();
        result
    }

    /// Pushes an already-applied command (end of a live-preview gesture). Ignored while playing.
    pub fn record(&mut self, cmd: Command) {
        if self.play.is_playing() {
            // Edits are refused while playing; a gesture cannot have started then.
            return;
        }
        self.history.record(cmd, &mut self.world);
        self.after_history_change();
    }

    /// Undo; shows the step name in the status bar.
    pub fn undo(&mut self) {
        if self.play.is_playing() {
            return;
        }
        if let Some(name) = self.history.undo(&mut self.world) {
            self.note(format!("Undo {name}"));
        }
        self.after_history_change();
    }

    /// Redo.
    pub fn redo(&mut self) {
        if self.play.is_playing() {
            return;
        }
        if let Some(name) = self.history.redo(&mut self.world) {
            self.note(format!("Redo {name}"));
        }
        self.after_history_change();
    }

    fn after_history_change(&mut self) {
        self.selection.retain_live(&self.world, &self.history);
    }

    /// True when there are unsaved edits.
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// True when `id` is in the tree (not destroyed, not parked by undo/delete).
    pub fn is_live(&self, id: InstanceId) -> bool {
        self.world.is_alive(id) && !is_detached(&self.world, &self.history, id)
    }

    /// Roots shown in Explorer (services and other non-parked roots).
    pub fn visible_roots(&self) -> Vec<InstanceId> {
        self.world
            .roots()
            .iter()
            .copied()
            .filter(|&r| !self.history.is_parked(r))
            .collect()
    }

    /// A copy of the world without parked instances (what Save writes and Play runs).
    pub fn clean_world(&self) -> World {
        let mut w = self.world.deep_clone();
        for id in self.history.parked() {
            w.destroy(id);
        }
        w
    }

    /// Writes the scene file and marks the history clean.
    pub fn save(&mut self) -> Result<(), StudioError> {
        if !self.can_save {
            self.note("Saving is off in this window".into());
            return Ok(());
        }
        let world = self.clean_world();
        save_scene(&world, &self.scene_path).map_err(|e| StudioError::Scene {
            path: self.scene_path.clone(),
            message: e.to_string(),
        })?;
        self.history.mark_saved();
        self.note("Saved".into());
        self.output.push(
            Tag::Studio,
            Level::Info,
            format!("Saved {}", file_name(&self.scene_path)),
        );
        Ok(())
    }

    /// Shows `text` in the status bar for [`NOTE_TIME`].
    pub fn note(&mut self, text: String) {
        self.note = Some((text, Instant::now()));
    }

    /// The current status note, if still fresh.
    pub fn current_note(&self) -> Option<&str> {
        self.note
            .as_ref()
            .filter(|(_, at)| at.elapsed() < NOTE_TIME)
            .map(|(t, _)| t.as_str())
    }

    /// Queues a request for the UI.
    pub fn request(&mut self, r: UiRequest) {
        self.requests.push(r);
    }

    /// Takes queued UI requests.
    pub fn take_requests(&mut self) -> Vec<UiRequest> {
        std::mem::take(&mut self.requests)
    }

    /// Inserts `class` under `parent` (Workspace when `None`); a part lands at `position`.
    /// Selects the new instance.
    pub fn insert(
        &mut self,
        class: &str,
        parent: Option<InstanceId>,
        position: Option<Vec3>,
    ) -> Option<InstanceId> {
        let parent = parent.or_else(|| self.world.root("Workspace"))?;
        let mut props = Vec::new();
        if metallic_core::classes::is_subclass(class, "BasePart") {
            if let Some(p) = position {
                props.push(("Position".to_string(), Value::Vector3(p)));
            }
            props.push(("Anchored".to_string(), Value::Bool(true)));
        }
        let cmd = Command::Create {
            class: class.to_string(),
            parent,
            props,
            id: None,
        };
        self.apply(cmd).ok()?;
        let id = self.history.peek_undo()?.created().first().copied()?;
        self.selection.select_only(id);
        Some(id)
    }

    /// Deletes the selection (Delete key).
    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let cmd = Command::delete(self.selection.ids());
        let _ = self.apply(cmd);
    }

    /// Duplicates the selection, placing the copies on top of it, and selects the copies.
    pub fn duplicate_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let parts = self.selection.parts(&self.world);
        let height = Aabb::of_parts(&self.world, &parts).map_or(0.0, |b| b.size().y);
        let cmd = Command::Duplicate {
            sources: self.selection.ids().to_vec(),
            offset: Vec3::new(0.0, height, 0.0),
            copies: Vec::new(),
        };
        if self.apply(cmd).is_ok()
            && let Some(done) = self.history.peek_undo()
        {
            let copies = done.created();
            self.selection.set(copies);
        }
    }

    /// Groups the selection into a new Model and selects it.
    pub fn group_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let cmd = Command::Group {
            items: self
                .selection
                .ids()
                .iter()
                .map(|&id| crate::commands::Placement {
                    id,
                    parent: None,
                    index: 0,
                })
                .collect(),
            model: None,
        };
        if self.apply(cmd).is_ok()
            && let Some(m) = self
                .history
                .peek_undo()
                .and_then(|c| c.created().first().copied())
        {
            self.selection.select_only(m);
        }
    }

    /// Moves `ids` under `parent` (Explorer drag and drop).
    pub fn reparent(&mut self, ids: &[InstanceId], parent: InstanceId) {
        let from = ids
            .iter()
            .map(|&id| crate::commands::Placement {
                id,
                parent: None,
                index: 0,
            })
            .collect();
        let _ = self.apply(Command::Reparent { from, to: parent });
    }

    /// Sets one property on several instances as one undo step.
    pub fn set_property(&mut self, ids: &[InstanceId], prop: &str, value: Value) {
        let mut commands: Vec<Command> = ids
            .iter()
            .filter_map(|&id| {
                let old = self.world.get(id, prop).ok()?;
                (old != value).then(|| Command::set(id, prop, old, value.clone()))
            })
            .collect();
        let cmd = match commands.len() {
            0 => return,
            1 => commands.remove(0),
            n => Command::Batch {
                name: format!("Set {prop} of {n} objects"),
                commands,
            },
        };
        let _ = self.apply(cmd);
    }

    /// Starts the play mode `id` with a clean copy of the world.
    pub fn start_play(&mut self, id: &str) {
        let world = self.clean_world();
        let output = self.output.clone();
        let dir = self.project_dir.clone();
        self.play.start(
            id,
            PlayStart {
                world: &world,
                project_dir: &dir,
                output,
            },
        );
    }

    /// The bounding box of the selection's parts.
    pub fn selection_bounds(&self) -> Option<Aabb> {
        Aabb::of_parts(&self.world, &self.selection.parts(&self.world))
    }

    /// Display name of an instance (`Name`, or its class).
    pub fn display_name(&self, id: InstanceId) -> String {
        match self.world.get_ref(id, "Name") {
            Ok(Value::String(s)) => s.clone(),
            _ => self.world.class_name(id).unwrap_or("?").to_string(),
        }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
