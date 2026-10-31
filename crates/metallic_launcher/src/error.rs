//! The Launcher's error type. No variant carries a token or a join ticket.

use crate::deep_link::DeepLinkError;

/// Why a Launcher step failed.
#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    /// A `metallic://` link was refused.
    #[error("bad metallic:// link: {0}")]
    BadLink(#[from] DeepLinkError),
    /// A site address is not usable (the text explains why, without repeating the address).
    #[error("bad site address: {0}")]
    BadOrigin(String),
    /// The site address in `config/launcher_site_origin.txt` is still the placeholder.
    #[error("the site address is not configured yet")]
    NotConfigured,
    /// A part the step needs is not installed (the text names it).
    #[error("{0} is not installed")]
    NotInstalled(&'static str),
    /// Reading or writing a file failed.
    #[error("file error: {0}")]
    Io(String),
    /// A message from another Launcher was refused (the text says why, never what it held).
    #[error("bad hand-over message: {0}")]
    BadHandoff(&'static str),
    /// This platform does not support the step.
    #[error("not supported on this platform: {0}")]
    Unsupported(&'static str),
    /// The operating system refused or failed a call.
    #[error("platform error: {0}")]
    Platform(String),
    /// The same kind of step is already running (the text names it).
    #[error("{0} is already running")]
    Busy(&'static str),
    /// This Launcher must update before it can start the Player.
    #[error("the Launcher must update first")]
    UpdateRequired,
    /// The team build feed needs a GitHub sign-in first.
    #[error("sign in to GitHub for team builds first")]
    NeedsGithubSignIn,
    /// An install or update step failed (a fixed phrase, never a token or an address).
    #[error("{0}")]
    Update(String),
}
