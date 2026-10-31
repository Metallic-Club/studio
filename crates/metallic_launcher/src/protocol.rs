//! Registering `metallic://` so links open the Launcher. Registration is per user (`HKCU`, no
//! administrator rights). The registry itself is behind [`UriRegistry`]; the Windows one is in
//! `platform`, and [`MemoryRegistry`] serves tests.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::error::LauncherError;

/// The key of the `metallic` scheme under `HKEY_CURRENT_USER`.
pub const PROTOCOL_KEY: &str = r"Software\Classes\metallic";

/// A small registry: keys with a default value and named values, all text.
pub trait UriRegistry {
    /// Sets a value (`name` `None` is the key's default value), creating the key.
    fn set(&self, key: &str, name: Option<&str>, value: &str) -> Result<(), LauncherError>;
    /// Reads a value; `None` when the key or value does not exist.
    fn get(&self, key: &str, name: Option<&str>) -> Result<Option<String>, LauncherError>;
    /// Deletes a key and everything below it. A missing key is not an error.
    fn delete_tree(&self, key: &str) -> Result<(), LauncherError>;
}

fn command_for(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Registers `exe` as the handler of `metallic://` under [`PROTOCOL_KEY`]. An older value, for
/// example a stale path after the Launcher moved, is overwritten.
pub fn register(reg: &dyn UriRegistry, exe: &Path) -> Result<(), LauncherError> {
    register_at(reg, PROTOCOL_KEY, exe)
}

/// [`register`] under another key (tests use a key of their own).
pub fn register_at(reg: &dyn UriRegistry, key: &str, exe: &Path) -> Result<(), LauncherError> {
    reg.set(key, None, "URL:Metallic Protocol")?;
    reg.set(key, Some("URL Protocol"), "")?;
    reg.set(
        &format!(r"{key}\DefaultIcon"),
        None,
        &format!("\"{}\",0", exe.display()),
    )?;
    reg.set(
        &format!(r"{key}\shell\open\command"),
        None,
        &command_for(exe),
    )
}

/// Removes the registration, but only when the command points at `exe`: another program's
/// handler is left alone.
pub fn unregister(reg: &dyn UriRegistry, exe: &Path) -> Result<(), LauncherError> {
    unregister_at(reg, PROTOCOL_KEY, exe)
}

/// [`unregister`] under another key.
pub fn unregister_at(reg: &dyn UriRegistry, key: &str, exe: &Path) -> Result<(), LauncherError> {
    if is_registered_to_at(reg, key, exe) {
        reg.delete_tree(key)?;
    }
    Ok(())
}

/// True when the registered command is the one [`register`] writes for `exe`.
pub fn is_registered_to(reg: &dyn UriRegistry, exe: &Path) -> bool {
    is_registered_to_at(reg, PROTOCOL_KEY, exe)
}

/// [`is_registered_to`] under another key.
pub fn is_registered_to_at(reg: &dyn UriRegistry, key: &str, exe: &Path) -> bool {
    matches!(
        reg.get(&format!(r"{key}\shell\open\command"), None),
        Ok(Some(command)) if command == command_for(exe)
    )
}

/// True for a copy installed by Velopack: `<root>\current\<exe>` with `Update.exe` in `<root>`.
/// A copy run from `target\` is not installed and never registers itself.
pub fn is_installed_copy(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else {
        return false;
    };
    dir.file_name().is_some_and(|name| name == "current")
        && dir
            .parent()
            .is_some_and(|root| root.join("Update.exe").is_file())
}

/// A registry in memory, for tests. Key and value names ignore case, like the real one.
#[derive(Default)]
pub struct MemoryRegistry {
    values: Mutex<HashMap<(String, Option<String>), String>>,
}

fn normal(key: &str, name: Option<&str>) -> (String, Option<String>) {
    (
        key.trim_matches('\\').to_lowercase(),
        name.map(str::to_lowercase),
    )
}

fn lock_error<T>(_: T) -> LauncherError {
    LauncherError::Platform("registry lock".to_string())
}

impl UriRegistry for MemoryRegistry {
    fn set(&self, key: &str, name: Option<&str>, value: &str) -> Result<(), LauncherError> {
        let mut values = self.values.lock().map_err(lock_error)?;
        values.insert(normal(key, name), value.to_string());
        Ok(())
    }

    fn get(&self, key: &str, name: Option<&str>) -> Result<Option<String>, LauncherError> {
        let values = self.values.lock().map_err(lock_error)?;
        Ok(values.get(&normal(key, name)).cloned())
    }

    fn delete_tree(&self, key: &str) -> Result<(), LauncherError> {
        let (key, _) = normal(key, None);
        let below = format!("{key}\\");
        let mut values = self.values.lock().map_err(lock_error)?;
        values.retain(|(k, _), _| *k != key && !k.starts_with(&below));
        Ok(())
    }
}
