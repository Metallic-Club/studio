//! The theme the creator picked in Studio Settings, readable by every Metallic tool.
//!
//! Studio keeps `settings.toml` in its config folder ([`resolve_config_dir`]). Other tools read
//! only the `theme` line from it and never write it, so they follow Studio's choice without
//! depending on Studio. A missing, unreadable or unknown value gives Graphite.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::ThemeId;

/// The folder name under `%APPDATA%` (the Velopack pack id).
pub const STUDIO_FOLDER: &str = "MetallicStudio";
/// The settings file name inside the config folder.
pub const SETTINGS_FILE: &str = "settings.toml";
/// Largest `settings.toml` that is read (64 KiB); a bigger file counts as corrupt.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;

/// Which theme the creator picked. `System` follows the Windows light or dark setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    /// Always Graphite (the default).
    #[default]
    Graphite,
    /// Always Titanium.
    Titanium,
    /// Always Light.
    Light,
    /// Light when the system is light, Graphite when it is dark.
    System,
}

impl ThemeChoice {
    /// Every choice, in the order Settings lists them.
    pub const ALL: [ThemeChoice; 4] = [
        ThemeChoice::Graphite,
        ThemeChoice::Titanium,
        ThemeChoice::Light,
        ThemeChoice::System,
    ];

    /// The name stored in `settings.toml`.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeChoice::Graphite => "graphite",
            ThemeChoice::Titanium => "titanium",
            ThemeChoice::Light => "light",
            ThemeChoice::System => "system",
        }
    }

    /// The name creators see.
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Graphite => "Graphite",
            ThemeChoice::Titanium => "Titanium",
            ThemeChoice::Light => "Light",
            ThemeChoice::System => "System",
        }
    }

    /// Reads a stored name (any letter case); `None` for an unknown one.
    pub fn parse(text: &str) -> Option<ThemeChoice> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("system") {
            return Some(ThemeChoice::System);
        }
        ThemeId::parse(text).map(ThemeChoice::from)
    }

    /// The theme to draw, given what the system reports (`None` when it does not say).
    pub fn resolve(self, system: Option<egui::Theme>) -> ThemeId {
        match self {
            ThemeChoice::Graphite => ThemeId::Graphite,
            ThemeChoice::Titanium => ThemeId::Titanium,
            ThemeChoice::Light => ThemeId::Light,
            ThemeChoice::System => match system {
                Some(egui::Theme::Light) => ThemeId::Light,
                _ => ThemeId::Graphite,
            },
        }
    }
}

impl From<ThemeId> for ThemeChoice {
    fn from(id: ThemeId) -> Self {
        match id {
            ThemeId::Graphite => ThemeChoice::Graphite,
            ThemeId::Titanium => ThemeChoice::Titanium,
            ThemeId::Light => ThemeChoice::Light,
        }
    }
}

/// Studio's folder under an app-data base: `<base>\MetallicStudio`.
pub fn config_dir_in(base: &Path) -> PathBuf {
    base.join(STUDIO_FOLDER)
}

/// Where Studio keeps its settings, with the environment passed in: `METALLIC_STUDIO_CONFIG_DIR`,
/// else `%APPDATA%\MetallicStudio`, else `$XDG_CONFIG_HOME/MetallicStudio`, else
/// `~/.config/MetallicStudio`. Empty values count as unset. `None` if nothing is known.
pub fn resolve_config_dir(var: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let get = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = get("METALLIC_STUDIO_CONFIG_DIR") {
        return Some(dir);
    }
    if let Some(appdata) = get("APPDATA") {
        return Some(config_dir_in(&appdata));
    }
    if let Some(xdg) = get("XDG_CONFIG_HOME") {
        return Some(config_dir_in(&xdg));
    }
    get("HOME").map(|home| config_dir_in(&home.join(".config")))
}

/// The `theme` value of a `settings.toml` text; `None` when the text is not TOML or the value is
/// missing or unknown.
pub fn parse_theme_choice(text: &str) -> Option<ThemeChoice> {
    let table: toml::Table = text.parse().ok()?;
    ThemeChoice::parse(table.get("theme")?.as_str()?)
}

/// The theme choice saved in `file`; Graphite when the file is missing, too big or unreadable.
pub fn load_theme_choice(file: &Path) -> ThemeChoice {
    let small = std::fs::metadata(file).is_ok_and(|m| m.len() <= MAX_FILE_BYTES);
    small
        .then(|| std::fs::read_to_string(file).ok())
        .flatten()
        .and_then(|text| parse_theme_choice(&text))
        .unwrap_or_default()
}

/// The theme choice Studio Settings saved for this user (Graphite when none).
pub fn saved_theme_choice() -> ThemeChoice {
    resolve_config_dir(&|name| std::env::var_os(name))
        .map(|dir| load_theme_choice(&dir.join(SETTINGS_FILE)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_line_is_read_in_any_case() {
        assert_eq!(
            parse_theme_choice("theme = \"Titanium\"\ninterface_size = 120\n"),
            Some(ThemeChoice::Titanium)
        );
        assert_eq!(
            parse_theme_choice("theme = \"system\""),
            Some(ThemeChoice::System)
        );
    }

    #[test]
    fn bad_or_missing_values_give_graphite() {
        assert_eq!(parse_theme_choice("theme = \"neon\""), None);
        assert_eq!(parse_theme_choice("theme = 3"), None);
        assert_eq!(parse_theme_choice("not toml ["), None);
        assert_eq!(
            load_theme_choice(Path::new("no-such-folder/settings.toml")),
            ThemeChoice::Graphite
        );
    }

    #[test]
    fn a_saved_file_is_followed() {
        let dir = std::env::temp_dir().join(format!("mui-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join(SETTINGS_FILE);
        std::fs::write(&file, "theme = \"light\"\n").expect("write");
        assert_eq!(load_theme_choice(&file), ThemeChoice::Light);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_dir_prefers_the_override() {
        let var = |name: &str| match name {
            "METALLIC_STUDIO_CONFIG_DIR" => Some(OsString::from("override")),
            "APPDATA" => Some(OsString::from("appdata")),
            _ => None,
        };
        assert_eq!(resolve_config_dir(&var), Some(PathBuf::from("override")));
        let var = |name: &str| (name == "APPDATA").then(|| OsString::from("appdata"));
        assert_eq!(
            resolve_config_dir(&var),
            Some(PathBuf::from("appdata").join(STUDIO_FOLDER))
        );
    }
}
