//! Bounded settings loading and atomic same-directory replacement, without credentials.

use crate::{LauncherError, SiteOrigin};
use serde::{Deserialize, Serialize};
use std::{
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Maximum settings file size, checked with a bounded read before TOML parsing.
pub const MAX_SETTINGS_BYTES: usize = 64 * 1024;

/// User preferences. Missing fields receive defaults when old files are read.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherSettings {
    /// Keep the launcher in the tray when its window closes; defaults to true.
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    /// Enable explicitly requested developer behavior; defaults to false.
    #[serde(default)]
    pub developer_mode: bool,
    /// Developer origin, validated before persistence or use.
    #[serde(default)]
    pub dev_url: Option<String>,
}

fn default_close_to_tray() -> bool {
    true
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            developer_mode: false,
            dev_url: None,
        }
    }
}

impl fmt::Debug for LauncherSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LauncherSettings")
            .field("close_to_tray", &self.close_to_tray)
            .field("developer_mode", &self.developer_mode)
            .field("dev_url", &self.dev_url.as_ref().map(|_| "[configured]"))
            .finish()
    }
}

/// A partial settings edit. An absent dev_url leaves it unchanged; Some(None) clears it.
#[derive(Default)]
pub struct SettingsPatch {
    /// Change whether closing the window keeps the tray process.
    pub close_to_tray: Option<bool>,
    /// Change developer mode.
    pub developer_mode: Option<bool>,
    /// Replace or clear the developer origin.
    pub dev_url: Option<Option<String>>,
}

impl LauncherSettings {
    fn validate(&self) -> Result<(), LauncherError> {
        if let Some(url) = &self.dev_url {
            SiteOrigin::parse(url, true)?;
        }
        Ok(())
    }

    /// Applies an edit only after all fields validate; errors leave every old field unchanged.
    pub fn apply(&mut self, patch: SettingsPatch) -> Result<(), LauncherError> {
        let candidate = Self {
            close_to_tray: patch.close_to_tray.unwrap_or(self.close_to_tray),
            developer_mode: patch.developer_mode.unwrap_or(self.developer_mode),
            dev_url: patch.dev_url.unwrap_or_else(|| self.dev_url.clone()),
        };
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}

/// Persistence in the user's configuration folder. The store contains no credentials.
#[derive(Clone)]
pub struct SettingsStore {
    path: Option<PathBuf>,
}

impl SettingsStore {
    /// Uses METALLIC_LAUNCHER_CONFIG_DIR, then APPDATA/Metallic/Launcher, then XDG/HOME fallbacks.
    /// If none exist, loading still returns defaults; saving reports unavailable storage.
    pub fn open_default() -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let directory = var("METALLIC_LAUNCHER_CONFIG_DIR")
            .or_else(|| var("APPDATA").map(|path| path.join("Metallic/Launcher")))
            .or_else(|| var("XDG_CONFIG_HOME").map(|path| path.join("metallic/launcher")))
            .or_else(|| var("HOME").map(|path| path.join(".config/metallic/launcher")));
        Self {
            path: directory.map(|path| path.join("settings.toml")),
        }
    }

    /// Uses an explicit settings file path, useful for tests and portable configuration.
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    /// Returns defaults for missing, unreadable, invalid or oversized input, with a redacted warning.
    pub fn load(&self) -> LauncherSettings {
        let Some(path) = &self.path else {
            return LauncherSettings::default();
        };
        let load = || -> Option<LauncherSettings> {
            let file = fs::File::open(path).ok()?;
            if !file.metadata().ok()?.is_file() {
                return None;
            }
            let mut bytes = Vec::new();
            file.take((MAX_SETTINGS_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() > MAX_SETTINGS_BYTES {
                return None;
            }
            let text = std::str::from_utf8(&bytes).ok()?;
            let settings: LauncherSettings = toml::from_str(text).ok()?;
            settings.validate().ok()?;
            Some(settings)
        };
        load().unwrap_or_else(|| {
            log::warn!("Launcher settings could not be loaded; using defaults.");
            LauncherSettings::default()
        })
    }

    /// Validates, writes/syncs a unique sibling temporary file, then atomically replaces the target.
    /// A failed write/replacement cleans up its temporary file and preserves existing settings.
    /// Transient permission/sharing failures receive three bounded retries (10, 20, 40 ms).
    /// Atomic visibility does not promise durability across power loss of the containing directory.
    pub fn save(&self, settings: &LauncherSettings) -> Result<(), LauncherError> {
        settings.validate()?;
        let unavailable = || LauncherError::Io("Launcher settings could not be saved.".into());
        let path = self.path.as_deref().ok_or_else(unavailable)?;
        let text = toml::to_string(settings).map_err(|_| unavailable())?;
        if text.len() > MAX_SETTINGS_BYTES {
            return Err(unavailable());
        }
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|_| unavailable())?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".settings-")
            .suffix(".tmp")
            .tempfile_in(parent)
            .map_err(|_| unavailable())?;
        temporary
            .write_all(text.as_bytes())
            .map_err(|_| unavailable())?;
        temporary.as_file().sync_all().map_err(|_| unavailable())?;
        // Close the write handle before rename: another writer can otherwise see
        // an open, just-persisted target on Windows while replacing it again.
        persist_with_retry(temporary.into_temp_path(), path).map_err(|error| {
            LauncherError::Io(format!(
                "Launcher settings could not be replaced ({:?}).",
                error.kind()
            ))
        })?;
        Ok(())
    }
}

fn persist_with_retry(mut temporary: tempfile::TempPath, target: &Path) -> std::io::Result<()> {
    let mut delays = [10, 20, 40].into_iter();
    loop {
        match temporary.persist(target) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if matches!(
                    error.error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
                ) && let Some(delay) = delays.next()
                {
                    temporary = error.path;
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                } else {
                    return Err(error.error);
                }
            }
        }
    }
}
