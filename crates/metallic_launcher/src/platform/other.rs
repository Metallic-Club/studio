//! Stubs for systems other than Windows. Linux (WebKitGTK) and the others arrive later; until
//! then each step that needs the system says it is not supported.

use std::path::PathBuf;
use std::sync::Arc;

use metallic_launcher::{EventSink, LauncherError};
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
