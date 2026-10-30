//! Tests for the `metallic://` registration, with the in-memory registry.

use std::path::Path;

use metallic_launcher::protocol::{is_installed_copy, is_registered_to, register, unregister};
use metallic_launcher::{MemoryRegistry, PROTOCOL_KEY, UriRegistry};

const EXE: &str = r"C:\Users\A B\Metallic.exe";

fn get(reg: &MemoryRegistry, sub: &str, name: Option<&str>) -> Option<String> {
    let key = if sub.is_empty() {
        PROTOCOL_KEY.to_string()
    } else {
        format!(r"{PROTOCOL_KEY}\{sub}")
    };
    reg.get(&key, name).unwrap()
}

#[test]
fn register_writes_expected_keys() {
    let reg = MemoryRegistry::default();
    register(&reg, Path::new(EXE)).unwrap();
    assert_eq!(
        get(&reg, r"shell\open\command", None).unwrap(),
        "\"C:\\Users\\A B\\Metallic.exe\" \"%1\""
    );
    assert_eq!(get(&reg, "", Some("URL Protocol")).unwrap(), "");
    assert_eq!(get(&reg, "", None).unwrap(), "URL:Metallic Protocol");
    assert_eq!(
        get(&reg, "DefaultIcon", None).unwrap(),
        "\"C:\\Users\\A B\\Metallic.exe\",0"
    );
    assert!(is_registered_to(&reg, Path::new(EXE)));
}

#[test]
fn reregister_overwrites_stale_path() {
    let reg = MemoryRegistry::default();
    register(&reg, Path::new(r"C:\Old\Metallic.exe")).unwrap();
    assert!(!is_registered_to(&reg, Path::new(EXE)));
    register(&reg, Path::new(EXE)).unwrap();
    assert!(is_registered_to(&reg, Path::new(EXE)));
    assert!(!is_registered_to(&reg, Path::new(r"C:\Old\Metallic.exe")));
}

#[test]
fn unregister_removes_own_registration() {
    let reg = MemoryRegistry::default();
    register(&reg, Path::new(EXE)).unwrap();
    unregister(&reg, Path::new(EXE)).unwrap();
    assert!(get(&reg, "", None).is_none());
    assert!(get(&reg, r"shell\open\command", None).is_none());
}

#[test]
fn unregister_leaves_other_apps_registration() {
    let reg = MemoryRegistry::default();
    register(&reg, Path::new(r"C:\Other\App.exe")).unwrap();
    unregister(&reg, Path::new(EXE)).unwrap();
    assert!(is_registered_to(&reg, Path::new(r"C:\Other\App.exe")));
    assert!(get(&reg, "", None).is_some());
}

#[test]
fn only_a_velopack_copy_counts_as_installed() {
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join("current");
    std::fs::create_dir(&current).unwrap();
    let exe = current.join("Metallic.exe");
    assert!(!is_installed_copy(&exe), "no Update.exe yet");
    std::fs::write(dir.path().join("Update.exe"), b"").unwrap();
    assert!(is_installed_copy(&exe));
    // A build run from a target folder is never installed, even with an Update.exe above it.
    let target = dir.path().join("target");
    std::fs::create_dir(&target).unwrap();
    assert!(!is_installed_copy(&target.join("Metallic.exe")));
}
