//! The Launcher's settings file, `settings.toml`.
//!
//! Stored in `%APPDATA%\MetallicLauncher`, or in the folder named by the environment variable
//! `METALLIC_LAUNCHER_CONFIG_DIR` (tests, portable runs). A missing or damaged file gives the
//! defaults; saving writes a temporary file and renames it, so a crash never leaves half a file.

use std::path::PathBuf;

use metallic_update::Channel;
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::LauncherError;
use crate::site::SiteOrigin;

/// Environment variable that replaces the settings folder.
pub const CONFIG_DIR_ENV: &str = "METALLIC_LAUNCHER_CONFIG_DIR";

const FILE_NAME: &str = "settings.toml";

fn default_close_to_tray() -> bool {
    true
}

/// What the user can change in the Launcher. Every field has a default, so old files still load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherSettings {
    /// Closing the window hides it to the tray instead of quitting. Default `true`.
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    /// Developer mode: allows `--dev-url` and the saved development address.
    #[serde(default)]
    pub developer_mode: bool,
    /// A development server to trust like the site (loopback `http` allowed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_url: Option<String>,
    /// Which releases the Launcher and Studio installs follow. Default `dev`. Only the Launcher's
    /// own settings page may change it, and it applies after a restart.
    #[serde(default)]
    pub channel: Channel,
    /// The colour theme of the Launcher's own pages. Default Graphite. An unknown word in the
    /// file reads as the default, so a newer file never resets the other settings.
    #[serde(default, deserialize_with = "theme_or_default")]
    pub theme: Theme,
}

/// The colour theme of the Launcher's bundled pages: one of the shared design-token themes, or
/// `System`, which follows the Windows light/dark setting (light: Light, dark: Graphite).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the system's light or dark setting.
    System,
    /// The default dark theme.
    #[default]
    Graphite,
    /// A lighter dark theme.
    Titanium,
    /// The light ("Aluminium") theme.
    Light,
}

impl Theme {
    /// Every theme, in the order the settings page lists them.
    pub const ALL: [Theme; 4] = [
        Theme::System,
        Theme::Graphite,
        Theme::Titanium,
        Theme::Light,
    ];

    /// The word used in the settings file, the bridge and the pages' `data-theme` attribute.
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Graphite => "graphite",
            Theme::Titanium => "titanium",
            Theme::Light => "light",
        }
    }
}

/// Reads a theme word, giving the default for a word this build does not know or a value that is
/// not a word at all (`theme = 3`), so a bad theme never costs the other settings.
fn theme_or_default<'de, D>(deserializer: D) -> Result<Theme, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Word(String),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Value::deserialize(deserializer)? {
        Value::Word(word) => Theme::ALL
            .into_iter()
            .find(|theme| theme.as_str() == word)
            .unwrap_or_default(),
        Value::Other(_) => Theme::default(),
    })
}

impl Default for LauncherSettings {
    fn default() -> Self {
        LauncherSettings {
            close_to_tray: default_close_to_tray(),
            developer_mode: false,
            dev_url: None,
            channel: Channel::default(),
            theme: Theme::default(),
        }
    }
}

/// Reads a JSON field that may be absent (`None`), `null` (`Some(None)`) or a value (`Some(Some(v))`).
fn present_or_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A change to apply to the settings; `None` leaves a field alone. Read from the bridge as
/// camelCase JSON (`closeToTray`, `developerMode`, `devUrl`, `channel`, `theme`); unknown fields
/// are refused.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    /// New value for `close_to_tray`.
    #[serde(default)]
    pub close_to_tray: Option<bool>,
    /// New value for `developer_mode`.
    #[serde(default)]
    pub developer_mode: Option<bool>,
    /// New value for `dev_url`: `Some(None)` (JSON `null`) clears it.
    #[serde(default, deserialize_with = "present_or_null")]
    pub dev_url: Option<Option<String>>,
    /// New value for `channel` (`"dev"` or `"stable"`).
    #[serde(default)]
    pub channel: Option<Channel>,
    /// New value for `theme` (`"system"`, `"graphite"`, `"titanium"` or `"light"`).
    #[serde(default)]
    pub theme: Option<Theme>,
}

impl LauncherSettings {
    /// Applies `patch`. If any part is invalid (a `dev_url` that is not an `https` or loopback
    /// `http` origin), nothing changes.
    pub fn apply(&mut self, patch: SettingsPatch) -> Result<(), LauncherError> {
        if let Some(Some(url)) = &patch.dev_url {
            SiteOrigin::parse(url, true)?;
        }
        if let Some(value) = patch.close_to_tray {
            self.close_to_tray = value;
        }
        if let Some(value) = patch.developer_mode {
            self.developer_mode = value;
        }
        if let Some(value) = patch.dev_url {
            self.dev_url = value;
        }
        if let Some(value) = patch.channel {
            self.channel = value;
        }
        if let Some(value) = patch.theme {
            self.theme = value;
        }
        Ok(())
    }
}

/// Reads and writes the settings file.
#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: Option<PathBuf>,
}

impl SettingsStore {
    /// The default location: `METALLIC_LAUNCHER_CONFIG_DIR`, else `%APPDATA%\MetallicLauncher`.
    /// With neither, loading gives the defaults and saving fails.
    pub fn open_default() -> Self {
        let dir = std::env::var_os(CONFIG_DIR_ENV)
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("APPDATA")
                    .map(|appdata| crate::data_dirs::config_dir(&PathBuf::from(appdata)))
            });
        SettingsStore {
            path: dir.map(|dir| dir.join(FILE_NAME)),
        }
    }

    /// A store at exactly this file.
    pub fn at(path: PathBuf) -> Self {
        SettingsStore { path: Some(path) }
    }

    /// Loads the settings. A missing, unreadable or damaged file gives the defaults (logged).
    pub fn load(&self) -> LauncherSettings {
        let Some(path) = &self.path else {
            return LauncherSettings::default();
        };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return LauncherSettings::default();
            }
            Err(error) => {
                log::warn!("could not read the settings file ({error}); using the defaults");
                return LauncherSettings::default();
            }
        };
        match toml::from_str(&text) {
            Ok(settings) => settings,
            Err(error) => {
                log::warn!("the settings file is damaged ({error}); using the defaults");
                LauncherSettings::default()
            }
        }
    }

    /// Saves the settings: writes `settings.toml.tmp`, then renames it over `settings.toml`.
    pub fn save(&self, settings: &LauncherSettings) -> Result<(), LauncherError> {
        let io = |e: std::io::Error| LauncherError::Io(e.to_string());
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| LauncherError::Io("no settings folder".to_string()))?;
        let text =
            toml::to_string_pretty(settings).map_err(|e| LauncherError::Io(e.to_string()))?;
        if let Some(dir) = path.parent() {
            // A folder created here is marked as ours; one that existed is left unmarked.
            crate::data_dirs::create_owned(dir).map_err(io)?;
        }
        let mut temp = path.clone().into_os_string();
        temp.push(".tmp");
        let temp = PathBuf::from(temp);
        std::fs::write(&temp, text).map_err(io)?;
        std::fs::rename(&temp, path).map_err(io)
    }
}
