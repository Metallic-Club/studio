//! Windows 11: WebView2 settings, the system browser, the "WebView2 missing" message box.

use std::path::PathBuf;
use std::sync::Arc;

use ::windows::Win32::UI::Shell::ShellExecuteW;
use ::windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONINFORMATION, MB_YESNO, MessageBoxW, SW_SHOWNORMAL,
};
use ::windows::core::{BOOL, HSTRING, PWSTR, w};
use metallic_launcher::{AppEvent, EventSink, LauncherError, LocalPage, OfflineReason};
use url::Url;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_WEB_ERROR_STATUS, COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
};
use webview2_com::{NavigationCompletedEventHandler, take_pwstr};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

/// The text of the message box. The wording is fixed by the product text.
const MISSING_TEXT: &str = "Metallic needs the Microsoft Edge WebView2 Runtime. Install it, then start Metallic again.\n\nOpen the download page now?";

pub fn open_in_browser(url: &Url) -> Result<(), LauncherError> {
    let address = HSTRING::from(url.as_str());
    // SAFETY: both strings are valid, null-terminated UTF-16 for the duration of the call.
    let result = unsafe { ShellExecuteW(None, w!("open"), &address, None, None, SW_SHOWNORMAL) };
    // ShellExecuteW reports success with a value above 32.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(LauncherError::Platform(
            "the system browser could not be started".to_string(),
        ))
    }
}

/// Shows the message box. True when the user chose to open the download page.
pub fn show_webview_missing() -> bool {
    let text = HSTRING::from(MISSING_TEXT);
    // SAFETY: the strings are valid for the duration of the call; no owner window.
    let answer = unsafe { MessageBoxW(None, &text, w!("Metallic"), MB_YESNO | MB_ICONINFORMATION) };
    answer == IDYES
}

pub fn webview_data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(base)
            .join("Metallic")
            .join("Launcher")
            .join("WebView2"),
    )
}

pub fn webview_local_url(url: &Url) -> Url {
    let mut text = format!("http://metallic-app.localhost{}", url.path());
    if let Some(query) = url.query() {
        text.push('?');
        text.push_str(query);
    }
    Url::parse(&text).unwrap_or_else(|_| url.clone())
}

pub fn configure_builder(builder: WebViewBuilder<'_>, dev_mode: bool) -> WebViewBuilder<'_> {
    // Browser shortcuts (reload, print, find, zoom, save) only make sense for a developer.
    builder.with_browser_accelerator_keys(dev_mode)
}

pub fn watch_navigation_failures(view: &WebView, sink: Arc<dyn EventSink>) {
    let webview = view.webview();
    let handler = NavigationCompletedEventHandler::create(Box::new(move |webview, args| {
        let (Some(webview), Some(args)) = (webview, args) else {
            return Ok(());
        };
        // SAFETY: plain getters on live WebView2 objects, called on the UI thread that owns them.
        unsafe {
            let mut success = BOOL::default();
            args.IsSuccess(&mut success)?;
            if success.as_bool() {
                return Ok(());
            }
            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
            args.WebErrorStatus(&mut status)?;
            // A navigation that our own rules refused is cancelled; that is not a load failure.
            if status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                return Ok(());
            }
            // A failure on a bundled page must not start a loop.
            let mut source = PWSTR::null();
            webview.Source(&mut source)?;
            let source = take_pwstr(source);
            if let Ok(url) = Url::parse(&source)
                && metallic_launcher::site::is_local_page(&url)
            {
                return Ok(());
            }
        }
        sink.send(AppEvent::ShowLocal(LocalPage::Offline(
            OfflineReason::LoadFailed,
        )));
        Ok(())
    }));
    let mut token = 0i64;
    // SAFETY: the handler outlives the registration (WebView2 keeps its own reference).
    if let Err(error) = unsafe { webview.add_NavigationCompleted(&handler, &mut token) } {
        log::warn!("could not watch for failed page loads: {error}");
    }
}
