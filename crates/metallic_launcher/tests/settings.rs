//! Settings validation, persistence and URI-to-argument isolation.

use metallic_launcher::{DeepLink, LauncherConfig, LauncherSettings, SettingsPatch, SettingsStore};
use std::fs;

#[test]
fn missing_file_gives_defaults() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        SettingsStore::at(root.path().join("none.toml"))
            .load()
            .close_to_tray
    );
}

#[test]
fn corrupt_settings_fall_back_to_defaults() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.toml");
    for text in [
        "close_to_tray = [",
        "developer_mode = 'not bool'",
        "dev_url = 'http://evil.invalid'",
        &"x".repeat(100_000),
    ] {
        fs::write(&path, text).unwrap();
        assert_eq!(
            SettingsStore::at(path.clone()).load(),
            LauncherSettings::default()
        );
    }
}

#[test]
fn save_then_load_round_trips_and_replaces_an_existing_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("new/settings.toml");
    let store = SettingsStore::at(path.clone());
    store.save(&LauncherSettings::default()).unwrap();
    let settings = LauncherSettings {
        close_to_tray: false,
        developer_mode: true,
        dev_url: Some("http://localhost:3000".into()),
    };
    store.save(&settings).unwrap();
    assert_eq!(store.load(), settings);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn old_partial_settings_use_defaults() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.toml");
    fs::write(&path, "developer_mode = true").unwrap();
    let settings = SettingsStore::at(path).load();
    assert!(settings.close_to_tray && settings.developer_mode && settings.dev_url.is_none());
}

#[test]
fn patch_rejects_non_loopback_dev_url_without_partial_mutation() {
    let mut settings = LauncherSettings::default();
    let before = settings.clone();
    assert!(
        settings
            .apply(SettingsPatch {
                close_to_tray: Some(false),
                developer_mode: Some(true),
                dev_url: Some(Some("http://evil.invalid".into()))
            })
            .is_err()
    );
    assert_eq!(settings, before);
    settings
        .apply(SettingsPatch {
            dev_url: Some(Some("http://127.0.0.1:3000".into())),
            ..Default::default()
        })
        .unwrap();
    settings
        .apply(SettingsPatch {
            dev_url: Some(None),
            ..Default::default()
        })
        .unwrap();
    assert!(settings.dev_url.is_none());
}

#[test]
fn invalid_save_preserves_old_file_and_no_temporary_file_remains() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.toml");
    let store = SettingsStore::at(path.clone());
    store.save(&LauncherSettings::default()).unwrap();
    let before = fs::read(&path).unwrap();
    let invalid = LauncherSettings {
        dev_url: Some("javascript:private".into()),
        ..Default::default()
    };
    let error = store.save(&invalid).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("private"));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn failed_replacement_cleans_up_its_temporary_file() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("settings.toml");
    fs::create_dir(&target).unwrap();
    assert!(
        SettingsStore::at(target.clone())
            .save(&LauncherSettings::default())
            .is_err()
    );
    assert!(target.is_dir());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn uri_argument_disables_other_flags() {
    let settings = LauncherSettings {
        developer_mode: true,
        dev_url: Some("http://localhost:3000".into()),
        ..Default::default()
    };
    for args in [
        vec![
            "metallic://install",
            "--dev",
            "--dev-url",
            "http://127.0.0.1:1",
            "--hidden",
            "--exit-after-load",
        ],
        vec!["--dev", "--hidden", "METALLIC://install", "--unknown"],
    ] {
        let config =
            LauncherConfig::from_args(args.into_iter().map(str::to_owned), &settings).unwrap();
        assert!(
            !config.dev_mode
                && config.dev_origin.is_none()
                && !config.start_hidden
                && !config.exit_after_load
        );
        assert_eq!(config.start_link, Some(DeepLink::Install));
    }
    assert!(
        LauncherConfig::from_args(
            [
                "--dev",
                "metallic://play?ticket=private",
                "--dev-url",
                "http://localhost:1"
            ]
            .map(str::to_owned),
            &settings
        )
        .is_err()
    );
}

#[test]
fn ordinary_flags_and_persisted_developer_settings_work() {
    let settings = LauncherSettings {
        developer_mode: true,
        dev_url: Some("http://localhost:3000".into()),
        ..Default::default()
    };
    let config = LauncherConfig::from_args(
        ["--hidden", "--exit-after-load"].map(str::to_owned),
        &settings,
    )
    .unwrap();
    assert!(
        config.dev_mode
            && config.dev_origin.is_some()
            && config.start_hidden
            && config.exit_after_load
    );
    for args in [
        vec!["--unknown"],
        vec!["--dev-url"],
        vec![
            "--dev",
            "--dev-url",
            "http://localhost:1",
            "--dev-url",
            "http://localhost:2",
        ],
    ] {
        assert!(
            LauncherConfig::from_args(
                args.into_iter().map(str::to_owned),
                &LauncherSettings::default()
            )
            .is_err()
        );
    }
}

#[test]
fn hostile_argument_counts_are_bounded() {
    assert!(
        LauncherConfig::from_args(
            (0..100_000).map(|_| "--dev".to_owned()),
            &LauncherSettings::default()
        )
        .is_err()
    );
    assert!(
        LauncherConfig::from_args(["x".repeat(100_000)], &LauncherSettings::default()).is_err()
    );
}

#[test]
fn concurrent_saves_use_distinct_temporary_files_and_readers_see_complete_settings() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.toml");
    let store = SettingsStore::at(path.clone());
    store.save(&LauncherSettings::default()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let writers = [false, true].map(|developer_mode| {
        let store = store.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..16 {
                store
                    .save(&LauncherSettings {
                        developer_mode,
                        ..Default::default()
                    })
                    .unwrap();
            }
        })
    });
    barrier.wait();
    for _ in 0..64 {
        let text = fs::read_to_string(&path).unwrap();
        let settings: LauncherSettings =
            toml::from_str(&text).expect("atomic replacement must not expose partial TOML");
        assert!(settings.close_to_tray && settings.dev_url.is_none());
        std::thread::yield_now();
    }
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn settings_debug_does_not_echo_a_developer_url() {
    let settings = LauncherSettings {
        dev_url: Some("https://private.invalid".into()),
        ..Default::default()
    };
    assert!(!format!("{settings:?}").contains("private"));
}

#[test]
#[cfg(windows)]
fn persistent_windows_share_lock_fails_without_losing_settings_or_leaving_temps() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.toml");
    let store = SettingsStore::at(path.clone());
    store.save(&LauncherSettings::default()).unwrap();
    let before = fs::read(&path).unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(
        store
            .save(&LauncherSettings {
                close_to_tray: false,
                ..Default::default()
            })
            .is_err()
    );
    drop(locked);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
