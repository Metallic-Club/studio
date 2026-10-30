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
    /// This platform does not support the step.
    #[error("not supported on this platform: {0}")]
    Unsupported(&'static str),
    /// The operating system refused or failed a call.
    #[error("platform error: {0}")]
    Platform(String),
}
