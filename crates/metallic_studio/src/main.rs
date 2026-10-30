//! Metallic Studio entry point: opens the window and runs the app.

use std::sync::Arc;

use eframe::egui;
use metallic_studio::app::{Options, StudioApp, WINDOW_TITLE, icon_image};

fn main() -> eframe::Result {
    // Must run first: when the installer or an update starts Studio with a hook argument,
    // Velopack handles it here and exits.
    velopack::VelopackApp::build().run();
    env_logger::init();
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(message) => {
            eprintln!(
                "{message}\nusage: metallic-studio [--screenshot <png> --scene splash|start|error --location <dir>]"
            );
            std::process::exit(2);
        }
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([960.0, 600.0]);
    match icon_image() {
        Ok(img) => {
            viewport = viewport.with_icon(Arc::new(egui::IconData {
                rgba: img.rgba,
                width: img.width,
                height: img.height,
            }));
        }
        Err(e) => log::warn!("window icon not set: {e}"),
    }
    let native = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        native,
        Box::new(move |cc| Ok(Box::new(StudioApp::new(&cc.egui_ctx, options.clone())))),
    )
}
