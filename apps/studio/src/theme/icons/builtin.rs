//! Studio's built-in icons (#291, #312): Lucide SVGs in the user's icons folder, which Studio fills from the icons
//! built into it.
//!
//! The folder (#322) holds `icons.json` and a folder per part of Studio that draws icons (`explorer`, `assets`,
//! `panels`, `toolbar`, `interface`), each with the Lucide SVGs it uses (`explorer/box.svg`), and Lucide's licence in
//! `licenses`. `icons.json` says which file each of Studio's icon ids uses and which colour group it is drawn in
//! (`"explorer.part": { "file": "explorer/box.svg", "color": "object" }`), with a line on what the icon marks, and
//! each colour group's colour on the dark and the light mode. Pointing an entry at another SVG or group changes that
//! icon, and changing a group's colours changes every icon in it; a file that is missing or cannot be read falls back
//! to the icon's default file, with one warning.
//!
//! **The shipped icons are built into the executable** (#402, [`SHIPPED`]: the build lists `apps/studio/icons`), so a
//! Studio given to someone without any folder beside it still has them: startup copies them into the user's folder
//! ([`crate::folders::prepare`]), and whatever is missing there is read from them.
//!
//! An icon is read on first use and drawn at exactly the pixel size it is shown at, so it stays sharp at every
//! interface scale; each size is cached as a texture in the egui context. The custom icon folder setting
//! ([`super::custom`]) still overrides any of them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use egui::{Color32, Context, Id, TextureHandle, TextureId, TextureOptions};
use serde::{Deserialize, Serialize};

use super::{Group, Icon};

/// The folder's name.
pub const FOLDER: &str = "icons";

/// The icons Studio ships, built into it (#402): every file of `apps/studio/icons` by its path in the folder
/// (`explorer/box.svg`, `icons.json`), with its bytes.
pub static SHIPPED: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/shipped_icons.rs"));

/// The shipped file at `path` in the icons folder (`/` between folders).
pub fn shipped(path: &str) -> Option<&'static [u8]> {
    SHIPPED.iter().find(|(name, _)| *name == path).map(|(_, bytes)| *bytes)
}
/// The manifest's name, in the folder.
pub const MANIFEST: &str = "icons.json";
/// The manifest's format version: 2 has the colours and the sub-folders (#322), 1 was a flat folder.
pub const FORMAT: u32 = 2;

/// `icons.json`: which SVG each icon id uses and in which colour, and the colour groups' colours.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) format: u32,
    #[serde(default)]
    about: String,
    /// Each colour group's colours, by its id (format 2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    colors: BTreeMap<String, Colours>,
    pub(crate) icons: BTreeMap<String, Entry>,
}

/// One icon in `icons.json`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// The SVG it uses, relative to the folder.
    pub(crate) file: String,
    /// The colour group it is drawn in, by its id; none for the interface's icons, which take their text's colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) color: Option<String>,
    /// What the icon marks, for people editing the file.
    #[serde(rename = "for", default)]
    used_for: String,
}

/// A colour group in `icons.json`: its colour on the dark and the light mode, as `#rrggbb`.
#[derive(Debug, Serialize, Deserialize)]
struct Colours {
    dark: String,
    light: String,
    /// Which icons it colours, for people editing the file.
    #[serde(rename = "for", default)]
    used_for: String,
}

/// What `icons.json` says first, for people editing it.
const ABOUT: &str = "Which SVG file each of Metallic Studio's icons uses (relative to this folder) and which colour \
                     it is drawn in. To change an icon, put an SVG here and point its entry's file at it. To change \
                     a colour, edit its group's dark and light colours under colors, or point an icon's color at \
                     another group. Icons without a color take the colour of the text beside them. The ids never \
                     change. Studio reads this file when it starts.";

/// `#rrggbb` for `color`.
fn hex(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

/// The colour `#rrggbb` (or `rrggbb`) names, if it is one.
pub(crate) fn parse_hex(text: &str) -> Option<Color32> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some(Color32::from_rgb(channel(0)?, channel(2)?, channel(4)?))
}

/// `icons.json` as shipped: every colour group with its colours, and every icon with its default file and group, in
/// the table's order of ids.
pub fn manifest_json() -> String {
    manifest_json_with(&HashMap::new())
}

/// `icons.json` as shipped, but with the icons in `files` pointed at the files given there (relative to the folder):
/// an older folder's own changes, carried into the new layout (#322).
pub(crate) fn manifest_json_with(files: &HashMap<Icon, String>) -> String {
    let manifest = Manifest {
        format: FORMAT,
        about: ABOUT.to_owned(),
        colors: Group::ALL
            .iter()
            .filter(|group| group.is_coloured())
            .map(|group| {
                let (dark, light) = group.default_colours();
                (
                    group.id().to_owned(),
                    Colours {
                        dark: hex(dark),
                        light: hex(light),
                        used_for: group.label().to_owned(),
                    },
                )
            })
            .collect(),
        icons: Icon::ALL
            .iter()
            .map(|icon| {
                (
                    icon.id().to_owned(),
                    Entry {
                        file: files.get(icon).cloned().unwrap_or_else(|| icon.default_file()),
                        color: icon.group().is_coloured().then(|| icon.group().id().to_owned()),
                        used_for: icon.description().to_owned(),
                    },
                )
            })
            .collect(),
    };
    // Serialising plain strings and maps cannot fail.
    let mut text = serde_json::to_string_pretty(&manifest).unwrap_or_default();
    text.push('\n');
    text
}

/// The folder Studio reads its icons from (#312): `icons` in its own folder (`%APPDATA%\Metallic\Studio\icons`),
/// which startup fills from the shipped icons ([`crate::folders::prepare`]) and the user may edit. `None` when it is
/// not there (nor in tests): the shipped icons are read instead ([`SHIPPED`]).
pub fn folder() -> Option<&'static Path> {
    static FOLDER_USED: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOLDER_USED
        .get_or_init(|| {
            if cfg!(test) {
                return None;
            }
            crate::folders::icons_folder().filter(|dir| dir.join(MANIFEST).is_file())
        })
        .as_deref()
}

/// The shipped `icons.json`.
fn shipped_manifest() -> Option<Manifest> {
    shipped(MANIFEST).and_then(|bytes| serde_json::from_slice(bytes).ok())
}

/// `dir`'s `icons.json`, or why it could not be read.
pub(crate) fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    std::fs::read_to_string(dir.join(MANIFEST))
        .map_err(|error| error.to_string())
        .and_then(|text| serde_json::from_str::<Manifest>(&text).map_err(|error| error.to_string()))
}

/// The colours `icons.json` gives (#322): each group's where it differs from Studio's, and each icon's group where it
/// differs from its own. Empty when there is none.
#[derive(Debug, Default)]
struct ColourTable {
    groups: HashMap<Group, (Color32, Color32)>,
    icons: HashMap<Icon, Group>,
}

impl ColourTable {
    /// The table `manifest` gives; what it gets wrong is logged and left at Studio's own.
    fn of(manifest: &Manifest) -> Self {
        let mut table = Self::default();
        for (id, colours) in &manifest.colors {
            let Some(group) = Group::from_id(id).filter(|group| group.is_coloured()) else {
                log::warn!("icons: {MANIFEST} has colours for `{id}`, which is not a colour group");
                continue;
            };
            match (parse_hex(&colours.dark), parse_hex(&colours.light)) {
                (Some(dark), Some(light)) if (dark, light) != group.default_colours() => {
                    table.groups.insert(group, (dark, light));
                }
                (Some(_), Some(_)) => {}
                _ => log::warn!("icons: {MANIFEST}: `{id}`'s colours are not #rrggbb; using Studio's"),
            }
        }
        for (id, entry) in &manifest.icons {
            let (Some(icon), Some(name)) = (Icon::from_id(id), &entry.color) else {
                continue;
            };
            match Group::from_id(name).filter(|group| group.is_coloured()) {
                Some(group) if group != icon.group() => {
                    table.icons.insert(icon, group);
                }
                Some(_) => {}
                None => log::warn!("icons: {MANIFEST}: `{id}`'s color `{name}` is not a colour group"),
            }
        }
        table
    }
}

/// The colours of the icons folder's `icons.json`, read once.
fn colour_table() -> &'static ColourTable {
    static TABLE: OnceLock<ColourTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        folder()
            .and_then(|dir| read_manifest(dir).ok())
            .map(|manifest| ColourTable::of(&manifest))
            .unwrap_or_default()
    })
}

/// `group`'s colours on the dark and the light mode as `icons.json` changes them, or `None` where it keeps Studio's.
pub fn group_colours(group: Group) -> Option<(Color32, Color32)> {
    colour_table().groups.get(&group).copied()
}

/// The colour group `icon` is drawn in: the one `icons.json` names for it, else its own.
pub fn colour_group(icon: Icon) -> Group {
    colour_table().icons.get(&icon).copied().unwrap_or(icon.group())
}

/// The built-in icons as read so far: from which folder (none: the shipped ones), each icon's file, its SVG and its
/// textures by size.
struct Loaded {
    dir: Option<PathBuf>,
    /// Each icon's file in the folder, from `icons.json`; an icon missing there uses its default.
    files: HashMap<Icon, String>,
    /// Each icon's SVG once read; `None` when it could not be.
    svgs: HashMap<Icon, Option<Arc<[u8]>>>,
    /// Each icon drawn so far, by its side in pixels and whether it is filled.
    textures: HashMap<(Icon, u32, bool), Option<TextureHandle>>,
}

impl Loaded {
    /// Reads `icons.json` in `dir` (the shipped one without a folder): each icon's file, or its default where the
    /// manifest has none or a bad one.
    fn new(dir: Option<PathBuf>) -> Self {
        let mut files = HashMap::new();
        let manifest = match &dir {
            Some(dir) => read_manifest(dir)
                .map_err(|error| {
                    log::warn!(
                        "icons: {}: {error}; using the default files",
                        dir.join(MANIFEST).display()
                    )
                })
                .ok(),
            None => shipped_manifest(),
        };
        for (id, entry) in manifest.map(|manifest| manifest.icons).unwrap_or_default() {
            match Icon::from_id(&id) {
                Some(icon) => {
                    files.insert(icon, entry.file);
                }
                None => log::warn!("icons: {MANIFEST} names `{id}`, which is not an icon id"),
            }
        }
        Self {
            dir,
            files,
            svgs: HashMap::new(),
            textures: HashMap::new(),
        }
    }

    /// The file at `path` in the folder, else the shipped one.
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let from_disk = self.dir.as_ref().map(|dir| std::fs::read(dir.join(path)));
        match from_disk {
            Some(Ok(bytes)) => Ok(bytes),
            Some(Err(error)) => shipped(path).map(<[u8]>::to_vec).ok_or_else(|| error.to_string()),
            None => shipped(path)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| "not one of Studio's icons".to_owned()),
        }
    }

    /// `icon`'s SVG: its file from the manifest, else its default file; read once.
    fn svg(&mut self, icon: Icon) -> Option<Arc<[u8]>> {
        if let Some(svg) = self.svgs.get(&icon) {
            return svg.clone();
        }
        let default = icon.default_file();
        let chosen = self.files.get(&icon).cloned().unwrap_or_else(|| default.clone());
        let svg = match self.read(&chosen) {
            Ok(bytes) => Some(bytes),
            Err(error) if chosen != default => {
                log::warn!("icons: {chosen}: {error}; using {default}");
                self.read(&default).ok()
            }
            Err(error) => {
                log::warn!("icons: {chosen}: {error}");
                None
            }
        }
        .map(Arc::from);
        self.svgs.insert(icon, svg.clone());
        svg
    }

    /// `icon` drawn `side` pixels square, white on transparency for tinting; `filled`, its shapes are filled as well as
    /// outlined (#302: the play controls).
    fn texture(&mut self, ctx: &Context, icon: Icon, side: u32, filled: bool) -> Option<TextureId> {
        if let Some(texture) = self.textures.get(&(icon, side, filled)) {
            return texture.as_ref().map(TextureHandle::id);
        }
        let texture = self.svg(icon).and_then(|svg| {
            // Lucide outlines its shapes with `fill="none"` on the root: filling them is filling in currentColor.
            let svg: Vec<u8> = if filled {
                String::from_utf8_lossy(&svg)
                    .replace("fill=\"none\"", "fill=\"currentColor\"")
                    .into_bytes()
            } else {
                svg.to_vec()
            };
            match super::custom::rasterise_svg(&svg, side) {
                Ok(image) => {
                    // White, keeping the coverage: the paint's tint is the icon's colour.
                    let pixels = image
                        .pixels
                        .iter()
                        .map(|pixel| egui::Color32::from_white_alpha(pixel.a()))
                        .collect();
                    let image = egui::ColorImage::new(image.size, pixels);
                    let name = format!("icon {} {side} {filled}", icon.id());
                    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
                }
                Err(error) => {
                    log::warn!("icons: {} for {}: {error}", icon.default_file(), icon.id());
                    None
                }
            }
        });
        let id = texture.as_ref().map(TextureHandle::id);
        self.textures.insert((icon, side, filled), texture);
        id
    }
}

/// The built-in icons in a context.
#[derive(Clone)]
struct Shared(Arc<Mutex<Loaded>>);

fn shared_id() -> Id {
    Id::unique("metallic.icons.builtin")
}

/// The built-in icons of `ctx`, reading the user's icons folder (or the shipped icons) the first time.
fn shared(ctx: &Context) -> Shared {
    if let Some(shared) = ctx.data(|data| data.get_temp::<Shared>(shared_id())) {
        return shared;
    }
    let shared = Shared(Arc::new(Mutex::new(Loaded::new(folder().map(Path::to_path_buf)))));
    ctx.data_mut(|data| data.insert_temp(shared_id(), shared.clone()));
    shared
}

/// Reads the built-in icons from `dir` in `ctx` from now on, in place of the user's folder (tests).
#[cfg(test)]
pub fn use_folder(ctx: &Context, dir: &Path) {
    let shared = Shared(Arc::new(Mutex::new(Loaded::new(Some(dir.to_path_buf())))));
    ctx.data_mut(|data| data.insert_temp(shared_id(), shared));
}

/// `icon` as a texture `side` pixels square, white on transparency, or `None` when its SVG is missing or bad.
pub fn texture(ctx: &Context, icon: Icon, side: u32, filled: bool) -> Option<TextureId> {
    let shared = shared(ctx);
    let mut loaded = shared.0.lock().ok()?;
    loaded.texture(ctx, icon, side.clamp(1, 512), filled)
}

/// Every texture `icon` has been drawn with so far, filled (or outlined, `filled` false), at any size.
#[cfg(test)]
pub fn textures_of_kind(ctx: &Context, icon: Icon, filled: bool) -> Vec<TextureId> {
    let shared = shared(ctx);
    let Ok(loaded) = shared.0.lock() else {
        return Vec::new();
    };
    loaded
        .textures
        .iter()
        .filter(|((drawn, _, kind), _)| *drawn == icon && *kind == filled)
        .filter_map(|(_, texture)| texture.as_ref().map(TextureHandle::id))
        .collect()
}

/// Every texture `icon` has been drawn with so far, at any size (tests look for an icon in a frame by them).
#[cfg(test)]
pub fn textures_of(ctx: &Context, icon: Icon) -> Vec<TextureId> {
    let shared = shared(ctx);
    let Ok(loaded) = shared.0.lock() else {
        return Vec::new();
    };
    loaded
        .textures
        .iter()
        .filter(|((drawn, _, _), _)| *drawn == icon)
        .filter_map(|(_, texture)| texture.as_ref().map(TextureHandle::id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repository's icons folder, which the build builds into the executable.
    fn repo_folder() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(FOLDER)
    }

    #[test]
    fn every_icon_has_its_svg_and_the_manifest_is_current() {
        let dir = repo_folder();
        for icon in Icon::ALL {
            let file = dir.join(icon.default_file());
            let bytes = std::fs::read(&file).unwrap_or_else(|_| panic!("{} is missing", file.display()));
            assert!(
                super::super::custom::rasterise_svg(&bytes, 16).is_ok(),
                "{}",
                file.display()
            );
            assert!(!icon.description().is_empty(), "{}", icon.id());
        }
        let written = std::fs::read_to_string(dir.join(MANIFEST))
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            written == manifest_json(),
            "icons/icons.json is stale: run `cargo test -p metallic_studio write_the_icon_manifest -- --ignored`"
        );
        // Lucide's licence ships with them.
        assert!(dir.join("licenses/LICENSE-Lucide.txt").is_file());
        // Built in as they are (#402): every file, byte for byte, every icon's among them.
        for (path, bytes) in SHIPPED {
            assert_eq!(std::fs::read(dir.join(path)).unwrap(), *bytes, "{path}");
        }
        for path in Icon::ALL
            .iter()
            .map(|icon| icon.default_file())
            .chain([MANIFEST.to_owned(), "licenses/LICENSE-Lucide.txt".to_owned()])
        {
            assert!(shipped(&path).is_some(), "{path} is not built in");
        }
        // Nothing else: icons.json alone at the top, each SVG in its part's folder and used (#322).
        let mut top: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        top.sort();
        assert_eq!(
            top,
            [
                "assets",
                "explorer",
                "icons.json",
                "interface",
                "licenses",
                "panels",
                "toolbar"
            ]
        );
        let used: std::collections::HashSet<String> = Icon::ALL.iter().map(|icon| icon.default_file()).collect();
        for folder in ["assets", "explorer", "interface", "panels", "toolbar"] {
            for entry in std::fs::read_dir(dir.join(folder)).unwrap() {
                let file = format!("{folder}/{}", entry.unwrap().file_name().to_string_lossy());
                assert!(used.contains(&file), "{file} is not used by any icon");
            }
        }
    }

    /// Writes `icons/icons.json` from the table.
    #[test]
    #[ignore = "writes apps/studio/icons/icons.json"]
    fn write_the_icon_manifest() {
        std::fs::write(repo_folder().join(MANIFEST), manifest_json()).unwrap();
    }

    #[test]
    fn with_no_folder_at_all_the_icons_come_from_the_executable() {
        // Tests read no folder (#402): what someone given Studio alone gets.
        assert!(folder().is_none());
        let ctx = Context::default();
        assert!(texture(&ctx, Icon::ExplorerPart, 16, false).is_some());
        assert!(texture(&ctx, Icon::ToolbarPlay, 24, true).is_some());
        assert!(shipped_manifest().is_some_and(|manifest| manifest.format == FORMAT));
    }

    /// A folder of its own under the system's temporary folder, holding `files`.
    fn temp_folder(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-builtin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    const SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="2" y="2" width="20" height="20" fill="currentColor"/></svg>"#;
    const DOT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3" fill="currentColor"/></svg>"#;

    #[test]
    fn the_manifest_points_icons_at_files_and_bad_entries_fall_back() {
        let manifest = r#"{ "format": 1, "about": "", "icons": {
            "explorer.part": { "file": "mine/square.svg", "for": "A part." },
            "explorer.model": { "file": "gone.svg" },
            "explorer.teleporter": { "file": "dot.svg" }
        } }"#;
        let dir = temp_folder(
            "manifest",
            &[
                (MANIFEST, manifest),
                ("explorer/box.svg", DOT),
                ("explorer/shapes.svg", DOT),
                ("explorer/folder.svg", DOT),
            ],
        );
        std::fs::create_dir_all(dir.join("mine")).unwrap();
        std::fs::write(dir.join("mine/square.svg"), SQUARE).unwrap();
        let mut loaded = Loaded::new(Some(dir.clone()));
        // The part uses the file the manifest names, not explorer/box.svg.
        assert_eq!(loaded.svg(Icon::ExplorerPart).as_deref(), Some(SQUARE.as_bytes()));
        // The model's file is gone: its default, explorer/shapes.svg.
        assert_eq!(loaded.svg(Icon::ExplorerModel).as_deref(), Some(DOT.as_bytes()));
        // Not in the manifest: its default.
        assert_eq!(loaded.svg(Icon::ExplorerFolder).as_deref(), Some(DOT.as_bytes()));
        // Not in the folder at all: the one built into Studio (#402).
        assert_eq!(
            loaded.svg(Icon::ExplorerCamera).as_deref(),
            shipped(&Icon::ExplorerCamera.default_file())
        );

        // Drawn at the size asked, white for tinting, and cached.
        let ctx = Context::default();
        let a = loaded.texture(&ctx, Icon::ExplorerPart, 20, false).expect("a texture");
        assert_eq!(loaded.texture(&ctx, Icon::ExplorerPart, 20, false), Some(a), "cached");
        assert_ne!(
            loaded.texture(&ctx, Icon::ExplorerPart, 40, false),
            Some(a),
            "another size, another texture"
        );
        assert!(loaded.texture(&ctx, Icon::ExplorerCamera, 20, false).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_manifest_s_colours_change_groups_and_icons() {
        let manifest = r##"{ "format": 2, "icons": {
            "explorer.part": { "file": "explorer/box.svg", "color": "workspace" },
            "explorer.model": { "file": "explorer/shapes.svg", "color": "object" },
            "explorer.folder": { "file": "explorer/folder.svg", "color": "no_such_group" }
        }, "colors": {
            "light": { "dark": "#102030", "light": "a0b0c0" },
            "workspace": { "dark": "#6bc98c", "light": "#2e7d4f" },
            "value": { "dark": "red", "light": "#000000" },
            "nobody": { "dark": "#000000", "light": "#000000" }
        } }"##;
        let table = ColourTable::of(&serde_json::from_str(manifest).unwrap());
        // Changed colours are kept; Studio's own (the workspace's) and bad ones are not.
        assert_eq!(
            table.groups.get(&Group::Light),
            Some(&(Color32::from_rgb(0x10, 0x20, 0x30), Color32::from_rgb(0xa0, 0xb0, 0xc0)))
        );
        assert_eq!(table.groups.len(), 1);
        // The part is drawn in the workspace's colour; the model keeps its own; a bad group is ignored.
        assert_eq!(table.icons.get(&Icon::ExplorerPart), Some(&Group::Workspace));
        assert_eq!(table.icons.len(), 1);
        assert_eq!(parse_hex("#FfA000"), Some(Color32::from_rgb(0xff, 0xa0, 0x00)));
        assert_eq!(parse_hex("#ffa00"), None);
        assert_eq!(parse_hex("#ffa0zz"), None);
        // The shipped file changes nothing: every icon in its own group, in Studio's colours.
        let shipped = ColourTable::of(&serde_json::from_str(&manifest_json()).unwrap());
        assert!(shipped.groups.is_empty() && shipped.icons.is_empty());
        assert_eq!(colour_group(Icon::ExplorerPart), Group::Object);
    }

    #[test]
    fn a_bad_manifest_uses_every_default() {
        let dir = temp_folder(
            "bad-manifest",
            &[(MANIFEST, "{ not json"), ("explorer/box.svg", SQUARE)],
        );
        let mut loaded = Loaded::new(Some(dir.clone()));
        assert_eq!(loaded.svg(Icon::ExplorerPart).as_deref(), Some(SQUARE.as_bytes()));
        // No folder: the shipped icons.
        let mut none = Loaded::new(None);
        assert_eq!(
            none.svg(Icon::ExplorerPart).as_deref(),
            shipped(&Icon::ExplorerPart.default_file())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
