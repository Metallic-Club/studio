//! Pure navigation policy; only exact site/local origins carry capabilities.

use crate::deep_link::is_metallic;
use crate::site::is_local_page;
use crate::{DeepLink, LauncherConfig, SiteOrigin};
use url::Url;

/// Maximum navigation input size, checked before URL parsing.
pub const MAX_URL_LEN: usize = 16 * 1024;

/// Capability category of a page's actual URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageOrigin {
    /// Exact configured production/developer origin.
    Site,
    /// Bundled, capability-limited page.
    Local,
    /// Any other page, including about:blank; grants no capabilities.
    Other,
}

/// What the web-view host should do with a navigation or new-window request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavDecision {
    /// Load in the main view.
    Allow,
    /// Open an external HTTPS URL in the system browser.
    OpenInBrowser(Url),
    /// Dispatch a validated protocol action.
    HandleDeepLink(DeepLink),
    /// Refuse the request.
    Block,
}

/// Origin and navigation policy with no web-view dependency.
#[derive(Debug, Clone)]
pub struct UrlPolicy {
    site: Option<SiteOrigin>,
    dev_origin: Option<SiteOrigin>,
}

fn parse_url(text: &str) -> Option<Url> {
    if text.len() > MAX_URL_LEN
        || text.contains('\\')
        || text.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return None;
    }
    let url = Url::parse(text).ok()?;
    (url.username().is_empty() && url.password().is_none()).then_some(url)
}

impl UrlPolicy {
    /// Builds a policy; an accidentally populated dev origin is ignored outside developer mode.
    pub fn new(config: &LauncherConfig) -> Self {
        Self {
            site: config.site.as_ref().ok().cloned(),
            dev_origin: config.dev_mode.then(|| config.dev_origin.clone()).flatten(),
        }
    }

    fn parsed_origin(&self, url: &Url) -> PageOrigin {
        if self.site.as_ref().is_some_and(|site| site.matches(url))
            || self
                .dev_origin
                .as_ref()
                .is_some_and(|site| site.matches(url))
        {
            PageOrigin::Site
        } else if is_local_page(url) {
            PageOrigin::Local
        } else {
            PageOrigin::Other
        }
    }

    /// Classifies the actual sending page URL; malformed/oversized input grants no capability.
    pub fn origin_of(&self, text: &str) -> PageOrigin {
        parse_url(text).map_or(PageOrigin::Other, |url| self.parsed_origin(&url))
    }

    /// Allows only site/local/about:blank in the view, dispatches deep links, and externalizes HTTPS.
    pub fn navigation(&self, text: &str) -> NavDecision {
        if is_metallic(text) {
            return DeepLink::parse(text).map_or(NavDecision::Block, NavDecision::HandleDeepLink);
        }
        if text == "about:blank" {
            return NavDecision::Allow;
        }
        let Some(url) = parse_url(text) else {
            return NavDecision::Block;
        };
        match self.parsed_origin(&url) {
            PageOrigin::Site | PageOrigin::Local => NavDecision::Allow,
            PageOrigin::Other if url.scheme() == "https" => NavDecision::OpenInBrowser(url),
            PageOrigin::Other => NavDecision::Block,
        }
    }

    /// Applies the same policy to a popup; Allow means navigate the main view instead.
    pub fn new_window(&self, text: &str) -> NavDecision {
        self.navigation(text)
    }
}
