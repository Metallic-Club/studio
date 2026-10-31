//! Tests for the bundled pages, the site check and the first-page choice.

use metallic_launcher::local_pages::BUNDLED;
use metallic_launcher::{
    LOCAL_CSP, LauncherConfig, LauncherError, LocalPage, OfflineReason, SiteOrigin, SiteStatus,
    StartPage, check_site, local_url, serve, start_page, user_agent,
};
use metallic_update::{HttpClient, HttpError, HttpResponse};

fn config(
    site: Result<SiteOrigin, LauncherError>,
    dev_mode: bool,
    dev_origin: Option<SiteOrigin>,
) -> LauncherConfig {
    LauncherConfig {
        site,
        dev_origin,
        dev_mode,
        start_link: None,
        start_hidden: false,
        exit_after_load: false,
    }
}

fn site() -> Result<SiteOrigin, LauncherError> {
    SiteOrigin::parse("https://metallic.example", false)
}

#[test]
fn offline_page_is_served_with_strict_csp() {
    let r = serve("/offline.html");
    assert_eq!(r.status, 200);
    assert_eq!(r.content_type, "text/html; charset=utf-8");
    assert!(LOCAL_CSP.contains("default-src 'none'"));
    let headers = r.headers();
    assert!(headers.contains(&("Content-Security-Policy", LOCAL_CSP)));
    assert!(headers.contains(&("X-Content-Type-Options", "nosniff")));
}

#[test]
fn every_bundled_file_is_served_and_errors_carry_the_headers_too() {
    for (path, content_type, body) in BUNDLED {
        let r = serve(path);
        assert_eq!(r.status, 200, "{path}");
        assert_eq!(r.content_type, *content_type);
        assert_eq!(r.body, *body);
        assert!(!r.body.is_empty(), "{path}");
    }
    let missing = serve("/nope.html");
    assert_eq!(missing.status, 404);
    assert!(
        missing
            .headers()
            .contains(&("X-Content-Type-Options", "nosniff"))
    );
    assert!(
        missing
            .headers()
            .contains(&("Content-Security-Policy", LOCAL_CSP))
    );
}

#[test]
fn local_pages_reject_traversal() {
    for p in [
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/assets/../../x",
        "/OFFLINE.HTML",
        "",
        "/",
        "offline.html",
        "/offline.html?reason=unreachable",
        "//offline.html",
        "/offline.html/",
        "/pages/offline.html",
        "\\offline.html",
    ] {
        assert_eq!(serve(p).status, 404, "{p}");
    }
}

#[test]
fn local_urls_use_the_app_scheme() {
    assert_eq!(
        local_url(&LocalPage::Offline(OfflineReason::Unreachable)).as_str(),
        "metallic-app://local/offline.html?reason=unreachable"
    );
    assert_eq!(
        local_url(&LocalPage::Offline(OfflineReason::NotConfigured)).as_str(),
        "metallic-app://local/offline.html?reason=notconfigured"
    );
    assert_eq!(
        local_url(&LocalPage::Offline(OfflineReason::LoadFailed)).as_str(),
        "metallic-app://local/offline.html?reason=loadfailed"
    );
    assert_eq!(
        local_url(&LocalPage::Demo).as_str(),
        "metallic-app://local/demo.html"
    );
    assert_eq!(
        local_url(&LocalPage::Settings).as_str(),
        "metallic-app://local/settings.html"
    );
    // Every page address points at a file that is served.
    for page in [
        LocalPage::Offline(OfflineReason::Unreachable),
        LocalPage::Demo,
        LocalPage::Settings,
    ] {
        assert_eq!(serve(local_url(&page).path()).status, 200);
    }
}

/// True when `text` has an HTML event-handler attribute such as ` onclick=`.
fn has_event_handler(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(found) = text[i..].find(" on") {
        let start = i + found + 3;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_lowercase() {
            end += 1;
        }
        if end > start && bytes.get(end) == Some(&b'=') {
            return true;
        }
        i = start;
    }
    false
}

/// The pages' own text files: HTML, CSS and script (not fonts, images or licence texts).
fn page_text_files() -> impl Iterator<Item = (&'static str, &'static str)> {
    BUNDLED
        .iter()
        .filter(|(path, _, _)| {
            path.ends_with(".html") || path.ends_with(".css") || path.ends_with(".js")
        })
        .map(|(path, _, body)| (*path, std::str::from_utf8(body).expect("UTF-8")))
}

/// Every `url(...)` target in `css`, without quotes.
fn css_urls(css: &str) -> Vec<&str> {
    css.split("url(")
        .skip(1)
        .map(|rest| {
            rest.split(')')
                .next()
                .unwrap_or("")
                .trim_matches(['"', '\''])
        })
        .collect()
}

#[test]
fn bundled_pages_have_no_remote_urls_or_inline_script() {
    for (path, text) in page_text_files() {
        assert!(!text.contains("http://"), "{path}: remote address");
        assert!(!text.contains("https://"), "{path}: remote address");
        assert!(!text.contains("//cdn"), "{path}: remote address");
        assert!(!text.contains("@import"), "{path}: stylesheet import");
        // The only css url()s are the bundled fonts.
        for target in css_urls(text) {
            assert!(
                target.starts_with("/fonts/") && serve(target).status == 200,
                "{path}: css url({target})"
            );
        }
        if path.ends_with(".html") {
            assert!(!text.contains("style="), "{path}: inline style");
            assert!(!text.contains("<style"), "{path}: inline style");
            assert!(!has_event_handler(text), "{path}: inline event handler");
            assert!(!text.contains("javascript:") || text.contains("void(0)"));
            for part in text.split("<script").skip(1) {
                let tag_end = part.find('>').expect("script tag closes");
                let (tag, rest) = part.split_at(tag_end);
                assert!(tag.contains("src=\""), "{path}: script without src");
                let inner = rest[1..].split("</script>").next().unwrap_or("");
                assert!(inner.trim().is_empty(), "{path}: script with a body");
            }
        }
    }
}

#[test]
fn pages_load_the_bundled_fonts_and_set_the_theme_before_painting() {
    // Acceptance for the network-off case: Archivo and JetBrains Mono come from the Launcher.
    let css = std::str::from_utf8(serve("/pages.css").body).unwrap();
    for font in [
        "/fonts/Archivo-Variable.woff2",
        "/fonts/JetBrainsMono-Variable.woff2",
    ] {
        assert!(
            css.contains(&format!("url(\"{font}\") format(\"woff2\")")),
            "{font}"
        );
        let r = serve(font);
        assert_eq!((r.status, r.content_type), (200, "font/woff2"));
        assert_eq!(&r.body[..4], b"wOF2", "{font} is WOFF2");
    }
    // The token CSS names the same families the @font-face rules declare.
    let tokens = std::str::from_utf8(serve("/tokens.css").body).unwrap();
    assert!(tokens.contains("--font: 'Archivo'"));
    assert!(tokens.contains("--mono: 'JetBrains Mono'"));
    assert!(css.contains("font-family: \"Archivo\""));
    assert!(css.contains("font-family: \"JetBrains Mono\""));
    // The licences the settings page links to are served.
    let settings = std::str::from_utf8(serve("/settings.html").body).unwrap();
    for link in settings.split("href=\"").skip(1) {
        let target = link.split('"').next().unwrap();
        assert_eq!(serve(target).status, 200, "settings links to {target}");
    }
    assert!(
        std::str::from_utf8(serve("/fonts/LICENSE-Phosphor.txt").body)
            .unwrap()
            .contains("MIT License")
    );
    for licence in ["/fonts/OFL-Archivo.txt", "/fonts/OFL-JetBrainsMono.txt"] {
        let r = serve(licence);
        assert_eq!(r.status, 200, "{licence}");
        assert!(
            std::str::from_utf8(r.body)
                .unwrap()
                .contains("SIL OPEN FONT LICENSE")
        );
    }
    // Every page picks its theme in <head>, before the body paints, and names a theme the
    // token CSS has.
    for (path, text) in page_text_files().filter(|(p, _)| p.ends_with(".html")) {
        let head = text.split("</head>").next().unwrap();
        assert!(
            head.contains("<script src=\"/theme.js\"></script>"),
            "{path}"
        );
    }
    let theme_js = std::str::from_utf8(serve("/theme.js").body).unwrap();
    for id in metallic_tokens::parse(metallic_tokens::TOKENS_JSON)
        .unwrap()
        .theme_ids()
    {
        assert!(tokens.contains(&format!("[data-theme=\"{id}\"]")), "{id}");
        assert!(
            theme_js.contains(&format!("\"{id}\"")),
            "theme.js knows {id}"
        );
    }
}

#[test]
fn event_handler_check_catches_handlers() {
    assert!(has_event_handler("<a onclick=\"x()\">"));
    assert!(has_event_handler("<body onload=run()>"));
    assert!(!has_event_handler("<p>check one two</p>"));
    assert!(!has_event_handler("once upon a time on screen"));
}

#[test]
fn start_page_not_configured() {
    let c = config(Err(LauncherError::NotConfigured), false, None);
    assert_eq!(
        start_page(&c, None),
        StartPage::Local(LocalPage::Offline(OfflineReason::NotConfigured))
    );
    assert_eq!(
        start_page(&c, Some(SiteStatus::Up)),
        StartPage::Local(LocalPage::Offline(OfflineReason::NotConfigured))
    );
}

#[test]
fn start_page_unreachable() {
    let c = config(site(), false, None);
    assert_eq!(
        start_page(&c, Some(SiteStatus::Down)),
        StartPage::Local(LocalPage::Offline(OfflineReason::Unreachable))
    );
}

#[test]
fn start_page_site_when_up_or_unchecked() {
    let c = config(site(), false, None);
    let expected = StartPage::Site("https://metallic.example/".parse().unwrap());
    assert_eq!(start_page(&c, Some(SiteStatus::Up)), expected);
    assert_eq!(start_page(&c, None), expected);
}

#[test]
fn start_page_dev_without_url_is_demo() {
    let c = config(Err(LauncherError::NotConfigured), true, None);
    assert_eq!(start_page(&c, None), StartPage::Local(LocalPage::Demo));
}

#[test]
fn start_page_dev_url_wins() {
    let dev = SiteOrigin::parse("http://127.0.0.1:5173", true).unwrap();
    let c = config(site(), true, Some(dev));
    assert_eq!(
        start_page(&c, Some(SiteStatus::Down)),
        StartPage::Site("http://127.0.0.1:5173/".parse().unwrap())
    );
}

#[test]
fn user_agent_contains_launcher_token() {
    let ua = user_agent("130.0.0.0", "0.1.0");
    assert!(ua.ends_with(" MetallicLauncher/0.1.0 bridge/1"));
    assert!(ua.contains("Edg/130.0.0.0"));
    assert!(ua.contains("Chrome/130.0.0.0"));
}

/// A fake client that answers every GET the same way and remembers the address.
struct Fake {
    answer: Result<u16, &'static str>,
    asked: std::sync::Mutex<Vec<String>>,
}

impl Fake {
    fn new(answer: Result<u16, &'static str>) -> Self {
        Fake {
            answer,
            asked: Default::default(),
        }
    }
}

impl HttpClient for Fake {
    fn get(&self, url: &str, _headers: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        self.asked.lock().unwrap().push(url.to_string());
        match self.answer {
            Ok(status) => Ok(HttpResponse {
                status,
                body: String::new(),
            }),
            Err(message) => Err(HttpError::new(message)),
        }
    }

    fn post_form(
        &self,
        _url: &str,
        _headers: &[(&str, &str)],
        _form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        Err(HttpError::new("not used"))
    }
}

#[test]
fn site_check_up_and_down() {
    let origin = SiteOrigin::parse("https://metallic.example", false).unwrap();
    let ok = Fake::new(Ok(200));
    assert_eq!(check_site(&ok, &origin), SiteStatus::Up);
    assert_eq!(
        ok.asked.lock().unwrap().as_slice(),
        ["https://metallic.example/healthz"]
    );
    assert_eq!(check_site(&Fake::new(Ok(204)), &origin), SiteStatus::Up);
    assert_eq!(check_site(&Fake::new(Ok(503)), &origin), SiteStatus::Down);
    assert_eq!(check_site(&Fake::new(Ok(404)), &origin), SiteStatus::Down);
    assert_eq!(check_site(&Fake::new(Ok(302)), &origin), SiteStatus::Down);
    assert_eq!(
        check_site(&Fake::new(Err("no route")), &origin),
        SiteStatus::Down
    );
}
