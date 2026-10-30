//! Tests for the Metallic sign-in (device flow) and the web-session handoff. HTTP is a scripted
//! fake or a server on 127.0.0.1; nothing touches the network or the Credential Manager.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use metallic_launcher::platform_auth::{CLIENT_ID, DeviceCode, SCOPES};
use metallic_launcher::{
    AppEvent, BridgeEvent, EventSink, LauncherError, OneAtATime, PlatformAuth, SiteOrigin,
    platform_sign_in,
};
use metallic_update::{
    AuthError, HttpClient, HttpError, HttpResponse, MemorySecretStore, SecretStore, UreqClient,
};
use url::Url;

const SITE: &str = "https://metallic.example";

/// A cancel flag that is never set.
fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// One recorded request: url, headers, form body.
type Recorded = (String, Vec<(String, String)>, String);

/// A scripted HTTP client: one canned answer per call, every request recorded.
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
    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

impl HttpClient for MockHttp {
    fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        Err(HttpError::new("GET is not used"))
    }
    fn post_form(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let headers = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let body = form
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        self.requests
            .lock()
            .unwrap()
            .push((url.to_string(), headers, body));
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(HttpError::new("no scripted answer left")))
    }
}

fn reply(status: u16, body: &str) -> Result<HttpResponse, HttpError> {
    Ok(HttpResponse {
        status,
        body: body.to_string(),
    })
}

fn ok(body: &str) -> Result<HttpResponse, HttpError> {
    reply(200, body)
}

fn site() -> SiteOrigin {
    SiteOrigin::parse(SITE, false).unwrap()
}

/// An auth whose waits are recorded (in seconds) instead of slept.
fn auth_with(
    http: Arc<MockHttp>,
    store: Arc<MemorySecretStore>,
) -> (PlatformAuth, Arc<Mutex<Vec<u64>>>) {
    let waits = Arc::new(Mutex::new(Vec::new()));
    let recorded = waits.clone();
    let auth = PlatformAuth::new(site(), http, store)
        .with_sleeper(move |d| recorded.lock().unwrap().push(d.as_secs()));
    (auth, waits)
}

fn device_code(extra: &str, interval: u64, expires_in: u64) -> String {
    format!(
        r#"{{"device_code":"dc-secret-1","user_code":"ABCD-1234","verification_uri":"{SITE}/device"{extra},"expires_in":{expires_in},"interval":{interval}}}"#
    )
}

/// Queues a device-code answer in front of the scripted ones and starts.
fn code_for(auth: &PlatformAuth, http: &MockHttp, body: String) -> DeviceCode {
    http.answers.lock().unwrap().push_front(ok(&body));
    auth.start().unwrap()
}

fn future(seconds: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + seconds
}

fn session(access: &str, refresh: &str, expires_in: u64) -> String {
    format!(
        r#"{{"access_token":"{access}","refresh_token":"{refresh}","expires_at":{}}}"#,
        future(expires_in)
    )
}

#[test]
fn device_flow_happy_path() {
    let http = MockHttp::with(vec![
        ok(&device_code(
            &format!(r#","verification_uri_complete":"{SITE}/device?user_code=ABCD-1234""#),
            5,
            900,
        )),
        ok(r#"{"error":"authorization_pending"}"#),
        ok(r#"{"access_token":"at-1","refresh_token":"rt-1","expires_in":3600}"#),
    ]);
    let store = Arc::new(MemorySecretStore::default());
    let (auth, waits) = auth_with(http.clone(), store.clone());

    let code = auth.start().unwrap();
    assert_eq!(code.user_code, "ABCD-1234");
    assert_eq!(
        auth.browser_address(&code).unwrap().as_str(),
        "https://metallic.example/device?user_code=ABCD-1234"
    );
    auth.poll_until_done(&code, &never()).unwrap();

    let requests = http.requests();
    assert_eq!(
        requests[0].0,
        "https://metallic.example/v1/oauth/device/code"
    );
    assert!(requests[0].2.contains(&format!("client_id={CLIENT_ID}")));
    assert!(requests[0].2.contains(&format!("scope={SCOPES}")));
    assert_eq!(requests[1].0, "https://metallic.example/v1/oauth/token");
    assert!(
        requests[1]
            .2
            .contains("grant_type=urn:ietf:params:oauth:grant-type:device_code")
    );
    assert!(requests[1].2.contains("device_code=dc-secret-1"));
    assert_eq!(*waits.lock().unwrap(), [5, 5]);
    assert_eq!(
        auth.access_token().unwrap().unwrap().expose(),
        "at-1",
        "the new session is stored and read back"
    );
    assert!(store.get().unwrap().unwrap().contains("rt-1"));
}

#[test]
fn oauth_errors_may_arrive_as_http_400() {
    let http = MockHttp::with(vec![
        reply(400, r#"{"error":"authorization_pending"}"#),
        reply(400, r#"{"error":"access_denied"}"#),
    ]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, device_code("", 1, 900));
    assert!(matches!(
        auth.poll_until_done(&code, &never()),
        Err(AuthError::Denied)
    ));
    assert_eq!(http.requests().len(), 3, "start, pending, denied");
}

#[test]
fn slow_down_increases_interval() {
    let http = MockHttp::with(vec![
        ok(r#"{"error":"authorization_pending"}"#),
        ok(r#"{"error":"slow_down"}"#),
        ok(r#"{"error":"authorization_pending"}"#),
        ok(r#"{"access_token":"at","expires_in":3600}"#),
    ]);
    let (auth, waits) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, device_code("", 2, 900));
    auth.poll_until_done(&code, &never()).unwrap();
    assert_eq!(*waits.lock().unwrap(), [2, 2, 7, 7]);
}

#[test]
fn transient_failures_are_retried_three_times_then_reported() {
    let http = MockHttp::with(vec![
        reply(503, ""),
        Err(HttpError::new("reset")),
        ok(r#"{"access_token":"at","expires_in":3600}"#),
    ]);
    let (auth, waits) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, device_code("", 1, 900));
    auth.poll_until_done(&code, &never()).unwrap();
    assert_eq!(*waits.lock().unwrap(), [1, 1, 2]);

    let http = MockHttp::with(vec![
        reply(502, ""),
        reply(502, ""),
        reply(502, ""),
        reply(502, ""),
    ]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, device_code("", 1, 900));
    assert!(matches!(
        auth.poll_until_done(&code, &never()),
        Err(AuthError::Network(_))
    ));
}

#[test]
fn expired_code_reports_expired() {
    // The server says so ...
    let http = MockHttp::with(vec![ok(r#"{"error":"expired_token"}"#)]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, device_code("", 1, 900));
    assert!(matches!(
        auth.poll_until_done(&code, &never()),
        Err(AuthError::Expired)
    ));
    // ... or the lifetime runs out while the user has not approved.
    let http = MockHttp::with(vec![
        ok(r#"{"error":"authorization_pending"}"#),
        ok(r#"{"error":"authorization_pending"}"#),
    ]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, device_code("", 5, 10));
    assert!(matches!(
        auth.poll_until_done(&code, &never()),
        Err(AuthError::Expired)
    ));
}

#[test]
fn denied_reports_denied() {
    let http = MockHttp::with(vec![ok(r#"{"error":"access_denied"}"#)]);
    let store = Arc::new(MemorySecretStore::default());
    let (auth, _) = auth_with(http.clone(), store.clone());
    let code = code_for(&auth, &http, device_code("", 1, 900));
    assert!(matches!(
        auth.poll_until_done(&code, &never()),
        Err(AuthError::Denied)
    ));
    assert!(store.get().unwrap().is_none());
}

#[test]
fn cancel_stops_the_poll() {
    let http = MockHttp::with(vec![]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, device_code("", 1, 900));
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        auth.poll_until_done(&code, &cancel),
        Err(AuthError::Cancelled)
    ));
    assert_eq!(http.requests().len(), 1, "only the device-code request");
}

#[test]
fn refresh_rotates_and_stores_new_token() {
    let store = Arc::new(MemorySecretStore::default());
    store.set(&session("old-at", "old-rt", 10)).unwrap();
    let http = MockHttp::with(vec![ok(
        r#"{"access_token":"new-at","refresh_token":"new-rt","expires_in":3600}"#,
    )]);
    let (auth, _) = auth_with(http.clone(), store.clone());
    assert_eq!(auth.access_token().unwrap().unwrap().expose(), "new-at");

    let requests = http.requests();
    assert!(requests[0].2.contains("grant_type=refresh_token"));
    assert!(requests[0].2.contains("refresh_token=old-rt"));
    let raw = store.get().unwrap().unwrap();
    assert!(raw.contains("new-rt") && !raw.contains("old-rt"));
    // A fresh token is not refreshed again.
    assert_eq!(auth.access_token().unwrap().unwrap().expose(), "new-at");
    assert_eq!(http.requests().len(), 1);
}

#[test]
fn a_token_with_time_left_is_used_without_a_request() {
    let store = Arc::new(MemorySecretStore::default());
    store.set(&session("at", "rt", 3600)).unwrap();
    let http = MockHttp::with(vec![]);
    let (auth, _) = auth_with(http.clone(), store);
    assert_eq!(auth.access_token().unwrap().unwrap().expose(), "at");
    assert!(http.requests().is_empty());
}

#[test]
fn invalid_grant_clears_session() {
    let store = Arc::new(MemorySecretStore::default());
    store.set(&session("old-at", "old-rt", 1)).unwrap();
    // As HTTP 400, the usual shape for an OAuth error.
    let http = MockHttp::with(vec![reply(400, r#"{"error":"invalid_grant"}"#)]);
    let (auth, _) = auth_with(http, store.clone());
    assert!(auth.access_token().unwrap().is_none());
    assert!(store.get().unwrap().is_none());
}

#[test]
fn web_session_code_requires_signed_in() {
    let http = MockHttp::with(vec![]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    assert!(matches!(
        auth.web_session_code(),
        Err(AuthError::NotSignedIn)
    ));
    assert!(http.requests().is_empty(), "no request without a session");
}

#[test]
fn web_session_code_is_asked_with_the_bearer_and_builds_the_handoff_address() {
    let store = Arc::new(MemorySecretStore::default());
    store.set(&session("at-9", "rt", 3600)).unwrap();
    let http = MockHttp::with(vec![ok(
        r#"{"code":"wsc_AbC123-x","expiresAt":"2026-10-30T10:00:00Z"}"#,
    )]);
    let (auth, _) = auth_with(http.clone(), store);
    let code = auth.web_session_code().unwrap();
    let requests = http.requests();
    assert_eq!(
        requests[0].0,
        "https://metallic.example/v1/oauth/web-session"
    );
    assert!(
        requests[0]
            .1
            .contains(&("Authorization".to_string(), "Bearer at-9".to_string()))
    );
    let url = auth.handoff_url(&code).unwrap();
    assert_eq!(
        url.expose().as_str(),
        "https://metallic.example/auth/launcher-handoff?code=wsc_AbC123-x"
    );
}

#[test]
fn a_web_session_code_with_a_strange_shape_is_refused() {
    for bad in [
        r#"{"code":""}"#,
        r#"{"code":"a b"}"#,
        r#"{"code":"x&y=1"}"#,
        "{}",
    ] {
        let store = Arc::new(MemorySecretStore::default());
        store.set(&session("at", "rt", 3600)).unwrap();
        let (auth, _) = auth_with(MockHttp::with(vec![ok(bad)]), store);
        assert!(auth.web_session_code().is_err(), "{bad}");
    }
}

#[test]
fn verification_uri_off_site_is_not_opened() {
    for uri in [
        r#","verification_uri_complete":"https://evil.test/device?user_code=ABCD-1234""#,
        r#","verification_uri_complete":"http://metallic.example/device""#,
        r#","verification_uri_complete":"https://metallic.example.evil.test/device""#,
        r#","verification_uri_complete":"https://metallic.example@evil.test/device""#,
        r#","verification_uri_complete":"https://metallic.example:8443/device""#,
        r#","verification_uri_complete":"javascript:alert(1)""#,
    ] {
        let http = MockHttp::with(vec![]);
        let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
        let code = code_for(&auth, &http, device_code(uri, 1, 900));
        assert!(
            matches!(auth.browser_address(&code), Err(AuthError::UnsafeAddress)),
            "{uri}"
        );
    }
    // Through the bridge entry point: nothing is opened, nothing is polled, an error comes back.
    let http = MockHttp::with(vec![ok(&device_code(
        r#","verification_uri_complete":"https://evil.test/device""#,
        1,
        900,
    ))]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let opened = Arc::new(Mutex::new(0));
    let counter = opened.clone();
    let events = Arc::new(Events::default());
    let result = platform_sign_in(
        Arc::new(auth),
        events.clone(),
        Arc::new(move |_| *counter.lock().unwrap() += 1),
        &OneAtATime::new(),
    );
    assert!(result.is_err());
    assert_eq!(*opened.lock().unwrap(), 0);
    assert!(events.all().is_empty());
    assert_eq!(http.requests().len(), 1, "no poll after a refused address");
}

#[test]
fn sign_out_revokes_then_deletes_even_when_revoke_fails() {
    let store = Arc::new(MemorySecretStore::default());
    store
        .set(r#"{"access_token":"at","refresh_token":"rt-77"}"#)
        .unwrap();
    let http = MockHttp::with(vec![ok("{}")]);
    let (auth, _) = auth_with(http.clone(), store.clone());
    auth.sign_out().unwrap();
    let requests = http.requests();
    assert_eq!(requests[0].0, "https://metallic.example/v1/oauth/revoke");
    assert!(requests[0].2.contains("token=rt-77"));
    assert!(store.get().unwrap().is_none());

    store
        .set(r#"{"access_token":"at","refresh_token":"rt-77"}"#)
        .unwrap();
    let (auth, _) = auth_with(
        MockHttp::with(vec![Err(HttpError::new("down"))]),
        store.clone(),
    );
    auth.sign_out().unwrap();
    assert!(store.get().unwrap().is_none());
}

// ---- the bridge entry point ------------------------------------------------------------------

/// Collects what the Launcher would tell the window; a handoff address is kept apart.
#[derive(Default)]
struct Events {
    lines: Mutex<Vec<String>>,
    handoffs: Mutex<Vec<String>>,
}

impl Events {
    fn all(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
    fn wait_for(&self, text: &str) {
        for _ in 0..300 {
            if self.all().iter().any(|l| l == text) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("never saw {text:?}; saw {:?}", self.all());
    }
}

impl EventSink for Events {
    fn send(&self, event: AppEvent) {
        let line = match event {
            AppEvent::Bridge(BridgeEvent::SignIn { state, user_code }) => {
                format!("signIn {state} {}", user_code.unwrap_or_default())
            }
            AppEvent::Handoff(url) => {
                self.handoffs.lock().unwrap().push(url.expose().to_string());
                format!("{:?}", AppEvent::Handoff(url))
            }
            other => format!("{other:?}"),
        };
        self.lines.lock().unwrap().push(line);
    }
}

type Flow = (Arc<Events>, Arc<MockHttp>, Arc<Mutex<Vec<String>>>);

fn started_flow(answers: Vec<Result<HttpResponse, HttpError>>, guard: &OneAtATime) -> Flow {
    let http = MockHttp::with(answers);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let events = Arc::new(Events::default());
    let opened = Arc::new(Mutex::new(Vec::new()));
    let sink_opened = opened.clone();
    let started = platform_sign_in(
        Arc::new(auth),
        events.clone(),
        Arc::new(move |url: &Url| sink_opened.lock().unwrap().push(url.to_string())),
        guard,
    )
    .unwrap();
    assert_eq!(started.user_code, "ABCD-1234");
    assert_eq!(started.verification_uri, "https://metallic.example/device");
    (events, http, opened)
}

#[test]
fn sign_in_reports_pending_then_signed_in_and_hands_off() {
    let (events, http, opened) = started_flow(
        vec![
            ok(&device_code(
                &format!(r#","verification_uri_complete":"{SITE}/device?user_code=ABCD-1234""#),
                1,
                900,
            )),
            ok(r#"{"access_token":"at-5","refresh_token":"rt-5","expires_in":3600}"#),
            ok(r#"{"code":"wsc_OneTimeCode","expiresAt":"x"}"#),
        ],
        &OneAtATime::new(),
    );
    events.wait_for("signIn signedIn ");
    assert_eq!(events.all()[0], "signIn pending ABCD-1234");
    assert_eq!(
        *opened.lock().unwrap(),
        ["https://metallic.example/device?user_code=ABCD-1234"]
    );
    for _ in 0..100 {
        if !events.handoffs.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        *events.handoffs.lock().unwrap(),
        ["https://metallic.example/auth/launcher-handoff?code=wsc_OneTimeCode"]
    );
    // The event text the window sees about the handoff holds no code.
    assert!(events.all().iter().all(|l| !l.contains("wsc_")));
    assert_eq!(http.requests().len(), 3);
}

#[test]
fn sign_in_reports_denied_expired_and_failed() {
    for (answer, state) in [
        (r#"{"error":"access_denied"}"#, "denied"),
        (r#"{"error":"expired_token"}"#, "expired"),
        (r#"{"error":"server_error"}"#, "failed"),
    ] {
        let (events, _, _) = started_flow(
            vec![ok(&device_code("", 1, 900)), ok(answer)],
            &OneAtATime::new(),
        );
        events.wait_for(&format!("signIn {state} "));
        assert!(events.handoffs.lock().unwrap().is_empty());
    }
}

#[test]
fn a_failed_handoff_code_reports_failed() {
    let (events, _, _) = started_flow(
        vec![
            ok(&device_code("", 1, 900)),
            ok(r#"{"access_token":"at","expires_in":3600}"#),
            reply(500, ""),
        ],
        &OneAtATime::new(),
    );
    events.wait_for("signIn failed ");
}

/// A platform that never approves.
struct Pending;

impl HttpClient for Pending {
    fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        Err(HttpError::new("no"))
    }
    fn post_form(
        &self,
        url: &str,
        _: &[(&str, &str)],
        _: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        if url.ends_with("/device/code") {
            ok(&device_code("", 1, 900))
        } else {
            ok(r#"{"error":"authorization_pending"}"#)
        }
    }
}

#[test]
fn second_sign_in_while_pending_is_busy() {
    let guard = OneAtATime::new();
    let make = || {
        Arc::new(
            PlatformAuth::new(
                site(),
                Arc::new(Pending),
                Arc::new(MemorySecretStore::default()),
            )
            .with_sleeper(|_| std::thread::sleep(Duration::from_millis(5))),
        )
    };
    let events = Arc::new(Events::default());
    let open: Arc<dyn Fn(&Url) + Send + Sync> = Arc::new(|_| {});
    platform_sign_in(make(), events.clone(), open.clone(), &guard).unwrap();
    let second = platform_sign_in(make(), events.clone(), open.clone(), &guard);
    assert!(matches!(second, Err(LauncherError::Busy(_))));
    assert_eq!(
        events
            .all()
            .iter()
            .filter(|l| l.starts_with("signIn pending"))
            .count(),
        1,
        "the refused call announced nothing"
    );
    // Cancelling ends the first, and the place is free again.
    guard.cancel_running();
    events.wait_for("signIn failed ");
    for _ in 0..100 {
        if guard.try_begin().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the guard was never released");
}

// ---- a real server on this PC ----------------------------------------------------------------

type Seen = Vec<(String, String)>;

/// Serves one scripted response per request on 127.0.0.1 and records
/// `(request line, lower-cased request text)`.
fn fake_server(responses: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut data = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                data.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&data).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if data.len() >= end + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&data).to_string();
            let line = text.lines().next().unwrap_or("").to_string();
            seen.push((line, text.to_ascii_lowercase()));
            let answer = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(answer.as_bytes());
        }
        seen
    });
    (origin, handle)
}

#[test]
fn the_whole_flow_over_http_against_a_local_fake_server() {
    let device = r#"{"device_code":"dc-wire","user_code":"WIRE-0001","verification_uri":"https://metallic.example/device","expires_in":900,"interval":1}"#;
    let (origin, server) = fake_server(vec![
        (200, device.to_string()),
        (400, r#"{"error":"authorization_pending"}"#.to_string()),
        (
            200,
            r#"{"access_token":"at-wire","refresh_token":"rt-wire","expires_in":3600}"#.to_string(),
        ),
        (200, r#"{"code":"wsc_wire","expiresAt":"x"}"#.to_string()),
    ]);
    // A local development site may be plain http on the loopback address.
    let site = SiteOrigin::parse(&origin, true).unwrap();
    let store = Arc::new(MemorySecretStore::default());
    let auth = PlatformAuth::new(
        site,
        Arc::new(UreqClient::new(Duration::from_secs(5))),
        store,
    )
    .with_sleeper(|_| {});
    let code = auth.start().unwrap();
    assert_eq!(code.user_code, "WIRE-0001");
    // The fake's verification page is not on the (local) site, so it must not be opened.
    assert!(matches!(
        auth.browser_address(&code),
        Err(AuthError::UnsafeAddress)
    ));
    auth.poll_until_done(&code, &never()).unwrap();
    let handoff = auth.handoff_url(&auth.web_session_code().unwrap()).unwrap();
    assert_eq!(
        handoff.expose().as_str(),
        format!("{origin}/auth/launcher-handoff?code=wsc_wire")
    );

    let seen = server.join().unwrap();
    assert!(seen[0].0.starts_with("POST /v1/oauth/device/code "));
    assert!(seen[0].1.contains("client_id=metallic-launcher"));
    assert!(seen[0].1.contains("scope=profile%20play"));
    assert!(seen[1].0.starts_with("POST /v1/oauth/token "));
    assert!(seen[1].1.contains("device_code=dc-wire"));
    assert!(seen[3].0.starts_with("POST /v1/oauth/web-session "));
    assert!(seen[3].1.contains("authorization: bearer at-wire"));
}

// ---- secrets ---------------------------------------------------------------------------------

/// Captures every log record so the test can search the text.
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

#[test]
fn tokens_never_in_debug_or_logs() {
    const AT: &str = "SECRET-access-token-0a1b";
    const RT: &str = "SECRET-refresh-token-2c3d";
    const DC: &str = "SECRET-device-code-4e5f";
    const WS: &str = "wsc_SECRETwebsessioncode";
    let logs = capture();

    // The whole flow, with failures whose bodies echo secrets back.
    let body = format!(
        r#"{{"device_code":"{DC}","user_code":"ABCD-1234","verification_uri":"{SITE}/device","expires_in":900,"interval":1}}"#
    );
    let http = MockHttp::with(vec![
        ok(&body),
        reply(503, &format!("overloaded {AT} {RT} {DC}")),
        // The token lives one second, so the next call refreshes it first.
        ok(&format!(
            r#"{{"access_token":"{AT}","refresh_token":"{RT}","expires_in":1}}"#
        )),
        reply(
            400,
            &format!(r#"{{"error":"server_error","detail":"{AT} {RT}"}}"#),
        ),
        ok(&format!(
            r#"{{"access_token":"{AT}","refresh_token":"{RT}","expires_in":3600}}"#
        )),
        ok(&format!(r#"{{"code":"{WS}","expiresAt":"x"}}"#)),
    ]);
    let store = Arc::new(MemorySecretStore::default());
    let (auth, _) = auth_with(http.clone(), store.clone());
    let code = auth.start().unwrap();
    auth.poll_until_done(&code, &never()).unwrap();
    let failed_refresh = auth.access_token();
    assert!(failed_refresh.is_err());
    let ws = auth.web_session_code().unwrap();
    let handoff = auth.handoff_url(&ws).unwrap();
    let token = auth.access_token().unwrap().unwrap();
    auth.sign_out().unwrap();

    let events = Events::default();
    events.send(AppEvent::Handoff(handoff.clone()));

    // Everything that can be printed, in every format.
    let shown = [
        format!("{code:?}"),
        format!("{code:#?}"),
        format!("{auth:?}"),
        format!("{auth:#?}"),
        format!("{ws:?} {ws}"),
        format!("{token:?} {token}"),
        format!("{handoff:?}"),
        format!("{:?}", AppEvent::Handoff(handoff.clone())),
        events.all().join("\n"),
        format!("{:?}", failed_refresh.err()),
        format!("{:?}", auth.browser_address(&code)),
    ];
    for text in &shown {
        for secret in [AT, RT, DC, WS, "wsc_"] {
            assert!(!text.contains(secret), "{secret} leaked into: {text}");
        }
    }

    let lines = logs.0.lock().unwrap().clone();
    assert!(
        !lines.is_empty(),
        "the code under test should log something"
    );
    for line in &lines {
        for secret in [AT, RT, DC, WS] {
            assert!(!line.contains(secret), "{secret} leaked into log: {line}");
        }
    }
    // The only place the code exists is the address itself.
    assert!(handoff.expose().as_str().contains(WS));
}
