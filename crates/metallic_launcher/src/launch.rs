//! Starting the Player and Metallic Studio. The programs are started directly (no shell), and
//! the Player gets one argument only: the canonical link from `DeepLink::to_uri`, never the
//! text that was received.
//!
//! Player contract: `metallic-player.exe <metallic://play?place=<id>&ticket=<ticket>>`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::deep_link::{DeepLink, DeepLinkError};
use crate::error::LauncherError;

/// Starts programs. The real one lives in `platform`; tests record the calls.
pub trait Spawner {
    /// Starts `exe` with exactly `args`, without a shell and without a console window, and
    /// returns its process id.
    fn spawn(&self, exe: &Path, args: &[OsString]) -> Result<u32, LauncherError>;

    /// Starts `exe` like [`spawn`](Self::spawn), waits until it ends and returns its exit code.
    /// Used for installers. A program still running after `time_limit` is killed and the call
    /// fails. The default says "unsupported", so a spawner that cannot wait never pretends an
    /// installer finished.
    fn spawn_and_wait(
        &self,
        exe: &Path,
        args: &[OsString],
        time_limit: Duration,
    ) -> Result<i32, LauncherError> {
        let _ = (exe, args, time_limit);
        Err(LauncherError::Unsupported("waiting for a program"))
    }

    /// Opens `path` for reading so that nobody else can write to it or delete it while the
    /// returned handle is open (Windows: share mode read only). The default is a plain open, for
    /// systems without such a lock.
    fn open_locked(&self, path: &Path) -> Result<std::fs::File, LauncherError> {
        std::fs::File::open(path).map_err(|e| LauncherError::Io(e.to_string()))
    }
}

/// Runs `command` and waits for it. After `limit` the program is killed and the call fails, so
/// a program that waits for input cannot hold the Launcher for ever.
pub fn run_with_limit(command: &mut Command, limit: Duration) -> Result<i32, LauncherError> {
    let mut child = command
        .spawn()
        .map_err(|e| LauncherError::Platform(format!("could not run the program: {e}")))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code().unwrap_or(-1)),
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                return Err(LauncherError::Platform(format!("could not wait: {e}")));
            }
        }
        if started.elapsed() >= limit {
            // Killing a program that has just ended fails harmlessly.
            let _ = child.kill();
            let _ = child.wait();
            return Err(LauncherError::Update(
                "the installer took too long".to_string(),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Where the programs this Launcher starts are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductPaths {
    /// `metallic-player.exe` beside the Launcher. Empty when the Launcher's own path is unknown.
    pub player_exe: PathBuf,
    /// Metallic Studio, when it is installed.
    pub studio_exe: Option<PathBuf>,
}

/// The paths for this run: the Player beside `current_exe()`, Studio under `%LOCALAPPDATA%`.
pub fn product_paths() -> ProductPaths {
    let exe = std::env::current_exe().ok();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    product_paths_for(exe.as_deref(), local.as_deref())
}

/// [`product_paths`] with the inputs given (for tests).
pub fn product_paths_for(
    current_exe: Option<&Path>,
    local_app_data: Option<&Path>,
) -> ProductPaths {
    // An unknown folder gives an empty path, never a bare file name that a search could resolve.
    let player_exe = current_exe
        .and_then(Path::parent)
        .map(|dir| dir.join("metallic-player.exe"))
        .unwrap_or_default();
    let studio_exe = local_app_data
        .map(|base| {
            base.join("MetallicStudio")
                .join("current")
                .join("Metallic Studio.exe")
        })
        .filter(|path| path.is_file());
    ProductPaths {
        player_exe,
        studio_exe,
    }
}

/// Starts the Player for a Play link. Only `DeepLink::Play` is accepted; the Player receives
/// the canonical link as its single argument.
pub fn start_player(
    spawner: &dyn Spawner,
    paths: &ProductPaths,
    link: &DeepLink,
) -> Result<u32, LauncherError> {
    if !matches!(link, DeepLink::Play { .. }) {
        return Err(LauncherError::BadLink(DeepLinkError::UnknownAction));
    }
    if !paths.player_exe.is_file() {
        return Err(LauncherError::NotInstalled("player"));
    }
    spawner.spawn(&paths.player_exe, &[OsString::from(link.to_uri())])
}

/// Starts Metallic Studio with no arguments (it does not take a place yet).
pub fn start_studio(spawner: &dyn Spawner, paths: &ProductPaths) -> Result<u32, LauncherError> {
    match &paths.studio_exe {
        Some(exe) if exe.is_file() => spawner.spawn(exe, &[]),
        _ => Err(LauncherError::NotInstalled("studio")),
    }
}
