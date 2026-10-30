//! Launcher failures contain safe descriptions, never untrusted input or tickets.

use crate::DeepLinkError;

/// A launcher operation failed without exposing credentials or protocol input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LauncherError {
    /// Invalid protocol input; the underlying error never includes the input.
    #[error("Invalid Metallic link: {0}")]
    BadLink(#[from] DeepLinkError),
    /// A site origin failed validation; contains a safe explanation only.
    #[error("Invalid site origin: {0}")]
    BadOrigin(String),
    /// No production site origin has been configured in this build.
    #[error("The Metallic site is not configured.")]
    NotConfigured,
    /// The named product has not been installed.
    #[error("{0} is not installed.")]
    NotInstalled(&'static str),
    /// A persistence operation failed; contains a safe explanation only.
    #[error("{0}")]
    Io(String),
    /// A feature is unavailable on this platform.
    #[error("{0} is not supported on this platform.")]
    Unsupported(&'static str),
    /// A platform operation failed; contains a safe explanation only.
    #[error("{0}")]
    Platform(String),
    /// Command-line input was invalid, without including argument contents.
    #[error("Invalid launcher arguments: {0}")]
    BadArguments(&'static str),
}
