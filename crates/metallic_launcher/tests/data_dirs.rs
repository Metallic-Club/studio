//! Tests for the Launcher's own data folders and the marker-guarded cleanup. They use temp
//! folders and an injected base path, never the real `%LOCALAPPDATA%`.

use std::path::{Component, Path, PathBuf};

use metallic_launcher::data_dirs::{
    DATA_MARKER, RemoveOutcome, clean_up_in, config_dir, create_owned, local_data_dir,
    remove_owned, webview_dir,
};
use metallic_launcher::{SettingsStore, downloads_dir_for};

fn temp(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("metallic-data-dirs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn names(path: &Path, base: &Path) -> Vec<String> {
    path.strip_prefix(base)
        .unwrap()
        .components()
        .map(|c| match c {
            Component::Normal(n) => n.to_string_lossy().to_string(),
            _ => String::new(),
        })
        .collect()
}

#[test]
fn every_data_path_is_under_a_metallic_launcher_folder() {
    let base = Path::new("base");
    for path in [
        local_data_dir(base),
        webview_dir(base),
        downloads_dir_for(base),
        config_dir(base),
    ] {
        let parts = names(&path, base);
        assert_eq!(parts[0], "MetallicLauncher", "{path:?}");
        assert!(
            !parts.iter().any(|p| p.eq_ignore_ascii_case("Metallic")),
            "{path:?}"
        );
    }
}

#[test]
fn cleanup_leaves_a_folder_without_the_marker_alone() {
    let base = temp("unmarked");
    let folder = local_data_dir(&base);
    std::fs::create_dir_all(folder.join("webview2")).unwrap();
    std::fs::write(folder.join("brain.json"), "other app").unwrap();
    assert_eq!(remove_owned(&folder), RemoveOutcome::NotOurs);
    assert!(folder.join("brain.json").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cleanup_removes_a_marked_folder() {
    let base = temp("marked");
    let folder = local_data_dir(&base);
    create_owned(&folder).unwrap();
    assert!(folder.join(DATA_MARKER).exists());
    std::fs::create_dir_all(webview_dir(&base)).unwrap();
    assert_eq!(remove_owned(&folder), RemoveOutcome::Removed);
    assert!(!folder.exists());
    assert_eq!(remove_owned(&folder), RemoveOutcome::Missing);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_folder_that_already_existed_is_not_marked() {
    let base = temp("existing");
    let folder = local_data_dir(&base);
    std::fs::create_dir_all(&folder).unwrap();
    create_owned(&folder).unwrap();
    assert!(!folder.join(DATA_MARKER).exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn clean_up_in_touches_only_marked_launcher_folders() {
    let local = temp("clean-local");
    let roaming = temp("clean-roaming");
    // The other app's folders sit beside ours and must survive.
    let other_local = local.join("Metallic");
    let other_roaming = roaming.join("Metallic");
    for dir in [&other_local, &other_roaming] {
        std::fs::create_dir_all(dir.join("Launcher")).unwrap();
        std::fs::write(dir.join("metallic.exe"), "x").unwrap();
    }
    create_owned(&local_data_dir(&local)).unwrap();
    // A settings folder we did not create: no marker.
    std::fs::create_dir_all(config_dir(&roaming)).unwrap();
    std::fs::write(config_dir(&roaming).join("settings.toml"), "x").unwrap();

    clean_up_in(Some(&local), Some(&roaming));

    assert!(!local_data_dir(&local).exists());
    assert!(config_dir(&roaming).join("settings.toml").exists());
    assert!(other_local.join("metallic.exe").exists());
    assert!(other_roaming.join("Launcher").exists());
    let _ = std::fs::remove_dir_all(&local);
    let _ = std::fs::remove_dir_all(&roaming);
}

#[test]
fn saving_settings_marks_a_folder_it_creates() {
    let base = temp("settings");
    let dir = config_dir(&base);
    let store = SettingsStore::at(dir.join("settings.toml"));
    store.save(&store.load()).unwrap();
    assert!(dir.join(DATA_MARKER).exists());
    clean_up_in(None, Some(&base));
    assert!(!dir.exists());
    let _ = std::fs::remove_dir_all(&base);
}
