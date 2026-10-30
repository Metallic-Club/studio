//! Metallic Studio library part: code that the Studio app and its tests share.
//!
//! Task 13 adds the installer-side pieces: [`updater`] (update check, download, apply) and
//! [`github_auth`] (GitHub device sign-in and token storage).

#![warn(missing_docs)]

pub mod github_auth;
pub mod updater;
