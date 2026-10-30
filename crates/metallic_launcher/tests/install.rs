//! Tests for installing Studio, the Launcher's own update and the GitHub sign-in, with fake
//! sources and spawners. Nothing here touches the network or runs a real installer.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use metallic_launcher::{
    AppEvent, AssetSource, BridgeCommand, BridgeEvent, EventSink, Installer, LauncherError,
    LauncherUpdateState, PageOrigin, PlayerWatcher, ProcessLister, ProcessTracker, ProductStatus,
    RestartError, SelfUpdater, Spawner, UpdateSource, allowed, github_sign_in,
    read_installed_version, run_with_limit,
};
use metallic_update::github_auth::GithubAuth;
use metallic_update::{
    Channel, HttpClient, HttpError, HttpResponse, MemorySecretStore, ReleaseAsset, SecretStore,
    UpdateError, UpdateInfo,
};
use semver::Version;
use serde_json::json;
use sha2::{Digest, Sha256};

/// Collects the events the Launcher would send to the window.
#[derive(Default)]
struct Events(Mutex<Vec<String>>);

impl Events {
    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl EventSink for Events {
    fn send(&self, event: AppEvent) {
        let text = match event {
            AppEvent::Bridge(BridgeEvent::InstallProgress { product, percent }) => {
                format!("progress {product} {percent}")
            }
            AppEvent::Bridge(BridgeEvent::InstallFinished { product, ok }) => {
                format!("finished {product} {ok}")
            }
            AppEvent::Bridge(BridgeEvent::UpdateReady { version, required }) => {
                format!("ready {version} {required}")
            }
            AppEvent::Bridge(BridgeEvent::GithubSignIn { state, user_code }) => {
                format!("github {state} {}", user_code.unwrap_or_default())
            }
            AppEvent::Quit => "quit".to_string(),
            other => format!("{other:?}"),
        };
        self.0.lock().unwrap().push(text);
    }
}

fn write_sq_version(root: &Path, version: &str) {
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("sq.version"),
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?><package><metadata><id>MetallicStudio</id><version>{version}</version><title>Metallic Studio</title></metadata></package>"#
        ),
    )
    .unwrap();
}

#[test]
fn installed_version_read_from_sq_version() {
    let dir = tempfile::tempdir().unwrap();
    write_sq_version(dir.path(), "0.4.2-dev");
    assert_eq!(
        read_installed_version(dir.path()).as_deref(),
        Some("0.4.2-dev")
    );
    // A missing or unusable version is "unknown", never garbage.
    for bad in ["", "1.0 <b>", "a/b", "../x"] {
        write_sq_version(dir.path(), bad);
        assert_eq!(read_installed_version(dir.path()), None, "{bad:?}");
    }
    std::fs::write(dir.path().join("current").join("sq.version"), "no xml here").unwrap();
    assert_eq!(read_installed_version(dir.path()), None);
}

/// A fake release feed: `asset` is what the lookup finds; `download` writes `bytes` (or fails).
struct FakeAssets {
    asset: Option<ReleaseAsset>,
    download_error: bool,
    /// When set, `download` waits for a message first.
    hold: Mutex<Option<Receiver<()>>>,
    looked_up: Mutex<Vec<String>>,
}

impl FakeAssets {
    fn new(asset: Option<ReleaseAsset>) -> Self {
        FakeAssets {
            asset,
            download_error: false,
            hold: Mutex::new(None),
            looked_up: Mutex::new(Vec::new()),
        }
    }
}

fn asset(name: &str) -> ReleaseAsset {
    ReleaseAsset {
        version: Version::new(0, 4, 2),
        name: name.to_string(),
        api_url: "https://api.github.com/repos/x/y/releases/assets/1".to_string(),
        sha256: Sha256::digest(b"installer").into(),
        size: 9,
    }
}

impl AssetSource for FakeAssets {
    fn latest_asset(
        &self,
        channel: Channel,
        file_name: &str,
    ) -> Result<Option<ReleaseAsset>, UpdateError> {
        self.looked_up
            .lock()
            .unwrap()
            .push(format!("{} {file_name}", channel.as_str()));
        Ok(self.asset.clone())
    }

    fn download(
        &self,
        asset: &ReleaseAsset,
        dir: &Path,
        progress: &mut dyn FnMut(u8),
    ) -> Result<PathBuf, UpdateError> {
        if let Some(gate) = self.hold.lock().unwrap().take() {
            gate.recv().unwrap();
        }
        progress(10);
        progress(10);
        progress(60);
        if self.download_error {
            return Err(UpdateError::Install("digest mismatch".to_string()));
        }
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(&asset.name);
        std::fs::write(&path, b"installer").unwrap();
        progress(100);
        Ok(path)
    }
}

/// Records the installer runs. Running it "installs" Studio by writing `sq.version`.
struct FakeSpawner {
    runs: Mutex<Vec<(PathBuf, Vec<OsString>, bool)>>,
    exit_code: i32,
    studio_root: PathBuf,
    /// How many times the installer file was opened with a lock.
    locked: AtomicUsize,
    /// Make `spawn_and_wait` fail like a killed, hung installer.
    hang: bool,
}

impl Spawner for FakeSpawner {
    fn spawn(&self, _exe: &Path, _args: &[OsString]) -> Result<u32, LauncherError> {
        panic!("an installer is started with spawn_and_wait only");
    }

    fn open_locked(&self, path: &Path) -> Result<std::fs::File, LauncherError> {
        self.locked.fetch_add(1, Ordering::SeqCst);
        std::fs::File::open(path).map_err(|e| LauncherError::Io(e.to_string()))
    }

    fn spawn_and_wait(
        &self,
        exe: &Path,
        args: &[OsString],
        _time_limit: Duration,
    ) -> Result<i32, LauncherError> {
        if self.hang {
            return Err(LauncherError::Update("the installer took too long".into()));
        }
        // The verified file exists at this moment.
        self.runs
            .lock()
            .unwrap()
            .push((exe.to_path_buf(), args.to_vec(), exe.is_file()));
        if self.exit_code == 0 {
            write_sq_version(&self.studio_root, "0.4.2-dev");
        }
        Ok(self.exit_code)
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    downloads: PathBuf,
    spawner: Arc<FakeSpawner>,
}

fn fixture(exit_code: i32) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("MetallicStudio");
    let downloads = dir.path().join("Downloads");
    let spawner = Arc::new(FakeSpawner {
        runs: Mutex::new(Vec::new()),
        exit_code,
        studio_root: root.clone(),
        locked: AtomicUsize::new(0),
        hang: false,
    });
    Fixture {
        _dir: dir,
        root,
        downloads,
        spawner,
    }
}

fn installer(f: &Fixture, source: Arc<FakeAssets>) -> Installer {
    Installer::new(
        source,
        f.spawner.clone(),
        Channel::Dev,
        f.root.clone(),
        f.downloads.clone(),
    )
}

#[test]
fn studio_not_installed_when_folder_missing() {
    let f = fixture(0);
    let installer = installer(&f, Arc::new(FakeAssets::new(None)));
    assert!(!f.root.exists());
    assert_eq!(installer.studio_status(), ProductStatus::NotInstalled);
    write_sq_version(&f.root, "0.3.0");
    assert_eq!(
        installer.studio_status(),
        ProductStatus::Installed {
            version: "0.3.0".to_string()
        }
    );
}

#[test]
fn install_runs_the_installer_after_the_download_and_deletes_it() {
    let f = fixture(0);
    let source = Arc::new(FakeAssets::new(Some(asset("MetallicStudio-dev-Setup.exe"))));
    let installer = installer(&f, source.clone());
    let events = Events::default();
    installer.install_studio(&events).unwrap();
    assert_eq!(
        source.looked_up.lock().unwrap().as_slice(),
        ["dev MetallicStudio-dev-Setup.exe"]
    );
    let runs = f.spawner.runs.lock().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, f.downloads.join("MetallicStudio-dev-Setup.exe"));
    assert_eq!(runs[0].1, [OsString::from("--silent")]);
    assert!(runs[0].2, "the file was there when the installer ran");
    assert!(!runs[0].0.exists(), "the installer is deleted after it ran");
    // Progress is reported once per change, then the end.
    assert_eq!(
        events.all(),
        [
            "progress studio 10",
            "progress studio 60",
            "progress studio 100",
            "finished studio true"
        ]
    );
    assert_eq!(
        installer.studio_status(),
        ProductStatus::Installed {
            version: "0.4.2-dev".to_string()
        }
    );
}

#[test]
fn a_failed_download_never_runs_the_installer() {
    let f = fixture(0);
    let mut source = FakeAssets::new(Some(asset("MetallicStudio-dev-Setup.exe")));
    source.download_error = true;
    let installer = installer(&f, Arc::new(source));
    let events = Events::default();
    assert!(installer.install_studio(&events).is_err());
    assert!(f.spawner.runs.lock().unwrap().is_empty());
    assert_eq!(events.all().last().unwrap(), "finished studio false");
    assert!(matches!(
        installer.studio_status(),
        ProductStatus::Failed { .. }
    ));
}

#[test]
fn no_published_installer_is_a_failure_with_a_plain_reason() {
    let f = fixture(0);
    let installer = installer(&f, Arc::new(FakeAssets::new(None)));
    let events = Events::default();
    assert!(installer.install_studio(&events).is_err());
    let ProductStatus::Failed { reason } = installer.studio_status() else {
        panic!("expected a failure");
    };
    assert!(reason.contains("installer"), "{reason}");
}

#[test]
fn an_installer_that_fails_leaves_studio_not_installed() {
    let f = fixture(1603);
    let source = Arc::new(FakeAssets::new(Some(asset("MetallicStudio-dev-Setup.exe"))));
    let installer = installer(&f, source);
    let events = Events::default();
    assert!(installer.install_studio(&events).is_err());
    assert_eq!(events.all().last().unwrap(), "finished studio false");
    assert!(
        !f.downloads.join("MetallicStudio-dev-Setup.exe").exists(),
        "the installer is deleted even when it failed"
    );
}

#[test]
fn second_install_request_is_busy() {
    let f = fixture(0);
    let source = FakeAssets::new(Some(asset("MetallicStudio-dev-Setup.exe")));
    let (release, gate): (Sender<()>, Receiver<()>) = channel();
    *source.hold.lock().unwrap() = Some(gate);
    let installer = Arc::new(installer(&f, Arc::new(source)));
    let events = Arc::new(Events::default());
    installer.start_install_studio(events.clone()).unwrap();
    // The first install is held in its download: a second request is refused at once, and the
    // status says an install is running.
    assert!(matches!(
        installer.start_install_studio(events.clone()),
        Err(LauncherError::Busy(_))
    ));
    assert!(matches!(
        installer.install_studio(events.as_ref()),
        Err(LauncherError::Busy(_))
    ));
    assert!(matches!(
        installer.studio_status(),
        ProductStatus::Installing { .. }
    ));
    release.send(()).unwrap();
    for _ in 0..200 {
        if events.all().iter().any(|e| e.starts_with("finished")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(events.all().last().unwrap(), "finished studio true");
    // The place is free again.
    assert!(!matches!(
        installer.studio_status(),
        ProductStatus::Installing { .. }
    ));
}

// ---- the Launcher's own update -----------------------------------------------------------------

struct FakeUpdates {
    found: Mutex<VecDeque<Result<Option<UpdateInfo>, UpdateError>>>,
    applied: Mutex<Vec<bool>>,
    apply_error: bool,
}

impl FakeUpdates {
    fn with(found: Vec<Result<Option<UpdateInfo>, UpdateError>>) -> Arc<Self> {
        Arc::new(FakeUpdates {
            found: Mutex::new(found.into()),
            applied: Mutex::new(Vec::new()),
            apply_error: false,
        })
    }
}

impl UpdateSource for FakeUpdates {
    fn check(&self) -> Result<Option<UpdateInfo>, UpdateError> {
        self.found.lock().unwrap().pop_front().unwrap_or(Ok(None))
    }

    fn download(
        &self,
        _update: &UpdateInfo,
        progress: &mut dyn FnMut(u8),
    ) -> Result<(), UpdateError> {
        progress(50);
        progress(100);
        Ok(())
    }

    fn apply_after_exit(&self, restart: bool) -> Result<(), UpdateError> {
        if self.apply_error {
            return Err(UpdateError::Install("no".to_string()));
        }
        self.applied.lock().unwrap().push(restart);
        Ok(())
    }
}

fn update(version: &str, required: bool) -> UpdateInfo {
    UpdateInfo {
        version: Version::parse(version).unwrap(),
        required,
    }
}

/// A tracker with a fixed count.
struct FakeTracker(usize);

impl ProcessTracker for FakeTracker {
    fn running_players(&self) -> usize {
        self.0
    }
}

fn ready_updater(required: bool) -> (Arc<FakeUpdates>, Arc<Events>, SelfUpdater) {
    let source = FakeUpdates::with(vec![Ok(Some(update("0.5.0", required)))]);
    let events = Arc::new(Events::default());
    let updater = SelfUpdater::new(source.clone(), events.clone(), true);
    updater.check_now();
    (source, events, updater)
}

#[test]
fn a_found_update_is_downloaded_and_announced() {
    let (_source, events, updater) = ready_updater(false);
    assert_eq!(
        updater.state(),
        LauncherUpdateState::ReadyToRestart {
            version: "0.5.0".to_string(),
            required: false
        }
    );
    assert_eq!(events.all(), ["ready 0.5.0 false"]);
    assert!(updater.allow_play().is_ok());
}

#[test]
fn apply_waits_while_player_runs() {
    let (source, _events, updater) = ready_updater(false);
    // A Player is running: closing the Launcher does not apply the update.
    updater.on_quit(FakeTracker(1).running_players());
    assert!(source.applied.lock().unwrap().is_empty());
    // It stays ready, so the next quit with no Player applies it, silently and without a restart.
    assert!(matches!(
        updater.state(),
        LauncherUpdateState::ReadyToRestart { .. }
    ));
    updater.on_quit(FakeTracker(0).running_players());
    assert_eq!(*source.applied.lock().unwrap(), [false]);
    // Only once.
    updater.on_quit(0);
    assert_eq!(source.applied.lock().unwrap().len(), 1);
}

#[test]
fn restart_to_update_is_refused_while_a_player_runs() {
    let (source, events, updater) = ready_updater(false);
    assert_eq!(updater.restart_to_update(2), Err(RestartError::Busy));
    assert!(source.applied.lock().unwrap().is_empty());
    assert!(!events.all().contains(&"quit".to_string()));
    assert_eq!(updater.restart_to_update(0), Ok(()));
    assert_eq!(*source.applied.lock().unwrap(), [true], "with a restart");
    assert_eq!(events.all().last().unwrap(), "quit");
    // Closing afterwards does not apply a second time.
    updater.on_quit(0);
    assert_eq!(source.applied.lock().unwrap().len(), 1);
}

#[test]
fn restart_to_update_needs_a_downloaded_update() {
    let source = FakeUpdates::with(vec![]);
    let events = Arc::new(Events::default());
    let updater = SelfUpdater::new(source.clone(), events, true);
    assert_eq!(updater.restart_to_update(0), Err(RestartError::NotReady));
    updater.check_now();
    assert_eq!(updater.state(), LauncherUpdateState::UpToDate);
    assert_eq!(updater.restart_to_update(0), Err(RestartError::NotReady));
    updater.on_quit(0);
    assert!(source.applied.lock().unwrap().is_empty());
}

#[test]
fn a_failing_updater_does_not_close_the_window() {
    let fake = FakeUpdates {
        found: Mutex::new(vec![Ok(Some(update("0.5.0", false)))].into()),
        applied: Mutex::new(Vec::new()),
        apply_error: true,
    };
    let events = Arc::new(Events::default());
    let updater = SelfUpdater::new(Arc::new(fake), events.clone(), true);
    updater.check_now();
    assert_eq!(updater.restart_to_update(0), Err(RestartError::Failed));
    assert!(!events.all().contains(&"quit".to_string()));
}

#[test]
fn required_update_blocks_play() {
    let (_source, events, updater) = ready_updater(true);
    assert!(matches!(
        updater.allow_play(),
        Err(LauncherError::UpdateRequired)
    ));
    assert_eq!(events.all(), ["ready 0.5.0 true"]);
}

#[test]
fn update_failures_become_plain_states() {
    let cases: Vec<(Result<Option<UpdateInfo>, UpdateError>, LauncherUpdateState)> = vec![
        (
            Err(UpdateError::Unauthorized),
            LauncherUpdateState::NeedsGithubSignIn,
        ),
        (
            Err(UpdateError::NotInstalled("x".into())),
            LauncherUpdateState::NotInstalledCopy,
        ),
        (
            Err(UpdateError::Network("secret.host.example".into())),
            LauncherUpdateState::Failed {
                reason: "could not reach the update server".to_string(),
            },
        ),
    ];
    for (found, expected) in cases {
        let updater = SelfUpdater::new(
            FakeUpdates::with(vec![found]),
            Arc::new(Events::default()),
            true,
        );
        updater.check_now();
        assert_eq!(updater.state(), expected);
    }
}

#[test]
fn a_copy_that_is_not_installed_never_updates() {
    let source = FakeUpdates::with(vec![Ok(Some(update("0.5.0", true)))]);
    let updater = Arc::new(SelfUpdater::new(
        source.clone(),
        Arc::new(Events::default()),
        false,
    ));
    updater.start();
    updater.check_now();
    assert_eq!(updater.state(), LauncherUpdateState::NotInstalledCopy);
    assert_eq!(source.found.lock().unwrap().len(), 1, "no check was made");
    assert!(updater.allow_play().is_ok());
}

/// A fixed list of running programs.
struct FakeLister(Option<Vec<PathBuf>>);

impl ProcessLister for FakeLister {
    fn running_images(&self) -> Option<Vec<PathBuf>> {
        self.0.clone()
    }
}

fn watcher(list: Option<Vec<&str>>) -> PlayerWatcher {
    PlayerWatcher::new(
        Arc::new(FakeLister(
            list.map(|l| l.into_iter().map(PathBuf::from).collect()),
        )),
        PathBuf::from(r"C:\Apps\Metallic\current\metallic-player.exe"),
    )
}

#[test]
fn apply_waits_for_player_started_elsewhere() {
    // A Player that this Launcher run did not start (a double-click, an earlier Launcher) is
    // found by its image path, in any letter case and slash style. The Launcher itself, which
    // sits in the same folder, is not a Player.
    let watcher = watcher(Some(vec![
        r"C:\Windows\explorer.exe",
        r"C:\Apps\Metallic\current\Metallic.exe",
        r"c:/apps/metallic/CURRENT/Metallic-Player.EXE",
        r"D:\other\metallic-player.exe",
    ]));
    assert_eq!(watcher.running_players(), 1);
    let source = FakeUpdates::with(vec![Ok(Some(update("0.5.0", false)))]);
    let updater = SelfUpdater::new(source.clone(), Arc::new(Events::default()), true);
    updater.check_now();
    updater.on_quit(watcher.running_players());
    assert!(
        source.applied.lock().unwrap().is_empty(),
        "the update waits"
    );
    assert_eq!(
        updater.restart_to_update(watcher.running_players()),
        Err(RestartError::Busy)
    );
    // When the Player has ended, the update goes ahead.
    let none = self::watcher(Some(vec![r"C:\Windows\explorer.exe"]));
    assert_eq!(none.running_players(), 0);
    updater.on_quit(none.running_players());
    assert_eq!(*source.applied.lock().unwrap(), [false]);
}

#[test]
fn an_unreadable_process_list_counts_as_a_running_player() {
    assert_eq!(watcher(None).running_players(), 1);
    let unknown = PlayerWatcher::new(Arc::new(FakeLister(Some(vec![]))), PathBuf::new());
    assert_eq!(unknown.running_players(), 0);
}

#[test]
fn no_new_player_while_the_update_is_applying() {
    let (_source, _events, updater) = ready_updater(false);
    assert!(updater.allow_play().is_ok());
    updater.restart_to_update(0).unwrap();
    assert!(matches!(
        updater.allow_play(),
        Err(LauncherError::UpdateRequired)
    ));
}

// ---- settings: the site may not switch the channel ---------------------------------------------

#[test]
fn site_cannot_change_channel() {
    let command: BridgeCommand = serde_json::from_value(json!({
        "cmd": "setSettings",
        "args": {"patch": {"channel": "stable"}}
    }))
    .unwrap();
    assert!(!allowed(PageOrigin::Site, &command));
    assert!(!allowed(PageOrigin::Other, &command));
    assert!(allowed(PageOrigin::Local, &command));
    // Not even together with a setting the site may change.
    let mixed: BridgeCommand = serde_json::from_value(json!({
        "cmd": "setSettings",
        "args": {"patch": {"closeToTray": false, "channel": "dev"}}
    }))
    .unwrap();
    assert!(!allowed(PageOrigin::Site, &mixed));
}

// ---- GitHub sign-in ----------------------------------------------------------------------------

struct ScriptedGithub(Mutex<VecDeque<String>>);

impl HttpClient for ScriptedGithub {
    fn get(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        Err(HttpError::new("no GET expected"))
    }

    fn post_form(
        &self,
        _url: &str,
        _headers: &[(&str, &str)],
        _form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let body = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| HttpError::new("nothing scripted"))?;
        Ok(HttpResponse { status: 200, body })
    }
}

fn run_sign_in(answers: &[&str]) -> (Vec<String>, Vec<String>, bool, Arc<MemorySecretStore>) {
    let http = Arc::new(ScriptedGithub(Mutex::new(
        answers.iter().map(|a| a.to_string()).collect(),
    )));
    let store = Arc::new(MemorySecretStore::default());
    let auth = Arc::new(GithubAuth::new("client".into(), http, store.clone()).without_sleeping());
    let events = Arc::new(Events::default());
    let opened = Arc::new(Mutex::new(Vec::new()));
    let opened_by_ui = opened.clone();
    let (done_tx, done_rx) = channel::<bool>();
    github_sign_in(
        auth,
        events.clone(),
        Arc::new(move |address: &str| opened_by_ui.lock().unwrap().push(address.to_string())),
        move |ok| done_tx.send(ok).unwrap(),
    )
    .unwrap();
    let ok = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let opened = opened.lock().unwrap().clone();
    (events.all(), opened, ok, store)
}

const DEVICE_CODE: &str = r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","expires_in":900,"interval":1}"#;

#[test]
fn github_sign_in_shows_the_code_opens_the_page_and_stores_the_session() {
    let (events, opened, ok, store) = run_sign_in(&[
        DEVICE_CODE,
        r#"{"error":"authorization_pending"}"#,
        r#"{"access_token":"ghu_not_in_any_event"}"#,
    ]);
    assert!(ok);
    assert_eq!(events, ["github pending ABCD-1234", "github signedIn "]);
    assert_eq!(opened, ["https://github.com/login/device"]);
    assert!(store.get().unwrap().is_some());
    assert!(
        !events.iter().any(|e| e.contains("ghu_")),
        "no token in any event"
    );
}

#[test]
fn github_sign_in_reports_a_refusal() {
    let (events, _opened, ok, store) = run_sign_in(&[DEVICE_CODE, r#"{"error":"access_denied"}"#]);
    assert!(!ok);
    assert_eq!(events.last().unwrap(), "github denied ");
    assert!(store.get().unwrap().is_none());
}

// ---- stuck installs and the locked installer ---------------------------------------------------

fn wait_for_finish(events: &Events) {
    for _ in 0..200 {
        if events.all().iter().any(|e| e.starts_with("finished")) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("the install never finished");
}

#[test]
fn installer_flag_resets_on_every_exit() {
    let good_asset = Some(asset("MetallicStudio-dev-Setup.exe"));

    // 1. The lookup fails.
    let f = fixture(0);
    let mut source = FakeAssets::new(good_asset.clone());
    source.download_error = true;
    let installer = Arc::new(installer(&f, Arc::new(source)));
    assert!(installer.install_studio(&Events::default()).is_err());
    assert!(!matches!(
        installer.studio_status(),
        ProductStatus::Installing { .. }
    ));

    // 2. The installer hangs and is killed by the time limit.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("MetallicStudio");
    let spawner = Arc::new(FakeSpawner {
        runs: Mutex::new(Vec::new()),
        exit_code: 0,
        studio_root: root.clone(),
        locked: AtomicUsize::new(0),
        hang: true,
    });
    let installer = Installer::new(
        Arc::new(FakeAssets::new(good_asset.clone())),
        spawner,
        Channel::Dev,
        root,
        dir.path().join("Downloads"),
    )
    .with_time_limit(Duration::from_millis(50));
    assert!(installer.install_studio(&Events::default()).is_err());
    assert!(matches!(
        installer.studio_status(),
        ProductStatus::Failed { .. }
    ));
    assert!(
        !dir.path()
            .join("Downloads")
            .join("MetallicStudio-dev-Setup.exe")
            .exists(),
        "the installer file is deleted after a kill too"
    );

    // 3. The code panics on the install thread: the place is freed and the end is reported.
    struct Panicking;
    impl AssetSource for Panicking {
        fn latest_asset(&self, _: Channel, _: &str) -> Result<Option<ReleaseAsset>, UpdateError> {
            panic!("a bug in the lookup");
        }
        fn download(
            &self,
            _: &ReleaseAsset,
            _: &Path,
            _: &mut dyn FnMut(u8),
        ) -> Result<PathBuf, UpdateError> {
            unreachable!()
        }
    }
    let f = fixture(0);
    let installer = Arc::new(Installer::new(
        Arc::new(Panicking),
        f.spawner.clone(),
        Channel::Dev,
        f.root.clone(),
        f.downloads.clone(),
    ));
    let events = Arc::new(Events::default());
    installer.start_install_studio(events.clone()).unwrap();
    wait_for_finish(&events);
    assert_eq!(events.all().last().unwrap(), "finished studio false");
    assert!(matches!(
        installer.studio_status(),
        ProductStatus::Failed { .. }
    ));
    // A new install may start.
    assert!(installer.start_install_studio(events.clone()).is_ok());
    wait_for_finish(&events);
}

#[test]
fn installer_is_rehashed_from_locked_handle() {
    // The source hands back a file that does not have the published SHA-256 (it was swapped
    // after the download check). The installer must not run it, and must delete it.
    struct Swapping(FakeAssets);
    impl AssetSource for Swapping {
        fn latest_asset(&self, c: Channel, n: &str) -> Result<Option<ReleaseAsset>, UpdateError> {
            self.0.latest_asset(c, n)
        }
        fn download(
            &self,
            a: &ReleaseAsset,
            d: &Path,
            p: &mut dyn FnMut(u8),
        ) -> Result<PathBuf, UpdateError> {
            let path = self.0.download(a, d, p)?;
            std::fs::write(&path, b"swapped!!").unwrap();
            Ok(path)
        }
    }
    let f = fixture(0);
    let source = Swapping(FakeAssets::new(Some(asset("MetallicStudio-dev-Setup.exe"))));
    let swapped = installer_with(&f, Arc::new(source));
    let events = Events::default();
    assert!(swapped.install_studio(&events).is_err());
    assert!(
        f.spawner.runs.lock().unwrap().is_empty(),
        "the swapped file never ran"
    );
    assert_eq!(
        f.spawner.locked.load(Ordering::SeqCst),
        1,
        "it was opened locked"
    );
    assert!(!f.downloads.join("MetallicStudio-dev-Setup.exe").exists());
    assert_eq!(events.all().last().unwrap(), "finished studio false");

    // The honest file is locked once, then runs.
    let f = fixture(0);
    let installer = installer(
        &f,
        Arc::new(FakeAssets::new(Some(asset("MetallicStudio-dev-Setup.exe")))),
    );
    installer.install_studio(&Events::default()).unwrap();
    assert_eq!(f.spawner.locked.load(Ordering::SeqCst), 1);
    assert_eq!(f.spawner.runs.lock().unwrap().len(), 1);
}

fn installer_with(f: &Fixture, source: Arc<dyn AssetSource>) -> Installer {
    Installer::new(
        source,
        f.spawner.clone(),
        Channel::Dev,
        f.root.clone(),
        f.downloads.clone(),
    )
}

#[cfg(windows)]
#[test]
fn a_program_that_runs_too_long_is_killed() {
    // `ping` is a harmless built-in that runs about 30 seconds.
    let started = std::time::Instant::now();
    let result = run_with_limit(
        std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null()),
        Duration::from_millis(500),
    );
    assert!(
        matches!(result, Err(LauncherError::Update(_))),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}
