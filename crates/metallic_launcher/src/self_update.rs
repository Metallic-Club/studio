//! The Launcher's own update: check on start and every [`CHECK_INTERVAL`], download in the
//! background, apply when the Launcher closes.
//!
//! The update is applied only when no Player started by this Launcher is running, because the
//! update replaces the files the Player is started from. If a Player runs, the downloaded update
//! stays ready for the next quit. "Restart to update" (from the page or the tray) follows the
//! same rule and answers [`RestartError::Busy`] instead of waiting. A `required` update makes
//! [`SelfUpdater::allow_play`] refuse until it is applied.
//!
//! The Velopack and GitHub parts sit behind [`UpdateSource`], so the rules here are tested with
//! a fake and no test touches the network.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};

use metallic_update::{CHECK_INTERVAL, UpdateChecker, UpdateError, UpdateInfo};

use crate::app_event::{AppEvent, EventSink};
use crate::bridge::BridgeEvent;
use crate::error::LauncherError;
use crate::install::LauncherUpdateState;

/// Counts the Player processes that are running.
pub trait ProcessTracker: Send + Sync {
    /// How many Players are running now.
    fn running_players(&self) -> usize;
}

/// Lists the programs running on this PC. The real one (`platform`) takes a Windows process
/// snapshot; tests use a fixed list.
pub trait ProcessLister: Send + Sync {
    /// The full path of every running program's image, or `None` when the list cannot be read.
    fn running_images(&self) -> Option<Vec<PathBuf>>;
}

/// Counts running Players by image path: any process that runs the Player's `.exe`, whoever
/// started it (this Launcher, an earlier one, a double-click). A Player that cannot be told
/// apart from the others because the list is unreadable counts as running, so an update waits
/// rather than replacing files under a Player.
pub struct PlayerWatcher {
    lister: Arc<dyn ProcessLister>,
    player_exe: PathBuf,
}

impl PlayerWatcher {
    /// Watches for processes running `player_exe`.
    pub fn new(lister: Arc<dyn ProcessLister>, player_exe: PathBuf) -> Self {
        PlayerWatcher { lister, player_exe }
    }
}

/// A path as text that compares equal across case and slash style (Windows paths).
fn comparable(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

impl ProcessTracker for PlayerWatcher {
    fn running_players(&self) -> usize {
        if self.player_exe.as_os_str().is_empty() {
            return 0;
        }
        let Some(images) = self.lister.running_images() else {
            return 1;
        };
        let wanted = comparable(&self.player_exe);
        images
            .iter()
            .filter(|image| comparable(image) == wanted)
            .count()
    }
}

/// Where updates come from. [`UpdateChecker`] is the real one.
pub trait UpdateSource: Send + Sync {
    /// The newest update for this Launcher, if there is one.
    fn check(&self) -> Result<Option<UpdateInfo>, UpdateError>;
    /// Downloads `update`, reporting percent.
    fn download(
        &self,
        update: &UpdateInfo,
        progress: &mut dyn FnMut(u8),
    ) -> Result<(), UpdateError>;
    /// Arranges for the downloaded update to be applied once this process has exited; with
    /// `restart` the Launcher starts again afterwards. Does not exit.
    fn apply_after_exit(&self, restart: bool) -> Result<(), UpdateError>;
}

impl UpdateSource for UpdateChecker {
    fn check(&self) -> Result<Option<UpdateInfo>, UpdateError> {
        UpdateChecker::check(self)
    }

    fn download(
        &self,
        update: &UpdateInfo,
        progress: &mut dyn FnMut(u8),
    ) -> Result<(), UpdateError> {
        UpdateChecker::download(self, update, progress)
    }

    fn apply_after_exit(&self, restart: bool) -> Result<(), UpdateError> {
        UpdateChecker::apply_after_exit(self, restart)
    }
}

/// Why "restart to update" did not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartError {
    /// A Player is running; the update waits until it closes.
    Busy,
    /// No update is downloaded.
    NotReady,
    /// The updater could not be started.
    Failed,
}

struct Inner {
    state: LauncherUpdateState,
    /// The updater has been told to apply the update when this process exits.
    scheduled: bool,
    /// Wakes the check thread for an early check.
    wake: Option<Sender<()>>,
}

/// Checks for Launcher updates and applies them at the right moment.
pub struct SelfUpdater {
    source: Arc<dyn UpdateSource>,
    sink: Arc<dyn EventSink>,
    inner: Mutex<Inner>,
}

impl SelfUpdater {
    /// An updater for `source`. A copy that was not installed with the installer
    /// (`installed == false`) never checks and reports `NotInstalledCopy`.
    pub fn new(source: Arc<dyn UpdateSource>, sink: Arc<dyn EventSink>, installed: bool) -> Self {
        SelfUpdater {
            source,
            sink,
            inner: Mutex::new(Inner {
                state: if installed {
                    LauncherUpdateState::UpToDate
                } else {
                    LauncherUpdateState::NotInstalledCopy
                },
                scheduled: false,
                wake: None,
            }),
        }
    }

    /// Where the update stands.
    pub fn state(&self) -> LauncherUpdateState {
        self.inner
            .lock()
            .map(|inner| inner.state.clone())
            .unwrap_or(LauncherUpdateState::Failed {
                reason: "internal error".to_string(),
            })
    }

    fn set_state(&self, state: LauncherUpdateState) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.state = state;
        }
    }

    /// Starts the check thread: a check now, then one every [`CHECK_INTERVAL`] or when
    /// [`check_soon`](Self::check_soon) is called. Does nothing for a copy that is not installed.
    pub fn start(self: &Arc<Self>) {
        let (wake, wakes) = channel::<()>();
        match self.inner.lock() {
            Ok(mut inner) => {
                if inner.state == LauncherUpdateState::NotInstalledCopy || inner.wake.is_some() {
                    return;
                }
                inner.wake = Some(wake);
            }
            Err(_) => return,
        }
        let updater = self.clone();
        let spawned = std::thread::Builder::new()
            .name("launcher-update".to_string())
            .spawn(move || {
                loop {
                    updater.check_now();
                    // A timeout is the next scheduled check; a wake is an early one.
                    if let Err(RecvTimeoutError::Disconnected) = wakes.recv_timeout(CHECK_INTERVAL)
                    {
                        return;
                    }
                }
            });
        if let Err(error) = spawned {
            log::warn!("could not start the update check: {error}");
        }
    }

    /// Asks the check thread for a check now (the tray's "Check for updates").
    pub fn check_soon(&self) {
        if let Ok(inner) = self.inner.lock()
            && let Some(wake) = &inner.wake
        {
            // The thread only ends with the process; a failed send has nobody to tell.
            let _ = wake.send(());
        }
    }

    /// Checks, and downloads what is found, on the calling thread. Does nothing while a check
    /// or download runs, when an update is already downloaded, or for a copy that is not
    /// installed. Posts `UpdateReady` when an update is ready.
    pub fn check_now(&self) {
        match self.state() {
            LauncherUpdateState::NotInstalledCopy
            | LauncherUpdateState::Checking
            | LauncherUpdateState::Downloading { .. }
            | LauncherUpdateState::ReadyToRestart { .. } => return,
            _ => {}
        }
        self.set_state(LauncherUpdateState::Checking);
        let result = self.source.check().and_then(|found| {
            let Some(update) = found else {
                return Ok(None);
            };
            self.set_state(LauncherUpdateState::Downloading { percent: 0 });
            self.source.download(&update, &mut |percent| {
                self.set_state(LauncherUpdateState::Downloading { percent });
            })?;
            Ok(Some(update))
        });
        match result {
            Ok(None) => self.set_state(LauncherUpdateState::UpToDate),
            Ok(Some(update)) => {
                let version = update.version.to_string();
                self.set_state(LauncherUpdateState::ReadyToRestart {
                    version: version.clone(),
                    required: update.required,
                });
                self.sink.send(AppEvent::Bridge(BridgeEvent::UpdateReady {
                    version,
                    required: update.required,
                }));
            }
            Err(error) => {
                log::warn!("the Launcher update did not work: {error}");
                self.set_state(match error {
                    UpdateError::Unauthorized => LauncherUpdateState::NeedsGithubSignIn,
                    UpdateError::NotInstalled(_) => LauncherUpdateState::NotInstalledCopy,
                    _ => LauncherUpdateState::Failed {
                        reason: match crate::install::launcher_error(&error) {
                            LauncherError::Update(phrase) => phrase,
                            _ => "the update did not work".to_string(),
                        },
                    },
                });
            }
        }
    }

    /// Whether the Player may be started: not while a required update is waiting.
    pub fn allow_play(&self) -> Result<(), LauncherError> {
        // Once the update is set to apply, files are about to be replaced: no new Player.
        let applying = self
            .inner
            .lock()
            .map(|inner| inner.scheduled)
            .unwrap_or(true);
        match self.state() {
            LauncherUpdateState::ReadyToRestart { required: true, .. } => {
                Err(LauncherError::UpdateRequired)
            }
            LauncherUpdateState::ReadyToRestart { .. } if applying => {
                Err(LauncherError::UpdateRequired)
            }
            _ => Ok(()),
        }
    }

    /// Called when the Launcher is closing. Applies a downloaded update silently, without a
    /// restart, but only when no Player is running (`players_running == 0`); otherwise the
    /// update stays for the next quit.
    pub fn on_quit(&self, players_running: usize) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if !matches!(inner.state, LauncherUpdateState::ReadyToRestart { .. }) || inner.scheduled {
            return;
        }
        if players_running > 0 {
            log::info!("an update is ready but a Player is running; it waits for the next quit");
            return;
        }
        match self.source.apply_after_exit(false) {
            Ok(()) => inner.scheduled = true,
            Err(error) => log::warn!("could not start the update: {error}"),
        }
    }

    /// "Restart to update": applies the downloaded update with a restart, then asks the window
    /// to close. Refused with `Busy` while a Player runs.
    pub fn restart_to_update(&self, players_running: usize) -> Result<(), RestartError> {
        let mut inner = self.inner.lock().map_err(|_| RestartError::Failed)?;
        if !matches!(inner.state, LauncherUpdateState::ReadyToRestart { .. }) {
            return Err(RestartError::NotReady);
        }
        if players_running > 0 {
            return Err(RestartError::Busy);
        }
        if !inner.scheduled {
            self.source.apply_after_exit(true).map_err(|error| {
                log::warn!("could not start the update: {error}");
                RestartError::Failed
            })?;
            inner.scheduled = true;
        }
        drop(inner);
        self.sink.send(AppEvent::Quit);
        Ok(())
    }
}
