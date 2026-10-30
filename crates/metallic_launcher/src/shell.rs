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

use metallic_launcher::{
    AppEvent, BRIDGE_VERSION, Bridge, BridgeErrorCode, BridgeEvent, BridgeHost, DeepLink,
    EventSink, LauncherConfig, LauncherError, LauncherInfo, LauncherSettings, LocalPage,
    NavDecision, OfflineReason, PageOrigin, PlaceId, SettingsPatch, SettingsStore, SiteOrigin,
    SiteStatus, StartPage, UrlPolicy, check_site, local_url, serve, start_page, user_agent,
};
use metallic_update::{HttpClient, UreqClient};
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

use crate::platform;

/// The graphite page background (`--bg-base` in the visual language), so no white shows before
/// the first page paints.
const BACKGROUND: (u8, u8, u8, u8) = (22, 21, 27, 255);

/// How long the site check may take (the health check and the offline re-checks).
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
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

/// What the bridge can do in this build. Playing, installing, Studio and sign-in arrive in later
/// steps and answer `notAvailable` until then.
struct ShellHost {
    dev_mode: bool,
    settings: Mutex<LauncherSettings>,
    store: SettingsStore,
    http: Arc<dyn HttpClient>,
    check_origin: Option<SiteOrigin>,
    sink: Arc<dyn EventSink>,
}

impl BridgeHost for ShellHost {
    fn info(&self) -> LauncherInfo {
        LauncherInfo {
            launcher_version: env!("CARGO_PKG_VERSION").to_string(),
            bridge_version: BRIDGE_VERSION,
            platform: "windows",
            channel: "stable".to_string(),
            dev_mode: self.dev_mode,
        }
    }

    fn play(&self, _link: DeepLink) -> Result<(), BridgeErrorCode> {
        log::info!("play was asked for; starting the Player arrives in a later step");
        Err(BridgeErrorCode::NotAvailable)
    }

    fn install_status(&self) -> Value {
        json!({ "player": "notInstalled", "studio": "notInstalled", "launcher": "upToDate" })
    }

    fn install_studio(&self) -> Result<(), BridgeErrorCode> {
        Err(BridgeErrorCode::NotAvailable)
    }

    fn open_studio(&self, _place: Option<PlaceId>) -> Result<(), BridgeErrorCode> {
        Err(BridgeErrorCode::NotAvailable)
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
        Err(BridgeErrorCode::NotAvailable)
    }

    fn sign_out(&self) -> Result<(), BridgeErrorCode> {
        Err(BridgeErrorCode::NotAvailable)
    }

    fn github_sign_in(&self) -> Result<Value, BridgeErrorCode> {
        Err(BridgeErrorCode::NotAvailable)
    }

    fn retry_site(&self) {
        match &self.check_origin {
            Some(origin) => spawn_check(self.http.clone(), origin.clone(), self.sink.clone()),
            None => self.sink.send(AppEvent::SiteStatus(SiteStatus::Down)),
        }
    }

    fn restart_to_update(&self) -> Result<(), BridgeErrorCode> {
        log::info!("a restart to update was asked for; updates arrive in a later step");
        Err(BridgeErrorCode::Failed)
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

/// Decodes the placeholder icon from `branding/`.
fn window_icon() -> Option<Icon> {
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
    Icon::from_rgba(rgba, info.width, info.height).ok()
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
pub fn run(config: LauncherConfig, webview_version: &str) -> Result<(), LauncherError> {
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
    if config.start_link.is_some() {
        log::info!("started with a metallic:// link; it is handled in a later step");
    }

    let bridge = Bridge::new(UrlPolicy::new(&config));
    let host = ShellHost {
        dev_mode: config.dev_mode,
        settings: Mutex::new(SettingsStore::open_default().load()),
        store: SettingsStore::open_default(),
        http: http.clone(),
        check_origin: check_origin.clone(),
        sink: sink.clone(),
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
            } => *control_flow = ControlFlow::Exit,
            Event::UserEvent(app_event) => match app_event {
                AppEvent::Quit => *control_flow = ControlFlow::Exit,
                AppEvent::Focus => {
                    shared.shown.store(true, Ordering::SeqCst);
                    window.set_visible(true);
                    window.set_minimized(false);
                    window.set_focus();
                }
                AppEvent::RunScript(script) => run_script(&view, &script),
                AppEvent::Navigate(url) => match policy.origin_of(url.as_str()) {
                    PageOrigin::Site => load(&view, &shared, url.as_str(), false),
                    PageOrigin::Local => {
                        let address = platform::webview_local_url(&url);
                        load(&view, &shared, address.as_str(), false);
                    }
                    PageOrigin::Other => log::warn!("refused to load an address outside the site"),
                },
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
                AppEvent::DeepLink(_) => {
                    log::info!("received a metallic:// link; it is handled in a later step");
                }
                AppEvent::BridgeMessage { page_url, body } => {
                    handle_bridge_message(&view, &bridge, &host, &page_url, &body);
                }
                AppEvent::Bridge(event) => send_bridge_event(&view, &bridge, &event),
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
