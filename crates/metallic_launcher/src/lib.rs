//! Window-free Metallic Launcher policy and configuration.
//!
//! Protocol input is validated and canonically serialized before later launch code can use it.
//! Origin checks and navigation decisions grant no capabilities to lookalikes or external pages.
//! This crate creates no window, launches no process and stores no credentials.
#![warn(missing_docs)]

pub mod deep_link;
pub mod error;
pub mod settings;
pub mod site;
pub mod url_policy;

pub use deep_link::{DeepLink, DeepLinkError, JoinTicket, MAX_URI_LEN, PlaceId, SCHEME, SitePath};
pub use error::LauncherError;
pub use settings::{LauncherSettings, SettingsPatch, SettingsStore};
pub use site::{LauncherConfig, SiteOrigin, embedded_site_origin, is_local_page};
pub use url_policy::{NavDecision, PageOrigin, UrlPolicy};
