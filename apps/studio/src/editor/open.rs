//! Opening a project off the UI thread (ADR 0009, Screens): `Project::open`, the start scene into a new game, the
//! asset index, and the asset loader, one named step at a time behind the progress overlay.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;

use metallic_engine::MeshAssets;
use metallic_engine::metallic_core::DataModel;
use metallic_engine::metallic_project::{Project, SceneIds};

/// The steps the overlay names, in order.
pub const STEPS: [&str; 4] = [
    "Opening the project",
    "Loading the start scene",
    "Checking the assets",
    "Starting the asset loader",
];

/// One asset of the project's index, as the Assets panel lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetRow {
    /// Its path in the project (`Meshes/Rock.glb`).
    pub path: String,
    /// Its stable id, as text.
    pub id: String,
    /// Its size in bytes, when its member is there.
    pub size: Option<u64>,
    /// Whether the index names it but the project no longer has it.
    pub missing: bool,
}

/// What opening produced.
pub struct Opened {
    /// The project.
    pub project: Project,
    /// The game, with the start scene loaded.
    pub dm: DataModel,
    /// The scene ids of the start scene's instances, for saving it back.
    pub scene_ids: SceneIds,
    /// The asset index, by path.
    pub assets: Vec<AssetRow>,
    /// The mesh and texture loader, for the viewport's renderer.
    pub mesh_assets: MeshAssets,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("project", &self.project)
            .field("instances", &self.dm.len())
            .field("assets", &self.assets.len())
            .finish_non_exhaustive()
    }
}

/// What the worker reports.
enum Progress {
    /// Step `n` (from 0) is running.
    Started(usize),
    /// Done: the project, or why it could not be opened.
    Finished(Result<Box<Opened>, String>),
}

/// A project being opened on a worker thread, as last polled.
#[derive(Debug)]
pub struct Opening {
    path: PathBuf,
    receiver: Receiver<Progress>,
    step: usize,
    result: Option<Result<Box<Opened>, String>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Progress::Started(n) => f.debug_tuple("Started").field(n).finish(),
            Progress::Finished(result) => f.debug_tuple("Finished").field(&result.is_ok()).finish(),
        }
    }
}

impl Opening {
    /// Starts opening the project at `path`.
    pub fn spawn(path: PathBuf) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let file = path.clone();
        let thread = std::thread::Builder::new()
            .name("studio open project".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let result = open(&file, |n| {
                    // Nobody listens once the project was closed while opening: that is fine.
                    let _ = sender.send(Progress::Started(n));
                });
                match &result {
                    Ok(_) => log::info!("projects: opened {} in {:.0?}", file.display(), started.elapsed()),
                    Err(error) => log::warn!("projects: could not open {}: {error}", file.display()),
                }
                let _ = sender.send(Progress::Finished(result.map(Box::new)));
            })?;
        Ok(Self {
            path,
            receiver,
            step: 0,
            result: None,
            thread: Some(thread),
        })
    }

    /// The project's file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads what the worker reported. Never blocks. Returns the outcome once, when it is done.
    pub fn poll(&mut self) -> Option<Result<Box<Opened>, String>> {
        loop {
            match self.receiver.try_recv() {
                Ok(Progress::Started(n)) => self.step = n,
                Ok(Progress::Finished(result)) => {
                    self.result = Some(result);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.result.is_none() {
                        self.result = Some(Err(format!(
                            "Studio stopped while {}",
                            STEPS.get(self.step).copied().unwrap_or("opening it").to_lowercase()
                        )));
                    }
                    break;
                }
            }
        }
        let result = self.result.take()?;
        if let Some(thread) = self.thread.take() {
            // It has sent its last message (or died), so this returns at once.
            let _ = thread.join();
        }
        Some(result)
    }

    /// The step running, from 0.
    pub fn step(&self) -> usize {
        self.step
    }
}

/// Opens the project at `path`, calling `step(n)` as each of [`STEPS`] starts.
fn open(path: &Path, mut step: impl FnMut(usize)) -> Result<Opened, String> {
    step(0);
    let mut project = Project::open(path).map_err(|error| error.to_string())?;
    for warning in project.warnings() {
        log::warn!("projects: {}: {warning}", path.display());
    }

    step(1);
    let mut dm = metallic_engine::metallic_classes::new_game()
        .map_err(|error| format!("Studio could not start an empty game: {error}"))?;
    let loaded = project.load_start_scene(&mut dm).map_err(|error| error.to_string())?;
    for warning in &loaded.warnings {
        log::warn!("scene: {}: {warning}", project.manifest().start_scene);
    }

    step(2);
    match project.refresh_assets() {
        Ok(report) => {
            for asset in &report.missing {
                log::warn!("assets: `{}` is in the index but not in the project", asset.path);
            }
        }
        Err(error) => log::warn!("assets: could not bring the index up to date: {error}"),
    }
    let assets = asset_rows(&project);

    step(3);
    let mesh_assets = MeshAssets::new(project.asset_reader());
    Ok(Opened {
        project,
        dm,
        scene_ids: loaded.ids,
        assets,
        mesh_assets,
    })
}

/// The project's asset index as rows, by path.
pub fn asset_rows(project: &Project) -> Vec<AssetRow> {
    let mut rows: Vec<AssetRow> = project
        .assets()
        .iter()
        .map(|(path, id)| {
            let missing = !project.contains(path);
            AssetRow {
                path: path.to_owned(),
                id: id.to_string(),
                size: (!missing)
                    .then(|| project.fingerprint(path).ok().map(|fingerprint| fingerprint.size))
                    .flatten(),
                missing,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows
}
