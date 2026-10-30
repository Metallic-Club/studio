//! Signing in to the Metallic platform with the OAuth device flow, and handing the session to
//! the web view.
//!
//! The user signs in in the system browser (the Launcher never sees a password): the Launcher asks
//! the platform for a short code ([`PlatformAuth::start`]), opens the verification page, and polls
//! until the user has approved ([`PlatformAuth::poll_until_done`]).
//!
//! ## What is stored
//!
//! Only what is needed to resume: the refresh token, as UTF-8 bytes in the Windows Credential
//! Manager (`Metallic` / `platform`) behind [`SecretStore`], never in `settings.toml`. The access
//! token stays in memory and is fetched again from the refresh token after a restart. A value over
//! the Credential Manager's 2560-byte limit is refused with [`AuthError::TooLarge`], and a token
//! over 256 bytes with [`AuthError::TokenTooLong`] (the platform's tokens are 47 bytes).
//!
//! Refresh tokens rotate on every use, and reusing an old one revokes the whole family. So
//! refreshes run one at a time (a caller that arrives meanwhile waits and gets the result), the new
//! refresh token is stored before the new session is used, a refresh is never repeated with the old
//! token once the server answered, and a lost answer is retried once with the same token.
//!
//! ## The web view
//!
//! The web view is signed in separately. With its access token the Launcher asks for a one-time
//! code ([`PlatformAuth::web_session_code`], 60 s, single use) and navigates to
//! `<site>/auth/launcher-handoff?code=<code>`; the site redeems the code itself and sets its own
//! cookie. That code is the only secret that ever appears in an address, so it travels as a
//! [`Token`] and a [`HandoffUrl`], both of which print a placeholder instead of their value.
//!
//! [`platform_sign_in`] ties this together for the bridge: it answers at once and a worker does the
//! network calls, reporting as `signIn` events (`pending` with the code to type, then `signedIn`,
//! `denied`, `expired` or `failed`). A signed-in Launcher skips the browser step and only signs the
//! web view in. A second call while one runs gets the running flow's code again.
//! [`sign_out_in_background`] forgets the session at once and revokes it on a worker.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use metallic_update::{AuthError, HttpClient, HttpResponse, MAX_SECRET_BYTES, SecretStore, Token};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::app_event::{AppEvent, EventSink};
use crate::bridge::BridgeEvent;
use crate::error::LauncherError;
use crate::site::SiteOrigin;

/// The OAuth client id of the Launcher (public, not a secret).
pub const CLIENT_ID: &str = "metallic-launcher";
/// The scopes the Launcher asks for: the profile (needed for the web-session handoff) and play.
pub const SCOPES: &str = "profile play";
/// Credential Manager entry: service name.
pub const KEYRING_SERVICE: &str = "Metallic";
/// Credential Manager entry: account name.
pub const KEYRING_ACCOUNT: &str = "platform";
/// The longest token accepted from the server, in bytes (the platform rejects longer ones too).
pub const MAX_TOKEN_BYTES: usize = 256;

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const REFRESH_GRANT: &str = "refresh_token";
const SLOW_DOWN_STEP_SECONDS: u64 = 5;
/// RFC 8628: the poll interval when the server names none.
const DEFAULT_POLL_SECONDS: u64 = 5;
const MIN_POLL_SECONDS: u64 = 1;
const MAX_POLL_SECONDS: u64 = 60;
/// The longest a device code is believed to live.
const MAX_CODE_LIFETIME_SECONDS: u64 = 1800;
/// The longest an access token is believed to live.
const MAX_TOKEN_LIFETIME_SECONDS: u64 = 24 * 3600;
/// Retries while polling for approval (transient failures).
const POLL_RETRIES: u32 = 3;
/// A refresh whose answer was lost is tried once more with the same token.
const REFRESH_RETRIES: u32 = 1;
/// The web-session request is made at most three times.
const HANDOFF_RETRIES: u32 = 2;
/// An access token that expires within this many seconds is refreshed first.
const REFRESH_MARGIN_SECONDS: u64 = 300;
/// The longest web-session code accepted (it goes into an address).
const MAX_CODE_LEN: usize = 200;

/// What the platform sends back for a new sign-in. `Debug` hides the device code.
#[derive(Clone, Debug)]
pub struct DeviceCode {
    /// The short code the user types on the verification page.
    pub user_code: String,
    /// The page where the user types the code.
    pub verification_uri: String,
    /// The same page with the code filled in, when the platform offers it.
    pub verification_uri_complete: Option<String>,
    /// Seconds between polls (1 to 60; 5 when the server names none).
    pub interval: u64,
    /// Seconds until the code expires (at most 30 minutes).
    pub expires_in: u64,
    device_code: Token,
}

#[derive(Deserialize)]
struct DeviceCodeAnswer {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: u64,
    #[serde(default)]
    interval: u64,
}

/// The platform's answer to a poll or a refresh. Not `Debug`: it carries tokens.
#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct WebSessionAnswer {
    code: String,
}

/// What is stored in the credential store: just what is needed to resume. Not `Debug`.
#[derive(Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    version: u32,
    refresh_token: String,
}

/// The access token, kept in memory only. Not `Debug`.
struct Cached {
    access_token: String,
    /// Unix seconds; `None` when the platform sent no lifetime.
    expires_at: Option<u64>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Accepts a token of at most [`MAX_TOKEN_BYTES`].
fn capped(token: String) -> Result<String, AuthError> {
    if token.len() > MAX_TOKEN_BYTES {
        Err(AuthError::TokenTooLong)
    } else {
        Ok(token)
    }
}

/// A provider error code for the log and the error value: lower-case letters and `_` only,
/// 64 at most; anything else is shown as `unknown` so a server cannot write into the log.
fn provider(code: &str) -> AuthError {
    let clean = !code.is_empty()
        && code.len() <= 64
        && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    AuthError::Provider(if clean { code } else { "unknown" }.to_string())
}

/// Locks a mutex, carrying on after a panic elsewhere (the data is plain state).
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The address that signs the web view in: `<site>/auth/launcher-handoff?code=<code>`. `Debug`
/// prints a placeholder, because the address holds a one-time secret.
#[derive(Clone)]
pub struct HandoffUrl(Url);

impl HandoffUrl {
    /// The address. Load it in the web view; never log it.
    pub fn expose(&self) -> &Url {
        &self.0
    }
}

impl fmt::Debug for HandoffUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HandoffUrl(<redacted>)")
    }
}

/// A session that was forgotten locally and still has to be revoked on the platform.
/// `Debug` hides the token.
pub struct Revocation(Option<Token>);

impl Revocation {
    /// True when there is a refresh token to revoke.
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }
}

impl fmt::Debug for Revocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Revocation(<redacted>)")
    }
}

/// Waits between polls and retries. Tests replace it.
type Sleeper = Arc<dyn Fn(Duration) + Send + Sync>;

/// The Metallic platform sign-in. The platform API is on the site's origin (`<origin>/v1/...`).
pub struct PlatformAuth {
    site: SiteOrigin,
    http: Arc<dyn HttpClient>,
    store: Arc<dyn SecretStore>,
    sleeper: Option<Sleeper>,
    /// Held for the whole of a refresh: refresh tokens rotate, so two refreshes at once would
    /// make the second fail and (the platform treats reuse as theft) end the session.
    refresh_lock: Mutex<()>,
    /// The in-memory access token. Held whenever the store is read or changed (never across
    /// an HTTP request), so a sign-out cannot fall between a check and a save.
    cache: Mutex<Option<Cached>>,
}

impl fmt::Debug for PlatformAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformAuth")
            .field("site", &self.site)
            .finish_non_exhaustive()
    }
}

impl PlatformAuth {
    /// Creates the sign-in helper for the platform at `site`.
    pub fn new(site: SiteOrigin, http: Arc<dyn HttpClient>, store: Arc<dyn SecretStore>) -> Self {
        Self {
            site,
            http,
            store,
            sleeper: None,
            refresh_lock: Mutex::new(()),
            cache: Mutex::new(None),
        }
    }

    /// Replaces the waiting between polls and retries (tests record the waits and skip them).
    pub fn with_sleeper(mut self, sleeper: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleeper = Some(Arc::new(sleeper));
        self
    }

    fn api(&self, path: &str) -> String {
        format!("{}{path}", self.site.origin_text())
    }

    /// Waits `time`, in short slices so a cancel is noticed.
    fn wait(&self, time: Duration, cancel: &AtomicBool) {
        if let Some(sleeper) = &self.sleeper {
            sleeper(time);
            return;
        }
        let slice = Duration::from_millis(200);
        let mut left = time;
        while !left.is_zero() && !cancel.load(Ordering::SeqCst) {
            let step = left.min(slice);
            std::thread::sleep(step);
            left -= step;
        }
    }

    /// One POST. HTTP 5xx and transport errors are `Network` (retryable); any other status is
    /// returned for the caller to read, because OAuth errors arrive as HTTP 400 with a JSON body.
    fn post(
        &self,
        path: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, AuthError> {
        let mut all = vec![("Accept", "application/json")];
        all.extend_from_slice(headers);
        let response = self
            .http
            .post_form(&self.api(path), &all, form)
            .map_err(|e| AuthError::Network(e.to_string()))?;
        match response.status {
            500..=599 => Err(AuthError::Network(format!(
                "HTTP status {}",
                response.status
            ))),
            _ => Ok(response),
        }
    }

    /// Like [`post`](Self::post), retrying a transient failure up to `retries` times after
    /// waiting 1, 2, then 4 seconds.
    fn post_retrying(
        &self,
        path: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
        retries: u32,
        cancel: &AtomicBool,
    ) -> Result<HttpResponse, AuthError> {
        let mut tries = 0;
        loop {
            match self.post(path, headers, form) {
                Err(AuthError::Network(message)) if tries < retries => {
                    log::warn!("platform request failed ({message}); retrying");
                    self.wait(Duration::from_secs(1 << tries), cancel);
                    if cancel.load(Ordering::SeqCst) {
                        return Err(AuthError::Cancelled);
                    }
                    tries += 1;
                }
                other => return other,
            }
        }
    }

    /// Asks the platform for a device code (one attempt, no retries).
    pub fn start(&self) -> Result<DeviceCode, AuthError> {
        let response = self.post(
            "/v1/oauth/device/code",
            &[],
            &[("client_id", CLIENT_ID), ("scope", SCOPES)],
        )?;
        if !(200..=299).contains(&response.status) {
            return Err(AuthError::BadResponse(format!(
                "HTTP status {}",
                response.status
            )));
        }
        let answer: DeviceCodeAnswer = serde_json::from_str(&response.body)
            .map_err(|e| AuthError::BadResponse(e.to_string()))?;
        let interval = match answer.interval {
            0 => DEFAULT_POLL_SECONDS,
            n => n.clamp(MIN_POLL_SECONDS, MAX_POLL_SECONDS),
        };
        Ok(DeviceCode {
            user_code: answer.user_code,
            verification_uri: answer.verification_uri,
            verification_uri_complete: answer.verification_uri_complete,
            interval,
            expires_in: answer.expires_in.min(MAX_CODE_LIFETIME_SECONDS),
            device_code: Token::new(answer.device_code),
        })
    }

    /// The page to open in the system browser: `verification_uri_complete` when there is one,
    /// else `verification_uri`. It must be `https` on the site's own origin; anything else is
    /// [`AuthError::UnsafeAddress`] and must not be opened.
    pub fn browser_address(&self, code: &DeviceCode) -> Result<Url, AuthError> {
        let text = code
            .verification_uri_complete
            .as_deref()
            .unwrap_or(&code.verification_uri);
        let url = Url::parse(text).map_err(|_| AuthError::UnsafeAddress)?;
        if url.scheme() == "https" && self.site.matches(&url) {
            Ok(url)
        } else {
            Err(AuthError::UnsafeAddress)
        }
    }

    /// Polls until the user approves, refuses or the code expires. Blocks, so run it off the UI
    /// thread. On success the refresh token is stored and the access token kept in memory.
    /// Setting `cancel` ends it with [`AuthError::Cancelled`]; tokens that arrive after a cancel
    /// are revoked, not stored. A session already stored is replaced, and revoked.
    pub fn poll_until_done(&self, code: &DeviceCode, cancel: &AtomicBool) -> Result<(), AuthError> {
        let mut interval = code.interval.clamp(MIN_POLL_SECONDS, MAX_POLL_SECONDS);
        let mut waited = 0u64;
        loop {
            if waited >= code.expires_in {
                return Err(AuthError::Expired);
            }
            self.wait(Duration::from_secs(interval), cancel);
            waited = waited.saturating_add(interval);
            if cancel.load(Ordering::SeqCst) {
                return Err(AuthError::Cancelled);
            }
            let response = self.post_retrying(
                "/v1/oauth/token",
                &[],
                &[
                    ("client_id", CLIENT_ID),
                    ("device_code", code.device_code.expose()),
                    ("grant_type", DEVICE_GRANT),
                ],
                POLL_RETRIES,
                cancel,
            )?;
            let answer: TokenAnswer = serde_json::from_str(&response.body)
                .map_err(|_| AuthError::BadResponse(format!("HTTP status {}", response.status)))?;
            if answer.access_token.is_some() {
                return self.adopt(answer, cancel);
            }
            match answer.error.as_deref() {
                Some("authorization_pending") => {}
                Some("slow_down") => {
                    interval = (interval + SLOW_DOWN_STEP_SECONDS).min(MAX_POLL_SECONDS);
                }
                Some("expired_token") => return Err(AuthError::Expired),
                Some("access_denied") => return Err(AuthError::Denied),
                Some(other) => return Err(provider(other)),
                None => {
                    return Err(AuthError::BadResponse(
                        "neither a token nor an error".to_string(),
                    ));
                }
            }
        }
    }

    /// Takes the tokens of an approved sign-in: stores the refresh token and keeps the access
    /// token in memory, unless the sign-in was cancelled meanwhile.
    fn adopt(&self, answer: TokenAnswer, cancel: &AtomicBool) -> Result<(), AuthError> {
        let access_token = capped(answer.access_token.unwrap_or_default())?;
        let refresh_token = answer.refresh_token.map(capped).transpose()?;
        let expires_at = answer
            .expires_in
            .map(|s| unix_now().saturating_add(s.min(MAX_TOKEN_LIFETIME_SECONDS)));
        let old_refresh;
        {
            let mut cache = locked(&self.cache);
            // Checked under the lock a sign-out also takes: either the sign-out came first and
            // this sees the cancel, or this saves first and the sign-out deletes it.
            if cancel.load(Ordering::SeqCst) {
                drop(cache);
                if let Some(token) = &refresh_token {
                    self.revoke_token(token);
                }
                return Err(AuthError::Cancelled);
            }
            old_refresh = self.read_refresh().ok().flatten();
            let saved = match &refresh_token {
                Some(token) => self.write_refresh(token),
                None => self.store.delete(),
            };
            if let Err(error) = saved {
                drop(cache);
                if let Some(token) = &refresh_token {
                    self.revoke_token(token);
                }
                return Err(error);
            }
            *cache = Some(Cached {
                access_token,
                expires_at,
            });
        }
        if let Some(old) = old_refresh
            && Some(&old) != refresh_token.as_ref()
        {
            self.revoke_token(&old);
        }
        log::info!("platform sign-in finished; session stored");
        Ok(())
    }

    /// The stored refresh token. Call with the cache lock held.
    fn read_refresh(&self) -> Result<Option<String>, AuthError> {
        Ok(self
            .store
            .get_secret()?
            .and_then(|raw| serde_json::from_slice::<Stored>(&raw).ok())
            .map(|stored| stored.refresh_token))
    }

    /// Stores the refresh token (UTF-8 bytes). Call with the cache lock held.
    fn write_refresh(&self, token: &str) -> Result<(), AuthError> {
        let raw = serde_json::to_vec(&Stored {
            version: 1,
            refresh_token: token.to_string(),
        })
        .map_err(|e| AuthError::Store(e.to_string()))?;
        if raw.len() > MAX_SECRET_BYTES {
            return Err(AuthError::TooLarge);
        }
        self.store.set_secret(&raw)
    }

    /// Asks the platform to revoke a refresh token. One try; a failure is only logged.
    fn revoke_token(&self, refresh_token: &str) {
        if let Err(error) = self.post(
            "/v1/oauth/revoke",
            &[],
            &[
                ("client_id", CLIENT_ID),
                ("token", refresh_token),
                ("token_type_hint", "refresh_token"),
            ],
        ) {
            log::warn!("could not revoke a platform session: {error}");
        }
    }

    /// The access token, refreshed first when under 300 s are left. `Ok(None)` means "sign in
    /// (again)": nothing is stored, or the platform refused the refresh token (`invalid_grant`),
    /// in which case the stored session is deleted.
    ///
    /// Refreshes run one at a time: a caller that arrives during a refresh waits and then uses
    /// its result. The new refresh token is stored before the new access token is returned. The
    /// refresh is retried once, with the same token, only when no answer came back.
    pub fn access_token(&self) -> Result<Option<Token>, AuthError> {
        let _refreshing = locked(&self.refresh_lock);
        let now = unix_now();
        let refresh_token = {
            let mut cache = locked(&self.cache);
            let stored = self.read_refresh()?;
            if let Some(cached) = cache.as_ref() {
                let fresh = cached
                    .expires_at
                    .is_none_or(|at| at > now.saturating_add(REFRESH_MARGIN_SECONDS));
                // With no refresh token a token that has not expired yet is all there is.
                let last_use = stored.is_none() && cached.expires_at.is_some_and(|at| at > now);
                if fresh || last_use {
                    return Ok(Some(Token::new(cached.access_token.clone())));
                }
            }
            let Some(token) = stored else {
                *cache = None;
                return Ok(None);
            };
            token
        };

        let never = AtomicBool::new(false);
        let response = self.post_retrying(
            "/v1/oauth/token",
            &[],
            &[
                ("client_id", CLIENT_ID),
                ("grant_type", REFRESH_GRANT),
                ("refresh_token", refresh_token.as_str()),
            ],
            REFRESH_RETRIES,
            &never,
        )?;
        let answer: TokenAnswer = serde_json::from_str(&response.body)
            .map_err(|_| AuthError::BadResponse(format!("HTTP status {}", response.status)))?;
        match answer.error.as_deref() {
            Some("invalid_grant") => {
                log::info!("the platform refused the refresh token; session cleared");
                let mut cache = locked(&self.cache);
                if self.read_refresh()?.as_deref() == Some(refresh_token.as_str()) {
                    self.store.delete()?;
                }
                *cache = None;
                return Ok(None);
            }
            Some(other) => return Err(provider(other)),
            None => {}
        }
        let Some(access_token) = answer.access_token else {
            return Err(AuthError::BadResponse(
                "neither a token nor an error".to_string(),
            ));
        };
        let access_token = capped(access_token)?;
        // A server that does not rotate keeps the old refresh token valid.
        let new_refresh = match answer.refresh_token {
            Some(token) => capped(token)?,
            None => refresh_token.clone(),
        };
        let expires_at = answer
            .expires_in
            .map(|s| now.saturating_add(s.min(MAX_TOKEN_LIFETIME_SECONDS)));
        {
            let mut cache = locked(&self.cache);
            if self.read_refresh()?.as_deref() != Some(refresh_token.as_str()) {
                // Signed out (or signed in again) while the request ran: the new tokens are
                // not wanted.
                drop(cache);
                self.revoke_token(&new_refresh);
                return Ok(None);
            }
            if let Err(error) = self.write_refresh(&new_refresh) {
                // The old refresh token is spent, so it must not stay behind to be reused.
                let _ = self.store.delete();
                *cache = None;
                return Err(error);
            }
            *cache = Some(Cached {
                access_token: access_token.clone(),
                expires_at,
            });
        }
        log::info!("platform token refreshed");
        Ok(Some(Token::new(access_token)))
    }

    /// Asks the platform for a one-time code that signs the web view in (single use, 60 s).
    /// Needs a signed-in session ([`AuthError::NotSignedIn`] otherwise). A server or network
    /// failure is tried three times in all. The code is a [`Token`]: never log it.
    pub fn web_session_code(&self) -> Result<Token, AuthError> {
        self.web_session_code_until(&AtomicBool::new(false))
    }

    /// Like [`web_session_code`](Self::web_session_code), and ends early when `cancel` is set.
    pub fn web_session_code_until(&self, cancel: &AtomicBool) -> Result<Token, AuthError> {
        let token = self.access_token()?.ok_or(AuthError::NotSignedIn)?;
        let bearer = format!("Bearer {}", token.expose());
        let response = self.post_retrying(
            "/v1/oauth/web-session",
            &[("Authorization", &bearer)],
            &[],
            HANDOFF_RETRIES,
            cancel,
        )?;
        match response.status {
            200..=299 => {}
            401 | 403 => return Err(AuthError::NotSignedIn),
            other => return Err(AuthError::BadResponse(format!("HTTP status {other}"))),
        }
        let answer: WebSessionAnswer = serde_json::from_str(&response.body)
            .map_err(|_| AuthError::BadResponse("unreadable web-session answer".to_string()))?;
        let safe = !answer.code.is_empty()
            && answer.code.len() <= MAX_CODE_LEN
            && answer
                .code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !safe {
            return Err(AuthError::BadResponse(
                "the web-session code has an unexpected shape".to_string(),
            ));
        }
        Ok(Token::new(answer.code))
    }

    /// The address that signs the web view in with `code`.
    pub fn handoff_url(&self, code: &Token) -> Result<HandoffUrl, AuthError> {
        let mut url = Url::parse(&self.api("/auth/launcher-handoff"))
            .map_err(|_| AuthError::UnsafeAddress)?;
        url.query_pairs_mut().append_pair("code", code.expose());
        Ok(HandoffUrl(url))
    }

    /// Forgets the session on this PC at once (memory and credential store) and returns what
    /// is left to revoke on the platform with [`revoke`](Self::revoke). Does no network call.
    pub fn sign_out_local(&self) -> Result<Revocation, AuthError> {
        let mut cache = locked(&self.cache);
        let refresh = self.read_refresh().ok().flatten();
        *cache = None;
        self.store.delete()?;
        Ok(Revocation(refresh.map(Token::new)))
    }

    /// Revokes a forgotten session on the platform. One try; a failure is only logged.
    pub fn revoke(&self, revocation: Revocation) {
        if let Some(token) = revocation.0 {
            self.revoke_token(token.expose());
        }
    }

    /// Signs out now: [`sign_out_local`](Self::sign_out_local), then [`revoke`](Self::revoke)
    /// (which blocks for one request). The bridge uses [`sign_out_in_background`] instead.
    pub fn sign_out(&self) -> Result<(), AuthError> {
        let revocation = self.sign_out_local()?;
        self.revoke(revocation);
        Ok(())
    }
}

/// Lets one sign-in run at a time. Cloning shares the same state.
#[derive(Clone, Default)]
pub struct OneAtATime(Arc<OneAtATimeState>);

#[derive(Default)]
struct OneAtATimeState {
    running: AtomicBool,
    cancel: AtomicBool,
    /// The code and page of the running sign-in, once known.
    shown: Mutex<Option<(String, String)>>,
}

/// Proof that this sign-in is the running one; dropping it lets the next one start.
pub struct SignInTicket(OneAtATime);

impl OneAtATime {
    /// A guard with nothing running.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the place if nobody has it. `None` means a sign-in is already running.
    pub fn try_begin(&self) -> Option<SignInTicket> {
        if self.0.running.swap(true, Ordering::SeqCst) {
            return None;
        }
        self.0.cancel.store(false, Ordering::SeqCst);
        *locked(&self.0.shown) = None;
        Some(SignInTicket(self.clone()))
    }

    /// Asks the running sign-in (if any) to stop.
    pub fn cancel_running(&self) {
        self.0.cancel.store(true, Ordering::SeqCst);
    }

    /// The code and verification page of the running sign-in, once it has them.
    pub fn current(&self) -> Option<(String, String)> {
        locked(&self.0.shown).clone()
    }
}

impl SignInTicket {
    /// True once [`OneAtATime::cancel_running`] was called.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &(self.0).0.cancel
    }

    fn show(&self, user_code: &str, verification_uri: &str) {
        *locked(&(self.0).0.shown) = Some((user_code.to_string(), verification_uri.to_string()));
    }
}

impl Drop for SignInTicket {
    fn drop(&mut self) {
        (self.0).0.running.store(false, Ordering::SeqCst);
    }
}

/// What the page is told at once when it asks to sign in. The code is there only when a sign-in
/// was already running and had it; otherwise it follows as a `signIn` event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignInReply {
    /// The short code the user types.
    pub user_code: Option<String>,
    /// The verification page, without a query (on the site's origin, checked).
    pub verification_uri: Option<String>,
}

fn pending_event(user_code: &str, verification_uri: &str) -> AppEvent {
    AppEvent::Bridge(BridgeEvent::SignIn {
        state: "pending".to_string(),
        user_code: Some(user_code.to_string()),
        verification_uri: Some(verification_uri.to_string()),
    })
}

fn state_for(error: &AuthError) -> &'static str {
    match error {
        AuthError::Denied => "denied",
        AuthError::Expired => "expired",
        _ => "failed",
    }
}

/// Sends the closing `signIn` event, and `failed` if the worker ends without having sent one
/// (a panic, say), so the page is never left at `pending`.
struct Closing {
    sink: Arc<dyn EventSink>,
    sent: bool,
}

impl Closing {
    fn send(&mut self, state: &str) {
        self.sent = true;
        self.sink.send(AppEvent::Bridge(BridgeEvent::SignIn {
            state: state.to_string(),
            user_code: None,
            verification_uri: None,
        }));
    }
}

impl Drop for Closing {
    fn drop(&mut self) {
        if !self.sent {
            self.send("failed");
        }
    }
}

/// Starts the Metallic sign-in. Returns at once; a worker does the network calls.
///
/// - Signed in already (or refreshable): only the handoff is done.
/// - Otherwise the worker asks for a device code, sends it as a `signIn` event
///   (`pending`, `userCode`, `verificationUri`), calls `open` with the verification page (checked
///   to be `https` on the site's origin) and polls.
/// - It ends as a `signIn` event: `signedIn` (followed by [`AppEvent::Handoff`]), `denied`,
///   `expired` or `failed`. A failure is final for that attempt: nothing is left half-stored,
///   and a signed-in session makes the next attempt skip the browser.
///
/// A call while one is running returns the running one's code (and repeats the `pending`
/// event) instead of starting another.
pub fn platform_sign_in(
    auth: Arc<PlatformAuth>,
    sink: Arc<dyn EventSink>,
    open: Arc<dyn Fn(&Url) + Send + Sync>,
    guard: &OneAtATime,
) -> Result<SignInReply, LauncherError> {
    let Some(ticket) = guard.try_begin() else {
        let current = guard.current();
        if let Some((code, uri)) = &current {
            sink.send(pending_event(code, uri));
        }
        return Ok(SignInReply {
            user_code: current.as_ref().map(|(code, _)| code.clone()),
            verification_uri: current.map(|(_, uri)| uri),
        });
    };
    std::thread::Builder::new()
        .name("platform-sign-in".to_string())
        .spawn(move || {
            let mut closing = Closing {
                sink: sink.clone(),
                sent: false,
            };
            let cancel = ticket.cancel_flag();
            let outcome = (|| {
                if auth.access_token()?.is_none() {
                    let code = auth.start()?;
                    let address = auth.browser_address(&code)?;
                    let mut shown = address.clone();
                    shown.set_query(None);
                    shown.set_fragment(None);
                    ticket.show(&code.user_code, shown.as_str());
                    sink.send(pending_event(&code.user_code, shown.as_str()));
                    open(&address);
                    auth.poll_until_done(&code, cancel)?;
                }
                if cancel.load(Ordering::SeqCst) {
                    return Err(AuthError::Cancelled);
                }
                let code = auth.web_session_code_until(cancel)?;
                let url = auth.handoff_url(&code)?;
                if cancel.load(Ordering::SeqCst) {
                    return Err(AuthError::Cancelled);
                }
                Ok(url)
            })();
            match outcome {
                Ok(url) => {
                    closing.send("signedIn");
                    sink.send(AppEvent::Handoff(url));
                }
                Err(error) => {
                    log::warn!("the platform sign-in did not finish: {error}");
                    closing.send(state_for(&error));
                }
            }
        })
        .map_err(|e| LauncherError::Io(e.to_string()))?;
    Ok(SignInReply::default())
}

/// Signs out: stops a running sign-in, forgets the session on this PC at once, and revokes it on
/// the platform from a worker thread (best effort). The caller waits for no network call.
pub fn sign_out_in_background(
    auth: Arc<PlatformAuth>,
    guard: &OneAtATime,
) -> Result<(), AuthError> {
    guard.cancel_running();
    let revocation = auth.sign_out_local()?;
    if revocation.is_some()
        && let Err(error) = std::thread::Builder::new()
            .name("platform-revoke".to_string())
            .spawn(move || auth.revoke(revocation))
    {
        log::warn!("could not start the revoke: {error}");
    }
    Ok(())
}
