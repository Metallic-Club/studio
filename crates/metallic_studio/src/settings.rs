//! Studio's own settings: theme, interface size and reduced motion, kept in `settings.toml` in
//! the config folder (see [`crate::data_dirs::resolve_config_dir`]).
//!
//! Loading never fails: a missing or corrupt file gives the defaults (a wrong value in one field
//! only resets that field), and a failed save is only logged, so a broken config can never stop
//! Studio from starting. Saving writes a temporary file and renames it, like the colour palettes.

use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::theme;
pub use crate::theme::ThemeChoice;

/// The smallest interface size, in percent.
pub const SIZE_MIN: u16 = 90;
/// The largest interface size, in percent.
pub const SIZE_MAX: u16 = 150;
/// Settings moves the interface size in steps of this many percent.
pub const SIZE_STEP: u16 = 10;
/// The interface size when nothing is saved.
pub const SIZE_DEFAULT: u16 = 100;
/// Largest `settings.toml` that is read (64 KiB); a bigger file counts as corrupt.
pub const MAX_FILE_BYTES: u64 = theme::saved::MAX_FILE_BYTES;
/// The file name inside the config folder.
pub const FILE_NAME: &str = theme::saved::SETTINGS_FILE;

/// What the creator can change in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Colour theme.
    pub theme: ThemeChoice,
    /// Interface size in percent, [`SIZE_MIN`] to [`SIZE_MAX`].
    pub interface_size: u16,
    /// Turns the interface animations off or shortens them (#134).
    pub reduce_motion: bool,
    /// First-time hints in the editor (building-tools spec 14.1).
    pub show_tips: bool,
    /// Developer mode as chosen in Settings; `None` until changed there, which means the build
    /// channel's default (see [`Settings::dev_mode`]).
    pub developer_mode: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            interface_size: SIZE_DEFAULT,
            reduce_motion: false,
            show_tips: true,
            developer_mode: None,
        }
    }
}

/// `settings.toml` as written: every field optional, so older and partly wrong files still load.
#[derive(Default, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interface_size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reduce_motion: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_tips: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    developer_mode: Option<bool>,
}

impl Settings {
    /// An interface size brought into range.
    pub fn clamp_size(percent: i64) -> u16 {
        percent.clamp(i64::from(SIZE_MIN), i64::from(SIZE_MAX)) as u16
    }

    /// Developer mode for this build: the saved choice, else on for the `dev` channel and off
    /// for `stable`.
    pub fn dev_mode(&self) -> crate::devmode::DevMode {
        crate::devmode::DevMode::resolve(self.developer_mode, crate::devmode::build_channel())
    }

    /// The factor for egui's `zoom_factor`.
    pub fn zoom_factor(&self) -> f32 {
        f32::from(self.interface_size) / 100.0
    }

    /// Reads `text` as `settings.toml`; `None` when it is not valid TOML of the right shape.
    pub fn parse(text: &str) -> Option<Settings> {
        let file: SettingsFile = toml::from_str(text).ok()?;
        let defaults = Settings::default();
        Some(Settings {
            theme: file
                .theme
                .as_deref()
                .and_then(ThemeChoice::parse)
                .unwrap_or(defaults.theme),
            interface_size: file
                .interface_size
                .map_or(defaults.interface_size, Settings::clamp_size),
            reduce_motion: file.reduce_motion.unwrap_or(defaults.reduce_motion),
            show_tips: file.show_tips.unwrap_or(defaults.show_tips),
            developer_mode: file.developer_mode,
        })
    }

    /// The text of `settings.toml`.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(&SettingsFile {
            theme: Some(self.theme.as_str().to_owned()),
            interface_size: Some(i64::from(self.interface_size)),
            reduce_motion: Some(self.reduce_motion),
            show_tips: Some(self.show_tips),
            developer_mode: self.developer_mode,
        })
    }

    /// Reads `file`; a missing, unreadable or corrupt file gives the defaults.
    pub fn load(file: &Path) -> Settings {
        // A settings file is a few lines; refuse anything big before reading it.
        let small = fs::metadata(file).is_ok_and(|m| m.len() <= MAX_FILE_BYTES);
        let text = if small {
            fs::read_to_string(file).ok()
        } else {
            None
        };
        let Some(text) = text else {
            return Settings::default();
        };
        Settings::parse(&text).unwrap_or_else(|| {
            log::warn!("ignoring unreadable settings file");
            Settings::default()
        })
    }

    /// Writes `file` through a temporary file and a rename, creating the folder if needed.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        let text = self.to_toml().map_err(|e| e.to_string())?;
        if let Some(parent) = file.parent() {
            crate::data_dirs::create_owned(parent).map_err(|e| e.to_string())?;
        }
        crate::colour::palette::write_atomic(file, &text).map_err(|e| e.to_string())
    }
}

/// The settings plus the file they are saved to (none for an in-memory copy, used by captures
/// and tests so they never touch the real config folder).
#[derive(Clone, Debug, Default)]
pub struct SettingsStore {
    file: Option<PathBuf>,
    /// The current values; call [`SettingsStore::save`] after changing them.
    pub settings: Settings,
}

impl SettingsStore {
    /// Loads `<config dir>/settings.toml`.
    pub fn load_default() -> Self {
        Self::load(crate::recent::config_dir().map(|d| d.join(FILE_NAME)))
    }

    /// Loads `file` (or the defaults, unsaved, when `None`). Never fails.
    pub fn load(file: Option<PathBuf>) -> Self {
        let settings = file.as_deref().map(Settings::load).unwrap_or_default();
        Self { file, settings }
    }

    /// An unsaved store holding `settings`.
    pub fn in_memory(settings: Settings) -> Self {
        Self {
            file: None,
            settings,
        }
    }

    /// True when changes are saved to a file.
    pub fn is_persistent(&self) -> bool {
        self.file.is_some()
    }

    /// Writes the settings; failures are logged, never returned.
    pub fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Err(e) = self.settings.save(file) {
            log::warn!("could not save settings: {e}");
        }
    }
}

/// Puts `settings` into effect on `ctx`: reduced motion, the theme (resolved against the system
/// theme egui reports) and the interface size. Cheap enough to call when nothing changed.
pub fn apply_settings(ctx: &egui::Context, settings: &Settings) {
    theme::set_reduce_motion(settings.reduce_motion);
    let id = settings.theme.resolve(ctx.system_theme());
    if id != theme::active()
        || ctx.data(|d| d.get_temp::<Applied>(egui::Id::NULL)) != Some(Applied(*settings))
    {
        theme::apply(ctx, id);
        ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, Applied(*settings)));
    }
    if (ctx.zoom_factor() - settings.zoom_factor()).abs() > f32::EPSILON {
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min_size_for_zoom(
            ctx.zoom_factor(),
            settings.zoom_factor(),
        )));
        ctx.set_zoom_factor(settings.zoom_factor());
    }
}

/// The window's minimum size to send when the zoom goes from `old` to `new`. egui turns the size
/// into pixels with the zoom still in force (the new zoom starts with the next pass), so it is
/// scaled by `new / old`: the window then never gets smaller than the minimum in points.
pub fn min_size_for_zoom(old: f32, new: f32) -> egui::Vec2 {
    crate::splash::MAIN_MIN_SIZE * (new / old.max(f32::EPSILON))
}

/// The settings [`apply_settings`] last put into effect on a context.
#[derive(Clone, Copy, PartialEq)]
struct Applied(Settings);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeId;

    fn temp_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("cfg").join(FILE_NAME);
        (dir, file)
    }

    #[test]
    fn defaults_are_graphite_full_size_with_motion() {
        let s = Settings::default();
        assert_eq!(s.theme, ThemeChoice::Graphite);
        assert_eq!(s.interface_size, 100);
        assert!(!s.reduce_motion);
        assert_eq!(s.zoom_factor(), 1.0);
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let (_dir, file) = temp_file();
        for theme in ThemeChoice::ALL {
            let s = Settings {
                theme,
                interface_size: 120,
                reduce_motion: true,
                show_tips: false,
                developer_mode: Some(false),
            };
            s.save(&file).expect("saved");
            assert_eq!(Settings::load(&file), s);
        }
        let text = fs::read_to_string(&file).expect("file");
        assert!(text.contains("theme = \"system\""), "{text}");
        assert!(text.contains("interface_size = 120"), "{text}");
        assert!(text.contains("reduce_motion = true"), "{text}");
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let (_dir, file) = temp_file();
        Settings::default().save(&file).expect("saved");
        let names: Vec<_> = fs::read_dir(file.parent().expect("dir"))
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .filter(|n| n != crate::data_dirs::DATA_MARKER)
            .collect();
        assert_eq!(names, [FILE_NAME]);
    }

    #[test]
    fn a_file_over_64_kib_gives_the_defaults() {
        let (_dir, file) = temp_file();
        fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
        let padding = "# x
"
        .repeat(20_000);
        fs::write(
            &file,
            format!(
                "interface_size = 130
{padding}"
            ),
        )
        .expect("write");
        assert!(fs::metadata(&file).expect("meta").len() > MAX_FILE_BYTES);
        assert_eq!(Settings::load(&file), Settings::default());
        fs::write(
            &file,
            "interface_size = 130
",
        )
        .expect("write");
        assert_eq!(Settings::load(&file).interface_size, 130);
        // A folder where the file should be is a read error, not a panic.
        let dir_as_file = file.parent().expect("dir").join("sub");
        fs::create_dir_all(&dir_as_file).expect("mkdir");
        assert_eq!(Settings::load(&dir_as_file), Settings::default());
    }

    #[test]
    fn the_minimum_window_size_follows_the_zoom() {
        let min = crate::splash::MAIN_MIN_SIZE;
        assert_eq!(min_size_for_zoom(1.0, 1.0), min);
        assert_eq!(min_size_for_zoom(1.0, 1.5), min * 1.5);
        assert_eq!(min_size_for_zoom(1.5, 1.0), min / 1.5);
        assert!(min_size_for_zoom(0.0, 1.5).x.is_finite());
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let (_dir, file) = temp_file();
        assert_eq!(Settings::load(&file), Settings::default());
    }

    #[test]
    fn a_corrupt_file_gives_the_defaults() {
        let (_dir, file) = temp_file();
        fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
        for junk in [
            "theme = ",
            "[[[ not toml",
            "interface_size = \"big\"",
            "reduce_motion = 3",
            "\u{0}\u{1}\u{2}",
        ] {
            fs::write(&file, junk).expect("write");
            assert_eq!(Settings::load(&file), Settings::default(), "{junk:?}");
        }
        fs::write(&file, [0xff_u8, 0xfe, 0xfd]).expect("write");
        assert_eq!(Settings::load(&file), Settings::default());
    }

    #[test]
    fn an_unknown_theme_name_keeps_the_other_fields() {
        let s = Settings::parse("theme = \"neon\"\ninterface_size = 130\nreduce_motion = true\n")
            .expect("valid toml");
        assert_eq!(s.theme, ThemeChoice::Graphite);
        assert_eq!(s.interface_size, 130);
        assert!(s.reduce_motion);
    }

    #[test]
    fn out_of_range_sizes_are_clamped() {
        for (stored, want) in [
            (10, 90),
            (89, 90),
            (90, 90),
            (150, 150),
            (151, 150),
            (9000, 150),
            (-5, 90),
        ] {
            let s = Settings::parse(&format!("interface_size = {stored}")).expect("toml");
            assert_eq!(s.interface_size, want, "stored {stored}");
        }
    }

    #[test]
    fn theme_names_parse_in_any_case() {
        assert_eq!(ThemeChoice::parse("Graphite"), Some(ThemeChoice::Graphite));
        assert_eq!(ThemeChoice::parse(" LIGHT "), Some(ThemeChoice::Light));
        assert_eq!(ThemeChoice::parse("titanium"), Some(ThemeChoice::Titanium));
        assert_eq!(ThemeChoice::parse("System"), Some(ThemeChoice::System));
        assert_eq!(ThemeChoice::parse(""), None);
        assert_eq!(ThemeChoice::parse("aluminium"), None);
        for choice in ThemeChoice::ALL {
            assert_eq!(ThemeChoice::parse(choice.as_str()), Some(choice));
        }
    }

    #[test]
    fn system_follows_the_reported_theme() {
        assert_eq!(
            ThemeChoice::System.resolve(Some(egui::Theme::Light)),
            ThemeId::Light
        );
        assert_eq!(
            ThemeChoice::System.resolve(Some(egui::Theme::Dark)),
            ThemeId::Graphite
        );
        assert_eq!(ThemeChoice::System.resolve(None), ThemeId::Graphite);
        assert_eq!(
            ThemeChoice::Titanium.resolve(Some(egui::Theme::Light)),
            ThemeId::Titanium
        );
    }

    #[test]
    fn the_store_saves_only_when_it_has_a_file() {
        let (_dir, file) = temp_file();
        let mut store = SettingsStore::load(Some(file.clone()));
        assert!(store.is_persistent());
        store.settings.theme = ThemeChoice::Light;
        store.save();
        assert_eq!(
            SettingsStore::load(Some(file)).settings.theme,
            ThemeChoice::Light
        );

        let mut memory = SettingsStore::in_memory(Settings::default());
        memory.settings.reduce_motion = true;
        memory.save();
        assert!(!memory.is_persistent());
    }

    #[test]
    fn applying_settings_switches_theme_zoom_and_motion() {
        let ctx = egui::Context::default();
        let s = Settings {
            theme: ThemeChoice::Light,
            interface_size: 130,
            reduce_motion: true,
            show_tips: true,
            developer_mode: None,
        };
        apply_settings(&ctx, &s);
        assert_eq!(theme::active(), ThemeId::Light);
        assert!(theme::reduce_motion());
        // The zoom becomes active at the end of the pass.
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        assert_eq!(ctx.zoom_factor(), 1.3);
        assert_eq!(
            ctx.global_style().visuals.panel_fill,
            theme::palette_of(ThemeId::Light).bg_base
        );
        assert_eq!(ctx.global_style().animation_time, 0.0);
        apply_settings(&ctx, &Settings::default());
        assert_eq!(theme::active(), ThemeId::Graphite);
        assert!(!theme::reduce_motion());
        assert_eq!(
            ctx.global_style().visuals.panel_fill,
            theme::palette_of(ThemeId::Graphite).bg_base
        );
    }
}
