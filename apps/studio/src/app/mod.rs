//! The app: [`Studio`], the one window's screens as a state machine (ADR 0009, Screens), and the shell that runs it
//! on Studio's own winit loop with the engine's render device ([`run`]).
//!
//! [`Studio`] knows nothing of winit or wgpu: it draws into an `egui::Ui` and asks the shell for window changes
//! through [`WindowRequest`]s, so the UI tests drive it without a window or a GPU. The engine's side of the viewport
//! is the shell's too ([`viewport`]): it draws the game into the texture the Viewport panel shows.

mod editor;
pub mod game;
mod game_input;
mod native;
#[cfg(test)]
mod offscreen;
mod shell;
pub mod sign_in;
pub mod splash;
pub mod startup;
pub mod viewport;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use egui::{
    Align, Align2, Color32, ColorImage, CornerRadius, Frame, Id, Label, Layout, Rect, RichText, Stroke, TextureHandle,
    TextureOptions, UiBuilder, pos2, vec2,
};

use crate::dock::{Dock, PanelContext};
use crate::editor::{EditorState, Opening, TreeEdit};
use crate::emblem;
use crate::keymap::{ActivationKind, Context, Focus, Keymap, Overrides, ids};
use crate::logging::LogBuffer;
use crate::panels;
use crate::projects::{FileDialog, Projects, RAIL_MARK, name_from_path};
use crate::settings::{SettingsStore, SettingsWindow, StudioSettings};
use crate::theme::icons::custom::CustomIcons;
use crate::theme::icons::{self, Icon};
use crate::theme::tokens::{Accent, radius, shadow, size, space, text};
use crate::theme::{ButtonKind, Look, ThemeState, button, current_colors, fonts};
use crate::toolbar::{self, StudioCommand, ToolAction, Toolbar};
use crate::viewport::tools::Tool;
use editor::MenuAction;
use metallic_engine::metallic_classes::input::feed::CursorState;
pub use shell::run;
use splash::{Splash, Status};
use startup::{Loaded, Startup};

/// Which screen the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    /// The loading splash, over the startup steps.
    Splash,
    /// Sign in, or continue offline.
    SignIn,
    /// The projects page.
    Projects,
    /// The editor, with a project opening or open.
    Editor,
}

/// A change only the shell can make to the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRequest {
    /// The splash is done: the window becomes Studio's main window, centred where the splash was.
    BecomeMainWindow,
}

/// Something Studio tells the user at the bottom of the window until it is dismissed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notice {
    title: &'static str,
    body: String,
}

/// The Editor screen: the project opening behind the progress overlay, or open.
enum EditorScreen {
    Opening(Opening),
    Open(Box<EditorState>),
}

/// Studio's name in the window's title.
pub const APP_TITLE: &str = "Metallic Studio";

/// Whether a toolbar tool can be used now (#232): the game's controls as the menus allow them; a plugin's tool only
/// once the plugin API exists.
fn tool_enabled(action: &ToolAction, play: editor::MenuState) -> bool {
    match action {
        ToolAction::Studio(StudioCommand::Play) => play.allows(MenuAction::Play),
        ToolAction::Studio(StudioCommand::Pause) => play.allows(MenuAction::Pause),
        ToolAction::Studio(StudioCommand::Step) => play.allows(MenuAction::Step),
        ToolAction::Studio(StudioCommand::Stop) => play.allows(MenuAction::Stop),
        ToolAction::Studio(StudioCommand::Undo) => play.allows(MenuAction::Undo),
        ToolAction::Studio(StudioCommand::Redo) => play.allows(MenuAction::Redo),
        ToolAction::Studio(_) => true,
        ToolAction::Plugin { .. } => false,
    }
}

/// Studio's interface state: the screen shown, the theme, the settings and what startup loaded.
pub struct Studio {
    screen: Screen,
    splash: Splash,
    /// The accent the splash's bar is drawn in: the saved one, once the shell has peeked at the settings (#319).
    splash_accent: Accent,
    startup: Startup,
    /// What the startup steps produced, once the splash is done: the renderer the viewport will use (#217). The
    /// projects list, the templates and the layout move from it to their owners.
    loaded: Option<Loaded>,
    /// The projects page and its list.
    projects: Projects,
    /// The project the Editor screen shows.
    open_project: Option<PathBuf>,
    /// The Editor screen's project, opening or open.
    editor: Option<EditorScreen>,
    /// The project the command line named, opened once the splash is done.
    open_after_splash: Option<PathBuf>,
    /// Whether the project opening came from the command line, so it joins Recent once it has opened.
    add_to_recent: bool,
    /// The editor's panels and their layout, which outlive any one project.
    dock: Dock,
    /// The editor's toolbar (#232): its tabs and their tools.
    toolbar: Toolbar,
    /// The main layout, kept while a toolbar tab shows a layout of its own.
    main_layout: Option<egui_tiles::Tree<crate::dock::PanelId>>,
    /// The toolbar tab whose own layout the dock shows, by id.
    shown_layout_tab: Option<u32>,
    /// The log the Output panel reads.
    log: Arc<LogBuffer>,
    theme: ThemeState,
    /// The settings: in memory until the splash has read them, then the file's.
    settings: SettingsStore,
    /// The Settings window, on every screen.
    settings_window: SettingsWindow,
    /// Whether the editor's About window is open (Help, About Metallic Studio).
    about_open: bool,
    /// Every shortcut's command and bindings: the settings' overrides over the defaults.
    keymap: Keymap,
    /// What to tell the user (about the settings file, or a new default shortcut left out), first shown first.
    notices: Vec<Notice>,
    /// Whether the system asks for reduced motion (the Appearance setting can follow it).
    system_reduces_motion: bool,
    /// The interface scale last applied.
    applied_scale: Option<f32>,
    /// The custom icon folder's icons (#231).
    icons: CustomIcons,
    /// The emblem, until the first frame uploads it.
    emblem_image: Option<ColorImage>,
    emblem: Option<TextureHandle>,
    /// The mark at the sizes the screens after the splash show it, in pixels: drawn for each, never scaled.
    marks: Vec<(u32, TextureHandle)>,
    requests: Vec<WindowRequest>,
    /// The Game tab's input hand-off: what it remembers between frames.
    game_input: game_input::GameInput,
    /// Whether the running game had the keyboard and mouse after the last frame's hand-off.
    game_had_input: bool,
    /// Game sessions started so far: each gets the next number.
    sessions: u64,
}

impl std::fmt::Debug for Studio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Studio")
            .field("screen", &self.screen)
            .field("startup", &self.startup)
            .field("loaded", &self.loaded)
            .field("projects", &self.projects)
            .field("open_project", &self.open_project)
            .field("dock", &self.dock)
            .finish_non_exhaustive()
    }
}

impl Studio {
    /// Studio on the splash, over `startup`'s steps. `emblem` is the splash's emblem, and `reduce_motion` the
    /// system's preference until the settings say otherwise.
    pub fn new(startup: Startup, emblem: Option<ColorImage>, reduce_motion: bool) -> Self {
        Self {
            screen: Screen::Splash,
            splash: Splash::new(),
            splash_accent: Accent::default(),
            startup,
            loaded: None,
            projects: Projects::new(),
            open_project: None,
            editor: None,
            open_after_splash: None,
            add_to_recent: false,
            dock: Dock::in_memory(panels::PANELS),
            toolbar: Toolbar::in_memory(panels::PANELS),
            main_layout: None,
            shown_layout_tab: None,
            log: Arc::clone(crate::logging::global()),
            // The splash's look: the default dark mode and accent, before the settings are read.
            theme: ThemeState::new(Look {
                reduce_motion,
                ..Look::default()
            }),
            settings: SettingsStore::in_memory(),
            settings_window: SettingsWindow::default(),
            about_open: false,
            keymap: Keymap::studio(&Overrides::new()),
            notices: Vec::new(),
            system_reduces_motion: reduce_motion,
            applied_scale: None,
            icons: CustomIcons::default(),
            emblem_image: emblem,
            emblem: None,
            marks: Vec::new(),
            requests: Vec::new(),
            sessions: 0,
            game_input: game_input::GameInput::default(),
            game_had_input: false,
        }
    }

    /// Draws the splash's bar in `accent`: the one the settings file names, read before the splash's first frame
    /// (#319). The rest of the splash keeps its own look.
    pub fn set_splash_accent(&mut self, accent: Accent) {
        self.splash_accent = accent;
    }

    /// Asks `dialog` for files and folders (the shell gives the system's; until then every dialog is cancelled).
    pub fn set_file_dialog(&mut self, dialog: Box<dyn FileDialog>) {
        self.projects.set_file_dialog(dialog);
    }

    /// Opens the project at `path` once the splash is done, skipping sign-in and the projects page (a `.mtproj` given
    /// on the command line). It joins Recent once it has opened; if it cannot be opened, the projects page says why.
    pub fn open_after_splash(&mut self, path: PathBuf) {
        self.open_after_splash = Some(path);
    }

    /// Has the Output panel read `log` instead of the process's log.
    pub fn set_log(&mut self, log: Arc<LogBuffer>) {
        self.log = log;
    }

    /// The projects page and its list.
    pub fn projects(&self) -> &Projects {
        &self.projects
    }

    /// The projects page and its list, to change.
    pub fn projects_mut(&mut self) -> &mut Projects {
        &mut self.projects
    }

    /// The project the Editor screen shows, opening or open.
    pub fn open_project(&self) -> Option<&Path> {
        self.open_project.as_deref()
    }

    /// The open project and its panels' state, once it has opened.
    pub fn editor(&self) -> Option<&EditorState> {
        match &self.editor {
            Some(EditorScreen::Open(editor)) => Some(editor),
            _ => None,
        }
    }

    /// The open project and its panels' state, to change.
    pub fn editor_mut(&mut self) -> Option<&mut EditorState> {
        match &mut self.editor {
            Some(EditorScreen::Open(editor)) => Some(editor),
            _ => None,
        }
    }

    /// Whether a project is opening behind the progress overlay.
    pub fn is_opening(&self) -> bool {
        matches!(self.editor, Some(EditorScreen::Opening(_)))
    }

    /// The editor's panels and their layout.
    pub fn dock(&self) -> &Dock {
        &self.dock
    }

    /// The editor's panels and their layout, to change.
    pub fn dock_mut(&mut self) -> &mut Dock {
        &mut self.dock
    }

    /// Goes to the projects page, looking at the projects' files again. A project open is closed.
    fn show_projects(&mut self) {
        self.editor = None;
        self.open_project = None;
        self.add_to_recent = false;
        self.projects.refresh();
        self.screen = Screen::Projects;
    }

    /// Goes to the Editor screen and starts opening the project at `path` behind the progress overlay.
    fn open(&mut self, path: PathBuf) {
        log::info!("projects: opening {}", path.display());
        match Opening::spawn(path.clone()) {
            Ok(opening) => {
                self.editor = Some(EditorScreen::Opening(opening));
                self.open_project = Some(path);
                self.screen = Screen::Editor;
            }
            Err(error) => {
                self.projects
                    .set_error(format!("Could not open the project: no thread could start ({error})."));
            }
        }
    }

    /// Opens the project the command line named. A path with no file lands on the projects page with the error;
    /// a file that is not a project fails while opening and lands there the same way.
    fn open_from_command_line(&mut self, path: PathBuf) {
        if !path.is_file() {
            self.show_projects();
            self.projects.set_error(format!(
                "Could not open {}: there is no project file there.",
                path.display()
            ));
            return;
        }
        self.open(path);
        if self.screen == Screen::Editor {
            self.add_to_recent = true;
        } else {
            // No thread could start: the projects page already says so.
            self.show_projects();
        }
    }

    /// The name the projects list knows `path` by, or its file's.
    fn project_name(&self, path: &Path) -> String {
        self.projects
            .list()
            .get(path)
            .map_or_else(|| name_from_path(path), |entry| entry.name.clone())
    }

    /// The mark, `points` square at `ctx`'s scale, drawn for that size the first time it is asked for.
    fn mark(&mut self, ctx: &egui::Context, points: f32) -> TextureHandle {
        let pixels = (points * ctx.pixels_per_point()).round().max(1.0) as u32;
        if let Some((_, texture)) = self.marks.iter().find(|(size, _)| *size == pixels) {
            return texture.clone();
        }
        let mark = emblem::render(pixels);
        let image = ColorImage::from_rgba_unmultiplied([mark.width as usize, mark.height as usize], &mark.rgba);
        let texture = ctx.load_texture(format!("mark-{pixels}"), image, TextureOptions::LINEAR);
        self.marks.push((pixels, texture.clone()));
        texture
    }

    /// The screen shown.
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// What startup loaded, once the splash is done.
    pub fn loaded(&self) -> Option<&Loaded> {
        self.loaded.as_ref()
    }

    /// The renderer the splash made for the viewport, for the viewport to adopt; `None` once taken, or if there was
    /// none.
    pub fn take_viewport_renderer(&mut self) -> Option<metallic_engine::metallic_render::Renderer> {
        self.loaded.as_mut()?.viewport_renderer.take()
    }

    /// The window changes asked for since the last call.
    pub fn take_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.requests)
    }

    /// What the window is cleared to under the interface: nothing around the splash's shape, the theme's ground
    /// after it.
    pub fn clear_color(&self) -> Color32 {
        match self.screen {
            Screen::Splash => Color32::TRANSPARENT,
            _ => self.theme.colors().palette.bg,
        }
    }

    /// The settings now.
    pub fn settings(&self) -> &StudioSettings {
        self.settings.settings()
    }

    /// The Settings pages' state.
    pub fn settings_window(&mut self) -> &mut SettingsWindow {
        &mut self.settings_window
    }

    /// Whether the Settings window shows.
    pub fn is_settings_open(&self) -> bool {
        self.settings_window.is_open()
    }

    /// What Studio is telling the user (about the settings file, or the keymap), until it is dismissed.
    pub fn notice(&self) -> Option<&str> {
        self.notices.first().map(|notice| notice.body.as_str())
    }

    /// The editor's toolbar (#232).
    pub fn toolbar(&self) -> &Toolbar {
        &self.toolbar
    }

    /// The editor's toolbar, to change: the plugin API registers its tools through it.
    pub fn toolbar_mut(&mut self) -> &mut Toolbar {
        &mut self.toolbar
    }

    /// The keymap.
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// The keymap, to change: what the Keybinds page changes is saved with the settings.
    #[cfg(test)]
    pub(crate) fn keymap_mut(&mut self) -> &mut Keymap {
        &mut self.keymap
    }

    /// Draws one frame into `ui`, the whole window, at `now`.
    pub fn ui(&mut self, ui: &mut egui::Ui, now: Instant) {
        // Edits to icons.json show without a restart (#403).
        if crate::theme::icons::builtin::refresh() {
            ui.ctx().request_repaint();
        }
        let ctx = ui.ctx().clone();
        // The splash keeps its own look; the settings apply from the first screen after it.
        if self.screen != Screen::Splash {
            self.apply_settings(&ctx);
        }
        self.theme.apply(&ctx);
        if self.screen != Screen::Splash {
            // Before anything is drawn, so a key a command used reaches nothing else.
            self.run_keymap(&ctx);
        }
        if self.screen == Screen::Editor {
            // After the keymap took the play controls, before any panel sees a key.
            self.hand_off_input(&ctx);
        }
        let before = (self.screen != Screen::Splash).then(|| self.settings.settings().clone());
        match self.screen {
            Screen::Splash => self.splash(ui, now),
            Screen::SignIn => {
                let mark = self.mark(&ctx, sign_in::MARK);
                let skip = &mut self.settings.settings_mut().general.skip_sign_in;
                if sign_in::show(ui, Some(&mark), skip) == sign_in::SignIn::Offline {
                    self.show_projects();
                }
            }
            Screen::Projects => {
                let mark = self.mark(&ctx, RAIL_MARK);
                if let Some(path) = self.projects.show(ui, Some(&mark), &self.keymap) {
                    self.open(path);
                }
            }
            Screen::Editor => self.editor_ui(ui),
        }
        if let Some(before) = before {
            if self.screen != Screen::Editor {
                self.gear(ui);
            }
            self.settings_ui(&ctx, &before);
        }
    }

    /// The Editor screen: the progress overlay while the project opens, then the top bar and the dock.
    fn editor_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(step) = self.poll_opening() {
            let name = self.project_name(&self.open_project.clone().unwrap_or_default());
            editor::opening_overlay(ui, &name, step);
            return;
        }
        if self.screen != Screen::Editor {
            // It could not be opened: the projects page says why from the next frame.
            ui.ctx().request_repaint();
            return;
        }
        let Some(EditorScreen::Open(editor)) = &mut self.editor else {
            // No project: nothing to edit.
            self.show_projects();
            return;
        };
        {
            // What left the game since the last frame is not selected (#347).
            let scene = editor.scene();
            scene.selection.retain_in_game(scene.dm);
        }
        if editor.is_playing() {
            // The game's frame first, so the panels show it; it runs every frame, Game tab in front or not.
            let dt = ctx.input(|input| input.stable_dt);
            editor.tick(std::time::Duration::from_secs_f32(dt.max(0.0)));
            ctx.request_repaint();
        }
        let full = ui.max_rect();
        let bar = Rect::from_min_size(full.min, vec2(full.width(), editor::BAR_HEIGHT));
        let action = ui
            .scope_builder(UiBuilder::new().max_rect(bar), |ui| {
                let settings_open = self.settings_window.is_open();
                editor::top_bar(ui, editor, &self.dock, &self.keymap, settings_open)
            })
            .inner;
        // The toolbar under the menu bar (#232).
        let strip = Rect::from_min_size(
            pos2(full.left(), bar.bottom()),
            vec2(full.width(), toolbar::TOOLBAR_HEIGHT),
        );
        let play = editor::MenuState::of(editor);
        let dock = &self.dock;
        let gizmo = (editor.viewport.gizmo.tool, editor.viewport.gizmo.local);
        let editing = &mut self.settings.settings_mut().editing;
        let ran = ui
            .scope_builder(UiBuilder::new().max_rect(strip), |ui| {
                self.toolbar.ui(
                    ui,
                    |action| tool_enabled(action, play),
                    |action| match action {
                        ToolAction::Studio(StudioCommand::TogglePanel(id)) => dock.is_open(id),
                        ToolAction::Studio(StudioCommand::Transform(tool)) => *tool == gizmo.0,
                        ToolAction::Studio(StudioCommand::LocalSpace) => gizmo.1,
                        _ => false,
                    },
                    editing,
                )
            })
            .inner;
        let body = Rect::from_min_max(pos2(full.left(), strip.bottom()), full.max);
        let p = current_colors(&ctx).palette;
        ui.painter().rect_filled(body, 0.0, p.bg);
        let mut cx = PanelContext {
            editor,
            settings: self.settings.settings_mut(),
        };
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            self.dock.ui(ui, &mut cx);
        });
        crate::panels::insert::frame(&ctx, cx.editor);
        if let Some(action) = action {
            self.menu_action(action);
        }
        for event in ran {
            let toolbar::ToolbarEvent::Run(action) = event;
            self.run_tool(action);
        }
        // What the frame changed is one undo step, unless a button is held: a drag goes on (#348).
        if let Some(editor) = self.editor_mut() {
            let holding = ctx.input(|input| input.pointer.any_down());
            editor.scene().commit(Instant::now(), holding);
        }
        self.show_toolbar_layout();
        // After the menus' action, so it opens in the frame it was chosen in, as the Settings window does.
        if self.about_open {
            let mark = self.mark(&ctx, editor::ABOUT_MARK);
            editor::about_window(&ctx, &mut self.about_open, Some(&mark));
        }
        // Not while a tab or a divider is dragged: the layout is written once, when the drag ends.
        let busy = ctx.input(|input| input.pointer.any_down()) || self.dock.is_dragging(&ctx);
        if let Err(error) = self.dock.save_if_changed(busy) {
            log::warn!("layout: could not save it: {error}");
        }
        if let Err(error) = self.toolbar.save_if_changed(busy) {
            log::warn!("toolbar: could not save it: {error}");
        }
    }

    /// Takes what the opening worker reported. Returns the step it is at while it still runs; once it is done the
    /// editor has the project, or the projects page says why it could not be opened.
    fn poll_opening(&mut self) -> Option<usize> {
        let Some(EditorScreen::Opening(opening)) = &mut self.editor else {
            return None;
        };
        match opening.poll() {
            None => Some(opening.step()),
            Some(Ok(opened)) => {
                if std::mem::take(&mut self.add_to_recent)
                    && let Some(path) = self.open_project.clone()
                {
                    // Only now, so a file that is not a project never joins Recent.
                    let name = match opened.project.manifest().name.trim() {
                        "" => name_from_path(&path),
                        name => name.to_owned(),
                    };
                    let list = self.projects.list_mut();
                    list.opened(&path, &name, std::time::SystemTime::now());
                    list.save_or_log();
                    self.projects.select(path);
                }
                let editor = EditorState::new(*opened, Arc::clone(&self.log));
                self.editor = Some(EditorScreen::Open(Box::new(editor)));
                None
            }
            Some(Err(error)) => {
                let name = self.project_name(&self.open_project.clone().unwrap_or_default());
                self.show_projects();
                self.projects
                    .set_error(format!("Could not open \u{201C}{name}\u{201D}. {error}"));
                None
            }
        }
    }

    /// Does what a menu item asked for.
    fn menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::Save => self.save(),
            MenuAction::Close => self.show_projects(),
            MenuAction::Toggle(id) => self.dock.toggle(id),
            MenuAction::ResetLayout => self.reset_layout(),
            MenuAction::Settings => self.toggle_settings(),
            MenuAction::KeyboardShortcuts => {
                self.settings_window.show_category(crate::settings::Category::Keybinds);
                self.settings_window.set_open(true);
            }
            MenuAction::About => self.about_open = true,
            MenuAction::Play => self.play(),
            MenuAction::Pause => {
                if let Some(editor) = self.editor_mut() {
                    editor.toggle_pause();
                }
            }
            MenuAction::Step => {
                if let Some(editor) = self.editor_mut() {
                    editor.step();
                }
            }
            MenuAction::Stop => self.stop(),
            MenuAction::Undo => {
                if let Some(editor) = self.editor_mut() {
                    editor.scene().undo(Instant::now());
                }
            }
            MenuAction::Redo => {
                if let Some(editor) = self.editor_mut() {
                    editor.scene().redo(Instant::now());
                }
            }
            MenuAction::Transform(tool) => {
                if let Some(editor) = self.editor_mut() {
                    editor.viewport.gizmo.tool = tool;
                }
            }
            MenuAction::Snap => {
                let editing = &mut self.settings.settings_mut().editing;
                let on = !(editing.snap_moves || editing.snap_turns);
                editing.snap_moves = on;
                editing.snap_turns = on;
            }
            MenuAction::LocalSpace => {
                if let Some(editor) = self.editor_mut() {
                    editor.viewport.gizmo.local = !editor.viewport.gizmo.local;
                }
            }
            MenuAction::QuarterTurn { tilt } => {
                if let Some(editor) = self.editor_mut()
                    && !editor.is_playing()
                    && !editor.viewport.gizmo.is_dragging()
                {
                    let local = editor.viewport.gizmo.local;
                    crate::viewport::tools::quarter_turn(&mut editor.scene(), local, tilt);
                }
            }
            MenuAction::Tree(edit) => {
                if let Some(editor) = self.editor_mut() {
                    editor.scene().tree_edit(edit);
                }
            }
            MenuAction::SelectAll => {
                if let Some(editor) = self.editor_mut() {
                    let scene = editor.scene();
                    scene.explorer.select_all(scene.dm, scene.selection);
                }
            }
        }
    }

    /// Does what a toolbar tool asks (#232): Studio's own commands as the menus run them. A plugin's tool reaches its
    /// plugin through the plugin API, once there is one.
    fn run_tool(&mut self, action: ToolAction) {
        let menu = match action {
            ToolAction::Studio(command) => match command {
                StudioCommand::Save => MenuAction::Save,
                StudioCommand::Close => MenuAction::Close,
                StudioCommand::Play => MenuAction::Play,
                StudioCommand::Pause => MenuAction::Pause,
                StudioCommand::Step => MenuAction::Step,
                StudioCommand::Stop => MenuAction::Stop,
                StudioCommand::Undo => MenuAction::Undo,
                StudioCommand::Transform(tool) => MenuAction::Transform(tool),
                StudioCommand::LocalSpace => MenuAction::LocalSpace,
                StudioCommand::Snap => MenuAction::Snap,
                StudioCommand::Redo => MenuAction::Redo,
                StudioCommand::ResetLayout => MenuAction::ResetLayout,
                StudioCommand::Settings => MenuAction::Settings,
                StudioCommand::TogglePanel(id) => MenuAction::Toggle(id),
            },
            ToolAction::Plugin { plugin, command } => {
                log::warn!("toolbar: {plugin}'s `{command}` needs the plugin API, which Studio does not have yet");
                return;
            }
        };
        self.menu_action(menu);
    }

    /// Shows the dock layout the toolbar's front tab asks for (#232): a tab with "Load a layout for this tab" gets
    /// its own (starting from the main one the first time), kept up to date while it shows; any other tab, the main
    /// layout, which alone is written to `layout.json`.
    fn show_toolbar_layout(&mut self) {
        use crate::dock::layout::{from_json, to_json};

        let want = self.toolbar.layout_tab();
        if want == self.shown_layout_tab {
            // The tab's layout follows what the user does to it.
            if let Some(id) = want {
                let json = to_json(&self.dock.persisted());
                if let Some(tab) = self.toolbar.config_mut().tab_mut(id)
                    && tab.layout.as_deref() != Some(json.as_str())
                {
                    tab.layout = Some(json);
                }
            }
            return;
        }
        // Leaving: the main layout is kept (and written, if it changed), or the tab's layout was kept up to date.
        if self.shown_layout_tab.is_none() {
            if let Err(error) = self.dock.save_if_changed(false) {
                log::warn!("layout: could not save it: {error}");
            }
            self.main_layout = Some(self.dock.persisted());
        }
        let playing = self.editor().is_some_and(EditorState::is_playing);
        match want {
            Some(id) => {
                let own = self
                    .toolbar
                    .config()
                    .tab(id)
                    .and_then(|tab| tab.layout.as_deref())
                    .and_then(|json| from_json(json, panels::PANELS).ok());
                let tree = own
                    .or_else(|| self.main_layout.clone())
                    .unwrap_or_else(|| self.dock.persisted());
                self.dock.set_saving(false);
                self.dock.replace(tree);
            }
            None => {
                let main = self.main_layout.take().unwrap_or_else(|| self.dock.persisted());
                self.dock.replace(main);
                self.dock.set_saving(true);
            }
        }
        if playing {
            // The Game tab is the session's, whichever layout shows.
            self.dock.open(panels::ids::GAME);
        }
        self.shown_layout_tab = want;
    }

    /// The window's title (#254): the project's name with a `*` while it has unsaved changes, then the app's, in the
    /// editor (`Space Race* - Metallic Studio`), and the app's alone elsewhere. The shell sets it when it changes.
    pub fn window_title(&self) -> String {
        if self.screen != Screen::Editor {
            return APP_TITLE.to_owned();
        }
        match (self.editor(), self.open_project.as_deref()) {
            (Some(editor), _) => {
                let unsaved = if editor.is_modified() { "*" } else { "" };
                format!("{}{unsaved} - {APP_TITLE}", editor.name())
            }
            // Opening: the name the projects list knows it by.
            (None, Some(path)) => format!("{} - {APP_TITLE}", self.project_name(path)),
            (None, None) => APP_TITLE.to_owned(),
        }
    }

    /// Whether a game session runs in the editor (Play in Studio).
    pub fn is_playing(&self) -> bool {
        self.editor().is_some_and(EditorState::is_playing)
    }

    /// Whether the running game has the keyboard and mouse (ADR 0010 §4): the Game tab has the focus (Play gives it, a
    /// click in the tab gives it back, Esc and a click elsewhere take it), the Settings window does not, and Studio's
    /// window is focused.
    pub fn game_has_input(&self, ctx: &egui::Context) -> bool {
        self.is_playing()
            && self.dock.focus().focused == Some(panels::ids::GAME)
            && !self.settings_window.has_focus()
            && ctx.input(|input| input.focused)
    }

    /// Where the editor camera holds the pointer while it looks or orbits (#385), in points: the shell keeps it there,
    /// shown.
    pub fn held_pointer(&self) -> Option<egui::Pos2> {
        match &self.editor {
            Some(EditorScreen::Open(editor)) if !self.game_had_input => editor.viewport.hold,
            _ => None,
        }
    }

    /// What the running game asks of the cursor while it has the input, and the Game tab's area (points): what the
    /// shell holds or hides its cursor for. `None` while the game does not have the input.
    pub fn game_cursor(&self) -> Option<(CursorState, Rect)> {
        if !self.game_had_input {
            return None;
        }
        let session = self.editor()?.play.as_ref()?;
        Some((session.player.cursor(), session.view.rect?))
    }

    /// The frame's hand-off to the game (ADR 0010 §4): when it takes or lets go of the input, it gets the focus or
    /// releases what it holds; while it has the input, the frame's keys, text, pointer and wheel become its events
    /// and the panels see no keys. Esc goes to the game, then the input comes back to Studio.
    fn hand_off_input(&mut self, ctx: &egui::Context) {
        let has_input = self.game_has_input(ctx);
        let Some(EditorScreen::Open(editor)) = &mut self.editor else {
            self.game_had_input = false;
            return;
        };
        let Some(session) = editor.play.as_mut() else {
            self.game_had_input = false;
            return;
        };
        let engine = session.player.engine_mut();
        if has_input != self.game_had_input {
            let events = if has_input {
                self.game_input.take(ctx.input(|input| input.modifiers))
            } else {
                self.game_input.release()
            };
            for event in events {
                engine.push_input(event);
            }
            self.game_had_input = has_input;
        }
        if !has_input {
            return;
        }
        let Some(rect) = session.view.rect else {
            return;
        };
        // A click anywhere but the Game tab (a menu, the top bar, a panel) takes the input back.
        let elsewhere = ctx.input(|input| {
            input.pointer.any_pressed() && input.pointer.interact_pos().is_some_and(|at| !rect.contains(at))
        });
        if elsewhere {
            if self.dock.focus().focused == Some(panels::ids::GAME) {
                self.dock.set_focused(None);
            }
            for event in self.game_input.release() {
                engine.push_input(event);
            }
            self.game_had_input = false;
            return;
        }
        let events = ctx.input(|input| input.events.clone());
        let converted = self.game_input.convert(&events, rect, ctx.pixels_per_point());
        for event in converted.events {
            engine.push_input(event);
        }
        // The keys were the game's: no panel or widget acts on them too.
        ctx.input_mut(|input| {
            input
                .events
                .retain(|event| !matches!(event, egui::Event::Key { .. } | egui::Event::Text(_)));
        });
        if converted.escape {
            self.dock.set_focused(None);
            for event in self.game_input.release() {
                engine.push_input(event);
            }
            self.game_had_input = false;
        }
    }

    /// Opens Insert Object (#411) for the selected instance, or Workspace with nothing selected.
    fn open_insert(&mut self) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let scene = editor.scene();
        let parent = scene
            .selection
            .active()
            .or_else(|| crate::viewport::pick::workspace(scene.dm));
        if let Some(parent) = parent {
            scene.explorer.open_insert(parent);
        }
    }

    /// Starts a game session of the open project (ADR 0010): the scene as it is now runs in an embedded Player, drawn
    /// with the Rendering settings, its output in Output, and the Game tab opens in front with the focus. Tells the
    /// user why if it cannot start.
    fn play(&mut self) {
        let graphics = self.settings.settings().rendering.clone();
        let log = Arc::clone(&self.log);
        self.sessions += 1;
        let number = self.sessions;
        let Some(editor) = self.editor_mut() else {
            return;
        };
        if editor.is_playing() {
            return;
        }
        match editor.play(graphics, log, number) {
            Ok(()) => {
                self.dock.open(panels::ids::GAME);
                self.dock.set_focused(Some(panels::ids::GAME));
            }
            Err(error) => {
                log::warn!("play: the game could not start: {error}");
                self.notices.push(Notice {
                    title: "The game could not start",
                    body: error,
                });
            }
        }
    }

    /// Ends the game session: the Game tab closes, the Viewport's comes to the front, and the scene is as it was
    /// before Play.
    fn stop(&mut self) {
        if self.editor_mut().is_some_and(EditorState::stop) {
            self.dock.close(panels::ids::GAME);
            self.dock.open(panels::ids::VIEWPORT);
        }
    }

    /// Puts the panels back where they start; the Game tab stays while a game runs.
    fn reset_layout(&mut self) {
        self.dock.reset();
        if self.is_playing() {
            self.dock.open(panels::ids::GAME);
        }
    }

    /// Whether the editor's About window is open.
    pub fn is_about_open(&self) -> bool {
        self.about_open
    }

    /// Saves the open project, telling the user if it failed.
    fn save(&mut self) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        if editor.is_playing() {
            // ADR 0010 §3: a save never looks like saving the game's state.
            log::warn!("projects: saving is off while the game runs; stop it first");
            return;
        }
        if let Err(error) = editor.save() {
            log::warn!("projects: could not save: {error}");
            self.notices.push(Notice {
                title: "The project was not saved",
                body: error,
            });
        }
    }

    /// Gives the interface the settings' look and scale. Each is applied only when it changed.
    fn apply_settings(&mut self, ctx: &egui::Context) {
        let appearance = &self.settings.settings().appearance;
        self.theme.set(appearance.look(self.system_reduces_motion));
        // Every frame: it also takes in a folder read in the background (#231).
        self.icons.update(ctx, &appearance.icon_folder);
        if self.applied_scale != Some(appearance.ui_scale) {
            // The scale is the setting's alone: egui's own Ctrl +/- zoom would drift from it unseen.
            ctx.options_mut(|options| options.zoom_with_keyboard = false);
            ctx.set_zoom_factor(appearance.ui_scale);
            self.applied_scale = Some(appearance.ui_scale);
        }
    }

    /// The gear in the screen's top-right corner, which opens and closes Settings. The editor has its own, in its
    /// top bar.
    fn gear(&mut self, ui: &mut egui::Ui) {
        let p = current_colors(ui.ctx()).palette;
        let screen = ui.max_rect();
        let at = egui::Rect::from_min_size(
            pos2(screen.right() - space::S5 - size::CONTROL, screen.top() + space::S5),
            vec2(size::CONTROL, size::CONTROL),
        );
        let response = ui.interact(at, ui.scope_id().with("gear"), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Settings"));
        let hovered = response.hovered();
        if hovered {
            ui.painter()
                .rect_filled(at, CornerRadius::same(radius::ITEM), p.surface_hover);
        }
        icons::paint_at(
            ui.painter(),
            at.center(),
            egui::Align2::CENTER_CENTER,
            Icon::ToolbarSettings,
            text::TITLE,
            if hovered { p.text } else { p.muted },
        );
        // Every binding of the command, not just the first.
        let tip = match self.keymap.shortcut_text(ids::SETTINGS_OPEN) {
            Some(keys) => format!("Settings ({keys})"),
            None => "Settings".to_owned(),
        };
        if response.on_hover_text(tip).clicked() {
            self.toggle_settings();
        }
    }

    /// Opens or closes the Settings window, on any screen.
    fn toggle_settings(&mut self) {
        let open = self.settings_window.is_open();
        self.settings_window.set_open(!open);
    }

    /// Runs the frame's input through the keymap and does what the commands ask. Every Studio shortcut is here; no
    /// screen or panel reads keys itself.
    fn run_keymap(&mut self, ctx: &egui::Context) {
        if self.settings_window.is_capturing() {
            // A keybind is being recorded: its keys are recorded, not obeyed (#266).
            self.settings_window.listen(ctx, &mut self.keymap);
            return;
        }
        // In the editor, the focused panel's context, and the one under the pointer for navigation, unless the
        // Settings window has the focus or the pointer. The projects page has the focus unless a dialog or the
        // Settings window is over it.
        let editing = self.editor().is_some();
        let over_settings = ctx
            .input(|input| input.pointer.latest_pos())
            .is_some_and(|at| self.settings_window.contains(at));
        let (context, hovered) = match self.screen {
            // The editor's own commands (save, close, layout) still run; the panels' do not.
            Screen::Editor if editing && self.settings_window.has_focus() => (Context::Editor, None),
            Screen::Editor if editing => (
                self.dock.context(),
                self.dock.hovered_context().filter(|_| !over_settings),
            ),
            Screen::Projects if !self.projects.is_creating() && !self.settings_window.is_open() => {
                (Context::Projects, None)
            }
            _ => (Context::Global, None),
        };
        let focus = Focus {
            context,
            hovered,
            typing: ctx.egui_wants_keyboard_input(),
            playing: self.game_has_input(ctx),
        };
        let activations = self.keymap.dispatch(ctx, focus);
        if let Some(EditorScreen::Open(editor)) = &mut self.editor {
            let (motion, pointer, dt, fast) = ctx.input(|input| {
                // The mouse's own movement while the pointer is held (#385): the pointer itself does not move.
                let motion = input.pointer.motion().unwrap_or(input.pointer.delta());
                (
                    motion,
                    input.pointer.latest_pos(),
                    input.stable_dt,
                    input.modifiers.shift,
                )
            });
            let input = crate::viewport::NavInput {
                motion,
                pointer,
                dt,
                fast,
                dm: &editor.dm,
                selection: editor.selection.ids(),
            };
            let speeds = &mut self.settings.settings_mut().viewport;
            if editor.viewport.navigate(&self.keymap, &activations, input, speeds) {
                ctx.request_repaint();
            }
        }
        for activation in activations {
            if activation.kind != ActivationKind::Press {
                continue;
            }
            match activation.command {
                ids::SETTINGS_OPEN => self.toggle_settings(),
                ids::PROJECT_CLOSE if editing => self.show_projects(),
                ids::FILE_SAVE if editing => self.save(),
                ids::VIEW_RESET_LAYOUT if editing => self.reset_layout(),
                ids::TEST_PLAY if editing => self.menu_action(MenuAction::Play),
                ids::TEST_PAUSE if editing => self.menu_action(MenuAction::Pause),
                ids::TEST_STEP if editing => self.menu_action(MenuAction::Step),
                ids::TEST_STOP if editing => self.menu_action(MenuAction::Stop),
                ids::EDIT_SELECT_ALL if editing => self.menu_action(MenuAction::SelectAll),
                ids::EDIT_INSERT if editing => self.open_insert(),
                ids::EDIT_UNDO if editing => self.menu_action(MenuAction::Undo),
                ids::EDIT_REDO if editing => self.menu_action(MenuAction::Redo),
                ids::TOOL_SELECT if editing => self.menu_action(MenuAction::Transform(Tool::Select)),
                ids::TOOL_MOVE if editing => self.menu_action(MenuAction::Transform(Tool::Move)),
                ids::TOOL_SCALE if editing => self.menu_action(MenuAction::Transform(Tool::Scale)),
                ids::TOOL_ROTATE if editing => self.menu_action(MenuAction::Transform(Tool::Rotate)),
                ids::TOOL_LOCAL_SPACE if editing => self.menu_action(MenuAction::LocalSpace),
                ids::TOOL_TURN if editing => self.menu_action(MenuAction::QuarterTurn { tilt: false }),
                ids::TOOL_TILT if editing => self.menu_action(MenuAction::QuarterTurn { tilt: true }),
                id @ (ids::EDIT_CUT
                | ids::EDIT_COPY
                | ids::EDIT_PASTE
                | ids::EDIT_PASTE_INTO
                | ids::EDIT_DUPLICATE
                | ids::EDIT_RENAME
                | ids::EDIT_DELETE)
                    if editing =>
                {
                    let edit = match id {
                        ids::EDIT_CUT => TreeEdit::Cut,
                        ids::EDIT_COPY => TreeEdit::Copy,
                        ids::EDIT_PASTE => TreeEdit::Paste,
                        ids::EDIT_PASTE_INTO => TreeEdit::PasteInto,
                        ids::EDIT_DUPLICATE => TreeEdit::Duplicate,
                        ids::EDIT_RENAME => TreeEdit::Rename,
                        _ => TreeEdit::Delete,
                    };
                    self.menu_action(MenuAction::Tree(edit));
                }
                id @ (ids::PROJECTS_OPEN | ids::PROJECTS_NEW | ids::PROJECTS_ADD) if context == Context::Projects => {
                    if let Some(path) = self.projects.run(id) {
                        self.open(path);
                    }
                }
                // The viewport's navigation is the editor camera's, above.
                _ => {}
            }
        }
    }

    /// The Settings window, the notices, and saving what changed since `before`.
    fn settings_ui(&mut self, ctx: &egui::Context, before: &StudioSettings) {
        self.settings_window
            .show(ctx, self.settings.settings_mut(), &mut self.keymap);
        if self.keymap.take_changed() {
            self.settings.settings_mut().keybinds.overrides = self.keymap.overrides().clone();
        }
        if self.settings.settings() != before {
            // Restyled at the start of the next pass, which this asks for at once.
            ctx.request_repaint();
        }
        self.show_notice(ctx);
        // Not while a value is dragged, so a drag writes once when it ends rather than every frame.
        if !ctx.input(|input| input.pointer.any_down())
            && let Err(error) = self.settings.save_if_changed()
        {
            log::warn!("settings: could not save them: {error}");
        }
    }

    /// The first notice, if any, at the bottom of the window until dismissed.
    fn show_notice(&mut self, ctx: &egui::Context) {
        let Some(Notice { title, body: notice }) = self.notices.first() else {
            return;
        };
        let colors = current_colors(ctx);
        let p = colors.palette;
        let mut dismissed = false;
        egui::Area::new(Id::unique("metallic.settings.notice"))
            .anchor(Align2::CENTER_BOTTOM, [0.0, -space::S8])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(size::BORDER, p.border))
                    .corner_radius(CornerRadius::same(radius::PANEL))
                    .shadow(shadow::MENU)
                    .inner_margin(space::S6)
                    .show(ui, |ui| {
                        ui.set_width(520.0);
                        ui.horizontal(|ui| {
                            icons::show(ui, Icon::UiWarning, text::TITLE, colors.accent);
                            ui.label(
                                RichText::new(*title)
                                    .font(fonts::named(fonts::STRONG, text::BODY_LG))
                                    .color(p.text),
                            );
                        });
                        ui.add(Label::new(RichText::new(notice.as_str()).color(p.muted)).wrap());
                        ui.add_space(space::S4);
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            dismissed = button(ui, ButtonKind::Secondary, "Dismiss").clicked();
                        });
                    });
            });
        if dismissed {
            self.notices.remove(0);
        }
    }

    /// Builds the keymap from the settings' overrides. A new default that would take one of the user's keys is left
    /// out: the user is told, and the choice is saved so it is told once.
    fn load_keymap(&mut self) {
        self.keymap = Keymap::studio(&self.settings.settings().keybinds.overrides);
        for body in self.keymap.take_notices() {
            self.notices.push(Notice {
                title: "A new shortcut was left out",
                body,
            });
        }
        if self.keymap.take_changed() {
            self.settings.settings_mut().keybinds.overrides = self.keymap.overrides().clone();
        }
    }

    fn splash(&mut self, ui: &mut egui::Ui, now: Instant) {
        self.startup.poll();
        if let Some(image) = self.emblem_image.take() {
            self.emblem = Some(ui.ctx().load_texture("emblem", image, TextureOptions::LINEAR));
        }
        let status = Status {
            steps: self.startup.names(),
            done: self.startup.done(),
        };
        let motion = self.theme.look().motion();
        self.splash
            .show(ui, now, status, self.emblem.as_ref(), motion, self.splash_accent);

        if self.splash.finished(self.startup.is_finished(), now) {
            let mut loaded = self.startup.take_loaded().unwrap_or_default();
            self.settings = std::mem::take(&mut loaded.settings);
            if let Some(body) = self.settings.take_notice() {
                self.notices.push(Notice {
                    title: "Settings were reset",
                    body,
                });
            }
            self.load_keymap();
            let mut list = std::mem::take(&mut loaded.projects);
            if let Some(body) = list.take_notice() {
                self.notices.push(Notice {
                    title: "The projects list was reset",
                    body,
                });
            }
            self.projects.set_list(list);
            self.projects.set_templates(std::mem::take(&mut loaded.templates));
            self.projects.set_games_folder(loaded.games_folder.clone());
            self.dock = std::mem::take(&mut loaded.layout);
            self.toolbar = std::mem::take(&mut loaded.toolbar);
            self.loaded = Some(loaded);
            self.screen = if self.settings.settings().general.skip_sign_in {
                Screen::Projects
            } else {
                Screen::SignIn
            };
            // A project from the command line skips sign-in and the projects page (ADR 0009).
            if let Some(path) = self.open_after_splash.take() {
                self.open_from_command_line(path);
            }
            self.requests.push(WindowRequest::BecomeMainWindow);
        }
        // Polls the steps and runs the minimum time out; vsync paces it.
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use std::path::PathBuf;

    use egui::{Key, Modifiers, Role};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::offscreen::{self, Offscreen, read_texture};
    use super::startup::{Step, projects_step, settings_step, wait};
    use super::*;
    use crate::projects::dialog::ScriptedDialog;
    use crate::projects::{PROJECTS_FILE, View, templates_dir};
    use crate::settings::keybinds::LISTENING;
    use crate::settings::{Category, SETTINGS_FILE};
    use crate::theme::tokens;
    use crate::theme::{ThemeMode, fonts};

    /// A settings file of its own for `test`, holding `text` (none when `None`).
    fn settings_file(test: &str, text: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-studio-app-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SETTINGS_FILE);
        if let Some(text) = text {
            std::fs::write(&path, text).unwrap();
        }
        path
    }

    /// Studio in a test harness at a settable clock, starting at `start`.
    struct Test {
        studio: Studio,
        now: Instant,
    }

    fn harness(steps: Vec<Step>) -> Harness<'static, Test> {
        let studio = Studio::new(Startup::spawn(steps).unwrap(), None, false);
        Harness::builder()
            .with_size(splash::SIZE)
            .with_context_setup(fonts::install)
            .build_ui_state(
                |ui, test: &mut Test| test.studio.ui(ui, test.now),
                Test {
                    studio,
                    now: Instant::now(),
                },
            )
    }

    /// The real settings step over `path`, and the real projects step over `projects.toml` beside it and the
    /// repository's templates.
    fn steps_reading(path: PathBuf) -> Vec<Step> {
        let list = path.with_file_name(PROJECTS_FILE);
        vec![
            settings_step(Some(path)),
            projects_step(Some(list), templates_dir(), None),
        ]
    }

    /// Steps whose settings file says light and mint.
    fn quick_steps() -> Vec<Step> {
        let path = settings_file(
            &format!("quick-{:?}", std::thread::current().id()),
            Some("[appearance]\nmode = \"light\"\naccent = \"mint\"\n"),
        );
        steps_reading(path)
    }

    /// Studio past its splash on the settings file at `path`, at the main window's size.
    fn past_the_splash(path: PathBuf) -> Harness<'static, Test> {
        let mut harness = harness(steps_reading(path));
        finish_steps(&mut harness, splash::MIN_TIME);
        harness.set_size(egui::vec2(1280.0, 800.0));
        // The sign-in card centres itself by its height in the frame before.
        settle(&mut harness);
        harness
    }

    /// Runs the frames that show what a click did: the one that handles it and the one that draws the result.
    fn settle(harness: &mut Harness<'_, Test>) {
        harness.step();
        harness.step();
    }

    /// Runs the steps out and moves the clock `by` past the splash's first frame.
    fn finish_steps(harness: &mut Harness<'_, Test>, by: Duration) {
        harness.step();
        wait(&mut harness.state_mut().studio.startup);
        harness.state_mut().now += by;
        harness.step();
    }

    #[test]
    fn the_splash_s_bar_is_in_the_saved_accent() {
        // The bar is filled once a step is done: the first of two.
        let (_go, gate) = mpsc::channel::<()>();
        let mut harness = harness(vec![
            Step::new("Reading settings", |_| {}),
            Step::new("Starting the renderer", move |_| {
                let _ = gate.recv();
            }),
        ]);
        harness.state_mut().studio.set_splash_accent(Accent::Blue);
        harness.step();
        while harness.state().studio.startup.done() < 1 {
            harness.state_mut().studio.startup.poll();
            std::thread::yield_now();
        }
        harness.state_mut().now += Duration::from_secs(2);
        harness.step();
        harness.step();
        let filled = |color: egui::Color32| {
            harness.output().shapes.iter().any(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) => rect.fill == color,
                _ => false,
            })
        };
        assert!(filled(Accent::Blue.on(true)), "the bar is blue");
        assert!(!filled(Accent::default().on(true)), "not the default orange");
    }

    #[test]
    fn the_splash_shows_the_running_step() {
        let (go, gate) = mpsc::channel::<()>();
        let mut harness = harness(vec![
            Step::new("Reading settings", |_| {}),
            Step::new("Starting the renderer", move |_| gate.recv().unwrap()),
            Step::new("Reading recent projects", |_| {}),
        ]);
        // Until the second step reports, the first is named.
        for _ in 0..2000 {
            harness.step();
            if harness.query_by_label("Starting the renderer").is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        harness.get_by_label("Starting the renderer");
        harness.get_by_label("2 of 3");
        harness.get_by_label("Metallic");
        harness.get_by_label("Studio");
        harness.get_by_label(&format!("Version {}", crate::VERSION));
        // Blocked on a step, the splash stays however long it takes.
        harness.state_mut().now += Duration::from_secs(5);
        harness.step();
        assert_eq!(harness.state().studio.screen(), Screen::Splash);

        go.send(()).unwrap();
        wait(&mut harness.state_mut().studio.startup);
        harness.step();
        assert_eq!(harness.state().studio.screen(), Screen::SignIn);
    }

    #[test]
    fn the_splash_never_finishes_before_its_minimum_time() {
        let mut harness = harness(quick_steps());
        finish_steps(&mut harness, Duration::from_millis(100));
        assert!(harness.state().studio.startup.is_finished());
        assert_eq!(
            harness.state().studio.screen(),
            Screen::Splash,
            "every step done, but only 100 ms shown"
        );
        assert!(harness.state_mut().studio.take_requests().is_empty());
        harness.get_by_label("Reading recent projects");
        harness.get_by_label("2 of 2");

        harness.state_mut().now += splash::MIN_TIME;
        harness.step();
        assert_eq!(harness.state().studio.screen(), Screen::SignIn);
    }

    #[test]
    fn the_screens_go_splash_sign_in_projects_editor() {
        let mut harness = harness(quick_steps());
        assert_eq!(harness.state().studio.clear_color(), Color32::TRANSPARENT);
        finish_steps(&mut harness, splash::MIN_TIME);
        let studio = &mut harness.state_mut().studio;
        assert_eq!(studio.screen(), Screen::SignIn);
        assert_eq!(studio.take_requests(), [WindowRequest::BecomeMainWindow]);
        assert!(studio.take_requests().is_empty(), "asked once");
        assert!(studio.loaded().is_some());

        harness.set_size(egui::vec2(1280.0, 800.0));
        harness.step();
        // The settings' look replaces the splash's dark one.
        let studio = &harness.state().studio;
        assert_eq!(studio.theme.look().mode, ThemeMode::Light);
        assert_eq!(studio.theme.colors().accent, Accent::Mint.colors().light);
        assert_eq!(studio.clear_color(), tokens::LIGHT.bg);
        harness.get_by_label("Sign in");
        harness.get_by_label("WELCOME");
        harness.get_by_label("Online services aren't available yet.");
        // A click switches the screen in its frame; the next frame draws the new one.
        let click = |harness: &mut Harness<'_, Test>, label: &str| {
            harness.get_by_label(label).click();
            harness.step();
            harness.step();
        };
        click(&mut harness, "Continue offline");
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_role_and_label(Role::Tab, "Recent");
        harness.get_by_label("No projects yet");

        let dir = std::env::temp_dir().join(format!("metallic-studio-app-screens-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = make_project_once(&dir, "A Game");
        harness.state_mut().studio.open(path.clone());
        assert_eq!(harness.state().studio.screen(), Screen::Editor);
        assert!(harness.state().studio.is_opening());
        wait_open(&mut harness);
        assert_eq!(harness.state().studio.editor().unwrap().path(), path);
        assert!(harness.state().studio.window_title().starts_with("A Game"));

        click(&mut harness, "File");
        harness.get_by_role_and_label(Role::MenuItem, "Close project").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        assert!(harness.state().studio.editor().is_none());
        assert!(harness.state_mut().studio.take_requests().is_empty());
    }

    #[test]
    fn skipping_sign_in_goes_straight_to_the_projects() {
        let path = settings_file("skip", Some("[general]\nskip_sign_in = true\n"));
        let harness = past_the_splash(path);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        assert!(harness.state().studio.notice().is_none());
    }

    #[test]
    fn settings_draws_lucide_in_place_of_egui_s_own_glyphs() {
        let mut harness = past_the_splash(settings_file("lucide-widgets", None));
        let window = harness.state_mut().studio.settings_window();
        window.set_open(true);
        window.show_category(Category::Appearance);
        settle(&mut harness);
        // The drop-down's arrow is Lucide's chevron (#278), not egui's triangle.
        assert!(
            !icon_meshes(&harness, Icon::UiCollapse).is_empty(),
            "the Interface scale drop-down"
        );
        // The window's own header closes it with Lucide's X.
        assert!(!icon_meshes(&harness, Icon::UiClose).is_empty());
        harness.get_by_role_and_label(Role::Button, "Close Settings").click();
        settle(&mut harness);
        assert!(!harness.state().studio.is_settings_open());
    }

    #[test]
    fn switching_the_theme_in_settings_restyles_at_once_and_saves() {
        let path = settings_file("live", None);
        let mut harness = past_the_splash(path.clone());
        let style = |harness: &Harness<'_, Test>| harness.ctx.style_of(harness.ctx.theme());
        assert_eq!(style(&harness).visuals.panel_fill, tokens::DARK.bg);
        assert!(!path.exists(), "nothing changed yet, so nothing written");

        harness.get_by_role_and_label(Role::Button, "Settings").click();
        settle(&mut harness);
        assert!(harness.state().studio.is_settings_open());
        harness.get_by_role_and_label(Role::Tab, "Appearance").click();
        settle(&mut harness);
        assert_eq!(
            harness.state_mut().studio.settings_window().category(),
            Category::Appearance
        );
        // The window keeps its size: no row asks for more width than it has.
        for _ in 0..5 {
            harness.step();
        }
        // The window: the largest thing called Settings (the gear is too).
        let window = harness
            .query_all_by_label("Settings")
            .map(|node| node.rect())
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap();
        assert!(window.width() < 900.0 && window.right() <= 1280.0, "{window:?}");

        harness.get_by_label("Light").click();
        settle(&mut harness);
        let studio = &harness.state().studio;
        assert_eq!(studio.settings().appearance.mode, ThemeMode::Light);
        assert!(!studio.theme.colors().dark);
        assert_eq!(studio.clear_color(), tokens::LIGHT.bg);
        assert_eq!(style(&harness).visuals.panel_fill, tokens::LIGHT.bg);
        assert!(!style(&harness).visuals.dark_mode);

        harness
            .get_by_role_and_label(Role::RadioButton, Accent::Blue.label())
            .click();
        settle(&mut harness);
        assert_eq!(
            harness.state().studio.theme.colors().accent,
            Accent::Blue.colors().light
        );

        // #263: the custom colour, after the swatches, shows the palette and does nothing until there is a picker.
        let custom = harness.get_by_role_and_label(Role::Button, "Custom colour");
        assert!(egui_kittest::kittest::NodeT::accesskit_node(&custom).is_disabled());
        let blue = harness
            .get_by_role_and_label(Role::RadioButton, Accent::Blue.label())
            .rect();
        assert!(custom.rect().left() > blue.right(), "after the swatches");
        custom.click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.settings().appearance.accent, Accent::Blue);

        harness.get_by_role_and_label(Role::CheckBox, "High contrast").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.theme.colors().palette.muted, tokens::LIGHT.text);

        // Each change was written as it was made.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("mode = \"light\""), "{text}");
        assert!(text.contains("accent = \"blue\""), "{text}");
        assert!(text.contains("high_contrast = true"), "{text}");
    }

    #[test]
    fn the_interface_scale_and_reduced_motion_apply_live() {
        let mut harness = past_the_splash(settings_file("scale", None));
        assert_eq!(harness.ctx.zoom_factor(), 1.0);
        assert!(harness.ctx.style_of(harness.ctx.theme()).animation_time > 0.0);
        let appearance = &mut harness.state_mut().studio.settings.settings_mut().appearance;
        appearance.ui_scale = 1.5;
        appearance.motion = crate::settings::MotionPreference::Reduced;
        settle(&mut harness);
        assert_eq!(harness.ctx.zoom_factor(), 1.5);
        assert_eq!(harness.ctx.style_of(harness.ctx.theme()).animation_time, 0.0);
    }

    #[test]
    fn primary_comma_opens_and_closes_settings() {
        let mut harness = past_the_splash(settings_file("shortcut", None));
        assert!(!harness.state().studio.is_settings_open());
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(harness.state().studio.is_settings_open());
        harness.get_by_role_and_label(Role::Tab, "General");
        // The screen stays under the window.
        harness.get_by_label("Continue offline");
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(!harness.state().studio.is_settings_open());
        assert!(harness.query_by_role_and_label(Role::Tab, "General").is_none());
    }

    #[test]
    fn the_search_box_filters_every_page() {
        let mut harness = past_the_splash(settings_file("search", None));
        let window = harness.state_mut().studio.settings_window();
        window.set_open(true);
        window.set_query("contrast");
        settle(&mut harness);
        harness.get_by_role_and_label(Role::CheckBox, "High contrast");
        harness.get_by_label("APPEARANCE");
        assert!(harness.query_by_label("Quality preset").is_none());
        assert!(harness.query_by_label("Skip sign-in").is_none());

        // By description too: none of these is named "frames".
        harness.state_mut().studio.settings_window().set_query("frames");
        settle(&mut harness);
        harness.get_by_label("RENDERING");
        harness.get_by_label("Frame cap");
        harness.get_by_label("Dynamic resolution");
        assert!(harness.query_by_label("APPEARANCE").is_none());
        assert!(harness.query_by_label("High contrast").is_none());

        harness.state_mut().studio.settings_window().set_query("teleport");
        settle(&mut harness);
        harness.get_by_label_contains("No settings match");

        // Keybinds are found too: by key, by name, by description.
        harness.state_mut().studio.settings_window().set_query("wheel");
        settle(&mut harness);
        harness.get_by_label("KEYBINDS");
        harness.get_by_label("Zoom");
        harness.get_by_label("Fly speed");
        assert!(harness.query_by_label("Look around").is_none());

        // Picking a category clears the search.
        harness.get_by_role_and_label(Role::Tab, "Keybinds").click();
        settle(&mut harness);
        assert_eq!(harness.state_mut().studio.settings_window().query(), "");
        harness.get_by_label("Fly forward");
        // #264: in sections, Fly camera inside Viewport.
        harness.get_by_role_and_label(Role::Button, "Viewport");
        harness.get_by_role_and_label(Role::Button, "Fly camera");
    }

    /// Studio past the splash, a project open in the editor, with the Settings window on the Keybinds page.
    fn on_the_keybinds_page(path: PathBuf) -> Harness<'static, Test> {
        let mut harness = editing(path, "Keybinds");
        let studio = &mut harness.state_mut().studio;
        studio.settings_window().show_category(Category::Keybinds);
        studio.settings_window().set_open(true);
        settle(&mut harness);
        harness
    }

    #[test]
    fn keybind_sections_collapse_nest_and_reset_their_own_commands() {
        let mut harness = on_the_keybinds_page(settings_file("sections", None));
        harness.set_size(egui::vec2(1280.0, 2400.0));
        settle(&mut harness);
        // Viewport holds two sections; Fly camera's commands are in it.
        harness.get_by_role_and_label(Role::Button, "Viewport");
        harness.get_by_role_and_label(Role::Button, "Fly camera");
        harness.get_by_role_and_label(Role::Button, "Mouse");
        harness.get_by_label("Fly forward");

        // Collapsing Fly camera hides its commands, not Mouse's.
        harness
            .get_by_role_and_label(Role::Button, "Fly camera")
            .click_accesskit();
        animate(&mut harness);
        assert!(harness.query_by_label("Fly forward").is_none());
        harness.get_by_label("Look around");
        // Collapsing Viewport hides the sections inside it.
        harness
            .get_by_role_and_label(Role::Button, "Viewport")
            .click_accesskit();
        animate(&mut harness);
        assert!(harness.query_by_role_and_label(Role::Button, "Mouse").is_none());
        assert!(harness.query_by_label("Look around").is_none());
        // Open again, Fly camera is still collapsed.
        harness
            .get_by_role_and_label(Role::Button, "Viewport")
            .click_accesskit();
        animate(&mut harness);
        harness.get_by_label("Look around");
        assert!(harness.query_by_label("Fly forward").is_none());
        harness
            .get_by_role_and_label(Role::Button, "Fly camera")
            .click_accesskit();
        animate(&mut harness);
        harness.get_by_label("Fly forward");

        // No Reset while nothing is changed.
        assert!(harness.query_by_label("Reset").is_none());
        let keymap = harness.state_mut().studio.keymap_mut();
        keymap.remove_binding(ids::FLY_FORWARD, 0);
        keymap.remove_binding(ids::ZOOM, 0);
        keymap.remove_binding(ids::FILE_SAVE, 0);
        settle(&mut harness);
        // A Reset on each changed command, on Fly camera, Mouse and Viewport around them, and on Project.
        assert_eq!(harness.query_all_by_label("Reset").count(), 3 + 4);
        // Fly camera's Reset: the first in Viewport's card after Viewport's own.
        let fly = harness.get_by_role_and_label(Role::Button, "Fly camera").rect();
        let reset = harness
            .query_all_by_label("Reset")
            .find(|reset| (reset.rect().center().y - fly.center().y).abs() < 1.0)
            .expect("Fly camera has a Reset");
        reset.click_accesskit();
        settle(&mut harness);
        let keymap = harness.state().studio.keymap();
        assert!(keymap.is_default(ids::FLY_FORWARD));
        assert!(!keymap.is_default(ids::ZOOM), "Mouse is another section");
        assert!(!keymap.is_default(ids::FILE_SAVE));

        // Viewport's Reset takes in the sections inside it.
        keymap_changed_in(&mut harness, ids::FLY_BACK);
        let viewport = harness.get_by_role_and_label(Role::Button, "Viewport").rect();
        harness
            .query_all_by_label("Reset")
            .find(|reset| (reset.rect().center().y - viewport.center().y).abs() < 1.0)
            .expect("Viewport has a Reset")
            .click_accesskit();
        settle(&mut harness);
        let keymap = harness.state().studio.keymap();
        assert!(keymap.is_default(ids::FLY_BACK) && keymap.is_default(ids::ZOOM));
        assert!(!keymap.is_default(ids::FILE_SAVE), "Project is another section");
    }

    /// Runs frames until a section has finished opening or closing.
    fn animate(harness: &mut Harness<'_, Test>) {
        for _ in 0..30 {
            harness.step();
        }
    }

    /// The tint of each mesh in the last frame that draws `icon` inside `area`.
    fn icon_meshes_in(harness: &Harness<'_, Test>, icon: Icon, area: egui::Rect) -> Vec<egui::Color32> {
        let textures = crate::theme::icons::builtin::textures_of(&harness.ctx, icon);
        harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Mesh(mesh)
                    if textures.contains(&mesh.texture_id) && area.contains_rect(mesh.calc_bounds()) =>
                {
                    mesh.vertices.first().map(|v| v.color)
                }
                _ => None,
            })
            .collect()
    }

    /// The tint of each mesh in the last frame that draws `icon` from the built-in icons folder (#291).
    fn icon_meshes(harness: &Harness<'_, Test>, icon: Icon) -> Vec<egui::Color32> {
        let textures = crate::theme::icons::builtin::textures_of(&harness.ctx, icon);
        harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Mesh(mesh) if textures.contains(&mesh.texture_id) => {
                    mesh.vertices.first().map(|v| v.color)
                }
                _ => None,
            })
            .collect()
    }

    /// Unbinds command `id`, so it differs from its default.
    fn keymap_changed_in(harness: &mut Harness<'_, Test>, id: &str) {
        harness.state_mut().studio.keymap_mut().remove_binding(id, 0);
        settle(harness);
    }

    /// The label of command `id`'s first binding, as this platform shows it.
    fn label_of(harness: &Harness<'_, Test>, id: &str) -> String {
        let keymap = harness.state().studio.keymap();
        keymap.label(&keymap.bindings(id)[0])
    }

    #[test]
    fn a_focused_text_field_keeps_its_keys_from_commands() {
        let mut harness = on_the_keybinds_page(settings_file("typing", None));
        // The settings' search box is then the only text field.
        for panel in [panels::ids::EXPLORER, panels::ids::OUTPUT] {
            harness.state_mut().studio.dock_mut().close(panel);
        }
        settle(&mut harness);
        assert!(harness.state().studio.editor().is_some());
        // In the search box, Primary+W is text editing, not Close project.
        harness.get_all_by_role(Role::TextInput).next().unwrap().click();
        settle(&mut harness);
        assert!(harness.ctx.egui_wants_keyboard_input());
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Editor);
        // Settings is marked "while typing": it still closes.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(!harness.state().studio.is_settings_open());
        // Nothing focused now: Primary+W closes the project.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
    }

    #[test]
    fn a_shortcut_is_captured_and_saved() {
        let path = settings_file("capture", None);
        let mut harness = on_the_keybinds_page(path.clone());
        let settings = label_of(&harness, ids::SETTINGS_OPEN);
        harness.get_by_label(&settings).click();
        settle(&mut harness);
        assert!(harness.state_mut().studio.settings_window().is_capturing());
        // In place, no dialog (#266).
        harness.get_by_label(LISTENING);
        // While capturing, the keymap does not run: Primary+, is recorded (the default again), not obeyed.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_capturing());
        assert!(harness.state().studio.is_settings_open());
        assert!(harness.state().studio.keymap().is_default(ids::SETTINGS_OPEN));

        harness.get_by_label(&settings).click();
        settle(&mut harness);
        harness.key_press(Key::F10);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_capturing());
        let studio = &harness.state().studio;
        assert_eq!(
            studio.keymap().shortcut_text(ids::SETTINGS_OPEN).as_deref(),
            Some("F10")
        );
        assert_eq!(studio.settings().keybinds.overrides[ids::SETTINGS_OPEN], ["F10"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"settings.open\" = [\"F10\"]"), "{text}");
        // Reset on the command, and on its section (#264).
        assert_eq!(harness.query_all_by_label("Reset").count(), 2);

        // The new key works, and the old one no longer does.
        harness.key_press(Key::F10);
        settle(&mut harness);
        assert!(!harness.state().studio.is_settings_open());
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(!harness.state().studio.is_settings_open());
        harness.key_press(Key::F10);
        settle(&mut harness);
        assert!(harness.state().studio.is_settings_open());

        // Esc cancels a capture and changes nothing.
        harness.get_by_label("F10").click();
        settle(&mut harness);
        harness.key_press(Key::Escape);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_capturing());
        assert_eq!(
            harness.state().studio.settings().keybinds.overrides[ids::SETTINGS_OPEN],
            ["F10"]
        );

        // A two-step sequence, in place of F10.
        harness.get_by_label("F10").click_secondary();
        settle(&mut harness);
        harness.get_by_label("Record a two-step sequence").click();
        settle(&mut harness);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::K);
        settle(&mut harness);
        assert!(
            harness.state_mut().studio.settings_window().is_capturing(),
            "waits for the second key"
        );
        harness.get_by_label_contains(", ...");
        harness.key_press(Key::S);
        settle(&mut harness);
        assert_eq!(
            harness.state().studio.settings().keybinds.overrides[ids::SETTINGS_OPEN],
            ["Primary+K, S"]
        );

        // Reset gives the default back and removes the override from the file.
        harness.query_all_by_label("Reset").last().unwrap().click();
        settle(&mut harness);
        assert!(harness.state().studio.keymap().is_default(ids::SETTINGS_OPEN));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("settings.open"), "{text}");
    }

    #[test]
    fn a_shortcut_is_recorded_in_place_once_every_key_is_released() {
        let path = settings_file("held", None);
        let mut harness = on_the_keybinds_page(path);
        let settings = label_of(&harness, ids::SETTINGS_OPEN);
        let capturing = |harness: &mut Harness<'_, Test>| harness.state_mut().studio.settings_window().is_capturing();
        harness.get_by_label(&settings).click();
        settle(&mut harness);
        // The shortcut itself shows "..." while Studio listens.
        harness.get_by_label(LISTENING);
        assert!(harness.query_by_label(&settings).is_none());

        // F10 held, then Shift as well: nothing is saved while anything is held, and what is held shows.
        harness.key_down(Key::F10);
        settle(&mut harness);
        assert!(capturing(&mut harness));
        harness.get_by_label("F10");
        harness.event(egui::Event::ModifiersChanged(Modifiers::SHIFT));
        settle(&mut harness);
        harness.get_by_label("Shift+F10");
        // F10 let go first: Shift is still held, so it keeps listening.
        harness.key_up(Key::F10);
        settle(&mut harness);
        assert!(capturing(&mut harness), "Shift is still held");
        assert!(harness.state().studio.keymap().is_default(ids::SETTINGS_OPEN));
        // Everything let go: saved, with both.
        harness.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        settle(&mut harness);
        assert!(!capturing(&mut harness));
        assert_eq!(
            harness.state().studio.settings().keybinds.overrides[ids::SETTINGS_OPEN],
            ["Shift+F10"]
        );

        // A click anywhere else cancels, and changes nothing.
        harness.get_by_label("Shift+F10").click();
        settle(&mut harness);
        assert!(capturing(&mut harness));
        harness.get_by_label("Opens or closes the Settings window.").click();
        settle(&mut harness);
        assert!(!capturing(&mut harness));
        assert_eq!(
            harness.state().studio.settings().keybinds.overrides[ids::SETTINGS_OPEN],
            ["Shift+F10"]
        );

        // A recording nobody can see stops: Settings closed while listening.
        harness.get_by_label("Shift+F10").click();
        settle(&mut harness);
        harness.state_mut().studio.settings_window().set_open(false);
        settle(&mut harness);
        assert!(!capturing(&mut harness));
    }

    #[test]
    fn the_plus_beside_a_command_adds_another_shortcut() {
        let mut harness = on_the_keybinds_page(settings_file("plus", None));
        let settings = label_of(&harness, ids::SETTINGS_OPEN);
        let plus = harness.get_by_role_and_label(Role::Button, "Add a shortcut to Settings");
        // To the right of the command's shortcut.
        assert!(plus.rect().left() > harness.get_by_label(&settings).rect().right());
        plus.click();
        settle(&mut harness);
        assert!(harness.state_mut().studio.settings_window().is_capturing());
        harness.key_press(Key::F10);
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_capturing());
        let keymap = harness.state().studio.keymap();
        let labels: Vec<String> = keymap
            .bindings(ids::SETTINGS_OPEN)
            .iter()
            .map(|binding| keymap.label(binding))
            .collect();
        assert_eq!(labels, [settings, "F10".to_owned()], "added, not replaced");
        // Every command has one, unbound ones too.
        harness.get_by_role_and_label(Role::Button, "Add a shortcut to Reset layout");
    }

    #[test]
    fn a_conflict_offers_replace_swap_and_cancel() {
        let mut harness = on_the_keybinds_page(settings_file("conflict", None));
        let settings = label_of(&harness, ids::SETTINGS_OPEN);
        let close = label_of(&harness, ids::PROJECT_CLOSE);
        // Clicks the chip labelled `chip`, presses Primary+W, and expects it to clash with `owner`.
        let capture = |harness: &mut Harness<'_, Test>, chip: &str, owner: &str| {
            harness.get_by_label(chip).click();
            settle(harness);
            harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
            settle(harness);
            harness.get_by_label(&format!("{close} is already used by {owner}."));
        };
        capture(&mut harness, &settings, "Close project (Editor)");
        harness.get_by_label("Cancel").click();
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().is_capturing());
        assert!(
            harness.state().studio.keymap().is_default(ids::SETTINGS_OPEN),
            "cancelled"
        );

        // Swap: Settings takes Primary+W, Close project gets Primary+, .
        capture(&mut harness, &settings, "Close project (Editor)");
        harness.get_by_label("Swap").click();
        settle(&mut harness);
        let keymap = harness.state().studio.keymap();
        assert_eq!(keymap.shortcut_text(ids::SETTINGS_OPEN), Some(close.clone()));
        assert_eq!(keymap.shortcut_text(ids::PROJECT_CLOSE), Some(settings.clone()));

        // Replace: Close project takes Primary+W back from Settings, which is left unbound.
        capture(&mut harness, &settings, "Settings (Global)");
        harness.get_by_label("Replace").click();
        settle(&mut harness);
        let keymap = harness.state().studio.keymap();
        assert_eq!(keymap.shortcut_text(ids::PROJECT_CLOSE), Some(close));
        assert!(keymap.bindings(ids::SETTINGS_OPEN).is_empty());
        // Settings, and Reset layout, which has no default.
        assert_eq!(harness.query_all_by_label("Unbound").count(), 2);
        assert_eq!(
            harness.state().studio.settings().keybinds.overrides[ids::SETTINGS_OPEN],
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_new_default_that_would_take_a_users_key_is_left_out_and_told_once() {
        let path = settings_file("stolen", Some("[keybinds]\n\"settings.open\" = [\"Primary+W\"]\n"));
        let harness = past_the_splash(path.clone());
        harness.get_by_label("A new shortcut was left out");
        harness.get_by_label_contains("Close project");
        assert!(harness.state().studio.keymap().bindings(ids::PROJECT_CLOSE).is_empty());
        // Saved, so the next start is quiet.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"project.close\" = []"), "{text}");
        let again = past_the_splash(path);
        assert!(again.state().studio.notice().is_none());
        assert!(again.state().studio.keymap().bindings(ids::PROJECT_CLOSE).is_empty());
    }

    #[test]
    fn a_corrupt_settings_file_is_kept_and_reported_once() {
        let path = settings_file("corrupt", Some("[appearance\nmode = "));
        let mut harness = past_the_splash(path.clone());
        assert!(path.with_extension("toml.bak").is_file());
        assert_eq!(harness.state().studio.settings(), &StudioSettings::default());
        harness.get_by_label("Settings were reset");
        harness.get_by_label_contains("settings.toml.bak");

        harness.get_by_label("Dismiss").click();
        settle(&mut harness);
        assert!(harness.state().studio.notice().is_none());
        assert!(harness.query_by_label("Settings were reset").is_none());
    }

    // The sign-in screen and the projects page (#215).

    /// A folder of its own for `test`, holding `settings.toml` (`settings`) and `projects.toml` (`projects`) when
    /// given. Returns the settings file's path; the projects list is beside it.
    fn config(test: &str, settings: Option<&str>, projects: Option<&str>) -> PathBuf {
        let path = settings_file(test, settings);
        if let Some(projects) = projects {
            std::fs::write(path.with_file_name(PROJECTS_FILE), projects).unwrap();
        }
        path
    }

    /// A real project called `name` in `dir`, made from the repository's Baseplate template.
    fn make_project(dir: &Path, name: &str) -> PathBuf {
        let template =
            metallic_engine::metallic_project::Template::from_dir(templates_dir().unwrap().join("baseplate-3d"))
                .unwrap();
        let path = dir.join(format!("{name}.mtproj"));
        metallic_engine::metallic_project::Project::create(&path, &template, name).unwrap();
        path
    }

    /// `projects.toml` text listing `projects` (path, name), most recent first.
    fn list_of(projects: &[(&Path, &str)]) -> String {
        projects
            .iter()
            .enumerate()
            .map(|(n, (path, name))| {
                format!(
                    "[[project]]\npath = '{}'\nname = \"{name}\"\nlast_opened = {}\n",
                    path.display(),
                    1_700_000_000 - n * 3600
                )
            })
            .collect()
    }

    /// Clicks the node labelled `label` twice, a frame apart: a double-click.
    fn double_click(harness: &mut Harness<'_, Test>, label: &str) {
        harness.get_by_label(label).click();
        harness.step();
        // The harness's clock jumps by whole repaint delays between frames; a double-click is two clicks 50 ms
        // apart.
        let time = harness.ctx.input(|input| input.time);
        harness.input_mut().time = Some(time + 0.05);
        harness.get_by_label(label).click();
        settle(harness);
        harness.input_mut().time = None;
    }

    /// Studio past the splash on the projects page, asking `dialog` for files.
    fn on_the_projects_page(path: PathBuf, dialog: &ScriptedDialog) -> Harness<'static, Test> {
        let mut harness = past_the_splash(path);
        harness.state_mut().studio.set_file_dialog(Box::new(dialog.clone()));
        if harness.state().studio.screen() == Screen::SignIn {
            harness.get_by_label("Continue offline").click();
            settle(&mut harness);
        }
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness
    }

    #[test]
    fn continuing_offline_goes_to_the_projects_and_saves_nothing() {
        let path = config("offline", None, None);
        let mut harness = past_the_splash(path.clone());
        assert_eq!(harness.state().studio.screen(), Screen::SignIn);
        harness.get_by_label("Sign in to Metallic");
        let sign_in = harness.get_by_role_and_label(Role::Button, "Sign in");
        assert!(
            egui_kittest::kittest::NodeT::accesskit_node(&sign_in).is_disabled(),
            "there are no servers yet"
        );
        harness.get_by_label("Online services aren't available yet.");

        harness.get_by_label("Continue offline").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        assert!(!harness.state().studio.settings().general.skip_sign_in);
        assert!(!path.exists(), "nothing changed, nothing written");
        // Opened again, it asks again.
        assert_eq!(past_the_splash(path).state().studio.screen(), Screen::SignIn);
    }

    #[test]
    fn dont_ask_again_is_saved_and_skips_sign_in_from_then_on() {
        let path = config("dont-ask", None, None);
        let mut harness = past_the_splash(path.clone());
        harness.get_by_role_and_label(Role::CheckBox, "Don't ask again").click();
        settle(&mut harness);
        assert!(harness.state().studio.settings().general.skip_sign_in);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("skip_sign_in = true"), "{text}");
        harness.get_by_label("Continue offline").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);

        assert_eq!(past_the_splash(path).state().studio.screen(), Screen::Projects);
    }

    #[test]
    fn a_new_project_is_made_from_a_template_listed_and_opened() {
        let path = config("new", Some("[general]\nskip_sign_in = true\n"), None);
        let folder = path.with_file_name("Made here");
        let dialog = ScriptedDialog::new([Some(folder.clone())]);
        let mut harness = on_the_projects_page(path.clone(), &dialog);

        // Primary+N opens the dialog, Cancel closes it.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
        settle(&mut harness);
        assert!(harness.state().studio.projects().is_creating());
        harness.get_by_label("Create a project");
        harness.get_by_label("Cancel").click();
        settle(&mut harness);
        assert!(!harness.state().studio.projects().is_creating());

        harness.get_by_label("New project").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::RadioButton, "Baseplate").click();
        let name = harness.get_all_by_role(Role::TextInput).next().unwrap();
        name.set_text_value_accesskit("Space Race");
        settle(&mut harness);
        harness.get_by_label("Browse\u{2026}").click();
        settle(&mut harness);
        assert_eq!(dialog.asked(), ["folder"]);
        let made = folder.join("Space Race.mtproj");
        harness.get_by_label(&format!("Makes {}", made.display()));

        harness.get_by_label("Create").click();
        settle(&mut harness);
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Editor);
        let opened = studio.open_project().unwrap().to_path_buf();
        assert_eq!(opened, std::path::absolute(&made).unwrap());
        let project = metallic_engine::metallic_project::Project::open(&opened).unwrap();
        assert_eq!(project.manifest().name, "Space Race");
        assert_eq!(studio.projects().list().entries()[0].path, opened);
        wait_open(&mut harness);
        assert!(harness.state().studio.window_title().starts_with("Space Race"));
        let list = std::fs::read_to_string(path.with_file_name(PROJECTS_FILE)).unwrap();
        assert!(list.contains("name = \"Space Race\""), "{list}");

        // Making it again refuses to replace the file, and says so in the dialog.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        settle(&mut harness);
        harness.get_by_label("New project").click();
        settle(&mut harness);
        harness
            .state_mut()
            .studio
            .projects_mut()
            .set_new_project("Space Race", &folder);
        harness.get_by_label("Create").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_label_contains("a file is already there");
    }

    #[test]
    fn adding_a_project_lists_and_opens_it_and_a_non_project_is_refused() {
        let path = config("add", Some("[general]\nskip_sign_in = true\n"), None);
        let dir = path.parent().unwrap().to_path_buf();
        let project = make_project(&dir, "Added Game");
        let not_a_project = dir.join("notes.mtproj");
        std::fs::write(&not_a_project, "just text").unwrap();
        let dialog = ScriptedDialog::new([None, Some(not_a_project), Some(project.clone())]);
        let mut harness = on_the_projects_page(path.clone(), &dialog);

        // Cancelled: nothing happens.
        harness.get_by_label("Add project").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        assert!(harness.state().studio.projects().list().entries().is_empty());

        // Not a project: refused, and said why.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::O);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_label_contains("Could not add that project");
        assert!(harness.state().studio.projects().list().entries().is_empty());

        harness.get_by_label("Add project").click();
        settle(&mut harness);
        assert_eq!(dialog.asked(), ["project", "project", "project"]);
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Editor);
        assert_eq!(studio.open_project(), Some(project.as_path()));
        let entry = &studio.projects().list().entries()[0];
        assert_eq!(entry.name, "Added Game");
        assert!(entry.last_opened.is_some());
        let list = std::fs::read_to_string(path.with_file_name(PROJECTS_FILE)).unwrap();
        assert!(list.contains("Added Game"), "{list}");
    }

    #[test]
    fn a_missing_project_is_marked_and_does_not_open() {
        let dir = std::env::temp_dir().join(format!("metallic-studio-app-missing-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let here = make_project_once(&dir, "Still Here");
        let gone = dir.join("Gone Game.mtproj");
        let path = config(
            "missing",
            Some("[general]\nskip_sign_in = true\n"),
            Some(&list_of(&[(&gone, "Gone Game"), (&here, "Still Here")])),
        );
        let mut harness = on_the_projects_page(path, &ScriptedDialog::default());
        harness.get_by_label("Gone Game");
        harness.get_by_label("Still Here");
        assert_eq!(harness.query_all_by_label("File missing").count(), 1);

        double_click(&mut harness, "Gone Game");
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_label_contains("is not at");
        harness.get_by_label("Dismiss").click();
        settle(&mut harness);
        assert!(harness.state().studio.projects().error().is_none());
    }

    /// A real project called `name` in `dir`, made the first time it is asked for.
    fn make_project_once(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}.mtproj"));
        if path.is_file() { path } else { make_project(dir, name) }
    }

    #[test]
    fn removing_a_project_takes_it_off_the_list_and_keeps_its_file() {
        let dir = std::env::temp_dir().join(format!("metallic-studio-app-remove-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let keep = make_project_once(&dir, "Keep Me");
        let other = make_project_once(&dir, "Other");
        let path = config(
            "remove",
            Some("[general]\nskip_sign_in = true\n"),
            Some(&list_of(&[(&keep, "Keep Me"), (&other, "Other")])),
        );
        let mut harness = on_the_projects_page(path.clone(), &ScriptedDialog::default());
        harness.get_by_label("Keep Me").click_secondary();
        settle(&mut harness);
        harness.get_by_label("Remove from list").click();
        settle(&mut harness);

        assert!(harness.query_by_label("Keep Me").is_none());
        harness.get_by_label("Other");
        assert!(keep.is_file(), "the file is left alone");
        let list = std::fs::read_to_string(path.with_file_name(PROJECTS_FILE)).unwrap();
        assert!(!list.contains("Keep Me") && list.contains("Other"), "{list}");
    }

    #[test]
    fn opening_a_project_moves_to_the_editor_with_its_path() {
        let dir = std::env::temp_dir().join(format!("metallic-studio-app-open-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = make_project_once(&dir, "First");
        let second = make_project_once(&dir, "Second");
        let path = config(
            "open",
            Some("[general]\nskip_sign_in = true\n"),
            Some(&list_of(&[(&first, "First"), (&second, "Second")])),
        );
        let mut harness = on_the_projects_page(path.clone(), &ScriptedDialog::default());

        // A click selects.
        harness.get_by_label("Second").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.projects().selected(), Some(second.as_path()));

        // A double-click opens.
        double_click(&mut harness, "Second");
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Editor);
        assert_eq!(studio.open_project(), Some(second.as_path()));
        wait_open(&mut harness);
        assert_eq!(harness.state().studio.editor().unwrap().path(), second);
        // It moved to the front of the list, with the time.
        let entries = harness.state().studio.projects().list().entries();
        assert_eq!(entries[0].path, second);
        assert!(entries[0].last_opened.unwrap() > 1_700_000_000);

        // Enter opens the selected card.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_label("First").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.projects().selected(), Some(first.as_path()));
        harness.key_press(Key::Enter);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Editor);
        assert_eq!(harness.state().studio.open_project(), Some(first.as_path()));
    }

    #[test]
    fn the_templates_view_starts_a_new_project() {
        let path = config("templates", Some("[general]\nskip_sign_in = true\n"), None);
        let mut harness = on_the_projects_page(path, &ScriptedDialog::default());
        harness.get_by_role_and_label(Role::Tab, "Templates").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.projects().view(), View::Templates);
        harness.get_by_label("3D template").click();
        settle(&mut harness);
        assert!(harness.state().studio.projects().is_creating());
        harness.get_by_role_and_label(Role::RadioButton, "Baseplate");
    }

    #[test]
    fn a_corrupt_projects_list_is_kept_as_bak_and_the_list_starts_empty() {
        let path = config(
            "corrupt-list",
            Some("[general]\nskip_sign_in = true\n"),
            Some("[[project]\npath = "),
        );
        let list = path.with_file_name(PROJECTS_FILE);
        let harness = past_the_splash(path);
        assert!(list.with_extension("toml.bak").is_file());
        assert!(!list.exists());
        assert!(harness.state().studio.projects().list().entries().is_empty());
        harness.get_by_label("The projects list was reset");
        harness.get_by_label_contains("projects.toml.bak");
        harness.get_by_label("No projects yet");
    }

    // The editor (#216).

    /// Runs frames until the project opening behind the overlay is open, or has failed; then settles.
    fn wait_open(harness: &mut Harness<'_, Test>) {
        for _ in 0..10_000 {
            harness.step();
            if !harness.state().studio.is_opening() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!harness.state().studio.is_opening(), "the project never opened");
        settle(harness);
    }

    /// Studio past the splash on the settings at `path`, with a project called `name` (made beside them) open.
    fn editing(path: PathBuf, name: &str) -> Harness<'static, Test> {
        let mut harness = past_the_splash(path.clone());
        let project = make_project_once(path.parent().unwrap(), name);
        harness.state_mut().studio.open(project);
        wait_open(&mut harness);
        let studio = &harness.state().studio;
        assert!(studio.editor().is_some(), "{:?}", studio.projects().error());
        harness
    }

    /// Studio given `project` on the command line, run past its splash on the settings at `path`.
    fn launched_with(path: PathBuf, project: PathBuf) -> Harness<'static, Test> {
        let mut harness = harness(steps_reading(path));
        harness.state_mut().studio.open_after_splash(project);
        finish_steps(&mut harness, splash::MIN_TIME);
        harness.set_size(egui::vec2(1280.0, 800.0));
        harness
    }

    #[test]
    fn a_project_on_the_command_line_opens_straight_into_the_editor_and_joins_recent() {
        // Sign-in is not skipped in the settings: the command line skips it, and the projects page, anyway.
        let path = config("launch", None, None);
        let dir = path.parent().unwrap().join("with spaces");
        std::fs::create_dir_all(&dir).unwrap();
        let project = make_project_once(&dir, "Launched Game");
        let mut harness = launched_with(path.clone(), project.clone());
        let studio = &mut harness.state_mut().studio;
        assert_eq!(studio.screen(), Screen::Editor);
        assert!(studio.is_opening());
        assert_eq!(studio.take_requests(), [WindowRequest::BecomeMainWindow]);

        wait_open(&mut harness);
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Editor);
        assert_eq!(studio.editor().unwrap().path(), project);
        let recent = &studio.projects().list().entries()[0];
        assert_eq!(
            (recent.path.as_path(), recent.name.as_str()),
            (project.as_path(), "Launched Game")
        );
        assert!(recent.last_opened.is_some());
        assert!(harness.state().studio.window_title().starts_with("Launched Game"));
        assert!(harness.state().studio.window_title().ends_with(" - Metallic Studio"));
        assert!(
            harness.query_by_label("Launched Game").is_none(),
            "not in the top bar (#254)"
        );
        let list = std::fs::read_to_string(path.with_file_name(PROJECTS_FILE)).unwrap();
        assert!(list.contains("name = \"Launched Game\""), "{list}");

        // Closing it goes to the projects page, where it is listed.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
        harness.get_by_label("Launched Game");
    }

    #[test]
    fn a_bad_path_on_the_command_line_lands_on_the_projects_page_with_the_error() {
        let path = config("launch-missing", None, None);
        let missing = path.parent().unwrap().join("Not Here.mtproj");
        let mut harness = launched_with(path.clone(), missing.clone());
        settle(&mut harness);
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Projects);
        assert!(studio.editor().is_none());
        let error = studio.projects().error().unwrap().to_owned();
        assert!(error.contains(&missing.display().to_string()), "{error}");
        assert!(studio.projects().list().entries().is_empty());
        harness.get_by_label(&error);

        // A file that is not a project fails while it opens, and lands there the same way, not in Recent.
        let path = config("launch-junk", None, None);
        let junk = path.parent().unwrap().join("Junk.mtproj");
        std::fs::write(&junk, "not a zip archive").unwrap();
        let mut harness = launched_with(path.clone(), junk);
        assert_eq!(harness.state().studio.screen(), Screen::Editor);
        wait_open(&mut harness);
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Projects);
        let error = studio.projects().error().unwrap().to_owned();
        assert!(error.starts_with("Could not open \u{201C}Junk\u{201D}."), "{error}");
        assert!(studio.projects().list().entries().is_empty());
        assert!(!path.with_file_name(PROJECTS_FILE).exists(), "nothing to save");
        harness.get_by_label(&error);
    }

    /// Clicks the node labelled `label` and runs the frames that show what it did.
    fn click_label(harness: &mut Harness<'_, Test>, label: &str) {
        // Not a toolbar tab of the same name (#232: the Plugins tab and the Plugins menu).
        harness
            .get_all_by_label(label)
            .find(|node| egui_kittest::kittest::NodeT::accesskit_node(node).role() != Role::Tab)
            .expect("a node with that label")
            .click();
        settle(harness);
    }

    /// The default layout's panels, in registry order.
    const DEFAULT_PANELS: [&str; 5] = [
        panels::ids::VIEWPORT,
        panels::ids::EXPLORER,
        panels::ids::PROPERTIES,
        panels::ids::OUTPUT,
        panels::ids::ASSETS,
    ];

    #[test]
    fn the_template_opens_with_its_tree_and_the_selected_parts_properties() {
        let mut harness = editing(settings_file("editor-tree", None), "Tree Game");
        // Properties shares the right column with the Explorer (#256): tall enough for the rows checked below.
        harness.set_size(egui::vec2(1280.0, 1400.0));
        settle(&mut harness);
        assert_eq!(harness.state().studio.dock().open_panels(), DEFAULT_PANELS);
        for tab in ["Viewport", "Explorer", "Assets", "Properties", "Output"] {
            harness.get_by_role_and_label(Role::Tab, tab);
        }
        assert!(harness.state().studio.window_title().starts_with("Tree Game"));
        harness.get_by_role_and_label(Role::TreeItem, "Workspace");
        harness.get_by_role_and_label(Role::TreeItem, "Lighting");
        harness.get_by_role_and_label(Role::TreeItem, "SpawnLocation");
        assert!(harness.query_by_role_and_label(Role::TreeItem, "RunService").is_none());
        harness.get_by_label("Select something in the Explorer to see its properties.");

        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        let studio = &harness.state().studio;
        let editor = studio.editor().unwrap();
        assert_eq!(editor.dm.name(editor.selection.active().unwrap()).unwrap(), "Baseplate");
        // A click in a panel gives it the focus, and the keymap its context.
        assert_eq!(studio.dock().focus().focused, Some(panels::ids::EXPLORER));
        assert_eq!(studio.dock().context(), Context::Explorer);
        harness.get_by_label("TRANSFORM");
        harness.get_by_role_and_label(Role::Label, "Size");
        // Edited in place (#349): a vector's text (#380), the enum's drop-down, the colour's swatch.
        harness.get_by_role_and_label(Role::TextInput, "Size");
        harness.get_by_role_and_label(Role::ComboBox, "Material");
        harness.get_by_role_and_label(Role::ColorWell, "Color");

        // The Assets tab beside Output (#256): the template's one asset is its scene, and Output goes behind it.
        harness.get_by_role_and_label(Role::Button, "Clear");
        harness.get_by_role_and_label(Role::Tab, "Assets").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::ListItem, "Scenes/Main.scene");
        harness.get_by_label("1 asset");
        assert!(harness.query_by_role_and_label(Role::Button, "Clear").is_none());
        harness.get_by_role_and_label(Role::TreeItem, "Workspace");

        // Collapsing the workspace hides what is in it.
        click_label(&mut harness, "Collapse Workspace");
        assert!(harness.query_by_role_and_label(Role::TreeItem, "Baseplate").is_none());
        click_label(&mut harness, "Expand Workspace");
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate");
    }

    #[test]
    fn the_line_between_names_and_values_drags_and_stays() {
        let mut harness = editing(settings_file("editor-split", None), "Split Game");
        harness.set_size(egui::vec2(1280.0, 1400.0));
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        let grip = harness.get_by_label("Name column width").rect();
        let before = crate::panels::properties::name_share(&harness.ctx);
        // Dragged right: the line follows the pointer.
        drag(&mut harness, grip.center(), grip.center() + egui::vec2(40.0, 0.0));
        let moved = harness.get_by_label("Name column width").rect();
        assert!(
            (moved.center().x - grip.center().x - 40.0).abs() < 1.0,
            "{moved:?} from {grip:?}"
        );
        assert!(crate::panels::properties::name_share(&harness.ctx) > before);
        // Dragged past the panel's side: it stops short of it, the names still there.
        drag(&mut harness, moved.center(), egui::pos2(0.0, moved.center().y));
        let stopped = harness.get_by_label("Name column width").rect();
        let size = harness.get_by_role_and_label(Role::Label, "Size").rect();
        assert!(
            stopped.center().x > size.left() + 40.0,
            "{stopped:?}, the name at {size:?}"
        );
        // It stays where it was left, for another part too.
        harness.get_by_role_and_label(Role::TreeItem, "SpawnLocation").click();
        settle(&mut harness);
        let again = harness.get_by_label("Name column width").rect();
        assert!((again.center().x - stopped.center().x).abs() < 1.0);
    }

    /// The names of what is selected in the editor, the active one last.
    fn selected_names(harness: &Harness<'_, Test>) -> Vec<String> {
        let editor = harness.state().studio.editor().unwrap();
        editor
            .selection
            .ids()
            .iter()
            .map(|&id| editor.dm.name(id).unwrap().to_owned())
            .collect()
    }

    #[test]
    fn explorer_clicks_select_several_and_select_all_takes_every_row() {
        let mut harness = editing(settings_file("editor-multi", None), "Multi Game");
        harness.set_size(egui::vec2(1280.0, 900.0));
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        harness
            .get_by_role_and_label(Role::TreeItem, "SpawnLocation")
            .click_modifiers(Modifiers::COMMAND);
        settle(&mut harness);
        assert_eq!(selected_names(&harness), ["Baseplate", "SpawnLocation"]);
        // Both rows show selected; Properties follows the active one.
        for row in ["Baseplate", "SpawnLocation"] {
            let node = harness.get_by_role_and_label(Role::TreeItem, row);
            let toggled = egui_kittest::kittest::NodeT::accesskit_node(&node).toggled();
            assert_eq!(toggled, Some(egui::accesskit::Toggled::True), "{row}");
        }
        harness.get_by_role_and_label(Role::Label, "Size");
        // Shift: the range from the anchor (the Ctrl-clicked row) up to Workspace.
        harness
            .get_by_role_and_label(Role::TreeItem, "Workspace")
            .click_modifiers(Modifiers::SHIFT);
        settle(&mut harness);
        let names = selected_names(&harness);
        assert_eq!(names.first().map(String::as_str), Some("SpawnLocation"));
        assert_eq!(names.last().map(String::as_str), Some("Workspace"));
        assert!(!names.contains(&"Lighting".to_owned()), "{names:?}");
        // A plain click selects only that row.
        harness.get_by_role_and_label(Role::TreeItem, "Lighting").click();
        settle(&mut harness);
        assert_eq!(selected_names(&harness), ["Lighting"]);
        // Primary+A: every row shown, Lighting still the active one.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        settle(&mut harness);
        let all = selected_names(&harness);
        assert!(all.len() > 5 && all.contains(&"Baseplate".to_owned()), "{all:?}");
        assert_eq!(all.last().map(String::as_str), Some("Lighting"));
        // Closing Workspace marks it: it holds something selected.
        click_label(&mut harness, "Collapse Workspace");
        harness.get_by_role_and_label(Role::TreeItem, "Workspace");
    }

    /// The editor's Baseplate.
    fn editor_baseplate(harness: &Harness<'_, Test>) -> metallic_engine::metallic_core::Id {
        let dm = &harness.state().studio.editor().unwrap().dm;
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        dm.find_first_child(workspace, "Baseplate", false).unwrap().unwrap()
    }

    #[test]
    fn edits_undo_and_redo_from_the_keys_and_the_edit_menu_and_mark_the_title() {
        let mut harness = editing(settings_file("editor-undo", None), "Undo Game");
        harness.set_size(egui::vec2(1280.0, 900.0));
        settle(&mut harness);
        let baseplate = editor_baseplate(&harness);
        let transparency = |harness: &Harness<'_, Test>| {
            harness
                .state()
                .studio
                .editor()
                .unwrap()
                .dm
                .get_by_name(baseplate, "Transparency")
        };
        // A new project counts as unsaved until its first save (#228): save, so the mark is the history's.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
        settle(&mut harness);
        assert!(!harness.state().studio.window_title().contains('*'));
        click_label(&mut harness, "Edit");
        assert!(!menu_item_enabled(&harness, "Undo") && !menu_item_enabled(&harness, "Redo"));
        harness.key_press(Key::Escape);
        // An edit through the scene is a step at the end of the frame, and marks the title.
        harness
            .state_mut()
            .studio
            .editor_mut()
            .unwrap()
            .scene()
            .set_by_name(baseplate, "Transparency", Value::Number(0.5))
            .unwrap();
        settle(&mut harness);
        assert!(harness.state().studio.window_title().starts_with("Undo Game*"));
        click_label(&mut harness, "Edit");
        assert!(menu_item_enabled(&harness, "Undo") && !menu_item_enabled(&harness, "Redo"));
        harness.key_press(Key::Escape);
        settle(&mut harness);

        // Primary+Z takes it back, and the title is clean again; Primary+Y and Primary+Shift+Z redo it.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(&mut harness);
        assert_eq!(transparency(&harness), Ok(Value::Number(0.0)));
        assert!(!harness.state().studio.window_title().contains('*'));
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        settle(&mut harness);
        assert_eq!(transparency(&harness), Ok(Value::Number(0.5)));
        click_label(&mut harness, "Edit");
        harness.get_by_role_and_label(Role::MenuItem, "Undo").click();
        settle(&mut harness);
        assert_eq!(transparency(&harness), Ok(Value::Number(0.0)));
        harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        settle(&mut harness);
        assert_eq!(transparency(&harness), Ok(Value::Number(0.5)));

        // Saving clears the mark; undoing past the save marks it again.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
        settle(&mut harness);
        assert!(!harness.state().studio.window_title().contains('*'));
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(&mut harness);
        assert!(harness.state().studio.window_title().contains('*'));

        // A click that selects is a step: undo puts back what was selected before.
        harness.get_by_role_and_label(Role::TreeItem, "Lighting").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "SpawnLocation").click();
        settle(&mut harness);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(&mut harness);
        assert_eq!(selected_names(&harness), ["Lighting"]);
    }

    /// Types `text` into the field labelled `label` (a text field, or a number field, which a click opens for typing)
    /// in place of what it holds, and presses Enter.
    fn type_into(harness: &mut Harness<'_, Test>, label: &str, text: &str) {
        let field = |harness: &Harness<'_, Test>| {
            let found = harness.get_all_by_label(label).find(|node| {
                let role = egui_kittest::kittest::NodeT::accesskit_node(node).role();
                matches!(role, Role::TextInput | Role::SpinButton)
            });
            found.map(|node| node.rect()).expect("a field with that label")
        };
        let rect = field(harness);
        left_click(harness, rect);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness.key_press(Key::Backspace);
        harness.event(egui::Event::Text(text.to_owned()));
        harness.key_press(Key::Enter);
        settle(harness);
    }

    #[test]
    fn properties_edit_each_kind_of_value_and_undo_puts_it_back() {
        let mut harness = editing(settings_file("editor-edit-props", None), "Edit Game");
        harness.set_size(egui::vec2(1280.0, 1500.0));
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        let baseplate = editor_baseplate(&harness);
        let value = |harness: &Harness<'_, Test>, name: &str| {
            harness
                .state()
                .studio
                .editor()
                .unwrap()
                .dm
                .get_by_name(baseplate, name)
                .unwrap()
        };
        let undo = |harness: &mut Harness<'_, Test>| {
            harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            settle(harness);
        };

        // A check box.
        assert_eq!(value(&harness, "Anchored"), Value::Bool(true));
        harness.get_by_role_and_label(Role::CheckBox, "Anchored").click();
        settle(&mut harness);
        assert_eq!(value(&harness, "Anchored"), Value::Bool(false));
        undo(&mut harness);
        assert_eq!(value(&harness, "Anchored"), Value::Bool(true));

        // A string, written when the field is left: the Explorer follows.
        type_into(&mut harness, "Name", "Floor");
        assert_eq!(value(&harness, "Name"), Value::String("Floor".into()));
        harness.get_by_role_and_label(Role::TreeItem, "Floor");
        undo(&mut harness);
        assert_eq!(value(&harness, "Name"), Value::String("Baseplate".into()));

        // A vector, typed as text (#380).
        type_into(&mut harness, "Size", "100, 16, 512");
        let Value::Vector3(size) = value(&harness, "Size") else {
            panic!("a vector");
        };
        assert_eq!((size.x, size.y, size.z), (100.0, 16.0, 512.0));
        undo(&mut harness);
        assert!(matches!(value(&harness, "Size"), Value::Vector3(size) if size.x == 512.0));

        // A CFrame's position row moves the part; its orientation row turns it.
        let placed = value(&harness, "CFrame");
        // A blank leaves that component.
        type_into(&mut harness, "CFrame Position", ", 20, ");
        let Value::CFrame(frame) = value(&harness, "CFrame") else {
            panic!("a CFrame");
        };
        assert!((frame.position().y - 20.0).abs() < 1e-4);
        type_into(&mut harness, "CFrame Orientation", "0, 90, 0");
        let Value::Vector3(turned) = value(&harness, "Orientation") else {
            panic!("a vector");
        };
        assert!((turned.y - 90.0).abs() < 1e-3, "{turned:?}");
        // Two edits of one property this close together are one step (#348).
        undo(&mut harness);
        assert_eq!(value(&harness, "CFrame"), placed);

        // A colour's channel.
        type_into(&mut harness, "Color", "255, -, -");
        let Value::Color3(color) = value(&harness, "Color") else {
            panic!("a colour");
        };
        assert_eq!(color.r, 1.0);
        undo(&mut harness);
        // One number fills every component (#382).
        type_into(&mut harness, "Color", "255");
        assert_eq!(
            value(&harness, "Color"),
            Value::Color3(metallic_engine::metallic_core::Color3::new(1.0, 1.0, 1.0))
        );
        harness.get_by_role_and_label(Role::TextInput, "Color");
        assert!(
            harness.query_all_by_value("255, 255, 255").next().is_some(),
            "the field shows them all"
        );
        undo(&mut harness);

        // An enum, from its drop-down.
        harness.get_by_role_and_label(Role::ComboBox, "Material").click();
        settle(&mut harness);
        harness.get_by_label("Wood").click();
        settle(&mut harness);
        assert!(matches!(value(&harness, "Material"), Value::Enum(item) if item.name() == "Wood"));
        undo(&mut harness);
        assert!(matches!(value(&harness, "Material"), Value::Enum(item) if item.name() == "Concrete"));

        // A number past its range is kept in it; a reference is not a field.
        type_into(&mut harness, "Transparency", "4");
        assert_eq!(value(&harness, "Transparency"), Value::Number(1.0));
        assert!(harness.query_by_role_and_label(Role::TextInput, "Parent").is_none());
        assert_eq!(
            harness.query_all_by_label("Workspace").count(),
            2,
            "the Explorer's row and the Parent's name"
        );
    }

    #[test]
    fn properties_of_several_instances_show_mixed_values_and_edit_them_all_in_one_step() {
        let mut harness = editing(settings_file("editor-multi-props", None), "Multi Props Game");
        harness.set_size(egui::vec2(1280.0, 1500.0));
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        harness
            .get_by_role_and_label(Role::TreeItem, "SpawnLocation")
            .click_modifiers(Modifiers::COMMAND);
        settle(&mut harness);
        harness.get_by_label("2 Parts");
        let sizes = |harness: &Harness<'_, Test>| {
            let editor = harness.state().studio.editor().unwrap();
            editor
                .selection
                .ids()
                .iter()
                .map(|&id| match editor.dm.get_by_name(id, "Size").unwrap() {
                    Value::Vector3(v) => (v.x, v.y, v.z),
                    other => panic!("{other:?}"),
                })
                .collect::<Vec<_>>()
        };
        let before = sizes(&harness);
        assert_ne!(before[0], before[1], "the template's two parts differ in size");
        // One component typed reaches both, each keeping its own others; one undo step takes both back.
        type_into(&mut harness, "Size", "-, 5, -");
        let after = sizes(&harness);
        assert_eq!(after[0], (before[0].0, 5.0, before[0].2));
        assert_eq!(after[1], (before[1].0, 5.0, before[1].2));
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(&mut harness);
        assert_eq!(sizes(&harness), before);
        assert_eq!(
            selected_names(&harness),
            ["Baseplate", "SpawnLocation"],
            "the selection stays"
        );
        // A name they disagree on is mixed; typing one names both.
        harness.get_by_role_and_label(Role::TextInput, "Name");
        type_into(&mut harness, "Name", "Pad");
        assert_eq!(selected_names(&harness), ["Pad", "Pad"]);
    }

    /// The names under the editor's `parent` (a service's name), in order.
    fn children_of(harness: &Harness<'_, Test>, parent: &str) -> Vec<String> {
        let dm = &harness.state().studio.editor().unwrap().dm;
        let service = dm.service(dm.registry().class_id(parent).unwrap()).unwrap();
        dm.children(service)
            .unwrap()
            .map(|id| dm.name(id).unwrap().to_owned())
            .collect()
    }

    #[test]
    fn tree_edits_duplicate_delete_cut_copy_paste_and_rename_and_each_undoes() {
        let mut harness = editing(settings_file("editor-tree-edits", None), "Tree Edits Game");
        harness.set_size(egui::vec2(1280.0, 1000.0));
        settle(&mut harness);
        let original = children_of(&harness, "Workspace");
        assert_eq!(original, ["Camera", "Baseplate", "SpawnLocation"]);
        let press = |harness: &mut Harness<'_, Test>, modifiers: Modifiers, key: Key| {
            harness.key_press_modifiers(modifiers, key);
            settle(harness);
        };
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);

        // Duplicate: a copy just after it, selected.
        press(&mut harness, Modifiers::COMMAND, Key::D);
        assert_eq!(
            children_of(&harness, "Workspace"),
            ["Camera", "Baseplate", "Baseplate", "SpawnLocation"]
        );
        let copy = harness.state().studio.editor().unwrap().selection.active().unwrap();
        assert_ne!(Some(copy), Some(editor_baseplate(&harness)));
        press(&mut harness, Modifiers::COMMAND, Key::Z);
        assert_eq!(children_of(&harness, "Workspace"), original);

        // Delete: out of the tree, nothing selected; undo brings the same one back in its place, selected.
        let baseplate = editor_baseplate(&harness);
        press(&mut harness, Modifiers::NONE, Key::Delete);
        assert_eq!(children_of(&harness, "Workspace"), ["Camera", "SpawnLocation"]);
        assert!(selected_names(&harness).is_empty());
        press(&mut harness, Modifiers::COMMAND, Key::Z);
        assert_eq!(children_of(&harness, "Workspace"), original);
        assert_eq!(editor_baseplate(&harness), baseplate);
        assert_eq!(selected_names(&harness), ["Baseplate"]);

        // Copy, then paste with a service selected: inside it. Paste into: inside the selected part.
        press(&mut harness, Modifiers::COMMAND, Key::C);
        harness
            .get_by_role_and_label(Role::TreeItem, "ReplicatedStorage")
            .click();
        settle(&mut harness);
        press(&mut harness, Modifiers::COMMAND, Key::V);
        assert_eq!(children_of(&harness, "ReplicatedStorage"), ["Baseplate"]);
        assert_eq!(selected_names(&harness), ["Baseplate"], "the pasted copy is selected");
        harness.get_by_role_and_label(Role::TreeItem, "SpawnLocation").click();
        settle(&mut harness);
        press(&mut harness, Modifiers::COMMAND | Modifiers::SHIFT, Key::V);
        {
            let editor = harness.state().studio.editor().unwrap();
            let pasted = editor.selection.active().unwrap();
            assert_eq!(
                editor.dm.name(editor.dm.parent(pasted).unwrap().unwrap()).unwrap(),
                "SpawnLocation"
            );
        }
        press(&mut harness, Modifiers::COMMAND, Key::Z);
        press(&mut harness, Modifiers::COMMAND, Key::Z);
        press(&mut harness, Modifiers::COMMAND, Key::Z);
        assert!(children_of(&harness, "ReplicatedStorage").is_empty());

        // Cut moves it to the clipboard; a service is never cut or deleted.
        harness.get_by_role_and_label(Role::TreeItem, "SpawnLocation").click();
        settle(&mut harness);
        press(&mut harness, Modifiers::COMMAND, Key::X);
        assert_eq!(children_of(&harness, "Workspace"), ["Camera", "Baseplate"]);
        harness.get_by_role_and_label(Role::TreeItem, "Lighting").click();
        settle(&mut harness);
        press(&mut harness, Modifiers::NONE, Key::Delete);
        harness.get_by_role_and_label(Role::TreeItem, "Lighting");
        press(&mut harness, Modifiers::COMMAND, Key::V);
        assert_eq!(children_of(&harness, "Lighting"), ["SpawnLocation"]);

        // Rename in place: F2, type, Enter; Escape keeps the name.
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        press(&mut harness, Modifiers::NONE, Key::F2);
        harness.get_by_role_and_label(Role::TextInput, "Name of Baseplate");
        harness.key_press(Key::Backspace);
        harness.event(egui::Event::Text("Floor".to_owned()));
        press(&mut harness, Modifiers::NONE, Key::Enter);
        harness.get_by_role_and_label(Role::TreeItem, "Floor");
        press(&mut harness, Modifiers::NONE, Key::F2);
        harness.event(egui::Event::Text("Nope".to_owned()));
        press(&mut harness, Modifiers::NONE, Key::Escape);
        harness.get_by_role_and_label(Role::TreeItem, "Floor");
        press(&mut harness, Modifiers::COMMAND, Key::Z);
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate");

        // A row's right-click menu: Duplicate.
        let row = harness.get_by_role_and_label(Role::TreeItem, "Baseplate").rect();
        right_click(&mut harness, row);
        harness.get_by_role_and_label(Role::Button, "Duplicate").click();
        settle(&mut harness);
        assert_eq!(children_of(&harness, "Workspace"), ["Camera", "Baseplate", "Baseplate"]);
    }

    #[test]
    fn insert_object_creates_the_chosen_class_inside_the_row_selected_and_undoes() {
        let mut harness = editing(settings_file("editor-insert", None), "Insert Game");
        harness.set_size(egui::vec2(1280.0, 1000.0));
        settle(&mut harness);
        // A row's + opens the window for that row.
        harness
            .get_by_role_and_label(Role::Button, "Insert into Workspace")
            .click();
        settle(&mut harness);
        harness.get_by_label(crate::panels::insert::TITLE);
        harness.get_by_role_and_label(Role::Button, "Insert Part");
        // Its search keeps what matches; choosing inserts, selects, and closes the window.
        harness.event(egui::Event::Text("fold".to_owned()));
        settle(&mut harness);
        assert!(harness.query_by_role_and_label(Role::Button, "Insert Part").is_none());
        harness.get_by_role_and_label(Role::Button, "Insert Folder").click();
        settle(&mut harness);
        assert!(harness.query_by_label(crate::panels::insert::TITLE).is_none());
        assert_eq!(
            children_of(&harness, "Workspace"),
            ["Camera", "Baseplate", "SpawnLocation", "Folder"]
        );
        assert_eq!(selected_names(&harness), ["Folder"]);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(&mut harness);
        assert_eq!(
            children_of(&harness, "Workspace"),
            ["Camera", "Baseplate", "SpawnLocation"]
        );

        // From a row's menu; a part lands in front of the camera.
        let row = harness.get_by_role_and_label(Role::TreeItem, "Workspace").rect();
        right_click(&mut harness, row);
        harness
            .get_by_role_and_label(Role::Button, "Insert Object\u{2026}")
            .click();
        settle(&mut harness);
        let aim = {
            let editor = harness.state().studio.editor().unwrap();
            crate::viewport::aim(&editor.dm, &editor.viewport.camera)
        };
        // Below the fold of the list: the screen reader's click.
        harness
            .get_by_role_and_label(Role::Button, "Insert Part")
            .click_accesskit();
        settle(&mut harness);
        let editor = harness.state().studio.editor().unwrap();
        let part = editor.selection.active().unwrap();
        assert_eq!(editor.dm.class_name(part).unwrap(), "Part");
        // Where the camera looks (#412): resting on what is there, lifted off it by half its size along the surface's
        // normal.
        let at = crate::viewport::position_of(&editor.dm, part).unwrap();
        let normal = aim.normal.expect("the camera looks at something");
        let half = metallic_engine::metallic_core::glam::Vec3::new(4.0, 1.0, 2.0) * 0.5;
        assert!(
            at.abs_diff_eq(aim.point + normal * normal.abs().dot(half), 1e-3),
            "{at} vs {aim:?}"
        );
        assert!(
            (at - editor.viewport.camera.position).dot(editor.viewport.camera.forward()) > 1.0,
            "in front of the camera"
        );

        // Into something with a pivot: at that pivot. A light in a part sits at its middle.
        for (class, place) in [("Part", "at the Baseplate's pivot"), ("PointLight", "at its middle")] {
            harness
                .get_by_role_and_label(Role::Button, "Insert into Baseplate")
                .click();
            settle(&mut harness);
            harness.event(egui::Event::Text(class.to_owned()));
            settle(&mut harness);
            harness
                .get_by_role_and_label(Role::Button, &format!("Insert {class}"))
                .click();
            settle(&mut harness);
            let editor = harness.state().studio.editor().unwrap();
            let new = editor.selection.active().unwrap();
            let baseplate = editor_baseplate(&harness);
            let pivot = metallic_engine::metallic_classes::base::world_object::pivot(&editor.dm, baseplate).unwrap();
            let where_it_is = metallic_engine::metallic_classes::base::world_object::pivot(&editor.dm, new).unwrap();
            assert!(
                where_it_is.position().abs_diff_eq(pivot.position(), 1e-3),
                "{class}: {place}"
            );
        }
    }

    #[test]
    fn shift_space_opens_insert_object_for_the_selection_with_the_explorer_closed() {
        let mut harness = editing(settings_file("editor-insert-key", None), "Insert Key Game");
        harness.set_size(egui::vec2(1280.0, 1000.0));
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        // The window is Studio's, not the Explorer's (#411): it shows with the Explorer closed.
        harness.get_by_role_and_label(Role::Button, "Explorer tool").click();
        settle(&mut harness);
        assert!(!harness.state().studio.dock().is_open(panels::ids::EXPLORER));
        harness.key_press_modifiers(Modifiers::SHIFT, Key::Space);
        settle(&mut harness);
        harness.get_by_label(crate::panels::insert::TITLE);
        harness.event(egui::Event::Text("fold".to_owned()));
        settle(&mut harness);
        harness.get_by_role_and_label(Role::Button, "Insert Folder").click();
        settle(&mut harness);
        assert_eq!(
            children_of_part(&harness, "Baseplate"),
            ["Folder"],
            "inside the selection"
        );
    }

    /// The names of the children of Workspace's `name`.
    fn children_of_part(harness: &Harness<'_, Test>, name: &str) -> Vec<String> {
        let dm = &harness.state().studio.editor().unwrap().dm;
        let workspace = crate::viewport::pick::workspace(dm).unwrap();
        let part = dm.find_first_child(workspace, name, false).unwrap().unwrap();
        dm.children(part)
            .unwrap()
            .map(|id| dm.name(id).unwrap().to_owned())
            .collect()
    }

    #[test]
    fn dragging_explorer_rows_moves_them_inside_or_between_rows_and_undoes() {
        let mut harness = editing(settings_file("editor-drag-rows", None), "Drag Rows Game");
        harness.set_size(egui::vec2(1280.0, 1000.0));
        settle(&mut harness);
        let row = |harness: &Harness<'_, Test>, name: &str| harness.get_by_role_and_label(Role::TreeItem, name).rect();
        // Onto the middle of a row: inside it.
        let (from, to) = (
            row(&harness, "SpawnLocation").center(),
            row(&harness, "ReplicatedStorage").center(),
        );
        drag(&mut harness, from, to);
        assert_eq!(children_of(&harness, "ReplicatedStorage"), ["SpawnLocation"]);
        assert_eq!(
            selected_names(&harness),
            ["SpawnLocation"],
            "the dragged row is selected"
        );
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(&mut harness);
        assert_eq!(
            children_of(&harness, "Workspace"),
            ["Camera", "Baseplate", "SpawnLocation"]
        );
        // Near the top of a row: before it, among its siblings.
        let camera = row(&harness, "Camera");
        let from = row(&harness, "SpawnLocation").center();
        drag(&mut harness, from, egui::pos2(camera.center().x, camera.top() + 2.0));
        assert_eq!(
            children_of(&harness, "Workspace"),
            ["SpawnLocation", "Camera", "Baseplate"]
        );
        // A row onto itself, or a service, does nothing.
        let from = row(&harness, "SpawnLocation").center();
        drag(&mut harness, from, from + egui::vec2(30.0, 0.0));
        let (from, to) = (
            row(&harness, "Lighting").center(),
            row(&harness, "ReplicatedStorage").center(),
        );
        drag(&mut harness, from, to);
        assert_eq!(
            children_of(&harness, "Workspace"),
            ["SpawnLocation", "Camera", "Baseplate"]
        );
        assert!(children_of(&harness, "ReplicatedStorage").is_empty());
    }

    /// Where `name` (in Workspace) shows in the viewport, on the screen.
    fn on_screen(harness: &Harness<'_, Test>, name: &str) -> egui::Pos2 {
        let editor = harness.state().studio.editor().unwrap();
        let rect = editor.viewport.rect.expect("the viewport was drawn");
        let view = crate::viewport::View::new(&editor.viewport.camera, rect);
        let workspace = crate::viewport::pick::workspace(&editor.dm).unwrap();
        let id = editor.dm.find_first_child(workspace, name, false).unwrap().unwrap();
        view.project(crate::viewport::position_of(&editor.dm, id).unwrap())
            .expect("in front of the camera")
    }

    #[test]
    fn a_click_in_the_viewport_selects_the_part_under_the_pointer() {
        let mut harness = editing(settings_file("editor-viewport-pick", None), "Pick Game");
        harness.set_size(egui::vec2(1280.0, 900.0));
        settle(&mut harness);
        let spawn = on_screen(&harness, "SpawnLocation");
        left_click(&mut harness, egui::Rect::from_center_size(spawn, egui::Vec2::ZERO));
        assert_eq!(selected_names(&harness), ["SpawnLocation"]);
        // Ctrl adds what is under the pointer: the baseplate, beside the spawn.
        let floor = spawn + egui::vec2(0.0, 60.0);
        harness.event(egui::Event::PointerMoved(floor));
        for pressed in [true, false] {
            let press = egui::Event::PointerButton {
                pos: floor,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::COMMAND,
            };
            harness.event_modifiers(press, Modifiers::COMMAND);
        }
        settle(&mut harness);
        assert_eq!(selected_names(&harness), ["SpawnLocation", "Baseplate"]);
        // A click on the sky selects nothing.
        let sky = harness
            .state()
            .studio
            .editor()
            .unwrap()
            .viewport
            .rect
            .unwrap()
            .center_top()
            + egui::vec2(0.0, 10.0);
        left_click(&mut harness, egui::Rect::from_center_size(sky, egui::Vec2::ZERO));
        assert!(selected_names(&harness).is_empty());
    }

    #[test]
    fn dragging_a_part_in_the_viewport_carries_it_across_the_floor() {
        let mut harness = editing(settings_file("editor-viewport-carry", None), "Carry Game");
        harness.set_size(egui::vec2(1280.0, 900.0));
        settle(&mut harness);
        let spawn = || {
            let workspace = |editor: &EditorState| crate::viewport::pick::workspace(&editor.dm).unwrap();
            move |harness: &Harness<'_, Test>| {
                let editor = harness.state().studio.editor().unwrap();
                let id = editor
                    .dm
                    .find_first_child(workspace(editor), "SpawnLocation", false)
                    .unwrap()
                    .unwrap();
                crate::viewport::position_of(&editor.dm, id).unwrap()
            }
        };
        let at = spawn();
        let before = at(&harness);
        // Not selected yet: the drag selects it, and carries it over the baseplate, still sitting on it.
        let from = on_screen(&harness, "SpawnLocation");
        drag(&mut harness, from, from + egui::vec2(120.0, 40.0));
        assert_eq!(selected_names(&harness), ["SpawnLocation"]);
        let after = at(&harness);
        assert!((after - before).length() > 1.0, "{before} to {after}");
        assert!((after.y - before.y).abs() < 1e-3, "it stays on the floor");
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(&mut harness);
        assert!((at(&harness) - before).length() < 1e-4);
    }

    #[test]
    fn the_explorer_search_filters_the_tree() {
        let mut harness = editing(settings_file("editor-search", None), "Search Game");
        let explorer = &mut harness.state_mut().studio.editor_mut().unwrap().explorer;
        explorer.set_query("spawn");
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "Workspace");
        harness.get_by_role_and_label(Role::TreeItem, "SpawnLocation");
        assert!(harness.query_by_role_and_label(Role::TreeItem, "Baseplate").is_none());
        assert!(harness.query_by_role_and_label(Role::TreeItem, "Lighting").is_none());

        // A match is selected like any row.
        harness.get_by_role_and_label(Role::TreeItem, "SpawnLocation").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TextInput, "Size");

        let explorer = &mut harness.state_mut().studio.editor_mut().unwrap().explorer;
        explorer.set_query("teleporter");
        settle(&mut harness);
        harness.get_by_label_contains("Nothing is called");
        assert!(harness.query_by_role_and_label(Role::TreeItem, "Workspace").is_none());
    }

    #[test]
    fn a_project_that_cannot_be_opened_goes_back_to_the_projects_with_the_error() {
        let path = config("editor-broken", Some("[general]\nskip_sign_in = true\n"), None);
        let broken = path.with_file_name("Broken.mtproj");
        std::fs::write(&broken, "not a zip archive").unwrap();
        let mut harness = on_the_projects_page(path, &ScriptedDialog::default());
        harness.state_mut().studio.open(broken);
        assert_eq!(harness.state().studio.screen(), Screen::Editor);
        wait_open(&mut harness);
        let studio = &harness.state().studio;
        assert_eq!(studio.screen(), Screen::Projects);
        assert!(studio.editor().is_none() && studio.open_project().is_none());
        let error = studio.projects().error().unwrap();
        assert!(error.starts_with("Could not open \u{201C}Broken\u{201D}."), "{error}");
        harness.get_by_label_contains("Could not open \u{201C}Broken\u{201D}");
    }

    #[test]
    fn the_view_menu_and_close_buttons_change_the_layout_which_persists() {
        let path = settings_file("editor-layout", None);
        let layout = path.with_file_name(crate::dock::LAYOUT_FILE);
        let mut harness = editing(path.clone(), "Layout Game");
        assert!(!layout.exists(), "nothing changed, nothing written");

        click_label(&mut harness, "View");
        let output = harness.get_by_role_and_label(Role::MenuItemCheckBox, "Output");
        assert_eq!(
            egui_kittest::kittest::NodeT::accesskit_node(&output).toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        output.click();
        settle(&mut harness);
        assert!(!harness.state().studio.dock().is_open(panels::ids::OUTPUT));
        assert!(harness.query_by_role_and_label(Role::Tab, "Output").is_none());
        let text = std::fs::read_to_string(&layout).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
        assert!(!text.contains("\"output\"") && text.contains("\"explorer\""), "{text}");

        // A tab's close button.
        click_label(&mut harness, "Close Properties");
        assert!(!harness.state().studio.dock().is_open(panels::ids::PROPERTIES));

        drop(harness);

        // Started again, Studio has the same layout.
        let mut harness = editing(path, "Layout Game");
        assert_eq!(
            harness.state().studio.dock().open_panels(),
            [panels::ids::VIEWPORT, panels::ids::EXPLORER, panels::ids::ASSETS]
        );
        // The View menu's check marks say the same.
        click_label(&mut harness, "View");
        let output = harness.get_by_role_and_label(Role::MenuItemCheckBox, "Output");
        assert_eq!(
            egui_kittest::kittest::NodeT::accesskit_node(&output).toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        output.click();
        settle(&mut harness);
        assert!(harness.state().studio.dock().is_open(panels::ids::OUTPUT));

        // Window, Reset layout.
        click_label(&mut harness, "Window");
        harness.get_by_role_and_label(Role::MenuItem, "Reset layout").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.dock().open_panels(), DEFAULT_PANELS);
        let text = std::fs::read_to_string(&layout).unwrap();
        assert!(text.contains("\"properties\""), "{text}");
    }

    // The menu bar (#239).

    /// Whether the menu item `label` is shown enabled.
    fn menu_item_enabled(harness: &Harness<'_, Test>, label: &str) -> bool {
        let item = harness.get_by_role_and_label(Role::MenuItem, label);
        !egui_kittest::kittest::NodeT::accesskit_node(&item).is_disabled()
    }

    #[test]
    fn the_seven_menus_open_with_working_items_enabled_and_planned_ones_not() {
        let mut harness = editing(settings_file("editor-menus", None), "Menus Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        for menu in editor::MENUS {
            click_label(&mut harness, menu.title);
            for entry in menu.entries {
                match *entry {
                    editor::Entry::Item { label, action, .. } => {
                        // Without a game, Pause, Step and Stop are off (#246).
                        let expected = editor::MenuState::default().allows(action);
                        assert_eq!(menu_item_enabled(&harness, label), expected, "{}: {label}", menu.title);
                    }
                    editor::Entry::Planned(label) => {
                        assert!(!menu_item_enabled(&harness, label), "{}: {label}", menu.title);
                    }
                    editor::Entry::Panels => {
                        harness.get_by_role_and_label(Role::MenuItemCheckBox, "Explorer");
                    }
                    editor::Entry::Separator => {}
                }
            }
            // Compact (#276): one width per menu, as its longest line needs, not the old fixed 240.
            let items: Vec<egui::Rect> = harness
                .query_all_by_role(Role::MenuItem)
                .chain(harness.query_all_by_role(Role::MenuItemCheckBox))
                .map(|item| item.rect())
                .collect();
            let width = items[0].width();
            assert!(
                items.iter().all(|item| (item.width() - width).abs() < 0.01),
                "{}",
                menu.title
            );
            assert!(width < 240.0, "{} is {width} wide", menu.title);
            assert!(width >= 150.0, "{} is {width} wide", menu.title);
            // Escape closes it, ready for the next.
            harness.key_press(Key::Escape);
            settle(&mut harness);
            assert!(harness.query_by_role(Role::MenuItem).is_none(), "{} closed", menu.title);
        }

        // A planned item does nothing when clicked.
        click_label(&mut harness, "File");
        harness.get_by_role_and_label(Role::MenuItem, "New project").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Editor);

        // File, Settings opens the Settings window; Window, Settings closes it.
        harness.key_press(Key::Escape);
        settle(&mut harness);
        click_label(&mut harness, "File");
        harness.get_by_role_and_label(Role::MenuItem, "Settings").click();
        settle(&mut harness);
        assert!(harness.state().studio.is_settings_open());
        click_label(&mut harness, "Window");
        harness.get_by_role_and_label(Role::MenuItem, "Settings").click();
        settle(&mut harness);
        assert!(!harness.state().studio.is_settings_open());

        // Help, Keyboard shortcuts opens Settings on its Keybinds page.
        click_label(&mut harness, "Help");
        harness
            .get_by_role_and_label(Role::MenuItem, "Keyboard shortcuts")
            .click();
        settle(&mut harness);
        assert!(harness.state().studio.is_settings_open());
        assert_eq!(
            harness.state_mut().studio.settings_window().category(),
            Category::Keybinds
        );
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);

        // Help, About shows the version, and Close closes it.
        click_label(&mut harness, "Help");
        harness
            .get_by_role_and_label(Role::MenuItem, "About Metallic Studio")
            .click();
        settle(&mut harness);
        assert!(harness.state().studio.is_about_open());
        harness.get_by_label(&format!("Version {}", env!("CARGO_PKG_VERSION")));
        harness.get_by_role_and_label(Role::Button, "Close").click();
        settle(&mut harness);
        assert!(!harness.state().studio.is_about_open());
        // Its header's Lucide close button closes it too (#278).
        click_label(&mut harness, "Help");
        harness
            .get_by_role_and_label(Role::MenuItem, "About Metallic Studio")
            .click();
        settle(&mut harness);
        harness
            .get_by_role_and_label(Role::Button, "Close About Metallic Studio")
            .click();
        settle(&mut harness);
        assert!(!harness.state().studio.is_about_open());

        // File, Close project goes back to the projects.
        click_label(&mut harness, "File");
        harness.get_by_role_and_label(Role::MenuItem, "Close project").click();
        settle(&mut harness);
        assert_eq!(harness.state().studio.screen(), Screen::Projects);
    }

    #[test]
    fn the_menus_show_their_commands_bindings() {
        let path = settings_file(
            "editor-menu-keys",
            Some("[keybinds]\n\"view.reset_layout\" = [\"F9\"]\n"),
        );
        let mut harness = editing(path, "Keys Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        // The bindings are painted beside their items.
        let painted = |harness: &Harness<'_, Test>, text: &str| {
            harness.output().shapes.iter().any(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) => shape.galley.text() == text,
                _ => false,
            })
        };
        assert!(!painted(&harness, "F9"));
        click_label(&mut harness, "Window");
        assert!(painted(&harness, "F9"), "Reset layout shows its binding");
        harness.key_press(Key::Escape);
        settle(&mut harness);
        click_label(&mut harness, "File");
        let save = harness.state().studio.keymap().shortcut_text(ids::FILE_SAVE).unwrap();
        assert!(painted(&harness, &save), "Save shows {save}");
    }

    // Play in Studio: the Game tab with Play, Pause, Step and Stop (#246).

    use crate::logging::LogBuffer;
    use metallic_engine::metallic_core::Value;

    /// The editor's game has a Script whose source `Scripts/Play.luau` prints a line, then on Heartbeat turns the
    /// Baseplate and counts the game's frames in a `NumberValue` called Frames under `ReplicatedStorage`.
    fn add_play_script(harness: &mut Harness<'static, Test>) {
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        let dm = &mut editor.dm;
        let registry = dm.registry().clone();
        let class = |name: &str| registry.class_id(name).unwrap();
        let service = dm.service(class("ServerScriptService")).unwrap();
        let script = dm.create(class("Script")).unwrap();
        dm.set_name(script, "Play".into()).unwrap();
        dm.set_by_name(script, "Source", Value::String("Scripts/Play.luau".into()))
            .unwrap();
        dm.set_parent(script, Some(service)).unwrap();
        editor
            .project
            .write(
                "Scripts/Play.luau",
                "print('hello from play')\n\
                 local frames = Instance.new('NumberValue')\n\
                 frames.Name = 'Frames'\n\
                 frames.Parent = game:GetService('ReplicatedStorage')\n\
                 local part = workspace:FindFirstChild('Baseplate') :: Part\n\
                 game:GetService('RunService').Heartbeat:Connect(function(dt)\n\
                     frames.Value += 1\n\
                     part.CFrame = part.CFrame * CFrame.Angles(0, dt, 0)\n\
                 end)\n",
            )
            .unwrap();
    }

    /// `Workspace.Baseplate`'s `CFrame` in `dm`.
    fn baseplate(dm: &metallic_engine::metallic_core::DataModel) -> Value {
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        let part = dm.find_first_child(workspace, "Baseplate", false).unwrap().unwrap();
        dm.get_by_name(part, "CFrame").unwrap()
    }

    /// The game's frame count (the Play script's `Frames` value).
    fn game_frames(studio: &Studio) -> f64 {
        let session = studio.editor().unwrap().play.as_ref().unwrap();
        let dm = session.player.engine().data_model();
        let storage = dm
            .service(dm.registry().class_id("ReplicatedStorage").unwrap())
            .unwrap();
        let frames = dm.find_first_child(storage, "Frames", false).unwrap().unwrap();
        match dm.get_by_name(frames, "Value").unwrap() {
            Value::Number(n) => n,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_play_controls_are_filled_and_coloured_by_state() {
        let mut harness = editing(settings_file("play-controls", None), "Controls Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        let colors = current_colors(&harness.ctx);
        let grey = colors.palette.text.gamma_multiply(0.48);
        let green = editor::Control::Play.ink(&colors);
        let red = editor::Control::Stop.ink(&colors);
        // The top bar's controls (the toolbar's Home tab has tools of the same icons, #232).
        let bar = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, editor::BAR_HEIGHT + 8.0));
        let icon_meshes = |harness: &Harness<'_, Test>, icon| icon_meshes_in(harness, icon, bar);
        // No game: Play is a green icon on no green fill; Pause, Step and Stop are greyed out (#302).
        assert_eq!(icon_meshes(&harness, Icon::ToolbarPlay), [green]);
        assert_eq!(icon_meshes(&harness, Icon::ToolbarStop), [grey]);
        assert_eq!(icon_meshes(&harness, Icon::ToolbarPause), [grey]);
        let play = harness.get_by_role_and_label(Role::Button, "Play").rect();
        let green_fill = harness.output().shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => rect.rect == play && rect.fill == tokens::play::FILL,
            _ => false,
        });
        assert!(!green_fill, "Play has no green background");

        // Playing: Play is greyed out, Pause white, Stop a red icon.
        harness.key_press(Key::F5);
        settle(&mut harness);
        assert!(harness.state().studio.is_playing());
        assert_eq!(icon_meshes(&harness, Icon::ToolbarPlay), [grey]);
        assert_eq!(icon_meshes(&harness, Icon::ToolbarPause), [colors.palette.text]);
        assert_eq!(icon_meshes(&harness, Icon::ToolbarStop), [red]);
        assert_ne!(red, grey);
        harness.key_press_modifiers(Modifiers::SHIFT, Key::F5);
        settle(&mut harness);
    }

    #[test]
    fn the_toolbar_s_play_controls_look_like_the_top_bar_s() {
        let mut harness = editing(settings_file("toolbar-play", None), "Toolbar Play Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        let colors = current_colors(&harness.ctx);
        let grey = colors.palette.text.gamma_multiply(0.48);
        let (green, red) = (editor::Control::Play.ink(&colors), editor::Control::Stop.ink(&colors));
        // Each tool's icon in its button: its colour, and whether it is filled.
        let drawn = |harness: &Harness<'_, Test>, icon: Icon, label: &str| {
            let area = harness
                .get_by_role_and_label(Role::Button, &format!("{label} tool"))
                .rect();
            let filled = crate::theme::icons::builtin::textures_of_kind(&harness.ctx, icon, true);
            let all = crate::theme::icons::builtin::textures_of(&harness.ctx, icon);
            harness
                .output()
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Mesh(mesh)
                        if all.contains(&mesh.texture_id) && area.contains_rect(mesh.calc_bounds()) =>
                    {
                        mesh.vertices
                            .first()
                            .map(|v| (v.color, filled.contains(&mesh.texture_id)))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        // No game: Play a filled green triangle; Stop greyed out but filled, as on the top bar.
        assert_eq!(drawn(&harness, Icon::ToolbarPlay, "Play"), [(green, true)]);
        assert_eq!(drawn(&harness, Icon::ToolbarStop, "Stop"), [(grey, true)]);
        assert_eq!(drawn(&harness, Icon::ToolbarPause, "Pause"), [(grey, false)]);
        // Playing: Play greyed out, Pause in the text colour, Stop a filled red square.
        harness.key_press(Key::F5);
        settle(&mut harness);
        assert!(harness.state().studio.is_playing());
        assert_eq!(drawn(&harness, Icon::ToolbarPlay, "Play"), [(grey, true)]);
        assert_eq!(
            drawn(&harness, Icon::ToolbarPause, "Pause"),
            [(colors.palette.text, false)]
        );
        assert_eq!(drawn(&harness, Icon::ToolbarStop, "Stop"), [(red, true)]);
        harness.key_press_modifiers(Modifiers::SHIFT, Key::F5);
        settle(&mut harness);
    }

    #[test]
    fn play_runs_the_scene_in_a_game_tab_and_stop_puts_everything_back() {
        let path = settings_file("editor-play", None);
        let layout = path.with_file_name(crate::dock::LAYOUT_FILE);
        let log = Arc::new(LogBuffer::new());
        let mut harness = editing(path, "Play Game");
        harness.state_mut().studio.set_log(Arc::clone(&log));
        harness.state_mut().studio.editor_mut().unwrap().output =
            crate::panels::output::OutputState::new(Arc::clone(&log));
        add_play_script(&mut harness);
        settle(&mut harness);
        let before = baseplate(&harness.state().studio.editor().unwrap().dm);
        assert!(
            harness.query_by_role_and_label(Role::Tab, "Game").is_none(),
            "no Game tab yet"
        );
        log.push(
            log::Level::Info,
            "metallic_studio::app",
            "before play",
            crate::logging::Origin::Engine,
        );
        settle(&mut harness);
        let output = |harness: &Harness<'_, Test>| -> Vec<String> {
            let editor = harness.state().studio.editor().unwrap();
            editor.output.lines().iter().map(|line| line.message.clone()).collect()
        };
        assert!(output(&harness).contains(&"before play".to_owned()));

        // F5: the Game tab opens in front, and the scene's script runs in the Player.
        harness.key_press(Key::F5);
        settle(&mut harness);
        let studio = &harness.state().studio;
        assert!(studio.is_playing());
        assert!(studio.dock().is_open(panels::ids::GAME));
        harness.get_by_role_and_label(Role::Tab, "Game");
        let mut lines = Vec::new();
        log.read_since(0, &mut lines);
        assert!(
            lines
                .iter()
                .any(|line| line.message == "hello from play" && line.origin == crate::logging::Origin::Script),
            "{lines:?}"
        );
        // The Output shows the new session alone (#383).
        let shown = output(&harness);
        assert!(!shown.contains(&"before play".to_owned()), "{shown:?}");
        assert!(shown.contains(&"hello from play".to_owned()), "{shown:?}");
        let studio = &harness.state().studio;
        let session = studio.editor().unwrap().play.as_ref().unwrap();
        assert_ne!(
            baseplate(session.player.engine().data_model()),
            before,
            "the game's Baseplate turns"
        );
        assert_eq!(baseplate(&studio.editor().unwrap().dm), before, "the editor's does not");

        // Save is off while playing, in the menu and by its shortcut.
        click_label(&mut harness, "File");
        assert!(!menu_item_enabled(&harness, "Save"));
        harness.key_press(Key::Escape);
        settle(&mut harness);
        assert!(menu_item_enabled_after(&mut harness, "Test", "Stop"));

        // F7 pauses: the game's frames stop. F6 steps one frame. F7 again resumes.
        harness.key_press(Key::F7);
        settle(&mut harness);
        assert!(harness.state().studio.editor().unwrap().is_paused());
        let paused = game_frames(&harness.state().studio);
        settle(&mut harness);
        assert_eq!(
            game_frames(&harness.state().studio),
            paused,
            "nothing runs while paused"
        );
        harness.key_press(Key::F6);
        settle(&mut harness);
        assert_eq!(
            game_frames(&harness.state().studio),
            paused + 1.0,
            "a step is one frame"
        );
        harness.key_press(Key::F7);
        settle(&mut harness);
        assert!(!harness.state().studio.editor().unwrap().is_paused());
        assert!(game_frames(&harness.state().studio) > paused + 1.0);

        // The layout is saved without the Game tab.
        harness.state_mut().studio.dock_mut().close(panels::ids::OUTPUT);
        settle(&mut harness);
        let text = std::fs::read_to_string(&layout).unwrap();
        assert!(!text.contains("\"game\""), "{text}");

        // Shift+F5 stops: the Game tab goes, the Viewport is in front, and the editor's scene is as it was.
        harness.key_press_modifiers(Modifiers::SHIFT, Key::F5);
        settle(&mut harness);
        let studio = &harness.state().studio;
        assert!(!studio.is_playing());
        assert!(!studio.dock().is_open(panels::ids::GAME));
        assert!(harness.query_by_role_and_label(Role::Tab, "Game").is_none());
        assert_eq!(baseplate(&studio.editor().unwrap().dm), before);

        // And again.
        harness.key_press(Key::F5);
        settle(&mut harness);
        assert!(harness.state().studio.is_playing());
        assert!(game_frames(&harness.state().studio) >= 1.0, "a new game from the start");
        harness.key_press_modifiers(Modifiers::SHIFT, Key::F5);
        settle(&mut harness);
        assert!(!harness.state().studio.is_playing());
    }

    /// Opens menu `menu` and says whether its item `label` is enabled, then closes the menu.
    fn menu_item_enabled_after(harness: &mut Harness<'static, Test>, menu: &str, label: &str) -> bool {
        click_label(harness, menu);
        let enabled = menu_item_enabled(harness, label);
        harness.key_press(Key::Escape);
        settle(harness);
        enabled
    }

    #[test]
    fn the_play_buttons_and_the_test_menu_drive_a_session() {
        let mut harness = editing(settings_file("editor-play-buttons", None), "Buttons Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        add_play_script(&mut harness);
        settle(&mut harness);
        // Without a game only Play is on.
        let enabled = |harness: &Harness<'_, Test>, label: &str| {
            let button = harness.get_by_role_and_label(Role::Button, label);
            !egui_kittest::kittest::NodeT::accesskit_node(&button).is_disabled()
        };
        assert!(enabled(&harness, "Play"));
        for label in ["Pause", "Step", "Stop"] {
            assert!(!enabled(&harness, label), "{label}");
        }
        harness.get_by_role_and_label(Role::Button, "Play").click();
        settle(&mut harness);
        assert!(harness.state().studio.is_playing());
        assert!(!enabled(&harness, "Play"));
        assert!(enabled(&harness, "Pause") && enabled(&harness, "Stop") && !enabled(&harness, "Step"));
        harness.get_by_role_and_label(Role::Button, "Pause").click();
        settle(&mut harness);
        assert!(enabled(&harness, "Resume") && enabled(&harness, "Step"));
        // Test, Stop.
        click_label(&mut harness, "Test");
        harness.get_by_role_and_label(Role::MenuItem, "Stop").click();
        settle(&mut harness);
        assert!(!harness.state().studio.is_playing());
        // Test, Play.
        click_label(&mut harness, "Test");
        harness.get_by_role_and_label(Role::MenuItem, "Play").click();
        settle(&mut harness);
        assert!(harness.state().studio.is_playing());
    }

    // Icons coloured by group and a custom icon folder (#231).

    #[test]
    fn a_custom_icon_folder_replaces_the_explorer_s_part_icon() {
        use crate::theme::icons::custom;

        let mut harness = editing(settings_file("custom-icons", None), "Icons Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        // Every part is drawn with the built-in SVG (#291), in its group's colour.
        let colors = current_colors(&harness.ctx);
        let part_glyphs = |harness: &Harness<'_, Test>| icon_meshes(harness, Icon::ExplorerPart).len();
        assert!(part_glyphs(&harness) >= 2, "Baseplate and SpawnLocation");
        let tinted = icon_meshes(&harness, Icon::ExplorerPart)
            .iter()
            .all(|&tint| tint == Icon::ExplorerPart.color(&colors));
        assert!(tinted, "in its group's colour");

        // A folder with explorer.part.svg: read in the background, then drawn in place of the glyph.
        let dir = std::env::temp_dir().join(format!("metallic-studio-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("explorer.part.svg"),
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="16" fill="currentColor"/></svg>"#,
        )
        .unwrap();
        harness
            .state_mut()
            .studio
            .settings
            .settings_mut()
            .appearance
            .icon_folder = dir.display().to_string();
        for _ in 0..2000 {
            harness.step();
            if custom::get(&harness.ctx, Icon::ExplorerPart).is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let texture = custom::get(&harness.ctx, Icon::ExplorerPart)
            .expect("the folder's part icon")
            .texture
            .id();
        settle(&mut harness);
        assert_eq!(part_glyphs(&harness), 0, "no built-in part icon left");
        let images = harness
            .output()
            .shapes
            .iter()
            .filter(|clipped| matches!(&clipped.shape, egui::Shape::Mesh(mesh) if mesh.texture_id == texture))
            .count();
        assert!(images >= 2, "Baseplate and SpawnLocation draw the folder's icon");
        // Saved with the settings.
        assert_eq!(
            harness.state().studio.settings().appearance.icon_folder,
            dir.display().to_string()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Dock tabs: titles only (#255), the active one by its fill (#288).

    #[test]
    fn dock_tabs_show_their_title_alone_and_the_active_one_by_its_fill() {
        let mut harness = editing(settings_file("editor-tab-style", None), "Tabs Style Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        let colors = current_colors(&harness.ctx);
        let active = crate::dock::active_tab_fill(&colors.palette);
        assert_ne!(active, colors.palette.header, "apart from the tab bar");
        let output = harness.output();
        // Fills of rects exactly covering `tab`.
        let fills = |tab: egui::Rect| -> Vec<egui::Color32> {
            output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Rect(rect) if rect.rect == tab => Some(rect.fill),
                    _ => None,
                })
                .collect()
        };
        // Output is in front of Assets: it has the active fill, Assets none (#288).
        let output_tab = harness.get_by_role_and_label(Role::Tab, "Output").rect();
        let assets_tab = harness.get_by_role_and_label(Role::Tab, "Assets").rect();
        assert_eq!(fills(output_tab), [active]);
        assert!(fills(assets_tab).is_empty(), "an inactive tab is the bar's colour");
        for (title, icon) in [("Explorer", Icon::PanelExplorer), ("Viewport", Icon::PanelViewport)] {
            let tab = harness.get_by_role_and_label(Role::Tab, title).rect();
            // No accent underline any more.
            let underline = output.shapes.iter().any(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) => {
                    rect.fill == colors.accent && tab.contains_rect(rect.rect) && rect.rect.height() < 4.0
                }
                _ => false,
            });
            assert!(!underline, "{title} has an accent line");
            // No icon in the tab.
            let textures = crate::theme::icons::builtin::textures_of(&harness.ctx, icon);
            let has_icon = output.shapes.iter().any(|clipped| match &clipped.shape {
                egui::Shape::Mesh(mesh) => textures.contains(&mesh.texture_id) && tab.intersects(mesh.calc_bounds()),
                _ => false,
            });
            assert!(!has_icon, "{title} draws its icon");
        }
    }

    // Live editing while playing (#248).

    #[test]
    fn while_playing_the_explorer_and_properties_show_the_running_game() {
        let mut harness = editing(settings_file("editor-live", None), "Live Game");
        harness.set_size(egui::vec2(1280.0, 900.0));
        add_play_script(&mut harness);
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        let editor_selection = harness.state().studio.editor().unwrap().selection.clone();
        assert!(
            harness.query_by_label_contains("Running game").is_none(),
            "no banner before Play"
        );

        // Play: the selection comes along (by scene id), Properties shows the game's Baseplate, and the Explorer
        // shows what only the running game has: the script's Frames value.
        harness.key_press(Key::F5);
        settle(&mut harness);
        // No banner while playing either (#309).
        assert!(harness.query_by_label_contains("Running game").is_none());
        {
            let studio = &mut harness.state_mut().studio;
            let editor = studio.editor_mut().unwrap();
            let scene = editor.scene();
            assert!(scene.playing);
            let selected = scene.selection.active().expect("carried into the game");
            assert_eq!(scene.dm.name(selected).unwrap(), "Baseplate");
            // The game's own id, not the editor's.
            let session = editor.play.as_ref().unwrap();
            assert!(session.player.engine().data_model().contains(selected));
        }
        harness.get_by_label("TRANSFORM");
        let storage = harness.get_by_role_and_label(Role::TreeItem, "ReplicatedStorage");
        storage.click();
        settle(&mut harness);
        // Expanding goes through the caret; the row click selects it, and Properties follows the game.
        {
            let scene = harness.state_mut().studio.editor_mut().unwrap().scene();
            let storage = scene.selection.active().expect("ReplicatedStorage selected");
            assert_eq!(scene.dm.name(storage).unwrap(), "ReplicatedStorage");
            scene.explorer.set_expanded(storage, true);
        }
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "Frames");
        assert!(
            harness.state().studio.editor().unwrap().selection == editor_selection,
            "the editor's selection is untouched"
        );

        // A change made to the game through the scene the panels edit is the game's at once.
        {
            let editor = harness.state_mut().studio.editor_mut().unwrap();
            let scene = editor.scene();
            let storage = scene.selection.active().unwrap();
            let frames = scene.dm.find_first_child(storage, "Frames", false).unwrap().unwrap();
            scene.dm.set_name(frames, "Ticks".into()).unwrap();
        }
        settle(&mut harness);
        harness.get_by_role_and_label(Role::TreeItem, "Ticks");

        // Stop: the editor's tree, selection and Explorer are as they were.
        harness.key_press_modifiers(Modifiers::SHIFT, Key::F5);
        settle(&mut harness);
        assert!(harness.query_by_label_contains("Running game").is_none());
        assert!(harness.query_by_role_and_label(Role::TreeItem, "Ticks").is_none());
        assert_eq!(harness.state().studio.editor().unwrap().selection, editor_selection);
        harness.get_by_label("TRANSFORM");
    }

    // The input hand-off to the game (#247).

    /// The lines the game printed since the session began.
    fn game_prints(log: &LogBuffer) -> Vec<String> {
        let mut lines = Vec::new();
        log.read_since(0, &mut lines);
        lines
            .into_iter()
            .filter(|line| line.origin == crate::logging::Origin::Script)
            .map(|line| line.message)
            .collect()
    }

    #[test]
    fn the_game_gets_the_keys_while_its_tab_has_the_focus_and_esc_gives_them_back() {
        let log = Arc::new(LogBuffer::new());
        let mut harness = editing(settings_file("editor-game-input", None), "Input Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        harness.state_mut().studio.set_log(Arc::clone(&log));
        {
            let editor = harness.state_mut().studio.editor_mut().unwrap();
            let dm = &mut editor.dm;
            let registry = dm.registry().clone();
            let service = dm.service(registry.class_id("ServerScriptService").unwrap()).unwrap();
            let script = dm.create(registry.class_id("Script").unwrap()).unwrap();
            dm.set_by_name(script, "Source", Value::String("Scripts/Keys.luau".into()))
                .unwrap();
            dm.set_parent(script, Some(service)).unwrap();
            editor
                .project
                .write(
                    "Scripts/Keys.luau",
                    "game:GetService('UserInputService').InputBegan:Connect(function(input)\n\
                         print('began ' .. input.KeyCode.Name)\n\
                     end)\n",
                )
                .unwrap();
        }
        settle(&mut harness);
        let camera_before = camera(&harness);

        // Play gives the game the input: W reaches it, and not the editor camera.
        harness.key_press(Key::F5);
        settle(&mut harness);
        assert!(harness.state().studio.game_has_input(&harness.ctx));
        hold(&mut harness, Key::W);
        settle(&mut harness);
        assert!(
            game_prints(&log).contains(&"began W".to_owned()),
            "{:?}",
            game_prints(&log)
        );
        assert_eq!(camera(&harness), camera_before);

        // Esc: the game sees it, then Studio has the keys back. W no longer reaches the game.
        harness.event(physical(Key::Escape, true));
        harness.step();
        harness.event(physical(Key::Escape, false));
        settle(&mut harness);
        assert!(game_prints(&log).contains(&"began Escape".to_owned()));
        assert!(!harness.state().studio.game_has_input(&harness.ctx));
        let seen = game_prints(&log).len();
        hold(&mut harness, Key::W);
        settle(&mut harness);
        assert_eq!(game_prints(&log).len(), seen, "W stays with Studio");

        // A click in the Game tab gives the game the keys again; Shift+F5 still stops it.
        let tab = area_of(&harness.state().studio, panels::ids::GAME).center();
        harness.event(egui::Event::PointerMoved(tab));
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: tab,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            harness.step();
        }
        settle(&mut harness);
        assert!(harness.state().studio.game_has_input(&harness.ctx));
        hold(&mut harness, Key::D);
        settle(&mut harness);
        assert!(game_prints(&log).contains(&"began D".to_owned()));
        harness.key_press_modifiers(Modifiers::SHIFT, Key::F5);
        settle(&mut harness);
        assert!(!harness.state().studio.is_playing());
    }

    // Settings as a floating window in the editor (#238).

    #[test]
    fn settings_opens_in_a_window_of_its_own_where_one_can_open() {
        use std::cell::RefCell;

        thread_local! {
            /// What the stand-in for the shell was asked to draw: each viewport's id and title.
            static DRAWN: RefCell<Vec<(egui::ViewportId, Option<String>)>> = const { RefCell::new(Vec::new()) };
            /// Whether its window's close button is pressed.
            static CLOSING: RefCell<bool> = const { RefCell::new(false) };
            /// The labels its contents gave screen readers.
            static LABELS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        }
        let path = settings_file("own-window", None);
        let mut harness = editing(path, "Own Window Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        // The shell's part (#331), as `app::shell` does it with a real window: each immediate viewport drawn in a pass
        // of its own, with its own input.
        egui::Context::set_immediate_viewport_renderer(|ctx, viewport| {
            let egui::ImmediateViewport {
                ids,
                builder,
                mut viewport_ui_cb,
            } = viewport;
            DRAWN.with(|drawn| drawn.borrow_mut().push((ids.this, builder.title.clone())));
            let mut input = egui::RawInput {
                viewport_id: ids.this,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(780.0, 640.0))),
                ..Default::default()
            };
            let info = input.viewports.entry(ids.this).or_default();
            if CLOSING.with(|closing| *closing.borrow()) {
                info.events.push(egui::ViewportEvent::Close);
            }
            ctx.enable_accesskit();
            let mut output = ctx.run_ui(input, |ui| viewport_ui_cb(ui));
            output.textures_delta.clear();
            let labels = output
                .platform_output
                .accesskit_update
                .map(|update| {
                    update
                        .nodes
                        .iter()
                        .filter_map(|(_, node)| node.label().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            LABELS.with(|seen| *seen.borrow_mut() = labels);
        });
        harness.ctx.set_embed_viewports(false);
        harness.get_by_role_and_label(Role::Button, "Settings").click();
        settle(&mut harness);
        assert!(harness.state().studio.is_settings_open());
        // Drawn in a window of its own called Settings, with its pages there and not in Studio's window.
        let drawn = DRAWN.with(|drawn| drawn.borrow().clone());
        assert!(
            drawn.iter().any(|(_, title)| title.as_deref() == Some("Settings")),
            "{drawn:?}"
        );
        assert!(drawn.iter().all(|(viewport, _)| *viewport != egui::ViewportId::ROOT));
        let labels = LABELS.with(|labels| labels.borrow().clone());
        assert!(labels.iter().any(|label| label == "Appearance"), "{labels:?}");
        assert!(harness.query_by_role_and_label(Role::Tab, "Appearance").is_none());
        // Its window's close button closes Settings.
        CLOSING.with(|closing| *closing.borrow_mut() = true);
        settle(&mut harness);
        assert!(!harness.state().studio.is_settings_open());
    }

    #[test]
    fn in_the_editor_settings_is_a_floating_window_that_applies_and_saves() {
        let path = settings_file("editor-settings-window", None);
        let mut harness = editing(path.clone(), "Settings Game");
        // Room around the window, so the editor can be clicked beside it.
        harness.set_size(egui::vec2(1600.0, 1000.0));
        settle(&mut harness);
        let panels_before = harness.state().studio.dock().open_panels();
        // The gear opens the window over the editor; the dock does not change.
        harness.get_by_role_and_label(Role::Button, "Settings").click();
        settle(&mut harness);
        assert!(harness.state().studio.is_settings_open());
        harness.get_by_role_and_label(Role::Tab, "General");
        assert_eq!(harness.state().studio.dock().open_panels(), panels_before);
        assert!(
            harness.query_by_role_and_label(Role::Tab, "Settings").is_none(),
            "not a tab"
        );
        // The editor is still there under it.
        harness.get_by_role_and_label(Role::Tab, "Explorer");

        // A setting changed in it applies at once and is saved.
        harness.get_by_role_and_label(Role::Tab, "Appearance").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::Button, "Light").click();
        settle(&mut harness);
        assert_eq!(
            harness.state().studio.settings().appearance.mode,
            crate::theme::ThemeMode::Light
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("mode = \"light\""), "{text}");

        // While it has the focus, the panels' shortcuts do not reach the viewport under it.
        let viewport = area_of(&harness.state().studio, panels::ids::VIEWPORT);
        harness
            .state_mut()
            .studio
            .dock_mut()
            .set_focused(Some(panels::ids::VIEWPORT));
        let window_has_focus = harness.state_mut().studio.settings_window().has_focus();
        assert!(window_has_focus, "the last click was in it");
        let before = camera(&harness);
        hold(&mut harness, Key::W);
        assert_eq!(
            camera(&harness),
            before,
            "W in the Settings window does not fly the camera"
        );
        // A click in the viewport gives it the focus back.
        let beside = viewport.left_bottom() + egui::vec2(20.0, -20.0);
        let window = harness.state_mut().studio.settings_window().contains(beside);
        assert!(!window, "{beside:?} is clear of the window");
        harness.event(egui::Event::PointerMoved(beside));
        harness.event(egui::Event::PointerButton {
            pos: beside,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        harness.event(egui::Event::PointerButton {
            pos: beside,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        settle(&mut harness);
        assert!(!harness.state_mut().studio.settings_window().has_focus());
        hold(&mut harness, Key::W);
        assert_ne!(camera(&harness), before, "W in the viewport flies the camera again");

        // Primary+, closes it, and opens it again.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(!harness.state().studio.is_settings_open());
        assert!(harness.query_by_role_and_label(Role::Tab, "General").is_none());
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        settle(&mut harness);
        assert!(harness.state().studio.is_settings_open());
    }

    #[test]
    fn reset_layout_is_a_command_and_the_menus_show_bindings() {
        let path = settings_file("editor-reset", Some("[keybinds]\n\"view.reset_layout\" = [\"F9\"]\n"));
        let mut harness = editing(path, "Reset Game");
        harness.state_mut().studio.dock_mut().close(panels::ids::EXPLORER);
        settle(&mut harness);
        assert!(!harness.state().studio.dock().is_open(panels::ids::EXPLORER));
        harness.key_press(Key::F9);
        settle(&mut harness);
        assert_eq!(harness.state().studio.dock().open_panels(), DEFAULT_PANELS);
    }

    #[test]
    fn saving_writes_the_scene_into_the_project() {
        let mut harness = editing(settings_file("editor-save", None), "Save Game");
        let editor = harness.state_mut().studio.editor_mut().unwrap();
        let workspace = editor
            .dm
            .service(editor.dm.registry().class_id("Workspace").unwrap())
            .unwrap();
        let baseplate = editor
            .dm
            .find_first_child(workspace, "Baseplate", false)
            .unwrap()
            .unwrap();
        editor.dm.set_name(baseplate, "Floor".into()).unwrap();
        // The window's title marks the unsaved change (#254), until the save.
        settle(&mut harness);
        assert_eq!(harness.state().studio.window_title(), "Save Game* - Metallic Studio");
        harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
        settle(&mut harness);
        assert!(harness.state().studio.notice().is_none());
        assert_eq!(harness.state().studio.window_title(), "Save Game - Metallic Studio");
        let file = harness.state().studio.editor().unwrap().path().to_path_buf();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::W);
        settle(&mut harness);
        assert_eq!(
            harness.state().studio.window_title(),
            "Metallic Studio",
            "the projects page"
        );

        let mut project = metallic_engine::metallic_project::Project::open(&file).unwrap();
        let mut dm = metallic_engine::metallic_classes::new_game().unwrap();
        project.load_start_scene(&mut dm).unwrap();
        let workspace = dm.service(dm.registry().class_id("Workspace").unwrap()).unwrap();
        assert!(dm.find_first_child(workspace, "Floor", false).unwrap().is_some());
        assert!(dm.find_first_child(workspace, "Baseplate", false).unwrap().is_none());
    }

    #[test]
    fn the_output_shows_the_log_with_a_filter_and_clear() {
        use crate::logging::Origin;
        use log::Level;

        let path = settings_file("editor-output", None);
        let mut harness = past_the_splash(path.clone());
        let log = Arc::new(LogBuffer::new());
        log.push(Level::Info, "metallic_project", "opened the test game", Origin::Engine);
        log.push(
            Level::Warn,
            "metallic_engine::mesh_assets",
            "Meshes/Rock.glb is missing",
            Origin::Engine,
        );
        harness.state_mut().studio.set_log(Arc::clone(&log));
        let project = make_project_once(path.parent().unwrap(), "Output Game");
        harness.state_mut().studio.open(project);
        wait_open(&mut harness);
        harness.get_by_label("opened the test game");
        harness.get_by_label("Meshes/Rock.glb is missing");
        // A script's line, logged while the panel shows.
        log.push(
            Level::Error,
            "Workspace.Door.Script:3",
            "attempt to index nil",
            Origin::Script,
        );
        settle(&mut harness);
        harness.get_by_label("attempt to index nil");

        harness.get_by_role_and_label(Role::CheckBox, "Messages").click();
        settle(&mut harness);
        assert!(harness.query_by_label("opened the test game").is_none());
        harness.get_by_label("attempt to index nil");

        click_label(&mut harness, "Clear");
        assert!(harness.query_by_label("attempt to index nil").is_none());
        // Empty, and no note says so (#286).
        assert!(harness.query_by_label_contains("output is empty").is_none());
        assert!(harness.query_by_label("No lines match the filter.").is_none());
    }

    // Tab bars above their panels (#236), and striped Properties rows (#230).

    /// The panels in the tab bar holding panel `id`, in order.
    fn tab_bar_of(studio: &Studio, id: &str) -> Vec<String> {
        use egui_tiles::{Container, Tile};
        let tiles = &studio.dock().tree().tiles;
        for (_, tile) in tiles.iter() {
            if let Tile::Container(Container::Tabs(tabs)) = tile {
                let panes: Vec<String> = tabs
                    .children
                    .iter()
                    .filter_map(|child| match tiles.get(*child) {
                        Some(Tile::Pane(pane)) => Some(pane.0.clone()),
                        _ => None,
                    })
                    .collect();
                if panes.iter().any(|pane| pane == id) {
                    return panes;
                }
            }
        }
        panic!("{id} is in no tab bar");
    }

    /// Presses the primary button at `from`, drags to `to` over a few frames and lets go there.
    fn drag(harness: &mut Harness<'_, Test>, from: egui::Pos2, to: egui::Pos2) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        harness.event(egui::Event::PointerMoved(from));
        harness.step();
        harness.event(button(from, true));
        harness.step();
        for i in 1..=8 {
            harness.event(egui::Event::PointerMoved(from.lerp(to, i as f32 / 8.0)));
            harness.step();
        }
        harness.event(button(to, false));
        settle(harness);
    }

    #[test]
    fn the_viewport_s_menu_turns_the_axis_gizmo_off_and_other_panels_have_none() {
        let mut harness = editing(settings_file("viewport-menu", None), "Viewport Menu Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        // Three dots at the right of the Viewport's tab bar (#387), on its row.
        let dots = harness.get_by_role_and_label(Role::Button, "Viewport options").rect();
        let tab = harness.get_by_role_and_label(Role::Tab, "Viewport").rect();
        assert!(dots.left() > tab.right() && (dots.center().y - tab.center().y).abs() < 1.0);
        assert!(
            harness
                .query_by_role_and_label(Role::Button, "Explorer options")
                .is_none()
        );
        assert!(harness.state().studio.settings().viewport.show_axes);
        harness.get_by_role_and_label(Role::Button, "Viewport options").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::CheckBox, "Axis gizmo").click();
        settle(&mut harness);
        assert!(!harness.state().studio.settings().viewport.show_axes);
    }

    #[test]
    fn the_tab_bars_sit_above_their_panels_with_no_setting_to_move_them() {
        // A file from #230's Studio, with the tab bars at the bottom: the key is kept, and nothing moves.
        let path = settings_file("editor-tabs", Some("[appearance]\ntab_position = \"bottom\"\n"));
        let mut harness = editing(path.clone(), "Tabs Game");
        let tab = harness.get_by_role_and_label(Role::Tab, "Properties").rect();
        let note = harness
            .get_by_label("Select something in the Explorer to see its properties.")
            .rect();
        let explorer = harness.get_by_role_and_label(Role::Tab, "Explorer").rect();
        let workspace = harness.get_by_role_and_label(Role::TreeItem, "Workspace").rect();
        assert!(tab.bottom() <= note.top(), "{tab:?} over {note:?}");
        assert!(explorer.bottom() <= workspace.top(), "{explorer:?} over {workspace:?}");
        // At the very top of their areas.
        let tiles = &harness.state().studio.dock().tree().tiles;
        let area = tiles
            .iter()
            .find_map(|(&tile, kind)| match kind {
                egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))
                    if tabs.children.iter().any(|child| {
                        matches!(tiles.get(*child), Some(egui_tiles::Tile::Pane(pane)) if pane.0 == panels::ids::PROPERTIES)
                    }) =>
                {
                    tiles.rect(tile)
                }
                _ => None,
            })
            .unwrap();
        assert!((tab.top() - area.top()).abs() < 0.5, "{tab:?} in {area:?}");

        // Settings → Appearance has no Panel tabs row.
        harness.get_by_role_and_label(Role::Button, "Settings").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::Tab, "Appearance").click();
        settle(&mut harness);
        harness.set_size(egui::vec2(1280.0, 1400.0));
        settle(&mut harness);
        harness.get_by_label("Reduced motion");
        assert!(harness.query_by_label("Panel tabs").is_none());
        harness.get_by_role_and_label(Role::Button, "Settings").click();
        settle(&mut harness);

        // A click on a tab opens it, and its close button closes it.
        harness.get_by_role_and_label(Role::Tab, "Assets").click();
        settle(&mut harness);
        harness.get_by_label("1 asset");
        click_label(&mut harness, "Close Assets");
        assert!(!harness.state().studio.dock().is_open(panels::ids::ASSETS));
    }

    #[test]
    fn tabs_drag_to_a_new_place_and_into_another_bar() {
        let mut harness = editing(settings_file("editor-drag", None), "Drag Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        assert_eq!(
            tab_bar_of(&harness.state().studio, panels::ids::OUTPUT),
            [panels::ids::OUTPUT, panels::ids::ASSETS]
        );
        // Assets dragged to the left of Output, in the same bar.
        let output = harness.get_by_role_and_label(Role::Tab, "Output").rect();
        let assets = harness.get_by_role_and_label(Role::Tab, "Assets").rect();
        drag(
            &mut harness,
            assets.center(),
            egui::pos2(output.left() + 4.0, output.center().y),
        );
        assert_eq!(
            tab_bar_of(&harness.state().studio, panels::ids::OUTPUT),
            [panels::ids::ASSETS, panels::ids::OUTPUT]
        );
        // Assets dragged onto the Properties bar joins it, after Properties, and opens.
        let assets = harness.get_by_role_and_label(Role::Tab, "Assets").rect();
        let properties = harness.get_by_role_and_label(Role::Tab, "Properties").rect();
        drag(
            &mut harness,
            assets.center(),
            egui::pos2(properties.right() + 20.0, properties.center().y),
        );
        assert_eq!(
            tab_bar_of(&harness.state().studio, panels::ids::PROPERTIES),
            [panels::ids::PROPERTIES, panels::ids::ASSETS],
            "assets is in {:?}",
            tab_bar_of(&harness.state().studio, panels::ids::ASSETS)
        );
        harness.get_by_label("1 asset");
    }

    // Dock guides (#237).

    /// The area of the tab bar holding panel `id`, as last laid out.
    fn area_of(studio: &Studio, id: &str) -> egui::Rect {
        let tiles = &studio.dock().tree().tiles;
        let pane = tiles.find_pane(&crate::dock::PanelId(id.into())).unwrap();
        tiles.rect(tiles.parent_of(pane).unwrap()).unwrap()
    }

    #[test]
    fn the_viewport_shows_the_corner_drawn_this_frame() {
        use metallic_engine::metallic_render::ViewportTarget;

        let mut harness = editing(settings_file("viewport-resize", None), "Resize Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        let texture = egui::TextureId::User(77);
        // Where the image is drawn and the texture coordinates of its far corner.
        let image = |harness: &Harness<'_, Test>| {
            harness
                .output()
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Mesh(mesh) if mesh.texture_id == texture => {
                        let uv = mesh.vertices.iter().fold(egui::pos2(0.0, 0.0), |far, v| far.max(v.uv));
                        Some((mesh.calc_bounds(), uv))
                    }
                    _ => None,
                })
                .expect("the viewport's image")
        };
        let wanted = harness.state().studio.editor().unwrap().viewport.pixels.unwrap();
        let panel = harness.state().studio.editor().unwrap().viewport.rect.unwrap();
        let show = |harness: &mut Harness<'_, Test>, target: (u32, u32)| {
            let viewport = &mut harness.state_mut().studio.editor_mut().unwrap().viewport;
            viewport.texture = Some(texture);
            viewport.texture_pixels = Some(target);
            viewport.texture_limit = 16_384;
            harness.step();
            image(harness)
        };
        let share = |drawn: u32, of: u32| drawn as f32 / of as f32;
        // A target of the panel's size: all of it, filling the panel.
        let (at, uv) = show(&mut harness, wanted);
        assert_eq!(at, panel);
        assert_eq!(uv, egui::pos2(1.0, 1.0));
        // A larger target (#330): its top-left corner of the panel's size, still filling the panel, never stretched.
        let larger = (wanted.0 + 300, wanted.1 + 200);
        let (at, uv) = show(&mut harness, larger);
        assert_eq!(at, panel);
        assert_eq!(uv, egui::pos2(share(wanted.0, larger.0), share(wanted.1, larger.1)));
        // A target the panel outgrew: the engine grows it this frame, as the panel works out too, and the corner is
        // the panel's size in the grown one.
        let smaller = (wanted.0 - 200, wanted.1 - 100);
        let room = ViewportTarget::room_for(wanted, smaller, 16_384);
        assert!(room.0 >= wanted.0 && room.1 >= wanted.1);
        let (at, uv) = show(&mut harness, smaller);
        assert_eq!(at, panel);
        assert_eq!(uv, egui::pos2(share(wanted.0, room.0), share(wanted.1, room.1)));
    }

    /// The toolbar's tab called `name`.
    fn toolbar_tab(harness: &Harness<'_, Test>, name: &str) -> egui::Rect {
        harness
            .get_all_by_role_and_label(Role::Tab, name)
            .map(|tab| tab.rect())
            .find(|tab| tab.top() < editor::BAR_HEIGHT + toolbar::TAB_ROW)
            .expect("a toolbar tab")
    }

    /// Clicks the toolbar's tab called `name`.
    fn click_tab(harness: &mut Harness<'_, Test>, name: &str) {
        let tab = toolbar_tab(harness, name);
        left_click(harness, tab);
    }

    /// Clicks the middle of `rect`.
    fn left_click(harness: &mut Harness<'_, Test>, rect: egui::Rect) {
        let at = rect.center();
        harness.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        settle(harness);
    }

    /// Opens the menu of `node` with a right-click.
    fn right_click(harness: &mut Harness<'_, Test>, node: egui::Rect) {
        let at = node.center();
        harness.event(egui::Event::PointerMoved(at));
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        settle(harness);
    }

    #[test]
    fn the_tabs_window_shows_hides_and_places_the_tabs() {
        let path = settings_file("toolbar-tabs", None);
        let mut harness = editing(path.clone(), "Tabs Window Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        // A right-click on a tab opens the Tabs window, not a menu (#329).
        let model = toolbar_tab(&harness, "Model");
        right_click(&mut harness, model);
        // A window's first frame only measures it; the next places it.
        settle(&mut harness);
        assert!(harness.state().studio.toolbar().is_editing_tabs());
        for name in ["Home", "Model", "UI", "Scripts", "Plugins"] {
            harness.get_by_role_and_label(Role::CheckBox, &format!("Show {name}"));
            harness.get_by_role_and_label(Role::CheckBox, &format!("Own layout for {name}"));
        }
        // Hiding Model takes it out of the row.
        harness.get_by_role_and_label(Role::CheckBox, "Show Model").click();
        settle(&mut harness);
        let config = |harness: &Harness<'_, Test>| harness.state().studio.toolbar().config().clone();
        assert!(!config(&harness).tabs[1].shown);
        assert!(
            harness
                .query_all_by_role_and_label(Role::Tab, "Model")
                .all(|tab| tab.rect().top() >= editor::BAR_HEIGHT + toolbar::TAB_ROW),
            "no Model tab in the toolbar"
        );
        // Scripts loads a layout of its own; Plugins moves up.
        harness
            .get_by_role_and_label(Role::CheckBox, "Own layout for Scripts")
            .click();
        settle(&mut harness);
        assert!(config(&harness).tabs[3].own_layout);
        harness.get_by_role_and_label(Role::Button, "Move Plugins up").click();
        settle(&mut harness);
        let names: Vec<String> = config(&harness).tabs.iter().map(|tab| tab.name.clone()).collect();
        assert_eq!(names, ["Home", "Model", "UI", "Plugins", "Scripts"]);
        // The tabs move to the right of the row.
        let home = toolbar_tab(&harness, "Home");
        harness.get_by_role(Role::ComboBox).click();
        settle(&mut harness);
        harness.step();
        harness.get_by_role_and_label(Role::Button, "Right").click();
        settle(&mut harness);
        assert_eq!(config(&harness).align, toolbar::TabAlign::Right);
        let moved = toolbar_tab(&harness, "Home");
        assert!(moved.left() > home.left() + 400.0, "{moved:?} from {home:?}");
        // Escape closes it; the toolbar is saved with all of it.
        harness.key_press(Key::Escape);
        settle(&mut harness);
        assert!(!harness.state().studio.toolbar().is_editing_tabs());
        let text = std::fs::read_to_string(path.with_file_name(toolbar::TOOLBAR_FILE)).unwrap();
        assert!(text.contains("\"align\": \"right\""), "{text}");
        assert!(text.contains("\"shown\": false"), "{text}");
    }

    #[test]
    fn a_toolbar_tab_is_renamed_in_place() {
        let mut harness = editing(settings_file("toolbar-rename", None), "Rename Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        let text_at = |harness: &Harness<'_, Test>, text: &str| {
            harness
                .output()
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(shape) if shape.galley.text() == text => {
                        Some(shape.pos + shape.galley.rect.min.to_vec2())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{text} is not drawn"))
        };
        let model = toolbar_tab(&harness, "Model");
        let before = text_at(&harness, "Model");
        // A double-click starts renaming: the name stays where it was, with no frame around it (#326).
        left_click(&mut harness, model);
        // The second click 50 ms after the first, as `double_click` does.
        let time = harness.ctx.input(|input| input.time);
        harness.input_mut().time = Some(time + 0.05);
        left_click(&mut harness, model);
        harness.input_mut().time = None;
        let field = harness
            .get_all_by_role(Role::TextInput)
            .find(|field| field.rect().top() < editor::BAR_HEIGHT + toolbar::TAB_ROW)
            .expect("the tab's name, being edited")
            .rect();
        let during = text_at(&harness, "Model");
        assert!((during - before).length() < 0.5, "{during:?}, was {before:?}");
        assert!(
            field.width() < model.width(),
            "no wider than the tab: {field:?} in {model:?}"
        );
        let outlined = harness.output().shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(shape) => shape.stroke.width > 0.0 && shape.rect.intersects(field),
            _ => false,
        });
        assert!(!outlined, "no outline around the name");
        // Typing goes on after the name; Enter keeps it.
        harness.event(egui::Event::Text("s".to_owned()));
        settle(&mut harness);
        harness.key_press(Key::Enter);
        settle(&mut harness);
        assert_eq!(harness.state().studio.toolbar().config().tabs[1].name, "Models");
        toolbar_tab(&harness, "Models");
    }

    #[test]
    fn the_toolbar_has_tabs_of_tools_that_can_be_changed() {
        let path = settings_file("toolbar", None);
        let mut harness = editing(path.clone(), "Toolbar Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        // The default tabs, Home in front with Studio's tools (#232).
        for name in ["Home", "Model", "UI", "Scripts", "Plugins"] {
            toolbar_tab(&harness, name);
        }
        assert_eq!(harness.state().studio.toolbar().config().active, 0);
        // A tool runs its command: the Explorer tool closes the Explorer.
        assert!(harness.state().studio.dock().is_open(panels::ids::EXPLORER));
        harness.get_by_role_and_label(Role::Button, "Explorer tool").click();
        settle(&mut harness);
        assert!(!harness.state().studio.dock().is_open(panels::ids::EXPLORER));
        harness.get_by_role_and_label(Role::Button, "Explorer tool").click();
        settle(&mut harness);
        // Home starts with the transform tools (#374), and 1..4 pick them.
        harness.get_by_role_and_label(Role::Button, "Rotate tool");
        harness.key_press(Key::Num3);
        settle(&mut harness);
        assert_eq!(
            harness.state().studio.editor().unwrap().viewport.gizmo.tool,
            crate::viewport::tools::Tool::Scale
        );

        // Model holds the transform tools (#355): the one in use, and Snap while it is on (#359), are shown on.
        click_tab(&mut harness, "Model");
        harness.get_by_role_and_label(Role::Button, "Move tool").click();
        settle(&mut harness);
        assert_eq!(
            harness.state().studio.editor().unwrap().viewport.gizmo.tool,
            crate::viewport::tools::Tool::Move
        );
        // Snap is the increments (#375): a switch, a field and arrows for each.
        assert!(harness.state().studio.settings().editing.snap_moves);
        harness.get_by_role_and_label(Role::CheckBox, "Snap turns").click();
        harness.get_by_role_and_label(Role::Button, "Move increment up").click();
        settle(&mut harness);
        let editing = harness.state().studio.settings().editing;
        assert!(editing.snap_moves && !editing.snap_turns);
        assert_eq!(editing.move_step, 2.0);
        harness
            .get_by_role_and_label(Role::Button, "Rotate increment up")
            .click();
        settle(&mut harness);
        assert_eq!(
            harness.state().studio.settings().editing.turn_step,
            15.0,
            "off: the arrows do nothing"
        );
        harness.get_by_role_and_label(Role::SpinButton, "Move increment");
        // Scripts: an empty toolbar, with an Add tool button (#328) that opens the Add window.
        click_tab(&mut harness, "Scripts");
        assert_eq!(harness.state().studio.toolbar().config().active, 3);
        assert!(harness.state().studio.toolbar().config().tabs[3].elements.is_empty());
        harness.get_by_role_and_label(Role::Button, "Add tool").click();
        settle(&mut harness);
        assert!(harness.state().studio.toolbar().is_adding());
        harness.key_press(Key::Escape);
        settle(&mut harness);
        assert!(!harness.state().studio.toolbar().is_adding());
        // Right-click the row: Add...
        let row = harness.get_by_role_and_label(Role::Toolbar, "Toolbar").rect();
        right_click(&mut harness, row);
        harness.get_by_label("Add...").click();
        settle(&mut harness);
        // A window's first frame only measures it.
        harness.step();
        // The Add window (#327): every element in its group, with what it does. Several in a row go in order.
        assert!(harness.state().studio.toolbar().is_adding());
        for heading in ["PROJECT", "GAME", "STUDIO", "PANELS", "LAYOUT"] {
            harness.get_by_label(heading);
        }
        let painted = |harness: &Harness<'_, Test>, text: &str| {
            harness.output().shapes.iter().any(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) => shape.galley.text() == text,
                _ => false,
            })
        };
        assert!(painted(&harness, "Runs the game."), "each tool with what it does");
        harness.get_by_role_and_label(Role::Button, "Add Save").click();
        settle(&mut harness);
        // Below the fold: clicked as a screen reader would.
        harness
            .get_by_role_and_label(Role::Button, "Add Separator")
            .click_accesskit();
        settle(&mut harness);
        let model = |harness: &Harness<'_, Test>| harness.state().studio.toolbar().config().tabs[3].elements.clone();
        assert_eq!(
            model(&harness),
            [
                toolbar::Element::tool(toolbar::tools::ids::SAVE),
                toolbar::Element::Separator
            ]
        );
        harness
            .get_by_role_and_label(Role::Button, &format!("Close {}", toolbar::add::TITLE))
            .click();
        settle(&mut harness);
        assert!(!harness.state().studio.toolbar().is_adding());
        // Right-click the separator: Remove.
        let separator = harness.get_by_label("Separator").rect();
        right_click(&mut harness, separator);
        harness.get_by_label("Remove").click();
        settle(&mut harness);
        assert_eq!(model(&harness), [toolbar::Element::tool(toolbar::tools::ids::SAVE)]);
        // Right-click the new tool: Remove.
        let save = harness.get_by_role_and_label(Role::Button, "Save tool").rect();
        right_click(&mut harness, save);
        harness.get_by_label("Remove").click();
        settle(&mut harness);
        assert!(model(&harness).is_empty());

        // A plugin's tool, registered while Studio runs, is offered under Plugins.
        let paint = toolbar::Tool::plugin("terrain", "paint", "Paint terrain", "Paints terrain.", None);
        harness
            .state_mut()
            .studio
            .toolbar_mut()
            .registry_mut()
            .register(paint)
            .unwrap();
        right_click(&mut harness, row);
        harness.get_by_label("Add...").click();
        settle(&mut harness);
        // A window's first frame only measures it.
        harness.step();
        harness.get_by_label("TERRAIN (PLUGIN)");
        harness
            .get_by_role_and_label(Role::Button, "Add Paint terrain")
            .click_accesskit();
        settle(&mut harness);
        // Escape closes the window.
        harness.key_press(Key::Escape);
        settle(&mut harness);
        assert!(!harness.state().studio.toolbar().is_adding());
        assert_eq!(model(&harness), [toolbar::Element::tool("plugin.terrain.paint")]);
        // Disabled until the plugin API runs it.
        let tool = harness.get_by_role_and_label(Role::Button, "Paint terrain tool");
        assert!(egui_kittest::kittest::NodeT::accesskit_node(&tool).is_disabled());

        // Rename a tab and delete another in the Tabs window (#329).
        let plugins = toolbar_tab(&harness, "Plugins");
        right_click(&mut harness, plugins);
        settle(&mut harness);
        let window = harness.get_by_role_and_label(Role::CheckBox, "Show Scripts").rect();
        let field = harness
            .get_all_by_role(Role::TextInput)
            .find(|field| (field.rect().center().y - window.center().y).abs() < 12.0)
            .expect("Scripts' name in the Tabs window")
            .rect();
        left_click(&mut harness, field);
        harness.key_press(Key::End);
        harness.event(egui::Event::Text("Code".to_owned()));
        settle(&mut harness);
        let names = |harness: &Harness<'_, Test>| -> Vec<String> {
            let config = harness.state().studio.toolbar().config();
            config.tabs.iter().map(|tab| tab.name.clone()).collect()
        };
        assert!(
            names(&harness).contains(&"ScriptsCode".to_owned()),
            "{:?}",
            names(&harness)
        );
        harness.get_by_role_and_label(Role::Button, "Delete Plugins").click();
        settle(&mut harness);
        harness.key_press(Key::Escape);
        settle(&mut harness);
        assert!(!names(&harness).contains(&"Plugins".to_owned()));
        // A new tab from the +.
        harness.get_by_role_and_label(Role::Button, "New tab").click();
        settle(&mut harness);
        harness.key_press(Key::Enter);
        settle(&mut harness);
        assert!(names(&harness).contains(&"New tab".to_owned()));

        // UI loads a layout of its own: closing Output there leaves the main layout as it was.
        click_tab(&mut harness, "UI");
        assert!(
            harness.state().studio.dock().is_open(panels::ids::OUTPUT),
            "starts as the main"
        );
        harness.state_mut().studio.dock_mut().close(panels::ids::OUTPUT);
        settle(&mut harness);
        click_tab(&mut harness, "Home");
        assert!(
            harness.state().studio.dock().is_open(panels::ids::OUTPUT),
            "the main layout is back"
        );
        click_tab(&mut harness, "UI");
        assert!(
            !harness.state().studio.dock().is_open(panels::ids::OUTPUT),
            "UI's own layout"
        );
        let layout = std::fs::read_to_string(path.with_file_name(crate::dock::LAYOUT_FILE)).unwrap_or_default();
        assert!(
            layout.is_empty() || layout.contains("\"output\""),
            "layout.json keeps the main layout"
        );
        click_tab(&mut harness, "Home");

        // Saved, and read back by the next Studio.
        let saved = harness.state().studio.toolbar().config().clone();
        let text = std::fs::read_to_string(path.with_file_name(toolbar::TOOLBAR_FILE)).unwrap();
        assert!(text.contains("\"ScriptsCode\""), "{text}");
        let again = past_the_splash(path);
        let mut read = again.state().studio.toolbar().config().clone();
        read.active = saved.active;
        assert_eq!(read, saved);
    }

    #[test]
    fn the_top_bar_has_no_mark() {
        let mut harness = editing(settings_file("bar-no-mark", None), "Bar Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        // The window's title bar shows the mark (#299): the bar starts with the File menu, and nothing is drawn before it.
        let file = harness.get_by_label("File").rect();
        let images_before = harness.output().shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Mesh(mesh) => {
                let bounds = mesh.calc_bounds();
                bounds.bottom() <= file.bottom() + 8.0 && bounds.right() <= file.left()
            }
            _ => false,
        });
        assert!(!images_before, "an image left of the File menu");
    }

    #[test]
    fn docked_panels_sit_flush_with_each_other_and_the_window() {
        let mut harness = editing(settings_file("flush", None), "Flush Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        let studio = &harness.state().studio;
        let viewport = area_of(studio, panels::ids::VIEWPORT);
        let output = area_of(studio, panels::ids::OUTPUT);
        let explorer = area_of(studio, panels::ids::EXPLORER);
        let properties = area_of(studio, panels::ids::PROPERTIES);
        // No gap between groups (#273).
        assert!(
            (viewport.bottom() - output.top()).abs() < 0.01,
            "{viewport:?} {output:?}"
        );
        assert!(
            (explorer.bottom() - properties.top()).abs() < 0.01,
            "{explorer:?} {properties:?}"
        );
        assert!(
            (viewport.right() - explorer.left()).abs() < 0.01,
            "{viewport:?} {explorer:?}"
        );
        // None around the dock: it meets the frame's left, right and bottom, and the toolbar. The harness frames the
        // UI with a margin of its own (Studio's window has none): the top bar's top-left corner is the frame's.
        let margin = viewport.top() - editor::BAR_HEIGHT - toolbar::TOOLBAR_HEIGHT;
        assert!(
            (viewport.left() - margin).abs() < 0.01 && (output.left() - margin).abs() < 0.01,
            "{viewport:?}"
        );
        assert!((explorer.right() - (1280.0 - margin)).abs() < 0.01, "{explorer:?}");
        assert!((output.bottom() - (800.0 - margin)).abs() < 0.01, "{output:?}");
        assert!((properties.bottom() - (800.0 - margin)).abs() < 0.01, "{properties:?}");
    }

    /// Where the guide for `side` is drawn over the area of the tab bar holding `id`.
    fn guide(studio: &Studio, id: &str, side: crate::dock::guides::Side) -> egui::Pos2 {
        side.square(area_of(studio, id)).center()
    }

    #[test]
    fn a_dragged_tab_s_chip_shows_its_name_alone() {
        use crate::dock::guides::Side;
        let mut harness = editing(settings_file("drag-chip", None), "Drag Chip Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        // Mid-drag, over the viewport's guides: the chip, and no panel icon anywhere (#303).
        let from = harness.get_by_role_and_label(Role::Tab, "Output").rect().center();
        let to = guide(&harness.state().studio, panels::ids::VIEWPORT, Side::Right);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        harness.event(egui::Event::PointerMoved(from));
        harness.step();
        harness.event(button(from, true));
        harness.step();
        for i in 1..=8 {
            harness.event(egui::Event::PointerMoved(from.lerp(to, i as f32 / 8.0)));
            harness.step();
        }
        assert!(
            harness.state().studio.dock().is_dragging(&harness.ctx),
            "still dragging"
        );
        let named = harness.output().shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => text.galley.text() == "Output",
            _ => false,
        });
        assert!(named, "the chip names the panel");
        // Below the toolbar, whose Home tab has the panels' tools (#232).
        let dock = egui::Rect::from_min_max(
            egui::pos2(0.0, editor::BAR_HEIGHT + toolbar::TOOLBAR_HEIGHT + 8.0),
            egui::pos2(1280.0, 800.0),
        );
        for panel in [Icon::PanelOutput, Icon::PanelViewport, Icon::PanelExplorer] {
            assert!(icon_meshes_in(&harness, panel, dock).is_empty(), "{panel:?} drawn");
        }
        harness.event(button(to, false));
        settle(&mut harness);
    }

    #[test]
    fn dropping_on_a_dock_guide_docks_the_panel_there() {
        use crate::dock::guides::Side;
        let mut harness = editing(settings_file("editor-guides", None), "Guides Game");
        // Output on the right square of the viewport's area: beside the viewport, in a bar of its own.
        let output = harness.get_by_role_and_label(Role::Tab, "Output").rect();
        let to = guide(&harness.state().studio, panels::ids::VIEWPORT, Side::Right);
        drag(&mut harness, output.center(), to);
        let studio = &harness.state().studio;
        assert_eq!(tab_bar_of(studio, panels::ids::OUTPUT), [panels::ids::OUTPUT]);
        let (viewport, output) = (
            area_of(studio, panels::ids::VIEWPORT),
            area_of(studio, panels::ids::OUTPUT),
        );
        assert!(output.left() >= viewport.right(), "{output:?} right of {viewport:?}");
        assert!((output.top() - viewport.top()).abs() < 0.5, "side by side");

        // Assets on the centre square of the Properties area: a tab there, opened.
        let assets = harness.get_by_role_and_label(Role::Tab, "Assets").rect();
        let to = guide(&harness.state().studio, panels::ids::PROPERTIES, Side::Centre);
        drag(&mut harness, assets.center(), to);
        assert_eq!(
            tab_bar_of(&harness.state().studio, panels::ids::PROPERTIES),
            [panels::ids::PROPERTIES, panels::ids::ASSETS],
            "assets is in {:?}",
            tab_bar_of(&harness.state().studio, panels::ids::ASSETS)
        );
        harness.get_by_label("1 asset");

        // Explorer on the bottom square of its own area is nothing to split: it is that area's only tab now.
        let before = harness.state().studio.dock().tree().clone();
        let explorer = harness.get_by_role_and_label(Role::Tab, "Explorer").rect();
        let to = guide(&harness.state().studio, panels::ids::EXPLORER, Side::Bottom);
        drag(&mut harness, explorer.center(), to);
        assert_eq!(harness.state().studio.dock().tree(), &before);
    }

    #[test]
    fn a_drop_away_from_the_guides_and_tab_bars_changes_nothing() {
        let mut harness = editing(settings_file("editor-guides-miss", None), "Miss Game");
        let before = harness.state().studio.dock().tree().clone();
        // Into the viewport's area, well away from its cross and its tab bar.
        let output = harness.get_by_role_and_label(Role::Tab, "Output").rect();
        let viewport = area_of(&harness.state().studio, panels::ids::VIEWPORT);
        drag(
            &mut harness,
            output.center(),
            viewport.left_bottom() + egui::vec2(30.0, -30.0),
        );
        assert_eq!(harness.state().studio.dock().tree(), &before);
    }

    #[test]
    fn the_guides_show_while_a_tab_is_dragged_over_an_area() {
        use crate::dock::guides::{SQUARE, Side};
        let mut harness = editing(settings_file("editor-guides-show", None), "Show Game");
        let output = harness.get_by_role_and_label(Role::Tab, "Output").rect();
        let to = guide(&harness.state().studio, panels::ids::VIEWPORT, Side::Left);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        harness.event(egui::Event::PointerMoved(output.center()));
        harness.step();
        harness.event(button(output.center(), true));
        harness.step();
        for i in 1..=8 {
            harness.event(egui::Event::PointerMoved(output.center().lerp(to, i as f32 / 8.0)));
            harness.step();
        }
        // Five squares in the foreground, one lit with the accent.
        let accent = current_colors(&harness.ctx).accent;
        let squares: Vec<egui::epaint::RectShape> = harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if (rect.rect.width() - SQUARE).abs() < 0.01 => Some(rect.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(squares.len(), 5, "{squares:?}");
        let lit: Vec<_> = squares.iter().filter(|square| square.stroke.color == accent).collect();
        assert_eq!(lit.len(), 1);
        assert!(lit[0].rect.contains(to));
        harness.event(button(to, false));
        settle(&mut harness);
        assert!(
            area_of(&harness.state().studio, panels::ids::OUTPUT).right()
                <= area_of(&harness.state().studio, panels::ids::VIEWPORT).left()
        );
    }

    #[test]
    fn properties_rows_are_striped_with_alternating_shades() {
        let mut harness = editing(settings_file("editor-stripes", None), "Stripes Game");
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        harness.get_by_label("TRANSFORM");
        let shades = tokens::DARK.rows();
        // The rows' fills, top to bottom.
        let mut rows: Vec<(egui::Rect, egui::Color32)> = harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect)
                    if (rect.fill == shades.darker || rect.fill == shades.brighter)
                        && (rect.rect.height() - size::ROW).abs() < 0.01 =>
                {
                    // Shown, not just shaped: the clip spans it too.
                    Some((rect.rect.intersect(clipped.clip_rect), rect.fill))
                }
                _ => None,
            })
            .collect();
        rows.sort_by(|a, b| a.0.top().total_cmp(&b.0.top()));
        assert!(rows.len() >= 8, "{} rows", rows.len());
        // Within a group the rows touch, keep one height, and alternate; each group starts on the darker shade.
        let mut runs = 0;
        for pair in rows.windows(2) {
            let ((above, upper), (below, lower)) = (pair[0], pair[1]);
            if (below.top() - above.bottom()).abs() < 0.01 {
                assert_ne!(upper, lower, "two touching rows of one shade at {}", below.top());
            } else {
                assert!(below.top() > above.bottom());
                assert_eq!(lower, shades.darker, "a group starts on the darker shade");
                runs += 1;
            }
        }
        assert!(runs >= 1, "more than one group");
        // A row of fields keeps the row's height (#349).
        let size = harness.get_by_role_and_label(Role::TextInput, "Size").rect();
        assert!(size.height() <= crate::theme::tokens::size::ROW, "{size:?}");
    }

    #[test]
    fn the_output_toolbar_is_one_line() {
        let mut harness = editing(settings_file("output-toolbar", None), "Output Toolbar Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        let centre = |node: egui_kittest::Node<'_>| node.rect().center().y;
        let errors = centre(harness.get_by_role_and_label(Role::CheckBox, "Errors"));
        let debug = centre(harness.get_by_role_and_label(Role::CheckBox, "Debug"));
        let clear = centre(harness.get_by_role_and_label(Role::Button, "Clear"));
        let search = harness
            .get_all_by_role(Role::TextInput)
            // The bottom left: the Output's, not the Explorer's.
            .find(|field| field.rect().left() < 1000.0 && field.rect().top() > 400.0)
            .map(centre)
            .expect("the output's search box");
        // On one level (#275), within a point (layout rounds to pixels).
        for (name, y) in [("Debug", debug), ("Clear", clear), ("search", search)] {
            assert!((y - errors).abs() < 1.0, "{name} at {y}, the toggles at {errors}");
        }
    }

    #[test]
    fn a_selected_explorer_row_is_plain() {
        let mut harness = editing(settings_file("explorer-plain", None), "Plain Selection Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        harness.get_by_role_and_label(Role::TreeItem, "Baseplate").click();
        settle(&mut harness);
        let row = harness.get_by_role_and_label(Role::TreeItem, "Baseplate").rect();
        let colors = current_colors(&harness.ctx);
        let output = harness.output();
        let in_row: Vec<&egui::epaint::RectShape> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                // The row's height; its fills reach past it to the panel's sides (#301).
                egui::Shape::Rect(rect)
                    if rect.rect.top() >= row.top() - 0.5 && rect.rect.bottom() <= row.bottom() + 0.5 =>
                {
                    Some(rect)
                }
                _ => None,
            })
            .collect();
        // No accent bar (#300).
        assert!(
            in_row.iter().all(|rect| rect.fill != colors.accent),
            "an accent mark on the row"
        );
        // A square raised fill over the whole row.
        let fill = in_row
            .iter()
            .find(|rect| rect.fill == colors.palette.surface_hover)
            .expect("the selected fill");
        assert_eq!(fill.corner_radius, egui::CornerRadius::ZERO);
        // The icon keeps its group's colour.
        let part = Icon::ExplorerPart.color(&colors);
        let tints = icon_meshes(&harness, Icon::ExplorerPart);
        assert!(tints.iter().all(|&tint| tint == part), "{tints:?} vs {part:?}");
        assert_ne!(part, colors.accent);
    }

    #[test]
    fn explorer_rows_touch_and_alternate_shades() {
        let mut harness = editing(settings_file("explorer-stripes", None), "Explorer Stripes Game");
        harness.set_size(egui::vec2(1280.0, 800.0));
        settle(&mut harness);
        // The tree's rows, top to bottom: no gap between them (#274).
        let mut items: Vec<egui::Rect> = harness
            .get_all_by_role(Role::TreeItem)
            .map(|item| item.rect())
            .collect();
        items.sort_by(|a, b| a.top().total_cmp(&b.top()));
        assert!(items.len() >= 6, "{} rows", items.len());
        for pair in items.windows(2) {
            assert!((pair[1].top() - pair[0].bottom()).abs() < 0.01, "{pair:?}");
        }
        // Each row is on a stripe, and neighbours have the two shades.
        let shades = tokens::DARK.rows();
        let output = harness.output();
        // A stripe spans the panel from side to side (#301), across the row's height.
        let pane = area_of(&harness.state().studio, panels::ids::EXPLORER);
        let stripe = |row: egui::Rect| {
            output.shapes.iter().find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect)
                    if rect.rect.y_range() == row.y_range()
                        && (rect.fill == shades.darker || rect.fill == shades.brighter) =>
                {
                    Some((rect.rect, rect.fill))
                }
                _ => None,
            })
        };
        let stripes: Vec<(egui::Rect, egui::Color32)> =
            items.iter().map(|&row| stripe(row).expect("a striped row")).collect();
        for (band, _) in &stripes {
            assert!((band.left() - pane.left()).abs() < 0.5, "{band:?} in {pane:?}");
            assert!((band.right() - pane.right()).abs() < 0.5, "{band:?} in {pane:?}");
        }
        let fills: Vec<egui::Color32> = stripes.iter().map(|(_, fill)| *fill).collect();
        assert_eq!(fills[0], shades.darker, "the first row is the darker shade");
        for pair in fills.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
    }

    /// Runs `studio` for `frames` frames of `size` points at `pixels_per_point`, drawn by egui-wgpu on the engine's
    /// offscreen device as the shell draws them (the viewport too), calling `before(studio, frame)` before each.
    /// Returns the last frame as RGBA rows.
    fn render_offscreen(
        studio: &mut Studio,
        size: egui::Vec2,
        pixels_per_point: f32,
        frames: usize,
        mut before: impl FnMut(&mut Studio, usize),
    ) -> offscreen::Pixels {
        let mut offscreen = Offscreen::new(size, pixels_per_point).expect("an offscreen device");
        for frame in 0..frames {
            before(studio, frame);
            offscreen.frame(studio);
        }
        offscreen.read()
    }

    /// Writes RGBA rows to `path` as a PNG.
    fn write_png(path: &Path, (width, height, pixels): (u32, u32, Vec<u8>)) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::High);
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
    }

    /// Studio over `steps`, the steps done, so its second frame is past the splash.
    fn studio_after_startup(steps: Vec<Step>) -> Studio {
        let mut studio = Studio::new(Startup::spawn(steps).unwrap(), None, false);
        wait(&mut studio.startup);
        studio
    }

    /// The screenshots in `docs/screenshots` (`cargo test -p metallic_studio -- --ignored screenshots`). Offscreen:
    /// no window, no real input, and the scripted file dialog.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshots_of_the_sign_in_and_projects_screens() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let size = egui::vec2(1280.0, 800.0);

        let path = config("shot-sign-in", None, None);
        let mut studio = studio_after_startup(steps_reading(path));
        let shot = render_offscreen(&mut studio, size, 1.25, 40, |_, _| {});
        assert_eq!(studio.screen(), Screen::SignIn);
        write_png(&out.join("215-sign-in.png"), shot);

        let dir = std::env::temp_dir().join(format!("metallic-studio-shot-projects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let samples = [
            ("Space Race", 2 * 3600, true),
            ("Obby Tower", 26 * 3600, true),
            ("Kart Prototype", 4 * 86_400, true),
            ("Old Jam Game", 70 * 86_400, false),
            ("Lighting Tests", 200 * 86_400, true),
        ];
        let mut list = String::new();
        for (name, ago, exists) in samples {
            let file = if exists {
                make_project_once(&dir, name)
            } else {
                dir.join(format!("{name}.mtproj"))
            };
            list.push_str(&format!(
                "[[project]]\npath = '{}'\nname = \"{name}\"\nlast_opened = {}\n",
                file.display(),
                now - ago
            ));
        }
        let path = config("shot-projects", Some("[general]\nskip_sign_in = true\n"), Some(&list));
        let mut studio = studio_after_startup(steps_reading(path));
        let selected = dir.join("Obby Tower.mtproj");
        let shot = render_offscreen(&mut studio, size, 1.25, 40, |studio, frame| {
            if frame == 2 {
                studio.projects_mut().select(selected.clone());
            }
        });
        assert_eq!(studio.screen(), Screen::Projects);
        write_png(&out.join("215-projects.png"), shot);

        let shot = render_offscreen(&mut studio, size, 1.25, 40, |studio, frame| {
            if frame == 2 {
                let projects = studio.projects_mut();
                projects.new_project(None);
                projects.set_new_project("Space Race 2", Path::new("C:/Users/me/Documents/Metallic"));
            }
        });
        assert!(studio.projects().is_creating());
        write_png(&out.join("215-new-project.png"), shot);
    }

    /// The editor's screenshot (`cargo test -p metallic_studio -- --ignored screenshot_of_the_editor`): the template
    /// project open, a part selected. Offscreen, like the others.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_the_editor() {
        use crate::logging::Origin;
        use log::Level;

        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-editor", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let log = Arc::new(LogBuffer::new());
        for (level, source, message) in [
            (
                Level::Info,
                "metallic_studio::app",
                "projects: opening Space Race.mtproj",
            ),
            (
                Level::Info,
                "metallic_render",
                "render: NVIDIA GeForce RTX 3060 (Vulkan)",
            ),
            (
                Level::Info,
                "metallic_studio::editor",
                "projects: opened Space Race.mtproj in 41ms",
            ),
            (
                Level::Warn,
                "metallic_project::scene",
                "scene: Scenes/Main.scene: unknown property `Glow` on Part",
            ),
        ] {
            log.push(level, source, message, Origin::Engine);
        }
        log.push(
            Level::Info,
            "Workspace.Spawner.Script:4",
            "Round 1 starting",
            Origin::Script,
        );
        studio.set_log(log);
        let shot = render_offscreen(&mut studio, egui::vec2(1280.0, 800.0), 1.25, 40, |studio, frame| {
            if frame == 2 {
                studio.open(project.clone());
                for _ in 0..10_000 {
                    if studio.poll_opening().is_none() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                let editor = studio.editor_mut().expect("the project opened");
                let workspace = editor
                    .dm
                    .service(editor.dm.registry().class_id("Workspace").unwrap())
                    .unwrap();
                let part = editor.dm.find_first_child(workspace, "SpawnLocation", false).unwrap();
                editor.select(part);
                // The Move tool's gizmo on it (#355).
                editor.viewport.gizmo.tool = Tool::Move;
            }
        });
        assert_eq!(studio.screen(), Screen::Editor);
        write_png(&out.join("216-editor.png"), shot);
    }

    /// The toolbar's windows (`cargo test -p metallic_studio -- --ignored screenshot_of_the_toolbar_windows`): the Add
    /// window (#327) and the Tabs window (#329) over the editor. Offscreen, like the others.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_the_toolbar_windows() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config(
            "shot-toolbar",
            Some(
                "[general]
skip_sign_in = true
",
            ),
            None,
        );
        let project = make_project_once(path.parent().unwrap(), "Toolbar Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let open = |studio: &mut Studio| {
            studio.open(project.clone());
            for _ in 0..10_000 {
                if studio.poll_opening().is_none() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let shot = render_offscreen(&mut studio, egui::vec2(1280.0, 800.0), 1.25, 40, |studio, frame| {
            if frame == 2 {
                open(studio);
                studio.toolbar_mut().select_tab(1);
            }
            if frame == 6 {
                studio.toolbar_mut().open_add(0);
            }
        });
        write_png(&out.join("327-add-window.png"), shot);
        let shot = render_offscreen(&mut studio, egui::vec2(1280.0, 800.0), 1.25, 40, |studio, frame| {
            if frame == 2 {
                studio.toolbar_mut().close_add();
                studio.toolbar_mut().select_tab(0);
                studio.toolbar_mut().config_mut().tabs[3].shown = false;
                studio.toolbar_mut().config_mut().align = crate::toolbar::TabAlign::Center;
                studio.toolbar_mut().set_editing_tabs(true);
            }
        });
        write_png(&out.join("329-tabs-window.png"), shot);
    }

    /// The screenshot of #230 (`cargo test -p metallic_studio -- --ignored screenshot_of_striped_properties`): the
    /// template project open with a part selected, and its Properties panel's striped rows close up. Offscreen, like
    /// the others.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_striped_properties() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-properties", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let pixels_per_point = 1.25;
        let shot = render_offscreen(
            &mut studio,
            egui::vec2(1280.0, 800.0),
            pixels_per_point,
            40,
            |studio, frame| {
                if frame == 2 {
                    studio.open(project.clone());
                    for _ in 0..10_000 {
                        if studio.poll_opening().is_none() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let editor = studio.editor_mut().expect("the project opened");
                    let workspace = editor
                        .dm
                        .service(editor.dm.registry().class_id("Workspace").unwrap())
                        .unwrap();
                    let part = editor.dm.find_first_child(workspace, "Baseplate", false).unwrap();
                    editor.select(part);
                }
            },
        );
        assert_eq!(studio.screen(), Screen::Editor);
        // The Properties panel's area, from the dock's last layout.
        let tiles = &studio.dock().tree().tiles;
        let area = tiles
            .iter()
            .find_map(|(&tile, kind)| match kind {
                egui_tiles::Tile::Pane(pane) if pane.0 == panels::ids::PROPERTIES => tiles.rect(tile),
                _ => None,
            })
            .expect("the Properties panel is laid out");
        let (width, _, pixels) = &shot;
        let to_px = |v: f32| (v * pixels_per_point).round() as u32;
        let (left, top, right, bottom) = (
            to_px(area.left()),
            to_px(area.top()),
            to_px(area.right()),
            to_px(area.bottom()),
        );
        let mut crop = Vec::with_capacity(((right - left) * (bottom - top) * 4) as usize);
        for y in top..bottom {
            let row = ((y * width + left) * 4) as usize;
            crop.extend_from_slice(&pixels[row..row + ((right - left) * 4) as usize]);
        }
        write_png(&out.join("230-properties.png"), (right - left, bottom - top, crop));
    }

    /// The screenshot of #237 (`cargo test -p metallic_studio -- --ignored screenshot_of_the_dock_guides`): the Output
    /// tab dragged over the viewport, its left guide lit. Offscreen, like the others.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_the_dock_guides() {
        use crate::dock::guides::Side;
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-guides", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let mut offscreen = Offscreen::new(egui::vec2(1280.0, 800.0), 1.25).expect("an offscreen device");
        for _ in 0..3 {
            offscreen.frame(&mut studio);
        }
        studio.open(project);
        for _ in 0..10_000 {
            if studio.poll_opening().is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        for _ in 0..20 {
            offscreen.frame(&mut studio);
        }
        assert_eq!(studio.screen(), Screen::Editor);
        let from = area_of(&studio, panels::ids::OUTPUT).left_top() + egui::vec2(40.0, 16.0);
        let to = guide(&studio, panels::ids::VIEWPORT, Side::Left);
        let button = |pressed| egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        offscreen.frame_with(&mut studio, vec![egui::Event::PointerMoved(from)]);
        offscreen.frame_with(&mut studio, vec![button(true)]);
        for i in 1..=12 {
            let at = from.lerp(to, i as f32 / 12.0);
            offscreen.frame_with(&mut studio, vec![egui::Event::PointerMoved(at)]);
        }
        for _ in 0..4 {
            offscreen.frame(&mut studio);
        }
        assert!(studio.dock().is_dragging(&offscreen.ctx), "still dragging");
        write_png(&out.join("237-dock-guides.png"), offscreen.read());
    }

    /// The screenshot of #238 (`cargo test -p metallic_studio -- --ignored screenshot_of_the_settings_window`): the
    /// editor with the Settings window floating over it, on the Appearance page.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_the_settings_window() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-settings-window", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let shot = render_offscreen(&mut studio, egui::vec2(1280.0, 800.0), 1.25, 40, |studio, frame| {
            if frame == 2 {
                studio.open(project.clone());
                for _ in 0..10_000 {
                    if studio.poll_opening().is_none() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                studio.settings_window().show_category(Category::Appearance);
                studio.settings_window().set_open(true);
            }
        });
        assert_eq!(studio.screen(), Screen::Editor);
        assert!(studio.is_settings_open());
        write_png(&out.join("238-settings-window.png"), shot);
    }

    /// The screenshot of #264 (`cargo test -p metallic_studio -- --ignored screenshot_of_keybind_sections`): the
    /// Keybinds page filtered to "fly", Viewport's two sections inside it, one command changed so its sections show
    /// Reset. Offscreen, like the others.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_keybind_sections() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-keybind-sections", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let shot = render_offscreen(&mut studio, egui::vec2(1280.0, 800.0), 1.25, 40, |studio, frame| {
            if frame == 2 {
                studio.open(project.clone());
                for _ in 0..10_000 {
                    if studio.poll_opening().is_none() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                studio.keymap_mut().remove_binding(ids::FLY_UP, 0);
                studio.settings_window().show_category(Category::Keybinds);
                studio.settings_window().keybinds().set_filter("fly");
                studio.settings_window().set_open(true);
            }
        });
        assert!(studio.is_settings_open());
        write_png(&out.join("264-keybind-sections.png"), shot);
    }

    /// The screenshot of #266 (`cargo test -p metallic_studio -- --ignored screenshot_of_recording_a_keybind`): the
    /// Keybinds page with Save's shortcut being recorded in place. Offscreen, like the others.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_recording_a_keybind() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-keybind-recording", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let shot = render_offscreen(&mut studio, egui::vec2(1280.0, 800.0), 1.25, 40, |studio, frame| {
            if frame == 2 {
                studio.open(project.clone());
                for _ in 0..10_000 {
                    if studio.poll_opening().is_none() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                studio.settings_window().show_category(Category::Keybinds);
                studio.settings_window().keybinds().set_filter("project");
                studio.settings_window().set_open(true);
            }
            if frame == 4 {
                studio.settings_window().keybinds().capture(ids::FILE_SAVE, Some(0));
            }
        });
        assert!(studio.settings_window().is_capturing());
        write_png(&out.join("266-keybind-recording.png"), shot);
    }

    /// The screenshot of #231 (`cargo test -p metallic_studio -- --ignored screenshot_of_custom_icons`): the editor
    /// with a custom icon folder: a monochrome part icon drawn in its group's colour, a full-colour workspace icon as
    /// it is. Offscreen, like the others.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_custom_icons() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-custom-icons", Some("[general]\nskip_sign_in = true\n"), None);
        let icons = path.parent().unwrap().join("icons");
        std::fs::create_dir_all(&icons).unwrap();
        std::fs::write(
            icons.join("explorer.part.svg"),
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 2 22 7v10l-10 5L2 17V7z" fill="currentColor"/></svg>"#,
        )
        .unwrap();
        std::fs::write(
            icons.join("explorer.workspace.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" fill="#3b82f6"/><path d="M4 9c4 2 8-2 12 0s4 3 4 3v3c-3-2-6 1-10-1S4 13 4 13z" fill="#22c55e"/></svg>"##,
        )
        .unwrap();
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let shot = render_offscreen(&mut studio, egui::vec2(1280.0, 800.0), 1.25, 40, |studio, frame| {
            if frame == 2 {
                studio.open(project.clone());
                for _ in 0..10_000 {
                    if studio.poll_opening().is_none() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                studio.settings.settings_mut().appearance.icon_folder = icons.display().to_string();
            }
            if frame == 4 {
                // The folder is read in the background; a later frame takes it in.
                std::thread::sleep(Duration::from_millis(300));
            }
        });
        assert_eq!(studio.screen(), Screen::Editor);
        write_png(&out.join("231-custom-icons.png"), shot);
    }

    /// The screenshot of #277 (`cargo test -p metallic_studio -- --ignored screenshot_of_icon_sections`): the editor
    /// with an object from each Explorer section, each section in its own colour. Offscreen, like the others.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_icon_sections() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-icon-sections", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let shot = render_offscreen(&mut studio, egui::vec2(1280.0, 800.0), 1.25, 40, |studio, frame| {
            if frame == 2 {
                studio.open(project.clone());
                for _ in 0..10_000 {
                    if studio.poll_opening().is_none() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                let dm = &mut studio.editor_mut().expect("the project opened").dm;
                let registry = dm.registry().clone();
                let class = |name: &str| registry.class_id(name).unwrap();
                let workspace = dm.service(class("Workspace")).unwrap();
                let storage = dm.service(class("ReplicatedStorage")).unwrap();
                let add = |dm: &mut metallic_engine::metallic_core::DataModel, name: &str, parent| {
                    let id = dm.create(class(name)).unwrap();
                    dm.set_parent(id, Some(parent)).unwrap();
                    id
                };
                let model = add(dm, "Model", workspace);
                add(dm, "Part", model);
                add(dm, "PointLight", workspace);
                add(dm, "SpotLight", workspace);
                let folder = add(dm, "Folder", storage);
                add(dm, "Script", folder);
                add(dm, "LocalScript", folder);
                add(dm, "ModuleScript", folder);
                add(dm, "BindableEvent", folder);
                add(dm, "IntValue", folder);
                add(dm, "StringValue", folder);
                for id in [model, folder] {
                    studio.editor_mut().unwrap().explorer.set_expanded(id, true);
                }
                let storage_id = storage;
                studio.editor_mut().unwrap().explorer.set_expanded(storage_id, true);
                // The Explorer the whole column, so every section shows.
                studio.dock_mut().close(panels::ids::PROPERTIES);
            }
        });
        assert_eq!(studio.screen(), Screen::Editor);
        write_png(&out.join("277-icon-sections.png"), shot);
    }

    /// The screenshot of #239 (`cargo test -p metallic_studio -- --ignored screenshot_of_the_menu_bar`): the editor
    /// with its File menu open, working items and planned (disabled) ones.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_the_menu_bar() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-menu-bar", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let mut offscreen = Offscreen::new(egui::vec2(1280.0, 800.0), 1.25).expect("an offscreen device");
        for _ in 0..3 {
            offscreen.frame(&mut studio);
        }
        studio.open(project);
        for _ in 0..10_000 {
            if studio.poll_opening().is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        for _ in 0..20 {
            offscreen.frame(&mut studio);
        }
        assert_eq!(studio.screen(), Screen::Editor);
        let titles: Vec<(&'static str, egui::Rect)> = offscreen
            .ctx
            .data(|data| data.get_temp(editor::menu_titles_id()))
            .expect("the menu titles are drawn");
        let file = titles.iter().find(|(title, _)| *title == "File").unwrap().1.center();
        let button = |pressed| egui::Event::PointerButton {
            pos: file,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        offscreen.frame_with(&mut studio, vec![egui::Event::PointerMoved(file)]);
        offscreen.frame_with(&mut studio, vec![button(true)]);
        offscreen.frame_with(&mut studio, vec![button(false)]);
        // The pointer on Save, so its hover shows.
        let save = file + egui::vec2(60.0, 4.0 + 28.0 * 3.0 + 18.0);
        offscreen.frame_with(&mut studio, vec![egui::Event::PointerMoved(save)]);
        for _ in 0..6 {
            offscreen.frame(&mut studio);
        }
        write_png(&out.join("239-menu-bar.png"), offscreen.read());
    }

    /// The screenshot of #246 (`cargo test -p metallic_studio -- --ignored screenshot_of_the_game_tab`): a game
    /// running in the Game tab, its script's output in Output, and the play buttons in the top bar.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_the_game_tab() {
        use metallic_engine::metallic_core::Value;

        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-game-tab", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        studio.set_log(Arc::new(crate::logging::LogBuffer::new()));
        let mut offscreen = Offscreen::new(egui::vec2(1280.0, 800.0), 1.25).expect("an offscreen device");
        for _ in 0..3 {
            offscreen.frame(&mut studio);
        }
        studio.open(project);
        for _ in 0..10_000 {
            if studio.poll_opening().is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        offscreen.frame(&mut studio);
        // A script that spins the spawn pad and tints it, and says so.
        let editor = studio.editor_mut().expect("the project opened");
        let dm = &mut editor.dm;
        let registry = dm.registry().clone();
        let class = |name: &str| registry.class_id(name).unwrap();
        let service = dm.service(class("ServerScriptService")).unwrap();
        let script = dm.create(class("Script")).unwrap();
        dm.set_name(script, "Spin".into()).unwrap();
        dm.set_by_name(script, "Source", Value::String("Scripts/Spin.luau".into()))
            .unwrap();
        dm.set_parent(script, Some(service)).unwrap();
        editor
            .project
            .write(
                "Scripts/Spin.luau",
                "print('Spin: the spawn pad turns while the game runs')\n\
                 local pad = workspace:FindFirstChild('SpawnLocation') :: Part\n\
                 pad.Color = Color3.fromRGB(255, 155, 84)\n\
                 game:GetService('RunService').Heartbeat:Connect(function(dt)\n\
                     pad.CFrame = pad.CFrame * CFrame.Angles(0, dt * 2, 0)\n\
                 end)\n",
            )
            .unwrap();
        let workspace = editor.dm.service(class("Workspace")).unwrap();
        let pad = editor.dm.find_first_child(workspace, "SpawnLocation", false).unwrap();
        editor.select(pad);
        studio.menu_action(MenuAction::Play);
        assert!(studio.is_playing(), "{:?}", studio.notice());
        for _ in 0..40 {
            offscreen.frame(&mut studio);
        }
        // #248: the Explorer and Properties show the running game, the spawn pad selected in it.
        write_png(&out.join("246-game-tab.png"), offscreen.read());
    }

    // The viewport (#217).

    /// A key press or release of `key`, as its physical key (the fly commands are bound to physical keys).
    fn physical(key: Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    /// The pointer moved to `at`, then `button` pressed or released there with `modifiers`, a frame each.
    fn pointer(
        harness: &mut Harness<'_, Test>,
        at: egui::Pos2,
        button: egui::PointerButton,
        pressed: bool,
        modifiers: Modifiers,
    ) {
        harness.event(egui::Event::PointerMoved(at));
        harness.event(egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers,
        });
        harness.step();
    }

    /// `key` held for a few frames, then released.
    fn hold(harness: &mut Harness<'_, Test>, key: Key) {
        harness.event(physical(key, true));
        harness.step();
        for _ in 0..3 {
            harness.step();
        }
        harness.event(physical(key, false));
        harness.step();
    }

    /// A right-drag (look) from `from` by `by`.
    fn right_drag(harness: &mut Harness<'_, Test>, from: egui::Pos2, by: egui::Vec2) {
        let button = egui::PointerButton::Secondary;
        harness.event(egui::Event::PointerMoved(from));
        harness.step();
        pointer(harness, from, button, true, Modifiers::NONE);
        harness.event(egui::Event::PointerMoved(from + by));
        harness.step();
        pointer(harness, from + by, button, false, Modifiers::NONE);
    }

    /// One wheel line at `at`, with `modifiers`.
    fn wheel(harness: &mut Harness<'_, Test>, at: egui::Pos2, modifiers: Modifiers) {
        harness.event(egui::Event::PointerMoved(at));
        harness.step();
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            modifiers,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        });
        harness.step();
    }

    fn camera(harness: &Harness<'_, Test>) -> crate::viewport::EditorCamera {
        harness.state().studio.editor().unwrap().viewport.camera
    }

    #[test]
    fn the_editor_camera_moves_by_the_keymap_only_with_focus_or_hover() {
        let mut harness = editing(settings_file("viewport-input", None), "Fly Game");
        let rect = harness
            .state()
            .studio
            .editor()
            .unwrap()
            .viewport
            .rect
            .expect("the viewport was drawn");
        let inside = rect.center();
        let explorer = harness
            .get_by_role_and_label(Role::TreeItem, "Workspace")
            .rect()
            .center();
        assert!(!rect.contains(explorer));
        let start = camera(&harness);
        // It starts where the template's camera is, looking down at the baseplate.
        assert_eq!(
            start.position,
            metallic_engine::metallic_core::glam::Vec3::new(0.0, 20.0, 40.0)
        );
        assert!(start.forward().y < 0.0);

        // Keys need the viewport's focus: nothing has it, then it is only hovered.
        hold(&mut harness, Key::W);
        assert_eq!(camera(&harness), start, "no focus");
        harness.event(egui::Event::PointerMoved(inside));
        harness.step();
        hold(&mut harness, Key::W);
        assert_eq!(camera(&harness), start, "hovered is not focused");

        // A click gives it focus: W flies forward, E up.
        pointer(
            &mut harness,
            inside,
            egui::PointerButton::Primary,
            true,
            Modifiers::NONE,
        );
        pointer(
            &mut harness,
            inside,
            egui::PointerButton::Primary,
            false,
            Modifiers::NONE,
        );
        assert_eq!(
            harness.state().studio.dock().focus().focused,
            Some(panels::ids::VIEWPORT)
        );
        hold(&mut harness, Key::W);
        let flown = camera(&harness);
        assert!(
            (flown.position - start.position).dot(start.forward()) > 0.1,
            "{flown:?}"
        );
        hold(&mut harness, Key::E);
        assert!(camera(&harness).position.y > flown.position.y);

        // Another panel takes the focus, and the keys with it.
        pointer(
            &mut harness,
            explorer,
            egui::PointerButton::Primary,
            true,
            Modifiers::NONE,
        );
        pointer(
            &mut harness,
            explorer,
            egui::PointerButton::Primary,
            false,
            Modifiers::NONE,
        );
        assert_eq!(
            harness.state().studio.dock().focus().focused,
            Some(panels::ids::EXPLORER)
        );
        let before = camera(&harness);
        hold(&mut harness, Key::W);
        assert_eq!(camera(&harness), before, "the Explorer has focus");

        // The mouse starts over the viewport without focus: right-drag looks, the wheel zooms, Shift+wheel sets the
        // secondary speed (#385), Alt+left-drag orbits.
        right_drag(&mut harness, explorer, egui::vec2(40.0, 0.0));
        assert_eq!(camera(&harness), before, "a drag over the Explorer");
        right_drag(&mut harness, inside, egui::vec2(40.0, 0.0));
        let turned = camera(&harness);
        assert!(turned.yaw() < before.yaw(), "turned right");
        assert_eq!(turned.position, before.position, "looking does not move it");

        wheel(&mut harness, explorer, Modifiers::NONE);
        assert_eq!(camera(&harness), turned, "the wheel over the Explorer");
        wheel(&mut harness, inside, Modifiers::NONE);
        let zoomed = camera(&harness);
        assert!(
            (zoomed.position - turned.position).dot(turned.forward()) > 0.0,
            "zoomed in"
        );
        wheel(&mut harness, inside, Modifiers::SHIFT);
        assert!(harness.state().studio.settings().viewport.secondary_speed > 3.0);

        let orbiting = camera(&harness);
        pointer(&mut harness, inside, egui::PointerButton::Primary, true, Modifiers::ALT);
        harness.event(egui::Event::PointerMoved(inside + egui::vec2(60.0, 0.0)));
        harness.step();
        pointer(
            &mut harness,
            inside + egui::vec2(60.0, 0.0),
            egui::PointerButton::Primary,
            false,
            Modifiers::ALT,
        );
        let orbited = camera(&harness);
        assert!(orbited.position.distance(orbiting.position) > 0.1, "it moved around");
        assert!(orbited.yaw() != orbiting.yaw(), "turning to keep the pivot in view");

        // The game's own camera never moved.
        let editor = harness.state().studio.editor().unwrap();
        let workspace = editor
            .dm
            .service(editor.dm.registry().class_id("Workspace").unwrap())
            .unwrap();
        let game = metallic_engine::metallic_render::CameraReader::new()
            .read(&editor.dm, workspace)
            .unwrap();
        assert_eq!(game.position, start.position);
    }

    /// Studio drawn offscreen with the template project open, its viewport drawn at the panel's size. `None` without
    /// an adapter.
    fn viewport_shown(test: &str, size: egui::Vec2) -> Option<(Studio, Offscreen)> {
        let mut offscreen = Offscreen::new(size, 1.0)?;
        let settings = "[general]\nskip_sign_in = true\n[rendering]\npreset = \"medium\"\nanti_aliasing = \"off\"\n";
        let path = config(test, Some(settings), None);
        let project = make_project_once(path.parent().unwrap(), "Viewport Game");
        let mut studio = studio_after_startup(steps_reading(path));
        offscreen.frame(&mut studio);
        offscreen.frame(&mut studio);
        assert_eq!(studio.screen(), Screen::Projects);
        studio.open(project);
        for _ in 0..10_000 {
            if studio.poll_opening().is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(studio.editor().is_some(), "{:?}", studio.projects().error());
        settle_viewport(&mut studio, &mut offscreen);
        Some((studio, offscreen))
    }

    /// Draws frames until the viewport's texture is at the panel's size and shown.
    fn settle_viewport(studio: &mut Studio, offscreen: &mut Offscreen) {
        for _ in 0..2000 {
            offscreen.frame(studio);
            let viewport = &studio.editor().unwrap().viewport;
            let target = offscreen.viewport.scene().viewport().map(|target| target.size());
            if viewport.texture.is_some() && target.is_some() && target == viewport.pixels {
                // One more frame paints it.
                offscreen.frame(studio);
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the viewport never settled at the panel's size");
    }

    /// The mean difference per channel of two RGBA images of the same size.
    fn mean_difference(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len());
        let sum: u64 = a.iter().zip(b).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
        sum as f64 / a.len() as f64
    }

    /// The RGB of pixel (`x`, `y`) of `image`.
    fn rgb((width, _, pixels): &offscreen::Pixels, x: u32, y: u32) -> [u8; 3] {
        let at = ((y * width + x) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2]]
    }

    #[test]
    fn the_viewport_shows_the_scene_as_the_engine_draws_it() {
        use metallic_engine::metallic_render::settings::{GraphicsSettings, Preset};
        use metallic_engine::{FrameStats, SceneRenderer};

        let Some((mut studio, mut offscreen)) = viewport_shown("viewport-scene", egui::vec2(960.0, 600.0)) else {
            return;
        };
        let target = offscreen.viewport.scene().viewport().unwrap();
        let (width, height) = target.size();
        let shown = read_texture(&offscreen.context, target.texture());
        // The scene, not a flat fill: sky at the top, the baseplate at the bottom.
        let [sky, ground] = [rgb(&shown, width / 2, 2), rgb(&shown, width / 2, height - 3)];
        let apart: u32 = sky.iter().zip(ground).map(|(a, b)| u32::from(a.abs_diff(b))).sum();
        assert!(apart > 40, "sky {sky:?} and ground {ground:?}");

        // The engine's own render of the same game from the same camera, at the same size and settings.
        let graphics = studio.settings().rendering.clone();
        assert_eq!(graphics.preset, Preset::Medium);
        let editor = studio.editor_mut().unwrap();
        let mut reference = SceneRenderer::default();
        reference.set_graphics(graphics);
        reference.attach_viewport(&offscreen.context, width, height).unwrap();
        reference.set_camera(Some(editor.viewport.camera.camera()));
        for _ in 0..3 {
            reference.render_viewport(&mut editor.dm, FrameStats::default());
        }
        let expected = read_texture(&offscreen.context, reference.viewport().unwrap().texture());
        let difference = mean_difference(&shown.2, &expected.2);
        assert!(
            difference < 2.0,
            "the viewport differs from the engine's render by {difference}"
        );

        // And the panel shows that texture, one texel to a pixel.
        let frame = offscreen.read();
        let rect = studio.editor().unwrap().viewport.rect.unwrap();
        let (left, top) = (rect.left() as u32, rect.top() as u32);
        let mut off = 0u64;
        let mut samples = 0u64;
        for y in (4..height - 4).step_by(23) {
            for x in (4..width - 4).step_by(29) {
                let [a, b] = [rgb(&frame, left + x, top + y), rgb(&shown, x, y)];
                off += a.iter().zip(b).map(|(a, b)| u64::from(a.abs_diff(b))).sum::<u64>();
                samples += 3;
            }
        }
        let off = off as f64 / samples as f64;
        assert!(off < 3.0, "the panel differs from the texture by {off}");

        // Rendering settings apply live.
        studio.settings.settings_mut().rendering = GraphicsSettings::new(Preset::Low);
        offscreen.frame(&mut studio);
        let graphics = offscreen.viewport.scene().graphics().unwrap();
        assert_eq!(graphics.preset, Preset::Low);
    }

    #[test]
    fn resizing_the_panel_draws_its_new_size_in_the_same_frame() {
        let Some((mut studio, mut offscreen)) = viewport_shown("viewport-resize", egui::vec2(960.0, 600.0)) else {
            return;
        };
        let centre_of = |studio: &Studio, frame: &offscreen::Pixels| {
            let centre = studio.editor().unwrap().viewport.rect.unwrap().center();
            rgb(frame, centre.x as u32, centre.y as u32)
        };
        let before = centre_of(&studio, &offscreen.read());
        let drawn = |offscreen: &Offscreen| offscreen.viewport.scene().viewport_drawn_size();
        let target = |offscreen: &Offscreen| {
            let target = offscreen.viewport.scene().viewport().unwrap();
            (target.size(), target.generation())
        };
        // Smaller (#330): the very next frame is drawn at the panel's new size, in the same target.
        let (size, generation) = target(&offscreen);
        offscreen.set_size(egui::vec2(800.0, 560.0));
        offscreen.frame(&mut studio);
        assert_eq!(drawn(&offscreen), studio.editor().unwrap().viewport.pixels);
        assert_eq!(target(&offscreen), (size, generation), "no texture made");
        let now = centre_of(&studio, &offscreen.read());
        let off: u32 = now.iter().zip(before).map(|(a, b)| u32::from(a.abs_diff(b))).sum();
        assert!(off < 60, "the centre went from {before:?} to {now:?}");
        // Larger than the target: it grows in that same frame, with room to spare, and the frame fills the panel.
        offscreen.set_size(egui::vec2(1400.0, 900.0));
        offscreen.frame(&mut studio);
        let pixels = studio.editor().unwrap().viewport.pixels.unwrap();
        assert_eq!(drawn(&offscreen), Some(pixels));
        let (grown, again) = target(&offscreen);
        assert!(
            again > generation && grown.0 > pixels.0 && grown.1 > pixels.1,
            "{grown:?} for {pixels:?}"
        );
        let now = centre_of(&studio, &offscreen.read());
        let off: u32 = now.iter().zip(before).map(|(a, b)| u32::from(a.abs_diff(b))).sum();
        assert!(off < 60, "the centre went from {before:?} to {now:?}");
        // A little larger again: it fits, so nothing is made.
        offscreen.set_size(egui::vec2(1420.0, 910.0));
        offscreen.frame(&mut studio);
        assert_eq!(target(&offscreen), (grown, again));
        assert_eq!(drawn(&offscreen), studio.editor().unwrap().viewport.pixels);
    }

    /// The viewport's screenshot (`cargo test -p metallic_studio -- --ignored screenshot_of_the_viewport`): the
    /// template project in the editor, the viewport showing the baseplate scene. Offscreen, like the others.
    #[test]
    #[ignore = "writes docs/screenshots; needs a GPU or a software adapter"]
    fn screenshot_of_the_viewport() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
        let path = config("shot-viewport", Some("[general]\nskip_sign_in = true\n"), None);
        let project = make_project_once(path.parent().unwrap(), "Space Race");
        let mut studio = studio_after_startup(steps_reading(path));
        let mut offscreen = Offscreen::new(egui::vec2(1280.0, 800.0), 1.25).expect("an offscreen device");
        offscreen.frame(&mut studio);
        offscreen.frame(&mut studio);
        studio.open(project);
        for _ in 0..10_000 {
            if studio.poll_opening().is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        settle_viewport(&mut studio, &mut offscreen);
        // Long enough for Auto's dynamic resolution and the pipelines to settle.
        for _ in 0..60 {
            offscreen.frame(&mut studio);
        }
        assert_eq!(studio.screen(), Screen::Editor);
        write_png(&out.join("217-viewport.png"), offscreen.read());
    }
}
