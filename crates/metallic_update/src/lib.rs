//! Shared update and GitHub sign-in code for Metallic Studio and the Metallic Launcher.
//!
//! No UI lives here. [`feed`] checks GitHub Releases and drives Velopack, [`github_auth`] signs
//! the user in with GitHub's device flow, [`secret`] stores the session, and [`http`] holds the
//! HTTP trait and the redacting [`Token`] wrapper that keep tokens out of logs.
//!
//! Library code returns [`UpdateError`] / [`AuthError`]; the apps decide what to show.

#![warn(missing_docs)]

pub mod feed;
pub mod github_auth;
pub mod http;
pub mod secret;

pub use feed::{
    CHECK_INTERVAL, Channel, LAUNCHER_FEED, ReleaseFeed, STUDIO_FEED, UpdateChecker, UpdateError,
    UpdateInfo,
};
pub use github_auth::{AuthError, GithubAuth};
pub use http::{HttpClient, HttpError, HttpResponse, Token, TokenProvider, UreqClient};
pub use secret::{KeyringSecretStore, MemorySecretStore, SecretStore};
/// Re-export so the apps name the same Velopack version (`VelopackApp::build().run()`).
pub use velopack;
