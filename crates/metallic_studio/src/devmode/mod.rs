//! Developer mode (#14): the switch that shows Studio's tools for people working on Studio and
//! the engine, not on their game.
//!
//! - [`DevMode`] is on by default in builds of the `dev` channel and off on `stable`
//!   ([`build_channel`]); Settings > Developer mode overrides it and the choice is saved in
//!   `settings.toml`.
//! - While it is on, the **Developer** menu lists every action whose id starts with
//!   [`ACTION_PREFIX`]. The built-in ones are the performance overlay ([`overlay`], F3), the log
//!   viewer ([`log_view`]), the Instance inspector ([`inspector`]), feature flags ([`flags`]) and
//!   Report a bug ([`bug_report`]).
//! - **Other tools join by registering a `dev.*` action** (the Profiler panel of #23 and the
//!   script debugger of #22, for example). [`crate::actions::ActionRegistry::register`] then
//!   disables it while developer mode is off and keeps it usable during Play, and the menu lists
//!   it; nothing else needs to change here.

pub mod bug_report;
pub mod flags;
pub mod inspector;
pub mod log_view;
pub mod overlay;

pub use flags::FeatureFlags;

use crate::updater::Channel;

/// Actions whose id starts with this belong to developer mode: off while it is off, usable
/// during Play, listed in the Developer menu.
pub const ACTION_PREFIX: &str = "dev.";

/// The channel this build was made for: `METALLIC_STUDIO_CHANNEL` at compile time (the release
/// script sets it), else the default channel (`dev`).
pub fn build_channel() -> Channel {
    channel_named(option_env!("METALLIC_STUDIO_CHANNEL"))
}

/// The channel called `name` ("stable" or "dev"); anything else is the default.
pub fn channel_named(name: Option<&str>) -> Channel {
    match name {
        Some("stable") => Channel::Stable,
        Some("dev") => Channel::Dev,
        _ => Channel::default(),
    }
}

/// Whether developer mode is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevMode {
    /// True when the developer tools are shown.
    pub enabled: bool,
}

impl DevMode {
    /// The default for builds of `channel`: on for `dev`, off for `stable`.
    pub fn default_for(channel: Channel) -> DevMode {
        DevMode {
            enabled: matches!(channel, Channel::Dev),
        }
    }

    /// The saved choice (`Some` once the creator changed it in Settings), else the channel's
    /// default.
    pub fn resolve(saved: Option<bool>, channel: Channel) -> DevMode {
        DevMode {
            enabled: saved.unwrap_or(DevMode::default_for(channel).enabled),
        }
    }
}

/// What the developer tools show, kept in [`crate::editor::EditorState::dev`] so actions can
/// toggle it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DevState {
    /// Developer mode is on (set by the app from Settings each frame).
    pub enabled: bool,
    /// The performance overlay is shown.
    pub overlay: bool,
    /// The log viewer is open.
    pub log: bool,
    /// The Instance inspector is open.
    pub inspector: bool,
    /// The feature flags panel is open.
    pub flags: bool,
}

/// The developer tools' UI state, kept by the editor window.
#[derive(Default)]
pub struct DevTools {
    /// Frame times and counts for the overlay.
    pub overlay: overlay::Overlay,
    /// The log viewer's filters.
    pub log: log_view::LogView,
    /// The Feature flags panel's "new flag" text.
    pub new_flag: String,
    /// A bug report waiting for its window capture.
    pub report: Option<bug_report::ReportJob>,
    /// The GPU, as the bug report names it ("NVIDIA GeForce RTX 3060 Ti (Vulkan)").
    pub gpu: Option<String>,
}

/// True for an action id that belongs to developer mode.
pub fn is_dev_action(id: &str) -> bool {
    id.starts_with(ACTION_PREFIX)
}
