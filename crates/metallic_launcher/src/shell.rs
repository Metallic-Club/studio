//! The window and the web view, and the event loop that drives them. This is thin glue: what to
//! show and what to allow is decided by the library modules (`start_page`, `UrlPolicy`, `serve`).
//!
//! Other parts of the Launcher never touch the web view. They post [`AppEvent`]s through the
//! [`EventSink`] this module builds on the event loop.

use std::borrow::Cow;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use metallic_launcher::deep_link::SitePath;
use metallic_launcher::platform_auth::{KEYRING_ACCOUNT, KEYRING_SERVICE};
use metallic_launcher::{
    AppEvent, BRIDGE_VERSION, Bridge, BridgeErrorCode, BridgeEvent, BridgeHost, CloseAction,
    DeepLink, EventSink, GithubAssets, HandoffListener, HandoffMessage, InstallStatus, Installer,
    LauncherConfig, LauncherError, LauncherInfo, LauncherSettings, LocalPage, NavDecision,
    OfflineReason, OneAtATime, PageOrigin, PlaceId, PlatformAuth, PlayerWatcher, ProcessTracker,
    RestartError, SelfUpdater, SettingsPatch, SettingsStore, SiteOrigin, SiteStatus, StartPage,
    TrayAction, UrlPolicy, check_site, downloads_dir_for, github_sign_in, is_installed_copy,
    local_url, on_close_requested, platform_sign_in, player_status, product_paths, serve,
    start_page, start_player, start_studio, studio_root_for, tray_menu, user_agent,
};
use metallic_update::github_auth::{StoredTokenProvider, embedded_client_id};
use metallic_update::{
    GithubAuth, HttpClient, KeyringSecretStore, LAUNCHER_FEED, Token, TokenProvider, UpdateChecker,
    UreqClient,
};
use serde_json::{Value, json};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Icon, WindowBuilder};
use url::Url;
use wry::http::{Method, Request, Response};
use wry::{
    NewWindowResponse, PageLoadEvent, PermissionResponse, WebContext, WebView, WebViewBuilder,
};

use crate::platform::{self, SystemSpawner};

/// The graphite page background (`--bg-base` in the visual language), so no white shows before
/// the first page paints.
const BACKGROUND: (u8, u8, u8, u8) = (22, 21, 27, 255);

/// How long the site check may take (the health check and the offline re-checks).
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a request of the Metallic sign-in may take.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(8);
/// The first wait between re-checks while offline, and the longest.
const RECHECK_FIRST: Duration = Duration::from_secs(10);
const RECHECK_LONGEST: Duration = Duration::from_secs(60);
/// The window appears at the latest this long after start, even if no page has loaded.
const SHOW_AT_LATEST: Duration = Duration::from_millis(2500);
/// With `--exit-after-load`, the Launcher waits this long after the first load so the page's own
/// start-up requests can finish.
const EXIT_GRACE: Duration = Duration::from_millis(1500);

/// The event loop's proxy as an [`EventSink`].
struct ProxySink(Mutex<EventLoopProxy<AppEvent>>);

impl EventSink for ProxySink {
    fn send(&self, event: AppEvent) {
        if let Ok(proxy) = self.0.lock() {
            // The loop has already closed when this fails; there is nothing left to tell.
            let _ = proxy.send_event(event);
        }
    }
}

/// State the web view's callbacks and the check threads share.
struct Shared {
    /// The bundled offline page is on screen.
    offline: AtomicBool,
    /// The site (not a bundled page) has finished loading at least once since the last navigation.
    site_loaded: AtomicBool,
    /// The window has been shown.
    shown: AtomicBool,
}

/// The GitHub token for the private team feed: none until the user signs in.
struct NoToken;

impl TokenProvider for NoToken {
    fn token(&self) -> Option<Token> {
        None
    }
}

/// What the bridge can do in this build. Playing and opening Studio start the installed
/// programs; installing Studio and updating the Launcher are in `install` and `self_update`;
/// the Metallic sign-in is in `platform_auth` (`notAvailable` while the site address is not set).
struct ShellHost {
    dev_mode: bool,
    settings: Mutex<LauncherSettings>,
    store: SettingsStore,
    http: Arc<dyn HttpClient>,
    check_origin: Option<SiteOrigin>,
    sink: Arc<dyn EventSink>,
    installer: Arc<Installer>,
    updater: Arc<SelfUpdater>,
    players: Arc<PlayerWatcher>,
    /// GitHub sign-in for team builds; `None` until the client id is configured.
    github: Option<Arc<GithubAuth>>,
    /// Metallic sign-in; `None` while the site address is not configured.
    platform: Option<Arc<PlatformAuth>>,
    /// One Metallic sign-in at a time.
    platform_flight: OneAtATime,
    /// One GitHub sign-in at a time.
    github_flight: OneAtATime,
}

/// What a sign-in start becomes on the bridge: `busy` while one runs, else `failed`.
fn sign_in_code(error: LauncherError) -> BridgeErrorCode {
    match error {
        LauncherError::Busy(_) => BridgeErrorCode::Busy,
        _ => BridgeErrorCode::Failed,
    }
}

impl BridgeHost for ShellHost {
    fn info(&self) -> LauncherInfo {
        LauncherInfo {
            launcher_version: env!("CARGO_PKG_VERSION").to_string(),
            bridge_version: BRIDGE_VERSION,
            platform: "windows",
            channel: self.settings().channel.as_str().to_string(),
            dev_mode: self.dev_mode,
        }
    }

    fn play(&self, link: DeepLink) -> Result<(), BridgeErrorCode> {
        start_the_player(&link, &self.updater).map_err(|error| match error {
            LauncherError::NotInstalled(_) => BridgeErrorCode::NotInstalled,
            LauncherError::UpdateRequired => BridgeErrorCode::UpdateRequired,
            _ => BridgeErrorCode::Failed,
        })
    }

    fn install_status(&self) -> Value {
        let status = InstallStatus {
            player: player_status(&product_paths()),
            studio: self.installer.studio_status(),
            launcher: self.updater.state(),
        };
        serde_json::to_value(status).unwrap_or_else(|_| json!({}))
    }

    fn install_studio(&self) -> Result<(), BridgeErrorCode> {
        self.installer
            .start_install_studio(self.sink.clone())
            .map_err(|error| match error {
                LauncherError::Busy(_) => BridgeErrorCode::Busy,
                _ => BridgeErrorCode::Failed,
            })
    }

    fn open_studio(&self, place: Option<PlaceId>) -> Result<(), BridgeErrorCode> {
        start_the_studio(place).map_err(|error| match error {
            LauncherError::NotInstalled(_) => BridgeErrorCode::NotInstalled,
            _ => BridgeErrorCode::Failed,
        })
    }

    fn settings(&self) -> LauncherSettings {
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    fn update_settings(&self, patch: SettingsPatch) -> Result<LauncherSettings, BridgeErrorCode> {
        let mut current = self.settings.lock().map_err(|_| BridgeErrorCode::Failed)?;
        let mut changed = current.clone();
        changed
            .apply(patch)
            .map_err(|_| BridgeErrorCode::BadRequest)?;
        self.store.save(&changed).map_err(|error| {
            log::warn!("could not save the settings: {error}");
            BridgeErrorCode::Failed
        })?;
        *current = changed.clone();
        Ok(changed)
    }

    fn sign_in(&self) -> Result<Value, BridgeErrorCode> {
        let Some(auth) = self.platform.clone() else {
            return Err(BridgeErrorCode::NotAvailable);
        };
        let started = platform_sign_in(
            auth,
            self.sink.clone(),
            Arc::new(|address| {
                if let Err(error) = platform::open_in_browser(address) {
                    log::warn!("could not open the sign-in page: {error}");
                }
            }),
            &self.platform_flight,
        )
        .map_err(sign_in_code)?;
        Ok(json!({
            "userCode": started.user_code,
            "verificationUri": started.verification_uri,
        }))
    }

    fn sign_out(&self) -> Result<(), BridgeErrorCode> {
        let Some(auth) = &self.platform else {
            return Err(BridgeErrorCode::NotAvailable);
        };
        self.platform_flight.cancel_running();
        auth.sign_out().map_err(|error| {
            log::warn!("could not sign out: {error}");
            BridgeErrorCode::Failed
        })
    }

    fn github_sign_in(&self) -> Result<Value, BridgeErrorCode> {
        let Some(auth) = self.github.clone() else {
            return Err(BridgeErrorCode::NotAvailable);
        };
        let updater = self.updater.clone();
        github_sign_in(
            auth,
            self.sink.clone(),
            Arc::new(|address| {
                if let Err(error) = metallic_update::github_auth::open_in_browser(address) {
                    log::warn!("could not open the GitHub page: {error}");
                }
            }),
            &self.github_flight,
            move |signed_in| {
                if signed_in {
                    updater.check_soon();
                }
            },
        )
        .map_err(sign_in_code)?;
        Ok(json!({ "started": true }))
    }

    fn retry_site(&self) {
        match &self.check_origin {
            Some(origin) => spawn_check(self.http.clone(), origin.clone(), self.sink.clone()),
            None => self.sink.send(AppEvent::SiteStatus(SiteStatus::Down)),
        }
    }

    fn restart_to_update(&self) -> Result<(), BridgeErrorCode> {
        self.updater
            .restart_to_update(self.players.running_players())
            .map_err(|error| match error {
                RestartError::Busy => BridgeErrorCode::Busy,
                RestartError::NotReady | RestartError::Failed => BridgeErrorCode::Failed,
            })
    }
}

/// Starts the Player for a Play link and logs the outcome (never the link: it holds a ticket).
/// Refused while a required Launcher update waits.
fn start_the_player(link: &DeepLink, updater: &SelfUpdater) -> Result<(), LauncherError> {
    updater.allow_play()?;
    match start_player(&SystemSpawner, &product_paths(), link) {
        Ok(pid) => {
            log::info!("started the Player (process {pid})");
            Ok(())
        }
        Err(error) => {
            log::warn!("could not start the Player: {error}");
            Err(error)
        }
    }
}

/// Starts Metallic Studio. It does not take a place yet, so a place is ignored (and logged).
fn start_the_studio(place: Option<PlaceId>) -> Result<(), LauncherError> {
    if place.is_some() {
        log::info!("Studio does not take a place yet; the place is ignored");
    }
    match start_studio(&SystemSpawner, &product_paths()) {
        Ok(pid) => {
            log::info!("started Studio (process {pid})");
            Ok(())
        }
        Err(error) => {
            log::warn!("could not start Studio: {error}");
            Err(error)
        }
    }
}

/// True when both addresses are the same document (a `#fragment` does not make a new page).
fn same_document(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(mut a), Ok(mut b)) => {
            a.set_fragment(None);
            b.set_fragment(None);
            a == b
        }
        _ => false,
    }
}

/// Runs a script in the view, logging a failure.
fn run_script(view: &WebView, script: &str) {
    if let Err(error) = view.evaluate_script(script) {
        log::warn!("could not run a script in the page: {error}");
    }
}

/// Handles one message from a page: checks it, runs it and gives the answer back, but only to the
/// page that asked. A message that does not come from the page the view shows now (a frame, or a
/// page that has since navigated) gets no answer.
fn handle_bridge_message(
    view: &WebView,
    bridge: &Bridge,
    host: &ShellHost,
    page_url: &str,
    body: &str,
) {
    let Ok(shown) = view.url() else { return };
    if !same_document(page_url, &shown) {
        log::warn!("dropped a bridge message that did not come from the page on screen");
        return;
    }
    let Some(reply) = bridge.handle(page_url, body, host) else {
        return;
    };
    let Ok(shown) = view.url() else { return };
    if let Some(script) = bridge.deliver(reply, &shown) {
        run_script(view, &script);
    }
}

/// Tells the page on screen about `event`, if it may hear of it.
fn send_bridge_event(view: &WebView, bridge: &Bridge, event: &BridgeEvent) {
    let Ok(shown) = view.url() else { return };
    if let Some(script) = bridge.event_script(event, &shown) {
        run_script(view, &script);
    }
}

/// Runs a script the shell itself wants in the page, with the same in-page origin guard as a
/// bridge reply: it does nothing unless the page on screen is the site or a bundled page, and
/// the page re-checks its `location.origin` before the script runs.
fn run_guarded_script(view: &WebView, bridge: &Bridge, script: &str) {
    let Ok(shown) = view.url() else { return };
    match bridge.guard_script(script, &shown) {
        Some(guarded) => run_script(view, &guarded),
        None => log::info!("dropped a script: the page on screen may not receive it"),
    }
}

/// Decodes the placeholder icon from `branding/` to RGBA pixels: (pixels, width, height).
fn icon_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let bytes = include_bytes!("../../../branding/placeholder-icon.png");
    let mut decoder = png::Decoder::new(Cursor::new(&bytes[..]));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(info.buffer_size());
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => buffer,
        (png::ColorType::Rgb, png::BitDepth::Eight) => buffer
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => return None,
    };
    Some((rgba, info.width, info.height))
}

/// The window icon.
fn window_icon() -> Option<Icon> {
    let (rgba, width, height) = icon_rgba()?;
    Icon::from_rgba(rgba, width, height).ok()
}

/// Carries out a `metallic://` link in the first Launcher. Links from the web view and from
/// other starts have been parsed before they get here.
fn handle_deep_link(
    link: DeepLink,
    site: Option<&SiteOrigin>,
    offline: bool,
    sink: &dyn EventSink,
    updater: &SelfUpdater,
) {
    // The Install flow: show the window and the downloads page, or the local page when offline.
    let show_downloads = || {
        sink.send(AppEvent::Focus);
        match (site, offline) {
            (Some(site), false) => match SitePath::new("/downloads") {
                Ok(path) => sink.send(AppEvent::Navigate(site.url(&path))),
                Err(_) => sink.send(AppEvent::ShowLocal(LocalPage::Settings)),
            },
            _ => sink.send(AppEvent::ShowLocal(LocalPage::Settings)),
        }
    };
    match &link {
        DeepLink::Play { .. } => {
            match start_the_player(&link, updater) {
                Err(LauncherError::NotInstalled(_)) => show_downloads(),
                // The site shows the update state; bring the window up so the user sees it.
                Err(LauncherError::UpdateRequired) => sink.send(AppEvent::Focus),
                _ => {}
            }
        }
        DeepLink::StudioOpen { place } => {
            if let Err(LauncherError::NotInstalled(_)) = start_the_studio(*place) {
                show_downloads();
            }
        }
        DeepLink::Open { path } => {
            sink.send(AppEvent::Focus);
            let Some(site) = site else { return };
            let target = match path {
                Some(path) => site.url(path),
                None => match SitePath::new("/") {
                    Ok(root) => site.url(&root),
                    Err(_) => return,
                },
            };
            sink.send(AppEvent::Navigate(target));
        }
        DeepLink::Install => show_downloads(),
    }
}

/// Reads hand-overs from later starts and posts them to the window. A link is parsed again
/// here; a bad one is logged (without its text) and dropped.
fn spawn_handoff_listener(mut listener: Box<dyn HandoffListener>, sink: Arc<dyn EventSink>) {
    std::thread::spawn(move || {
        let mut failures = 0;
        loop {
            match listener.accept() {
                Ok(message) => {
                    failures = 0;
                    match message {
                        HandoffMessage::Focus => sink.send(AppEvent::Focus),
                        HandoffMessage::Link(text) => match DeepLink::parse(&text) {
                            Ok(link) => sink.send(AppEvent::DeepLink(link)),
                            Err(error) => {
                                log::warn!("dropped a link from another start: {error}");
                            }
                        },
                    }
                }
                // A client that misbehaved (left, stayed silent, sent garbage) is not the pipe's
                // fault: the pipe has been reset, so keep listening and do not count it.
                Err(error @ LauncherError::BadHandoff(_)) => {
                    log::warn!("a hand-over from another start was refused: {error}");
                }
                Err(error) => {
                    log::warn!("a hand-over from another start failed: {error}");
                    failures += 1;
                    if failures >= 20 {
                        log::error!("giving up on hand-overs from other starts");
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
}

/// What the web view's address should be for a start page or a bundled page.
fn local_address(page: &LocalPage) -> String {
    platform::webview_local_url(&local_url(page)).to_string()
}

/// Runs one site check on a new thread and reports the result to the sink.
fn spawn_check(http: Arc<dyn HttpClient>, origin: SiteOrigin, sink: Arc<dyn EventSink>) {
    std::thread::spawn(move || {
        let status = check_site(http.as_ref(), &origin);
        sink.send(AppEvent::SiteStatus(status));
    });
}

/// While the offline page is up, checks the site every 10 seconds, doubling the wait up to 60.
fn spawn_recheck_loop(
    http: Arc<dyn HttpClient>,
    origin: SiteOrigin,
    sink: Arc<dyn EventSink>,
    shared: Arc<Shared>,
) {
    std::thread::spawn(move || {
        let mut wait = RECHECK_FIRST;
        loop {
            // Sleep in short steps so the wait starts over once the page is no longer offline.
            let mut slept = Duration::ZERO;
            while slept < wait {
                std::thread::sleep(Duration::from_secs(1));
                slept += Duration::from_secs(1);
                if !shared.offline.load(Ordering::SeqCst) {
                    wait = RECHECK_FIRST;
                    slept = Duration::ZERO;
                }
            }
            let status = check_site(http.as_ref(), &origin);
            sink.send(AppEvent::SiteStatus(status));
            wait = (wait * 2).min(RECHECK_LONGEST);
        }
    });
}

/// The answer of the `metallic-app` protocol: GET and HEAD are served, other methods get a 405.
fn protocol_response(request: Request<Vec<u8>>, dev_mode: bool) -> Response<Cow<'static, [u8]>> {
    let method = request.method().clone();
    let (status, answer) = if matches!(method, Method::GET | Method::HEAD) {
        let path = request.uri().path();
        let answer = serve(if path == "/demo.html" && !dev_mode {
            ""
        } else {
            path
        });
        (answer.status, answer)
    } else {
        (405, serve(""))
    };
    let mut builder = Response::builder().status(status);
    for (name, value) in answer.headers() {
        builder = builder.header(name, value);
    }
    let body: Cow<'static, [u8]> = if method == Method::HEAD {
        Cow::Borrowed(&[])
    } else {
        Cow::Borrowed(answer.body)
    };
    builder.body(body).unwrap_or_else(|_| {
        let mut empty = Response::new(Cow::Borrowed(&[][..]));
        *empty.status_mut() = wry::http::StatusCode::INTERNAL_SERVER_ERROR;
        empty
    })
}

/// Opens the window and runs until it closes.
pub fn run(
    config: LauncherConfig,
    webview_version: &str,
    handoffs: Option<Box<dyn HandoffListener>>,
) -> Result<(), LauncherError> {
    let platform_error = |what: &str, error: &dyn std::fmt::Display| {
        LauncherError::Platform(format!("{what}: {error}"))
    };

    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let sink: Arc<dyn EventSink> = Arc::new(ProxySink(Mutex::new(event_loop.create_proxy())));
    let policy = Arc::new(UrlPolicy::new(&config));
    let http: Arc<dyn HttpClient> = Arc::new(UreqClient::new(CHECK_TIMEOUT));
    let shared = Arc::new(Shared {
        offline: AtomicBool::new(false),
        site_loaded: AtomicBool::new(false),
        shown: AtomicBool::new(false),
    });

    // The origin to re-check: the development address in developer mode, else the site.
    let check_origin: Option<SiteOrigin> = if config.dev_mode {
        config.dev_origin.clone()
    } else {
        config.site.as_ref().ok().cloned()
    };

    let window = WindowBuilder::new()
        .with_title("Metallic")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(LogicalSize::new(960.0, 600.0))
        .with_window_icon(window_icon())
        // Hidden until the first page has painted, so the window never opens as a white box.
        .with_visible(false)
        .build(&event_loop)
        .map_err(|e| platform_error("could not open the window", &e))?;

    let start = start_page(&config, None);
    let start_address = match &start {
        StartPage::Site(url) => url.to_string(),
        StartPage::Local(page) => local_address(page),
    };
    if let StartPage::Local(LocalPage::Offline(_)) = &start {
        shared.offline.store(true, Ordering::SeqCst);
    }

    let bridge = Bridge::new(UrlPolicy::new(&config));
    let settings = SettingsStore::open_default().load();
    let channel = settings.channel;
    // The GitHub session for the private team feed lives in the Windows credential store; sign-in
    // needs the app's client id, and without it team builds are simply not offered.
    let github = match embedded_client_id() {
        Ok(client_id) => Some(Arc::new(GithubAuth::new(
            client_id,
            Arc::new(UreqClient::default()),
            Arc::new(KeyringSecretStore::new("MetallicLauncher", "github")),
        ))),
        Err(error) => {
            log::info!("GitHub sign-in is not available: {error}");
            None
        }
    };
    // The Metallic sign-in talks to the platform on the site's own origin. Its requests are
    // short (the user is waiting for the code); the session lives in the Credential Manager.
    let platform_auth = check_origin.clone().map(|origin| {
        Arc::new(PlatformAuth::new(
            origin,
            Arc::new(UreqClient::new(SIGN_IN_TIMEOUT)),
            Arc::new(KeyringSecretStore::new(KEYRING_SERVICE, KEYRING_ACCOUNT)),
        ))
    });
    let tokens: Arc<dyn TokenProvider> = match &github {
        Some(auth) => Arc::new(StoredTokenProvider(auth.clone())),
        None => Arc::new(NoToken),
    };
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    let (studio_root, downloads) = match &local_app_data {
        Some(base) => (studio_root_for(base), downloads_dir_for(base)),
        None => Default::default(),
    };
    let installer = Arc::new(Installer::new(
        Arc::new(GithubAssets::new(
            tokens.clone(),
            Arc::new(UreqClient::default()),
        )),
        Arc::new(SystemSpawner),
        channel,
        studio_root,
        downloads,
    ));
    let installed_copy = std::env::current_exe()
        .map(|exe| is_installed_copy(&exe))
        .unwrap_or(false);
    let updater = Arc::new(SelfUpdater::new(
        Arc::new(UpdateChecker::new(LAUNCHER_FEED, channel, tokens)),
        sink.clone(),
        installed_copy,
    ));
    let players = Arc::new(PlayerWatcher::new(
        Arc::new(platform::SystemProcesses),
        product_paths().player_exe,
    ));
    let host = ShellHost {
        dev_mode: config.dev_mode,
        settings: Mutex::new(settings),
        store: SettingsStore::open_default(),
        http: http.clone(),
        check_origin: check_origin.clone(),
        sink: sink.clone(),
        installer,
        updater: updater.clone(),
        players: players.clone(),
        github,
        platform: platform_auth,
        platform_flight: OneAtATime::new(),
        github_flight: OneAtATime::new(),
    };

    let mut context = WebContext::new(platform::webview_data_dir());
    let mut builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_background_color(BACKGROUND)
        .with_user_agent(user_agent(webview_version, env!("CARGO_PKG_VERSION")))
        .with_devtools(config.dev_mode)
        // `window.metallic`, for the main frame. (On Windows the script also reaches frames, so
        // bridge.js checks for that itself and the shell ignores messages from frames.)
        .with_initialization_script_for_main_only(bridge.init_script(), true)
        .with_url(start_address);

    // The demo page is for developer mode only; in a normal run it does not exist.
    let dev_mode = config.dev_mode;
    builder = builder.with_custom_protocol("metallic-app".to_string(), move |_id, request| {
        protocol_response(request, dev_mode)
    });

    // Navigation: the policy decides; the system browser only ever gets an https address.
    {
        let policy = policy.clone();
        let sink = sink.clone();
        builder = builder
            .with_navigation_handler(move |url| apply_decision(policy.navigation(&url), &sink));
    }
    // A pop-up never opens a second window: it loads in this one, goes to the browser, or is refused.
    {
        let policy = policy.clone();
        let sink = sink.clone();
        builder = builder.with_new_window_req_handler(move |url, _features| {
            match policy.new_window(&url) {
                NavDecision::Allow => match Url::parse(&url) {
                    Ok(address) => sink.send(AppEvent::Navigate(address)),
                    Err(_) => log::info!("blocked a new window"),
                },
                other => {
                    apply_decision(other, &sink);
                }
            }
            NewWindowResponse::Deny
        });
    }
    // No downloads and no permission prompts (camera, microphone, location, notifications, ...).
    builder = builder
        .with_download_started_handler(|_url, _path| {
            log::info!("refused a download");
            false
        })
        .with_permission_handler(|kind| {
            log::info!("refused a {kind} permission request");
            PermissionResponse::Deny
        });

    // Messages from pages go to the bridge, which checks them in the event loop.
    {
        let sink = sink.clone();
        builder = builder.with_ipc_handler(move |request| {
            let page_url = request.uri().to_string();
            let body = request.into_body();
            sink.send(AppEvent::BridgeMessage { page_url, body });
        });
    }

    // Show the window when the first page has loaded; with --exit-after-load, leave soon after.
    {
        let policy = policy.clone();
        let sink = sink.clone();
        let shared = shared.clone();
        let exit_after_load = config.exit_after_load;
        let exit_started = Arc::new(AtomicBool::new(false));
        builder = builder.with_on_page_load_handler(move |event, url| {
            if !matches!(event, PageLoadEvent::Finished) {
                return;
            }
            if policy.origin_of(&url) == PageOrigin::Site {
                shared.site_loaded.store(true, Ordering::SeqCst);
            }
            sink.send(AppEvent::Focus);
            if exit_after_load && !exit_started.swap(true, Ordering::SeqCst) {
                let sink = sink.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(EXIT_GRACE);
                    sink.send(AppEvent::Quit);
                });
            }
        });
    }

    // The tray icon. Without it (an unsupported system, or an error) the close button quits.
    let studio_installed = product_paths().studio_exe.is_some();
    let (icon, icon_width, icon_height) = icon_rgba().unwrap_or_default();
    let tray = match platform::create_tray(
        &tray_menu(studio_installed, false),
        icon,
        icon_width,
        icon_height,
        sink.clone(),
    ) {
        Ok(tray) => Some(tray),
        Err(error) => {
            log::warn!("no tray icon: {error}");
            None
        }
    };

    let builder = platform::configure_builder(builder, config.dev_mode);
    let view: WebView = builder
        .build(&window)
        .map_err(|e| platform_error("could not start the web view", &e))?;
    platform::watch_navigation_failures(&view, sink.clone());
    platform::watch_frame_navigations(&view, policy.clone());

    // Check the site in the background; a Down answer before the site has loaded shows the offline page.
    if let (StartPage::Site(_), Some(origin)) = (&start, &check_origin)
        && !config.dev_mode
    {
        spawn_check(http.clone(), origin.clone(), sink.clone());
    }
    if let Some(origin) = check_origin.clone() {
        spawn_recheck_loop(http.clone(), origin, sink.clone(), shared.clone());
    }
    // Look for Launcher updates in the background (an installed copy only).
    updater.start();
    // Hand-overs from later starts, and the link this start was given.
    if let Some(listener) = handoffs {
        spawn_handoff_listener(listener, sink.clone());
    }
    if let Some(link) = config.start_link.clone() {
        sink.send(AppEvent::DeepLink(link));
    }
    // The window shows at the latest after a moment, whatever the page does.
    {
        let sink = sink.clone();
        let shared = shared.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SHOW_AT_LATEST);
            if !shared.shown.load(Ordering::SeqCst) {
                sink.send(AppEvent::Focus);
            }
        });
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if tray.is_some() && on_close_requested(&host.settings()) == CloseAction::HideToTray
                {
                    window.set_visible(false);
                } else {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::LoopDestroyed => {
                // The window is closing: apply a downloaded update, unless a Player runs.
                host.updater.on_quit(host.players.running_players());
            }
            Event::UserEvent(app_event) => match app_event {
                AppEvent::Quit => *control_flow = ControlFlow::Exit,
                AppEvent::Focus => {
                    shared.shown.store(true, Ordering::SeqCst);
                    window.set_visible(true);
                    window.set_minimized(false);
                    window.set_focus();
                }
                AppEvent::RunScript(script) => run_guarded_script(&view, &bridge, &script),
                AppEvent::Tray(action) => match action {
                    TrayAction::Show => sink.send(AppEvent::Focus),
                    TrayAction::OpenStudio => {
                        let _ = start_the_studio(None);
                    }
                    TrayAction::CheckUpdates => host.updater.check_soon(),
                    TrayAction::RestartToUpdate => {
                        if let Err(error) = host.restart_to_update() {
                            log::info!("the update did not restart: {error:?}");
                        }
                    }
                    TrayAction::Quit => *control_flow = ControlFlow::Exit,
                },
                AppEvent::Navigate(url) => match policy.origin_of(url.as_str()) {
                    PageOrigin::Site => load(&view, &shared, url.as_str(), false),
                    PageOrigin::Local => {
                        let address = platform::webview_local_url(&url);
                        load(&view, &shared, address.as_str(), false);
                    }
                    PageOrigin::Other => log::warn!("refused to load an address outside the site"),
                },
                AppEvent::Handoff(handoff) => {
                    // Only ever the site; the address holds a one-time code and is not logged.
                    let url = handoff.expose();
                    if policy.origin_of(url.as_str()) == PageOrigin::Site {
                        load(&view, &shared, url.as_str(), false);
                    } else {
                        log::warn!("refused a sign-in handoff outside the site");
                    }
                }
                AppEvent::ShowLocal(page) => {
                    let offline = matches!(page, LocalPage::Offline(_));
                    load(&view, &shared, &local_address(&page), offline);
                }
                AppEvent::SiteStatus(status) => {
                    send_bridge_event(
                        &view,
                        &bridge,
                        &BridgeEvent::SiteStatus {
                            up: status == SiteStatus::Up,
                        },
                    );
                    let offline = shared.offline.load(Ordering::SeqCst);
                    match status {
                        SiteStatus::Up if offline => {
                            if let StartPage::Site(url) = start_page(&config, Some(SiteStatus::Up))
                            {
                                load(&view, &shared, url.as_str(), false);
                            }
                        }
                        SiteStatus::Down
                            if !offline && !shared.site_loaded.load(Ordering::SeqCst) =>
                        {
                            let page = LocalPage::Offline(OfflineReason::Unreachable);
                            load(&view, &shared, &local_address(&page), true);
                        }
                        _ => {}
                    }
                }
                AppEvent::DeepLink(link) => handle_deep_link(
                    link,
                    check_origin.as_ref(),
                    shared.offline.load(Ordering::SeqCst),
                    sink.as_ref(),
                    &host.updater,
                ),
                AppEvent::BridgeMessage { page_url, body } => {
                    handle_bridge_message(&view, &bridge, &host, &page_url, &body);
                }
                AppEvent::Bridge(event) => {
                    // A downloaded update puts "Restart to update" into the tray menu.
                    if matches!(event, BridgeEvent::UpdateReady { .. })
                        && let Some(tray) = &tray
                    {
                        tray.set_items(&tray_menu(studio_installed, true));
                    }
                    send_bridge_event(&view, &bridge, &event);
                }
            },
            _ => {}
        }
    })
}

/// Loads `address` in the main view and updates the shared flags.
fn load(view: &WebView, shared: &Shared, address: &str, offline: bool) {
    shared.offline.store(offline, Ordering::SeqCst);
    shared.site_loaded.store(false, Ordering::SeqCst);
    if let Err(error) = view.load_url(address) {
        log::warn!("could not load a page: {error}");
    }
}

/// Carries out a policy decision. Returns whether the view itself may load the address.
fn apply_decision(decision: NavDecision, sink: &Arc<dyn EventSink>) -> bool {
    match decision {
        NavDecision::Allow => true,
        NavDecision::OpenInBrowser(url) => {
            if let Err(error) = platform::open_in_browser(&url) {
                log::warn!("could not open the browser: {error}");
            } else {
                log::info!("opened a link in the system browser");
            }
            false
        }
        NavDecision::HandleDeepLink(link) => {
            sink.send(AppEvent::DeepLink(link));
            false
        }
        NavDecision::Block => {
            log::info!("blocked a navigation");
            false
        }
    }
}
