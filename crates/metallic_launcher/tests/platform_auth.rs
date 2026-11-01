//! Tests for the Metallic sign-in (device flow) and the web-session handoff. HTTP is a scripted
//! fake or a server on 127.0.0.1; nothing touches the network or the Credential Manager.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use metallic_launcher::platform_auth::{CLIENT_ID, DeviceCode, SCOPES};
use metallic_launcher::{
    AppEvent, BridgeEvent, EventSink, OneAtATime, PlatformAuth, SiteOrigin, platform_sign_in,
    sign_out_in_background,
};
use metallic_update::{
    AuthError, HttpClient, HttpError, HttpResponse, MAX_SECRET_BYTES, MemorySecretStore,
    SecretStore, UreqClient,
};
use url::Url;

const SITE: &str = "https://metallic.example";
const PENDING_LINE: &str = "signIn pending ABCD-1234 https://metallic.example/device";

/// A cancel flag that is never set.
fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// One recorded request: url, headers, form body.
type Recorded = (String, Vec<(String, String)>, String);

fn record(log: &Mutex<Vec<Recorded>>, url: &str, headers: &[(&str, &str)], form: &[(&str, &str)]) {
    let headers = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let body = form
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    log.lock().unwrap().push((url.to_string(), headers, body));
}

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
    fn count(&self, path: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.0.ends_with(path))
            .count()
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
        record(&self.requests, url, headers, form);
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

/// Puts a refresh token into the store the way the Launcher keeps it.
fn stored_refresh(store: &MemorySecretStore, token: &str) {
    store
        .set_secret(format!(r#"{{"version":1,"refresh_token":"{token}"}}"#).as_bytes())
        .unwrap();
}

fn tokens(access: &str, refresh: &str, expires_in: u64) -> String {
    format!(
        r#"{{"access_token":"{access}","refresh_token":"{refresh}","expires_in":{expires_in}}}"#
    )
}

/// What the store holds, as text.
fn raw(store: &MemorySecretStore) -> Option<String> {
    store
        .get_secret()
        .unwrap()
        .map(|b| String::from_utf8(b).unwrap())
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
        ok(&tokens("at-1", "rt-1", 3600)),
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
        "the access token is kept in memory"
    );
    // Only the refresh token is stored; the access token stays in memory.
    let stored = raw(&store).unwrap();
    assert!(stored.contains("rt-1") && !stored.contains("at-1"));
}

#[test]
fn a_missing_interval_means_five_seconds() {
    let body = r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://metallic.example/device","expires_in":900}"#;
    let http = MockHttp::with(vec![ok(r#"{"access_token":"at","expires_in":3600}"#)]);
    let (auth, waits) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, body.to_string());
    assert_eq!(code.interval, 5);
    auth.poll_until_done(&code, &never()).unwrap();
    assert_eq!(*waits.lock().unwrap(), [5]);
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
fn an_error_code_with_odd_characters_is_not_passed_on() {
    let http = MockHttp::with(vec![ok("{\"error\":\"bad\\nINJECTED line\"}")]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, device_code("", 1, 900));
    let error = auth.poll_until_done(&code, &never()).unwrap_err();
    assert_eq!(format!("{error}"), "sign-in failed: unknown");
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
    assert!(store.get_secret().unwrap().is_none());
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
fn a_token_over_256_bytes_is_refused() {
    let long = "m".repeat(257);
    let store = Arc::new(MemorySecretStore::default());
    let http = MockHttp::with(vec![ok(&tokens(&long, "rt", 3600))]);
    let (auth, _) = auth_with(http.clone(), store.clone());
    let code = code_for(&auth, &http, device_code("", 1, 900));
    assert!(matches!(
        auth.poll_until_done(&code, &never()),
        Err(AuthError::TokenTooLong)
    ));
    assert!(store.get_secret().unwrap().is_none());
    // The same for a refresh token, and for the answer to a refresh.
    let http = MockHttp::with(vec![ok(&tokens("at", &long, 3600))]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = code_for(&auth, &http, device_code("", 1, 900));
    assert!(matches!(
        auth.poll_until_done(&code, &never()),
        Err(AuthError::TokenTooLong)
    ));
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "rt");
    let (auth, _) = auth_with(MockHttp::with(vec![ok(&tokens(&long, "rt2", 3600))]), store);
    assert!(matches!(auth.access_token(), Err(AuthError::TokenTooLong)));
}

/// A credential store that is always too small.
struct FullStore;

impl SecretStore for FullStore {
    fn get(&self) -> Result<Option<String>, AuthError> {
        Ok(None)
    }
    fn set(&self, _: &str) -> Result<(), AuthError> {
        Err(AuthError::TooLarge)
    }
    fn delete(&self) -> Result<(), AuthError> {
        Ok(())
    }
    fn set_secret(&self, _: &[u8]) -> Result<(), AuthError> {
        Err(AuthError::TooLarge)
    }
}

#[test]
fn a_session_that_does_not_fit_is_a_typed_error_and_the_new_token_is_revoked() {
    let http = MockHttp::with(vec![ok(&tokens("at", "rt-big", 3600))]);
    let waits = Arc::new(Mutex::new(Vec::new()));
    let recorded = waits.clone();
    let auth = PlatformAuth::new(site(), http.clone(), Arc::new(FullStore))
        .with_sleeper(move |d: Duration| recorded.lock().unwrap().push(d.as_secs()));
    let code = code_for(&auth, &http, device_code("", 1, 900));
    assert!(matches!(
        auth.poll_until_done(&code, &never()),
        Err(AuthError::TooLarge)
    ));
    assert!(
        http.requests()
            .iter()
            .any(|r| r.0.ends_with("/revoke") && r.2.contains("token=rt-big"))
    );
}

#[test]
fn the_memory_store_has_the_credential_manager_limit() {
    let store = MemorySecretStore::default();
    assert_eq!(MAX_SECRET_BYTES, 2560);
    store.set_secret(&vec![b'x'; MAX_SECRET_BYTES]).unwrap();
    assert!(matches!(
        store.set_secret(&vec![b'x'; MAX_SECRET_BYTES + 1]),
        Err(AuthError::TooLarge)
    ));
    // Text is stored as UTF-16 on Windows: two bytes a character.
    store.set(&"x".repeat(MAX_SECRET_BYTES / 2)).unwrap();
    assert!(matches!(
        store.set(&"x".repeat(MAX_SECRET_BYTES / 2 + 1)),
        Err(AuthError::TooLarge)
    ));
}

#[test]
fn refresh_rotates_and_stores_new_token() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "old-rt");
    let http = MockHttp::with(vec![ok(&tokens("new-at", "new-rt", 3600))]);
    let (auth, _) = auth_with(http.clone(), store.clone());
    // After a restart only the refresh token is there, so the first use refreshes.
    assert_eq!(auth.access_token().unwrap().unwrap().expose(), "new-at");

    let requests = http.requests();
    assert!(requests[0].2.contains("grant_type=refresh_token"));
    assert!(requests[0].2.contains("refresh_token=old-rt"));
    let stored = raw(&store).unwrap();
    assert!(stored.contains("new-rt") && !stored.contains("old-rt") && !stored.contains("new-at"));
    // A fresh token is not refreshed again.
    assert_eq!(auth.access_token().unwrap().unwrap().expose(), "new-at");
    assert_eq!(http.requests().len(), 1);
}

#[test]
fn a_refresh_that_does_not_rotate_keeps_the_old_refresh_token() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "keep-rt");
    let http = MockHttp::with(vec![ok(r#"{"access_token":"at","expires_in":3600}"#)]);
    let (auth, _) = auth_with(http, store.clone());
    assert!(auth.access_token().unwrap().is_some());
    assert!(raw(&store).unwrap().contains("keep-rt"));
}

#[test]
fn a_token_with_time_left_is_used_without_a_request() {
    let http = MockHttp::with(vec![
        ok(&device_code("", 1, 900)),
        ok(&tokens("at-1", "rt-1", 3600)),
    ]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let code = auth.start().unwrap();
    auth.poll_until_done(&code, &never()).unwrap();
    assert_eq!(auth.access_token().unwrap().unwrap().expose(), "at-1");
    assert_eq!(http.requests().len(), 2, "no refresh request");
}

#[test]
fn invalid_grant_clears_session() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "old-rt");
    // As HTTP 400, the usual shape for an OAuth error.
    let http = MockHttp::with(vec![reply(400, r#"{"error":"invalid_grant"}"#)]);
    let (auth, _) = auth_with(http, store.clone());
    assert!(auth.access_token().unwrap().is_none());
    assert!(store.get_secret().unwrap().is_none());
}

#[test]
fn a_lost_refresh_answer_is_retried_once_with_the_same_token() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "rt-a");
    let http = MockHttp::with(vec![
        Err(HttpError::new("reset")),
        ok(&tokens("at", "rt-b", 3600)),
    ]);
    let (auth, _) = auth_with(http.clone(), store.clone());
    assert!(auth.access_token().unwrap().is_some());
    let requests = http.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.2.contains("refresh_token=rt-a")));
    assert!(raw(&store).unwrap().contains("rt-b"));

    // Two lost answers in a row: an error, and no third try.
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "rt-a");
    let http = MockHttp::with(vec![Err(HttpError::new("x")), Err(HttpError::new("y"))]);
    let (auth, _) = auth_with(http.clone(), store.clone());
    assert!(auth.access_token().is_err());
    assert_eq!(http.requests().len(), 2);
    assert!(raw(&store).unwrap().contains("rt-a"), "still stored");

    // The retry finds the token spent (the first request had got through): signed out.
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "rt-a");
    let http = MockHttp::with(vec![
        Err(HttpError::new("x")),
        reply(400, r#"{"error":"invalid_grant"}"#),
    ]);
    let (auth, _) = auth_with(http, store.clone());
    assert!(auth.access_token().unwrap().is_none());
    assert!(store.get_secret().unwrap().is_none());
}

#[test]
fn an_answered_refresh_is_never_repeated_with_the_old_token() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "rt-a");
    let http = MockHttp::with(vec![reply(400, r#"{"error":"server_error"}"#)]);
    let (auth, _) = auth_with(http.clone(), store);
    assert!(matches!(auth.access_token(), Err(AuthError::Provider(_))));
    assert_eq!(http.requests().len(), 1);
}

/// A platform that is slow to refresh, and counts the refresh requests.
struct SlowRefresh(AtomicUsize);

impl HttpClient for SlowRefresh {
    fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        Err(HttpError::new("no"))
    }
    fn post_form(
        &self,
        _: &str,
        _: &[(&str, &str)],
        _: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200));
        ok(&tokens("at-shared", "rt-new", 3600))
    }
}

#[test]
fn concurrent_refreshes_make_one_request() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "rt-old");
    let http = Arc::new(SlowRefresh(AtomicUsize::new(0)));
    let auth = Arc::new(PlatformAuth::new(site(), http.clone(), store.clone()));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let auth = auth.clone();
            std::thread::spawn(move || auth.access_token().unwrap().unwrap().expose().to_string())
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), "at-shared");
    }
    assert_eq!(http.0.load(Ordering::SeqCst), 1, "one refresh, not two");
    assert!(raw(&store).unwrap().contains("rt-new"));
}

#[test]
fn a_new_device_flow_revokes_the_session_it_replaces() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "old-rt");
    let http = MockHttp::with(vec![ok(&tokens("at", "new-rt", 3600)), ok("{}")]);
    let (auth, _) = auth_with(http.clone(), store.clone());
    let code = code_for(&auth, &http, device_code("", 1, 900));
    auth.poll_until_done(&code, &never()).unwrap();
    let revoke: Vec<_> = http
        .requests()
        .into_iter()
        .filter(|r| r.0.ends_with("/revoke"))
        .collect();
    assert_eq!(revoke.len(), 1);
    assert!(revoke[0].2.contains("token=old-rt"));
    assert!(raw(&store).unwrap().contains("new-rt"));
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
    stored_refresh(&store, "rt");
    let http = MockHttp::with(vec![
        ok(&tokens("at-9", "rt-2", 3600)),
        ok(r#"{"code":"wsc_AbC123-x","expiresAt":"2026-10-30T10:00:00Z"}"#),
    ]);
    let (auth, _) = auth_with(http.clone(), store);
    let code = auth.web_session_code().unwrap();
    let requests = http.requests();
    assert_eq!(
        requests[1].0,
        "https://metallic.example/v1/oauth/web-session"
    );
    assert!(
        requests[1]
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
fn the_web_session_request_is_made_three_times_at_most() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "rt");
    let http = MockHttp::with(vec![
        ok(&tokens("at", "rt-2", 3600)),
        reply(503, ""),
        Err(HttpError::new("reset")),
        ok(r#"{"code":"wsc_ok","expiresAt":"x"}"#),
    ]);
    let (auth, waits) = auth_with(http.clone(), store.clone());
    assert!(auth.web_session_code().is_ok());
    assert_eq!(http.count("/web-session"), 3);
    assert_eq!(*waits.lock().unwrap(), [1, 2]);

    let http = MockHttp::with(vec![
        ok(&tokens("at", "rt-2", 3600)),
        reply(503, ""),
        reply(503, ""),
        reply(503, ""),
        ok(r#"{"code":"wsc_late","expiresAt":"x"}"#),
    ]);
    let (auth, _) = auth_with(http.clone(), store);
    assert!(matches!(
        auth.web_session_code(),
        Err(AuthError::Network(_))
    ));
    assert_eq!(http.count("/web-session"), 3);
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
        stored_refresh(&store, "rt");
        let http = MockHttp::with(vec![ok(&tokens("at", "rt-2", 3600)), ok(bad)]);
        let (auth, _) = auth_with(http, store);
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
    // Through the bridge entry point: nothing is opened, nothing is polled, `failed` comes back.
    let http = MockHttp::with(vec![ok(&device_code(
        r#","verification_uri_complete":"https://evil.test/device""#,
        1,
        900,
    ))]);
    let (auth, _) = auth_with(http.clone(), Arc::new(MemorySecretStore::default()));
    let opened = Arc::new(Mutex::new(0));
    let counter = opened.clone();
    let events = Arc::new(Events::default());
    platform_sign_in(
        Arc::new(auth),
        events.clone(),
        Arc::new(move |_| *counter.lock().unwrap() += 1),
        &OneAtATime::new(),
    )
    .unwrap();
    events.wait_for("signIn failed ");
    assert_eq!(*opened.lock().unwrap(), 0);
    assert!(
        events
            .all()
            .iter()
            .all(|l| !l.starts_with("signIn pending"))
    );
    assert_eq!(http.requests().len(), 1, "no poll after a refused address");
}

#[test]
fn sign_out_revokes_then_deletes_even_when_revoke_fails() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "rt-77");
    let http = MockHttp::with(vec![ok("{}")]);
    let (auth, _) = auth_with(http.clone(), store.clone());
    auth.sign_out().unwrap();
    let requests = http.requests();
    assert_eq!(requests[0].0, "https://metallic.example/v1/oauth/revoke");
    assert!(requests[0].2.contains("token=rt-77"));
    assert!(store.get_secret().unwrap().is_none());

    stored_refresh(&store, "rt-77");
    let (auth, _) = auth_with(
        MockHttp::with(vec![Err(HttpError::new("down"))]),
        store.clone(),
    );
    auth.sign_out().unwrap();
    assert!(store.get_secret().unwrap().is_none());
}

#[test]
fn background_sign_out_forgets_at_once_and_revokes_on_a_worker() {
    let store = Arc::new(MemorySecretStore::default());
    stored_refresh(&store, "rt-bg");
    let http = MockHttp::with(vec![ok("{}")]);
    let (auth, _) = auth_with(http.clone(), store.clone());
    sign_out_in_background(Arc::new(auth), &OneAtATime::new()).unwrap();
    assert!(
        store.get_secret().unwrap().is_none(),
        "gone before the call returned"
    );
    for _ in 0..100 {
        if http.count("/revoke") == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the revoke never ran");
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
    fn handoffs(&self) -> Vec<String> {
        self.handoffs.lock().unwrap().clone()
    }
}

impl EventSink for Events {
    fn send(&self, event: AppEvent) {
        let line = match event {
            AppEvent::Bridge(BridgeEvent::SignIn {
                state,
                user_code,
                verification_uri,
            }) => {
                let mut line = format!("signIn {state} {}", user_code.unwrap_or_default());
                if let Some(uri) = verification_uri {
                    line.push(' ');
                    line.push_str(&uri);
                }
                line
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

fn started_flow(
    answers: Vec<Result<HttpResponse, HttpError>>,
    guard: &OneAtATime,
    store: Arc<MemorySecretStore>,
) -> Flow {
    let http = MockHttp::with(answers);
    let (auth, _) = auth_with(http.clone(), store);
    let events = Arc::new(Events::default());
    let opened = Arc::new(Mutex::new(Vec::new()));
    let sink_opened = opened.clone();
    let reply = platform_sign_in(
        Arc::new(auth),
        events.clone(),
        Arc::new(move |url: &Url| sink_opened.lock().unwrap().push(url.to_string())),
        guard,
    )
    .unwrap();
    assert!(
        reply.user_code.is_none(),
        "the first call answers `pending` and nothing else"
    );
    (events, http, opened)
}

fn fresh_store() -> Arc<MemorySecretStore> {
    Arc::new(MemorySecretStore::default())
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
            ok(&tokens("at-5", "rt-5", 3600)),
            ok(r#"{"code":"wsc_OneTimeCode","expiresAt":"x"}"#),
        ],
        &OneAtATime::new(),
        fresh_store(),
    );
    events.wait_for("signIn signedIn ");
    // The code and the page arrive as an event, not in the reply.
    assert_eq!(events.all()[0], PENDING_LINE);
    assert_eq!(
        *opened.lock().unwrap(),
        ["https://metallic.example/device?user_code=ABCD-1234"]
    );
    for _ in 0..100 {
        if !events.handoffs().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        events.handoffs(),
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
            fresh_store(),
        );
        events.wait_for(&format!("signIn {state} "));
        assert!(events.handoffs().is_empty());
    }
}

#[test]
fn a_signed_in_launcher_skips_the_browser_and_only_signs_the_web_view_in() {
    let store = fresh_store();
    stored_refresh(&store, "rt-old");
    let (events, http, opened) = started_flow(
        vec![
            ok(&tokens("at", "rt-new", 3600)),
            ok(r#"{"code":"wsc_again","expiresAt":"x"}"#),
        ],
        &OneAtATime::new(),
        store,
    );
    events.wait_for("signIn signedIn ");
    assert_eq!(http.count("/device/code"), 0);
    assert!(opened.lock().unwrap().is_empty());
    assert!(
        events
            .all()
            .iter()
            .all(|l| !l.starts_with("signIn pending"))
    );
    assert_eq!(
        events.handoffs(),
        ["https://metallic.example/auth/launcher-handoff?code=wsc_again"]
    );
}

#[test]
fn a_failed_handoff_reports_failed_and_leaves_the_session_for_a_retry() {
    let store = fresh_store();
    stored_refresh(&store, "rt-old");
    let (events, http, _) = started_flow(
        vec![
            ok(&tokens("at", "rt-new", 3600)),
            reply(500, ""),
            reply(500, ""),
            reply(500, ""),
        ],
        &OneAtATime::new(),
        store.clone(),
    );
    events.wait_for("signIn failed ");
    assert!(events.handoffs().is_empty(), "no Handoff after a failure");
    assert_eq!(http.count("/web-session"), 3);
    assert!(
        raw(&store).unwrap().contains("rt-new"),
        "still signed in: the next try needs no browser"
    );
}

#[test]
fn a_401_from_web_session_forces_a_refresh_and_the_handoff_is_sent() {
    let store = fresh_store();
    stored_refresh(&store, "rt-1");
    let (events, http, opened) = started_flow(
        vec![
            ok(&tokens("at-1", "rt-2", 3600)),
            reply(401, r#"{"error":"invalid_token"}"#),
            ok(&tokens("at-2", "rt-3", 3600)),
            ok(r#"{"code":"wsc_second","expiresAt":"x"}"#),
        ],
        &OneAtATime::new(),
        store.clone(),
    );
    events.wait_for("signIn signedIn ");
    assert_eq!(
        events.handoffs(),
        ["https://metallic.example/auth/launcher-handoff?code=wsc_second"]
    );
    assert!(opened.lock().unwrap().is_empty(), "no browser");
    let requests = http.requests();
    assert_eq!(requests.len(), 4);
    // The second refresh used the rotated token, and the retry carried the new access token.
    assert!(requests[2].2.contains("refresh_token=rt-2"));
    assert!(
        requests[3]
            .1
            .contains(&("Authorization".to_string(), "Bearer at-2".to_string()))
    );
    assert!(raw(&store).unwrap().contains("rt-3"));
}

#[test]
fn a_session_the_platform_keeps_refusing_is_deleted() {
    // 401 twice, even after a refresh: the session is bad, so the next sign-in must use the browser.
    let store = fresh_store();
    stored_refresh(&store, "rt-1");
    let (events, _, _) = started_flow(
        vec![
            ok(&tokens("at-1", "rt-2", 3600)),
            reply(401, "{}"),
            ok(&tokens("at-2", "rt-3", 3600)),
            reply(403, "{}"),
        ],
        &OneAtATime::new(),
        store.clone(),
    );
    events.wait_for("signIn failed ");
    assert!(events.handoffs().is_empty());
    assert!(store.get_secret().unwrap().is_none());

    // 401, then the refresh itself is refused (`invalid_grant`): also signed out.
    let store = fresh_store();
    stored_refresh(&store, "rt-1");
    let http = MockHttp::with(vec![
        ok(&tokens("at-1", "rt-2", 3600)),
        reply(401, "{}"),
        reply(400, r#"{"error":"invalid_grant"}"#),
    ]);
    let (auth, _) = auth_with(http, store.clone());
    assert!(matches!(
        auth.web_session_code(),
        Err(AuthError::NotSignedIn)
    ));
    assert!(store.get_secret().unwrap().is_none());
}

#[test]
fn errors_after_the_server_accepted_a_refresh_delete_the_spent_token() {
    let long = "m".repeat(257);
    let cases: Vec<(String, bool)> = vec![
        (tokens(&long, "rt-new", 3600), true),
        (tokens("at", &long, 3600), true),
        ("not json".to_string(), true),
        (r#"{"token_type":"bearer"}"#.to_string(), true),
    ];
    for (body, _) in cases {
        let store = fresh_store();
        stored_refresh(&store, "rt-spent");
        let (auth, _) = auth_with(MockHttp::with(vec![ok(&body)]), store.clone());
        assert!(auth.access_token().is_err(), "{body}");
        assert!(
            store.get_secret().unwrap().is_none(),
            "the spent token must not stay: {body}"
        );
    }
    // An error that is not an acceptance (HTTP 400, server_error) leaves the session alone.
    let store = fresh_store();
    stored_refresh(&store, "rt-kept");
    let http = MockHttp::with(vec![reply(400, r#"{"error":"server_error"}"#)]);
    let (auth, _) = auth_with(http, store.clone());
    assert!(auth.access_token().is_err());
    assert!(raw(&store).unwrap().contains("rt-kept"));
}

/// A platform that never approves, and counts the device-code requests.
struct Pending(AtomicUsize);

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
            self.0.fetch_add(1, Ordering::SeqCst);
            ok(&device_code("", 1, 900))
        } else {
            ok(r#"{"error":"authorization_pending"}"#)
        }
    }
}

#[test]
fn second_sign_in_while_pending_returns_the_running_code() {
    let guard = OneAtATime::new();
    let http = Arc::new(Pending(AtomicUsize::new(0)));
    let auth = Arc::new(
        PlatformAuth::new(site(), http.clone(), fresh_store())
            .with_sleeper(|_| std::thread::sleep(Duration::from_millis(5))),
    );
    let events = Arc::new(Events::default());
    let open: Arc<dyn Fn(&Url) + Send + Sync> = Arc::new(|_| {});
    let first = platform_sign_in(auth.clone(), events.clone(), open.clone(), &guard).unwrap();
    assert!(first.user_code.is_none());
    events.wait_for(PENDING_LINE);

    let second = platform_sign_in(auth.clone(), events.clone(), open.clone(), &guard).unwrap();
    assert_eq!(second.user_code.as_deref(), Some("ABCD-1234"));
    assert_eq!(
        second.verification_uri.as_deref(),
        Some("https://metallic.example/device")
    );
    let pending = events.all().iter().filter(|l| *l == PENDING_LINE).count();
    assert_eq!(pending, 2, "the event is sent again");
    assert_eq!(http.0.load(Ordering::SeqCst), 1, "no second device code");

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

/// A platform whose token request waits until the test lets it answer.
struct Gate {
    requests: Mutex<Vec<Recorded>>,
    entered: Mutex<Sender<()>>,
    release: Mutex<Receiver<()>>,
}

impl HttpClient for Gate {
    fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        Err(HttpError::new("no"))
    }
    fn post_form(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        record(&self.requests, url, headers, form);
        if url.ends_with("/device/code") {
            ok(&device_code("", 1, 900))
        } else if url.ends_with("/oauth/token") {
            self.entered.lock().unwrap().send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            ok(&tokens("at-late", "rt-late", 3600))
        } else if url.ends_with("/web-session") {
            ok(r#"{"code":"wsc_never","expiresAt":"x"}"#)
        } else {
            ok("{}")
        }
    }
}

#[test]
fn signing_out_during_the_poll_stores_nothing_and_sends_no_handoff() {
    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx) = channel();
    let gate = Arc::new(Gate {
        requests: Mutex::default(),
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    });
    let store = fresh_store();
    let auth =
        Arc::new(PlatformAuth::new(site(), gate.clone(), store.clone()).with_sleeper(|_| {}));
    let guard = OneAtATime::new();
    let events = Arc::new(Events::default());
    platform_sign_in(auth.clone(), events.clone(), Arc::new(|_: &Url| {}), &guard).unwrap();
    events.wait_for(PENDING_LINE);
    // The worker is inside the token request. The user approves in the browser, and at the same
    // moment signs out in the Launcher.
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    sign_out_in_background(auth.clone(), &guard).unwrap();
    release_tx.send(()).unwrap();

    events.wait_for("signIn failed ");
    assert!(store.get_secret().unwrap().is_none(), "nothing stored");
    assert!(events.handoffs().is_empty(), "no Handoff");
    let requests = gate.requests.lock().unwrap().clone();
    assert!(
        requests
            .iter()
            .any(|r| r.0.ends_with("/revoke") && r.2.contains("token=rt-late")),
        "the tokens that arrived late are revoked"
    );
    assert!(requests.iter().all(|r| !r.0.ends_with("/web-session")));
    assert!(
        auth.access_token().unwrap().is_none(),
        "and not kept in memory"
    );
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
        (200, tokens("at-wire", "rt-wire", 3600)),
        (200, r#"{"code":"wsc_wire","expiresAt":"x"}"#.to_string()),
    ]);
    // A local development site may be plain http on the loopback address.
    let site = SiteOrigin::parse(&origin, true).unwrap();
    let store = fresh_store();
    let auth = PlatformAuth::new(
        site,
        Arc::new(UreqClient::without_redirects(Duration::from_secs(5))),
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
        ok(&tokens(AT, RT, 1)),
        reply(
            400,
            &format!(r#"{{"error":"server_error","detail":"{AT} {RT}"}}"#),
        ),
        ok(&tokens(AT, RT, 3600)),
        ok(&format!(r#"{{"code":"{WS}","expiresAt":"x"}}"#)),
    ]);
    let store = fresh_store();
    let (auth, _) = auth_with(http.clone(), store.clone());
    let code = auth.start().unwrap();
    auth.poll_until_done(&code, &never()).unwrap();
    let failed_refresh = auth.access_token();
    assert!(failed_refresh.is_err());
    let ws = auth.web_session_code().unwrap();
    let handoff = auth.handoff_url(&ws).unwrap();
    let token = auth.access_token().unwrap().unwrap();
    let revocation = auth.sign_out_local().unwrap();
    let revocation_text = format!("{revocation:?}");
    auth.revoke(revocation);

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
        revocation_text,
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

/// A store whose delete always fails.
struct BrokenStore;

impl SecretStore for BrokenStore {
    fn get(&self) -> Result<Option<String>, AuthError> {
        Ok(None)
    }
    fn set(&self, _: &str) -> Result<(), AuthError> {
        Ok(())
    }
    fn delete(&self) -> Result<(), AuthError> {
        Err(AuthError::Store("locked".to_string()))
    }
}

/// #289: the uninstall deletes the saved sign-ins (in memory here, never the real store); one
/// failing store does not keep the others.
#[test]
fn uninstall_forgets_saved_sign_ins() {
    let platform = MemorySecretStore::default();
    let github = MemorySecretStore::default();
    platform.set_secret(b"refresh").unwrap();
    github.set("{}").unwrap();
    let empty = MemorySecretStore::default();
    assert_eq!(
        metallic_launcher::forget_saved_sign_ins(&[&platform, &BrokenStore, &github, &empty]),
        1
    );
    assert!(platform.get().unwrap().is_none());
    assert!(github.get().unwrap().is_none());
}
