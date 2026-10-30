//! Signing in to the Metallic platform with the OAuth device flow, and handing the session to
//! the web view.
//!
//! The user signs in in the system browser (the Launcher never sees a password): the Launcher asks
//! the platform for a short code ([`PlatformAuth::start`]), opens the verification page, and polls
//! until the user has approved ([`PlatformAuth::poll_until_done`]). The tokens are kept as one JSON
//! value in the Windows Credential Manager (`Metallic` / `platform`) behind [`SecretStore`], never
//! in `settings.toml`.
//!
//! The web view is signed in separately. With its access token the Launcher asks for a one-time
//! code ([`PlatformAuth::web_session_code`], 60 s, single use) and navigates to
//! `<site>/auth/launcher-handoff?code=<code>`; the site redeems the code itself and sets its own
//! cookie. That code is the only secret that ever appears in an address, so it travels as a
//! [`Token`] and a [`HandoffUrl`], both of which print a placeholder instead of their value.
//!
//! [`platform_sign_in`] ties this together for the bridge: it answers at once with the code the
//! user types and reports the rest as `signIn` events (`pending`, `signedIn`, `denied`, `expired`,
//! `failed`). [`OneAtATime`] makes a second sign-in while one is running answer `busy`.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use metallic_update::{AuthError, HttpClient, HttpResponse, SecretStore, Token};
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

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const REFRESH_GRANT: &str = "refresh_token";
const SLOW_DOWN_STEP_SECONDS: u64 = 5;
const MIN_POLL_SECONDS: u64 = 1;
/// Retries for a transient failure (network error or HTTP 5xx) while polling or refreshing.
const MAX_RETRIES: u32 = 3;
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
    /// Seconds between polls.
    pub interval: u64,
    /// Seconds until the code expires.
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

/// What is stored in the credential store. Not `Debug`: it carries tokens.
#[derive(Serialize, Deserialize)]
struct Session {
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    /// Unix seconds; `None` when the platform sent no lifetime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Session {
    fn needs_refresh(&self, now: u64) -> bool {
        self.expires_at
            .is_some_and(|at| at <= now + REFRESH_MARGIN_SECONDS)
    }
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

/// Waits between polls and retries. Tests replace it.
type Sleeper = Arc<dyn Fn(Duration) + Send + Sync>;

/// The Metallic platform sign-in. The platform API is on the site's origin (`<origin>/v1/...`).
pub struct PlatformAuth {
    site: SiteOrigin,
    http: Arc<dyn HttpClient>,
    store: Arc<dyn SecretStore>,
    sleeper: Option<Sleeper>,
    /// Held for the whole of `access_token`: refresh tokens rotate, so two refreshes at once
    /// would make the second fail and wipe the session the first saved.
    refresh_lock: Mutex<()>,
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

    /// Like [`post`](Self::post), retrying a transient failure up to `MAX_RETRIES` times after
    /// waiting 1, 2, then 4 seconds.
    fn post_retrying(
        &self,
        path: &str,
        form: &[(&str, &str)],
        cancel: &AtomicBool,
    ) -> Result<HttpResponse, AuthError> {
        let mut retries = 0;
        loop {
            match self.post(path, &[], form) {
                Err(AuthError::Network(message)) if retries < MAX_RETRIES => {
                    log::warn!("platform request failed ({message}); retrying");
                    self.wait(Duration::from_secs(1 << retries), cancel);
                    if cancel.load(Ordering::SeqCst) {
                        return Err(AuthError::Cancelled);
                    }
                    retries += 1;
                }
                other => return other,
            }
        }
    }

    /// Asks the platform for a device code (one attempt, no retries: the user is waiting).
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
        Ok(DeviceCode {
            user_code: answer.user_code,
            verification_uri: answer.verification_uri,
            verification_uri_complete: answer.verification_uri_complete,
            interval: answer.interval.max(MIN_POLL_SECONDS),
            expires_in: answer.expires_in,
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
    /// thread. On success the session is stored. Setting `cancel` ends it with
    /// [`AuthError::Cancelled`].
    pub fn poll_until_done(&self, code: &DeviceCode, cancel: &AtomicBool) -> Result<(), AuthError> {
        let mut interval = code.interval.max(MIN_POLL_SECONDS);
        let mut waited = 0u64;
        loop {
            if waited >= code.expires_in {
                return Err(AuthError::Expired);
            }
            self.wait(Duration::from_secs(interval), cancel);
            waited += interval;
            if cancel.load(Ordering::SeqCst) {
                return Err(AuthError::Cancelled);
            }
            let response = self.post_retrying(
                "/v1/oauth/token",
                &[
                    ("client_id", CLIENT_ID),
                    ("device_code", code.device_code.expose()),
                    ("grant_type", DEVICE_GRANT),
                ],
                cancel,
            )?;
            let answer: TokenAnswer = serde_json::from_str(&response.body)
                .map_err(|_| AuthError::BadResponse(format!("HTTP status {}", response.status)))?;
            if let Some(access_token) = answer.access_token.clone() {
                self.save(&Session {
                    access_token,
                    refresh_token: answer.refresh_token.clone(),
                    expires_at: answer.expires_in.map(|s| unix_now() + s),
                })?;
                log::info!("platform sign-in finished; session stored");
                return Ok(());
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

    fn load(&self) -> Result<Option<Session>, AuthError> {
        Ok(self
            .store
            .get()?
            .and_then(|raw| serde_json::from_str::<Session>(&raw).ok()))
    }

    fn save(&self, session: &Session) -> Result<(), AuthError> {
        let raw = serde_json::to_string(session).map_err(|e| AuthError::Store(e.to_string()))?;
        self.store.set(&raw)
    }

    /// The stored access token, refreshed first when under 300 s are left. `Ok(None)` means
    /// "sign in (again)": nothing is stored, or the session ended (`invalid_grant`, or expired
    /// with no refresh token), in which case the stored session is deleted. A rotated refresh
    /// token is stored before this returns.
    pub fn access_token(&self) -> Result<Option<Token>, AuthError> {
        let _guard = self
            .refresh_lock
            .lock()
            .map_err(|_| AuthError::Store("refresh lock was poisoned".to_string()))?;
        // Read the store only after taking the lock: another caller may have just refreshed.
        let Some(session) = self.load()? else {
            return Ok(None);
        };
        let now = unix_now();
        if !session.needs_refresh(now) {
            return Ok(Some(Token::new(session.access_token)));
        }
        let Some(refresh_token) = session.refresh_token.clone() else {
            log::info!("the platform session expired and cannot be refreshed; sign in again");
            self.store.delete()?;
            return Ok(None);
        };
        let never = AtomicBool::new(false);
        let response = self.post_retrying(
            "/v1/oauth/token",
            &[
                ("client_id", CLIENT_ID),
                ("grant_type", REFRESH_GRANT),
                ("refresh_token", refresh_token.as_str()),
            ],
            &never,
        )?;
        let answer: TokenAnswer = serde_json::from_str(&response.body)
            .map_err(|_| AuthError::BadResponse(format!("HTTP status {}", response.status)))?;
        match answer.error.as_deref() {
            Some("invalid_grant") => {
                log::info!("the platform refused the refresh token; session cleared");
                self.store.delete()?;
                return Ok(None);
            }
            Some(other) => return Err(AuthError::Provider(other.to_string())),
            None => {}
        }
        let Some(access_token) = answer.access_token.clone() else {
            return Err(AuthError::BadResponse(
                "neither a token nor an error".to_string(),
            ));
        };
        self.save(&Session {
            access_token: access_token.clone(),
            // A server that does not rotate keeps the old refresh token valid.
            refresh_token: answer.refresh_token.clone().or(Some(refresh_token)),
            expires_at: answer.expires_in.map(|s| now + s),
        })?;
        log::info!("platform token refreshed");
        Ok(Some(Token::new(access_token)))
    }

    /// Asks the platform for a one-time code that signs the web view in (single use, 60 s).
    /// Needs a signed-in session ([`AuthError::NotSignedIn`] otherwise). The code is a [`Token`]:
    /// it must never be logged.
    pub fn web_session_code(&self) -> Result<Token, AuthError> {
        let token = self.access_token()?.ok_or(AuthError::NotSignedIn)?;
        let bearer = format!("Bearer {}", token.expose());
        let response = self.post("/v1/oauth/web-session", &[("Authorization", &bearer)], &[])?;
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

    /// Revokes the refresh token on the platform (one try), then deletes the stored session
    /// whether or not that worked. The site clears its own cookie through its sign-out.
    pub fn sign_out(&self) -> Result<(), AuthError> {
        if let Ok(Some(Session {
            refresh_token: Some(refresh_token),
            ..
        })) = self.load()
            && let Err(error) = self.post(
                "/v1/oauth/revoke",
                &[],
                &[
                    ("client_id", CLIENT_ID),
                    ("token", refresh_token.as_str()),
                    ("token_type_hint", "refresh_token"),
                ],
            )
        {
            log::warn!("could not revoke the platform session: {error}");
        }
        self.store.delete()
    }
}

/// Lets one sign-in run at a time. Cloning shares the same state.
#[derive(Clone, Default)]
pub struct OneAtATime(Arc<OneAtATimeState>);

#[derive(Default)]
struct OneAtATimeState {
    running: AtomicBool,
    cancel: AtomicBool,
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
        Some(SignInTicket(self.clone()))
    }

    /// Asks the running sign-in (if any) to stop.
    pub fn cancel_running(&self) {
        self.0.cancel.store(true, Ordering::SeqCst);
    }
}

impl SignInTicket {
    /// True once [`OneAtATime::cancel_running`] was called.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &(self.0).0.cancel
    }
}

impl Drop for SignInTicket {
    fn drop(&mut self) {
        (self.0).0.running.store(false, Ordering::SeqCst);
    }
}

/// What the page is told when a sign-in starts: the code to type and the page it is typed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInStarted {
    /// The short code the user types.
    pub user_code: String,
    /// The verification page, without a query (it is on the site's origin, checked).
    pub verification_uri: String,
}

fn state_for(result: &Result<(), AuthError>) -> &'static str {
    match result {
        Ok(()) => "signedIn",
        Err(AuthError::Denied) => "denied",
        Err(AuthError::Expired) => "expired",
        Err(_) => "failed",
    }
}

/// Starts the Metallic sign-in. Answers at once with the code to type; the rest runs on a new
/// thread and ends as a `signIn` event (`signedIn`, `denied`, `expired` or `failed`).
///
/// `open` is called with the verification page, which has been checked to be `https` on the
/// site's origin. On success the web view is sent to the handoff address as
/// [`AppEvent::Handoff`]; if the handoff code cannot be had, the state is `failed`.
///
/// A second call while one is running is [`LauncherError::Busy`]. Asking the platform for the
/// code blocks the caller for one HTTP request, so give `auth` a short timeout.
pub fn platform_sign_in(
    auth: Arc<PlatformAuth>,
    sink: Arc<dyn EventSink>,
    open: Arc<dyn Fn(&Url) + Send + Sync>,
    guard: &OneAtATime,
) -> Result<SignInStarted, LauncherError> {
    let ticket = guard.try_begin().ok_or(LauncherError::Busy("sign-in"))?;
    let failed = |what: &str, error: &AuthError| {
        log::warn!("{what}: {error}");
        LauncherError::Update("could not start the sign-in".to_string())
    };
    let code = auth
        .start()
        .map_err(|e| failed("could not start the platform sign-in", &e))?;
    let address = auth
        .browser_address(&code)
        .map_err(|e| failed("refused the platform sign-in page", &e))?;
    let mut shown = address.clone();
    shown.set_query(None);
    shown.set_fragment(None);
    let started = SignInStarted {
        user_code: code.user_code.clone(),
        verification_uri: shown.to_string(),
    };
    sink.send(AppEvent::Bridge(BridgeEvent::SignIn {
        state: "pending".to_string(),
        user_code: Some(code.user_code.clone()),
    }));
    open(&address);

    std::thread::Builder::new()
        .name("platform-sign-in".to_string())
        .spawn(move || {
            let result = auth.poll_until_done(&code, ticket.cancel_flag());
            let handoff = match &result {
                Ok(()) => auth
                    .web_session_code()
                    .and_then(|code| auth.handoff_url(&code))
                    .inspect_err(|error| log::warn!("could not get the web session: {error}"))
                    .ok(),
                Err(error) => {
                    log::warn!("the platform sign-in did not finish: {error}");
                    None
                }
            };
            let state = match (&result, &handoff) {
                (Ok(()), None) => "failed",
                _ => state_for(&result),
            };
            sink.send(AppEvent::Bridge(BridgeEvent::SignIn {
                state: state.to_string(),
                user_code: None,
            }));
            if let Some(url) = handoff {
                sink.send(AppEvent::Handoff(url));
            }
        })
        .map_err(|e| LauncherError::Io(e.to_string()))?;
    Ok(started)
}
