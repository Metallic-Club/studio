//! Capturing one frame of the Studio window to a PNG (used for design review, not by creators).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, ColorImage};

use crate::StudioError;

/// Frames to wait before capturing, so textures and layout have settled.
const SETTLE_FRAMES: u32 = 8;

/// Writes `image` to `path` as an RGBA PNG.
pub fn save_png(path: &Path, image: &ColorImage) -> Result<(), StudioError> {
    let io = |e: std::io::Error| StudioError::io(path, e);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let file = std::fs::File::create(path).map_err(io)?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        image.width() as u32,
        image.height() as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let fail = |e: png::EncodingError| StudioError::Image {
        path: path.to_string_lossy().into_owned(),
        message: e.to_string(),
    };
    let mut writer = encoder.write_header().map_err(fail)?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    writer.write_image_data(&bytes).map_err(fail)
}

/// `image` encoded as an RGBA PNG in memory (bug reports).
pub fn png_bytes(image: &ColorImage) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width() as u32, image.height() as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
        writer.write_image_data(&bytes)?;
    }
    Ok(out)
}

/// Waits a few frames, requests one screenshot, saves it, then asks the window to close.
pub struct ScreenshotJob {
    path: PathBuf,
    frames: u32,
    requested: bool,
    done: Option<Receiver<()>>,
}

impl ScreenshotJob {
    /// Plans a capture to `path`.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            frames: 0,
            requested: false,
            done: None,
        }
    }

    /// Call once per frame. Returns true when the PNG is written and the window is closing.
    pub fn tick(&mut self, ctx: &egui::Context) -> bool {
        self.frames += 1;
        ctx.request_repaint();
        if !self.requested && self.frames >= SETTLE_FRAMES {
            let (tx, rx) = channel();
            let path = self.path.clone();
            ctx.request_screenshot(move |image| {
                if let Err(e) = save_png(&path, &image) {
                    log::error!("{e}");
                }
                let _ = tx.send(());
            });
            self.requested = true;
            self.done = Some(rx);
        }
        if self.done.as_ref().is_some_and(|rx| rx.try_recv().is_ok()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return true;
        }
        false
    }
}
