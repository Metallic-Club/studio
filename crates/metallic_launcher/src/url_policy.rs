//! What the Launcher's web view may load, and what happens to everything else.
//!
//! The rules, in order: the site, the development origin (developer mode), a bundled page and
//! exactly `about:blank` are allowed; a valid `metallic:` link becomes a deep link and an invalid
//! one is blocked; any other `https:` address opens in the system browser; everything else
//! (`file:`, `javascript:`, `data:`, `blob:`, `ms-msdt:`, plain `http:`, UNC paths, ...) is blocked.

use url::Url;

use crate::deep_link::{DeepLink, SCHEME};
use crate::site::{LauncherConfig, SiteOrigin, is_local_page};

/// Whether a page of `origin` may load. The site and other remote pages load only while frame
/// navigations are policed (`frames_policed`); if the frame handler could not be registered,
/// iframes would go unchecked, so only bundled pages load.
pub fn remote_content_allowed(frames_policed: bool, origin: PageOrigin) -> bool {
    frames_policed || origin == PageOrigin::Local
}

/// Which kind of page an address belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageOrigin {
    /// The Metallic site (or the development origin in developer mode): may use the bridge.
    Site,
    /// A page bundled into the Launcher.
    Local,
    /// Anything else, including unparsable text.
    Other,
}

/// What to do with a navigation or a new-window request.
#[derive(Debug, PartialEq, Eq)]
pub enum NavDecision {
    /// Let the view load it. For a new window: load it in the main view instead.
    Allow,
    /// Do not load it here; open it in the system browser.
    OpenInBrowser(Url),
    /// Do not load it; run this deep link instead.
    HandleDeepLink(DeepLink),
    /// Refuse it.
    Block,
}

/// Decides what the web view may load.
#[derive(Debug)]
pub struct UrlPolicy {
    site: Option<SiteOrigin>,
    dev_origin: Option<SiteOrigin>,
}

impl UrlPolicy {
    /// Builds the policy from the start-up configuration. A site that is not configured means
    /// no address counts as the site.
    pub fn new(config: &LauncherConfig) -> Self {
        UrlPolicy {
            site: config.site.as_ref().ok().cloned(),
            dev_origin: config.dev_origin.clone(),
        }
    }

    /// Classifies an address.
    pub fn origin_of(&self, url: &str) -> PageOrigin {
        let Ok(url) = Url::parse(url) else {
            return PageOrigin::Other;
        };
        let is_site =
            |origin: &Option<SiteOrigin>| origin.as_ref().is_some_and(|o| o.matches(&url));
        if is_site(&self.site) || is_site(&self.dev_origin) {
            PageOrigin::Site
        } else if is_local_page(&url) {
            PageOrigin::Local
        } else {
            PageOrigin::Other
        }
    }

    /// The origins a page may have to use the bridge, as `location.origin` shows them: the site,
    /// the development origin and the bundled pages. Used only for the convenience check in
    /// `bridge.js`; [`origin_of`](Self::origin_of) decides.
    pub fn bridge_origins(&self) -> Vec<String> {
        let mut origins: Vec<String> = [&self.site, &self.dev_origin]
            .into_iter()
            .flatten()
            .map(SiteOrigin::origin_text)
            .collect();
        origins.extend(
            [
                "http://metallic-app.localhost",
                "https://metallic-app.localhost",
                "metallic-app://local",
            ]
            .map(String::from),
        );
        origins
    }

    /// Decides a navigation of the main view to `url`.
    pub fn navigation(&self, url: &str) -> NavDecision {
        if url == "about:blank" {
            return NavDecision::Allow;
        }
        if self.origin_of(url) != PageOrigin::Other {
            return NavDecision::Allow;
        }
        let Ok(parsed) = Url::parse(url) else {
            return NavDecision::Block;
        };
        match parsed.scheme() {
            SCHEME => match DeepLink::parse(url) {
                Ok(link) => NavDecision::HandleDeepLink(link),
                Err(error) => {
                    log::warn!("blocked a bad metallic:// link: {error}");
                    NavDecision::Block
                }
            },
            "https" => {
                // Drop user info so a fake `https://metallic.example@evil.test/` prefix is not passed on.
                let mut parsed = parsed;
                let _ = parsed.set_username("");
                let _ = parsed.set_password(None);
                NavDecision::OpenInBrowser(parsed)
            }
            _ => NavDecision::Block,
        }
    }

    /// Decides a navigation of a frame (an `iframe`) inside a page: true to let it load. Only
    /// what [`navigation`](Self::navigation) allows loads; every other address is refused and is
    /// never sent to the system browser, because a frame is not something the user clicked.
    pub fn frame_navigation(&self, url: &str) -> bool {
        self.navigation(url) == NavDecision::Allow
    }

    /// Decides a request to open `url` in a new window. The site and bundled pages are
    /// `Allow`, meaning "load it in the main view"; `about:blank` is refused here, because a
    /// pop-up has no use for it; the rest is decided as in [`navigation`](Self::navigation).
    pub fn new_window(&self, url: &str) -> NavDecision {
        if url == "about:blank" {
            return NavDecision::Block;
        }
        self.navigation(url)
    }
}
