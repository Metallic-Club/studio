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
use serde::{Deserialize, Serialize};

use crate::http::{HttpClient, TokenProvider, UreqClient};

mod tagged;

/// How often Studio should check for updates after the check on start.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

const DEFAULT_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = "metallic-studio-updater";
const RELEASES_PER_PAGE: u32 = 50;
const MAX_RELEASE_PAGES: u32 = 5;

/// Update channel. Each release is tagged `<prefix>X.Y.Z-<channel>`. Written as `"dev"` or
/// `"stable"` in settings files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Team builds, published for every merge.
    #[default]
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

/// A downloadable file of a release, as the GitHub API describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseAsset {
    /// The version of the release the file belongs to.
    pub version: Version,
    /// The file name, for example `MetallicStudio-dev-Setup.exe`.
    pub name: String,
    /// The API address that serves the file's bytes (with `Accept: application/octet-stream`).
    pub api_url: String,
    /// The SHA-256 of the file, from the `digest` field GitHub computes at upload.
    pub sha256: [u8; 32],
    /// The size in bytes.
    pub size: u64,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

/// Reads `sha256:<64 hex digits>` into bytes.
fn parse_digest(text: &str) -> Option<[u8; 32]> {
    let hex = text.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
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
    let version = Version::parse(version).ok()?;
    // Match the release script's vX.Y.Z-channel grammar. Parseable semver
    // prereleases must not impersonate an installed-version history boundary.
    (version.pre.is_empty() && version.build.is_empty()).then_some(version)
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
        let releases = self.fetch_releases()?;
        Ok(self.pick_update(&releases))
    }

    /// The newest release of this feed and channel that has a file called `file_name`, with the
    /// file's address, size and SHA-256. `Ok(None)` when no release has such a file. A file
    /// whose `digest` is missing or is not `sha256:<64 hex digits>` is refused with
    /// [`UpdateError::BadResponse`], because a download could not be checked.
    pub fn latest_asset(&self, file_name: &str) -> Result<Option<ReleaseAsset>, UpdateError> {
        let releases = self.fetch_releases()?;
        let mut candidates: Vec<(Version, &GithubRelease)> = releases
            .iter()
            .filter(|r| !r.draft)
            .filter_map(|r| Some((version_from_tag(&r.tag_name, &self.feed, self.channel)?, r)))
            .collect();
        candidates.sort_by(|a, b| b.0.cmp(&a.0));
        let api_prefix = format!("{}/", self.api_base);
        for (version, release) in candidates {
            let Some(asset) = release.assets.iter().find(|a| a.name == file_name) else {
                continue;
            };
            let sha256 = asset
                .digest
                .as_deref()
                .and_then(parse_digest)
                .ok_or_else(|| {
                    UpdateError::BadResponse("the release file has no SHA-256 digest".into())
                })?;
            // The token is sent to this address, so it must be on the API host we asked.
            if !asset.url.starts_with(&api_prefix) {
                return Err(UpdateError::BadResponse(
                    "the release file is not served by the GitHub API".into(),
                ));
            }
            return Ok(Some(ReleaseAsset {
                version,
                name: asset.name.clone(),
                api_url: asset.url.clone(),
                sha256,
                size: asset.size,
            }));
        }
        Ok(None)
    }

    fn fetch_releases(&self) -> Result<Vec<GithubRelease>, UpdateError> {
        let token = self.tokens.token().ok_or(UpdateError::Unauthorized)?;
        let authorization = format!("Bearer {}", token.expose());
        let headers = [
            ("Accept", "application/vnd.github+json"),
            ("X-GitHub-Api-Version", "2022-11-28"),
            ("User-Agent", USER_AGENT),
            ("Authorization", authorization.as_str()),
        ];
        // Our release process publishes increasing versions. Keep all pages through
        // the installed-version boundary so skipped required releases remain visible.
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
            let page_releases: Vec<GithubRelease> = serde_json::from_str(&response.body)
                .map_err(|e| UpdateError::BadResponse(e.to_string()))?;
            let reached_installed = page_releases
                .iter()
                .filter(|release| !release.draft)
                .filter_map(|release| version_from_tag(&release.tag_name, &self.feed, self.channel))
                .any(|version| version <= self.current);
            let reached_end = page_releases.len() < RELEASES_PER_PAGE as usize;
            releases.extend(page_releases);
            if reached_installed || reached_end {
                break;
            }
            if page == MAX_RELEASE_PAGES {
                return Err(UpdateError::BadResponse(
                    "release history exceeds the update-check limit; required status is incomplete"
                        .into(),
                ));
            }
        }
        Ok(releases)
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

    fn update_source(
        &self,
        update: &UpdateInfo,
    ) -> Result<tagged::TaggedGithubSource, UpdateError> {
        let token = self.tokens.token().ok_or(UpdateError::Unauthorized)?;
        Ok(tagged::TaggedGithubSource::new(
            self.feed,
            self.channel,
            update.version.clone(),
            token,
            self.api_base.clone(),
            self.http.clone(),
        ))
    }

    fn velopack_manager(
        &self,
        update: &UpdateInfo,
    ) -> Result<velopack::UpdateManager, UpdateError> {
        let source = self.update_source(update)?;
        let options = velopack::UpdateOptions {
            ExplicitChannel: Some(self.feed.velopack_channel(self.channel)),
            ..Default::default()
        };
        velopack::UpdateManager::new(source, Some(options), None)
            .map_err(|e| UpdateError::NotInstalled(e.to_string()))
    }

    /// Downloads the selected version's full package and reports
    /// progress as percent, 0 to 100. Only works in an installed copy of the app. Does not
    /// restart; call [`apply_and_restart`](Self::apply_and_restart) afterwards.
    pub fn download(
        &self,
        update: &UpdateInfo,
        mut progress: impl FnMut(u8),
    ) -> Result<(), UpdateError> {
        let manager = self.velopack_manager(update)?;
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

    /// Starts the Velopack updater, which waits for this process to exit, then applies the
    /// downloaded update. Silent and without a restart when `restart` is false (the update
    /// installed after the app quit); with `restart` the app starts again afterwards. This
    /// does not exit: the caller closes the app normally, and the update happens after that.
    /// Call it once per run.
    pub fn apply_after_exit(&self, restart: bool) -> Result<(), UpdateError> {
        let downloaded = self
            .downloaded
            .lock()
            .map_err(|_| UpdateError::Install("internal lock was poisoned".to_string()))?;
        let Some((manager, info)) = downloaded.as_ref() else {
            return Err(UpdateError::Install(
                "no update has been downloaded".to_string(),
            ));
        };
        manager
            .wait_exit_then_apply_updates(info, !restart, restart, Vec::<String>::new())
            .map_err(|e| UpdateError::Install(e.to_string()))
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
