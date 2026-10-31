//! Tests for the shared update checker and GitHub sign-in. All HTTP is mocked or goes to 127.0.0.1;
//! nothing touches the internet or the real Windows Credential Manager.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use metallic_update::github_auth::{AuthError, DeviceFlowUi, GithubAuth, parse_client_id};
use metallic_update::{
    Channel, HttpClient, HttpError, HttpResponse, LAUNCHER_FEED, MemorySecretStore, ReleaseAsset,
    STUDIO_FEED, SecretStore, Token, UpdateChecker, UpdateError, UreqClient, download_verified,
};
use semver::Version;
use sha2::{Digest, Sha256};

/// One recorded request: url, headers, form body.
type Recorded = (String, Vec<(String, String)>, Option<String>);

/// A scripted HTTP client: pops one canned answer per call and records every request.
#[derive(Default)]
struct MockHttp {
    answers: Mutex<VecDeque<Result<HttpResponse, HttpError>>>,
    requests: Mutex<Vec<Recorded>>,
}

impl MockHttp {
    fn with(answers: Vec<Result<HttpResponse, HttpError>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            requests: Mutex::default(),
        })
    }

    fn next(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<HttpResponse, HttpError> {
        let headers = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.requests
            .lock()
            .unwrap()
            .push((url.to_string(), headers, body.map(str::to_string)));
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(HttpError::new("no scripted answer left")))
    }
}

impl HttpClient for MockHttp {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        self.next(url, headers, None)
    }

    fn post_form(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let body = form
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        self.next(url, headers, Some(&body))
    }
}

fn ok(body: &str) -> Result<HttpResponse, HttpError> {
    Ok(HttpResponse {
        status: 200,
        body: body.to_string(),
    })
}

fn release(tag: &str, body: &str) -> String {
    format!(r#"{{"tag_name":"{tag}","draft":false,"prerelease":true,"body":{body:?}}}"#)
}

fn feed(releases: &[String]) -> String {
    format!("[{}]", releases.join(","))
}

const SECRET: &str = "ghu_secret_token_value";
const REFRESH_SECRET: &str = "ghr_secret_refresh_value";

fn checker(channel: Channel, current: &str, http: Arc<MockHttp>) -> UpdateChecker {
    UpdateChecker::new(
        STUDIO_FEED,
        channel,
        Arc::new(Token::new(SECRET.to_string())),
    )
    .with_http(http)
    .with_current_version(Version::parse(current).unwrap())
}

#[test]
fn newer_version_detected() {
    let http = MockHttp::with(vec![ok(&feed(&[
        release("v0.1.0-dev", ""),
        release("v0.1.2-dev", "notes"),
        release("v0.1.1-dev", ""),
    ]))]);
    let update = checker(Channel::Dev, "0.1.0", http.clone())
        .check()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, Version::new(0, 1, 2));
    assert!(!update.required);
    let requests = http.requests.lock().unwrap();
    assert!(
        requests[0]
            .0
            .ends_with("/repos/Metallic-Club/studio/releases?per_page=50&page=1")
    );
    assert!(
        requests[0]
            .1
            .iter()
            .any(|(k, v)| k == "Authorization" && v == &format!("Bearer {SECRET}"))
    );
}

#[test]
fn same_version_no_update() {
    let http = MockHttp::with(vec![ok(&feed(&[
        release("v0.1.0-dev", ""),
        release("v0.0.9-dev", ""),
    ]))]);
    assert!(
        checker(Channel::Dev, "0.1.0", http)
            .check()
            .unwrap()
            .is_none()
    );
}

#[test]
fn channel_dev_ignores_stable_only_releases() {
    let stable_only = feed(&[release("v0.2.0-stable", ""), release("v0.1.0-dev", "")]);
    let dev = checker(
        Channel::Dev,
        "0.1.0",
        MockHttp::with(vec![ok(&stable_only)]),
    );
    assert!(dev.check().unwrap().is_none());
    let stable = checker(
        Channel::Stable,
        "0.1.0",
        MockHttp::with(vec![ok(&stable_only)]),
    );
    assert_eq!(
        stable.check().unwrap().unwrap().version,
        Version::new(0, 2, 0)
    );
}

#[test]
fn required_flag_parsed() {
    let body = "Fixes a save bug.\r\nRequired: true\r\n";
    let http = MockHttp::with(vec![ok(&feed(&[release("v0.3.0-dev", body)]))]);
    assert!(
        checker(Channel::Dev, "0.1.0", http)
            .check()
            .unwrap()
            .unwrap()
            .required
    );

    // A required release that is skipped over still makes the newest update required.
    let skipped = feed(&[
        release("v0.3.0-dev", "required: true"),
        release("v0.4.0-dev", "plain"),
    ]);
    let http = MockHttp::with(vec![ok(&skipped)]);
    assert!(
        checker(Channel::Dev, "0.1.0", http)
            .check()
            .unwrap()
            .unwrap()
            .required
    );

    // A required release that is already installed does not count.
    let http = MockHttp::with(vec![ok(&skipped)]);
    assert!(
        !checker(Channel::Dev, "0.3.0", http)
            .check()
            .unwrap()
            .unwrap()
            .required
    );
}

fn fifty_newer_dev_releases(page: u64) -> Vec<String> {
    (0..50)
        .map(|index| release(&format!("v0.1.{}-dev", 1_000 - page * 50 - index), ""))
        .collect()
}

#[test]
fn required_update_on_second_page_is_not_lost() {
    let first: Vec<_> = (2..=51)
        .rev()
        .map(|patch| release(&format!("v0.1.{patch}-dev"), ""))
        .collect();
    let http = MockHttp::with(vec![
        ok(&feed(&first)),
        ok(&feed(&[
            release("v0.1.1-dev", "required: true"),
            release("v0.1.0-dev", ""),
        ])),
    ]);
    let update = checker(Channel::Dev, "0.1.0", http.clone())
        .check()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, Version::new(0, 1, 51));
    assert!(update.required);
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].0.ends_with("per_page=50&page=2"));
}

#[test]
fn installed_version_boundary_finishes_a_full_page() {
    let mut first = fifty_newer_dev_releases(0);
    first[49] = release("v0.1.0-dev", "required: true");
    let http = MockHttp::with(vec![ok(&feed(&first))]);
    let update = checker(Channel::Dev, "0.1.0", http.clone())
        .check()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, Version::new(0, 1, 1_000));
    assert!(
        !update.required,
        "an already installed requirement does not count"
    );
    assert_eq!(http.requests.lock().unwrap().len(), 1);
}

#[test]
fn drafts_and_other_channels_do_not_end_required_history() {
    let mut first = fifty_newer_dev_releases(0);
    first[45] = release("v0.1.0-rc.1-dev", "");
    first[46] = release("v0.1.0+old-dev", "");
    first[47] = r#"{"tag_name":"v0.0.1-dev","draft":true}"#.into();
    first[48] = release("v0.0.1-stable", "required: true");
    first[49] = release("not-a-version-dev", "required: true");
    let http = MockHttp::with(vec![
        ok(&feed(&first)),
        ok(&feed(&[release("v0.1.1-dev", "required: true")])),
    ]);
    let update = checker(Channel::Dev, "0.1.0", http.clone())
        .check()
        .unwrap()
        .unwrap();
    assert!(update.required);
    assert_eq!(update.version, Version::new(0, 1, 1_000));
    assert_eq!(http.requests.lock().unwrap().len(), 2);
}

#[test]
fn prerelease_and_build_metadata_are_not_release_process_tags() {
    let http = MockHttp::with(vec![ok(&feed(&[
        release("v9.0.0-rc.1-dev", "required: true"),
        release("v9.0.0+local-dev", "required: true"),
        release("v0.1.0-dev", ""),
    ]))]);
    assert!(
        checker(Channel::Dev, "0.1.0", http)
            .check()
            .unwrap()
            .is_none()
    );
}

#[test]
fn empty_last_page_preserves_the_newest_version() {
    let first = fifty_newer_dev_releases(0);
    let http = MockHttp::with(vec![ok(&feed(&first)), ok("[]")]);
    let update = checker(Channel::Dev, "0.1.0", http.clone())
        .check()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, Version::new(0, 1, 1_000));
    assert_eq!(http.requests.lock().unwrap().len(), 2);
}

/// #240 follow-up: more than 250 releases behind, the check cannot see every skipped release,
/// so it fails closed: the newest version is offered as required (not a permanent error).
#[test]
fn truncated_history_offers_the_newest_update_as_required() {
    let http = MockHttp::with(
        (0..5)
            .map(|page| ok(&feed(&fifty_newer_dev_releases(page))))
            .collect(),
    );
    let update = checker(Channel::Dev, "0.1.0", http.clone())
        .check()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, Version::new(0, 1, 1_000));
    assert!(update.required, "an unproven history counts as required");
    assert_eq!(
        http.requests.lock().unwrap().len(),
        5,
        "request count remains bounded"
    );
}

/// #376 F4: releases of the other channel (or app) do not count toward the 250: a stable user
/// one release behind, under 300 dev releases, gets an optional update.
#[test]
fn other_channels_releases_do_not_make_an_update_required() {
    let mut answers: Vec<_> = (0..6)
        .map(|page| ok(&feed(&fifty_newer_dev_releases(page))))
        .collect();
    answers.push(ok(&feed(&[
        release("v1.0.1-stable", ""),
        release("v1.0.0-stable", ""),
    ])));
    let http = MockHttp::with(answers);
    let update = checker(Channel::Stable, "1.0.0", http.clone())
        .check()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, Version::new(1, 0, 1));
    assert!(!update.required, "one release behind is not too far behind");
    assert_eq!(http.requests.lock().unwrap().len(), 7);
}

#[test]
fn later_history_failure_cannot_offer_a_partial_required_flag() {
    let http = MockHttp::with(vec![
        ok(&feed(&fifty_newer_dev_releases(0))),
        Err(HttpError::new("offline")),
    ]);
    assert!(matches!(
        checker(Channel::Dev, "0.1.0", http.clone()).check(),
        Err(UpdateError::Network(_))
    ));
    assert_eq!(http.requests.lock().unwrap().len(), 2);
}

#[test]
fn network_error_is_not_fatal() {
    // Injected failure.
    let http = MockHttp::with(vec![Err(HttpError::new("connection reset"))]);
    let c = checker(Channel::Dev, "0.1.0", http);
    assert!(matches!(c.check(), Err(UpdateError::Network(_))));

    // Non-success statuses and junk answers are errors, not panics.
    let http = MockHttp::with(vec![Ok(HttpResponse {
        status: 401,
        body: String::new(),
    })]);
    assert!(matches!(
        checker(Channel::Dev, "0.1.0", http).check(),
        Err(UpdateError::Unauthorized)
    ));
    let http = MockHttp::with(vec![ok("not json")]);
    assert!(matches!(
        checker(Channel::Dev, "0.1.0", http).check(),
        Err(UpdateError::BadResponse(_))
    ));

    // `check_quietly` turns every failure into "no update".
    let http = MockHttp::with(vec![Err(HttpError::new("offline"))]);
    assert!(
        checker(Channel::Dev, "0.1.0", http)
            .check_quietly()
            .is_none()
    );

    // A real client against a closed local port fails with an error, never a panic.
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let c = UpdateChecker::new(STUDIO_FEED, Channel::Dev, Arc::new(Token::new("t".into())))
        .with_api_base(format!("http://127.0.0.1:{port}"))
        .unwrap()
        .with_http(Arc::new(UreqClient::new(Duration::from_secs(2))));
    assert!(matches!(c.check(), Err(UpdateError::Network(_))));
}

#[test]
fn ureq_client_talks_to_a_local_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_by_server = seen.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap();
        *seen_by_server.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).to_lowercase();
        let body = feed(&[release("v9.0.0-dev", "")]);
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(reply.as_bytes()).unwrap();
    });
    let c = UpdateChecker::new(
        STUDIO_FEED,
        Channel::Dev,
        Arc::new(Token::new("tok123".into())),
    )
    .with_api_base(format!("http://127.0.0.1:{port}"))
    .unwrap()
    .with_http(Arc::new(UreqClient::new(Duration::from_secs(5))))
    .with_current_version(Version::new(0, 1, 0));
    assert_eq!(c.check().unwrap().unwrap().version, Version::new(9, 0, 0));
    server.join().unwrap();
    let request = seen.lock().unwrap().clone();
    assert!(request.contains("authorization: bearer tok123"));
    assert!(request.contains("user-agent: metallic-studio"));
}

/// Captures every log record so tests can search the text.
struct Capture(Mutex<Vec<String>>);

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        self.0
            .lock()
            .unwrap()
            .push(format!("{} {}", record.level(), record.args()));
    }
    fn flush(&self) {}
}

fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<&'static Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let c: &'static Capture = Box::leak(Box::new(Capture(Mutex::default())));
        log::set_logger(c).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
        c
    })
}

struct NoUi;

impl DeviceFlowUi for NoUi {
    fn show_code(&mut self, _user_code: &str, _verification_uri: &str) {}
}

#[test]
fn token_never_logged() {
    let logs = capture();

    let token = Token::new(SECRET.to_string());
    assert!(!format!("{token:?}").contains(SECRET));
    assert!(!format!("{token}").contains(SECRET));
    assert!(!format!("{:#?}", Some(&token)).contains(SECRET));

    // Update check: success, HTTP failure whose body echoes the token, network failure.
    let http = MockHttp::with(vec![
        ok(&feed(&[release("v0.2.0-dev", "")])),
        Ok(HttpResponse {
            status: 403,
            body: format!("denied for {SECRET}"),
        }),
        Err(HttpError::new("connection reset")),
    ]);
    let c = checker(Channel::Dev, "0.1.0", http);
    let _ = c.check();
    let _ = c.check();
    let _ = c.check_quietly();

    // Device flow: the poll answer carries a token, and it is stored.
    let http = MockHttp::with(vec![
        ok(
            r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","expires_in":900,"interval":0}"#,
        ),
        ok(&format!(
            r#"{{"access_token":"{SECRET}","token_type":"bearer","scope":"repo"}}"#
        )),
    ]);
    let store = Arc::new(MemorySecretStore::default());
    let auth = GithubAuth::new("client-id".into(), http, store.clone()).without_sleeping();
    let signed_in = auth.device_sign_in(&mut NoUi).unwrap();
    let _ = format!("{signed_in:?} {signed_in}");
    assert_eq!(auth.stored_token().unwrap().unwrap().expose(), SECRET);

    let lines = logs.0.lock().unwrap().clone();
    assert!(
        !lines.is_empty(),
        "the code under test should log something"
    );
    for line in &lines {
        assert!(!line.contains(SECRET), "token leaked into log: {line}");
        assert!(
            !line.contains(REFRESH_SECRET),
            "refresh token leaked: {line}"
        );
    }
}

#[test]
fn device_flow_polls_until_approved() {
    let http = MockHttp::with(vec![
        ok(
            r#"{"device_code":"dc","user_code":"WXYZ-0001","verification_uri":"https://github.com/login/device","expires_in":900,"interval":5}"#,
        ),
        ok(r#"{"error":"authorization_pending"}"#),
        ok(r#"{"error":"slow_down"}"#),
        ok(r#"{"access_token":"tok","token_type":"bearer"}"#),
    ]);
    let store = Arc::new(MemorySecretStore::default());
    let auth = GithubAuth::new("abc".into(), http.clone(), store).without_sleeping();
    struct Ui(Vec<String>);
    impl DeviceFlowUi for Ui {
        fn show_code(&mut self, code: &str, uri: &str) {
            self.0.push(format!("{code} {uri}"));
        }
    }
    let mut ui = Ui(vec![]);
    let token = auth.device_sign_in(&mut ui).unwrap();
    assert_eq!(token.expose(), "tok");
    assert_eq!(
        ui.0,
        vec!["WXYZ-0001 https://github.com/login/device".to_string()]
    );
    let requests = http.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].2.as_deref().unwrap().contains("client_id=abc"));
    assert!(!requests[0].2.as_deref().unwrap().contains("scope"));
    assert!(requests[1].2.as_deref().unwrap().contains("device_code=dc"));
}

#[test]
fn placeholder_client_id_is_a_clear_error() {
    let err = parse_client_id("REPLACE_WITH_CLIENT_ID\n").unwrap_err();
    assert!(err.to_string().contains("config/github_app_client_id.txt"));
    assert!(parse_client_id("   \n").is_err());
    assert_eq!(parse_client_id(" Iv1.abc123 \n").unwrap(), "Iv1.abc123");
}

#[test]
fn http_response_debug_hides_body() {
    let response = HttpResponse {
        status: 200,
        body: format!(r#"{{"access_token":"{SECRET}"}}"#),
    };
    let shown = format!("{response:?}");
    assert!(!shown.contains(SECRET));
    assert!(!shown.contains("access_token"));
    assert!(shown.contains("200"));
}

#[test]
fn api_base_must_be_https_or_local() {
    let new = || UpdateChecker::new(STUDIO_FEED, Channel::Dev, Arc::new(Token::new("t".into())));
    assert!(new().with_api_base("https://api.github.com".into()).is_ok());
    assert!(new().with_api_base("http://127.0.0.1:8080".into()).is_ok());
    assert!(new().with_api_base("http://localhost:8080/".into()).is_ok());
    assert!(new().with_api_base("http://api.github.com".into()).is_err());
    assert!(
        new()
            .with_api_base("http://127.0.0.1.evil.example".into())
            .is_err()
    );
    assert!(
        new()
            .with_api_base("http://localhost@evil.example".into())
            .is_err()
    );
    assert!(new().with_api_base("ftp://x".into()).is_err());
}

#[test]
fn reads_up_to_five_pages_without_certifying_truncated_history() {
    let full_page_of_stable = || {
        let all: Vec<String> = (0..50)
            .map(|i| release(&format!("v1.0.{i}-stable"), ""))
            .collect();
        ok(&feed(&all))
    };
    // Match on page 2.
    let http = MockHttp::with(vec![
        full_page_of_stable(),
        ok(&feed(&[release("v0.5.0-dev", "")])),
    ]);
    let update = checker(Channel::Dev, "0.1.0", http.clone())
        .check()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, Version::new(0, 5, 0));
    assert!(http.requests.lock().unwrap()[1].0.ends_with("&page=2"));

    // No channel boundary in twenty full pages of other releases: bounded, not proven up to
    // date, and not forced either (none of them are this channel's).
    let http = MockHttp::with((0..25).map(|_| full_page_of_stable()).collect());
    assert!(matches!(
        checker(Channel::Dev, "0.1.0", http.clone()).check(),
        Err(UpdateError::BadResponse(_))
    ));
    assert_eq!(http.requests.lock().unwrap().len(), 20);

    // A boundary on the final allowed full page is still complete.
    let mut answers: Vec<_> = (0..4).map(|_| full_page_of_stable()).collect();
    let mut fifth = fifty_newer_dev_releases(0);
    fifth[49] = release("v0.1.0-dev", "");
    answers.push(ok(&feed(&fifth)));
    let http = MockHttp::with(answers);
    assert_eq!(
        checker(Channel::Dev, "0.1.0", http.clone())
            .check()
            .unwrap()
            .unwrap()
            .version,
        Version::new(0, 1, 1_000)
    );
    assert_eq!(http.requests.lock().unwrap().len(), 5);
}

fn device_code_answer() -> Result<HttpResponse, HttpError> {
    ok(
        r#"{"device_code":"dc","user_code":"AAAA-0000","verification_uri":"https://github.com/login/device","expires_in":900,"interval":1}"#,
    )
}

#[test]
fn polling_retries_three_transient_errors() {
    let token = r#"{"access_token":"tok","token_type":"bearer"}"#;
    let five_hundred = || {
        Ok(HttpResponse {
            status: 502,
            body: String::new(),
        })
    };
    let store = Arc::new(MemorySecretStore::default());

    let http = MockHttp::with(vec![
        device_code_answer(),
        Err(HttpError::new("reset")),
        five_hundred(),
        Err(HttpError::new("reset")),
        ok(token),
    ]);
    let auth = GithubAuth::new("abc".into(), http, store.clone()).without_sleeping();
    assert_eq!(auth.device_sign_in(&mut NoUi).unwrap().expose(), "tok");

    let http = MockHttp::with(vec![
        device_code_answer(),
        Err(HttpError::new("reset")),
        five_hundred(),
        Err(HttpError::new("reset")),
        Err(HttpError::new("reset")),
        ok(token),
    ]);
    let auth = GithubAuth::new("abc".into(), http, store).without_sleeping();
    assert!(auth.device_sign_in(&mut NoUi).is_err());
}

#[test]
fn expired_access_token_is_refreshed() {
    let store = Arc::new(MemorySecretStore::default());
    store
        .set(r#"{"access_token":"old","refresh_token":"r1","expires_at":1}"#)
        .unwrap();
    let http = MockHttp::with(vec![ok(
        r#"{"access_token":"new","refresh_token":"r2","expires_in":28800,"refresh_token_expires_in":15897600}"#,
    )]);
    let auth = GithubAuth::new("abc".into(), http.clone(), store.clone()).without_sleeping();
    assert_eq!(auth.valid_token().unwrap().unwrap().expose(), "new");
    let body = http.requests.lock().unwrap()[0].2.clone().unwrap();
    assert!(body.contains("grant_type=refresh_token"));
    assert!(body.contains("refresh_token=r1"));
    assert!(body.contains("client_id=abc"));
    let saved = store.get().unwrap().unwrap();
    assert!(saved.contains("r2") && !saved.contains("\"r1\""));

    // A fresh token is returned without any request.
    assert_eq!(auth.valid_token().unwrap().unwrap().expose(), "new");
    assert_eq!(http.requests.lock().unwrap().len(), 1);

    // Nothing stored: no token, no request.
    let empty = GithubAuth::new(
        "abc".into(),
        MockHttp::with(vec![]),
        Arc::new(MemorySecretStore::default()),
    );
    assert!(empty.valid_token().unwrap().is_none());
}

#[test]
fn expired_without_refresh_token_asks_to_sign_in_again() {
    let store = Arc::new(MemorySecretStore::default());
    store
        .set(r#"{"access_token":"old","expires_at":1}"#)
        .unwrap();
    let auth = GithubAuth::new("abc".into(), MockHttp::with(vec![]), store);
    assert!(auth.valid_token().unwrap().is_none());
}

/// #206: a network blip while refreshing a token that still works keeps the token instead of
/// asking to sign in again. An already expired token still needs the refresh.
#[test]
fn refresh_network_blip_keeps_the_still_valid_token() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let store = Arc::new(MemorySecretStore::default());
    store
        .set(&format!(
            r#"{{"access_token":"old","refresh_token":"r1","expires_at":{}}}"#,
            now + 120
        ))
        .unwrap();
    let offline = || Err(HttpError::new("offline"));
    let http = MockHttp::with(vec![offline(), offline(), offline(), offline()]);
    let auth = GithubAuth::new("abc".into(), http.clone(), store.clone()).without_sleeping();
    assert_eq!(auth.valid_token().unwrap().unwrap().expose(), "old");
    assert_eq!(
        http.requests.lock().unwrap().len(),
        4,
        "it did try to refresh"
    );
    assert!(
        store.get().unwrap().unwrap().contains("r1"),
        "the session is kept"
    );

    store
        .set(r#"{"access_token":"old","refresh_token":"r1","expires_at":1}"#)
        .unwrap();
    let http = MockHttp::with(vec![offline(), offline(), offline(), offline()]);
    let auth = GithubAuth::new("abc".into(), http, store).without_sleeping();
    assert!(matches!(auth.valid_token(), Err(AuthError::Network(_))));
}

/// #376 F1: the "keep the token" decision uses the clock after the retries, with a minute of
/// margin: a token that ran out while the refresh retried is not handed out.
#[test]
fn refresh_failure_rereads_the_clock_and_keeps_a_margin() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let offline = || Err(HttpError::new("offline"));
    let session = |expires_at: u64| {
        let store = Arc::new(MemorySecretStore::default());
        store
            .set(&format!(
                r#"{{"access_token":"old","refresh_token":"r1","expires_at":{expires_at}}}"#
            ))
            .unwrap();
        store
    };
    // Every reading is 100 s later: the token had 150 s at the start, 50 s after the retries.
    let now = Arc::new(AtomicU64::new(1_000));
    let ticking = {
        let now = now.clone();
        Arc::new(move || now.fetch_add(100, Ordering::SeqCst))
    };
    let http = MockHttp::with(vec![offline(), offline(), offline(), offline()]);
    let auth = GithubAuth::new("abc".into(), http, session(1_150))
        .without_sleeping()
        .with_clock(ticking);
    assert!(matches!(auth.valid_token(), Err(AuthError::Network(_))));
    // A fixed clock: 59 s left is not enough, 61 s is.
    for (left, kept) in [(59, false), (61, true)] {
        let http = MockHttp::with(vec![offline(), offline(), offline(), offline()]);
        let auth = GithubAuth::new("abc".into(), http, session(1_000 + left))
            .without_sleeping()
            .with_clock(Arc::new(|| 1_000));
        assert_eq!(auth.valid_token().is_ok(), kept, "{left} s left");
    }
    // Only a network error or a 5xx keeps the token: a 400 does not.
    let http = MockHttp::with(vec![Ok(HttpResponse {
        status: 400,
        body: String::new(),
    })]);
    let auth = GithubAuth::new("abc".into(), http, session(1_200))
        .without_sleeping()
        .with_clock(Arc::new(|| 1_000));
    assert!(matches!(auth.valid_token(), Err(AuthError::BadResponse(_))));
}

/// #376 F2: a cancel during the retries of a request ends the sign-in there.
#[test]
fn device_sign_in_cancel_is_seen_during_retries() {
    /// Fails every request and sets the cancel on the second one.
    struct CancelOnRetry<'a>(&'a std::sync::atomic::AtomicBool, Mutex<u32>);
    impl HttpClient for CancelOnRetry<'_> {
        fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
            Err(HttpError::new("unexpected GET"))
        }
        fn post_form(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &[(&str, &str)],
        ) -> Result<HttpResponse, HttpError> {
            let mut calls = self.1.lock().unwrap();
            *calls += 1;
            if *calls == 2 {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Err(HttpError::new("offline"))
        }
    }
    static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let http = Arc::new(CancelOnRetry(&CANCEL, Mutex::new(0)));
    let auth = GithubAuth::new(
        "abc".into(),
        http.clone(),
        Arc::new(MemorySecretStore::default()),
    );
    let started = std::time::Instant::now();
    let result = auth.device_sign_in_until(&mut NoUi, &CANCEL);
    assert!(matches!(result, Err(AuthError::Cancelled)), "{result:?}");
    assert_eq!(*http.1.lock().unwrap(), 2, "no third try after the cancel");
    // The 2 s back-off after the second failure was cut short.
    assert!(started.elapsed() < Duration::from_millis(1_800));
}

/// #376 F2: `cancel_sign_in` and the save share a lock. A cancel that wins stores nothing; a
/// cancel that comes once the token is saved leaves a finished sign-in.
#[test]
fn a_token_after_the_cancel_is_not_stored() {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    /// Answers the code, then holds the token answer until the test has cancelled.
    struct Gate {
        calls: Mutex<u32>,
        release: Mutex<Option<mpsc::Receiver<()>>>,
        arrived: Mutex<Option<mpsc::Sender<()>>>,
    }
    impl HttpClient for Gate {
        fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
            Err(HttpError::new("unexpected GET"))
        }
        fn post_form(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &[(&str, &str)],
        ) -> Result<HttpResponse, HttpError> {
            let first = {
                let mut calls = self.calls.lock().unwrap();
                *calls += 1;
                *calls == 1
            };
            if first {
                return device_code_answer();
            }
            if let Some(tx) = self.arrived.lock().unwrap().take() {
                let _ = tx.send(());
            }
            if let Some(rx) = self.release.lock().unwrap().take() {
                let _ = rx.recv();
            }
            ok(r#"{"access_token":"tok","token_type":"bearer"}"#)
        }
    }
    let (release_tx, release_rx) = mpsc::channel();
    let (arrived_tx, arrived_rx) = mpsc::channel();
    let store = Arc::new(MemorySecretStore::default());
    let auth = Arc::new(
        GithubAuth::new(
            "abc".into(),
            Arc::new(Gate {
                calls: Mutex::new(0),
                release: Mutex::new(Some(release_rx)),
                arrived: Mutex::new(Some(arrived_tx)),
            }),
            store.clone(),
        )
        .without_sleeping(),
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let worker = {
        let (auth, cancel) = (auth.clone(), cancel.clone());
        std::thread::spawn(move || auth.device_sign_in_until(&mut NoUi, &cancel))
    };
    // The token request is on the wire; cancel, then let the answer arrive.
    arrived_rx.recv().unwrap();
    auth.cancel_sign_in(&cancel);
    release_tx.send(()).unwrap();
    let result = worker.join().unwrap();
    assert!(matches!(result, Err(AuthError::Cancelled)), "{result:?}");
    assert!(
        store.get().unwrap().is_none(),
        "nothing stored after the cancel"
    );
}

/// Sets the cancel flag when the code is shown, as a user pressing Cancel would.
struct CancelOnCode<'a>(&'a std::sync::atomic::AtomicBool);

impl DeviceFlowUi for CancelOnCode<'_> {
    fn show_code(&mut self, _: &str, _: &str) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// #206: a device sign-in can be cancelled; it stops polling and stores nothing.
#[test]
fn device_sign_in_can_be_cancelled() {
    let store = Arc::new(MemorySecretStore::default());
    let http = MockHttp::with(vec![
        device_code_answer(),
        ok(r#"{"access_token":"tok","token_type":"bearer"}"#),
    ]);
    let auth = GithubAuth::new("abc".into(), http.clone(), store.clone());
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let started = std::time::Instant::now();
    let result = auth.device_sign_in_until(&mut CancelOnCode(&cancel), &cancel);
    assert!(matches!(result, Err(AuthError::Cancelled)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(1), "no poll wait");
    assert_eq!(http.requests.lock().unwrap().len(), 1, "no poll was sent");
    assert!(store.get().unwrap().is_none());
}

/// Counts refresh calls and answers slowly, so two racing callers overlap.
struct SlowRefreshHttp {
    calls: Mutex<u32>,
}

impl HttpClient for SlowRefreshHttp {
    fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        Err(HttpError::new("unexpected GET"))
    }

    fn post_form(
        &self,
        _: &str,
        _: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        std::thread::sleep(Duration::from_millis(200));
        // A rotating refresh token works once; the second use is refused.
        let used_old = form.contains(&("refresh_token", "r1"));
        let body = if *calls == 1 && used_old {
            r#"{"access_token":"new","refresh_token":"r2","expires_in":28800,"refresh_token_expires_in":15897600}"#
        } else {
            r#"{"error":"bad_refresh_token"}"#
        };
        ok(body)
    }
}

#[test]
fn concurrent_valid_token_refreshes_once() {
    let store = Arc::new(MemorySecretStore::default());
    store
        .set(r#"{"access_token":"old","refresh_token":"r1","expires_at":1}"#)
        .unwrap();
    let http = Arc::new(SlowRefreshHttp {
        calls: Mutex::new(0),
    });
    let auth =
        Arc::new(GithubAuth::new("abc".into(), http.clone(), store.clone()).without_sleeping());
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let auth = auth.clone();
            std::thread::spawn(move || auth.valid_token().unwrap().unwrap().expose().to_string())
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), "new");
    }
    assert_eq!(*http.calls.lock().unwrap(), 1);
    assert!(
        store.get().unwrap().unwrap().contains("r2"),
        "session must survive"
    );
}

#[test]
fn only_token_shaped_text_counts_as_a_bare_token() {
    let store = Arc::new(MemorySecretStore::default());
    let auth = GithubAuth::new("abc".into(), MockHttp::with(vec![]), store.clone());
    for junk in [
        "hello world",
        "not json {",
        "",
        "ghu_has space",
        "xyz_abc123",
    ] {
        store.set(junk).unwrap();
        assert!(auth.valid_token().unwrap().is_none(), "{junk:?}");
        assert!(auth.stored_token().unwrap().is_none(), "{junk:?}");
    }
    store.set("ghu_AbC123xyz").unwrap();
    assert_eq!(
        auth.valid_token().unwrap().unwrap().expose(),
        "ghu_AbC123xyz"
    );
}

#[test]
fn refresh_tokens_never_logged() {
    let logs = capture();
    let store = Arc::new(MemorySecretStore::default());
    store
        .set(&format!(
            r#"{{"access_token":"{SECRET}","refresh_token":"{REFRESH_SECRET}","expires_at":1}}"#
        ))
        .unwrap();
    let http = MockHttp::with(vec![ok(&format!(
        r#"{{"access_token":"{SECRET}2","refresh_token":"{REFRESH_SECRET}2","expires_in":28800}}"#
    ))]);
    let auth = GithubAuth::new("abc".into(), http, store).without_sleeping();
    assert!(auth.valid_token().unwrap().is_some());
    let lines = logs.0.lock().unwrap().clone();
    assert!(
        lines.iter().any(|l| l.contains("refreshed")),
        "refresh should log"
    );
    for line in &lines {
        assert!(
            !line.contains(SECRET) && !line.contains(REFRESH_SECRET),
            "{line}"
        );
    }
}

fn mixed_feed() -> String {
    feed(&[
        release("v0.2.0-dev", ""),
        release("launcher-v0.1.5-dev", ""),
        release("v0.1.0-dev", ""),
    ])
}

#[test]
fn launcher_feed_ignores_studio_tags() {
    let http = MockHttp::with(vec![ok(&mixed_feed())]);
    let checker = UpdateChecker::new(
        LAUNCHER_FEED,
        Channel::Dev,
        Arc::new(Token::new("t".into())),
    )
    .with_http(http)
    .with_current_version(Version::new(0, 1, 0));
    assert_eq!(
        checker.check().unwrap().unwrap().version,
        Version::new(0, 1, 5)
    );
}

#[test]
fn studio_feed_ignores_launcher_tags() {
    let http = MockHttp::with(vec![ok(&mixed_feed())]);
    let checker = UpdateChecker::new(STUDIO_FEED, Channel::Dev, Arc::new(Token::new("t".into())))
        .with_http(http)
        .with_current_version(Version::new(0, 1, 0));
    assert_eq!(
        checker.check().unwrap().unwrap().version,
        Version::new(0, 2, 0)
    );
}

#[test]
fn velopack_channel_name_has_feed_prefix() {
    assert_eq!(LAUNCHER_FEED.velopack_channel(Channel::Dev), "launcher-dev");
    assert_eq!(STUDIO_FEED.velopack_channel(Channel::Stable), "stable");
}

// ---- release files: lookup and verified download ---------------------------------------------

const SETUP: &str = "MetallicStudio-dev-Setup.exe";

fn digest_text(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn digest_text_of(asset: &ReleaseAsset) -> String {
    let hex: String = asset.sha256.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn release_with_asset(tag: &str, asset_name: &str, digest: Option<&str>) -> String {
    let mut asset = serde_json::json!({
        "name": asset_name,
        "url": "https://api.github.com/repos/Metallic-Club/studio/releases/assets/77",
        "size": 1234,
    });
    if let Some(digest) = digest {
        asset["digest"] = digest.into();
    }
    serde_json::json!({"tag_name": tag, "draft": false, "body": "", "assets": [asset]}).to_string()
}

#[test]
fn latest_asset_reads_digest() {
    let wanted = digest_text(b"hello");
    let http = MockHttp::with(vec![ok(&feed(&[
        // The newest release has no setup file; the one before it has.
        release("v0.1.5-dev", ""),
        release_with_asset("v0.1.4-dev", SETUP, Some(&wanted)),
        release_with_asset("v0.1.3-dev", SETUP, Some(&digest_text(b"older"))),
        release_with_asset("v0.1.6-stable", SETUP, Some(&digest_text(b"stable"))),
    ]))]);
    let asset = checker(Channel::Dev, "0.1.0", http)
        .latest_asset(SETUP)
        .unwrap()
        .unwrap();
    assert_eq!(asset.version, Version::new(0, 1, 4));
    assert_eq!(asset.name, SETUP);
    assert_eq!(asset.size, 1234);
    assert_eq!(digest_text_of(&asset), wanted);
    assert!(asset.api_url.starts_with("https://api.github.com/"));
}

#[test]
fn latest_asset_is_none_when_no_release_has_the_file() {
    let http = MockHttp::with(vec![ok(&feed(&[release("v0.1.5-dev", "")]))]);
    let found = checker(Channel::Dev, "0.1.0", http).latest_asset(SETUP);
    assert!(matches!(found, Ok(None)));
}

#[test]
fn asset_without_digest_is_refused() {
    for digest in [
        None,
        Some("md5:abcd"),
        Some("sha256:abcd"),
        Some("sha256:zz"),
    ] {
        let http = MockHttp::with(vec![ok(&feed(&[release_with_asset(
            "v0.1.4-dev",
            SETUP,
            digest,
        )]))]);
        let found = checker(Channel::Dev, "0.1.0", http).latest_asset(SETUP);
        assert!(
            matches!(found, Err(UpdateError::BadResponse(_))),
            "{digest:?}"
        );
    }
}

#[test]
fn asset_address_outside_the_api_is_refused() {
    let mut value: serde_json::Value = serde_json::from_str(&release_with_asset(
        "v0.1.4-dev",
        SETUP,
        Some(&digest_text(b"x")),
    ))
    .unwrap();
    value["assets"][0]["url"] = "https://evil.example/file".into();
    let http = MockHttp::with(vec![ok(&format!("[{value}]"))]);
    let found = checker(Channel::Dev, "0.1.0", http).latest_asset(SETUP);
    assert!(matches!(found, Err(UpdateError::BadResponse(_))));
}

/// A tiny HTTP server on 127.0.0.1: answers each connection with `reply(request_text)` and
/// records the request texts (lower-cased).
fn serve(
    connections: usize,
    reply: impl Fn(&str) -> Vec<u8> + Send + 'static,
) -> (u16, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let handle = std::thread::spawn(move || {
        for _ in 0..connections {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = Vec::new();
            let mut buf = [0u8; 2048];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let text = String::from_utf8_lossy(&request).to_lowercase();
            let answer = reply(&text);
            recorded.lock().unwrap().push(text);
            // The client may hang up early (an oversized body); that is not an error here.
            let _ = stream.write_all(&answer);
        }
    });
    (port, seen, handle)
}

fn ok_bytes(body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn asset_for(bytes: &[u8], url: String) -> ReleaseAsset {
    ReleaseAsset {
        version: Version::new(0, 1, 4),
        name: SETUP.to_string(),
        api_url: url,
        sha256: Sha256::digest(bytes).into(),
        size: bytes.len() as u64,
    }
}

fn client() -> UreqClient {
    UreqClient::new(Duration::from_secs(5))
}

fn folder_is_empty(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir).unwrap().next().is_none()
}

#[test]
fn download_writes_a_verified_file_and_reports_progress() {
    let bytes = vec![7u8; 300_000];
    let body = bytes.clone();
    let (port, seen, server) = serve(1, move |_| ok_bytes(&body));
    let dir = tempfile::tempdir().unwrap();
    let token = Token::new("tok-for-github".into());
    let mut percents = Vec::new();
    let path = download_verified(
        &client(),
        Some(&token),
        &asset_for(&bytes, format!("http://127.0.0.1:{port}/asset")),
        dir.path(),
        |p| percents.push(p),
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!dir.path().join(format!("{SETUP}.part")).exists());
    assert_eq!(percents.last(), Some(&100));
    assert!(percents.windows(2).all(|w| w[0] < w[1]), "{percents:?}");
    let request = seen.lock().unwrap()[0].clone();
    assert!(request.contains("accept: application/octet-stream"));
    assert!(request.contains("authorization: bearer tok-for-github"));
}

#[test]
fn digest_mismatch_deletes_file() {
    let expected = vec![1u8; 5000];
    let served = vec![2u8; 5000];
    let (port, _seen, server) = serve(1, move |_| ok_bytes(&served));
    let dir = tempfile::tempdir().unwrap();
    let result = download_verified(
        &client(),
        None,
        &asset_for(&expected, format!("http://127.0.0.1:{port}/asset")),
        dir.path(),
        |_| {},
    );
    server.join().unwrap();
    assert!(matches!(result, Err(UpdateError::Install(_))), "{result:?}");
    assert!(folder_is_empty(dir.path()), "no file and no .part is left");
}

#[test]
fn oversized_download_stops() {
    let expected = vec![1u8; 5000];
    let served = vec![1u8; 5001];
    let (port, _seen, server) = serve(1, move |_| ok_bytes(&served));
    let dir = tempfile::tempdir().unwrap();
    let result = download_verified(
        &client(),
        None,
        &asset_for(&expected, format!("http://127.0.0.1:{port}/asset")),
        dir.path(),
        |_| {},
    );
    server.join().unwrap();
    assert!(result.is_err());
    assert!(folder_is_empty(dir.path()));
}

#[test]
fn short_download_is_refused() {
    let expected = vec![1u8; 5000];
    let served = vec![1u8; 4000];
    let (port, _seen, server) = serve(1, move |_| ok_bytes(&served));
    let dir = tempfile::tempdir().unwrap();
    let result = download_verified(
        &client(),
        None,
        &asset_for(&expected, format!("http://127.0.0.1:{port}/asset")),
        dir.path(),
        |_| {},
    );
    server.join().unwrap();
    assert!(result.is_err());
    assert!(folder_is_empty(dir.path()));
}

#[test]
fn error_status_is_refused() {
    let (port, _seen, server) = serve(1, |_| {
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let dir = tempfile::tempdir().unwrap();
    let result = download_verified(
        &client(),
        None,
        &asset_for(b"abc", format!("http://127.0.0.1:{port}/asset")),
        dir.path(),
        |_| {},
    );
    server.join().unwrap();
    assert!(result.is_err());
    assert!(folder_is_empty(dir.path()));
}

#[test]
fn partial_download_is_not_used() {
    let bytes = vec![9u8; 4096];
    let body = bytes.clone();
    let (port, _seen, server) = serve(1, move |_| ok_bytes(&body));
    let dir = tempfile::tempdir().unwrap();
    // A killed run left half a file, and even a file with the final name.
    let stale = dir.path().join(format!("{SETUP}.part"));
    std::fs::write(&stale, b"half of an installer").unwrap();
    std::fs::write(dir.path().join(SETUP), b"not verified").unwrap();
    let path = download_verified(
        &client(),
        None,
        &asset_for(&bytes, format!("http://127.0.0.1:{port}/asset")),
        dir.path(),
        |_| {},
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(
        std::fs::read(path).unwrap(),
        bytes,
        "the new bytes, verified"
    );
    assert!(!stale.exists());
}

#[test]
fn token_not_forwarded_on_redirect() {
    let bytes = vec![5u8; 2048];
    let body = bytes.clone();
    // Server B (another port, so another host:port) serves the file and records what it gets.
    let (port_b, seen_b, server_b) = serve(1, move |_| ok_bytes(&body));
    let (port_a, seen_a, server_a) = serve(1, move |_| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port_b}/blob\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });
    let dir = tempfile::tempdir().unwrap();
    let token = Token::new("tok-must-stay-home".into());
    download_verified(
        &client(),
        Some(&token),
        &asset_for(&bytes, format!("http://127.0.0.1:{port_a}/asset")),
        dir.path(),
        |_| {},
    )
    .unwrap();
    server_a.join().unwrap();
    server_b.join().unwrap();
    assert!(seen_a.lock().unwrap()[0].contains("authorization: bearer tok-must-stay-home"));
    let at_b = seen_b.lock().unwrap()[0].clone();
    assert!(!at_b.contains("authorization"), "{at_b}");
    assert!(!at_b.contains("tok-must-stay-home"));
    assert!(at_b.contains("accept: application/octet-stream"));
}

#[test]
fn redirect_to_plain_http_elsewhere_is_refused() {
    let (port, _seen, server) = serve(1, |_| {
        b"HTTP/1.1 302 Found\r\nLocation: http://files.example/blob\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    });
    let dir = tempfile::tempdir().unwrap();
    let result = download_verified(
        &client(),
        Some(&Token::new("t".into())),
        &asset_for(b"abc", format!("http://127.0.0.1:{port}/asset")),
        dir.path(),
        |_| {},
    );
    server.join().unwrap();
    assert!(result.is_err());
    assert!(folder_is_empty(dir.path()));
}

#[test]
fn download_refuses_names_that_leave_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["..\\x.exe", "../x.exe", "C:x.exe", "", "a.part"] {
        let mut asset = asset_for(b"abc", "http://127.0.0.1:1/a".into());
        asset.name = name.to_string();
        let result = download_verified(&client(), None, &asset, dir.path(), |_| {});
        assert!(
            matches!(result, Err(UpdateError::BadResponse(_))),
            "{name:?}"
        );
    }
}

#[test]
fn streaming_is_off_unless_a_client_implements_it() {
    let http = MockHttp::with(vec![]);
    let dir = tempfile::tempdir().unwrap();
    let result = download_verified(
        http.as_ref(),
        None,
        &asset_for(b"abc", "https://api.github.com/x".into()),
        dir.path(),
        |_| {},
    );
    assert!(matches!(result, Err(UpdateError::Network(_))));
    assert!(folder_is_empty(dir.path()));
}

#[test]
fn stalled_download_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Sends the headers and a little of the body, then goes quiet with the connection open.
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf);
        let head = "HTTP/1.1 200 OK\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n";
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&[1u8; 100]).unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_secs(3));
    });
    let dir = tempfile::tempdir().unwrap();
    let http =
        UreqClient::new(Duration::from_secs(5)).with_stall_timeout(Duration::from_millis(400));
    let started = std::time::Instant::now();
    let result = download_verified(
        &http,
        None,
        &asset_for(&[1u8; 5000], format!("http://127.0.0.1:{port}/asset")),
        dir.path(),
        |_| {},
    );
    assert!(matches!(result, Err(UpdateError::Network(_))), "{result:?}");
    assert!(
        started.elapsed() < Duration::from_millis(2500),
        "gave up on its own, without waiting for the server"
    );
    assert!(folder_is_empty(dir.path()), "no file and no .part is left");
    server.join().unwrap();
}

#[test]
fn hostile_file_names_are_refused_even_with_odd_spelling() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "setup.exe.",
        "setup.exe ",
        "NUL.exe",
        "a*b.exe",
        "x.PART",
        "a|b",
    ] {
        let mut asset = asset_for(b"abc", "http://127.0.0.1:1/a".into());
        asset.name = name.to_string();
        let result = download_verified(&client(), None, &asset, dir.path(), |_| {});
        assert!(
            matches!(result, Err(UpdateError::BadResponse(_))),
            "{name:?}"
        );
    }
}
