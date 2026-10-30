//! The error type shared by all Studio modules.

use std::path::PathBuf;

/// Everything that can go wrong in the Studio shell, with creator-readable messages.
#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    /// The chosen project folder already has files in it; nothing was written.
    #[error("This folder isn't empty. Choose an empty folder or a new one.")]
    FolderNotEmpty(PathBuf),
    /// The project name cannot be used as a folder name.
    #[error("{0}")]
    InvalidName(String),
    /// A file or folder could not be read or written.
    #[error("Could not access {path}: {source}")]
    Io {
        /// The file or folder involved.
        path: PathBuf,
        /// The operating-system error.
        source: std::io::Error,
    },
    /// A `template.toml` or `metallic.toml` could not be understood.
    #[error("Invalid metadata in {path}: {message}")]
    Metadata {
        /// The file involved.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
    /// An image file could not be decoded.
    #[error("Could not read image {path}: {message}")]
    Image {
        /// The file involved (or a label for embedded data).
        path: String,
        /// What is wrong with it.
        message: String,
    },
    /// A scene or project file could not be read or written.
    #[error("Could not open or save {path}: {message}")]
    Scene {
        /// The file or folder involved.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
    /// The chosen folder is not a Metallic game.
    #[error("This folder isn't a Metallic game (it has no metallic.toml).")]
    NotAGame(PathBuf),
}

impl StudioError {
    /// Builds a [`StudioError::Io`] for `path`.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
