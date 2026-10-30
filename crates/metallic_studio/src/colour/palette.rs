//! Colour lists that outlive the popover: recent colours (per user) and saved palettes (per
//! project, one RON file each in `palettes/<name>.palette.ron`).
//!
//! As with the recent games list, loading never fails: unreadable files are skipped with a log
//! line. Saving returns an error so the popover can show it.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::maths::Rgb8;
use crate::StudioError;

/// How many recent colours are remembered.
pub const MAX_RECENT: usize = 12;
/// Longest palette name.
pub const MAX_NAME_LEN: usize = 48;
/// Most colours in one palette.
pub const MAX_PALETTE_LEN: usize = 256;
/// File name suffix of a saved palette.
pub const PALETTE_SUFFIX: &str = ".palette.ron";

#[derive(Default, Serialize, Deserialize)]
struct RecentFile {
    #[serde(default)]
    colours: Vec<Rgb8>,
}

/// The last committed colours, newest first, shared by every project of one user.
#[derive(Debug, Clone, Default)]
pub struct RecentColours {
    file: Option<PathBuf>,
    items: Vec<Rgb8>,
}

impl RecentColours {
    /// Loads `<config dir>/recent-colours.toml`.
    pub fn load_default() -> Self {
        Self::load(crate::recent::config_dir().map(|d| d.join("recent-colours.toml")))
    }

    /// Loads from `file`, or an in-memory list when `None`. Never fails.
    pub fn load(file: Option<PathBuf>) -> Self {
        let mut items = file
            .as_deref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|text| match toml::from_str::<RecentFile>(&text) {
                Ok(parsed) => Some(parsed.colours),
                Err(e) => {
                    log::warn!("ignoring unreadable recent colours file: {}", e.message());
                    None
                }
            })
            .unwrap_or_default();
        dedup_keep_first(&mut items);
        items.truncate(MAX_RECENT);
        Self { file, items }
    }

    /// The colours, newest first.
    pub fn items(&self) -> &[Rgb8] {
        &self.items
    }

    /// Moves `c` to the front (adding it if new), keeps 12, and saves. A failed save is logged.
    pub fn push(&mut self, c: Rgb8) {
        self.items.retain(|&x| x != c);
        self.items.insert(0, c);
        self.items.truncate(MAX_RECENT);
        self.save();
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let body = RecentFile {
            colours: self.items.clone(),
        };
        let result = toml::to_string(&body)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                if let Some(parent) = file.parent() {
                    crate::data_dirs::create_owned(parent).map_err(|e| e.to_string())?;
                }
                write_atomic(file, &text).map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            log::warn!("could not save recent colours: {e}");
        }
    }
}

fn dedup_keep_first(items: &mut Vec<Rgb8>) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|c| seen.insert(*c));
}

/// One saved palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    /// Display name, also the file name.
    pub name: String,
    /// The swatches, in the creator's order.
    pub colours: Vec<Rgb8>,
    /// True for palettes the project cannot edit (a Theme's role colours).
    pub read_only: bool,
}

/// What a `.palette.ron` file holds. The name comes from the file name, so renaming a file in
/// Explorer renames the palette.
#[derive(Serialize, Deserialize)]
struct PaletteFile {
    version: u32,
    colours: Vec<Rgb8>,
}

const PALETTE_VERSION: u32 = 1;

/// Checks a palette name: 1 to 48 letters, digits, spaces, `-` or `_`, not starting or ending
/// with a space. Rejects anything that could leave the palettes folder.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Give the palette a name.".into());
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(format!("Use at most {MAX_NAME_LEN} characters."));
    }
    if name.trim() != name {
        return Err("Remove the spaces at the start or end.".into());
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
    {
        return Err("Use letters, digits, spaces, - or _.".into());
    }
    // Windows device names cannot be file names, even with an extension.
    let upper = name.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        return Err("That name is reserved by Windows; choose another.".into());
    }
    Ok(())
}

/// The saved palettes of one project, backed by its `palettes/` folder (or memory only).
#[derive(Debug, Clone, Default)]
pub struct PaletteStore {
    dir: Option<PathBuf>,
    palettes: Vec<Palette>,
}

impl PaletteStore {
    /// An empty store that never touches the disk (demo and screenshots).
    pub fn in_memory(palettes: Vec<Palette>) -> Self {
        Self {
            dir: None,
            palettes,
        }
    }

    /// Loads every `*.palette.ron` in `<project>/palettes`, sorted by name. Never fails.
    pub fn load(project_dir: &Path) -> Self {
        let dir = project_dir.join("palettes");
        let mut palettes = Vec::new();
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(PALETTE_SUFFIX))
                else {
                    continue;
                };
                if validate_name(name).is_err() {
                    log::warn!("skipping palette with an unusable name: {name}");
                    continue;
                }
                match fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| ron::from_str::<PaletteFile>(&t).map_err(|e| e.to_string()))
                {
                    Ok(file) => {
                        let mut colours = file.colours;
                        colours.truncate(MAX_PALETTE_LEN);
                        palettes.push(Palette {
                            name: name.to_owned(),
                            colours,
                            read_only: false,
                        });
                    }
                    Err(e) => log::warn!("skipping unreadable palette {name}: {e}"),
                }
            }
        }
        palettes.sort_by_key(|p| p.name.to_lowercase());
        Self {
            dir: Some(dir),
            palettes,
        }
    }

    /// Adds a read-only palette (a Theme's role colours) at the front. It is never written.
    pub fn add_read_only(&mut self, name: &str, colours: Vec<Rgb8>) {
        self.palettes.insert(
            0,
            Palette {
                name: name.to_owned(),
                colours,
                read_only: true,
            },
        );
    }

    /// Every palette.
    pub fn palettes(&self) -> &[Palette] {
        &self.palettes
    }

    /// The palette at `index`.
    pub fn get(&self, index: usize) -> Option<&Palette> {
        self.palettes.get(index)
    }

    /// The file a palette is saved in, if this store is on disk.
    pub fn file_of(&self, name: &str) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|d| d.join(format!("{name}{PALETTE_SUFFIX}")))
    }

    fn name_taken(&self, name: &str, except: Option<usize>) -> bool {
        self.palettes
            .iter()
            .enumerate()
            .any(|(i, p)| Some(i) != except && p.name.eq_ignore_ascii_case(name))
    }

    /// Creates an empty palette and saves it. Returns its index.
    pub fn create(&mut self, name: &str) -> Result<usize, StudioError> {
        validate_name(name).map_err(StudioError::Palette)?;
        if self.name_taken(name, None) {
            return Err(StudioError::Palette(format!(
                "A palette named \"{name}\" already exists."
            )));
        }
        self.palettes.push(Palette {
            name: name.to_owned(),
            colours: Vec::new(),
            read_only: false,
        });
        let index = self.palettes.len() - 1;
        self.save(index)?;
        Ok(index)
    }

    /// A free name like "Palette 2".
    pub fn next_free_name(&self) -> String {
        (1..)
            .map(|n| format!("Palette {n}"))
            .find(|n| !self.name_taken(n, None))
            .unwrap_or_else(|| "Palette".into())
    }

    fn editable(&mut self, index: usize) -> Result<&mut Palette, StudioError> {
        match self.palettes.get_mut(index) {
            Some(p) if p.read_only => Err(StudioError::Palette(format!(
                "\"{}\" comes from the Theme and cannot be edited.",
                p.name
            ))),
            Some(p) => Ok(p),
            None => Err(StudioError::Palette(
                "That palette no longer exists.".into(),
            )),
        }
    }

    /// Appends `c` to a palette and saves it.
    pub fn add_colour(&mut self, index: usize, c: Rgb8) -> Result<(), StudioError> {
        let p = self.editable(index)?;
        if p.colours.len() >= MAX_PALETTE_LEN {
            return Err(StudioError::Palette(format!(
                "A palette holds at most {MAX_PALETTE_LEN} colours."
            )));
        }
        p.colours.push(c);
        self.save(index)
    }

    /// Removes the swatch at `at` and saves.
    pub fn remove_colour(&mut self, index: usize, at: usize) -> Result<(), StudioError> {
        let p = self.editable(index)?;
        if at < p.colours.len() {
            p.colours.remove(at);
        }
        self.save(index)
    }

    /// Moves the swatch at `from` to position `to` and saves.
    pub fn move_colour(&mut self, index: usize, from: usize, to: usize) -> Result<(), StudioError> {
        let p = self.editable(index)?;
        if from >= p.colours.len() {
            return Ok(());
        }
        let c = p.colours.remove(from);
        let to = to.min(p.colours.len());
        p.colours.insert(to, c);
        self.save(index)
    }

    /// Renames a palette and its file.
    pub fn rename(&mut self, index: usize, new_name: &str) -> Result<(), StudioError> {
        validate_name(new_name).map_err(StudioError::Palette)?;
        if self.name_taken(new_name, Some(index)) {
            return Err(StudioError::Palette(format!(
                "A palette named \"{new_name}\" already exists."
            )));
        }
        let old = self.editable(index)?.name.clone();
        if let (Some(from), Some(to)) = (self.file_of(&old), self.file_of(new_name))
            && from.exists()
        {
            fs::rename(&from, &to).map_err(|e| StudioError::io(&from, e))?;
        }
        self.editable(index)?.name = new_name.to_owned();
        self.save(index)
    }

    /// Deletes a palette and its file.
    pub fn delete(&mut self, index: usize) -> Result<(), StudioError> {
        let name = self.editable(index)?.name.clone();
        if let Some(file) = self.file_of(&name)
            && file.exists()
        {
            fs::remove_file(&file).map_err(|e| StudioError::io(&file, e))?;
        }
        self.palettes.remove(index);
        Ok(())
    }

    fn save(&self, index: usize) -> Result<(), StudioError> {
        let Some(p) = self.palettes.get(index) else {
            return Ok(());
        };
        let Some(file) = self.file_of(&p.name) else {
            return Ok(());
        };
        let body = PaletteFile {
            version: PALETTE_VERSION,
            colours: p.colours.clone(),
        };
        let text = ron::ser::to_string_pretty(&body, ron::ser::PrettyConfig::default())
            .map_err(|e| StudioError::Palette(e.to_string()))?;
        write_atomic(&file, &(text + "\n")).map_err(|e| StudioError::io(&file, e))
    }
}

/// Writes through a temporary file and a rename, so a crash never leaves half a file.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLUE: Rgb8 = Rgb8::new(82, 128, 204);
    const SAND: Rgb8 = Rgb8::new(0xD9, 0xC8, 0xA9);

    #[test]
    fn recent_keeps_twelve_newest_first_without_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recent-colours.toml");
        let mut recent = RecentColours::load(Some(file.clone()));
        for i in 0..20u8 {
            recent.push(Rgb8::new(i, i, i));
        }
        recent.push(Rgb8::new(15, 15, 15));
        assert_eq!(recent.items().len(), MAX_RECENT);
        assert_eq!(recent.items()[0], Rgb8::new(15, 15, 15));
        assert_eq!(recent.items()[1], Rgb8::new(19, 19, 19));
        assert_eq!(recent.items().iter().filter(|c| c.r == 15).count(), 1);

        let again = RecentColours::load(Some(file));
        assert_eq!(again.items(), recent.items());
    }

    #[test]
    fn recent_ignores_a_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("recent-colours.toml");
        fs::write(&file, "colours = [\"#zzzzzz\"").unwrap();
        assert!(RecentColours::load(Some(file)).items().is_empty());
    }

    #[test]
    fn palettes_round_trip_through_ron_files() {
        let project = tempfile::tempdir().unwrap();
        let mut store = PaletteStore::load(project.path());
        assert!(store.palettes().is_empty());
        let castle = store.create("Castle").unwrap();
        store.add_colour(castle, BLUE).unwrap();
        store.add_colour(castle, SAND).unwrap();
        store.create("Arcade_2").unwrap();

        let file = project.path().join("palettes").join("Castle.palette.ron");
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"#5280CC\""), "{text}");
        assert!(text.contains("version: 1"), "{text}");

        let loaded = PaletteStore::load(project.path());
        let names: Vec<&str> = loaded.palettes().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Arcade_2", "Castle"]);
        assert_eq!(loaded.get(1).unwrap().colours, [BLUE, SAND]);
        assert!(
            !project
                .path()
                .join("palettes")
                .join("Castle.palette.tmp")
                .exists()
        );
    }

    #[test]
    fn palette_edits_are_saved() {
        let project = tempfile::tempdir().unwrap();
        let mut store = PaletteStore::load(project.path());
        let i = store.create("Castle").unwrap();
        for c in [BLUE, SAND, Rgb8::new(1, 2, 3)] {
            store.add_colour(i, c).unwrap();
        }
        store.move_colour(i, 2, 0).unwrap();
        store.remove_colour(i, 1).unwrap();
        store.rename(i, "Keep").unwrap();
        let loaded = PaletteStore::load(project.path());
        assert_eq!(loaded.palettes().len(), 1);
        assert_eq!(loaded.get(0).unwrap().name, "Keep");
        assert_eq!(loaded.get(0).unwrap().colours, [Rgb8::new(1, 2, 3), SAND]);
        assert!(!project.path().join("palettes/Castle.palette.ron").exists());

        let mut store = loaded;
        store.delete(0).unwrap();
        assert!(PaletteStore::load(project.path()).palettes().is_empty());
    }

    #[test]
    fn palette_names_cannot_escape_the_folder_or_collide() {
        for bad in [
            "",
            " lead",
            "trail ",
            "../up",
            "a/b",
            "a\\b",
            "con:",
            "x.y",
            "nul",
            "COM1",
            &"n".repeat(49),
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
        assert!(validate_name("Castle walls-2_b").is_ok());
        let mut store = PaletteStore::in_memory(Vec::new());
        store.create("Castle").unwrap();
        assert!(store.create("castle").is_err());
        assert_eq!(store.next_free_name(), "Palette 1");
    }

    #[test]
    fn unreadable_palette_files_are_skipped() {
        let project = tempfile::tempdir().unwrap();
        let dir = project.path().join("palettes");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("Broken.palette.ron"),
            "(version: 1, colours: [\"nope\"])",
        )
        .unwrap();
        fs::write(
            dir.join("Good.palette.ron"),
            "(version: 1, colours: [\"teal\", \"#fff\"])",
        )
        .unwrap();
        fs::write(dir.join("notes.txt"), "not a palette").unwrap();
        let store = PaletteStore::load(project.path());
        assert_eq!(store.palettes().len(), 1);
        assert_eq!(
            store.get(0).unwrap().colours,
            [Rgb8::new(0, 128, 128), Rgb8::new(255, 255, 255)]
        );
    }

    #[test]
    fn theme_palettes_are_read_only() {
        let mut store = PaletteStore::in_memory(Vec::new());
        store.add_read_only("Soft", vec![BLUE]);
        assert!(store.add_colour(0, SAND).is_err());
        assert!(store.rename(0, "Mine").is_err());
        assert!(store.delete(0).is_err());
        assert_eq!(store.get(0).unwrap().colours, [BLUE]);
    }
}
