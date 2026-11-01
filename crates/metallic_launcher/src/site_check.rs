//! Is the site up, and which page should the window start on.

use metallic_update::HttpClient;
use url::Url;

use crate::deep_link::SitePath;
use crate::local_pages::{LocalPage, OfflineReason};
use crate::site::{LauncherConfig, SiteOrigin};

/// Whether the site answered its health check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteStatus {
    /// `GET /healthz` answered with a 2xx status.
    Up,
    /// Any other answer, or no answer.
    Down,
}

/// Asks the site `GET <origin>/healthz`. A 2xx answer is [`SiteStatus::Up`]; another status or a
/// network failure is [`SiteStatus::Down`]. The 5 second limit belongs to `http` (the shell
/// builds its client with it).
pub fn check_site(http: &dyn HttpClient, origin: &SiteOrigin) -> SiteStatus {
    let Ok(path) = SitePath::new("/healthz") else {
        return SiteStatus::Down;
    };
    match http.get(origin.url(&path).as_str(), &[]) {
        Ok(response) if (200..300).contains(&response.status) => SiteStatus::Up,
        _ => SiteStatus::Down,
    }
}

/// The first page of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartPage {
    /// Load this address from the web.
    Site(Url),
    /// Show a bundled page.
    Local(LocalPage),
}

/// Chooses the first page. `status` is the result of a site check, if one has finished.
///
/// No site address and not developer mode: the offline page (not configured). Developer mode with
/// a development address: that address. Developer mode without one: the demo page. Otherwise the
/// site, or the offline page (unreachable) when the check said `Down`.
pub fn start_page(config: &LauncherConfig, status: Option<SiteStatus>) -> StartPage {
    let root = |origin: &SiteOrigin| SitePath::new("/").map(|p| origin.url(&p));
    if config.dev_mode {
        return match config.dev_origin.as_ref().map(root) {
            Some(Ok(url)) => StartPage::Site(url),
            _ => StartPage::Local(LocalPage::Demo),
        };
    }
    match config.site.as_ref().map(root) {
        Ok(Ok(url)) => match status {
            Some(SiteStatus::Down) => {
                StartPage::Local(LocalPage::Offline(OfflineReason::Unreachable))
            }
            _ => StartPage::Site(url),
        },
        _ => StartPage::Local(LocalPage::Offline(OfflineReason::NotConfigured)),
    }
}

/// The User-Agent the web view sends: an Edge-style string for `webview_version` (for example
/// `130.0.2849.80`) followed by ` MetallicLauncher/<launcher_version> bridge/1`, so the site can
/// tell it runs inside the Launcher. This is a hint for page layout, not a security check.
pub fn user_agent(webview_version: &str, launcher_version: &str) -> String {
    let major = webview_version.split('.').next().unwrap_or("0");
    format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
         Chrome/{major}.0.0.0 Safari/537.36 Edg/{webview_version} \
         MetallicLauncher/{launcher_version} bridge/1"
    )
}
