//! The platform layer's error type.

/// Why a platform call failed. Backend errors are carried as text so no backend type leaks into the engine.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlatformError {
    /// The feature has no implementation on this platform yet (for example the event loop on Android and iOS
    /// before M9).
    #[error("{what} is not supported on {platform} yet")]
    Unsupported {
        /// What was asked for.
        what: &'static str,
        /// The platform it was asked on (`std::env::consts::OS`, or a console backend's name).
        platform: &'static str,
    },
    /// The event loop could not be created or stopped with an error.
    #[error("event loop error: {0}")]
    EventLoop(String),
    /// The OS refused to create a window.
    #[error("could not create a window: {0}")]
    CreateWindow(String),
    /// The OS refused a window change (fullscreen, cursor grab).
    #[error("window error: {0}")]
    Window(String),
    /// The app id is not of the form `qualifier.organization.application`.
    #[error("invalid app id {id:?}: {reason}")]
    InvalidAppId {
        /// The id that was given.
        id: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// The user folders could not be found (no home folder for the current user).
    #[error("no home folder for the current user, so there are no user folders")]
    NoHomeDir,
    /// A file association could not be registered or removed (a registry value or a file refused).
    #[error("{0}")]
    FileAssociation(String),
}

impl PlatformError {
    /// An [`PlatformError::Unsupported`] error for `what` on the platform this build targets.
    pub fn unsupported(what: &'static str) -> Self {
        Self::Unsupported {
            what,
            platform: std::env::consts::OS,
        }
    }
}
