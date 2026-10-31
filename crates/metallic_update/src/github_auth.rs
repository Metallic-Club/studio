//! GitHub sign-in for the Metallic apps using the device flow of a GitHub App.
//!
//! The app shows a short code, opens `https://github.com/login/device` in the browser, and the
//! user approves there. the app never sees a password. It uses a GitHub App (not an OAuth App):
//! the app has only "Contents: read" on `Metallic-Club/studio`, so its user tokens cannot touch any
//! other repository. No `scope` is sent, because GitHub Apps ignore it.
//!
//! The session (access token, plus refresh token and expiry when the app has user-token expiration
//! on) is kept as one JSON value in the Windows Credential Manager, behind the [`SecretStore`]
//! trait so tests use memory instead. The `keyring` crate names a Windows entry
//! `<account>.<service>` (Studio: `github.MetallicStudio`) and stores it with
//! `CRED_PERSIST_ENTERPRISE`, so it roams with a roaming profile. [`GithubAuth::valid_token`]
//! refreshes an expiring access token on its own.
//!
//! Every network call here blocks (a sign-in polls for up to 15 minutes, a refresh retries), so
//! a UI calls them on a worker thread; [`GithubAuth::device_sign_in_until`] can be cancelled.
//!
//! The app's client ID is not secret. It is committed in `config/github_app_client_id.txt` and
//! compiled in; until the owner registers the app, the file holds a placeholder and
//! [`embedded_client_id`] reports a clear error.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::http::{HttpClient, Token, TokenProvider};
use crate::secret::SecretStore;

/// The placeholder line that stands in for the client ID until the owner registers the app.
pub const CLIENT_ID_PLACEHOLDER: &str = "REPLACE_WITH_CLIENT_ID";

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const REFRESH_GRANT: &str = "refresh_token";
const SLOW_DOWN_STEP_SECONDS: u64 = 5;
const MIN_POLL_SECONDS: u64 = 1;
/// Retries for a transient failure (network error or HTTP 5xx).
const MAX_RETRIES: u32 = 3;
/// An access token that expires within this many seconds is refreshed first.
const REFRESH_MARGIN_SECONDS: u64 = 300;
/// After a failed refresh the current token is kept only while it has at least this long left.
const KEEP_MARGIN_SECONDS: u64 = 60;

/// Why signing in or storing the token failed. No message contains a token.
#[derive(Debug)]
pub enum AuthError {
    /// `config/github_app_client_id.txt` still holds the placeholder (or is empty).
    ClientIdNotConfigured,
    /// GitHub could not be reached.
    Network(String),
    /// GitHub answered with something unexpected.
    BadResponse(String),
    /// The user pressed "Cancel" or refused on the GitHub page.
    Denied,
    /// The code expired before the user approved it.
    Expired,
    /// GitHub reported another OAuth error (the error code is shown).
    Provider(String),
    /// The credential store failed.
    Store(String),
    /// Nobody is signed in (or the session ended), so the step cannot run.
    NotSignedIn,
    /// The server named an address that is not on the expected site; it is not opened.
    UnsafeAddress,
    /// The sign-in was cancelled on this PC before the user finished.
    Cancelled,
    /// The value is larger than the credential store takes ([`crate::secret::MAX_SECRET_BYTES`]).
    TooLarge,
    /// The server sent a token longer than the Launcher accepts.
    TokenTooLong,
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::ClientIdNotConfigured => f.write_str(
                "GitHub sign-in is not set up yet: put the GitHub App's client ID into \
                 config/github_app_client_id.txt (see docs/releasing.md) and rebuild",
            ),
            AuthError::Network(m) => write!(f, "could not reach the server: {m}"),
            AuthError::BadResponse(m) => write!(f, "unexpected answer from the server: {m}"),
            AuthError::Denied => f.write_str("sign-in was refused"),
            AuthError::Expired => f.write_str("the sign-in code expired; start again"),
            AuthError::Provider(code) => write!(f, "sign-in failed: {code}"),
            AuthError::Store(m) => write!(f, "credential store error: {m}"),
            AuthError::NotSignedIn => f.write_str("not signed in"),
            AuthError::UnsafeAddress => {
                f.write_str("the sign-in page is not on the expected site; it was not opened")
            }
            AuthError::Cancelled => f.write_str("the sign-in was cancelled"),
            AuthError::TooLarge => f.write_str("the session is too large for the credential store"),
            AuthError::TokenTooLong => f.write_str("the server sent a token that is too long"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Reads the client ID from the text of `config/github_app_client_id.txt`.
pub fn parse_client_id(text: &str) -> Result<String, AuthError> {
    let id = text.trim();
    if id.is_empty() || id == CLIENT_ID_PLACEHOLDER {
        return Err(AuthError::ClientIdNotConfigured);
    }
    Ok(id.to_string())
}

/// The client ID compiled in from `config/github_app_client_id.txt`.
pub fn embedded_client_id() -> Result<String, AuthError> {
    parse_client_id(include_str!("../../../config/github_app_client_id.txt"))
}

/// A [`TokenProvider`] that gets the signed-in user's token from [`GithubAuth::valid_token`],
/// refreshing it when needed.
pub struct StoredTokenProvider(pub Arc<GithubAuth>);

impl TokenProvider for StoredTokenProvider {
    fn token(&self) -> Option<Token> {
        match self.0.valid_token() {
            Ok(value) => value,
            Err(error) => {
                log::warn!("could not get the stored GitHub token: {error}");
                None
            }
        }
    }
}

/// What the UI does during sign-in: show the code the user must type on GitHub's page.
pub trait DeviceFlowUi {
    /// Called once with the code and the page to open. Show both.
    fn show_code(&mut self, user_code: &str, verification_uri: &str);
}

/// A [`DeviceFlowUi`] that also opens the verification page in the default browser.
pub struct BrowserUi<F: FnMut(&str, &str)> {
    /// Called with `(user_code, verification_uri)` so the UI can display them.
    pub on_code: F,
}

impl<F: FnMut(&str, &str)> DeviceFlowUi for BrowserUi<F> {
    fn show_code(&mut self, user_code: &str, verification_uri: &str) {
        (self.on_code)(user_code, verification_uri);
        if let Err(error) = open_in_browser(verification_uri) {
            log::warn!("could not open the browser: {error}");
        }
    }
}

/// True for an address on `https://github.com/`: the only pages the sign-in opens.
pub fn is_github_page(url: &str) -> bool {
    url.starts_with("https://github.com/")
}

/// Opens a GitHub page in the default browser. Refuses any other address. (The Launcher
/// opens it with its own browser opener after [`is_github_page`].)
pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    if !is_github_page(url) {
        return Err(std::io::Error::other(
            "refusing to open a non-GitHub address",
        ));
    }
    #[cfg(windows)]
    let mut command = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    command.spawn().map(|_| ())
}

#[derive(Deserialize)]
struct DeviceCodeAnswer {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default)]
    interval: u64,
}

/// GitHub's answer to a poll or a refresh. Not `Debug` on purpose: it carries tokens.
#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    refresh_token_expires_in: Option<u64>,
    error: Option<String>,
}

/// What is stored in the credential store. Not `Debug` on purpose.
#[derive(Serialize, Deserialize)]
struct Session {
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    /// Unix seconds; `None` when user-token expiration is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_expires_at: Option<u64>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Session {
    /// Reads a stored value. JSON is a session; a bare string counts (as a session without
    /// expiry) only if it has GitHub's token shape. Anything else is "no session".
    fn parse(raw: &str) -> Option<Session> {
        if let Ok(session) = serde_json::from_str::<Session>(raw) {
            return Some(session);
        }
        let bare = raw.trim();
        let prefixed = ["ghu_", "gho_", "ghp_"].iter().any(|p| bare.starts_with(p));
        let safe = bare.len() > 4 && bare.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        (prefixed && safe).then(|| Session {
            access_token: bare.to_string(),
            refresh_token: None,
            expires_at: None,
            refresh_expires_at: None,
        })
    }

    fn from_answer(access_token: String, answer: &TokenAnswer, now: u64) -> Session {
        Session {
            access_token,
            refresh_token: answer.refresh_token.clone(),
            expires_at: answer.expires_in.map(|s| now + s),
            refresh_expires_at: answer.refresh_token_expires_in.map(|s| now + s),
        }
    }

    fn needs_refresh(&self, now: u64) -> bool {
        self.expires_at
            .is_some_and(|at| at <= now + REFRESH_MARGIN_SECONDS)
    }
}

/// Runs the GitHub device flow and keeps the session in a [`SecretStore`].
pub struct GithubAuth {
    client_id: String,
    http: Arc<dyn HttpClient>,
    store: Arc<dyn SecretStore>,
    sleeping: bool,
    /// Held for the whole of `valid_token`: refresh tokens rotate, so two refreshes at once
    /// would make the second fail and wipe the session the first saved. A finished sign-in
    /// checks its cancel and saves under it, and [`GithubAuth::cancel_sign_in`] and
    /// [`GithubAuth::sign_out`] take it too, so a cancel and a save never interleave.
    refresh_lock: Mutex<()>,
    /// Seconds since 1970 (a test can replace it).
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl GithubAuth {
    /// Creates the sign-in helper for the GitHub App `client_id`.
    pub fn new(client_id: String, http: Arc<dyn HttpClient>, store: Arc<dyn SecretStore>) -> Self {
        Self {
            client_id,
            http,
            store,
            sleeping: true,
            refresh_lock: Mutex::new(()),
            clock: Arc::new(unix_now),
        }
    }

    /// Replaces the clock (tests): seconds since 1970.
    #[doc(hidden)]
    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> u64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    fn lock_session(&self) -> Result<std::sync::MutexGuard<'_, ()>, AuthError> {
        self.refresh_lock
            .lock()
            .map_err(|_| AuthError::Store("session lock was poisoned".to_string()))
    }

    /// Stops a running [`device_sign_in_until`](Self::device_sign_in_until) that was given
    /// `cancel`. Atomic with the sign-in's save: once this returns, that sign-in stores nothing
    /// more (a sign-in that saved before stays signed in; [`sign_out`](Self::sign_out) undoes
    /// it).
    pub fn cancel_sign_in(&self, cancel: &AtomicBool) {
        let _guard = self.lock_session();
        cancel.store(true, Ordering::SeqCst);
    }

    /// Makes polling and retrying skip their waits (tests).
    pub fn without_sleeping(mut self) -> Self {
        self.sleeping = false;
        self
    }

    /// The stored access token as it is, without refreshing. See [`valid_token`](Self::valid_token).
    pub fn stored_token(&self) -> Result<Option<Token>, AuthError> {
        Ok(self
            .store
            .get()?
            .and_then(|raw| Session::parse(&raw))
            .map(|session| Token::new(session.access_token)))
    }

    /// Forgets the stored session.
    pub fn sign_out(&self) -> Result<(), AuthError> {
        let _guard = self.lock_session()?;
        self.store.delete()
    }

    fn save(&self, session: &Session) -> Result<(), AuthError> {
        let raw = serde_json::to_string(session).map_err(|e| AuthError::Store(e.to_string()))?;
        self.store.set(&raw)
    }

    /// The stored access token, refreshed first when it is about to expire. `Ok(None)` means
    /// "sign in (again)": nothing is stored, or it expired and cannot be refreshed.
    pub fn valid_token(&self) -> Result<Option<Token>, AuthError> {
        let _guard = self.lock_session()?;
        // Read the store only after taking the lock: another caller may have just refreshed.
        let Some(session) = self.store.get()?.and_then(|raw| Session::parse(&raw)) else {
            return Ok(None);
        };
        let now = (self.clock)();
        if !session.needs_refresh(now) {
            return Ok(Some(Token::new(session.access_token)));
        }
        let refresh_usable = session.refresh_expires_at.is_none_or(|at| at > now);
        let Some(refresh_token) = session.refresh_token.filter(|_| refresh_usable) else {
            log::info!("stored GitHub token expired and cannot be refreshed; sign in again");
            return Ok(None);
        };
        let refreshed = self.post_retrying(
            ACCESS_TOKEN_URL,
            &[
                ("client_id", self.client_id.as_str()),
                ("grant_type", REFRESH_GRANT),
                ("refresh_token", refresh_token.as_str()),
            ],
            None,
        );
        let body = match refreshed {
            Ok(body) => body,
            // A network blip (or a 5xx, which `post` reports as one) in the refresh window is
            // not a reason to sign in again: the current token still works for a while (#206).
            // The clock is read again after the retries, and the token must have a minute left.
            Err(AuthError::Network(message))
                if session
                    .expires_at
                    .is_some_and(|at| at > (self.clock)().saturating_add(KEEP_MARGIN_SECONDS)) =>
            {
                log::warn!("could not refresh the GitHub token ({message}); using the current one");
                return Ok(Some(Token::new(session.access_token)));
            }
            Err(error) => return Err(error),
        };
        let answer: TokenAnswer =
            serde_json::from_str(&body).map_err(|e| AuthError::BadResponse(e.to_string()))?;
        if let Some(code) = answer.error.as_deref() {
            // The refresh token is no good any more (for example bad_refresh_token).
            self.store.delete()?;
            return Err(AuthError::Provider(code.to_string()));
        }
        let Some(access_token) = answer.access_token.clone() else {
            return Err(AuthError::BadResponse(
                "neither a token nor an error".to_string(),
            ));
        };
        self.save(&Session::from_answer(
            access_token.clone(),
            &answer,
            (self.clock)(),
        ))?;
        log::info!("GitHub token refreshed");
        Ok(Some(Token::new(access_token)))
    }

    fn post(&self, url: &str, form: &[(&str, &str)]) -> Result<String, AuthError> {
        let response = self
            .http
            .post_form(url, &[("Accept", "application/json")], form)
            .map_err(|e| AuthError::Network(e.to_string()))?;
        match response.status {
            200..=299 => Ok(response.body),
            // Transient server trouble: reported as a network error so it is retried.
            500..=599 => Err(AuthError::Network(format!(
                "HTTP status {}",
                response.status
            ))),
            other => Err(AuthError::BadResponse(format!("HTTP status {other}"))),
        }
    }

    /// Like `post`, but retries up to `MAX_RETRIES` times on network errors and HTTP 5xx,
    /// waiting 1, 2, then 4 seconds. With `cancel`, a cancel ends it with
    /// [`AuthError::Cancelled`] before a retry and during a wait.
    fn post_retrying(
        &self,
        url: &str,
        form: &[(&str, &str)],
        cancel: Option<&AtomicBool>,
    ) -> Result<String, AuthError> {
        let cancelled = || cancel.is_some_and(|c| c.load(Ordering::SeqCst));
        let mut retries = 0;
        loop {
            if cancelled() {
                return Err(AuthError::Cancelled);
            }
            match self.post(url, form) {
                Err(AuthError::Network(message)) if retries < MAX_RETRIES => {
                    log::warn!("GitHub request failed ({message}); retrying");
                    self.wait(Duration::from_secs(1 << retries), cancel);
                    retries += 1;
                }
                other => return other,
            }
        }
    }

    /// Waits `time` (not at all without sleeping), in short slices so a cancel is noticed.
    fn wait(&self, time: Duration, cancel: Option<&AtomicBool>) {
        if !self.sleeping {
            return;
        }
        let until = Instant::now() + time;
        while !cancel.is_some_and(|c| c.load(Ordering::SeqCst)) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Signs in with GitHub's device flow: asks for a code, hands it to `ui`, then polls until the
    /// user approves. Blocks the calling thread, so run it off the UI thread. On success the
    /// session is stored and the access token returned.
    pub fn device_sign_in(&self, ui: &mut dyn DeviceFlowUi) -> Result<Token, AuthError> {
        self.device_sign_in_until(ui, &AtomicBool::new(false))
    }

    /// [`device_sign_in`](Self::device_sign_in) that ends with [`AuthError::Cancelled`] soon
    /// after `cancel` is set (checked before each request, retry and poll, and while waiting).
    /// Cancel with [`cancel_sign_in`](Self::cancel_sign_in): the cancel check and the save
    /// share one lock, so a token that arrives after the cancel is never stored. It is dropped,
    /// not revoked: revoking a GitHub App user token needs the app's client secret, which no
    /// client has; the token expires on its own.
    pub fn device_sign_in_until(
        &self,
        ui: &mut dyn DeviceFlowUi,
        cancel: &AtomicBool,
    ) -> Result<Token, AuthError> {
        let cancelled = || cancel.load(Ordering::SeqCst);
        let body = self.post_retrying(
            DEVICE_CODE_URL,
            &[("client_id", self.client_id.as_str())],
            Some(cancel),
        )?;
        let code: DeviceCodeAnswer =
            serde_json::from_str(&body).map_err(|e| AuthError::BadResponse(e.to_string()))?;
        log::info!("GitHub device sign-in started; waiting for approval");
        ui.show_code(&code.user_code, &code.verification_uri);

        let started = Instant::now();
        let lifetime = Duration::from_secs(code.expires_in);
        let mut interval = code.interval.max(MIN_POLL_SECONDS);
        loop {
            if cancelled() {
                return Err(AuthError::Cancelled);
            }
            if started.elapsed() >= lifetime {
                return Err(AuthError::Expired);
            }
            self.wait(Duration::from_secs(interval), Some(cancel));
            if cancelled() {
                return Err(AuthError::Cancelled);
            }
            let body = self.post_retrying(
                ACCESS_TOKEN_URL,
                &[
                    ("client_id", self.client_id.as_str()),
                    ("device_code", code.device_code.as_str()),
                    ("grant_type", DEVICE_GRANT),
                ],
                Some(cancel),
            )?;
            let answer: TokenAnswer =
                serde_json::from_str(&body).map_err(|e| AuthError::BadResponse(e.to_string()))?;
            if let Some(value) = answer.access_token.clone() {
                // One lock with `cancel_sign_in`: cancelled first and nothing is saved, or saved
                // first and the cancel comes after a finished sign-in.
                let _guard = self.lock_session()?;
                if cancelled() {
                    log::info!("a GitHub token arrived after the sign-in was cancelled; dropped");
                    return Err(AuthError::Cancelled);
                }
                self.save(&Session::from_answer(
                    value.clone(),
                    &answer,
                    (self.clock)(),
                ))?;
                log::info!("GitHub sign-in finished; session stored");
                return Ok(Token::new(value));
            }
            match answer.error.as_deref() {
                Some("authorization_pending") => {}
                Some("slow_down") => interval += SLOW_DOWN_STEP_SECONDS,
                Some("expired_token") => return Err(AuthError::Expired),
                Some("access_denied") => return Err(AuthError::Denied),
                Some(other) => return Err(AuthError::Provider(other.to_string())),
                None => {
                    return Err(AuthError::BadResponse(
                        "neither a token nor an error".to_string(),
                    ));
                }
            }
        }
    }
}
