//! Starting the Player and Metallic Studio. The programs are started directly (no shell), and
//! the Player gets one argument only: the canonical link from `DeepLink::to_uri`, never the
//! text that was received.
//!
//! Player contract: `metallic-player.exe <metallic://play?place=<id>&ticket=<ticket>>`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::deep_link::{DeepLink, DeepLinkError};
use crate::error::LauncherError;

/// Starts programs. The real one lives in `platform`; tests record the calls.
pub trait Spawner {
    /// Starts `exe` with exactly `args`, without a shell and without a console window, and
    /// returns its process id.
    fn spawn(&self, exe: &Path, args: &[OsString]) -> Result<u32, LauncherError>;
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
