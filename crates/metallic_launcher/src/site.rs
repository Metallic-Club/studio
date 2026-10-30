//! The site origin the Launcher shows, the bundled-page address, and the start-up arguments.

use url::Url;

use crate::deep_link::{DeepLink, SitePath};
use crate::error::LauncherError;
use crate::settings::LauncherSettings;

/// The placeholder line in `config/launcher_site_origin.txt` until the site address is known.
pub const SITE_ORIGIN_PLACEHOLDER: &str = "REPLACE_WITH_SITE_ORIGIN";

/// The scheme of the pages bundled into the Launcher (offline page, settings).
pub const LOCAL_SCHEME: &str = "metallic-app";

/// The host of a bundled page where the platform maps the scheme to `http(s)://`
/// (Windows WebView2 serves `metallic-app://local/x` as `http://metallic-app.localhost/x`).
const LOCAL_MAPPED_HOST: &str = "metallic-app.localhost";

/// A web origin: scheme, host and port. Only `https`, or loopback `http` for development.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteOrigin {
    scheme: String,
    host: String,
    port: u16,
    base: Url,
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

impl SiteOrigin {
    /// Reads an origin such as `https://metallic.example`: `https` with a host, no user info,
    /// path `/` or empty, no query or fragment. With `allow_loopback_http`, also `http` on
    /// `localhost`, `127.0.0.1` or `[::1]`. The placeholder or empty text is `NotConfigured`.
    pub fn parse(text: &str, allow_loopback_http: bool) -> Result<SiteOrigin, LauncherError> {
        let text = text.trim();
        if text.is_empty() || text == SITE_ORIGIN_PLACEHOLDER {
            return Err(LauncherError::NotConfigured);
        }
        let bad = |why: &str| LauncherError::BadOrigin(why.to_string());
        let url = Url::parse(text).map_err(|_| bad("not a valid address"))?;
        let host = url.host_str().ok_or_else(|| bad("no host"))?.to_string();
        if !url.username().is_empty() || url.password().is_some() {
            return Err(bad("user info is not allowed"));
        }
        if !(url.path().is_empty() || url.path() == "/") {
            return Err(bad("a path is not allowed"));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(bad("a query or fragment is not allowed"));
        }
        match url.scheme() {
            "https" => {}
            "http" if allow_loopback_http && is_loopback_host(&host) => {}
            "http" => return Err(bad("plain http is only allowed to a loopback host")),
            _ => return Err(bad("the scheme must be https")),
        }
        let port = url.port_or_known_default().ok_or_else(|| bad("no port"))?;
        Ok(SiteOrigin {
            scheme: url.scheme().to_string(),
            host,
            port,
            base: url,
        })
    }

    /// True when `url` has this origin: same scheme, host and effective port, and no user info.
    pub fn matches(&self, url: &Url) -> bool {
        url.scheme() == self.scheme
            && url.host_str() == Some(self.host.as_str())
            && url.port_or_known_default() == Some(self.port)
            && url.username().is_empty()
            && url.password().is_none()
    }

    /// The address of `path` on this origin.
    pub fn url(&self, path: &SitePath) -> Url {
        // A SitePath starts with a single `/` and has no `\`, so the join cannot leave the origin.
        self.base
            .join(path.as_str())
            .unwrap_or_else(|_| self.base.clone())
    }
}

/// The site origin compiled in from `config/launcher_site_origin.txt`.
pub fn embedded_site_origin() -> Result<SiteOrigin, LauncherError> {
    SiteOrigin::parse(
        include_str!("../../../config/launcher_site_origin.txt"),
        false,
    )
}

/// True for a page bundled into the Launcher: `metallic-app://local/...`, or its
/// `http(s)://metallic-app.localhost/...` mapping on Windows.
pub fn is_local_page(url: &Url) -> bool {
    // Exact match: no port (the default port parses to none), no user info.
    if url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        LOCAL_SCHEME => url.host_str() == Some("local"),
        "http" | "https" => url.host_str() == Some(LOCAL_MAPPED_HOST),
        _ => false,
    }
}

/// How this run of the Launcher was asked to start.
#[derive(Debug)]
pub struct LauncherConfig {
    /// The Metallic site; an error (usually `NotConfigured`) makes the Launcher show the offline page.
    pub site: Result<SiteOrigin, LauncherError>,
    /// A development server that is trusted like the site (developer mode only).
    pub dev_origin: Option<SiteOrigin>,
    /// True in developer mode.
    pub dev_mode: bool,
    /// The link the Launcher was started with, if any.
    pub start_link: Option<DeepLink>,
    /// Start without showing the window (autostart).
    pub start_hidden: bool,
    /// Exit once the first page has loaded (test hook).
    pub exit_after_load: bool,
}

impl LauncherConfig {
    /// Reads the command line (without the program name).
    ///
    /// Flags: `--dev`, `--dev-url <url>` (needs developer mode from `--dev` or the settings),
    /// `--hidden`, `--exit-after-load`. **If the first argument starts with `metallic:` (any
    /// case), every other argument is ignored**: the protocol handler passes the link only, and
    /// a hostile link must not be able to add flags.
    pub fn from_args(
        args: impl IntoIterator<Item = String>,
        settings: &LauncherSettings,
    ) -> Result<LauncherConfig, LauncherError> {
        let mut config = LauncherConfig {
            site: embedded_site_origin(),
            dev_origin: None,
            dev_mode: false,
            start_link: None,
            start_hidden: false,
            exit_after_load: false,
        };
        let args: Vec<String> = args.into_iter().collect();
        if let Some(first) = args.first()
            && first
                .get(..SCHEME_PREFIX.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(SCHEME_PREFIX))
        {
            config.start_link = Some(DeepLink::parse(first)?);
            return Ok(config);
        }

        let mut dev_flag = false;
        let mut dev_url: Option<String> = None;
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--dev" => dev_flag = true,
                "--dev-url" => {
                    dev_url = Some(iter.next().ok_or_else(|| {
                        LauncherError::BadOrigin("--dev-url needs an address".to_string())
                    })?);
                }
                "--hidden" => config.start_hidden = true,
                "--exit-after-load" => config.exit_after_load = true,
                _ => log::warn!("ignoring an unknown start-up argument"),
            }
        }
        config.dev_mode = dev_flag || settings.developer_mode;
        match dev_url {
            Some(text) if config.dev_mode => {
                config.dev_origin = Some(SiteOrigin::parse(&text, true)?);
            }
            Some(_) => {
                return Err(LauncherError::BadOrigin(
                    "--dev-url needs developer mode".to_string(),
                ));
            }
            None if config.dev_mode => {
                if let Some(text) = &settings.dev_url {
                    match SiteOrigin::parse(text, true) {
                        Ok(origin) => config.dev_origin = Some(origin),
                        Err(error) => log::warn!("ignoring the saved development address: {error}"),
                    }
                }
            }
            None => {}
        }
        Ok(config)
    }
}

const SCHEME_PREFIX: &str = "metallic:";
