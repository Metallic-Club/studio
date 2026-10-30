//! `Metallic.exe`: the Metallic Launcher. Shows the Metallic site in a native window.
//!
//! This file decides what the user sees when something goes wrong; the library returns errors.

// Release builds are a window program, so Windows opens no console behind it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod platform;
mod shell;

use metallic_launcher::{
    Acquired, HandoffMessage, LauncherConfig, SettingsStore, is_installed_copy,
};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Velopack must run first: during an install, update or uninstall it handles its own
    // arguments (registering or removing `metallic://`) and exits.
    platform::run_velopack();

    let settings = SettingsStore::open_default().load();
    let config = match LauncherConfig::from_args(std::env::args().skip(1), &settings) {
        Ok(config) => config,
        Err(error) => {
            log::error!("could not start: {error}");
            std::process::exit(2);
        }
    };

    // Only one Launcher runs at a time. This comes before any window: a second start hands its
    // link (or a request to show the window) to the first one and leaves. `--exit-after-load`
    // is a test hook and neither takes part nor blocks other Launchers.
    let handoffs = if config.exit_after_load {
        None
    } else {
        match platform::acquire_instance() {
            Ok(Acquired::Primary(listener)) => Some(listener),
            Ok(Acquired::Secondary(mut sender)) => {
                let message = match &config.start_link {
                    // The canonical link, never the text that was received.
                    Some(link) => HandoffMessage::Link(link.to_uri()),
                    None => HandoffMessage::Focus,
                };
                match sender.send(&message) {
                    Ok(()) => std::process::exit(0),
                    Err(error) => {
                        log::error!("could not hand over to the running Launcher: {error}");
                        std::process::exit(1);
                    }
                }
            }
            Err(error) => {
                log::warn!("running without the single-instance check: {error}");
                None
            }
        }
    };

    // An installed copy keeps `metallic://` pointing at itself (the folder may have moved). A
    // copy run from `target\` never registers.
    if let Ok(exe) = std::env::current_exe()
        && is_installed_copy(&exe)
        && !platform::protocol_registered_to(&exe)
        && let Err(error) = platform::register_protocol(&exe)
    {
        log::warn!("could not register metallic:// links: {error}");
    }

    let version = match platform::webview_runtime() {
        Ok(version) => version,
        Err(error) => {
            log::error!("the web view runtime is missing: {error}");
            platform::show_webview_missing();
            std::process::exit(1);
        }
    };

    if let Err(error) = shell::run(config, &version, handoffs) {
        log::error!("the Launcher stopped: {error}");
        std::process::exit(1);
    }
}
