//! Metallic Launcher: the native shell around the Metallic platform website.
//!
//! This crate holds the logic that has no window: the `metallic://` deep links ([`deep_link`]),
//! the site origin and start-up arguments ([`site`]), the rule for what the web view may load
//! ([`url_policy`]), the Launcher's settings file ([`settings`]), the bundled pages
//! ([`local_pages`]), the site check and first page ([`site_check`]), the events the window
//! accepts ([`app_event`]), the checked bridge from pages to the Launcher
//! ([`bridge`]), the hand-over between Launchers ([`single_instance`]), the `metallic://`
//! registration ([`protocol`]), starting the Player and Studio ([`launch`]) and the tray
//! rules ([`tray`]), installing Studio ([`install`]) and the Launcher's own update
//! ([`self_update`]). The window itself is in the `Metallic` program (`main.rs`, `shell.rs`).
//!
//! Library code returns [`LauncherError`]; only `main.rs` decides what to show on failure.

#![warn(missing_docs)]

pub mod app_event;
pub mod bridge;
pub mod deep_link;
pub mod error;
pub mod install;
pub mod launch;
pub mod local_pages;
pub mod protocol;
pub mod self_update;
pub mod settings;
pub mod single_instance;
pub mod site;
pub mod site_check;
pub mod tray;
pub mod url_policy;

pub use app_event::{AppEvent, EventSink};
pub use bridge::{
    BRIDGE_VERSION, Bridge, BridgeCommand, BridgeErrorCode, BridgeEvent, BridgeHost, LauncherInfo,
    MAX_IN_FLIGHT, MAX_MESSAGE_BYTES, PendingReply, allowed,
};
pub use deep_link::{DeepLink, DeepLinkError, JoinTicket, PlaceId, SitePath};
pub use error::LauncherError;
pub use install::{
    AssetSource, GithubAssets, InstallStatus, Installer, LauncherUpdateState, ProductStatus,
    downloads_dir_for, github_sign_in, launcher_error, player_status, read_installed_version,
    studio_root_for, studio_setup_name,
};
pub use launch::{
    ProductPaths, Spawner, product_paths, product_paths_for, run_with_limit, start_player,
    start_studio,
};
pub use local_pages::{LOCAL_CSP, LocalPage, LocalResponse, OfflineReason, local_url, serve};
pub use protocol::{
    MemoryRegistry, PROTOCOL_KEY, UriRegistry, is_installed_copy, is_registered_to, register,
    unregister,
};
pub use self_update::{
    PlayerWatcher, ProcessLister, ProcessTracker, RestartError, SelfUpdater, UpdateSource,
};
pub use settings::{LauncherSettings, SettingsPatch, SettingsStore};
pub use single_instance::{
    Acquired, HandoffListener, HandoffMessage, HandoffSender, MAX_HANDOFF_BYTES, decode, encode,
};
pub use site::{LauncherConfig, SiteOrigin};
pub use site_check::{SiteStatus, StartPage, check_site, start_page, user_agent};
pub use tray::{CloseAction, TrayAction, on_close_requested, tray_menu};
pub use url_policy::{NavDecision, PageOrigin, UrlPolicy};
