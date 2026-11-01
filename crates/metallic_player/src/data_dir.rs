//! The Player's own data folder, where per-game input overrides live
//! (`<data dir>/input/<game id>/local.json`, docs/design/input-actions.md section 8.4).
//!
//! - `METALLIC_PLAYER_DATA_DIR` when set (tests, portable installs).
//! - Windows: `%APPDATA%\MetallicPlayer` (roaming settings, beside the Launcher's
//!   `%APPDATA%\MetallicLauncher`). Never `%LOCALAPPDATA%\Metallic`, which another app owns.
//! - macOS: `~/Library/Application Support/MetallicPlayer`.
//! - Other systems: `$XDG_DATA_HOME/metallic-player`, else `~/.local/share/metallic-player`.
use std::ffi::OsString;
use std::path::PathBuf;

/// Environment variable that overrides the folder.
pub const DATA_DIR_ENV: &str = "METALLIC_PLAYER_DATA_DIR";
/// The folder name on Windows and macOS.
pub const PLAYER_FOLDER: &str = "MetallicPlayer";

/// The Player's data folder for this machine, if one can be found.
pub fn data_dir() -> Option<PathBuf> {
    resolve(|k| std::env::var_os(k), Os::current())
}

/// The operating system families with different folder rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Os {
    Windows,
    Mac,
    Other,
}

impl Os {
    fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Other
        }
    }
}

fn resolve(var: impl Fn(&str) -> Option<OsString>, os: Os) -> Option<PathBuf> {
    let non_empty = |k: &str| var(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = non_empty(DATA_DIR_ENV) {
        return Some(dir);
    }
    match os {
        Os::Windows => non_empty("APPDATA").map(|d| d.join(PLAYER_FOLDER)),
        Os::Mac => non_empty("HOME").map(|h| {
            h.join("Library")
                .join("Application Support")
                .join(PLAYER_FOLDER)
        }),
        Os::Other => non_empty("XDG_DATA_HOME")
            .or_else(|| non_empty("HOME").map(|h| h.join(".local").join("share")))
            .map(|d| d.join("metallic-player")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn windows_uses_the_players_own_roaming_folder() {
        let vars = env(&[("APPDATA", "R"), ("LOCALAPPDATA", "L")]);
        let dir = resolve(vars, Os::Windows).unwrap();
        assert_eq!(dir, PathBuf::from("R").join("MetallicPlayer"));
        // Never the folder another app owns.
        assert!(!dir.starts_with(PathBuf::from("L").join("Metallic")));
    }

    #[test]
    fn the_environment_variable_wins() {
        let vars = env(&[(DATA_DIR_ENV, "X"), ("APPDATA", "R")]);
        assert_eq!(resolve(vars, Os::Windows), Some(PathBuf::from("X")));
    }

    #[test]
    fn other_systems_follow_xdg() {
        let vars = env(&[("HOME", "H")]);
        assert_eq!(
            resolve(vars, Os::Other),
            Some(PathBuf::from("H").join(".local/share/metallic-player"))
        );
        let vars = env(&[("XDG_DATA_HOME", "D"), ("HOME", "H")]);
        assert_eq!(
            resolve(vars, Os::Other),
            Some(PathBuf::from("D").join("metallic-player"))
        );
        assert_eq!(resolve(env(&[]), Os::Mac), None);
    }
}
