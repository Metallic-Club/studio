//! Feature flags: named on/off switches for unfinished features, read from `flags.toml` in the
//! config folder (`%APPDATA%\MetallicStudio\flags.toml`) and edited in the Developer > Feature
//! flags panel.
//!
//! The file is one `"name" = true|false` line per flag. Loading never fails: a missing, too big or
//! unreadable file gives no flags, and a line with a bad name or a value that is not a boolean
//! is skipped. Code asks [`FeatureFlags::is_on`]; a flag that is not in the file is off.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Layout, RichText};

use crate::theme::{self, fonts};
use crate::widgets::Button;

/// The file name inside the config folder.
pub const FILE_NAME: &str = "flags.toml";
/// Largest `flags.toml` that is read (64 KiB).
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Longest flag name.
pub const MAX_NAME: usize = 64;

/// The flags and the file they are saved to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeatureFlags {
    flags: BTreeMap<String, bool>,
    file: Option<PathBuf>,
}

/// True for a usable flag name: 1 to [`MAX_NAME`] characters of `a-z`, `0-9`, `_`, `-`, `.`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b))
}

impl FeatureFlags {
    /// No flags, saved nowhere.
    pub fn new() -> FeatureFlags {
        FeatureFlags::default()
    }

    /// Loads `<config dir>/flags.toml` (no flags, unsaved, when there is no config folder).
    pub fn load_default() -> FeatureFlags {
        match crate::recent::config_dir() {
            Some(dir) => FeatureFlags::load(&dir.join(FILE_NAME)),
            None => FeatureFlags::new(),
        }
    }

    /// Loads `file`; [`FeatureFlags::save`] writes back to it.
    pub fn load(file: &Path) -> FeatureFlags {
        let small = fs::metadata(file).is_ok_and(|m| m.len() <= MAX_FILE_BYTES);
        let text = if small {
            fs::read_to_string(file).ok()
        } else {
            None
        };
        let mut flags = text.as_deref().map(FeatureFlags::parse).unwrap_or_default();
        flags.file = Some(file.to_path_buf());
        flags
    }

    /// Reads `text` as `flags.toml`, skipping bad lines.
    pub fn parse(text: &str) -> FeatureFlags {
        let mut flags = FeatureFlags::new();
        match toml::from_str::<toml::Table>(text) {
            Ok(table) => {
                for (name, value) in table {
                    match value.as_bool() {
                        Some(on) if valid_name(&name) => {
                            flags.flags.insert(name, on);
                        }
                        _ => log::warn!("flags.toml: skipping {name}"),
                    }
                }
            }
            Err(e) => log::warn!("ignoring unreadable flags.toml: {}", e.message()),
        }
        flags
    }

    /// The text of `flags.toml`.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        for (name, on) in &self.flags {
            // Quoted, so a dot is part of the name rather than a table path.
            out.push_str(&format!("\"{name}\" = {on}\n"));
        }
        out
    }

    /// Writes the file it was loaded from (nothing for an unsaved set).
    pub fn save(&self) -> Result<(), String> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(parent) = file.parent() {
            crate::data_dirs::create_owned(parent).map_err(|e| e.to_string())?;
        }
        crate::colour::palette::write_atomic(file, &self.to_toml()).map_err(|e| e.to_string())
    }

    /// True when flag `name` is set and on.
    pub fn is_on(&self, name: &str) -> bool {
        self.flags.get(name).copied().unwrap_or(false)
    }

    /// Sets flag `name`; false (and no change) for an invalid name.
    pub fn set(&mut self, name: &str, on: bool) -> bool {
        if !valid_name(name) {
            return false;
        }
        self.flags.insert(name.to_owned(), on);
        true
    }

    /// Forgets flag `name` (it reads as off).
    pub fn remove(&mut self, name: &str) {
        self.flags.remove(name);
    }

    /// Every flag, by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, bool)> + '_ {
        self.flags.iter().map(|(n, on)| (n.as_str(), *on))
    }

    /// Number of flags.
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    /// True when there are no flags.
    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }
}

/// The Feature flags panel: toggle, add and remove flags. `new_name` is the add field's text.
/// Saves after every change. Returns false when the creator closed it.
pub fn panel(ctx: &egui::Context, flags: &mut FeatureFlags, new_name: &mut String) -> bool {
    let mut open = true;
    let mut changed = false;
    egui::Window::new(super::overlay::window_title("Feature flags"))
        .id(egui::Id::unique("dev-flags"))
        .open(&mut open)
        .default_width(320.0)
        .default_pos([ctx.content_rect().right() - 760.0, 190.0])
        .order(egui::Order::Foreground)
        .resizable(false)
        .frame(super::overlay::window_frame())
        .show(ctx, |ui| {
            ui.label(
                RichText::new("Saved in flags.toml in the Studio settings folder.")
                    .font(fonts::small())
                    .color(theme::text_2()),
            );
            ui.add_space(8.0);
            let names: Vec<(String, bool)> =
                flags.iter().map(|(n, on)| (n.to_owned(), on)).collect();
            if names.is_empty() {
                ui.label(RichText::new("No flags yet.").color(theme::text_3()));
            }
            for (name, mut on) in names {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, name.as_str()).changed() {
                        flags.set(&name, on);
                        changed = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let alt = format!("Remove {name}");
                        if ui
                            .add(
                                Button::ghost("")
                                    .icon(crate::theme::icons::X)
                                    .alt(&alt)
                                    .compact(),
                            )
                            .clicked()
                        {
                            flags.remove(&name);
                            changed = true;
                        }
                    });
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(new_name)
                        .hint_text("new_flag_name")
                        .desired_width(200.0),
                );
                let name = new_name.trim().to_owned();
                let ok = valid_name(&name);
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let add = ui
                    .add_enabled(ok, Button::secondary("Add").compact())
                    .clicked();
                if ok && (add || enter) {
                    flags.set(&name, true);
                    new_name.clear();
                    changed = true;
                }
            });
            if !new_name.trim().is_empty() && !valid_name(new_name.trim()) {
                ui.label(
                    RichText::new("Use a-z, 0-9, _, - and . only.")
                        .font(fonts::small())
                        .color(theme::warning()),
                );
            }
        });
    if changed && let Err(e) = flags.save() {
        log::warn!("could not save flags.toml: {e}");
    }
    open
}
