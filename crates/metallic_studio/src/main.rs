//! Metallic Studio entry point: opens the window and runs the app.

use std::sync::Arc;

use eframe::egui;
use metallic_studio::app::{Options, StudioApp, WINDOW_TITLE, icon_image};
use metallic_studio::splash;

fn main() -> eframe::Result {
    // Must run first: when the installer or an update starts Studio with a hook argument,
    // Velopack handles it here and exits.
    velopack::VelopackApp::build().run();
    env_logger::init();
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(message) => {
            eprintln!(
                "{message}\nusage: metallic-studio [--screenshot <png> --scene splash|start|firstrun|error|editor|playing|profiler|colour|components|settings --theme graphite|titanium|light --location <dir> --size <w>x<h>]"
            );
            std::process::exit(2);
        }
    };

    let mut viewport = egui::ViewportBuilder::default().with_title(WINDOW_TITLE);
    viewport = if options.shows_splash() {
        // The first window is the splash; it becomes the main window when startup is done.
        splash::viewport(viewport)
    } else {
        viewport
            .with_inner_size(options.size.unwrap_or(splash::MAIN_SIZE.into()))
            .with_min_inner_size(splash::MAIN_MIN_SIZE)
    };
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
        centered: true,
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        native,
        Box::new(move |cc| Ok(Box::new(StudioApp::new(&cc.egui_ctx, options.clone())))),
    )
}

/// eframe's wgpu setup, with GPU timestamps for the profiler where the adapter has them.
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_setup {
        let base = create.device_descriptor.clone();
        create.device_descriptor = std::sync::Arc::new(move |adapter| {
            let mut descriptor = base(adapter);
            descriptor.required_features |=
                adapter.features() & eframe::wgpu::Features::TIMESTAMP_QUERY;
            descriptor
        });
    }
    options
}
