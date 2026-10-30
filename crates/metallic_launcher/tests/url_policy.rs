//! Tests for the site origin and the navigation policy.

use metallic_launcher::{
    LauncherConfig, LauncherError, LauncherSettings, NavDecision, PageOrigin, SiteOrigin, UrlPolicy,
};

fn policy() -> UrlPolicy {
    let config = LauncherConfig {
        site: SiteOrigin::parse("https://metallic.example", false),
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
    assert_eq!(
        policy().navigation("https://metallic.example/games/1"),
        NavDecision::Allow
    );
    assert_eq!(policy().navigation("about:blank"), NavDecision::Allow);
    assert_eq!(
        policy().navigation("metallic-app://local/offline.html"),
        NavDecision::Allow
    );
    assert_eq!(
        policy().navigation("http://metallic-app.localhost/offline.html"),
        NavDecision::Allow
    );
}

#[test]
fn lookalike_hosts_are_not_the_site() {
    let p = policy();
    for u in [
        "https://metallic.example.evil.test/",
        "https://metallic.example@evil.test/",
        "https://evil.test/https://metallic.example",
        "https://xn--metallic-xyz.example/",
        "https://user@metallic.example/",
        "not a url",
    ] {
        assert_eq!(p.origin_of(u), PageOrigin::Other, "{u}");
    }
    assert_eq!(p.origin_of("https://metallic.example/x"), PageOrigin::Site);
}

#[test]
fn http_downgrade_and_other_port_are_not_the_site() {
    let p = policy();
    assert_eq!(p.navigation("http://metallic.example/"), NavDecision::Block);
    assert_eq!(
        p.origin_of("https://metallic.example:8443/"),
        PageOrigin::Other
    );
    assert_eq!(
        p.origin_of("https://metallic.example:443/"),
        PageOrigin::Site
    );
}

#[test]
fn other_https_opens_in_browser() {
    assert!(matches!(
        policy().navigation("https://discord.com/x"),
        NavDecision::OpenInBrowser(_)
    ));
}

#[test]
fn dangerous_schemes_blocked() {
    let p = policy();
    for u in [
        "file:///C:/Windows/win.ini",
        "javascript:alert(1)",
        "data:text/html,x",
        "blob:https://metallic.example/1",
        "ms-msdt:/id",
        "vbscript:x",
        "\\\\server\\share",
        "http://evil.test/",
        "about:blank#x",
    ] {
        assert_eq!(p.navigation(u), NavDecision::Block, "{u}");
    }
}

#[test]
fn metallic_link_in_view_becomes_deep_link() {
    let p = policy();
    assert!(matches!(
        p.navigation("metallic://play?place=1&ticket=jt_abcdefgh"),
        NavDecision::HandleDeepLink(_)
    ));
    assert_eq!(p.navigation("metallic://play?place=x"), NavDecision::Block);
}

#[test]
fn new_window_follows_the_same_rules() {
    let p = policy();
    assert_eq!(
        p.new_window("https://metallic.example/a"),
        NavDecision::Allow
    );
    assert_eq!(p.new_window("about:blank"), NavDecision::Block);
    assert!(matches!(
        p.new_window("https://discord.com/"),
        NavDecision::OpenInBrowser(_)
    ));
    assert_eq!(p.new_window("javascript:1"), NavDecision::Block);
}

#[test]
fn dev_origin_only_in_dev_mode() {
    let settings = LauncherSettings::default();
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(
        LauncherConfig::from_args(args(&["--dev-url", "http://localhost:3000"]), &settings)
            .is_err()
    );
    let config = LauncherConfig::from_args(
        args(&["--dev", "--dev-url", "http://localhost:3000"]),
        &settings,
    )
    .unwrap();
    assert_eq!(
        UrlPolicy::new(&config).origin_of("http://localhost:3000/"),
        PageOrigin::Site
    );
    // Without the flag the same address is just another plain-http page.
    assert_eq!(
        policy().navigation("http://localhost:3000/"),
        NavDecision::Block
    );
}

#[test]
fn loopback_http_only_for_dev() {
    assert!(SiteOrigin::parse("http://metallic.example", true).is_err());
    assert!(SiteOrigin::parse("http://127.0.0.1:3000", false).is_err());
    assert!(SiteOrigin::parse("http://127.0.0.1:3000", true).is_ok());
    assert!(SiteOrigin::parse("http://[::1]:3000", true).is_ok());
}

#[test]
fn origin_shape_is_strict() {
    for bad in [
        "https://metallic.example/path",
        "https://metallic.example/?q=1",
        "https://metallic.example/#f",
        "https://user@metallic.example",
        "ftp://metallic.example",
        "metallic.example",
    ] {
        assert!(SiteOrigin::parse(bad, true).is_err(), "{bad}");
    }
    assert!(SiteOrigin::parse("https://metallic.example/", false).is_ok());
}

#[test]
fn placeholder_site_is_not_configured() {
    assert!(matches!(
        SiteOrigin::parse("REPLACE_WITH_SITE_ORIGIN\n", false),
        Err(LauncherError::NotConfigured)
    ));
    assert!(matches!(
        SiteOrigin::parse("  ", false),
        Err(LauncherError::NotConfigured)
    ));
    // The committed file is still the placeholder, so no address counts as the site.
    assert!(metallic_launcher::site::embedded_site_origin().is_err());
}

#[test]
fn site_url_cannot_leave_the_origin() {
    use metallic_launcher::SitePath;
    let origin = SiteOrigin::parse("https://metallic.example:8443", false).unwrap();
    let url = origin.url(&SitePath::new("/games/1?a=b").unwrap());
    assert_eq!(url.as_str(), "https://metallic.example:8443/games/1?a=b");
    assert!(origin.matches(&url));
}
