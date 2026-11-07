//! Studio's built-in icons (#291): Lucide SVGs in the `icons` folder beside the executable, not compiled into it.
//!
//! The folder holds one SVG per Lucide icon Studio uses (`box.svg`, `scroll.svg`), Lucide's licence, and
//! `icons.json`, which says which file each of Studio's icon ids uses (`"explorer.part": { "file": "box.svg" }`), with a
//! line on what the icon marks. Pointing an entry at another SVG changes that icon; a file that is missing or cannot
//! be read falls back to the icon's default file, with one warning. The build copies the folder from
//! `apps/studio/icons` to beside the executable.
//!
//! An icon is read on first use and drawn at exactly the pixel size it is shown at, so it stays sharp at every
//! interface scale; each size is cached as a texture in the egui context. The custom icon folder setting
//! ([`super::custom`]) still overrides any of them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use egui::{Context, Id, TextureHandle, TextureId, TextureOptions};
use serde::{Deserialize, Serialize};

use super::Icon;

/// The folder's name, beside the executable.
pub const FOLDER: &str = "icons";
/// The manifest's name, in the folder.
pub const MANIFEST: &str = "icons.json";
/// The manifest's format version.
pub const FORMAT: u32 = 1;

/// `icons.json`: which SVG each icon id uses.
#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: u32,
    about: String,
    icons: BTreeMap<String, Entry>,
}

/// One icon in `icons.json`.
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    /// The SVG it uses, relative to the folder.
    file: String,
    /// What the icon marks, for people editing the file.
    #[serde(rename = "for", default)]
    used_for: String,
}

/// What `icons.json` says first, for people editing it.
const ABOUT: &str = "Which SVG file each of Metallic Studio's icons uses, relative to this folder. To change an \
                     icon, put an SVG here and point its entry's file at it. The ids never change.";

/// `icons.json` as shipped: every icon with its default file, in the table's order of ids.
pub fn manifest_json() -> String {
    let manifest = Manifest {
        format: FORMAT,
        about: ABOUT.to_owned(),
        icons: Icon::ALL
            .iter()
            .map(|icon| {
                (
                    icon.id().to_owned(),
                    Entry {
                        file: icon.default_file().to_owned(),
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

/// The icons folder beside the executable, or beside the folder it is in (where tests and examples run from), if
/// there is one.
pub fn folder() -> Option<&'static Path> {
    static FOLDER_FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOLDER_FOUND
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let dir = exe.parent()?;
            let found = [Some(dir), dir.parent()]
                .into_iter()
                .flatten()
                .map(|dir| dir.join(FOLDER))
                .find(|dir| dir.is_dir());
            if found.is_none() {
                log::warn!(
                    "icons: no {FOLDER} folder beside {}; Studio draws no icons",
                    exe.display()
                );
            }
            found
        })
        .as_deref()
}

/// The built-in icons as read so far: from which folder, each icon's file, its SVG and its textures by size.
struct Loaded {
    dir: Option<PathBuf>,
    /// Each icon's file, from `icons.json`; an icon missing there uses its default.
    files: HashMap<Icon, PathBuf>,
    /// Each icon's SVG once read; `None` when it could not be.
    svgs: HashMap<Icon, Option<Arc<[u8]>>>,
    /// Each icon drawn so far, by its side in pixels.
    textures: HashMap<(Icon, u32), Option<TextureHandle>>,
}

impl Loaded {
    /// Reads `icons.json` in `dir`: each icon's file, or its default where the manifest has none or a bad one.
    fn new(dir: Option<PathBuf>) -> Self {
        let mut files = HashMap::new();
        if let Some(dir) = &dir {
            let manifest = std::fs::read_to_string(dir.join(MANIFEST))
                .map_err(|error| error.to_string())
                .and_then(|text| serde_json::from_str::<Manifest>(&text).map_err(|error| error.to_string()));
            match manifest {
                Ok(manifest) => {
                    for (id, entry) in manifest.icons {
                        match Icon::from_id(&id) {
                            Some(icon) => {
                                files.insert(icon, dir.join(entry.file));
                            }
                            None => log::warn!("icons: {MANIFEST} names `{id}`, which is not an icon id"),
                        }
                    }
                }
                Err(error) => log::warn!(
                    "icons: {}: {error}; using the default files",
                    dir.join(MANIFEST).display()
                ),
            }
        }
        Self {
            dir,
            files,
            svgs: HashMap::new(),
            textures: HashMap::new(),
        }
    }

    /// `icon`'s SVG: its file from the manifest, else its default file; read once.
    fn svg(&mut self, icon: Icon) -> Option<Arc<[u8]>> {
        if let Some(svg) = self.svgs.get(&icon) {
            return svg.clone();
        }
        let dir = self.dir.as_ref()?;
        let default = dir.join(icon.default_file());
        let chosen = self.files.get(&icon).cloned().unwrap_or_else(|| default.clone());
        let svg = match std::fs::read(&chosen) {
            Ok(bytes) => Some(bytes),
            Err(error) if chosen != default => {
                log::warn!("icons: {}: {error}; using {}", chosen.display(), icon.default_file());
                std::fs::read(&default).ok()
            }
            Err(error) => {
                log::warn!("icons: {}: {error}", chosen.display());
                None
            }
        }
        .map(Arc::from);
        self.svgs.insert(icon, svg.clone());
        svg
    }

    /// `icon` drawn `side` pixels square, white on transparency for tinting.
    fn texture(&mut self, ctx: &Context, icon: Icon, side: u32) -> Option<TextureId> {
        if let Some(texture) = self.textures.get(&(icon, side)) {
            return texture.as_ref().map(TextureHandle::id);
        }
        let texture = self
            .svg(icon)
            .and_then(|svg| match super::custom::rasterise_svg(&svg, side) {
                Ok(image) => {
                    // White, keeping the coverage: the paint's tint is the icon's colour.
                    let pixels = image
                        .pixels
                        .iter()
                        .map(|pixel| egui::Color32::from_white_alpha(pixel.a()))
                        .collect();
                    let image = egui::ColorImage::new(image.size, pixels);
                    Some(ctx.load_texture(format!("icon {} {side}", icon.id()), image, TextureOptions::LINEAR))
                }
                Err(error) => {
                    log::warn!("icons: {} for {}: {error}", icon.default_file(), icon.id());
                    None
                }
            });
        let id = texture.as_ref().map(TextureHandle::id);
        self.textures.insert((icon, side), texture);
        id
    }
}

/// The built-in icons in a context.
#[derive(Clone)]
struct Shared(Arc<Mutex<Loaded>>);

fn shared_id() -> Id {
    Id::unique("metallic.icons.builtin")
}

/// The built-in icons of `ctx`, reading the folder beside the executable the first time.
fn shared(ctx: &Context) -> Shared {
    if let Some(shared) = ctx.data(|data| data.get_temp::<Shared>(shared_id())) {
        return shared;
    }
    let shared = Shared(Arc::new(Mutex::new(Loaded::new(folder().map(Path::to_path_buf)))));
    ctx.data_mut(|data| data.insert_temp(shared_id(), shared.clone()));
    shared
}

/// Reads the built-in icons from `dir` in `ctx` from now on, in place of the folder beside the executable (tests).
#[cfg(test)]
pub fn use_folder(ctx: &Context, dir: &Path) {
    let shared = Shared(Arc::new(Mutex::new(Loaded::new(Some(dir.to_path_buf())))));
    ctx.data_mut(|data| data.insert_temp(shared_id(), shared));
}

/// `icon` as a texture `side` pixels square, white on transparency, or `None` when its SVG is missing or bad.
pub fn texture(ctx: &Context, icon: Icon, side: u32) -> Option<TextureId> {
    let shared = shared(ctx);
    let mut loaded = shared.0.lock().ok()?;
    loaded.texture(ctx, icon, side.clamp(1, 512))
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
        .filter(|((drawn, _), _)| *drawn == icon)
        .filter_map(|(_, texture)| texture.as_ref().map(TextureHandle::id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repository's icons folder, which the build copies beside the executable.
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
        assert!(dir.join("LICENSE-Lucide.txt").is_file());
    }

    /// Writes `icons/icons.json` from the table.
    #[test]
    #[ignore = "writes apps/studio/icons/icons.json"]
    fn write_the_icon_manifest() {
        std::fs::write(repo_folder().join(MANIFEST), manifest_json()).unwrap();
    }

    #[test]
    fn the_build_puts_the_folder_beside_the_executable() {
        let found = folder().expect("an icons folder beside the test executable");
        assert!(found.join(MANIFEST).is_file(), "{}", found.display());
    }

    /// A folder of its own under the system's temporary folder, holding `files`.
    fn temp_folder(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-builtin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
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
                ("box.svg", DOT),
                ("shapes.svg", DOT),
                ("folder.svg", DOT),
            ],
        );
        std::fs::create_dir_all(dir.join("mine")).unwrap();
        std::fs::write(dir.join("mine/square.svg"), SQUARE).unwrap();
        let mut loaded = Loaded::new(Some(dir.clone()));
        // The part uses the file the manifest names, not box.svg.
        assert_eq!(loaded.svg(Icon::ExplorerPart).as_deref(), Some(SQUARE.as_bytes()));
        // The model's file is gone: its default, shapes.svg.
        assert_eq!(loaded.svg(Icon::ExplorerModel).as_deref(), Some(DOT.as_bytes()));
        // Not in the manifest: its default.
        assert_eq!(loaded.svg(Icon::ExplorerFolder).as_deref(), Some(DOT.as_bytes()));
        // No file at all: nothing, and no panic.
        assert!(loaded.svg(Icon::ExplorerCamera).is_none());

        // Drawn at the size asked, white for tinting, and cached.
        let ctx = Context::default();
        let a = loaded.texture(&ctx, Icon::ExplorerPart, 20).expect("a texture");
        assert_eq!(loaded.texture(&ctx, Icon::ExplorerPart, 20), Some(a), "cached");
        assert_ne!(
            loaded.texture(&ctx, Icon::ExplorerPart, 40),
            Some(a),
            "another size, another texture"
        );
        assert!(loaded.texture(&ctx, Icon::ExplorerCamera, 20).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_manifest_uses_every_default() {
        let dir = temp_folder("bad-manifest", &[(MANIFEST, "{ not json"), ("box.svg", SQUARE)]);
        let mut loaded = Loaded::new(Some(dir.clone()));
        assert_eq!(loaded.svg(Icon::ExplorerPart).as_deref(), Some(SQUARE.as_bytes()));
        // No folder: nothing, quietly.
        let mut none = Loaded::new(None);
        assert!(none.svg(Icon::ExplorerPart).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
