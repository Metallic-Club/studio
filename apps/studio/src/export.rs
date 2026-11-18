//! Exporting an offline game as a standalone build (#783, ADR 0006 "Export"): File > Export Game writes into a folder
//!
//! - the game's executable: the Metallic Player, named after the game (`Rover.exe`), which runs the `game.pak` beside
//!   it; on Windows with the game's icon (a project picture, Game Settings > Icon), its version and author, and
//!   marked a windowed program so no console opens with it;
//! - `game.pak`: the project, packed ([`Pak::pack`]);
//! - `graphics.toml`: the graphics the game starts with (Game Settings > Graphics), which its player can change.
//!
//! The player executable is the one beside Studio's own. A Studio run from the source tree may not have built it: then
//! the export builds it (`cargo build -p metallic_player`, release when Studio is), and says how when it cannot.
//! The export runs on a thread of its own, from the project as last saved: Studio saves before it starts.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use metallic_engine::metallic_project::{AssetId, GAME_GRAPHICS_FILE, Manifest, PAK_FILE, Pak, Project, ProjectMode};
use metallic_engine::metallic_render::settings::GraphicsSettings;

/// The player executable's file name.
fn player_file() -> String {
    format!("metallic_player{}", std::env::consts::EXE_SUFFIX)
}

/// An export in progress.
#[derive(Debug)]
pub struct Export {
    done: Receiver<Result<PathBuf, String>>,
    /// The game's name, for what Studio says meanwhile.
    pub name: String,
}

impl Export {
    /// Exports the game saved at `project` into `folder`, the player taken from beside `studio` (Studio's executable),
    /// on a thread of its own.
    pub fn start(project: PathBuf, folder: PathBuf, studio: PathBuf, name: String) -> Result<Self, String> {
        let (send, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("export".into())
            .spawn(move || {
                let _ = send.send(export(&project, &folder, &studio));
            })
            .map_err(|error| format!("Could not start the export: {error}."))?;
        Ok(Self { done, name })
    }

    /// The executable written, or what went wrong, once the export is done.
    pub fn poll(&self) -> Option<Result<PathBuf, String>> {
        match self.done.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("The export stopped.".to_owned())),
        }
    }
}

/// Exports the game saved at `project` into `folder`; returns the executable written.
pub fn export(project: &Path, folder: &Path, studio: &Path) -> Result<PathBuf, String> {
    let mut project = Project::open(project).map_err(|error| format!("Could not open the project: {error}."))?;
    let manifest = project.manifest().clone();
    if manifest.mode != ProjectMode::Standalone {
        return Err("Only offline games export as a program: choose Offline in Game Settings.".to_owned());
    }
    let player = find_player(studio)?;
    std::fs::create_dir_all(folder).map_err(|error| format!("Could not make {}: {error}.", folder.display()))?;
    let exe = folder.join(format!("{}{}", file_name(&manifest.name), std::env::consts::EXE_SUFFIX));
    std::fs::copy(&player, &exe)
        .map_err(|error| format!("Could not copy the player to {}: {error}.", exe.display()))?;
    Pak::pack(&mut project, folder.join(PAK_FILE)).map_err(|error| format!("Could not pack the game: {error}."))?;
    let graphics = project
        .read_text(GAME_GRAPHICS_FILE)
        .ok()
        .filter(|text| GraphicsSettings::from_text(text).is_ok())
        .unwrap_or_else(|| GraphicsSettings::default().to_text());
    std::fs::write(folder.join(GAME_GRAPHICS_FILE), graphics)
        .map_err(|error| format!("Could not write the graphics settings: {error}."))?;
    let bytes = std::fs::read(&exe).map_err(|error| format!("Could not read {}: {error}.", exe.display()))?;
    if bytes.starts_with(b"MZ") {
        let icon = icon_picture(&mut project, &manifest);
        let branded = brand(&bytes, &manifest, icon.as_ref())?;
        std::fs::write(&exe, branded).map_err(|error| format!("Could not write {}: {error}.", exe.display()))?;
    }
    log::info!("export: {} written to {}", manifest.name, folder.display());
    Ok(exe)
}

/// `name` as a file name: without the characters file systems refuse, `Game` when nothing is left.
pub fn file_name(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control())
        .collect();
    let kept = kept.trim().trim_end_matches('.').trim().to_owned();
    if kept.is_empty() { "Game".to_owned() } else { kept }
}

/// The player executable beside `studio`; built first when Studio runs from the source tree and it is not there.
fn find_player(studio: &Path) -> Result<PathBuf, String> {
    let beside = studio.with_file_name(player_file());
    if beside.is_file() {
        return Ok(beside);
    }
    let how = "Build it once with `cargo build --release -p metallic_player`, then export again.";
    let Some((workspace, release)) = source_tree(studio) else {
        return Err(format!(
            "Studio could not find the Metallic Player ({}) beside it. {how}",
            player_file()
        ));
    };
    log::info!("export: building the player in {}", workspace.display());
    let mut cargo = std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo.current_dir(&workspace).args(["build", "-p", "metallic_player"]);
    if release {
        cargo.arg("--release");
    }
    let built = cargo
        .output()
        .map_err(|error| format!("Could not run cargo to build the player ({error}). {how}"))?;
    if !built.status.success() || !beside.is_file() {
        let said = String::from_utf8_lossy(&built.stderr);
        let last = said
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default();
        return Err(format!("The player could not be built: {last}. {how}"));
    }
    Ok(beside)
}

/// The workspace Studio was built in, when `studio` is in its `target` folder, and whether it is a release build.
fn source_tree(studio: &Path) -> Option<(PathBuf, bool)> {
    let profile = studio.parent()?;
    let target = profile.parent()?;
    if target.file_name()? != "target" {
        return None;
    }
    let workspace = target.parent()?;
    let manifest = std::fs::read_to_string(workspace.join("Cargo.toml")).ok()?;
    let ours = manifest.contains("[workspace]") && workspace.join("apps").join("player").is_dir();
    ours.then(|| {
        (
            workspace.to_path_buf(),
            profile.file_name().is_some_and(|name| name == "release"),
        )
    })
}

/// The icon's picture (Game Settings > Icon), decoded: its pixels and size.
fn icon_picture(project: &mut Project, manifest: &Manifest) -> Option<(u32, u32, Vec<u8>)> {
    let id: AssetId = manifest.icon.parse().ok()?;
    let mut reader = project.asset_reader();
    match reader.load_image(&id.to_string()) {
        Ok(image) => Some((image.width, image.height, image.rgba)),
        Err(error) => {
            log::warn!("export: the icon could not be read: {error}");
            None
        }
    }
}

/// The sides an exported executable's icon has, largest first.
const ICON_SIDES: [u32; 5] = [256, 64, 48, 32, 16];

/// A Windows executable's `bytes` made the game's (#783): `icon` (a picture's size and pixels) as its icon, its
/// version and author in its version resource, and marked a windowed program, so no console opens with it.
pub fn brand(bytes: &[u8], manifest: &Manifest, icon: Option<&(u32, u32, Vec<u8>)>) -> Result<Vec<u8>, String> {
    let fail = |error: &dyn std::fmt::Display| format!("Could not set the program's icon and version: {error}.");
    let mut image = editpe::Image::parse(bytes).map_err(|error| fail(&error))?;
    let mut resources = image.resource_directory().cloned().unwrap_or_default();
    if let Some((width, height, rgba)) = icon {
        let ico = ico(*width, *height, rgba).map_err(|error| fail(&error))?;
        resources.set_main_icon(ico.as_slice()).map_err(|error| fail(&error))?;
    }
    resources
        .set_version_info(&version_info(manifest))
        .map_err(|error| fail(&error))?;
    image.set_resource_directory(resources).map_err(|error| fail(&error))?;
    let mut out = image.data().to_vec();
    windowed(&mut out);
    Ok(out)
}

/// Marks the executable `bytes` a windowed (GUI) program: Windows then opens no console with it.
fn windowed(bytes: &mut [u8]) {
    const GUI: u16 = 2;
    let Some(header) = bytes
        .get(0x3c..0x40)
        .and_then(|at| at.try_into().ok())
        .map(|at: [u8; 4]| u32::from_le_bytes(at) as usize)
    else {
        return;
    };
    // The signature, the file header (20 bytes), then the optional header, whose `Subsystem` is at 68 in both PE32
    // and PE32+.
    let at = header + 4 + 20 + 68;
    if bytes.get(header..header + 4) == Some(b"PE\0\0".as_slice())
        && let Some(field) = bytes.get_mut(at..at + 2)
    {
        field.copy_from_slice(&GUI.to_le_bytes());
    }
}

/// The version resource: the game's version (`1.2.0` gives 1.2.0.0; none gives 1.0.0.0), its name and its author.
fn version_info(manifest: &Manifest) -> editpe::VersionInfo {
    let parts: Vec<u16> = manifest
        .version
        .split(['.', '-', '+'])
        .map_while(|part| part.trim().parse().ok())
        .chain(std::iter::repeat(0))
        .take(4)
        .collect();
    let parts = if manifest.version.trim().is_empty() {
        vec![1, 0, 0, 0]
    } else {
        parts
    };
    let version = editpe::types::VersionU32 {
        major: (u32::from(parts[0]) << 16) | u32::from(parts[1]),
        minor: (u32::from(parts[2]) << 16) | u32::from(parts[3]),
    };
    let mut info = editpe::VersionInfo::default();
    info.info.file_version = version;
    info.info.product_version = version;
    let shown = if manifest.version.trim().is_empty() {
        "1.0.0".to_owned()
    } else {
        manifest.version.clone()
    };
    let mut strings = editpe::VersionStringTable {
        key: "040904B0".to_owned(),
        ..Default::default()
    };
    for (key, value) in [
        ("FileDescription", manifest.name.as_str()),
        ("ProductName", manifest.name.as_str()),
        ("FileVersion", shown.as_str()),
        ("ProductVersion", shown.as_str()),
        ("CompanyName", manifest.author.as_str()),
        ("OriginalFilename", &format!("{}.exe", file_name(&manifest.name))),
    ] {
        if !value.is_empty() {
            strings.strings.insert(key.to_owned(), value.to_owned());
        }
    }
    info.strings.push(strings);
    // US English, Unicode.
    info.vars.push(editpe::types::VersionU16 {
        major: 0x0409,
        minor: 1200,
    });
    info
}

/// An `.ico` of `rgba` (`width` × `height`), centred in a square and made [`ICON_SIDES`] across, each a PNG.
pub fn ico(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, png::EncodingError> {
    let mut images = Vec::new();
    for side in ICON_SIDES {
        let pixels = square(width, height, rgba, side);
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, side, side);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&pixels)?;
        }
        images.push((side, encoded));
    }
    let count = images.len() as u16;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * u32::from(count);
    for (side, png) in &images {
        // 256 is written as 0.
        let byte = if *side >= 256 { 0 } else { *side as u8 };
        out.extend_from_slice(&[byte, byte, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend_from_slice(&png);
    }
    Ok(out)
}

/// `rgba` (`width` × `height`) fitted into a transparent square `side` across, centred, each pixel the average of the
/// pixels it covers.
fn square(width: u32, height: u32, rgba: &[u8], side: u32) -> Vec<u8> {
    let (width, height) = (width.max(1) as f32, height.max(1) as f32);
    let scale = (side as f32 / width).min(side as f32 / height);
    let (shown_w, shown_h) = (width * scale, height * scale);
    let (left, top) = ((side as f32 - shown_w) / 2.0, (side as f32 - shown_h) / 2.0);
    let mut out = vec![0u8; (side * side * 4) as usize];
    let pixel = |x: u32, y: u32| -> [f32; 4] {
        let at = ((y * width as u32 + x) * 4) as usize;
        match rgba.get(at..at + 4) {
            Some(p) => [f32::from(p[0]), f32::from(p[1]), f32::from(p[2]), f32::from(p[3])],
            None => [0.0; 4],
        }
    };
    for y in 0..side {
        for x in 0..side {
            let (fx, fy) = (x as f32 + 0.5 - left, y as f32 + 0.5 - top);
            if fx < 0.0 || fy < 0.0 || fx >= shown_w || fy >= shown_h {
                continue;
            }
            // The source pixels under this one.
            let (x0, y0) = ((fx - 0.5) / scale, (fy - 0.5) / scale);
            let (x1, y1) = ((fx + 0.5) / scale, (fy + 0.5) / scale);
            let (sx0, sy0) = (x0.floor().max(0.0) as u32, y0.floor().max(0.0) as u32);
            let (sx1, sy1) = (
                (x1.ceil() as u32).clamp(sx0 + 1, width as u32),
                (y1.ceil() as u32).clamp(sy0 + 1, height as u32),
            );
            let mut sum = [0.0f32; 4];
            let mut count = 0.0;
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let p = pixel(sx, sy);
                    // Weighted by alpha, so transparent pixels lend no colour.
                    let a = p[3] / 255.0;
                    sum[0] += p[0] * a;
                    sum[1] += p[1] * a;
                    sum[2] += p[2] * a;
                    sum[3] += p[3];
                    count += 1.0;
                }
            }
            let alpha = sum[3] / count;
            let at = ((y * side + x) * 4) as usize;
            if alpha > 0.0 {
                let weight = alpha / 255.0 * count;
                out[at] = (sum[0] / weight).round().clamp(0.0, 255.0) as u8;
                out[at + 1] = (sum[1] / weight).round().clamp(0.0, 255.0) as u8;
                out[at + 2] = (sum[2] / weight).round().clamp(0.0, 255.0) as u8;
                out[at + 3] = alpha.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-export-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn template() -> metallic_engine::metallic_project::Template {
        metallic_engine::metallic_project::Template::from_dir(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/baseplate-3d"),
        )
        .unwrap()
    }

    #[test]
    fn a_game_exports_beside_a_copy_of_the_player() {
        let dir = folder("game");
        // Studio and its player, as an installed Studio has them.
        let studio = dir.join(format!("metallic_studio{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&studio, b"studio").unwrap();
        std::fs::write(dir.join(player_file()), b"the player").unwrap();
        let mut project = Project::create(dir.join("Rover.mtproj"), &template(), "Moon Rover: 2").unwrap();
        let mut manifest = project.manifest().clone();
        manifest.author = "Ada".into();
        project.set_manifest(manifest).unwrap();
        let low = GraphicsSettings {
            preset: metallic_engine::metallic_render::settings::Preset::Low,
            ..GraphicsSettings::default()
        };
        project.write(GAME_GRAPHICS_FILE, low.to_text()).unwrap();
        project.save().unwrap();

        let out = dir.join("Build");
        let exe = export(&dir.join("Rover.mtproj"), &out, &studio).unwrap();
        assert_eq!(
            exe,
            out.join(format!("Moon Rover 2{}", std::env::consts::EXE_SUFFIX)),
            "named after the game"
        );
        assert_eq!(
            std::fs::read(&exe).unwrap(),
            b"the player",
            "not a program: copied as it is"
        );
        let pak = metallic_engine::metallic_project::Pak::open(out.join(PAK_FILE)).unwrap();
        {
            use metallic_engine::metallic_project::GameFiles as _;
            assert_eq!(pak.manifest().author, "Ada");
        }
        let shipped = std::fs::read_to_string(out.join(GAME_GRAPHICS_FILE)).unwrap();
        assert_eq!(GraphicsSettings::from_text(&shipped).unwrap().preset, low.preset);
    }

    #[test]
    fn an_online_game_or_a_missing_player_says_why() {
        let dir = folder("refused");
        let studio = dir.join("metallic_studio");
        let mut project = Project::create(dir.join("Online.mtproj"), &template(), "Online").unwrap();
        let mut manifest = project.manifest().clone();
        manifest.mode = ProjectMode::Platform;
        project.set_manifest(manifest).unwrap();
        project.save().unwrap();
        let error = export(&dir.join("Online.mtproj"), &dir.join("Out"), &studio).unwrap_err();
        assert!(error.contains("Offline"), "{error}");

        let mut project = Project::create(dir.join("Offline.mtproj"), &template(), "Offline").unwrap();
        project.save().unwrap();
        let error = export(&dir.join("Offline.mtproj"), &dir.join("Out"), &studio).unwrap_err();
        assert!(error.contains("cargo build --release -p metallic_player"), "{error}");
    }

    #[test]
    fn names_become_file_names() {
        assert_eq!(file_name("Moon Rover: 2"), "Moon Rover 2");
        assert_eq!(file_name("a/b\\c?"), "abc");
        assert_eq!(file_name(" ... "), "Game");
    }

    #[test]
    fn an_icon_holds_every_size_as_a_png() {
        // A 3 × 2 red picture.
        let rgba: Vec<u8> = std::iter::repeat_n([255, 0, 0, 255], 6).flatten().collect();
        let ico = ico(3, 2, &rgba).unwrap();
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, ICON_SIDES.len() as u8, 0]);
        let first = u32::from_le_bytes(ico[18..22].try_into().unwrap()) as usize;
        assert_eq!(&ico[first..first + 8], b"\x89PNG\r\n\x1a\n");
        // Centred: red in the middle, clear above and below.
        let middle = square(3, 2, &rgba, 16);
        let at = |x: usize, y: usize| &middle[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4];
        assert_eq!(at(8, 8), &[255, 0, 0, 255]);
        assert_eq!(at(8, 0)[3], 0);
    }

    /// On Windows, a real program (this test's own) takes the game's icon and version, and opens no console.
    #[test]
    fn a_program_takes_the_game_s_icon_and_version() {
        let Ok(bytes) = std::fs::read(std::env::current_exe().unwrap()) else {
            return;
        };
        if !bytes.starts_with(b"MZ") {
            return;
        }
        let manifest = Manifest {
            author: "Ada".into(),
            version: "1.2.3".into(),
            ..Manifest::new("Rover")
        };
        let rgba: Vec<u8> = std::iter::repeat_n([0, 128, 255, 255], 4).flatten().collect();
        let branded = brand(&bytes, &manifest, Some(&(2, 2, rgba))).unwrap();
        let image = editpe::Image::parse(&branded).unwrap();
        let resources = image.resource_directory().unwrap();
        assert!(resources.get_main_icon().unwrap().is_some());
        let info = resources.get_version_info().unwrap().unwrap();
        let version = info.info.file_version;
        assert_eq!((version.major, version.minor), ((1 << 16) | 2, 3 << 16));
        assert_eq!(info.strings[0].strings["CompanyName"], "Ada");
        let header = u32::from_le_bytes(branded[0x3c..0x40].try_into().unwrap()) as usize;
        assert_eq!(
            &branded[header + 92..header + 94],
            &2u16.to_le_bytes(),
            "a windowed program"
        );
    }
}
