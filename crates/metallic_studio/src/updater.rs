//! Update check, download and apply for Metallic Studio.
//!
//! The code lives in the shared `metallic_update` crate (the Launcher uses it too); this module
//! re-exports it. Studio's releases are [`STUDIO_FEED`]: build a checker with
//! `UpdateChecker::new(STUDIO_FEED, channel, tokens)`.

pub use metallic_update::feed::*;
pub use metallic_update::http::*;
