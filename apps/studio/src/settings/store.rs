//! `settings.toml` on disk (ADR 0009, Settings): read once at startup, written only when something changed, and
//! never lost.
//!
//! - A missing file is the defaults; nothing is written until a setting changes.
//! - A write goes to `settings.toml.tmp` beside the file and is renamed over it, so a crash mid-write never leaves
//!   half a file.
//! - **A file that is not TOML is never overwritten:** it is renamed to `settings.toml.bak` (or `.bak.2`, ... when
//!   that is taken), Studio starts on the defaults, and the [`SettingsStore::take_notice`] says so once. If it cannot
//!   be moved aside, or cannot be read at all, nothing is written this run.

use std::io;
use std::path::{Path, PathBuf};

use metallic_engine::metallic_platform::{AppId, PlatformError, UserPaths};
use toml::Table;

use super::StudioSettings;
use crate::code::settings::{CODE_EDITOR_FILE, CodeEditor};

/// The first lines of `code_editor.toml` (#529).
const CODE_HEADER: &str = "# Metallic Studio's script editor settings (Settings, Code Editor). Studio rewrites this file when one \
                           changes; keys it does not know are kept.\n\n";

/// The settings file's name in Studio's config folder.
pub const SETTINGS_FILE: &str = "settings.toml";

/// The first line of every file written: the file is read and rewritten whole, so a comment of one's own is lost.
const HEADER: &str = "# Metallic Studio settings. Studio rewrites this file when a setting changes; keys it does not \
                      know are kept.\n";

/// Where `app`'s settings live for the current user: `settings.toml` in its config folder.
pub fn settings_path(app: &AppId) -> Result<PathBuf, PlatformError> {
    Ok(UserPaths::for_app(app)?.config_dir().join(SETTINGS_FILE))
}

/// The settings, the file they came from, and what was last saved there.
#[derive(Debug, Default)]
pub struct SettingsStore {
    /// The file; `None` keeps the settings in memory only.
    path: Option<PathBuf>,
    /// Whether the file may be written: not when it could not be read, or a damaged one could not be moved aside.
    writable: bool,
    /// The file as last read or written: the keys Studio does not know are kept from it.
    document: Table,
    /// The settings as last read or written.
    saved: StudioSettings,
    /// The settings now.
    settings: StudioSettings,
    /// What the user should be told about the file, until it is taken.
    notice: Option<String>,
    /// The script editor's file (`code_editor.toml`, #529) as last read or written: its other keys are kept.
    code_document: Table,
    /// Whether the script editor's file may be written: not when it could not be read.
    code_writable: bool,
}

impl SettingsStore {
    /// Settings that live in memory only: the defaults, never saved.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Reads the settings at `path`. Never fails: see the module's documentation for what happens when the file is
    /// missing or damaged.
    pub fn load(path: PathBuf) -> Self {
        let mut store = Self {
            path: Some(path.clone()),
            writable: true,
            code_writable: true,
            ..Self::default()
        };
        store.load_code(&path.with_file_name(CODE_EDITOR_FILE));
        let text = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return store,
            Err(error) => {
                log::warn!("settings: could not read {}: {error}", path.display());
                store.writable = false;
                store.notice = Some(format!(
                    "Studio could not read its settings file ({error}), so it started with the default settings \
                     and will not save changes this time. The file is {}.",
                    path.display()
                ));
                return store;
            }
        };
        let parsed = String::from_utf8(text)
            .map_err(|_| "it is not UTF-8 text".to_owned())
            .and_then(|text| text.parse::<Table>().map_err(|error| error.message().to_owned()));
        match parsed {
            Ok(document) => {
                let code = std::mem::take(&mut store.settings.code);
                store.settings = StudioSettings {
                    code,
                    ..StudioSettings::from_document(&document)
                };
                store.saved = store.settings.clone();
                store.document = document;
            }
            Err(reason) => store.set_aside(&path, &reason),
        }
        store
    }

    /// The settings saved at `path`, read without changing anything: the defaults when there is no file or it cannot
    /// be read. For what shows before startup reads them properly, such as the splash's accent (#319).
    pub fn peek(path: &Path) -> StudioSettings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.parse::<Table>().ok())
            .map_or_else(StudioSettings::default, |document| {
                StudioSettings::from_document(&document)
            })
    }

    /// Reads the script editor's settings at `path` (#529): the defaults when there is no file; a file that cannot be
    /// read or parsed is logged and left alone, and not written over.
    fn load_code(&mut self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Err(error) => {
                log::warn!("settings: could not read {}: {error}", path.display());
                self.code_writable = false;
                return;
            }
        };
        match text.parse::<Table>() {
            Ok(document) => {
                self.settings.code = CodeEditor::from_document(&document);
                self.saved.code = self.settings.code.clone();
                self.code_document = document;
            }
            Err(error) => {
                log::warn!(
                    "settings: {} is not valid ({}); the script editor uses its defaults and will not write over it",
                    path.display(),
                    error.message()
                );
                self.code_writable = false;
            }
        }
    }

    /// Moves a damaged file aside, so writing the settings never overwrites it.
    fn set_aside(&mut self, path: &Path, reason: &str) {
        let Some(backup) = backup_path(path) else {
            self.writable = false;
            return;
        };
        let name = backup.file_name().map_or_else(
            || backup.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        match std::fs::rename(path, &backup) {
            Ok(()) => {
                log::warn!(
                    "settings: {} is damaged ({reason}); kept as {}, starting with the defaults",
                    path.display(),
                    backup.display()
                );
                self.notice = Some(format!(
                    "Studio's settings file was damaged ({reason}), so Studio started with the default settings. \
                     The old file was kept as {name}, next to the new one."
                ));
            }
            Err(error) => {
                log::warn!(
                    "settings: {} is damaged ({reason}) and could not be moved aside: {error}",
                    path.display()
                );
                self.writable = false;
                self.notice = Some(format!(
                    "Studio's settings file was damaged ({reason}) and could not be kept aside ({error}), so Studio \
                     started with the default settings and will not save changes this time. The file is {}.",
                    path.display()
                ));
            }
        }
    }

    /// The settings now.
    pub fn settings(&self) -> &StudioSettings {
        &self.settings
    }

    /// The settings, to change; [`SettingsStore::save_if_changed`] writes the change.
    pub fn settings_mut(&mut self) -> &mut StudioSettings {
        &mut self.settings
    }

    /// The file, unless the settings live in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// What the user should be told about the settings file (it was damaged and moved aside, or unreadable); only
    /// the first call gets it.
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Whether the settings differ from what was last read or saved.
    pub fn is_changed(&self) -> bool {
        self.settings != self.saved
    }

    /// Writes the settings if they changed since they were read or last saved. Returns whether it wrote.
    ///
    /// A failed write is not retried until the settings change again, so a full disk is reported once per change
    /// rather than every frame.
    pub fn save_if_changed(&mut self) -> io::Result<bool> {
        if !self.is_changed() {
            return Ok(false);
        }
        self.settings.clamp();
        // Each file is written only when its own settings changed (#529).
        let code_changed = self.settings.code != self.saved.code;
        let main_changed = StudioSettings {
            code: self.saved.code.clone(),
            ..self.settings.clone()
        } != self.saved;
        self.saved = self.settings.clone();
        let Some(path) = self.path.clone() else {
            return Ok(false);
        };
        let mut wrote = false;
        if code_changed && self.code_writable {
            let document = self.settings.code.to_document(&self.code_document);
            write_atomically(
                &path.with_file_name(CODE_EDITOR_FILE),
                &format!("{CODE_HEADER}{document}"),
            )?;
            self.code_document = document;
            wrote = true;
        }
        if main_changed && self.writable {
            let document = self.settings.to_document(&self.document);
            write_atomically(&path, &format!("{HEADER}{document}"))?;
            self.document = document;
            wrote = true;
        }
        Ok(wrote)
    }
}

/// Writes `text` to `path` through a temporary file beside it, making its folder first.
pub(crate) fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

/// The first of `<file>.bak`, `<file>.bak.2`, ... that does not exist, so an older backup is never replaced.
pub(crate) fn backup_path(path: &Path) -> Option<PathBuf> {
    (1..=99)
        .map(|n| backup_name(path, n))
        .find(|candidate| !candidate.exists())
}

fn backup_name(path: &Path, n: u32) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    if n > 1 {
        name.push(format!(".{n}"));
    }
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeMode;
    use crate::theme::tokens::Accent;

    /// An empty folder of its own for `test`, and the settings file's path in it.
    fn file(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-studio-settings-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("config").join(SETTINGS_FILE)
    }

    /// #529: the script editor's settings are in `code_editor.toml`, not `settings.toml`, and each file is written only
    /// when its own settings change.
    #[test]
    fn the_code_editor_s_settings_have_a_file_of_their_own() {
        let path = file("code-editor");
        let code = path.with_file_name(CODE_EDITOR_FILE);
        let mut store = SettingsStore::load(path.clone());
        store.settings_mut().code.size = 16.0;
        store.settings_mut().code.insert_spaces = true;
        assert!(store.save_if_changed().unwrap());
        assert!(code.is_file());
        assert!(!path.exists(), "settings.toml did not change");
        let text = std::fs::read_to_string(&code).unwrap();
        assert!(
            text.contains("size = 16.0") && text.contains("indent = \"spaces\""),
            "{text}"
        );

        store.settings_mut().appearance.accent = Accent::Blue;
        std::fs::remove_file(&code).unwrap();
        assert!(store.save_if_changed().unwrap());
        assert!(path.is_file());
        assert!(!code.exists(), "the script editor's did not change");
        assert!(!std::fs::read_to_string(&path).unwrap().contains("indent"));

        // Read back, both.
        store.settings_mut().code.tab_width = 2;
        assert!(store.save_if_changed().unwrap());
        let read = SettingsStore::load(path);
        assert_eq!(read.settings().code.size, 16.0);
        assert_eq!(read.settings().code.tab_width, 2);
        assert_eq!(read.settings().appearance.accent, Accent::Blue);
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_is_not_written_until_a_change() {
        let path = file("missing");
        let mut store = SettingsStore::load(path.clone());
        assert_eq!(store.settings(), &StudioSettings::default());
        assert!(store.take_notice().is_none());
        assert!(!store.save_if_changed().unwrap());
        assert!(!path.exists(), "nothing changed, nothing written");

        store.settings_mut().appearance.accent = Accent::Blue;
        assert!(store.save_if_changed().unwrap());
        assert!(path.is_file());
        assert!(
            !path.with_extension("toml.tmp").exists(),
            "the temporary file is renamed over"
        );
        assert!(!store.save_if_changed().unwrap(), "written once");
    }

    #[test]
    fn saved_settings_load_back() {
        let path = file("round-trip");
        let mut store = SettingsStore::load(path.clone());
        let settings = store.settings_mut();
        settings.appearance.mode = ThemeMode::Light;
        settings.appearance.ui_scale = 1.1;
        settings.general.skip_sign_in = true;
        settings.rendering.vsync = Some(false);
        let wanted = settings.clone();
        store.save_if_changed().unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Metallic Studio settings."), "{text}");
        assert!(text.contains("ui_scale = 1.1\n"), "{text}");
        let again = SettingsStore::load(path);
        assert_eq!(again.settings(), &wanted);
        assert!(!again.is_changed());
    }

    #[test]
    fn a_file_out_of_range_is_clamped_but_not_rewritten() {
        let path = file("clamped");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = "[appearance]\nui_scale = 40\n";
        std::fs::write(&path, text).unwrap();
        let mut store = SettingsStore::load(path.clone());
        assert_eq!(store.settings().appearance.ui_scale, crate::settings::ui_scale::MAX);
        assert!(!store.save_if_changed().unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn a_corrupt_file_is_kept_as_bak_and_never_overwritten() {
        let path = file("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let damaged = "[appearance\nmode = \"light";
        std::fs::write(&path, damaged).unwrap();
        // An older backup is not replaced either.
        let older = backup_name(&path, 1);
        std::fs::write(&older, "older").unwrap();

        let mut store = SettingsStore::load(path.clone());
        assert_eq!(store.settings(), &StudioSettings::default());
        let notice = store.take_notice().expect("the user is told");
        assert!(notice.contains("settings.toml.bak.2"), "{notice}");
        assert!(store.take_notice().is_none(), "told once");
        let backup = backup_name(&path, 2);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), damaged);
        assert_eq!(std::fs::read_to_string(&older).unwrap(), "older");
        assert!(!path.exists());

        // The next save writes a fresh file and leaves the backups alone.
        store.settings_mut().appearance.high_contrast = true;
        assert!(store.save_if_changed().unwrap());
        assert!(SettingsStore::load(path.clone()).settings().appearance.high_contrast);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), damaged);
    }

    #[test]
    fn a_file_that_is_not_text_is_kept_as_bak() {
        let path = file("binary");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [0xFF, 0xFE, 0x00, 0x80]).unwrap();
        let mut store = SettingsStore::load(path.clone());
        assert!(store.take_notice().unwrap().contains("UTF-8"));
        assert_eq!(std::fs::read(backup_name(&path, 1)).unwrap(), [0xFF, 0xFE, 0x00, 0x80]);
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let path = file("unknown");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "[keybinds]\n\"file.save\" = [\"Primary+S\"]\n[later]\nthing = 1\n",
        )
        .unwrap();
        let mut store = SettingsStore::load(path.clone());
        store.settings_mut().appearance.mode = ThemeMode::System;
        assert!(store.save_if_changed().unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Primary+S") && text.contains("[later]"), "{text}");
        assert!(text.contains("mode = \"system\""), "{text}");
    }

    #[test]
    fn in_memory_settings_are_never_written() {
        let mut store = SettingsStore::in_memory();
        store.settings_mut().general.skip_sign_in = true;
        assert!(!store.save_if_changed().unwrap());
        assert!(!store.is_changed());
        assert!(store.path().is_none());
    }

    #[test]
    fn studio_keeps_its_settings_in_its_config_folder() {
        let app = AppId::parse(crate::STUDIO_APP_ID).unwrap();
        if let Ok(path) = settings_path(&app) {
            assert_eq!(path.file_name().unwrap(), SETTINGS_FILE);
            assert!(path.to_string_lossy().to_lowercase().contains("studio"));
        }
    }

    #[test]
    fn peeking_reads_the_file_without_touching_it() {
        let dir = std::env::temp_dir().join(format!("metallic-peek-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SETTINGS_FILE);
        assert_eq!(
            SettingsStore::peek(&path),
            StudioSettings::default(),
            "no file: the defaults"
        );
        std::fs::write(
            &path,
            "[appearance]
accent = \"rose\"
",
        )
        .unwrap();
        assert_eq!(SettingsStore::peek(&path).appearance.accent, Accent::Rose);
        // A damaged file gives the defaults and stays where it is, for startup to deal with.
        std::fs::write(&path, "not = [toml").unwrap();
        assert_eq!(SettingsStore::peek(&path), StudioSettings::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not = [toml");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
