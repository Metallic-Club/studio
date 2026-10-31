//! Studio's own data folders. They live under the Velopack pack id `MetallicStudio`, never under
//! `Metallic\`, which another app may own. Cleanup removes a folder only when it holds the marker
//! file Studio wrote when it created the folder.

use std::path::Path;

pub use crate::theme::saved::{STUDIO_FOLDER, config_dir_in, resolve_config_dir};

/// The file that says "Studio created this folder".
pub const DATA_MARKER: &str = ".metallic-studio";

/// Creates `dir`. When it did not exist, it also gets the marker; an existing folder is left as it
/// is (and stays unmarked, so cleanup will not delete it).
pub fn create_owned(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(DATA_MARKER), b"created by Metallic Studio\n")
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
        log::warn!("not removing a data folder: it has no Studio marker");
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

/// Cleanup with the base folders given: removes the marked Studio folders only.
pub fn clean_up_in(local_app_data: Option<&Path>, app_data: Option<&Path>) {
    for base in [local_app_data, app_data].into_iter().flatten() {
        remove_owned(&config_dir_in(base));
    }
}
