//! Stubs for systems other than Windows. Linux (WebKitGTK) and the others arrive later; until
//! then each step that needs the system says it is not supported.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use metallic_launcher::{Acquired, EventSink, LauncherError, Spawner, TrayAction, UrlPolicy};
use url::Url;
use wry::{WebView, WebViewBuilder};

pub fn open_in_browser(_url: &Url) -> Result<(), LauncherError> {
    Err(LauncherError::Unsupported("opening the system browser"))
}

pub fn show_webview_missing() -> bool {
    log::error!("the web view runtime is missing");
    false
}

pub fn webview_data_dir() -> Option<PathBuf> {
    None
}

pub fn webview_local_url(url: &Url) -> Url {
    url.clone()
}

pub fn configure_builder(builder: WebViewBuilder<'_>, _dev_mode: bool) -> WebViewBuilder<'_> {
    builder
}

pub fn watch_navigation_failures(_view: &WebView, _sink: Arc<dyn EventSink>) {}

pub fn watch_frame_navigations(_view: &WebView, _policy: Arc<UrlPolicy>) -> bool {
    true
}

pub fn acquire_instance() -> Result<Acquired, LauncherError> {
    Err(LauncherError::Unsupported("single instance"))
}

pub fn register_protocol(_exe: &Path) -> Result<(), LauncherError> {
    Err(LauncherError::Unsupported("metallic:// registration"))
}

pub fn protocol_registered_to(_exe: &Path) -> bool {
    false
}

/// Starts programs with `std::process::Command` (no shell).
pub struct SystemSpawner;

impl Spawner for SystemSpawner {
    fn spawn(&self, exe: &Path, args: &[OsString]) -> Result<u32, LauncherError> {
        std::process::Command::new(exe)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|child| child.id())
            .map_err(|e| LauncherError::Platform(format!("could not start the program: {e}")))
    }

    fn spawn_and_wait(
        &self,
        exe: &Path,
        args: &[OsString],
        time_limit: std::time::Duration,
    ) -> Result<i32, LauncherError> {
        metallic_launcher::run_with_limit(
            std::process::Command::new(exe)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
            time_limit,
        )
    }
}

/// No tray on this platform yet.
pub struct Tray;

impl Tray {
    pub fn set_items(&self, _items: &[(TrayAction, &'static str)]) {}
}

/// No way to look at processes on this platform yet.
pub struct SystemProcesses;

impl metallic_launcher::ProcessLister for SystemProcesses {
    fn running_images(&self) -> Option<Vec<PathBuf>> {
        None
    }
}

pub fn create_tray(
    _items: &[(TrayAction, &'static str)],
    _icon_rgba: Vec<u8>,
    _width: u32,
    _height: u32,
    _sink: Arc<dyn EventSink>,
) -> Result<Tray, LauncherError> {
    Err(LauncherError::Unsupported("the tray icon"))
}

pub fn run_velopack() {
    metallic_update::velopack::VelopackApp::build().run();
}
