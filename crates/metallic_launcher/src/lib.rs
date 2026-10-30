//! Metallic Launcher: the native shell around the Metallic platform website.
//!
//! This crate holds the logic that has no window: the `metallic://` deep links ([`deep_link`]),
//! the site origin and start-up arguments ([`site`]), the rule for what the web view may load
//! ([`url_policy`]) and the Launcher's settings file ([`settings`]). Everything that touches
//! the operating system or a window arrives in later tasks.
//!
//! Library code returns [`LauncherError`]; only `main.rs` decides what to show on failure.

#![warn(missing_docs)]

pub mod deep_link;
pub mod error;
pub mod settings;
pub mod site;
pub mod url_policy;

pub use deep_link::{DeepLink, DeepLinkError, JoinTicket, PlaceId, SitePath};
pub use error::LauncherError;
pub use settings::{LauncherSettings, SettingsPatch, SettingsStore};
pub use site::{LauncherConfig, SiteOrigin};
pub use url_policy::{NavDecision, PageOrigin, UrlPolicy};
