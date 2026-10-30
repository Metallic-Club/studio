//! `Metallic.exe`: the Metallic Launcher. Shows the Metallic site in a native window.
//!
//! This file decides what the user sees when something goes wrong; the library returns errors.

// Release builds are a window program, so Windows opens no console behind it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod platform;
mod shell;

use metallic_launcher::{LauncherConfig, SettingsStore};

fn main() {
    // Velopack must run first: during an install or update it handles its own arguments and exits.
    metallic_update::velopack::VelopackApp::build().run();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = SettingsStore::open_default().load();
    let config = match LauncherConfig::from_args(std::env::args().skip(1), &settings) {
        Ok(config) => config,
        Err(error) => {
            log::error!("could not start: {error}");
            std::process::exit(2);
        }
    };

    let version = match platform::webview_runtime() {
        Ok(version) => version,
        Err(error) => {
            log::error!("the web view runtime is missing: {error}");
            platform::show_webview_missing();
            std::process::exit(1);
        }
    };

    if let Err(error) = shell::run(config, &version) {
        log::error!("the Launcher stopped: {error}");
        std::process::exit(1);
    }
}
