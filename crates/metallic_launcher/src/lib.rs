//! Metallic Launcher: the native shell around the Metallic platform website.
//!
//! This crate holds the logic that has no window: the `metallic://` deep links ([`deep_link`]),
//! the site origin and start-up arguments ([`site`]), the rule for what the web view may load
//! ([`url_policy`]), the Launcher's settings file ([`settings`]), the bundled pages
//! ([`local_pages`]), the site check and first page ([`site_check`]) and the events the window
//! accepts ([`app_event`]). The window itself is in the `Metallic` program (`main.rs`, `shell.rs`).
//!
//! Library code returns [`LauncherError`]; only `main.rs` decides what to show on failure.

#![warn(missing_docs)]

pub mod app_event;
pub mod deep_link;
pub mod error;
pub mod local_pages;
pub mod settings;
pub mod site;
pub mod site_check;
pub mod url_policy;

pub use app_event::{AppEvent, EventSink};
pub use deep_link::{DeepLink, DeepLinkError, JoinTicket, PlaceId, SitePath};
pub use error::LauncherError;
pub use local_pages::{LOCAL_CSP, LocalPage, LocalResponse, OfflineReason, local_url, serve};
pub use settings::{LauncherSettings, SettingsPatch, SettingsStore};
pub use site::{LauncherConfig, SiteOrigin};
pub use site_check::{SiteStatus, StartPage, check_site, start_page, user_agent};
pub use url_policy::{NavDecision, PageOrigin, UrlPolicy};
