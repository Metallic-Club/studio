//! Update check, download and apply for Metallic Studio.
//!
//! Releases live on the private GitHub repo `Metallic-Club/studio`, one release per tag
//! `vX.Y.Z-<channel>` (`dev` or `stable`). [`UpdateChecker::check`] lists the releases through the
//! GitHub API with the signed-in user's token and reports the newest version of the chosen
//! channel that is newer than the running one. Downloading and applying go through the Velopack
//! crate, which only works in an installed copy of Studio (see `docs/releasing.md`).
//!
//! A failed check never blocks Studio: use [`UpdateChecker::check_quietly`] for the background
//! check, which logs the failure and answers "no update".
//!
//! HTTP goes through the [`HttpClient`] trait so tests can inject answers. Tokens are wrapped in
//! [`Token`], whose `Debug` and `Display` never show the value.

use std::fmt;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;

/// How often Studio should check for updates after the check on start.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// The private repository that holds the releases.
pub const RELEASE_REPO: &str = "Metallic-Club/studio";

const DEFAULT_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = "metallic-studio-updater";
const RELEASES_PER_PAGE: u32 = 50;
const MAX_RELEASE_PAGES: u32 = 5;

/// Update channel. Each release is tagged `vX.Y.Z-<channel>`.
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

/// A GitHub access token. `Debug` and `Display` print a placeholder, never the value.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Wraps a token value.
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The raw token. Use only to build an `Authorization` header; never log it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted token>")
    }
}

/// Supplies the signed-in user's token when one is needed.
pub trait TokenProvider: Send + Sync {
    /// The current token, or `None` when nobody is signed in.
    fn token(&self) -> Option<Token>;
}

impl TokenProvider for Token {
    fn token(&self) -> Option<Token> {
        Some(self.clone())
    }
}

/// A finished HTTP exchange. Any status code is a response; only transport problems are errors.
///
/// `Debug` omits the body, because the sign-in answers carry tokens.
#[derive(Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body as text.
    pub body: String,
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body", &format_args!("<{} bytes, hidden>", self.body.len()))
            .finish()
    }
}

/// A transport-level HTTP failure (no connection, timeout, bad TLS). Holds a short description
/// that never contains request headers.
#[derive(Clone, Debug)]
pub struct HttpError(String);

impl HttpError {
    /// Creates an error from a description.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HttpError {}

/// The few HTTP calls Studio needs. Implemented by [`UreqClient`]; tests inject their own.
pub trait HttpClient: Send + Sync {
    /// GET `url` with extra `headers`.
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, HttpError>;

    /// POST `form` (`application/x-www-form-urlencoded`) to `url` with extra `headers`.
    fn post_form(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError>;
}

/// [`HttpClient`] backed by `ureq`.
pub struct UreqClient {
    agent: ureq::Agent,
}

impl UreqClient {
    /// Creates a client whose requests give up after `timeout`.
    pub fn new(timeout: Duration) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent }
    }
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(20))
    }
}

fn read_response(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<HttpResponse, HttpError> {
    let mut response = result.map_err(|e| HttpError::new(e.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| HttpError::new(e.to_string()))?;
    Ok(HttpResponse { status, body })
}

/// Percent-encodes one form field (RFC 3986 unreserved characters stay as they are).
fn form_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

impl HttpClient for UreqClient {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        read_response(request.call())
    }

    fn post_form(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let body = form
            .iter()
            .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let mut request = self
            .agent
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        read_response(request.send(body))
    }
}

/// An update that is available for the chosen channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    /// The newest version offered.
    pub version: Version,
    /// True when this release, or one skipped on the way to it, is marked required.
    /// Studio then asks the user to restart instead of only offering it.
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
    /// This copy of Studio was not started from an installed (Velopack) location, so it cannot update itself.
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
                write!(f, "Studio is not installed with the installer: {m}")
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

/// Version of a tag `vX.Y.Z-<channel>` when the tag belongs to `channel`.
fn version_from_tag(tag: &str, channel: Channel) -> Option<Version> {
    let rest = tag.strip_prefix('v')?;
    let version = rest.strip_suffix(&format!("-{}", channel.as_str()))?;
    Version::parse(version).ok()
}

/// Checks GitHub Releases for a newer Studio and runs the Velopack download and apply steps.
pub struct UpdateChecker {
    channel: Channel,
    tokens: Arc<dyn TokenProvider>,
    http: Arc<dyn HttpClient>,
    api_base: String,
    current: Version,
    downloaded: Mutex<Option<(velopack::UpdateManager, velopack::UpdateInfo)>>,
}

impl UpdateChecker {
    /// Creates a checker for `channel` that authenticates with `tokens`. The running version is
    /// this crate's version.
    pub fn new(channel: Channel, tokens: Arc<dyn TokenProvider>) -> Self {
        // CARGO_PKG_VERSION is always valid semver; the fallback only keeps this function total.
        let current = Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(0, 0, 0));
        Self {
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
                "{}/repos/{RELEASE_REPO}/releases?per_page={RELEASES_PER_PAGE}&page={page}",
                self.api_base
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
                .any(|r| version_from_tag(&r.tag_name, self.channel).is_some());
            if has_match || releases.len() < RELEASES_PER_PAGE as usize {
                break;
            }
        }
        Ok(self.pick_update(&releases))
    }

    /// Like [`check`](Self::check), but a failure is logged and answered with `None`, so a bad
    /// network never blocks Studio.
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
                let version = version_from_tag(&r.tag_name, self.channel)?;
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
        let repo_url = format!("https://github.com/{RELEASE_REPO}");
        let source =
            velopack::sources::GithubSource::new(&repo_url, Some(token.expose().to_string()), true);
        let options = velopack::UpdateOptions {
            ExplicitChannel: Some(self.channel.as_str().to_string()),
            ..Default::default()
        };
        velopack::UpdateManager::new(source, Some(options), None)
            .map_err(|e| UpdateError::NotInstalled(e.to_string()))
    }

    /// Downloads the update described by `update` (delta packages when possible) and reports
    /// progress as percent, 0 to 100. Only works in an installed copy of Studio. Does not
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

    /// Applies the downloaded update and restarts Studio. Returns only if that failed (the
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
