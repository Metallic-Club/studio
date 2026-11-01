//! Tests for the settings file and the start-up arguments.

use metallic_launcher::{
    DeepLink, LauncherConfig, LauncherSettings, SettingsPatch, SettingsStore, Theme,
};

#[test]
fn missing_file_gives_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = SettingsStore::at(tmp.path().join("none.toml")).load();
    assert!(settings.close_to_tray);
    assert_eq!(settings, LauncherSettings::default());
}

#[test]
fn corrupt_settings_fall_back_to_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("settings.toml");
    std::fs::write(&path, "close_to_tray = [").unwrap();
    assert_eq!(SettingsStore::at(path).load(), LauncherSettings::default());
}

#[test]
fn old_files_without_new_fields_still_load() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("settings.toml");
    std::fs::write(&path, "developer_mode = true\n").unwrap();
    let settings = SettingsStore::at(path).load();
    assert!(settings.developer_mode);
    assert!(settings.close_to_tray);
}

#[test]
fn theme_defaults_to_graphite_and_an_unknown_word_keeps_the_other_settings() {
    assert_eq!(LauncherSettings::default().theme, Theme::Graphite);
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("settings.toml");
    std::fs::write(
        &path,
        "close_to_tray = false
theme = \"chrome\"
",
    )
    .unwrap();
    let settings = SettingsStore::at(path.clone()).load();
    assert!(!settings.close_to_tray);
    assert_eq!(settings.theme, Theme::Graphite);
    std::fs::write(
        &path,
        "theme = \"system\"
",
    )
    .unwrap();
    assert_eq!(SettingsStore::at(path).load().theme, Theme::System);
}

#[test]
fn a_theme_that_is_not_a_word_resets_only_the_theme() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("settings.toml");
    for bad in ["3", "true", "[\"light\"]", "{ name = \"light\" }"] {
        std::fs::write(
            &path,
            format!("close_to_tray = false\ndeveloper_mode = true\ntheme = {bad}\n"),
        )
        .unwrap();
        let settings = SettingsStore::at(path.clone()).load();
        assert!(!settings.close_to_tray, "{bad}");
        assert!(settings.developer_mode, "{bad}");
        assert_eq!(settings.theme, Theme::Graphite, "{bad}");
    }
}

#[test]
fn every_named_theme_is_a_shared_token_theme() {
    // The pages set `data-theme` to this word; the token CSS must have a block for it.
    let set = metallic_tokens::parse(metallic_tokens::TOKENS_JSON).unwrap();
    for theme in Theme::ALL {
        if theme != Theme::System {
            assert!(set.theme_ids().contains(&theme.as_str()), "{theme:?}");
        }
    }
    assert_eq!(set.theme_ids().len(), Theme::ALL.len() - 1);
}

#[test]
fn save_then_load_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SettingsStore::at(tmp.path().join("deep").join("settings.toml"));
    let settings = LauncherSettings {
        close_to_tray: false,
        developer_mode: true,
        dev_url: Some("http://localhost:3000".to_string()),
        channel: metallic_update::Channel::Stable,
        theme: Theme::Titanium,
    };
    store.save(&settings).unwrap();
    assert_eq!(store.load(), settings);
    assert!(!tmp.path().join("deep").join("settings.toml.tmp").exists());
    // Saving defaults (no dev_url) works too.
    store.save(&LauncherSettings::default()).unwrap();
    assert_eq!(store.load(), LauncherSettings::default());
}

#[test]
fn patch_applies_and_rejects_non_loopback_dev_url() {
    let mut settings = LauncherSettings::default();
    settings
        .apply(SettingsPatch {
            close_to_tray: Some(false),
            dev_url: Some(Some("http://127.0.0.1:3000".to_string())),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert!(!settings.close_to_tray);
    assert_eq!(settings.dev_url.as_deref(), Some("http://127.0.0.1:3000"));

    let before = settings.clone();
    let result = settings.apply(SettingsPatch {
        close_to_tray: Some(true),
        dev_url: Some(Some("http://evil.test".to_string())),
        ..SettingsPatch::default()
    });
    assert!(result.is_err());
    assert_eq!(settings, before);

    settings
        .apply(SettingsPatch {
            dev_url: Some(None),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(settings.dev_url, None);
}

#[test]
fn uri_argument_disables_other_flags() {
    let c = LauncherConfig::from_args(
        [
            "metallic://install".into(),
            "--dev".into(),
            "--dev-url".into(),
            "http://127.0.0.1:1".into(),
        ],
        &LauncherSettings::default(),
    )
    .unwrap();
    assert!(!c.dev_mode && c.dev_origin.is_none() && c.start_link == Some(DeepLink::Install));
    assert!(!c.start_hidden && !c.exit_after_load);
}

#[test]
fn uri_argument_ignores_developer_mode_setting_and_any_case() {
    let settings = LauncherSettings {
        developer_mode: true,
        dev_url: Some("http://127.0.0.1:1".to_string()),
        ..LauncherSettings::default()
    };
    let c = LauncherConfig::from_args(["METALLIC://install".to_string()], &settings).unwrap();
    assert!(!c.dev_mode && c.dev_origin.is_none());
    assert!(LauncherConfig::from_args(["metallic://play?place=x".to_string()], &settings).is_err());
}

#[test]
fn flags_are_read_and_saved_dev_url_needs_developer_mode() {
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let c = LauncherConfig::from_args(
        args(&["--hidden", "--exit-after-load", "--bogus"]),
        &LauncherSettings::default(),
    )
    .unwrap();
    assert!(c.start_hidden && c.exit_after_load && !c.dev_mode && c.start_link.is_none());

    let settings = LauncherSettings {
        developer_mode: true,
        dev_url: Some("http://localhost:4000".to_string()),
        ..LauncherSettings::default()
    };
    let c = LauncherConfig::from_args(args(&[]), &settings).unwrap();
    assert!(c.dev_mode && c.dev_origin.is_some());
    let off = LauncherSettings {
        developer_mode: false,
        ..settings
    };
    let c = LauncherConfig::from_args(args(&[]), &off).unwrap();
    assert!(!c.dev_mode && c.dev_origin.is_none());
    assert!(LauncherConfig::from_args(args(&["--dev", "--dev-url"]), &off).is_err());
}
