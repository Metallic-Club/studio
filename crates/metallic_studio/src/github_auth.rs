//! GitHub sign-in for Metallic Studio using the OAuth device flow.
//!
//! Studio shows a short code, opens `https://github.com/login/device` in the browser, and the
//! user approves there. Studio never sees a password. The resulting token (scope `repo`, needed
//! to read releases of the private repo) is kept in the Windows Credential Manager under
//! `MetallicStudio/github`, behind the [`SecretStore`] trait so tests use memory instead.
//!
//! The OAuth app's client ID is not secret. It is committed in `config/github_oauth_client_id.txt`
//! and compiled in; until the owner registers the app, the file holds a placeholder and
//! [`embedded_client_id`] reports a clear error.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

use crate::updater::{HttpClient, Token, TokenProvider};

/// The placeholder line that stands in for the client ID until the owner registers the app.
pub const CLIENT_ID_PLACEHOLDER: &str = "REPLACE_WITH_CLIENT_ID";

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const SCOPE: &str = "repo";
const KEYRING_SERVICE: &str = "MetallicStudio";
const KEYRING_ACCOUNT: &str = "github";
const SLOW_DOWN_STEP_SECONDS: u64 = 5;
const MIN_POLL_SECONDS: u64 = 1;

/// Why signing in or storing the token failed. No message contains the token.
#[derive(Debug)]
pub enum AuthError {
    /// `config/github_oauth_client_id.txt` still holds the placeholder (or is empty).
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
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::ClientIdNotConfigured => f.write_str(
                "GitHub sign-in is not set up yet: put the OAuth app's client ID into \
                 config/github_oauth_client_id.txt (see docs/releasing.md) and rebuild",
            ),
            AuthError::Network(m) => write!(f, "could not reach GitHub: {m}"),
            AuthError::BadResponse(m) => write!(f, "unexpected answer from GitHub: {m}"),
            AuthError::Denied => f.write_str("sign-in was refused on the GitHub page"),
            AuthError::Expired => f.write_str("the sign-in code expired; start again"),
            AuthError::Provider(code) => write!(f, "GitHub sign-in failed: {code}"),
            AuthError::Store(m) => write!(f, "credential store error: {m}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Reads the client ID from the text of `config/github_oauth_client_id.txt`.
pub fn parse_client_id(text: &str) -> Result<String, AuthError> {
    let id = text.trim();
    if id.is_empty() || id == CLIENT_ID_PLACEHOLDER {
        return Err(AuthError::ClientIdNotConfigured);
    }
    Ok(id.to_string())
}

/// The client ID compiled in from `config/github_oauth_client_id.txt`.
pub fn embedded_client_id() -> Result<String, AuthError> {
    parse_client_id(include_str!("../../../config/github_oauth_client_id.txt"))
}

/// Where the GitHub token is kept between runs.
pub trait SecretStore: Send + Sync {
    /// The stored token text, or `None` when nothing is stored.
    fn get(&self) -> Result<Option<String>, AuthError>;
    /// Stores (or replaces) the token text.
    fn set(&self, value: &str) -> Result<(), AuthError>;
    /// Removes the stored token; removing nothing is fine.
    fn delete(&self) -> Result<(), AuthError>;
}

/// In-memory [`SecretStore`] for tests.
#[derive(Default)]
pub struct MemorySecretStore {
    value: Mutex<Option<String>>,
}

impl MemorySecretStore {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<String>>, AuthError> {
        self.value
            .lock()
            .map_err(|_| AuthError::Store("lock was poisoned".to_string()))
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self) -> Result<Option<String>, AuthError> {
        Ok(self.lock()?.clone())
    }

    fn set(&self, value: &str) -> Result<(), AuthError> {
        *self.lock()? = Some(value.to_string());
        Ok(())
    }

    fn delete(&self) -> Result<(), AuthError> {
        *self.lock()? = None;
        Ok(())
    }
}

/// [`SecretStore`] in the operating system's credential store (Windows Credential Manager),
/// entry `MetallicStudio/github`.
#[derive(Default)]
pub struct KeyringStore;

impl KeyringStore {
    fn entry() -> Result<keyring::Entry, AuthError> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|e| AuthError::Store(e.to_string()))
    }
}

impl SecretStore for KeyringStore {
    fn get(&self) -> Result<Option<String>, AuthError> {
        match Self::entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Store(e.to_string())),
        }
    }

    fn set(&self, value: &str) -> Result<(), AuthError> {
        Self::entry()?
            .set_password(value)
            .map_err(|e| AuthError::Store(e.to_string()))
    }

    fn delete(&self) -> Result<(), AuthError> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::Store(e.to_string())),
        }
    }
}

/// A [`TokenProvider`] that reads the token from a [`SecretStore`] each time.
pub struct StoredTokenProvider(pub Arc<dyn SecretStore>);

impl TokenProvider for StoredTokenProvider {
    fn token(&self) -> Option<Token> {
        match self.0.get() {
            Ok(value) => value.map(Token::new),
            Err(error) => {
                log::warn!("could not read the stored GitHub token: {error}");
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

/// Opens a GitHub page in the default browser. Refuses any other address.
pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    if !url.starts_with("https://github.com/") {
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

#[derive(Deserialize)]
struct PollAnswer {
    access_token: Option<String>,
    error: Option<String>,
}

/// Runs the GitHub device flow and keeps the token in a [`SecretStore`].
pub struct GithubAuth {
    client_id: String,
    http: Arc<dyn HttpClient>,
    store: Arc<dyn SecretStore>,
    sleeping: bool,
}

impl GithubAuth {
    /// Creates the sign-in helper for the OAuth app `client_id`.
    pub fn new(client_id: String, http: Arc<dyn HttpClient>, store: Arc<dyn SecretStore>) -> Self {
        Self {
            client_id,
            http,
            store,
            sleeping: true,
        }
    }

    /// Makes polling skip its waits (tests).
    pub fn without_sleeping(mut self) -> Self {
        self.sleeping = false;
        self
    }

    /// The stored token, if the user signed in before.
    pub fn stored_token(&self) -> Result<Option<Token>, AuthError> {
        Ok(self.store.get()?.map(Token::new))
    }

    /// Forgets the stored token.
    pub fn sign_out(&self) -> Result<(), AuthError> {
        self.store.delete()
    }

    fn post(&self, url: &str, form: &[(&str, &str)]) -> Result<String, AuthError> {
        let response = self
            .http
            .post_form(url, &[("Accept", "application/json")], form)
            .map_err(|e| AuthError::Network(e.to_string()))?;
        if !(200..300).contains(&response.status) {
            return Err(AuthError::BadResponse(format!(
                "HTTP status {}",
                response.status
            )));
        }
        Ok(response.body)
    }

    /// Signs in with GitHub's device flow: asks for a code, hands it to `ui`, then polls until the
    /// user approves. Blocks the calling thread, so run it off the UI thread. On success the token
    /// is stored and returned.
    pub fn device_sign_in(&self, ui: &mut dyn DeviceFlowUi) -> Result<Token, AuthError> {
        let body = self.post(
            DEVICE_CODE_URL,
            &[("client_id", self.client_id.as_str()), ("scope", SCOPE)],
        )?;
        let code: DeviceCodeAnswer =
            serde_json::from_str(&body).map_err(|e| AuthError::BadResponse(e.to_string()))?;
        log::info!("GitHub device sign-in started; waiting for approval");
        ui.show_code(&code.user_code, &code.verification_uri);

        let mut interval = code.interval.max(MIN_POLL_SECONDS);
        let mut waited = 0u64;
        loop {
            if waited >= code.expires_in {
                return Err(AuthError::Expired);
            }
            if self.sleeping {
                std::thread::sleep(Duration::from_secs(interval));
            }
            waited += interval;
            let body = self.post(
                ACCESS_TOKEN_URL,
                &[
                    ("client_id", self.client_id.as_str()),
                    ("device_code", code.device_code.as_str()),
                    ("grant_type", DEVICE_GRANT),
                ],
            )?;
            let answer: PollAnswer =
                serde_json::from_str(&body).map_err(|e| AuthError::BadResponse(e.to_string()))?;
            if let Some(value) = answer.access_token {
                self.store.set(&value)?;
                log::info!("GitHub sign-in finished; token stored");
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
