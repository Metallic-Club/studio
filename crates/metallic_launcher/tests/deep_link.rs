//! Tests for `metallic://` link parsing.

use metallic_launcher::deep_link::MAX_URI_LEN;
use metallic_launcher::{DeepLink, DeepLinkError};

const TICKET: &str = "jt_q3Vn0w9LkS2c";

#[test]
fn play_link_round_trips() {
    let l =
        DeepLink::parse("metallic://play?place=2150000000000010&ticket=jt_q3Vn0w9LkS2c").unwrap();
    assert_eq!(DeepLink::parse(&l.to_uri()).unwrap(), l);
    assert_eq!(
        l.to_uri(),
        "metallic://play?place=2150000000000010&ticket=jt_q3Vn0w9LkS2c"
    );
}

#[test]
fn every_action_round_trips() {
    for uri in [
        "metallic://studio/open",
        "metallic://studio/open?place=42",
        "metallic://open",
        "metallic://open?path=%2Fgames%2F1%3Fa%3Db",
        "metallic://install",
    ] {
        let link = DeepLink::parse(uri).unwrap();
        assert_eq!(link.to_uri(), uri);
        assert_eq!(DeepLink::parse(&link.to_uri()).unwrap(), link);
    }
}

#[test]
fn scheme_is_case_insensitive() {
    assert!(DeepLink::parse("METALLIC://install").is_ok());
    assert_eq!(
        DeepLink::parse("METALLIC://install").unwrap().to_uri(),
        "metallic://install"
    );
}

#[test]
fn rejects_other_schemes() {
    for u in [
        "https://x/play",
        "metallic-app://local/x",
        "metallicx://play",
    ] {
        assert_eq!(DeepLink::parse(u), Err(DeepLinkError::NotMetallic));
    }
}

#[test]
fn rejects_overlong_uri() {
    let u = format!("metallic://open?path=/{}", "a".repeat(2100));
    assert_eq!(DeepLink::parse(&u), Err(DeepLinkError::TooLong));
    assert!(u.len() > MAX_URI_LEN);
}

#[test]
fn rejects_bad_ticket_chars() {
    assert_eq!(
        DeepLink::parse("metallic://play?place=1&ticket=jt_abc%22%20--dev"),
        Err(DeepLinkError::BadParam("ticket"))
    );
}

#[test]
fn rejects_duplicate_params() {
    assert_eq!(
        DeepLink::parse("metallic://play?place=1&place=2&ticket=jt_abcdefgh"),
        Err(DeepLinkError::DuplicateParam("place"))
    );
}

#[test]
fn percent_decoding_happens_once() {
    assert!(DeepLink::parse("metallic://play?place=%2531&ticket=jt_abcdefgh").is_err());
    assert!(DeepLink::parse("metallic://play?place=%31&ticket=jt_abcdefgh").is_ok());
}

#[test]
fn site_path_rejects_escapes() {
    for p in [
        "//evil.test",
        "/\\evil.test",
        "https://evil.test",
        "javascript:alert(1)",
        "/a%0Ab",
    ] {
        assert!(
            DeepLink::parse(&format!("metallic://open?path={p}")).is_err(),
            "{p}"
        );
    }
}

#[test]
fn place_id_rules() {
    for bad in ["0", "01", "-1", "+1", "", "18446744073709551616"] {
        assert!(
            DeepLink::parse(&format!("metallic://studio/open?place={bad}")).is_err(),
            "{bad}"
        );
    }
    assert!(DeepLink::parse("metallic://studio/open?place=18446744073709551615").is_ok());
}

#[test]
fn ticket_debug_is_redacted() {
    let link = DeepLink::parse(&format!("metallic://play?place=1&ticket={TICKET}")).unwrap();
    let shown = format!("{link:?}");
    assert!(!shown.contains("q3Vn0w9LkS2c"), "{shown}");
}

#[test]
fn errors_do_not_echo_the_input() {
    let err = DeepLink::parse("metallic://play?place=1&ticket=SECRETSECRET%22x").unwrap_err();
    assert!(!format!("{err} {err:?}").contains("SECRETSECRET"));
}

#[test]
fn missing_params_and_unknown_actions() {
    assert_eq!(
        DeepLink::parse("metallic://play?ticket=jt_abcdefgh"),
        Err(DeepLinkError::MissingParam("place"))
    );
    assert_eq!(
        DeepLink::parse("metallic://play?place=1"),
        Err(DeepLinkError::MissingParam("ticket"))
    );
    for u in [
        "metallic://nope",
        "metallic://studio",
        "metallic://install/extra",
        "metallic://user@install",
        "metallic://install:80",
        "metallic:install",
    ] {
        assert_eq!(DeepLink::parse(u), Err(DeepLinkError::UnknownAction), "{u}");
    }
}

#[test]
fn unknown_keys_are_ignored_and_raw_spaces_refused() {
    assert!(DeepLink::parse("metallic://install?utm=1&utm=2").is_ok());
    assert!(DeepLink::parse("metallic://open?path=/a b").is_err());
    assert!(DeepLink::parse("metallic://install\n--dev").is_err());
}

#[test]
fn site_path_rejects_invisible_format_characters() {
    for p in ["/a%E2%80%AEb", "/a%E2%80%8Bb"] {
        assert!(
            DeepLink::parse(&format!("metallic://open?path={p}")).is_err(),
            "{p}"
        );
    }
}
