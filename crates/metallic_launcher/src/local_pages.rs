//! The pages bundled into the Launcher (offline page, demo page, settings) and how they are served.
//!
//! The web view asks the `metallic-app` protocol for a file; [`serve`] answers from files compiled
//! into the program. Only the exact bundled names exist, so a path can never reach the disk.

use url::Url;

use crate::site::LOCAL_SCHEME;

/// The Content-Security-Policy sent with every bundled page: only the bundled files may load,
/// no inline script or style, no network from the page itself, no framing.
pub const LOCAL_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

/// Why the offline page is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfflineReason {
    /// This build does not know the site address yet.
    NotConfigured,
    /// The site did not answer.
    Unreachable,
    /// The web view could not load the page.
    LoadFailed,
}

impl OfflineReason {
    /// The word used in the page address (`?reason=...`).
    pub fn as_query(self) -> &'static str {
        match self {
            OfflineReason::NotConfigured => "notconfigured",
            OfflineReason::Unreachable => "unreachable",
            OfflineReason::LoadFailed => "loadfailed",
        }
    }
}

/// A page bundled into the Launcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalPage {
    /// "Can't reach Metallic" with a Retry button.
    Offline(OfflineReason),
    /// The developer demo page (developer mode only).
    Demo,
    /// The Launcher settings.
    Settings,
}

/// The address of a bundled page, in the form `metallic-app://local/offline.html?reason=unreachable`.
/// The shell turns it into the form its platform needs.
pub fn local_url(page: &LocalPage) -> Url {
    let text = match page {
        LocalPage::Offline(reason) => format!(
            "{LOCAL_SCHEME}://local/offline.html?reason={}",
            reason.as_query()
        ),
        LocalPage::Demo => format!("{LOCAL_SCHEME}://local/demo.html"),
        LocalPage::Settings => format!("{LOCAL_SCHEME}://local/settings.html"),
    };
    // The text is built from constants; a test parses every page, so this cannot fail.
    Url::parse(&text).unwrap_or_else(|_| unreachable!("bundled page addresses are constants"))
}

/// What the protocol handler sends back.
#[derive(Clone, Copy, Debug)]
pub struct LocalResponse {
    /// HTTP status: 200 or 404.
    pub status: u16,
    /// The `Content-Type` value.
    pub content_type: &'static str,
    /// The body.
    pub body: &'static [u8],
}

impl LocalResponse {
    /// The headers to send: `Content-Type`, the CSP, `X-Content-Type-Options: nosniff` and
    /// `Cache-Control: no-store`. Every response has them, errors included.
    pub fn headers(&self) -> [(&'static str, &'static str); 4] {
        [
            ("Content-Type", self.content_type),
            ("Content-Security-Policy", LOCAL_CSP),
            ("X-Content-Type-Options", "nosniff"),
            ("Cache-Control", "no-store"),
        ]
    }
}

const HTML: &str = "text/html; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";

/// Every bundled file: its path, content type and bytes. Matching is exact.
pub const BUNDLED: &[(&str, &str, &[u8])] = &[
    (
        "/offline.html",
        HTML,
        include_bytes!("../assets/pages/offline.html"),
    ),
    (
        "/demo.html",
        HTML,
        include_bytes!("../assets/pages/demo.html"),
    ),
    (
        "/settings.html",
        HTML,
        include_bytes!("../assets/pages/settings.html"),
    ),
    (
        "/pages.css",
        CSS,
        include_bytes!("../assets/pages/pages.css"),
    ),
    ("/pages.js", JS, include_bytes!("../assets/pages/pages.js")),
];

const NOT_FOUND: &[u8] = b"Not found";

/// Answers a request for `path` (the path only: no query, no fragment). Only the exact bundled
/// names are served; everything else, including `..`, encoded dots and other letter case, is a 404.
pub fn serve(path: &str) -> LocalResponse {
    match BUNDLED.iter().find(|(name, _, _)| *name == path) {
        Some((_, content_type, body)) => LocalResponse {
            status: 200,
            content_type,
            body,
        },
        None => LocalResponse {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: NOT_FOUND,
        },
    }
}
