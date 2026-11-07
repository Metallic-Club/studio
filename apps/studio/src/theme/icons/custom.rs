//! The custom icon folder (#231): a folder the user picks in Settings → Appearance → Icon folder, where a file named
//! after an icon's id (`explorer.part.svg` or `.png`) replaces that icon.
//!
//! [`CustomIcons`] reads the folder on a background thread when the setting changes or Reload is pressed, so a large
//! folder never stalls a frame: SVGs are rasterised with `resvg`, PNGs decoded with `png`. The UI thread then turns
//! the images into textures and keeps them in the egui context, where [`get`] finds them for [`super::paint`]. A file
//! that cannot be read keeps the built-in icon and logs one warning, which the Output panel shows.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use egui::{ColorImage, Context, Id, TextureHandle, TextureOptions};

use super::Icon;

/// The side, in pixels, an SVG is rasterised at: sharp at the largest size Studio draws an icon, at 2× scale.
pub const RASTER_SIZE: u32 = 64;
/// The largest PNG side read; a larger file is refused rather than kept in memory.
pub const MAX_PNG_SIZE: u32 = 512;
/// How far a pixel's colour may stray from the others' and the image still count as one colour.
const MONOCHROME_TOLERANCE: u8 = 24;

/// A custom icon, ready to draw.
#[derive(Clone)]
pub struct CustomImage {
    /// The texture.
    pub texture: TextureHandle,
    /// Whether it is one colour on transparency: drawn in its group's colour, as the built-in icons are.
    pub monochrome: bool,
}

/// A custom icon read from its file, before it is a texture.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedImage {
    /// Its pixels. A monochrome image is stored white, so a tint gives it any colour.
    pub image: ColorImage,
    /// Whether it is one colour on transparency.
    pub monochrome: bool,
}

/// What reading a folder gave: the icons it replaces, and a line for each file that could not be read.
#[derive(Debug, Default)]
pub struct Loaded {
    /// The icons with a readable file.
    pub icons: Vec<(Icon, LoadedImage)>,
    /// What went wrong with the others, one line each.
    pub warnings: Vec<String>,
    /// Whether the folder was not there (or not a folder): said only when the user asked for it with Reload, so a
    /// path half typed in Settings stays quiet.
    pub missing: bool,
}

/// The textures in use, kept in the egui context so painting needs nothing but a painter.
#[derive(Clone, Default)]
struct Set(Arc<HashMap<Icon, CustomImage>>);

fn set_id() -> Id {
    Id::unique("metallic.icons.custom")
}

fn reload_id() -> Id {
    Id::unique("metallic.icons.reload")
}

/// The custom image for `icon`, if the folder has one.
pub fn get(ctx: &Context, icon: Icon) -> Option<CustomImage> {
    ctx.data(|data| data.get_temp::<Set>(set_id()))
        .and_then(|set| set.0.get(&icon).cloned())
}

/// Asks Studio to read the folder again (Settings → Appearance → Icon folder → Reload).
pub fn request_reload(ctx: &Context) {
    ctx.data_mut(|data| data.insert_temp(reload_id(), true));
}

/// Studio's custom icons: which folder they come from, and the reading in progress.
#[derive(Debug, Default)]
pub struct CustomIcons {
    /// The folder read or being read; `None` for the built-in icons.
    folder: Option<PathBuf>,
    /// The background reading, until it is done.
    loading: Option<Receiver<Loaded>>,
    /// Whether Reload asked for this reading.
    asked: bool,
}

impl CustomIcons {
    /// Whether a folder is being read.
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// Called every frame with the Icon folder setting: starts reading the folder when it changed or Reload was
    /// pressed, and puts what a finished reading gave into `ctx`.
    pub fn update(&mut self, ctx: &Context, folder: &str) {
        let wanted = Some(folder.trim())
            .filter(|folder| !folder.is_empty())
            .map(PathBuf::from);
        let reload = ctx.data_mut(|data| data.remove_temp::<bool>(reload_id()).unwrap_or(false));
        if wanted != self.folder || reload {
            self.folder.clone_from(&wanted);
            self.loading = None;
            self.asked = reload;
            match wanted {
                None => {
                    ctx.data_mut(|data| data.insert_temp(set_id(), Set::default()));
                }
                Some(dir) => self.loading = spawn_reading(dir, ctx.clone()),
            }
        }
        let Some(receiver) = &self.loading else {
            return;
        };
        match receiver.try_recv() {
            Ok(loaded) => {
                self.loading = None;
                for warning in &loaded.warnings {
                    log::warn!("icons: {warning}; using the built-in icon");
                }
                if loaded.missing && self.asked {
                    let folder = self.folder.as_deref().unwrap_or(Path::new("")).display();
                    log::warn!("icons: {folder} is not a folder; using the built-in icons");
                }
                let mut set = HashMap::new();
                for (icon, image) in loaded.icons {
                    let texture = ctx.load_texture(format!("icon {}", icon.id()), image.image, TextureOptions::LINEAR);
                    set.insert(
                        icon,
                        CustomImage {
                            texture,
                            monochrome: image.monochrome,
                        },
                    );
                }
                log::info!("icons: {} custom icons in use", set.len());
                ctx.data_mut(|data| data.insert_temp(set_id(), Set(Arc::new(set))));
                ctx.request_repaint();
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(50)),
            Err(TryRecvError::Disconnected) => self.loading = None,
        }
    }

    /// Reads the folder now, on this thread, and puts the result into `ctx` (tests and screenshots).
    #[cfg(test)]
    pub fn load_now(&mut self, ctx: &Context, folder: &str) {
        self.update(ctx, folder);
        if let Some(receiver) = self.loading.take() {
            if let Ok(loaded) = receiver.recv() {
                let (sender, receiver) = std::sync::mpsc::channel();
                let _ = sender.send(loaded);
                self.loading = Some(receiver);
            }
            self.update(ctx, folder);
        }
    }
}

/// Reads `dir` on a new thread; its result arrives on the receiver, and wakes `ctx`.
fn spawn_reading(dir: PathBuf, ctx: Context) -> Option<Receiver<Loaded>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("metallic-icons".into())
        .spawn(move || {
            let loaded = load_folder(&dir);
            // The receiver is gone when the setting changed meanwhile: nobody wants this reading any more.
            let _ = sender.send(loaded);
            ctx.request_repaint();
        });
    match spawned {
        Ok(_) => Some(receiver),
        Err(error) => {
            log::warn!("icons: could not start reading the icon folder: {error}");
            None
        }
    }
}

/// Every icon `dir` replaces, and a line for each file in it that could not be read. A missing folder replaces
/// nothing and says so.
pub fn load_folder(dir: &Path) -> Loaded {
    let mut loaded = Loaded::default();
    if !dir.is_dir() {
        loaded.missing = true;
        return loaded;
    }
    for &icon in Icon::ALL {
        // The SVG first: it wins when both are there.
        let Some(path) = icon
            .file_names()
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
        else {
            continue;
        };
        match load_file(&path) {
            Ok(image) => loaded.icons.push((icon, image)),
            Err(error) => loaded.warnings.push(format!("{}: {error}", path.display())),
        }
    }
    loaded
}

/// One custom icon file, SVG or PNG by its extension.
pub fn load_file(path: &Path) -> Result<LoadedImage, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let svg = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
    let image = if svg {
        rasterise_svg(&bytes)?
    } else {
        decode_png(&bytes)?
    };
    Ok(finish(image))
}

/// An SVG drawn into a [`RASTER_SIZE`] square, its aspect kept and centred.
fn rasterise_svg(bytes: &[u8]) -> Result<ColorImage, String> {
    use resvg::{tiny_skia, usvg};

    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).map_err(|error| error.to_string())?;
    let size = tree.size();
    let side = RASTER_SIZE as f32;
    let scale = side / size.width().max(size.height());
    if !scale.is_finite() || scale <= 0.0 {
        return Err("the SVG has no size".to_owned());
    }
    let mut pixmap = tiny_skia::Pixmap::new(RASTER_SIZE, RASTER_SIZE).ok_or("could not make an image to draw in")?;
    let offset = |extent: f32| (side - extent * scale) / 2.0;
    let transform =
        tiny_skia::Transform::from_scale(scale, scale).post_translate(offset(size.width()), offset(size.height()));
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let side = RASTER_SIZE as usize;
    Ok(ColorImage::from_rgba_premultiplied([side, side], pixmap.data()))
}

/// A PNG, any colour type, as RGBA.
fn decode_png(bytes: &[u8]) -> Result<ColorImage, String> {
    let error = |error: png::DecodingError| error.to_string();
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    // Palettes and low bit depths expanded, 16 bits cut to 8.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(error)?;
    let (width, height) = reader.info().size();
    if width > MAX_PNG_SIZE || height > MAX_PNG_SIZE {
        return Err(format!(
            "the image is {width} × {height}, larger than {MAX_PNG_SIZE} on a side"
        ));
    }
    let size = reader.output_buffer_size().ok_or("the image is too large")?;
    let mut data = vec![0; size];
    let info = reader.next_frame(&mut data).map_err(error)?;
    let data = &data[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("the palette was not expanded".to_owned()),
    };
    let (width, height) = (info.width as usize, info.height as usize);
    if rgba.len() != width * height * 4 {
        return Err("the image's data is the wrong size".to_owned());
    }
    Ok(ColorImage::from_rgba_unmultiplied([width, height], &rgba))
}

/// Whether `image` is one colour on transparency (within a small tolerance); if so, the same image in white, so a
/// tint gives it the group's colour.
fn finish(image: ColorImage) -> LoadedImage {
    let mut seen: Option<[u8; 3]> = None;
    let mut monochrome = true;
    for pixel in &image.pixels {
        // Below a quarter opacity: edge smoothing, whose colour says little.
        if pixel.a() < 64 {
            continue;
        }
        let [r, g, b, a] = pixel.to_srgba_unmultiplied();
        debug_assert!(a >= 64);
        let colour = [r, g, b];
        match seen {
            None => seen = Some(colour),
            Some(first) => {
                if first
                    .iter()
                    .zip(colour)
                    .any(|(a, b)| a.abs_diff(b) > MONOCHROME_TOLERANCE)
                {
                    monochrome = false;
                    break;
                }
            }
        }
    }
    if !monochrome || seen.is_none() {
        return LoadedImage {
            image,
            monochrome: monochrome && seen.is_some(),
        };
    }
    let pixels = image
        .pixels
        .iter()
        .map(|pixel| egui::Color32::from_white_alpha(pixel.a()))
        .collect();
    LoadedImage {
        image: ColorImage::new(image.size, pixels),
        monochrome: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own under the system's temporary folder, empty.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("metallic-icons-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Lucide's `box` as an SVG: strokes in `currentColor`, as Lucide ships them.
    const LUCIDE_BOX: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"/></svg>"#;

    /// Two colours: a red square and a blue circle.
    const TWO_COLOURS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="5" height="10" fill="red"/><circle cx="7.5" cy="5" r="2.5" fill="blue"/></svg>"#;

    /// A 2 × 2 RGBA PNG.
    fn png_bytes(pixels: [[u8; 4]; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels.as_flattened()).unwrap();
        }
        out
    }

    #[test]
    fn a_folder_replaces_the_icons_it_has_files_for() {
        let dir = folder("replace");
        std::fs::write(dir.join("explorer.part.svg"), LUCIDE_BOX).unwrap();
        std::fs::write(dir.join("explorer.script.svg"), TWO_COLOURS).unwrap();
        let red = [255, 0, 0, 255];
        std::fs::write(dir.join("panel.output.png"), png_bytes([red, red, [0, 0, 0, 0], red])).unwrap();
        // Both: the SVG wins.
        std::fs::write(dir.join("explorer.model.svg"), LUCIDE_BOX).unwrap();
        std::fs::write(dir.join("explorer.model.png"), b"not a png").unwrap();
        // Not an icon id: left alone.
        std::fs::write(dir.join("explorer.teleporter.svg"), LUCIDE_BOX).unwrap();
        // Unreadable: the built-in icon, and one warning.
        std::fs::write(dir.join("explorer.camera.svg"), "<svg").unwrap();
        std::fs::write(dir.join("asset.image.png"), b"\x89PNG broken").unwrap();

        let loaded = load_folder(&dir);
        let icons: HashMap<Icon, LoadedImage> = loaded.icons.into_iter().collect();
        let mut found: Vec<Icon> = icons.keys().copied().collect();
        found.sort();
        assert_eq!(
            found,
            [
                Icon::ExplorerPart,
                Icon::ExplorerModel,
                Icon::ExplorerScript,
                Icon::PanelOutput
            ]
        );
        let part = &icons[&Icon::ExplorerPart];
        assert!(part.monochrome, "one colour on transparency");
        assert_eq!(part.image.size, [RASTER_SIZE as usize; 2]);
        assert!(part.image.pixels.iter().any(|pixel| pixel.a() > 200), "drawn");
        assert!(
            part.image.pixels.iter().all(|pixel| pixel.r() == pixel.a()),
            "stored white, for tinting"
        );
        assert!(!icons[&Icon::ExplorerScript].monochrome, "red and blue");
        let output = &icons[&Icon::PanelOutput];
        assert!(output.monochrome);
        assert_eq!(output.image.size, [2, 2]);
        assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
        assert!(loaded.warnings.iter().any(|line| line.contains("explorer.camera.svg")));
        assert!(loaded.warnings.iter().any(|line| line.contains("asset.image.png")));

        let missing = load_folder(&dir.join("nowhere"));
        assert!(missing.icons.is_empty() && missing.missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn studio_uses_a_folder_s_icons_until_it_is_cleared() {
        let dir = folder("use");
        std::fs::write(dir.join("explorer.part.svg"), LUCIDE_BOX).unwrap();
        let ctx = Context::default();
        let mut icons = CustomIcons::default();
        assert!(get(&ctx, Icon::ExplorerPart).is_none());
        icons.load_now(&ctx, &dir.display().to_string());
        assert!(!icons.is_loading());
        let part = get(&ctx, Icon::ExplorerPart).expect("the folder's part");
        assert!(part.monochrome);
        assert!(get(&ctx, Icon::ExplorerModel).is_none(), "no file: built in");

        // A file changed and Reload pressed: read again.
        std::fs::write(dir.join("explorer.part.svg"), TWO_COLOURS).unwrap();
        request_reload(&ctx);
        icons.load_now(&ctx, &dir.display().to_string());
        assert!(!get(&ctx, Icon::ExplorerPart).unwrap().monochrome);

        // The setting cleared: the built-in icons again.
        icons.update(&ctx, "  ");
        assert!(get(&ctx, Icon::ExplorerPart).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_large_png_is_refused() {
        let mut out = Vec::new();
        {
            let side = MAX_PNG_SIZE + 1;
            let mut encoder = png::Encoder::new(&mut out, side, 1);
            encoder.set_color(png::ColorType::Grayscale);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![0; side as usize]).unwrap();
        }
        assert!(decode_png(&out).unwrap_err().contains("larger than"));
    }
}
