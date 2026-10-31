//! The bridge between the page and the Launcher: the one way a web page can ask Metallic to do
//! something on this PC. This is the main attack surface, so every step is a check.
//!
//! How a call travels:
//!
//! 1. The page calls `window.metallic.invoke(cmd, args)` (see `assets/bridge.js`), which posts
//!    `{"id": 7, "cmd": "play", "args": {...}}` to the Launcher.
//! 2. [`Bridge::handle`] drops the message unless it comes from the site or a bundled page,
//!    is at most [`MAX_MESSAGE_BYTES`] long and parses into a [`BridgeCommand`] (unknown commands
//!    and unknown fields are refused). Then [`allowed`] decides whether this kind of page may use
//!    this command, and the [`BridgeHost`] does the work.
//! 3. [`Bridge::deliver`] turns the answer into a script, but only if the page showing now has
//!    the same origin as the page that asked. A page that navigated away never gets an answer.
//!
//! The host's answers carry values or one of the fixed [`BridgeErrorCode`]s, never internal text.
//! Tokens and join tickets never pass through here: logs name the origin kind and the command
//! only, and [`BridgeCommand`]'s `Debug` prints just the command name.
//!
//! ## Commands
//!
//! `getInfo`, `play`, `installStatus`, `installStudio`, `openStudio`, `getSettings`,
//! `setSettings`, `signIn`, `signOut`, `restartToUpdate` (site pages), and `githubSignIn`,
//! `retrySite` (bundled pages only).
//!
//! `restartToUpdate` takes no arguments. On success the Launcher closes and restarts into the
//! update, so the page simply goes away; if the update cannot be applied it rejects with `busy`
//! (a Player is running) or `failed` (nothing is downloaded, or applying did not start).
//!
//! `installStudio` answers at once (`busy` when an install is already running) and the install
//! goes on in the background: `installProgress` and `installFinished` events tell how it goes,
//! and `installStatus` answers `{player, studio, launcher}`, each an object with a `state`
//! (`notInstalled`, `installed` + `version`, `installing` + `percent`, `failed` + `reason`; for
//! the launcher `notInstalledCopy`, `upToDate`, `checking`, `downloading` + `percent`,
//! `readyToRestart` + `version` + `required`, `needsGithubSignIn`, `failed` + `reason`).
//! `signIn` answers at once with `{state: "pending"}` and does the rest on a worker: the code to
//! type arrives as a `signIn` event `{state: "pending", userCode, verificationUri}` and the
//! Launcher opens the verification page in the system browser. Later `signIn` events are
//! `signedIn`, `denied`, `expired` or `failed`, and on success the Launcher signs the web view
//! in itself (see `platform_auth`). A second `signIn` while one is running answers `pending`
//! again, with `userCode` and `verificationUri` once they are known, and repeats the event.
//! A signed-in Launcher skips the browser step and only signs the web view in. `signOut` forgets
//! the session at once and revokes it in the background. A second `githubSignIn` while one is
//! running answers `busy`.
//! `githubSignIn` starts the GitHub sign-in and answers at once; the code to type arrives in
//! `githubSignIn` events, which only bundled pages receive.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use url::Url;

use crate::deep_link::{DeepLink, PlaceId};
use crate::settings::{LauncherSettings, SettingsPatch};
use crate::url_policy::{PageOrigin, UrlPolicy};

/// The version of the message format. The site checks it before using the bridge.
pub const BRIDGE_VERSION: u32 = 1;
/// The longest message the Launcher reads, in bytes. Longer ones are dropped unread.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;
/// The most calls that may be running at once. More are answered `busy`. A call counts only
/// while the host works on it: an answer that waits to be delivered holds no place, and long
/// work (an install, a download) is done in the background, so it holds none either.
pub const MAX_IN_FLIGHT: usize = 32;
/// The shortest time between two Player starts.
const PLAY_INTERVAL: Duration = Duration::from_secs(2);
/// How many messages a page may send: a steady rate and a burst (more are answered `busy`).
const MESSAGES_PER_SECOND: f64 = 20.0;
const MESSAGE_BURST: f64 = 40.0;
/// How many disk-writing commands (`setSettings`) a page may send per second.
const WRITES_PER_SECOND: f64 = 2.0;

/// The text of `assets/bridge.js`, with the list of trusted origins still to be inserted.
const BRIDGE_JS: &str = include_str!("../assets/bridge.js");
const ORIGINS_MARKER: &str = "__METALLIC_ORIGINS__";

/// A command a page may send. The wire form is `{"cmd": "<name>", "args": {...}}`; argument
/// names are camelCase, and unknown commands or fields are refused.
#[derive(Clone, Deserialize)]
#[serde(
    tag = "cmd",
    content = "args",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BridgeCommand {
    /// Version, platform and channel of this Launcher.
    GetInfo,
    /// Start the Player with a `metallic://play` link.
    Play {
        /// The link; it must parse as a Play link.
        launch_uri: String,
    },
    /// What is installed.
    InstallStatus,
    /// Install Metallic Studio.
    InstallStudio,
    /// Open Metallic Studio, optionally on a place (a decimal place id).
    OpenStudio {
        /// The place id, if any.
        place: Option<String>,
    },
    /// The Launcher settings the page may see.
    GetSettings,
    /// Change Launcher settings.
    SetSettings {
        /// The change.
        patch: SettingsPatch,
    },
    /// Start signing in to Metallic.
    SignIn,
    /// Sign out of Metallic.
    SignOut,
    /// Start signing in to GitHub for team builds (bundled pages only).
    GithubSignIn,
    /// Check the site again now (bundled pages only).
    RetrySite,
    /// Restart the Launcher into a downloaded update.
    RestartToUpdate,
}

impl BridgeCommand {
    /// The command's name as it appears on the wire.
    pub fn name(&self) -> &'static str {
        match self {
            BridgeCommand::GetInfo => "getInfo",
            BridgeCommand::Play { .. } => "play",
            BridgeCommand::InstallStatus => "installStatus",
            BridgeCommand::InstallStudio => "installStudio",
            BridgeCommand::OpenStudio { .. } => "openStudio",
            BridgeCommand::GetSettings => "getSettings",
            BridgeCommand::SetSettings { .. } => "setSettings",
            BridgeCommand::SignIn => "signIn",
            BridgeCommand::SignOut => "signOut",
            BridgeCommand::GithubSignIn => "githubSignIn",
            BridgeCommand::RetrySite => "retrySite",
            BridgeCommand::RestartToUpdate => "restartToUpdate",
        }
    }
}

/// Prints the command name only: a `Play` command carries a join ticket that must not be logged.
impl fmt::Debug for BridgeCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Whether a page of this kind may send this command. `Site` pages may send everything except
/// `githubSignIn` and `retrySite`, and may change only `close_to_tray`; `Local` (bundled) pages
/// may send `getInfo`, `installStatus`, `getSettings`, `setSettings`, `githubSignIn` and
/// `retrySite`; other pages may send nothing.
pub fn allowed(origin: PageOrigin, cmd: &BridgeCommand) -> bool {
    match origin {
        PageOrigin::Other => false,
        PageOrigin::Site => match cmd {
            BridgeCommand::GithubSignIn | BridgeCommand::RetrySite => false,
            // Developer mode, the development address and the update channel are for the
            // Launcher's own settings page.
            // Listed field by field with no `..`: a new setting does not compile until it is
            // put on one side or the other, so it can never default to "the site may set it".
            BridgeCommand::SetSettings { patch } => {
                let SettingsPatch {
                    close_to_tray: _,
                    developer_mode: None,
                    dev_url: None,
                    channel: None,
                } = patch
                else {
                    return false;
                };
                true
            }
            _ => true,
        },
        PageOrigin::Local => matches!(
            cmd,
            BridgeCommand::GetInfo
                | BridgeCommand::InstallStatus
                | BridgeCommand::GetSettings
                | BridgeCommand::SetSettings { .. }
                | BridgeCommand::GithubSignIn
                | BridgeCommand::RetrySite
        ),
    }
}

/// Why a command failed, as the page sees it: a fixed word and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BridgeErrorCode {
    /// The message or its arguments are not valid.
    BadRequest,
    /// This page may not send this command.
    NotAllowed,
    /// The link is not a Play link, or is not valid.
    BadLink,
    /// The product is not installed.
    NotInstalled,
    /// Not available in this build or on this PC.
    NotAvailable,
    /// Another task is running; try again later.
    Busy,
    /// The Launcher must update first.
    UpdateRequired,
    /// It did not work.
    Failed,
}

/// What the Launcher tells the page about itself.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherInfo {
    /// The Launcher's version.
    pub launcher_version: String,
    /// The bridge version, [`BRIDGE_VERSION`].
    pub bridge_version: u32,
    /// The operating system, `"windows"`.
    pub platform: &'static str,
    /// The update channel.
    pub channel: String,
    /// True in developer mode.
    pub dev_mode: bool,
}

/// The things the bridge can do. The shell implements this; tests use a recording fake.
/// Methods return fixed error codes, never text.
pub trait BridgeHost {
    /// About this Launcher.
    fn info(&self) -> LauncherInfo;
    /// Starts the Player with a Play link.
    fn play(&self, link: DeepLink) -> Result<(), BridgeErrorCode>;
    /// What is installed, in the shape the site reads (`{player, studio, launcher}`).
    fn install_status(&self) -> Value;
    /// Starts installing Metallic Studio.
    fn install_studio(&self) -> Result<(), BridgeErrorCode>;
    /// Opens Metallic Studio.
    fn open_studio(&self, place: Option<PlaceId>) -> Result<(), BridgeErrorCode>;
    /// The current settings.
    fn settings(&self) -> LauncherSettings;
    /// Applies a change and returns the new settings.
    fn update_settings(&self, patch: SettingsPatch) -> Result<LauncherSettings, BridgeErrorCode>;
    /// Starts signing in; the value carries what the page shows (never a token).
    fn sign_in(&self) -> Result<Value, BridgeErrorCode>;
    /// Signs out.
    fn sign_out(&self) -> Result<(), BridgeErrorCode>;
    /// Starts signing in to GitHub; the value carries what the page shows (never a token).
    fn github_sign_in(&self) -> Result<Value, BridgeErrorCode>;
    /// Checks the site again now.
    fn retry_site(&self);
    /// Restarts into a downloaded update.
    fn restart_to_update(&self) -> Result<(), BridgeErrorCode>;
}

/// Something the Launcher tells the pages without being asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BridgeEvent {
    /// An install is running.
    InstallProgress {
        /// `"player"` or `"studio"`.
        product: String,
        /// 0 to 100 (larger values are shown as 100).
        percent: u8,
    },
    /// An install ended.
    InstallFinished {
        /// `"player"` or `"studio"`.
        product: String,
        /// True when it worked.
        ok: bool,
    },
    /// The sign-in moved on.
    SignIn {
        /// One of `pending`, `signedIn`, `denied`, `expired`, `failed`.
        state: String,
        /// The code the user types in the browser, while pending.
        #[serde(skip_serializing_if = "Option::is_none")]
        user_code: Option<String>,
        /// The page where the code is typed (on the site's origin), while pending.
        #[serde(skip_serializing_if = "Option::is_none")]
        verification_uri: Option<String>,
    },
    /// The GitHub sign-in for team builds moved on. Only bundled pages receive it.
    GithubSignIn {
        /// One of `pending`, `signedIn`, `denied`, `expired`, `failed`.
        state: String,
        /// The code the user types on GitHub's page, while pending.
        #[serde(skip_serializing_if = "Option::is_none")]
        user_code: Option<String>,
    },
    /// A Launcher update is ready.
    UpdateReady {
        /// The new version.
        version: String,
        /// True when the Launcher must restart before it can be used.
        required: bool,
    },
    /// The site went down or came back.
    SiteStatus {
        /// True when the site answers.
        up: bool,
    },
}

/// The sign-in states a page may be told about.
const SIGN_IN_STATES: [&str; 5] = ["pending", "signedIn", "denied", "expired", "failed"];

/// Counts a call while the host runs it; gives its place back when dropped.
struct InFlight(Arc<AtomicUsize>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// An answer waiting to be delivered to the page that asked.
pub struct PendingReply {
    id: u32,
    /// Where the asking page came from; the answer goes only to a page with this origin.
    origin_key: String,
    /// The `location.origin` values the answer may run in (checked in the page as well).
    guard: Vec<String>,
    outcome: Result<Value, BridgeErrorCode>,
}

/// Prints the call id only: the value may hold anything the host returned.
impl fmt::Debug for PendingReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PendingReply(id {})", self.id)
    }
}

/// The checked path from page messages to the [`BridgeHost`] and back.
pub struct Bridge {
    policy: UrlPolicy,
    in_flight: Arc<AtomicUsize>,
    last_play: Mutex<Option<Instant>>,
    /// Budget for all messages: about 20 a second, bursts of 40.
    budget: Mutex<Bucket>,
    /// Budget for commands that write to disk (`setSettings`): 2 a second.
    write_budget: Mutex<Bucket>,
}

/// A token bucket: `rate` tokens a second up to `capacity`.
struct Bucket {
    tokens: f64,
    rate: f64,
    capacity: f64,
    last: Instant,
}

impl Bucket {
    fn new(rate: f64, capacity: f64) -> Self {
        Bucket {
            tokens: capacity,
            rate,
            capacity,
            last: Instant::now(),
        }
    }

    /// Takes one token if there is one.
    fn take(&mut self) -> bool {
        let now = Instant::now();
        let gained = now.duration_since(self.last).as_secs_f64() * self.rate;
        self.tokens = (self.tokens + gained).min(self.capacity);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

fn take_from(bucket: &Mutex<Bucket>) -> bool {
    // A poisoned lock counts as "over budget".
    bucket.lock().is_ok_and(|mut bucket| bucket.take())
}

/// The name of a command in a log line: letters and digits only, 32 at most, so a hostile
/// message cannot write what it likes into the log.
fn loggable_name(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(|name| {
            name.chars()
                .filter(char::is_ascii_alphanumeric)
                .take(32)
                .collect()
        })
        .unwrap_or_default()
}

/// Turns a JSON value into text that is safe inside a script: besides JSON's own escaping,
/// `<`, `>`, `&` and the line separators U+2028 and U+2029 are written as `\uXXXX`. These
/// characters can only occur inside JSON strings, so the meaning is unchanged.
fn script_json(value: &Value) -> String {
    let text = value.to_string();
    let mut safe = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                safe.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => safe.push(c),
        }
    }
    safe
}

/// Wraps a call into the page's bridge so the page itself refuses to run it unless its
/// `location.origin` is one of `origins`. The check comes first: the script has been approved
/// for the page the view showed a moment ago, but it runs later, and a page that navigated in
/// between must not see it. `window.__metallicBridge` is defined by `bridge.js` at document
/// creation as a non-writable, non-configurable property, so on a page that passes the check
/// it cannot have been replaced by page code.
fn guarded_script(origins: &[String], call: &str) -> String {
    let list = script_json(&Value::from(origins.to_vec()));
    format!("(function(){{if({list}.indexOf(location.origin)===-1)return;{call}}})()")
}

/// Reads `{"cmd": .., "args": ..}` from a message. Extra top-level fields are refused. A missing
/// or `{}` argument object is accepted where the command takes none or only optional ones.
fn parse_command(message: &Map<String, Value>) -> Result<BridgeCommand, BridgeErrorCode> {
    if message
        .keys()
        .any(|key| !matches!(key.as_str(), "id" | "cmd" | "args"))
    {
        return Err(BridgeErrorCode::BadRequest);
    }
    let cmd = message
        .get("cmd")
        .filter(|cmd| cmd.is_string())
        .ok_or(BridgeErrorCode::BadRequest)?;
    let args = message.get("args");
    let mut shapes: Vec<Option<Value>> = vec![args.cloned()];
    match args {
        None | Some(Value::Null) => shapes.push(Some(json!({}))),
        Some(Value::Object(map)) if map.is_empty() => shapes.push(None),
        _ => {}
    }
    for shape in shapes {
        let mut candidate = Map::new();
        candidate.insert("cmd".to_string(), cmd.clone());
        if let Some(args) = shape {
            candidate.insert("args".to_string(), args);
        }
        if let Ok(command) = serde_json::from_value(Value::Object(candidate)) {
            return Ok(command);
        }
    }
    Err(BridgeErrorCode::BadRequest)
}

/// The settings a page of this kind may see. Site pages see `closeToTray` only.
fn settings_view(origin: PageOrigin, settings: &LauncherSettings) -> Value {
    if origin == PageOrigin::Local {
        json!({
            "closeToTray": settings.close_to_tray,
            "developerMode": settings.developer_mode,
            "devUrl": settings.dev_url,
            "channel": settings.channel.as_str(),
        })
    } else {
        json!({ "closeToTray": settings.close_to_tray })
    }
}

fn empty() -> Value {
    json!({})
}

fn done(result: Result<(), BridgeErrorCode>) -> Result<Value, BridgeErrorCode> {
    result.map(|()| empty())
}

impl Bridge {
    /// A bridge that trusts the pages `policy` calls the site or bundled pages.
    pub fn new(policy: UrlPolicy) -> Self {
        Bridge {
            policy,
            in_flight: Arc::new(AtomicUsize::new(0)),
            last_play: Mutex::new(None),
            budget: Mutex::new(Bucket::new(MESSAGES_PER_SECOND, MESSAGE_BURST)),
            write_budget: Mutex::new(Bucket::new(WRITES_PER_SECOND, WRITES_PER_SECOND)),
        }
    }

    /// The script the shell injects into the main frame before any page script runs: `bridge.js`
    /// with the trusted origins filled in. The script's own origin check is a convenience; the
    /// Rust checks decide.
    pub fn init_script(&self) -> String {
        let origins = Value::from(self.policy.bridge_origins());
        BRIDGE_JS.replace(ORIGINS_MARKER, &script_json(&origins))
    }

    /// The key that tells whether two addresses belong to the same trusted origin, or `None`
    /// for an address that may not use the bridge.
    fn origin_key(&self, url: &str) -> Option<String> {
        match self.policy.origin_of(url) {
            PageOrigin::Other => None,
            PageOrigin::Local => Some("local".to_string()),
            PageOrigin::Site => Url::parse(url)
                .ok()
                .map(|parsed| parsed.origin().ascii_serialization()),
        }
    }

    /// The `location.origin` values a page at `url` may be answered at, or `None` for a page that
    /// may not use the bridge. Bundled pages are `http(s)://metallic-app.localhost` or
    /// `metallic-app://local`, depending on the platform.
    fn guard_origins(&self, url: &str) -> Option<Vec<String>> {
        match self.policy.origin_of(url) {
            PageOrigin::Other => None,
            PageOrigin::Local => Some(
                [
                    "http://metallic-app.localhost",
                    "https://metallic-app.localhost",
                    "metallic-app://local",
                ]
                .map(String::from)
                .to_vec(),
            ),
            PageOrigin::Site => Url::parse(url)
                .ok()
                .map(|parsed| vec![parsed.origin().ascii_serialization()]),
        }
    }

    /// Handles one message from the page at `page_url`. Returns `None` (and logs the origin kind
    /// and, when readable, the command name; never the message) when the page may not use the
    /// bridge, the message is too long, or it has no readable id. Otherwise returns the answer,
    /// which may be an error code, to give to [`deliver`](Self::deliver).
    pub fn handle(
        &self,
        page_url: &str,
        body: &str,
        host: &dyn BridgeHost,
    ) -> Option<PendingReply> {
        if body.len() > MAX_MESSAGE_BYTES {
            log::warn!("dropped an oversized bridge message");
            return None;
        }
        let origin = self.policy.origin_of(page_url);
        let (Some(origin_key), Some(guard)) =
            (self.origin_key(page_url), self.guard_origins(page_url))
        else {
            log::warn!("dropped a bridge message from a {origin:?} page");
            return None;
        };
        // From here on the page is the site or a bundled page; the body is still untrusted.
        let Ok(Value::Object(message)) = serde_json::from_str::<Value>(body) else {
            log::warn!("dropped an unreadable bridge message from a {origin:?} page");
            return None;
        };
        let Some(id) = message
            .get("id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
        else {
            log::warn!("dropped a bridge message without a usable id from a {origin:?} page");
            return None;
        };
        let name = loggable_name(message.get("cmd"));

        if !take_from(&self.budget) {
            log::warn!("a {origin:?} page sends bridge messages too fast; answered busy");
            return Some(PendingReply {
                id,
                origin_key,
                guard,
                outcome: Err(BridgeErrorCode::Busy),
            });
        }

        // The place is held while the host works and is given back before the answer is
        // delivered, so an answer that waits (or a long job the host runs in the background)
        // never takes a place from other calls.
        let reserved = self.in_flight.fetch_add(1, Ordering::SeqCst);
        let outcome = {
            let _place = InFlight(self.in_flight.clone());
            if reserved >= MAX_IN_FLIGHT {
                log::warn!("too many bridge calls running; answered busy");
                Err(BridgeErrorCode::Busy)
            } else {
                self.run(origin, &message, &name, host)
            }
        };
        Some(PendingReply {
            id,
            origin_key,
            guard,
            outcome,
        })
    }

    /// Checks and carries out one command.
    fn run(
        &self,
        origin: PageOrigin,
        message: &Map<String, Value>,
        name: &str,
        host: &dyn BridgeHost,
    ) -> Result<Value, BridgeErrorCode> {
        let command = parse_command(message).inspect_err(|_| {
            log::warn!("refused a bad bridge message from a {origin:?} page (command {name:?})");
        })?;
        if !allowed(origin, &command) {
            log::warn!("a {origin:?} page may not send {}", command.name());
            return Err(BridgeErrorCode::NotAllowed);
        }
        match command {
            BridgeCommand::GetInfo => Ok(json!(host.info())),
            BridgeCommand::Play { launch_uri } => {
                let link = match DeepLink::parse(&launch_uri) {
                    Ok(link @ DeepLink::Play { .. }) => link,
                    _ => return Err(BridgeErrorCode::BadLink),
                };
                self.take_play_slot()?;
                done(host.play(link))
            }
            BridgeCommand::InstallStatus => Ok(host.install_status()),
            BridgeCommand::InstallStudio => done(host.install_studio()),
            BridgeCommand::OpenStudio { place } => {
                let place = match place {
                    Some(text) => Some(PlaceId::parse(&text).ok_or(BridgeErrorCode::BadRequest)?),
                    None => None,
                };
                done(host.open_studio(place))
            }
            BridgeCommand::GetSettings => Ok(settings_view(origin, &host.settings())),
            BridgeCommand::SetSettings { patch } => {
                if !take_from(&self.write_budget) {
                    log::warn!("settings are changed too often; answered busy");
                    return Err(BridgeErrorCode::Busy);
                }
                host.update_settings(patch)
                    .map(|settings| settings_view(origin, &settings))
            }
            BridgeCommand::SignIn => host.sign_in(),
            BridgeCommand::SignOut => done(host.sign_out()),
            BridgeCommand::GithubSignIn => host.github_sign_in(),
            BridgeCommand::RetrySite => {
                host.retry_site();
                Ok(empty())
            }
            BridgeCommand::RestartToUpdate => done(host.restart_to_update()),
        }
    }

    /// Allows one Player start per [`PLAY_INTERVAL`]; `Busy` otherwise.
    fn take_play_slot(&self) -> Result<(), BridgeErrorCode> {
        let mut last = self.last_play.lock().map_err(|_| BridgeErrorCode::Failed)?;
        let now = Instant::now();
        if last.is_some_and(|before| now.duration_since(before) < PLAY_INTERVAL) {
            return Err(BridgeErrorCode::Busy);
        }
        *last = Some(now);
        Ok(())
    }

    /// The script that gives `reply` to the page now showing at `current_page_url`, or `None`
    /// when that page does not have the same origin as the page that asked (it navigated away,
    /// or the origin changed). Run the script only when this returns `Some`.
    pub fn deliver(&self, reply: PendingReply, current_page_url: &str) -> Option<String> {
        if self.origin_key(current_page_url).as_deref() != Some(reply.origin_key.as_str()) {
            log::info!("dropped a bridge answer: the page changed");
            return None;
        }
        let answer = match &reply.outcome {
            Ok(value) => json!({ "ok": value }),
            Err(code) => json!({ "error": code }),
        };
        Some(guarded_script(
            &reply.guard,
            &format!(
                "window.__metallicBridge.resolve({},{})",
                reply.id,
                script_json(&answer)
            ),
        ))
    }

    /// Wraps any script the shell runs into the page so it runs only in a page whose
    /// `location.origin` is the one now showing at `current_page_url`, or `None` when that page
    /// may not use the bridge (another origin, a frame-less `about:` page, a dangerous scheme).
    /// Every script the shell evaluates must go through here or through
    /// [`deliver`](Self::deliver) / [`event_script`](Self::event_script).
    pub fn guard_script(&self, script: &str, current_page_url: &str) -> Option<String> {
        let guard = self.guard_origins(current_page_url)?;
        Some(guarded_script(&guard, script))
    }

    /// The script that tells the page now showing at `current_page_url` about `event`. Only the
    /// site and bundled pages get events. A sign-in state other than the five known ones is not
    /// sent.
    pub fn event_script(&self, event: &BridgeEvent, current_page_url: &str) -> Option<String> {
        let guard = self.guard_origins(current_page_url)?;
        let event = match event {
            BridgeEvent::SignIn { state, .. } | BridgeEvent::GithubSignIn { state, .. }
                if !SIGN_IN_STATES.contains(&state.as_str()) =>
            {
                log::warn!("not sending a sign-in event with an unknown state");
                return None;
            }
            // The GitHub code is for the Launcher's own pages; the site never hears of it.
            BridgeEvent::GithubSignIn { .. }
                if self.policy.origin_of(current_page_url) != PageOrigin::Local =>
            {
                return None;
            }
            BridgeEvent::InstallProgress { product, percent } => BridgeEvent::InstallProgress {
                product: product.clone(),
                percent: (*percent).min(100),
            },
            other => other.clone(),
        };
        Some(guarded_script(
            &guard,
            &format!(
                "window.__metallicBridge.emit({})",
                script_json(&json!(event))
            ),
        ))
    }
}
