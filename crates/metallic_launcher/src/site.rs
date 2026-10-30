//! Exact origins, bundled-page recognition and command-line isolation.

use url::Url;

use crate::deep_link::is_metallic;
use crate::{DeepLink, LauncherError, LauncherSettings, MAX_URI_LEN, SitePath};

/// Protocol used for bundled, capability-limited pages.
pub const LOCAL_SCHEME: &str = "metallic-app";
const MAX_ARGS: usize = 64;

/// An HTTPS origin, or an explicitly enabled HTTP loopback origin for development.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteOrigin {
    base: Url,
}

impl SiteOrigin {
    /// Validates an origin only: no credentials, path beyond `/`, query or fragment.
    pub fn parse(text: &str, allow_loopback_http: bool) -> Result<Self, LauncherError> {
        let bad = || {
            LauncherError::BadOrigin(
                "Use an HTTPS origin, or HTTP loopback in developer mode.".into(),
            )
        };
        if text.len() > MAX_URI_LEN {
            return Err(bad());
        }
        if text.trim().is_empty() || text.trim() == "REPLACE_WITH_SITE_ORIGIN" {
            return Err(LauncherError::NotConfigured);
        }
        // Validate the origin shape before WHATWG normalization can erase dot segments.
        let Some((_, authority_and_path)) = text.split_once("://") else {
            return Err(bad());
        };
        if authority_and_path
            .find('/')
            .is_some_and(|start| &authority_and_path[start..] != "/")
        {
            return Err(bad());
        }
        if text.contains(['\\', '@']) || text.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(bad());
        }
        let base = Url::parse(text).map_err(|_| bad())?;
        let permitted = base.scheme() == "https"
            || (allow_loopback_http
                && base.scheme() == "http"
                && matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")));
        if !permitted
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || !matches!(base.path(), "" | "/")
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(bad());
        }
        Ok(Self { base })
    }

    /// Compares scheme, canonical host and effective port, refusing credential-bearing URLs.
    pub fn matches(&self, url: &Url) -> bool {
        url.username().is_empty()
            && url.password().is_none()
            && self.base.scheme() == url.scheme()
            && self.base.host_str() == url.host_str()
            && self.base.port_or_known_default() == url.port_or_known_default()
    }

    /// Builds a URL by replacing only path/query/fragment, so a path cannot change the origin.
    pub fn url(&self, path: &SitePath) -> Url {
        let mut result = self.base.clone();
        let (before_fragment, fragment) = path
            .as_str()
            .split_once('#')
            .map_or((path.as_str(), None), |(path, fragment)| {
                (path, Some(fragment))
            });
        let (path, query) = before_fragment
            .split_once('?')
            .map_or((before_fragment, None), |(path, query)| (path, Some(query)));
        result.set_path(path);
        result.set_query(query);
        result.set_fragment(fragment);
        result
    }
}

/// Reads the compiled production origin. Placeholder builds remain explicitly unconfigured.
pub fn embedded_site_origin() -> Result<SiteOrigin, LauncherError> {
    SiteOrigin::parse(
        include_str!("../../../config/launcher_site_origin.txt").trim(),
        false,
    )
}

/// Recognizes only the bundled protocol host or WebView2's exact default-port mapping.
pub fn is_local_page(url: &Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match (url.scheme(), url.host_str(), url.port_or_known_default()) {
        (LOCAL_SCHEME, Some("local"), None) => true,
        ("http", Some("metallic-app.localhost"), Some(80)) => true,
        ("https", Some("metallic-app.localhost"), Some(443)) => true,
        _ => false,
    }
}

/// Parsed startup configuration; arguments exclude the executable name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherConfig {
    /// Production origin, including an explicit NotConfigured result for placeholder builds.
    pub site: Result<SiteOrigin, LauncherError>,
    /// Explicit developer origin; used only when developer mode is enabled.
    pub dev_origin: Option<SiteOrigin>,
    /// Whether developer capabilities are enabled.
    pub dev_mode: bool,
    /// Protocol action requested at startup.
    pub start_link: Option<DeepLink>,
    /// Start without showing the window, for local command-line use.
    pub start_hidden: bool,
    /// Test hook requesting exit after the initial page loads.
    pub exit_after_load: bool,
}

impl LauncherConfig {
    /// Parses bounded arguments. A `metallic:` argument at any position suppresses every flag
    /// and developer setting; malformed protocol input fails closed. Arguments after it are ignored.
    pub fn from_args(
        args: impl IntoIterator<Item = String>,
        settings: &LauncherSettings,
    ) -> Result<Self, LauncherError> {
        let mut collected = Vec::new();
        for argument in args {
            if is_metallic(&argument) {
                return Ok(Self {
                    site: embedded_site_origin(),
                    dev_origin: None,
                    dev_mode: false,
                    start_link: Some(DeepLink::parse(&argument)?),
                    start_hidden: false,
                    exit_after_load: false,
                });
            }
            if collected.len() >= MAX_ARGS || argument.len() > MAX_URI_LEN {
                return Err(LauncherError::BadArguments(
                    "too many or oversized arguments",
                ));
            }
            collected.push(argument);
        }
        let mut dev_mode = settings.developer_mode;
        let mut dev_url = None;
        let mut start_hidden = false;
        let mut exit_after_load = false;
        let mut arguments = collected.iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--dev" => dev_mode = true,
                "--hidden" => start_hidden = true,
                "--exit-after-load" => exit_after_load = true,
                "--dev-url" => {
                    if dev_url.is_some() {
                        return Err(LauncherError::BadArguments("duplicate developer URL"));
                    }
                    dev_url = Some(
                        arguments
                            .next()
                            .ok_or(LauncherError::BadArguments("developer URL missing"))?
                            .as_str(),
                    );
                }
                _ => return Err(LauncherError::BadArguments("unrecognized argument")),
            }
        }
        if dev_url.is_some() && !dev_mode {
            return Err(LauncherError::BadArguments(
                "developer URL requires developer mode",
            ));
        }
        let dev_origin = if dev_mode {
            dev_url
                .or(settings.dev_url.as_deref())
                .map(|url| SiteOrigin::parse(url, true))
                .transpose()?
        } else {
            None
        };
        Ok(Self {
            site: embedded_site_origin(),
            dev_origin,
            dev_mode,
            start_link: None,
            start_hidden,
            exit_after_load,
        })
    }
}
