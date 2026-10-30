//! Protocol parsing, canonicalization and secret-safe diagnostics.

use metallic_launcher::{DeepLink, DeepLinkError, JoinTicket, MAX_URI_LEN, PlaceId, SitePath};

#[test]
fn play_link_round_trips() {
    let link = DeepLink::parse(
        "metallic://play?ticket=example_123&place=2150000000000010&unknown=ignored",
    )
    .unwrap();
    assert_eq!(
        link.to_uri(),
        "metallic://play?place=2150000000000010&ticket=example_123"
    );
    assert_eq!(DeepLink::parse(&link.to_uri()).unwrap(), link);
}

#[test]
fn scheme_is_case_insensitive() {
    assert_eq!(
        DeepLink::parse("METALLIC://install").unwrap(),
        DeepLink::Install
    );
}

#[test]
fn rejects_other_schemes() {
    for uri in [
        "https://example.invalid/play",
        "metallic-app://local/x",
        "metallicx://play",
        "metallic",
    ] {
        assert_eq!(DeepLink::parse(uri), Err(DeepLinkError::NotMetallic));
    }
}

#[test]
fn rejects_overlong_uri() {
    assert_eq!(
        DeepLink::parse(&format!(
            "metallic://open?path=/{}",
            "a".repeat(MAX_URI_LEN)
        )),
        Err(DeepLinkError::TooLong)
    );
}

#[test]
fn rejects_bad_ticket_chars() {
    for ticket in [
        "short",
        "example%22%20--dev",
        "example+123",
        "example%00xx",
        "example/123",
        "%2565xample_123",
        &"x".repeat(129),
    ] {
        assert_eq!(
            DeepLink::parse(&format!("metallic://play?place=1&ticket={ticket}")),
            Err(DeepLinkError::BadParam("ticket"))
        );
    }
    assert!(JoinTicket::parse("example_123-ABC").is_ok());
}

#[test]
fn rejects_duplicate_params() {
    for (query, key) in [
        ("place=1&place=2&ticket=example_123", "place"),
        ("place=1&ticket=example_123&%74icket=example_456", "ticket"),
    ] {
        assert_eq!(
            DeepLink::parse(&format!("metallic://play?{query}")),
            Err(DeepLinkError::DuplicateParam(key))
        );
    }
    assert_eq!(
        DeepLink::parse("metallic://open?path=/a&path=/b"),
        Err(DeepLinkError::DuplicateParam("path"))
    );
}

#[test]
fn percent_decoding_happens_once() {
    assert!(DeepLink::parse("metallic://play?place=%2531&ticket=example_123").is_err());
    assert!(DeepLink::parse("metallic://play?place=%31&ticket=example_123").is_ok());
    let link = DeepLink::parse("metallic://open?path=%2Fgames%3Fq%3Dhello%2520world").unwrap();
    assert_eq!(DeepLink::parse(&link.to_uri()).unwrap(), link);
    assert_eq!(
        link,
        DeepLink::Open {
            path: Some(SitePath::parse("/games?q=hello%20world").unwrap())
        }
    );
}

#[test]
fn site_path_rejects_escapes() {
    for path in [
        "//evil.invalid",
        "/\\evil.invalid",
        "https://evil.invalid",
        "javascript:alert(1)",
        "/a%0Ab",
    ] {
        assert!(DeepLink::parse(&format!("metallic://open?path={path}")).is_err());
    }
    assert!(SitePath::parse(&format!("/{}", "a".repeat(512))).is_err());
    assert!(SitePath::parse("/games?q=ok#details").is_ok());
}

#[test]
fn place_id_rules() {
    for bad in ["0", "01", "-1", "+1", "", "18446744073709551616", "１"] {
        assert!(PlaceId::parse(bad).is_err());
        assert!(DeepLink::parse(&format!("metallic://studio/open?place={bad}")).is_err());
    }
    assert_eq!(
        PlaceId::parse("18446744073709551615").unwrap().to_string(),
        "18446744073709551615"
    );
}

#[test]
fn ticket_debug_is_redacted() {
    let link = DeepLink::parse("metallic://play?place=1&ticket=example_123").unwrap();
    assert!(!format!("{link:?}").contains("example_123"));
    assert!(
        !format!(
            "{:?}",
            DeepLink::parse("metallic://play?place=1&ticket=private%22value")
        )
        .contains("private")
    );
}

#[test]
fn invalid_authority_path_fragment_and_whitespace_are_rejected() {
    for uri in [
        "metallic://user@install",
        "metallic://install:123",
        "metallic://install/extra",
        "metallic://studio/other",
        "metallic://install#fragment",
        "metallic://play?place=1&ticket=example_123 --dev",
        "\nmetallic://install",
        "metallic:install",
        "metallic:///install",
    ] {
        assert!(
            DeepLink::parse(uri).is_err(),
            "unexpected accepted protocol shape"
        );
    }
}

#[test]
fn every_action_has_a_canonical_round_trip_and_required_fields() {
    for uri in [
        "metallic://studio/open",
        "metallic://studio/open?place=12",
        "metallic://open",
        "metallic://open?path=%2Fgames%2Fa+b%3Fq%3Dc%23d",
        "metallic://install?ignored=ok",
    ] {
        let link = DeepLink::parse(uri).unwrap();
        assert_eq!(DeepLink::parse(&link.to_uri()).unwrap(), link);
    }
    assert_eq!(
        DeepLink::parse("metallic://play?ticket=example_123"),
        Err(DeepLinkError::MissingParam("place"))
    );
    assert_eq!(
        DeepLink::parse("metallic://play?place=1"),
        Err(DeepLinkError::MissingParam("ticket"))
    );
}

#[test]
fn raw_protocol_shape_cannot_be_changed_by_url_normalization() {
    for uri in [
        "metallic://@install",
        "metallic://install:",
        "metallic://studio/a/../open",
        "metallic://studio/%2e/open",
        "metallic://studio/%6fpen",
    ] {
        assert!(
            DeepLink::parse(uri).is_err(),
            "normalized a non-protocol authority/path"
        );
    }
}
