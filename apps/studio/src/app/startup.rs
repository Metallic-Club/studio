//! What Studio does while the splash shows: real startup work, one named step at a time, on a worker thread so the
//! splash keeps drawing (ADR 0009, Screens).
//!
//! The render device is made before the splash, because the splash is drawn with it; the steps here are what comes
//! after it. [`studio_steps`] lists them.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;

use metallic_engine::metallic_platform::AppId;
use metallic_engine::metallic_render::{RenderContext, Renderer, RendererSettings, ViewportTarget};

use crate::STUDIO_APP_ID;
use crate::dock::{Dock, LAYOUT_FILE};
use crate::panels::PANELS;
use crate::projects::{
    ProjectEntry, ProjectList, TemplateInfo, list_templates, name_from_path, projects_path, templates_dir,
};
use crate::settings::{SettingsStore, settings_path};

/// What the startup steps produce, handed to the screens after the splash.
#[derive(Default)]
pub struct Loaded {
    /// The settings, read from `settings.toml` (in memory only if Studio has no config folder).
    pub settings: SettingsStore,
    /// The renderer the viewport will draw with, its first-frame pipelines compiled; `None` if it could not be made
    /// (the error is logged, and Studio runs without a viewport).
    pub viewport_renderer: Option<Renderer>,
    /// The projects list, read from `projects.toml`, most recently opened first, its missing files marked.
    pub projects: ProjectList,
    /// The templates a new project can start from.
    pub templates: Vec<TemplateInfo>,
    /// The games folder (`Documents\Metallic`, #312), whose games the list was given; `None` when Studio was started
    /// without one (tests).
    pub games_folder: Option<PathBuf>,
    /// The editor's dock layout, read from `layout.json` beside the settings (the default one in memory if Studio has no
    /// config folder).
    pub layout: Dock,
    /// The editor's toolbar, read from `toolbar.json` beside the settings (the default one in memory if Studio has no
    /// config folder).
    pub toolbar: crate::toolbar::Toolbar,
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loaded")
            .field("settings", &self.settings)
            .field("viewport_renderer", &self.viewport_renderer.is_some())
            .field("projects", &self.projects)
            .field("templates", &self.templates)
            .field("games_folder", &self.games_folder)
            .field("layout", &self.layout)
            .field("toolbar", &self.toolbar)
            .finish()
    }
}

/// The work of one step.
type Work = Box<dyn FnOnce(&mut Loaded) + Send>;

/// One startup step: the name the splash shows while it runs, and its work.
pub struct Step {
    name: &'static str,
    work: Work,
}

impl Step {
    /// A step called `name` that does `work`.
    pub fn new(name: &'static str, work: impl FnOnce(&mut Loaded) + Send + 'static) -> Self {
        Self {
            name,
            work: Box::new(work),
        }
    }

    /// The name the splash shows while it runs.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

impl std::fmt::Debug for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Step").field(&self.name).finish()
    }
}

/// What the worker reports.
enum Progress {
    /// Step `n` (from 0) is running.
    Started(usize),
    /// Every step is done.
    Finished(Box<Loaded>),
}

/// The startup steps running on their worker thread, as last polled.
#[derive(Debug)]
pub struct Startup {
    names: Vec<&'static str>,
    receiver: Receiver<Progress>,
    /// Steps done.
    done: usize,
    /// What the steps produced, once all are done and until it is taken.
    loaded: Option<Loaded>,
    finished: bool,
    thread: Option<JoinHandle<()>>,
}

impl Startup {
    /// Starts running `steps`, in order, on a worker thread.
    pub fn spawn(steps: Vec<Step>) -> std::io::Result<Self> {
        let names = steps.iter().map(Step::name).collect();
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("studio startup".into())
            .spawn(move || {
                let mut loaded = Loaded::default();
                for (n, step) in steps.into_iter().enumerate() {
                    // The splash may already be gone (Studio closed): then nobody listens, and that is fine.
                    let _ = sender.send(Progress::Started(n));
                    let started = std::time::Instant::now();
                    (step.work)(&mut loaded);
                    log::info!("startup: {} in {:.0?}", step.name, started.elapsed());
                }
                let _ = sender.send(Progress::Finished(Box::new(loaded)));
            })?;
        Ok(Self {
            names,
            receiver,
            done: 0,
            loaded: None,
            finished: false,
            thread: Some(thread),
        })
    }

    /// Reads what the worker reported since the last poll. Never blocks.
    pub fn poll(&mut self) {
        while !self.finished {
            match self.receiver.try_recv() {
                Ok(Progress::Started(n)) => self.done = n,
                Ok(Progress::Finished(loaded)) => self.finish(*loaded),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // A step panicked: Studio goes on with what a fresh install has.
                    log::error!(
                        "startup: the step `{}` stopped Studio's startup early; starting with defaults",
                        self.names.get(self.done).copied().unwrap_or("?")
                    );
                    self.finish(Loaded::default());
                }
            }
        }
    }

    fn finish(&mut self, loaded: Loaded) {
        self.done = self.names.len();
        self.loaded = Some(loaded);
        self.finished = true;
        if let Some(thread) = self.thread.take() {
            // It has sent its last message (or died), so this returns at once.
            let _ = thread.join();
        }
    }

    /// The steps' names, in order.
    pub fn names(&self) -> &[&'static str] {
        &self.names
    }

    /// How many steps are done.
    pub fn done(&self) -> usize {
        self.done
    }

    /// Whether every step is done.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// What the steps produced, once they are all done (only the first call gets it).
    pub fn take_loaded(&mut self) -> Option<Loaded> {
        self.loaded.take()
    }
}

/// Studio's startup steps, on the render device the splash is drawn with.
pub fn studio_steps(context: RenderContext) -> Vec<Step> {
    vec![
        Step::new("Preparing folders", |_| {
            crate::folders::prepare(crate::theme::icons::builtin::shipped_folder());
        }),
        settings_step(studio_settings_path()),
        Step::new("Starting the renderer", move |loaded| {
            loaded.viewport_renderer = start_renderer(&context);
        }),
        projects_step(
            studio_projects_path(),
            templates_dir(),
            Some(crate::folders::games_folder()),
        ),
    ]
}

/// The step that reads the projects list at `list` (`None`: there is no config folder, so it lives in memory), adds
/// every game in the `games` folder it does not list yet (#312), and reads the templates in `templates`.
pub fn projects_step(list: Option<PathBuf>, templates: Option<PathBuf>, games: Option<PathBuf>) -> Step {
    Step::new("Reading recent projects", move |loaded| {
        loaded.projects = match list {
            Some(path) => {
                log::info!("projects: {}", path.display());
                ProjectList::load(path)
            }
            None => ProjectList::in_memory(),
        };
        if let Some(games) = &games {
            let mut added = 0;
            for game in crate::folders::find_games(games) {
                let name = name_from_path(&game);
                added += usize::from(loaded.projects.push(ProjectEntry::new(game, name)));
            }
            if added > 0 {
                log::info!("projects: {added} games in {}", games.display());
                loaded.projects.save_or_log();
            }
        }
        loaded.games_folder = games;
        loaded.templates = match templates {
            Some(dir) => list_templates(&dir),
            None => {
                log::warn!("templates: no templates folder was found; New project has nothing to start from");
                Vec::new()
            }
        };
    })
}

/// The step that reads the settings at `path` (`None`: there is no config folder, so they live in memory), and the
/// dock layout beside them.
pub fn settings_step(path: Option<PathBuf>) -> Step {
    Step::new("Reading settings", move |loaded| {
        loaded.settings = match path {
            Some(path) => {
                log::info!("settings: {}", path.display());
                loaded.layout = Dock::load(path.with_file_name(LAYOUT_FILE), PANELS);
                loaded.toolbar =
                    crate::toolbar::Toolbar::load(path.with_file_name(crate::toolbar::TOOLBAR_FILE), PANELS);
                SettingsStore::load(path)
            }
            None => SettingsStore::in_memory(),
        };
    })
}

/// Where Studio's settings file is, if the user has a config folder.
pub(super) fn studio_settings_path() -> Option<PathBuf> {
    match AppId::parse(STUDIO_APP_ID).and_then(|app| settings_path(&app)) {
        Ok(path) => Some(path),
        Err(error) => {
            log::warn!("settings: no config folder ({error}); settings will not be saved");
            None
        }
    }
}

/// The viewport's renderer on the shared device, at a typical viewport size: it returns once the pipelines of its
/// first frame are compiled and queues the rest in the background, so the editor's first frame does not wait for
/// them. Where the device keeps a pipeline cache (Vulkan) the next start compiles less.
fn start_renderer(context: &RenderContext) -> Option<Renderer> {
    let settings = RendererSettings::new(ViewportTarget::FORMAT, 1280, 720);
    match Renderer::new(context, settings) {
        Ok(renderer) => Some(renderer),
        Err(error) => {
            log::warn!("renderer: could not start the viewport's renderer: {error}");
            None
        }
    }
}

/// Where Studio's projects list is, if the user has a config folder.
fn studio_projects_path() -> Option<PathBuf> {
    match AppId::parse(STUDIO_APP_ID).and_then(|app| projects_path(&app)) {
        Ok(path) => Some(path),
        Err(error) => {
            log::warn!("projects: no config folder ({error}); the projects list will not be saved");
            None
        }
    }
}

/// Polls until `startup` is finished, failing after a few seconds.
#[cfg(test)]
pub(crate) fn wait(startup: &mut Startup) {
    for _ in 0..5000 {
        startup.poll();
        if startup.is_finished() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("the startup steps never finished");
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn steps_run_in_order_and_report_progress() {
        let (go, gate) = mpsc::channel::<()>();
        let mut startup = Startup::spawn(vec![
            Step::new("one", move |loaded| {
                gate.recv().unwrap();
                loaded.templates.push(template("one"));
            }),
            Step::new("two", |loaded| loaded.templates.push(template("two"))),
        ])
        .unwrap();
        assert_eq!(startup.names(), ["one", "two"]);
        startup.poll();
        assert_eq!(startup.done(), 0);
        assert!(!startup.is_finished());
        assert!(startup.take_loaded().is_none());

        go.send(()).unwrap();
        wait(&mut startup);
        assert_eq!(startup.done(), 2);
        let loaded = startup.take_loaded().unwrap();
        assert_eq!(loaded.templates, [template("one"), template("two")]);
        assert!(startup.take_loaded().is_none(), "taken once");
    }

    #[test]
    fn a_failed_step_ends_startup_with_defaults() {
        let mut startup = Startup::spawn(vec![
            Step::new("breaks", |_| panic!("a broken step")),
            Step::new("never runs", |loaded| loaded.templates.push(template("no"))),
        ])
        .unwrap();
        wait(&mut startup);
        let loaded = startup.take_loaded().unwrap();
        assert!(loaded.templates.is_empty());
        assert!(loaded.projects.entries().is_empty());
        assert_eq!(loaded.settings.settings(), &crate::settings::StudioSettings::default());
    }

    #[test]
    fn the_settings_step_reads_the_file() {
        let dir = std::env::temp_dir().join(format!("metallic-studio-startup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(crate::settings::SETTINGS_FILE);
        std::fs::write(&path, "[general]\nskip_sign_in = true\n").unwrap();

        let mut startup = Startup::spawn(vec![settings_step(Some(path.clone())), settings_step(None)]).unwrap();
        wait(&mut startup);
        // The last step wins: no folder, settings in memory.
        let loaded = startup.take_loaded().unwrap();
        assert!(loaded.settings.path().is_none());

        let mut startup = Startup::spawn(vec![settings_step(Some(path.clone()))]).unwrap();
        wait(&mut startup);
        let loaded = startup.take_loaded().unwrap();
        assert_eq!(loaded.settings.path(), Some(path.as_path()));
        assert!(loaded.settings.settings().general.skip_sign_in);
    }

    #[test]
    fn the_projects_step_reads_the_list_and_the_templates() {
        let dir = std::env::temp_dir().join(format!("metallic-studio-startup-projects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(crate::projects::PROJECTS_FILE);
        let there = dir.join("There.mtproj");
        std::fs::write(&there, "a project").unwrap();
        std::fs::write(
            &path,
            format!(
                "[[project]]\npath = '{}'\nname = \"There\"\nlast_opened = 20\n[[project]]\npath = '{}'\nname = \
                 \"Gone\"\nlast_opened = 10\n",
                there.display(),
                dir.join("Gone.mtproj").display()
            ),
        )
        .unwrap();

        let mut startup = Startup::spawn(vec![projects_step(Some(path.clone()), templates_dir(), None)]).unwrap();
        assert_eq!(startup.names(), ["Reading recent projects"]);
        wait(&mut startup);
        let loaded = startup.take_loaded().unwrap();
        assert_eq!(loaded.projects.path(), Some(path.as_path()));
        let names: Vec<_> = loaded
            .projects
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["There", "Gone"]);
        let missing: Vec<_> = loaded.projects.entries().iter().map(|entry| entry.missing).collect();
        assert_eq!(missing, [false, true]);
        assert!(loaded.templates.iter().any(|template| template.id == "baseplate-3d"));

        let mut startup = Startup::spawn(vec![projects_step(None, None, None)]).unwrap();
        wait(&mut startup);
        let loaded = startup.take_loaded().unwrap();
        assert!(loaded.projects.path().is_none());
        assert!(loaded.templates.is_empty());
    }

    #[test]
    fn every_game_in_the_games_folder_is_listed() {
        let dir = std::env::temp_dir().join(format!("metallic-games-step-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let games = dir.join("Metallic");
        std::fs::create_dir_all(games.join("Space Race")).unwrap();
        std::fs::write(games.join("Space Race").join("Space Race.mtproj"), "").unwrap();
        std::fs::write(games.join("Racer.mtproj"), "").unwrap();
        let list = dir.join("projects.toml");
        let mut startup = Startup::spawn(vec![projects_step(Some(list.clone()), None, Some(games.clone()))]).unwrap();
        wait(&mut startup);
        let loaded = startup.take_loaded().unwrap();
        // Both, never opened, and saved so the list keeps them (#312).
        let names: Vec<&str> = loaded
            .projects
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["Racer", "Space Race"]);
        assert!(
            loaded
                .projects
                .entries()
                .iter()
                .all(|entry| entry.last_opened.is_none())
        );
        assert_eq!(loaded.games_folder.as_deref(), Some(games.as_path()));
        assert!(std::fs::read_to_string(&list).unwrap().contains("Space Race.mtproj"));
        // A second start adds nothing twice.
        let mut again = Startup::spawn(vec![projects_step(Some(list), None, Some(games))]).unwrap();
        wait(&mut again);
        assert_eq!(again.take_loaded().unwrap().projects.entries().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn template(id: &str) -> TemplateInfo {
        TemplateInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            kind: "3d".to_owned(),
            dir: PathBuf::from(id),
        }
    }
}
