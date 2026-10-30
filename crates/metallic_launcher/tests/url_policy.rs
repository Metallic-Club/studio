//! Navigation and origin checks are independent of a web view.

use metallic_launcher::{
    DeepLink, LauncherConfig, LauncherError, LauncherSettings, NavDecision, PageOrigin, SiteOrigin,
    SitePath, UrlPolicy, embedded_site_origin, is_local_page,
};
use url::Url;

fn policy() -> UrlPolicy {
    let config = LauncherConfig {
        site: Ok(SiteOrigin::parse("https://metallic.example", false).unwrap()),
        dev_origin: None,
        dev_mode: false,
        start_link: None,
        start_hidden: false,
        exit_after_load: false,
    };
    UrlPolicy::new(&config)
}

#[test]
fn site_navigation_allowed() {
    let p = policy();
    for uri in [
        "https://metallic.example/games/1",
        "https://METALLIC.example:443/games?q=ok#x",
    ] {
        assert_eq!(p.navigation(uri), NavDecision::Allow);
        assert_eq!(p.new_window(uri), NavDecision::Allow);
        assert_eq!(p.origin_of(uri), PageOrigin::Site);
    }
}

#[test]
fn lookalike_hosts_are_not_the_site() {
    for uri in [
        "https://metallic.example.evil.invalid/",
        "https://metallic.example@evil.invalid/",
        "https://evil.invalid/https://metallic.example",
        "https://xn--metallic-xyz.example/",
        "https://metallic.example./",
        "https://user@metallic.example/",
    ] {
        assert_eq!(policy().origin_of(uri), PageOrigin::Other);
    }
}

#[test]
fn http_downgrade_and_other_port_are_not_the_site() {
    assert_eq!(
        policy().navigation("http://metallic.example/"),
        NavDecision::Block
    );
    assert_eq!(
        policy().origin_of("https://metallic.example:8443/"),
        PageOrigin::Other
    );
}

#[test]
fn other_https_opens_in_browser() {
    assert!(matches!(
        policy().navigation("https://external.invalid/help"),
        NavDecision::OpenInBrowser(_)
    ));
    assert!(matches!(
        policy().new_window("https://external.invalid/help"),
        NavDecision::OpenInBrowser(_)
    ));
}

#[test]
fn dangerous_schemes_blocked() {
    for uri in [
        "file:///C:/Windows/win.ini",
        "javascript:alert(1)",
        "data:text/html,x",
        "blob:https://metallic.example/1",
        "ms-msdt:/id",
        "vbscript:x",
        "\\\\server\\share",
        "http://evil.invalid/",
        "about:blank#x",
        "about:config",
        " https://metallic.example/",
        "https://metallic.example/\n",
    ] {
        assert_eq!(policy().navigation(uri), NavDecision::Block);
    }
    assert_eq!(policy().navigation("about:blank"), NavDecision::Allow);
    assert_eq!(policy().origin_of("about:blank"), PageOrigin::Other);
}

#[test]
fn metallic_link_in_view_becomes_deep_link() {
    assert!(matches!(
        policy().navigation("metallic://play?place=1&ticket=example_123"),
        NavDecision::HandleDeepLink(_)
    ));
    assert_eq!(
        policy().navigation("metallic://play?place=x"),
        NavDecision::Block
    );
    assert_eq!(
        policy().new_window("metallic://install"),
        NavDecision::HandleDeepLink(DeepLink::Install)
    );
}

#[test]
fn dev_origin_only_in_dev_mode() {
    let args = ["--dev-url", "http://localhost:3000"];
    assert!(
        LauncherConfig::from_args(args.map(str::to_owned), &LauncherSettings::default()).is_err()
    );
    let config = LauncherConfig::from_args(
        ["--dev-url", "http://localhost:3000", "--dev"].map(str::to_owned),
        &LauncherSettings::default(),
    )
    .unwrap();
    assert_eq!(
        UrlPolicy::new(&config).origin_of("http://localhost:3000/"),
        PageOrigin::Site
    );
    let mut config = config;
    config.dev_mode = false;
    assert_eq!(
        UrlPolicy::new(&config).origin_of("http://localhost:3000/"),
        PageOrigin::Other
    );
}

#[test]
fn loopback_http_only_for_dev() {
    for host in ["localhost", "127.0.0.1", "[::1]"] {
        let uri = format!("http://{host}:3000");
        assert!(SiteOrigin::parse(&uri, true).is_ok());
        assert!(SiteOrigin::parse(&uri, false).is_err());
    }
    for uri in [
        "http://evil.invalid",
        "http://localhost.evil.invalid",
        "http://127.0.0.2",
    ] {
        assert!(SiteOrigin::parse(uri, true).is_err());
    }
}

#[test]
fn placeholder_site_is_not_configured() {
    assert_eq!(embedded_site_origin(), Err(LauncherError::NotConfigured));
    assert_eq!(
        SiteOrigin::parse("REPLACE_WITH_SITE_ORIGIN\n", false),
        Err(LauncherError::NotConfigured)
    );
    assert_eq!(
        SiteOrigin::parse("", false),
        Err(LauncherError::NotConfigured)
    );
}

#[test]
fn origin_rejects_credentials_paths_queries_fragments_and_never_echoes_input() {
    for uri in [
        "https://user:private@example.invalid",
        "https://example.invalid/a",
        "https://example.invalid/a/..",
        "https://example.invalid/%2e%2e",
        "https://example.invalid/?q=private",
        "https://example.invalid/#private",
        "file:///private",
        "https:\\example.invalid",
        "https://example.invalid\n",
    ] {
        let error = SiteOrigin::parse(uri, false).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private"));
    }
}

#[test]
fn site_paths_cannot_replace_the_origin() {
    let origin = SiteOrigin::parse("https://metallic.example:8443", false).unwrap();
    for path in [
        "/games?q=hello%20world#x",
        "/%2F%2Fevil.invalid",
        "/../games",
        "/unicode/é",
    ] {
        let target = origin.url(&SitePath::parse(path).unwrap());
        assert!(origin.matches(&target));
    }
}

#[test]
fn local_pages_only_trust_the_exact_hosts_and_default_ports() {
    for uri in [
        "metallic-app://local/offline.html",
        "http://metallic-app.localhost/settings.html",
        "https://metallic-app.localhost/demo.html",
    ] {
        assert!(is_local_page(&Url::parse(uri).unwrap()));
        assert_eq!(policy().navigation(uri), NavDecision::Allow);
        assert_eq!(policy().origin_of(uri), PageOrigin::Local);
    }
    for uri in [
        "metallic-app://evil/offline.html",
        "http://metallic-app.localhost:3000/",
        "https://metallic-app.localhost.evil.invalid/",
        "metallic-app://user@local/offline.html",
        "metallic-app://local:123/offline.html",
    ] {
        assert!(!is_local_page(&Url::parse(uri).unwrap()));
        assert_eq!(policy().origin_of(uri), PageOrigin::Other);
    }
}

#[test]
fn hostile_navigation_is_bounded() {
    let input = format!("https://metallic.example/{}", "x".repeat(100_000));
    assert_eq!(policy().navigation(&input), NavDecision::Block);
    assert_eq!(policy().origin_of(&input), PageOrigin::Other);
    assert!(SiteOrigin::parse(&input, false).is_err());
    assert!(SiteOrigin::parse(&" ".repeat(100_000), false).is_err());
}
