//! Studio's own data folders live under `MetallicStudio`, never `Metallic`, and cleanup removes
//! only folders that hold Studio's marker. Every test uses a temp folder as the injected base.

use metallic_studio::data_dirs::{
    DATA_MARKER, RemoveOutcome, STUDIO_FOLDER, clean_up_in, config_dir_in, create_owned,
    remove_owned, resolve_config_dir,
};
use std::path::Path;

#[test]
fn folder_is_the_pack_id() {
    assert_eq!(STUDIO_FOLDER, "MetallicStudio");
}

#[test]
fn config_dir_is_under_the_injected_base_and_pack_id() {
    let base = Path::new("base");
    assert_eq!(config_dir_in(base), base.join("MetallicStudio"));
}

#[test]
fn resolve_prefers_override_then_appdata_then_xdg_then_home() {
    let get = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| std::ffi::OsString::from(*v))
        }
    };
    let all = get(&[
        ("METALLIC_STUDIO_CONFIG_DIR", "o"),
        ("APPDATA", "a"),
        ("XDG_CONFIG_HOME", "x"),
        ("HOME", "h"),
    ]);
    assert_eq!(resolve_config_dir(&all), Some(Path::new("o").to_path_buf()));
    let no_override = get(&[("APPDATA", "a"), ("XDG_CONFIG_HOME", "x"), ("HOME", "h")]);
    assert_eq!(
        resolve_config_dir(&no_override),
        Some(Path::new("a").join("MetallicStudio"))
    );
    let xdg = get(&[("XDG_CONFIG_HOME", "x"), ("HOME", "h")]);
    assert_eq!(
        resolve_config_dir(&xdg),
        Some(Path::new("x").join("MetallicStudio"))
    );
    let home = get(&[("HOME", "h")]);
    assert_eq!(
        resolve_config_dir(&home),
        Some(Path::new("h").join(".config").join("MetallicStudio"))
    );
    assert_eq!(resolve_config_dir(&get(&[])), None);
    // No path may name the bare `Metallic` folder.
    for p in [&no_override, &xdg, &home] {
        let dir = resolve_config_dir(p).unwrap();
        assert!(!dir.components().any(|c| c.as_os_str() == "Metallic"));
    }
}

#[test]
fn create_owned_marks_a_new_folder() {
    let base = tempfile::tempdir().unwrap();
    let dir = config_dir_in(base.path());
    create_owned(&dir).unwrap();
    assert!(dir.join(DATA_MARKER).is_file());
}

#[test]
fn create_owned_leaves_an_existing_folder_unmarked() {
    let base = tempfile::tempdir().unwrap();
    let dir = config_dir_in(base.path());
    std::fs::create_dir_all(&dir).unwrap();
    create_owned(&dir).unwrap();
    assert!(!dir.join(DATA_MARKER).exists());
}

#[test]
fn remove_refuses_a_folder_without_the_marker() {
    let base = tempfile::tempdir().unwrap();
    let dir = config_dir_in(base.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("metallic.exe"), b"someone else's").unwrap();
    assert_eq!(remove_owned(&dir), RemoveOutcome::NotOurs);
    assert!(dir.join("metallic.exe").is_file());
}

#[test]
fn remove_deletes_a_marked_folder_and_reports_missing() {
    let base = tempfile::tempdir().unwrap();
    let dir = config_dir_in(base.path());
    create_owned(&dir).unwrap();
    std::fs::write(dir.join("recent.toml"), b"x").unwrap();
    assert_eq!(remove_owned(&dir), RemoveOutcome::Removed);
    assert!(!dir.exists());
    assert_eq!(remove_owned(&dir), RemoveOutcome::Missing);
}

#[test]
fn clean_up_leaves_the_other_apps_metallic_folders_alone() {
    let roaming = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let other = local.path().join("Metallic");
    std::fs::create_dir_all(other.join("Launcher")).unwrap();
    std::fs::write(other.join("metallic.exe"), b"x").unwrap();
    create_owned(&config_dir_in(roaming.path())).unwrap();
    create_owned(&config_dir_in(local.path())).unwrap();

    clean_up_in(Some(local.path()), Some(roaming.path()));

    assert!(!config_dir_in(roaming.path()).exists());
    assert!(!config_dir_in(local.path()).exists());
    assert!(other.join("metallic.exe").is_file());
    assert!(other.join("Launcher").is_dir());
}
