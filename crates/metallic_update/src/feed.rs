//! Update check, download and apply, for every Metallic app that ships through GitHub Releases.
//!
//! Releases live on the private GitHub repo named by a [`ReleaseFeed`]: one release per tag
//! `<tag_prefix>X.Y.Z-<channel>` (`dev` or `stable`). [`UpdateChecker::check`] lists the releases
//! through the GitHub API with the signed-in user's token and reports the newest version of the
//! chosen channel that is newer than the running one. Downloading and applying go through the
//! Velopack crate, which only works in an installed copy of the app (see `docs/releasing.md`).
//!
//! A failed check never blocks the app: use [`UpdateChecker::check_quietly`] for the background
//! check, which logs the failure and answers "no update".

use std::fmt;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;

use crate::http::{HttpClient, TokenProvider, UreqClient};

/// How often Studio should check for updates after the check on start.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

const DEFAULT_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = "metallic-studio-updater";
const RELEASES_PER_PAGE: u32 = 50;
const MAX_RELEASE_PAGES: u32 = 5;

/// Update channel. Each release is tagged `<prefix>X.Y.Z-<channel>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Team builds, published for every merge.
    Dev,
    /// Releases for everyone else, published later.
    Stable,
}

impl Channel {
    /// The channel name used in tags and by `vpk --channel`.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Dev => "dev",
            Channel::Stable => "stable",
        }
    }
}

/// Where an app's releases are published and how their tags and Velopack channels are named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseFeed {
    /// The GitHub repository, `owner/name`.
    pub repo: &'static str,
    /// Text before the version in a tag (`v` gives `v0.2.0-dev`).
    pub tag_prefix: &'static str,
    /// Text before the channel name in the Velopack channel (`launcher-` gives `launcher-dev`).
    pub velopack_channel_prefix: &'static str,
}

/// The Metallic Studio releases: tags `vX.Y.Z-<channel>`, Velopack channel `<channel>`.
pub const STUDIO_FEED: ReleaseFeed = ReleaseFeed {
    repo: "Metallic-Club/studio",
    tag_prefix: "v",
    velopack_channel_prefix: "",
};

/// The Metallic Launcher releases, in the same repository: tags `launcher-vX.Y.Z-<channel>`,
/// Velopack channel `launcher-<channel>`.
pub const LAUNCHER_FEED: ReleaseFeed = ReleaseFeed {
    repo: "Metallic-Club/studio",
    tag_prefix: "launcher-v",
    velopack_channel_prefix: "launcher-",
};

impl ReleaseFeed {
    /// The Velopack channel name for `channel`, for example `launcher-dev`.
    pub fn velopack_channel(&self, channel: Channel) -> String {
        format!("{}{}", self.velopack_channel_prefix, channel.as_str())
    }
}

/// An update that is available for the chosen channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    /// The newest version offered.
    pub version: Version,
    /// True when this release, or one skipped on the way to it, is marked required.
    /// the app then asks the user to restart instead of only offering it.
    pub required: bool,
}

/// Why an update step failed. None of the messages contain the token.
#[derive(Debug)]
pub enum UpdateError {
    /// The server could not be reached or the connection broke.
    Network(String),
    /// Nobody is signed in, or GitHub refused the token (HTTP 401, 403 or 404 on a private repo).
    Unauthorized,
    /// GitHub answered with something that is not a release list.
    BadResponse(String),
    /// This copy of the app was not started from an installed (Velopack) location, so it cannot update itself.
    NotInstalled(String),
    /// Downloading or applying the update failed.
    Install(String),
    /// A setting is not allowed (for example a non-HTTPS API address).
    BadConfig(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Network(m) => write!(f, "could not reach GitHub: {m}"),
            UpdateError::Unauthorized => {
                f.write_str("GitHub refused access to the releases; sign in again")
            }
            UpdateError::BadResponse(m) => write!(f, "unexpected answer from GitHub: {m}"),
            UpdateError::NotInstalled(m) => {
                write!(f, "the app is not installed with the installer: {m}")
            }
            UpdateError::Install(m) => write!(f, "update failed: {m}"),
            UpdateError::BadConfig(m) => write!(f, "bad update setting: {m}"),
        }
    }
}

impl std::error::Error for UpdateError {}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    body: Option<String>,
}

/// True when a release body contains a line `required: true` (any case, surrounding space ignored).
fn body_marks_required(body: &str) -> bool {
    body.lines()
        .any(|line| line.trim().eq_ignore_ascii_case("required: true"))
}

/// Version of a tag `<tag_prefix>X.Y.Z-<channel>` when the tag belongs to `feed` and `channel`.
fn version_from_tag(tag: &str, feed: &ReleaseFeed, channel: Channel) -> Option<Version> {
    let rest = tag.strip_prefix(feed.tag_prefix)?;
    let version = rest.strip_suffix(&format!("-{}", channel.as_str()))?;
    Version::parse(version).ok()
}

/// Checks GitHub Releases for a newer version of an app and runs the Velopack download and apply steps.
pub struct UpdateChecker {
    feed: ReleaseFeed,
    channel: Channel,
    tokens: Arc<dyn TokenProvider>,
    http: Arc<dyn HttpClient>,
    api_base: String,
    current: Version,
    downloaded: Mutex<Option<(velopack::UpdateManager, velopack::UpdateInfo)>>,
}

impl UpdateChecker {
    /// Creates a checker for `feed` and `channel` that authenticates with `tokens`. The running
    /// version is this crate's version, which is the workspace version every app shares.
    pub fn new(feed: ReleaseFeed, channel: Channel, tokens: Arc<dyn TokenProvider>) -> Self {
        // CARGO_PKG_VERSION is always valid semver; the fallback only keeps this function total.
        let current = Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(0, 0, 0));
        Self {
            feed,
            channel,
            tokens,
            http: Arc::new(UreqClient::default()),
            api_base: DEFAULT_API_BASE.to_string(),
            current,
            downloaded: Mutex::new(None),
        }
    }

    /// Replaces the HTTP client (tests).
    pub fn with_http(mut self, http: Arc<dyn HttpClient>) -> Self {
        self.http = http;
        self
    }

    /// Replaces the GitHub API base URL. Only `https://` is accepted, or `http://` to `127.0.0.1`
    /// or `localhost` (tests), so the token can never travel unencrypted to another host.
    pub fn with_api_base(mut self, api_base: String) -> Result<Self, UpdateError> {
        let (secure, rest) = if let Some(rest) = api_base.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = api_base.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(UpdateError::BadConfig(
                "API address must start with https://".into(),
            ));
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.is_empty() || authority.contains('@') {
            return Err(UpdateError::BadConfig(
                "API address has no usable host".into(),
            ));
        }
        let host = authority.split(':').next().unwrap_or("");
        if !secure && host != "127.0.0.1" && host != "localhost" {
            return Err(UpdateError::BadConfig(
                "plain http is only allowed to 127.0.0.1 or localhost".into(),
            ));
        }
        self.api_base = api_base.trim_end_matches('/').to_string();
        Ok(self)
    }

    /// Replaces the version that counts as "running" (tests).
    pub fn with_current_version(mut self, current: Version) -> Self {
        self.current = current;
        self
    }

    /// Asks GitHub for the newest release of the channel. `Ok(None)` means "up to date".
    pub fn check(&self) -> Result<Option<UpdateInfo>, UpdateError> {
        let token = self.tokens.token().ok_or(UpdateError::Unauthorized)?;
        let authorization = format!("Bearer {}", token.expose());
        let headers = [
            ("Accept", "application/vnd.github+json"),
            ("X-GitHub-Api-Version", "2022-11-28"),
            ("User-Agent", USER_AGENT),
            ("Authorization", authorization.as_str()),
        ];
        // Newest releases come first. Read pages until one holds a release of our channel
        // (the newest of the channel is then on it), the list ends, or five pages are read.
        let mut releases: Vec<GithubRelease> = Vec::new();
        for page in 1..=MAX_RELEASE_PAGES {
            let url = format!(
                "{}/repos/{}/releases?per_page={RELEASES_PER_PAGE}&page={page}",
                self.api_base, self.feed.repo
            );
            log::debug!("checking {url} for {} updates", self.channel.as_str());
            let response = self
                .http
                .get(&url, &headers)
                .map_err(|e| UpdateError::Network(e.to_string()))?;
            match response.status {
                200..=299 => {}
                401 | 403 | 404 => return Err(UpdateError::Unauthorized),
                other => return Err(UpdateError::BadResponse(format!("HTTP status {other}"))),
            }
            releases = serde_json::from_str(&response.body)
                .map_err(|e| UpdateError::BadResponse(e.to_string()))?;
            let has_match = releases
                .iter()
                .any(|r| version_from_tag(&r.tag_name, &self.feed, self.channel).is_some());
            if has_match || releases.len() < RELEASES_PER_PAGE as usize {
                break;
            }
        }
        Ok(self.pick_update(&releases))
    }

    /// Like [`check`](Self::check), but a failure is logged and answered with `None`, so a bad
    /// network never blocks the app.
    pub fn check_quietly(&self) -> Option<UpdateInfo> {
        match self.check() {
            Ok(update) => update,
            Err(error) => {
                log::warn!("update check failed: {error}");
                None
            }
        }
    }

    fn pick_update(&self, releases: &[GithubRelease]) -> Option<UpdateInfo> {
        let newer: Vec<(Version, bool)> = releases
            .iter()
            .filter(|r| !r.draft)
            .filter_map(|r| {
                let version = version_from_tag(&r.tag_name, &self.feed, self.channel)?;
                let required = r.body.as_deref().is_some_and(body_marks_required);
                Some((version, required))
            })
            .filter(|(version, _)| *version > self.current)
            .collect();
        let version = newer.iter().map(|(v, _)| v).max()?.clone();
        let required = newer.iter().any(|(_, required)| *required);
        log::info!("update available: {version} (required: {required})");
        Some(UpdateInfo { version, required })
    }

    fn velopack_manager(&self) -> Result<velopack::UpdateManager, UpdateError> {
        let token = self.tokens.token().ok_or(UpdateError::Unauthorized)?;
        let repo_url = format!("https://github.com/{}", self.feed.repo);
        let source =
            velopack::sources::GithubSource::new(&repo_url, Some(token.expose().to_string()), true);
        let options = velopack::UpdateOptions {
            ExplicitChannel: Some(self.feed.velopack_channel(self.channel)),
            ..Default::default()
        };
        velopack::UpdateManager::new(source, Some(options), None)
            .map_err(|e| UpdateError::NotInstalled(e.to_string()))
    }

    /// Downloads the update described by `update` (delta packages when possible) and reports
    /// progress as percent, 0 to 100. Only works in an installed copy of the app. Does not
    /// restart; call [`apply_and_restart`](Self::apply_and_restart) afterwards.
    pub fn download(
        &self,
        update: &UpdateInfo,
        mut progress: impl FnMut(u8),
    ) -> Result<(), UpdateError> {
        let manager = self.velopack_manager()?;
        let found = manager
            .check_for_updates()
            .map_err(|e| UpdateError::Install(e.to_string()))?;
        let velopack::UpdateCheck::UpdateAvailable(velopack_info) = found else {
            return Err(UpdateError::Install(
                "the release feed no longer offers an update".to_string(),
            ));
        };
        let offered = Version::parse(&velopack_info.TargetFullRelease.Version)
            .map_err(|e| UpdateError::BadResponse(e.to_string()))?;
        if offered != update.version {
            return Err(UpdateError::Install(format!(
                "the feed offers {offered}, not {}; check again",
                update.version
            )));
        }
        let (sender, receiver) = mpsc::channel::<i16>();
        let result = std::thread::scope(|scope| {
            let worker_manager = manager.clone();
            let worker_info = &velopack_info;
            let worker =
                scope.spawn(move || worker_manager.download_updates(worker_info, Some(sender)));
            for percent in receiver {
                progress(percent.clamp(0, 100) as u8);
            }
            worker.join()
        });
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(UpdateError::Install(e.to_string())),
            Err(_) => return Err(UpdateError::Install("download thread panicked".to_string())),
        }
        *self
            .downloaded
            .lock()
            .map_err(|_| UpdateError::Install("internal lock was poisoned".to_string()))? =
            Some((manager, *velopack_info));
        Ok(())
    }

    /// Applies the downloaded update and restarts the app. Returns only if that failed (the
    /// process exits on success), so save any work first.
    pub fn apply_and_restart(&self) -> Result<(), UpdateError> {
        let downloaded = self
            .downloaded
            .lock()
            .map_err(|_| UpdateError::Install("internal lock was poisoned".to_string()))?
            .take();
        let Some((manager, info)) = downloaded else {
            return Err(UpdateError::Install(
                "no update has been downloaded".to_string(),
            ));
        };
        manager
            .apply_updates_and_restart(&info)
            .map_err(|e| UpdateError::Install(e.to_string()))
    }
}
