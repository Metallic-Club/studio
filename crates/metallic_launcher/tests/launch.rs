//! Tests for starting the Player and Studio and for the tray rules, with a recording spawner.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use metallic_launcher::{
    CloseAction, DeepLink, LauncherError, LauncherSettings, ProductPaths, Spawner, TrayAction,
    on_close_requested, product_paths_for, start_player, start_studio, tray_menu,
};

#[derive(Default)]
struct RecordingSpawner {
    calls: Mutex<Vec<(PathBuf, Vec<OsString>)>>,
}

impl Spawner for RecordingSpawner {
    fn spawn(&self, exe: &Path, args: &[OsString]) -> Result<u32, LauncherError> {
        self.calls
            .lock()
            .unwrap()
            .push((exe.to_path_buf(), args.to_vec()));
        Ok(7)
    }
}

fn paths_with_player() -> (tempfile::TempDir, ProductPaths) {
    let dir = tempfile::tempdir().unwrap();
    let player = dir.path().join("metallic-player.exe");
    std::fs::write(&player, b"").unwrap();
    let paths = ProductPaths {
        player_exe: player,
        studio_exe: None,
    };
    (dir, paths)
}

#[test]
fn player_gets_canonical_uri_as_single_arg() {
    let (_dir, paths) = paths_with_player();
    let sp = RecordingSpawner::default();
    let link = DeepLink::parse("METALLIC://play?ticket=jt_abcdefgh&place=1&x=y").unwrap();
    assert_eq!(start_player(&sp, &paths, &link).unwrap(), 7);
    let calls = sp.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, paths.player_exe);
    assert_eq!(
        calls[0].1,
        vec![OsString::from("metallic://play?place=1&ticket=jt_abcdefgh")]
    );
}

#[test]
fn uri_argument_disables_other_flags() {
    // A ticket cannot carry a space or a quote, so the one argument cannot be split. (A ticket
    // that looks like a flag is still inside a `metallic://` argument, so it is not a flag.)
    for hostile in [
        "metallic://play?place=1&ticket=jt_abcdefgh%20--dev-url=x",
        "metallic://play?place=1&ticket=jt_abcdefgh\" --dev-url=x",
    ] {
        assert!(DeepLink::parse(hostile).is_err(), "{hostile}");
    }
    let (_dir, paths) = paths_with_player();
    let sp = RecordingSpawner::default();
    let link = DeepLink::parse("metallic://play?place=1&ticket=jt_abcdefgh&--dev-url=x").unwrap();
    start_player(&sp, &paths, &link).unwrap();
    let calls = sp.calls.lock().unwrap();
    assert_eq!(calls[0].1.len(), 1);
    assert!(!calls[0].1[0].to_string_lossy().contains("dev-url"));
}

#[test]
fn non_play_link_refused() {
    let (_dir, paths) = paths_with_player();
    let sp = RecordingSpawner::default();
    for text in [
        "metallic://install",
        "metallic://studio/open",
        "metallic://open",
    ] {
        let link = DeepLink::parse(text).unwrap();
        assert!(start_player(&sp, &paths, &link).is_err());
    }
    assert!(sp.calls.lock().unwrap().is_empty());
}

#[test]
fn missing_player_is_not_installed_error() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ProductPaths {
        player_exe: dir.path().join("metallic-player.exe"),
        studio_exe: None,
    };
    let sp = RecordingSpawner::default();
    let link = DeepLink::parse("metallic://play?place=1&ticket=jt_abcdefgh").unwrap();
    assert!(matches!(
        start_player(&sp, &paths, &link),
        Err(LauncherError::NotInstalled("player"))
    ));
    assert!(sp.calls.lock().unwrap().is_empty());
}

#[test]
fn studio_starts_with_no_arguments_and_needs_an_install() {
    let sp = RecordingSpawner::default();
    let none = ProductPaths {
        player_exe: PathBuf::new(),
        studio_exe: None,
    };
    assert!(matches!(
        start_studio(&sp, &none),
        Err(LauncherError::NotInstalled("studio"))
    ));
    let dir = tempfile::tempdir().unwrap();
    let studio = dir.path().join("Metallic Studio.exe");
    std::fs::write(&studio, b"").unwrap();
    let paths = ProductPaths {
        player_exe: PathBuf::new(),
        studio_exe: Some(studio.clone()),
    };
    start_studio(&sp, &paths).unwrap();
    assert_eq!(*sp.calls.lock().unwrap(), vec![(studio, Vec::new())]);
}

#[test]
fn product_paths_follow_the_install_layout() {
    let dir = tempfile::tempdir().unwrap();
    let studio = dir.path().join("MetallicStudio").join("current");
    std::fs::create_dir_all(&studio).unwrap();
    let exe = Path::new("C:/Apps/current/Metallic.exe");
    let none = product_paths_for(Some(exe), Some(dir.path()));
    assert_eq!(
        none.player_exe,
        Path::new("C:/Apps/current/metallic-player.exe")
    );
    assert_eq!(none.studio_exe, None, "no Studio exe yet");
    std::fs::write(studio.join("Metallic Studio.exe"), b"").unwrap();
    let found = product_paths_for(Some(exe), Some(dir.path()));
    assert_eq!(found.studio_exe, Some(studio.join("Metallic Studio.exe")));
    // Unknown own path: the Player path is empty, never a bare name.
    assert_eq!(product_paths_for(None, None).player_exe, PathBuf::new());
}

#[test]
fn close_hides_when_close_to_tray() {
    let mut settings = LauncherSettings::default();
    assert_eq!(on_close_requested(&settings), CloseAction::HideToTray);
    settings.close_to_tray = false;
    assert_eq!(on_close_requested(&settings), CloseAction::Quit);
}

#[test]
fn tray_menu_order() {
    assert_eq!(tray_menu(false).last().unwrap().0, TrayAction::Quit);
    let with: Vec<&str> = tray_menu(true).iter().map(|(_, text)| *text).collect();
    assert_eq!(
        with,
        ["Open Metallic", "Open Studio", "Check for updates", "Quit"]
    );
    let without: Vec<&str> = tray_menu(false).iter().map(|(_, text)| *text).collect();
    assert_eq!(without, ["Open Metallic", "Check for updates", "Quit"]);
}

#[test]
fn tray_action_ids_round_trip() {
    for (action, _) in tray_menu(true) {
        assert_eq!(TrayAction::from_id(action.id()), Some(action));
    }
    assert_eq!(TrayAction::from_id("rm -rf"), None);
}
