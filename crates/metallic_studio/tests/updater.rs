//! Tests for the update checker and GitHub sign-in. All HTTP is mocked or goes to 127.0.0.1;
//! nothing touches the internet or the real Windows Credential Manager.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use metallic_studio::github_auth::{
    DeviceFlowUi, GithubAuth, MemorySecretStore, SecretStore, parse_client_id,
};
use metallic_studio::updater::{
    Channel, HttpClient, HttpError, HttpResponse, Token, UpdateChecker, UpdateError, UreqClient,
};
use semver::Version;

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

fn checker(channel: Channel, current: &str, http: Arc<MockHttp>) -> UpdateChecker {
    UpdateChecker::new(channel, Arc::new(Token::new(SECRET.to_string())))
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
            .ends_with("/repos/Metallic-Club/studio/releases?per_page=50")
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
    let c = UpdateChecker::new(Channel::Dev, Arc::new(Token::new("t".into())))
        .with_api_base(format!("http://127.0.0.1:{port}"))
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
    let c = UpdateChecker::new(Channel::Dev, Arc::new(Token::new("tok123".into())))
        .with_api_base(format!("http://127.0.0.1:{port}"))
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
    assert_eq!(store.get().unwrap().as_deref(), Some(SECRET));

    let lines = logs.0.lock().unwrap().clone();
    assert!(
        !lines.is_empty(),
        "the code under test should log something"
    );
    for line in &lines {
        assert!(!line.contains(SECRET), "token leaked into log: {line}");
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
        ok(r#"{"access_token":"tok","token_type":"bearer","scope":"repo"}"#),
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
    assert!(requests[0].2.as_deref().unwrap().contains("scope=repo"));
    assert!(requests[1].2.as_deref().unwrap().contains("device_code=dc"));
}

#[test]
fn placeholder_client_id_is_a_clear_error() {
    let err = parse_client_id("REPLACE_WITH_CLIENT_ID\n").unwrap_err();
    assert!(
        err.to_string()
            .contains("config/github_oauth_client_id.txt")
    );
    assert!(parse_client_id("   \n").is_err());
    assert_eq!(parse_client_id(" Iv1.abc123 \n").unwrap(), "Iv1.abc123");
}
