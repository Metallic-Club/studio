//! The Launcher's own data folders. They live under the Velopack pack id `MetallicLauncher`,
//! never under `Metallic\`, which another app may own. Cleanup removes a folder only when it holds
//! the marker file this Launcher wrote when it created the folder.

use std::path::{Path, PathBuf};

/// The folder name under `%LOCALAPPDATA%` and `%APPDATA%` (the Velopack pack id).
pub const LAUNCHER_FOLDER: &str = "MetallicLauncher";

/// The file that says "this Launcher created this folder".
pub const DATA_MARKER: &str = ".metallic-launcher";

/// Downloads and the WebView2 profile: `<local app data>\MetallicLauncher\data`.
pub fn local_data_dir(local_app_data: &Path) -> PathBuf {
    local_app_data.join(LAUNCHER_FOLDER).join("data")
}

/// The WebView2 profile folder.
pub fn webview_dir(local_app_data: &Path) -> PathBuf {
    local_data_dir(local_app_data).join("WebView2")
}

/// Where installers are downloaded.
pub fn downloads_dir(local_app_data: &Path) -> PathBuf {
    local_data_dir(local_app_data).join("Downloads")
}

/// The settings folder: `<app data>\MetallicLauncher`.
pub fn config_dir(app_data: &Path) -> PathBuf {
    app_data.join(LAUNCHER_FOLDER)
}

/// Creates `dir`. When it did not exist, it also gets the marker; an existing folder is left as
/// it is (and stays unmarked, so cleanup will not delete it).
pub fn create_owned(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(DATA_MARKER), b"created by the Metallic Launcher\n")
}

/// What [`remove_owned`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// The folder was ours and is gone.
    Removed,
    /// There was no folder.
    Missing,
    /// The folder has no marker (or could not be removed): left alone.
    NotOurs,
}

/// Deletes `dir` and everything in it, but only when it holds the marker.
pub fn remove_owned(dir: &Path) -> RemoveOutcome {
    if !dir.exists() {
        return RemoveOutcome::Missing;
    }
    if !dir.join(DATA_MARKER).is_file() {
        log::warn!("not removing a data folder: it has no Launcher marker");
        return RemoveOutcome::NotOurs;
    }
    match std::fs::remove_dir_all(dir) {
        Ok(()) => RemoveOutcome::Removed,
        Err(error) => {
            log::warn!("could not remove a data folder: {error}");
            RemoveOutcome::NotOurs
        }
    }
}

/// The uninstall cleanup with the base folders given: removes the marked Launcher folders only.
pub fn clean_up_in(local_app_data: Option<&Path>, app_data: Option<&Path>) {
    if let Some(local) = local_app_data {
        remove_owned(&local_data_dir(local));
    }
    if let Some(roaming) = app_data {
        remove_owned(&config_dir(roaming));
    }
}
