//! Everything that depends on the operating system. The logic modules never use `cfg(windows)`;
//! this folder does. Windows is in `windows.rs`; other systems get the stubs in `other.rs`
//! that return `LauncherError::Unsupported`.

use std::path::PathBuf;
use std::sync::Arc;

use metallic_launcher::{EventSink, LauncherError, UrlPolicy};
use url::Url;
use wry::{WebView, WebViewBuilder};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
use other as imp;

/// Where the "install WebView2" page is.
const WEBVIEW2_DOWNLOAD: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

/// Opens `url` in the system browser. Only `https` is accepted; anything else is refused.
pub fn open_in_browser(url: &Url) -> Result<(), LauncherError> {
    if url.scheme() != "https" {
        return Err(LauncherError::Platform(
            "only https addresses open in the browser".to_string(),
        ));
    }
    imp::open_in_browser(url)
}

/// The version of the web view runtime, or why there is none.
pub fn webview_runtime() -> Result<String, String> {
    wry::webview_version().map_err(|error| error.to_string())
}

/// Tells the user the web view runtime is missing. Windows: a message box whose "Yes" opens the
/// download page in the browser.
pub fn show_webview_missing() {
    if imp::show_webview_missing()
        && let Ok(url) = Url::parse(WEBVIEW2_DOWNLOAD)
        && let Err(error) = open_in_browser(&url)
    {
        log::warn!("could not open the download page: {error}");
    }
}

/// The folder for the web view's own data (cookies, cache), or `None` for the system default.
pub fn webview_data_dir() -> Option<PathBuf> {
    imp::webview_data_dir()
}

/// The address the web view must load for a bundled page. Windows serves the `metallic-app`
/// protocol as `http://metallic-app.localhost/...`; other systems use the scheme as it is.
pub fn webview_local_url(url: &Url) -> Url {
    imp::webview_local_url(url)
}

/// Applies the settings that only exist on this platform to the web view builder.
pub fn configure_builder(builder: WebViewBuilder<'_>, dev_mode: bool) -> WebViewBuilder<'_> {
    imp::configure_builder(builder, dev_mode)
}

/// Reports a failed main-frame navigation to the sink as the "load failed" offline page.
pub fn watch_navigation_failures(view: &WebView, sink: Arc<dyn EventSink>) {
    imp::watch_navigation_failures(view, sink);
}

/// Asks `policy` about every navigation inside a frame and cancels the ones it refuses.
pub fn watch_frame_navigations(view: &WebView, policy: Arc<UrlPolicy>) {
    imp::watch_frame_navigations(view, policy);
}
