//! Installing Metallic Studio, and telling what is installed.
//!
//! [`Installer::install_studio`] looks up `MetallicStudio-<channel>-Setup.exe` on the Studio
//! release feed, downloads it with the SHA-256 check of [`metallic_update::download_verified`],
//! runs it through a [`Spawner`], waits for it, and deletes it. The installer only ever runs
//! after the download has been verified; any failure before that leaves no file behind. One
//! install runs at a time.
//!
//! The Player is not installed here: it ships beside the Launcher and is updated with it, so
//! [`player_status`] only reports whether it is there.
//!
//! [`github_sign_in`] runs the GitHub device sign-in the private team feed needs and reports it
//! as [`BridgeEvent::GithubSignIn`] events.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use metallic_update::github_auth::DeviceFlowUi;
use metallic_update::{
    AuthError, Channel, GithubAuth, HttpClient, ReleaseAsset, STUDIO_FEED, TokenProvider,
    UpdateChecker, UpdateError, download_verified,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::app_event::{AppEvent, EventSink};
use crate::bridge::BridgeEvent;
use crate::error::LauncherError;
use crate::launch::{ProductPaths, Spawner};
use crate::platform_auth::OneAtATime;

/// How long Studio's installer may run before it is killed and the install counts as failed.
pub const INSTALLER_TIME_LIMIT: Duration = Duration::from_secs(10 * 60);

/// What the Launcher knows about the Player or Metallic Studio on this PC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ProductStatus {
    /// Not on this PC.
    NotInstalled,
    /// Installed.
    Installed {
        /// The installed version.
        version: String,
    },
    /// An install is running.
    Installing {
        /// How far the download is, 0 to 100.
        percent: u8,
    },
    /// The last install did not work.
    Failed {
        /// A short reason, safe to show (never an address or a token).
        reason: String,
    },
}

/// Where the Launcher's own update stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum LauncherUpdateState {
    /// This copy was not installed with the installer (a build from `target\`), so it cannot
    /// update itself.
    NotInstalledCopy,
    /// Nothing newer is published.
    UpToDate,
    /// Looking for an update.
    Checking,
    /// An update is downloading.
    Downloading {
        /// 0 to 100.
        percent: u8,
    },
    /// An update is downloaded; it is applied when the Launcher closes (or on "Restart to update").
    ReadyToRestart {
        /// The new version.
        version: String,
        /// True when the Launcher must update before it can start the Player.
        required: bool,
    },
    /// The team feed refused: sign in to GitHub first.
    NeedsGithubSignIn,
    /// The last check or download did not work.
    Failed {
        /// A short reason, safe to show.
        reason: String,
    },
}

/// Everything `installStatus` tells the page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstallStatus {
    /// The Player.
    pub player: ProductStatus,
    /// Metallic Studio.
    pub studio: ProductStatus,
    /// The Launcher's own update.
    pub launcher: LauncherUpdateState,
}

/// The version of the app installed at `app_root` (`%LOCALAPPDATA%\MetallicStudio` for Studio):
/// the `<version>` element of `current\sq.version`, or `None` when the folder, the file or the
/// element is missing or unusable.
pub fn read_installed_version(app_root: &Path) -> Option<String> {
    let file = std::fs::File::open(app_root.join("current").join("sq.version")).ok()?;
    let mut text = String::new();
    // A version file is a few hundred bytes; never read an unbounded file.
    file.take(64 * 1024).read_to_string(&mut text).ok()?;
    let start = text.find("<version>")? + "<version>".len();
    let end = start + text[start..].find("</version>")?;
    let version = text[start..end].trim();
    let usable = !version.is_empty()
        && version.len() <= 64
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    usable.then(|| version.to_string())
}

/// The Player's status: it ships with the Launcher, so it has the Launcher's version.
pub fn player_status(paths: &ProductPaths) -> ProductStatus {
    if paths.player_exe.is_file() {
        ProductStatus::Installed {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    } else {
        ProductStatus::NotInstalled
    }
}

/// Studio's install folder under `%LOCALAPPDATA%`.
pub fn studio_root_for(local_app_data: &Path) -> PathBuf {
    local_app_data.join("MetallicStudio")
}

/// Where installers are downloaded: `%LOCALAPPDATA%\Metallic\Launcher\Downloads`.
pub fn downloads_dir_for(local_app_data: &Path) -> PathBuf {
    local_app_data
        .join("Metallic")
        .join("Launcher")
        .join("Downloads")
}

/// The name of Studio's installer for `channel`, as Velopack names it.
pub fn studio_setup_name(channel: Channel) -> String {
    format!("MetallicStudio-{}-Setup.exe", channel.as_str())
}

/// Finds and downloads release files. [`GithubAssets`] is the real one; tests use a fake so no
/// test touches the network.
pub trait AssetSource: Send + Sync {
    /// The newest release file called `file_name` on `channel`.
    fn latest_asset(
        &self,
        channel: Channel,
        file_name: &str,
    ) -> Result<Option<ReleaseAsset>, UpdateError>;

    /// Downloads `asset` into `dir`, checks it, and returns its path. Must leave no file in
    /// `dir` when it fails.
    fn download(
        &self,
        asset: &ReleaseAsset,
        dir: &Path,
        progress: &mut dyn FnMut(u8),
    ) -> Result<PathBuf, UpdateError>;
}

/// [`AssetSource`] on the Studio release feed (GitHub Releases), signed in with `tokens`.
pub struct GithubAssets {
    tokens: Arc<dyn TokenProvider>,
    http: Arc<dyn HttpClient>,
}

impl GithubAssets {
    /// Uses `tokens` for the private feed and `http` for every request.
    pub fn new(tokens: Arc<dyn TokenProvider>, http: Arc<dyn HttpClient>) -> Self {
        GithubAssets { tokens, http }
    }
}

impl AssetSource for GithubAssets {
    fn latest_asset(
        &self,
        channel: Channel,
        file_name: &str,
    ) -> Result<Option<ReleaseAsset>, UpdateError> {
        UpdateChecker::new(STUDIO_FEED, channel, self.tokens.clone())
            .with_http(self.http.clone())
            .latest_asset(file_name)
    }

    fn download(
        &self,
        asset: &ReleaseAsset,
        dir: &Path,
        progress: &mut dyn FnMut(u8),
    ) -> Result<PathBuf, UpdateError> {
        let token = self.tokens.token();
        download_verified(self.http.as_ref(), token.as_ref(), asset, dir, progress)
    }
}

/// Turns an update failure into the Launcher's error, with a fixed phrase that holds no address
/// and no token.
pub fn launcher_error(error: &UpdateError) -> LauncherError {
    let phrase = match error {
        UpdateError::Unauthorized => return LauncherError::NeedsGithubSignIn,
        UpdateError::Network(_) => "could not reach the update server",
        UpdateError::BadResponse(_) => "the update server sent something unexpected",
        UpdateError::NotInstalled(_) => "this copy cannot update itself",
        UpdateError::Install(_) => "the download or install failed",
        UpdateError::BadConfig(_) => "the update settings are not valid",
    };
    LauncherError::Update(phrase.to_string())
}

#[derive(Default)]
struct StudioState {
    running: bool,
    percent: u8,
    failure: Option<String>,
}

/// Installs Metallic Studio, one install at a time.
pub struct Installer {
    source: Arc<dyn AssetSource>,
    spawner: Arc<dyn Spawner + Send + Sync>,
    channel: Channel,
    studio_root: PathBuf,
    downloads: PathBuf,
    time_limit: Duration,
    state: Mutex<StudioState>,
}

/// Ends an install however the code leaves: the normal way reports the outcome with
/// [`complete`](Self::complete); any other way out (an early return, a panic) frees the place
/// and reports a failure, so the Installer can never stay "running" for good.
struct RunGuard<'a> {
    installer: &'a Installer,
    sink: &'a dyn EventSink,
    done: bool,
}

impl RunGuard<'_> {
    fn complete(&mut self, outcome: Result<(), &LauncherError>) {
        self.done = true;
        self.installer.finish(outcome);
        self.sink
            .send(AppEvent::Bridge(BridgeEvent::InstallFinished {
                product: "studio".to_string(),
                ok: outcome.is_ok(),
            }));
    }
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            log::error!("the Studio install ended without a result");
            self.complete(Err(&LauncherError::Update(
                "the install failed".to_string(),
            )));
        }
    }
}

/// Reads `file` from the start and says whether its SHA-256 is `expected`.
fn file_has_sha256(mut file: &std::fs::File, expected: &[u8; 32]) -> std::io::Result<bool> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    Ok(&digest == expected)
}

impl Installer {
    /// An installer that follows `channel`, downloads into `downloads` and looks for Studio in
    /// `studio_root`.
    pub fn new(
        source: Arc<dyn AssetSource>,
        spawner: Arc<dyn Spawner + Send + Sync>,
        channel: Channel,
        studio_root: PathBuf,
        downloads: PathBuf,
    ) -> Self {
        Installer {
            source,
            spawner,
            channel,
            studio_root,
            downloads,
            time_limit: INSTALLER_TIME_LIMIT,
            state: Mutex::new(StudioState::default()),
        }
    }

    /// Replaces how long the installer may run ([`INSTALLER_TIME_LIMIT`]).
    pub fn with_time_limit(mut self, time_limit: Duration) -> Self {
        self.time_limit = time_limit;
        self
    }

    /// Studio's status: installing while an install runs, else the last failure until the next
    /// attempt, else what is on disk.
    pub fn studio_status(&self) -> ProductStatus {
        if let Ok(state) = self.state.lock() {
            if state.running {
                return ProductStatus::Installing {
                    percent: state.percent,
                };
            }
            if let Some(reason) = &state.failure {
                return ProductStatus::Failed {
                    reason: reason.clone(),
                };
            }
        }
        match read_installed_version(&self.studio_root) {
            Some(version) => ProductStatus::Installed { version },
            None => ProductStatus::NotInstalled,
        }
    }

    /// Takes the one install place, or says another install is running.
    fn claim(&self) -> Result<(), LauncherError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| LauncherError::Update("internal error".to_string()))?;
        if state.running {
            return Err(LauncherError::Busy("an install"));
        }
        *state = StudioState {
            running: true,
            percent: 0,
            failure: None,
        };
        Ok(())
    }

    /// Installs Studio and waits until it is done. Progress and the end are posted to `sink` as
    /// `InstallProgress` and `InstallFinished` events. Returns `Busy` at once when an install is
    /// already running.
    pub fn install_studio(&self, sink: &dyn EventSink) -> Result<(), LauncherError> {
        self.claim()?;
        self.run_claimed(sink)
    }

    /// Like [`install_studio`](Self::install_studio) but does the work on a new thread and
    /// returns as soon as the install place is taken (`Busy` otherwise). This is what the
    /// bridge uses, so a long install never holds up the window.
    pub fn start_install_studio(
        self: &Arc<Self>,
        sink: Arc<dyn EventSink>,
    ) -> Result<(), LauncherError> {
        self.claim()?;
        let installer = self.clone();
        let spawned = std::thread::Builder::new()
            .name("studio-install".to_string())
            .spawn(move || {
                // The outcome is in the status and the events; the thread has nobody to return it to.
                let _ = installer.run_claimed(sink.as_ref());
            });
        if let Err(error) = spawned {
            self.finish(Err(&LauncherError::Io(error.to_string())));
            return Err(LauncherError::Io(error.to_string()));
        }
        Ok(())
    }

    fn finish(&self, outcome: Result<(), &LauncherError>) {
        if let Ok(mut state) = self.state.lock() {
            state.running = false;
            state.failure = match outcome {
                Ok(()) => None,
                Err(LauncherError::NeedsGithubSignIn) => {
                    Some("Sign in to GitHub for team builds first.".to_string())
                }
                Err(LauncherError::Update(phrase)) => Some(phrase.clone()),
                Err(_) => Some("the install failed".to_string()),
            };
        }
    }

    fn run_claimed(&self, sink: &dyn EventSink) -> Result<(), LauncherError> {
        let mut guard = RunGuard {
            installer: self,
            sink,
            done: false,
        };
        let result = self.do_install(sink);
        if let Err(error) = &result {
            log::warn!("the Studio install failed: {error}");
        }
        guard.complete(result.as_ref().map(|_| ()));
        result
    }

    fn do_install(&self, sink: &dyn EventSink) -> Result<(), LauncherError> {
        let file_name = studio_setup_name(self.channel);
        let asset = self
            .source
            .latest_asset(self.channel, &file_name)
            .map_err(|e| launcher_error(&e))?
            .ok_or_else(|| {
                LauncherError::Update("no Studio installer has been published yet".to_string())
            })?;
        let mut last = None;
        let setup = self
            .source
            .download(&asset, &self.downloads, &mut |percent| {
                if last != Some(percent) {
                    last = Some(percent);
                    if let Ok(mut state) = self.state.lock() {
                        state.percent = percent;
                    }
                    sink.send(AppEvent::Bridge(BridgeEvent::InstallProgress {
                        product: "studio".to_string(),
                        percent,
                    }));
                }
            })
            .map_err(|e| launcher_error(&e))?;
        // The download was checked, but the file could still be swapped before it runs. Open it
        // with writing and deleting denied, check the hash again from that handle, and keep the
        // handle until the installer has ended.
        let locked = self.spawner.open_locked(&setup)?;
        match file_has_sha256(&locked, &asset.sha256) {
            Ok(true) => {}
            Ok(false) => {
                drop(locked);
                let _ = std::fs::remove_file(&setup);
                return Err(LauncherError::Update(
                    "the installer changed after the download".to_string(),
                ));
            }
            Err(error) => {
                drop(locked);
                let _ = std::fs::remove_file(&setup);
                return Err(LauncherError::Io(error.to_string()));
            }
        }
        // Only now, with a verified file, does the installer run. It is deleted whatever happens.
        // TODO(Task 9): `--silent` for Velopack's Setup.exe is from memory; check with a real run
        // (no dialog, exit code 0, Studio not started afterwards, `sq.version` written).
        let outcome =
            self.spawner
                .spawn_and_wait(&setup, &[OsString::from("--silent")], self.time_limit);
        drop(locked);
        if let Err(error) = std::fs::remove_file(&setup) {
            log::warn!("could not delete the Studio installer: {error}");
        }
        match outcome? {
            0 => {}
            code => {
                log::warn!("the Studio installer ended with code {code}");
                return Err(LauncherError::Update("the installer did not finish".into()));
            }
        }
        if read_installed_version(&self.studio_root).is_none() {
            return Err(LauncherError::Update(
                "Studio is not installed after the installer ended".into(),
            ));
        }
        Ok(())
    }
}

/// Shows the device-flow code as bridge events and opens the verification page through `open`.
struct EventUi<'a> {
    sink: &'a dyn EventSink,
    open: &'a dyn Fn(&str),
}

impl DeviceFlowUi for EventUi<'_> {
    fn show_code(&mut self, user_code: &str, verification_uri: &str) {
        self.sink.send(AppEvent::Bridge(BridgeEvent::GithubSignIn {
            state: "pending".to_string(),
            user_code: Some(user_code.to_string()),
        }));
        (self.open)(verification_uri);
    }
}

/// Starts the GitHub sign-in for team builds on a new thread. The code the user types arrives as
/// a `githubSignIn` event with state `pending`, `open` is called with the page to show (the
/// shell opens it in the system browser), and the end is a state of `signedIn`, `denied`,
/// `expired` or `failed`. `done` is called with `true` when the user signed in. Only one
/// GitHub sign-in runs at a time: while `guard` has one, this is [`LauncherError::Busy`].
pub fn github_sign_in(
    auth: Arc<GithubAuth>,
    sink: Arc<dyn EventSink>,
    open: Arc<dyn Fn(&str) + Send + Sync>,
    guard: &OneAtATime,
    done: impl FnOnce(bool) + Send + 'static,
) -> Result<(), LauncherError> {
    let ticket = guard.try_begin().ok_or(LauncherError::Busy("sign-in"))?;
    std::thread::Builder::new()
        .name("github-sign-in".to_string())
        .spawn(move || {
            let _ticket = ticket;
            let mut ui = EventUi {
                sink: sink.as_ref(),
                open: open.as_ref(),
            };
            let result = auth.device_sign_in(&mut ui);
            if let Err(error) = &result {
                log::warn!("the GitHub sign-in did not finish: {error}");
            }
            let state = match &result {
                Ok(_) => "signedIn",
                Err(AuthError::Denied) => "denied",
                Err(AuthError::Expired) => "expired",
                Err(_) => "failed",
            };
            sink.send(AppEvent::Bridge(BridgeEvent::GithubSignIn {
                state: state.to_string(),
                user_code: None,
            }));
            done(result.is_ok());
        })
        .map(|_| ())
        .map_err(|e| LauncherError::Io(e.to_string()))
}
