//! The player's graphics settings on disk: one small text file in the user's config folder
//! ([`metallic_platform::UserPaths::config_dir`]), in the format of [`GraphicsSettings::to_text`].
//!
//! A missing file is the Auto preset; a file that cannot be read or is damaged is the Auto preset too, with a
//! warning saying why, and is written again whole on the next save. Saving writes a temporary file beside it and
//! renames it over the old one, so a crash mid-write never leaves half a file.

use std::io;
use std::path::{Path, PathBuf};

use metallic_platform::{AppId, PlatformError, UserPaths};
use metallic_render::settings::GraphicsSettings;

/// The settings file's name in the config folder.
pub const GRAPHICS_FILE: &str = "graphics.toml";

/// Where `app`'s graphics settings live for the current user.
pub fn graphics_settings_path(app: &AppId) -> Result<PathBuf, PlatformError> {
    Ok(UserPaths::for_app(app)?.config_dir().join(GRAPHICS_FILE))
}

/// What [`load_graphics_settings`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedGraphics {
    /// The settings: the file's, or the Auto preset when it is missing or damaged.
    pub settings: GraphicsSettings,
    /// Whether the file was there and read whole.
    pub from_file: bool,
    /// Why the file was not used, when it was there but could not be read or understood.
    pub problem: Option<String>,
}

/// Reads the settings at `path`. Never fails: a missing file gives the Auto preset, and a damaged one the Auto
/// preset with [`LoadedGraphics::problem`] saying what was wrong (the caller logs it).
pub fn load_graphics_settings(path: &Path) -> LoadedGraphics {
    let fallback = |problem: Option<String>| LoadedGraphics {
        settings: GraphicsSettings::default(),
        from_file: false,
        problem,
    };
    match std::fs::read_to_string(path) {
        Ok(text) => match GraphicsSettings::from_text(&text) {
            Ok(settings) => LoadedGraphics {
                settings,
                from_file: true,
                problem: None,
            },
            Err(error) => fallback(Some(format!("{}: {error}; using the Auto preset", path.display()))),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => fallback(None),
        Err(error) => fallback(Some(format!(
            "could not read {}: {error}; using the Auto preset",
            path.display()
        ))),
    }
}

/// Writes `settings` to `path`, making its folder first.
pub fn save_graphics_settings(path: &Path, settings: &GraphicsSettings) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let temporary = path.with_extension("toml.tmp");
    std::fs::write(&temporary, settings.to_text())?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use metallic_render::settings::{Calibration, Preset, Tier};

    use super::*;

    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-graphics-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_missing_file_is_auto_without_a_warning() {
        let loaded = load_graphics_settings(&folder("missing").join(GRAPHICS_FILE));
        assert_eq!(loaded.settings, GraphicsSettings::new(Preset::Auto));
        assert!(!loaded.from_file);
        assert_eq!(loaded.problem, None);
    }

    #[test]
    fn saved_settings_load_back() {
        let dir = folder("saved");
        let path = dir.join("nested").join(GRAPHICS_FILE);
        let settings = GraphicsSettings {
            render_scale: Some(0.8),
            calibration: Some(Calibration {
                tier: Tier::High,
                gpu_ms: 4.5,
                adapter: "GPU".into(),
                driver: "1".into(),
            }),
            ..GraphicsSettings::new(Preset::Auto)
        };
        save_graphics_settings(&path, &settings).expect("saved");
        let loaded = load_graphics_settings(&path);
        assert_eq!(loaded.settings, settings);
        assert!(loaded.from_file);
        assert!(!path.with_extension("toml.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_file_is_auto_with_a_warning() {
        let dir = folder("damaged");
        std::fs::create_dir_all(&dir).expect("folder");
        let path = dir.join(GRAPHICS_FILE);
        std::fs::write(&path, "preset = \"high\"\nrender_scale = lots\n").expect("written");
        let loaded = load_graphics_settings(&path);
        assert_eq!(loaded.settings, GraphicsSettings::new(Preset::Auto));
        assert!(!loaded.from_file);
        let problem = loaded.problem.expect("a warning");
        assert!(problem.contains("line 2") && problem.contains("Auto"), "{problem}");
        // Not UTF-8 at all.
        std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("written");
        assert!(load_graphics_settings(&path).problem.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
