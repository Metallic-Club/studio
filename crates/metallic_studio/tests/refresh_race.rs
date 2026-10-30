//! Refresh races use independent auth locks and fake transports/stores only.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use metallic_studio::github_auth::{AuthError, GithubAuth, MemorySecretStore, SecretStore};
use metallic_studio::updater::{HttpClient, HttpError, HttpResponse};

const OLD: &str = r#"{"access_token":"old-access","refresh_token":"old-refresh","expires_at":1}"#;
const ROTATED: &str = r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":28800,"refresh_token_expires_in":15897600}"#;
const INVALID: &str = r#"{"error":"bad_refresh_token"}"#;
const NEW: &str = r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_at":18446744073709551615}"#;

struct GatedHttp {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    response: &'static str,
}

impl HttpClient for GatedHttp {
    fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        panic!("refresh must not GET")
    }

    fn post_form(
        &self,
        _: &str,
        _: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        assert!(form.contains(&("grant_type", "refresh_token")));
        assert!(form.contains(&("refresh_token", "old-refresh")));
        self.entered.send(()).unwrap();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| HttpError::new("fixture refresh gate timed out"))?;
        Ok(HttpResponse {
            status: 200,
            body: self.response.into(),
        })
    }
}

fn gated_http(
    entered: mpsc::Sender<()>,
    response: &'static str,
) -> (Arc<GatedHttp>, mpsc::Sender<()>) {
    let (release, receive) = mpsc::channel();
    (
        Arc::new(GatedHttp {
            entered,
            release: Mutex::new(receive),
            response,
        }),
        release,
    )
}

#[test]
fn stale_refresher_returns_the_rotated_session_without_deleting_it() {
    let store = Arc::new(MemorySecretStore::default());
    store.set(OLD).unwrap();
    let (entered, arrivals) = mpsc::channel();
    let (winner_http, release_winner) = gated_http(entered.clone(), ROTATED);
    let (stale_http, release_stale) = gated_http(entered, INVALID);
    // These instances intentionally have different process-local refresh locks.
    let winner =
        GithubAuth::new("fixture-client".into(), winner_http, store.clone()).without_sleeping();
    let stale =
        GithubAuth::new("fixture-client".into(), stale_http, store.clone()).without_sleeping();
    let first = std::thread::spawn(move || winner.valid_token());
    let second = std::thread::spawn(move || stale.valid_token());
    // Bounded channel gates form a barrier: both used the old stored refresh token.
    arrivals.recv_timeout(Duration::from_secs(5)).unwrap();
    arrivals.recv_timeout(Duration::from_secs(5)).unwrap();
    release_winner.send(()).unwrap();
    assert_eq!(
        first.join().unwrap().unwrap().unwrap().expose(),
        "new-access"
    );
    let saved = store.get().unwrap().unwrap();
    release_stale.send(()).unwrap();
    let result = second.join().unwrap();
    assert_eq!(store.get().unwrap().as_deref(), Some(saved.as_str()));
    assert!(
        result.is_ok(),
        "stale refresh must recover the newer session"
    );
    assert_eq!(result.unwrap().unwrap().expose(), "new-access");
}

struct ReplyHttp {
    replacement: Option<(Arc<MemorySecretStore>, &'static str)>,
    network_error: bool,
    calls: AtomicUsize,
}

impl HttpClient for ReplyHttp {
    fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        panic!("refresh must not GET")
    }

    fn post_form(
        &self,
        _: &str,
        _: &[(&str, &str)],
        _: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.network_error {
            return Err(HttpError::new("fixture connection failure"));
        }
        if let Some((store, replacement)) = &self.replacement {
            store.set(replacement).unwrap();
        }
        Ok(HttpResponse {
            status: 200,
            body: INVALID.into(),
        })
    }
}

fn reply(replacement: Option<(Arc<MemorySecretStore>, &'static str)>) -> Arc<ReplyHttp> {
    Arc::new(ReplyHttp {
        replacement,
        network_error: false,
        calls: AtomicUsize::new(0),
    })
}

fn auth(http: Arc<dyn HttpClient>, store: Arc<dyn SecretStore>) -> GithubAuth {
    GithubAuth::new("fixture-client".into(), http, store).without_sleeping()
}

#[test]
fn unchanged_invalid_session_is_removed_by_atomic_memory_store() {
    let store = Arc::new(MemorySecretStore::default());
    store.set(OLD).unwrap();
    let result = auth(reply(None), store.clone()).valid_token();
    assert!(matches!(result, Err(AuthError::Provider(code)) if code == "bad_refresh_token"));
    assert!(store.get().unwrap().is_none());
}

#[test]
fn changed_expired_same_refresh_empty_or_malformed_sessions_are_preserved_not_reused() {
    for replacement in [
        r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_at":1}"#,
        r#"{"access_token":"new-access","refresh_token":"old-refresh"}"#,
        r#"{"access_token":"","refresh_token":"new-refresh"}"#,
        "malformed fixture session",
    ] {
        let store = Arc::new(MemorySecretStore::default());
        store.set(OLD).unwrap();
        let http = reply(Some((store.clone(), replacement)));
        let result = auth(http, store.clone()).valid_token();
        assert!(matches!(result, Err(AuthError::Provider(code)) if code == "bad_refresh_token"));
        assert_eq!(store.get().unwrap().as_deref(), Some(replacement));
    }
}

/// Models a backend without atomic conditional delete, as native keyrings do.
#[derive(Default)]
struct NoConditionalDelete {
    inner: MemorySecretStore,
    deletes: AtomicUsize,
}

impl SecretStore for NoConditionalDelete {
    fn get(&self) -> Result<Option<String>, AuthError> {
        self.inner.get()
    }
    fn set(&self, value: &str) -> Result<(), AuthError> {
        self.inner.set(value)
    }
    fn delete(&self) -> Result<(), AuthError> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.inner.delete()
    }
    // Intentionally inherit the default that declines conditional mutation.
}

#[test]
fn unsupported_backend_keeps_invalid_entry_but_manual_sign_out_still_deletes() {
    let store = Arc::new(NoConditionalDelete::default());
    store.set(OLD).unwrap();
    let auth = auth(reply(None), store.clone());
    assert!(matches!(auth.valid_token(), Err(AuthError::Provider(_))));
    assert_eq!(store.get().unwrap().as_deref(), Some(OLD));
    assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
    auth.sign_out().unwrap();
    assert!(store.get().unwrap().is_none());
    assert_eq!(store.deletes.load(Ordering::SeqCst), 1);
}

struct CompareGate {
    inner: Arc<MemorySecretStore>,
    comparing: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl SecretStore for CompareGate {
    fn get(&self) -> Result<Option<String>, AuthError> {
        self.inner.get()
    }
    fn set(&self, value: &str) -> Result<(), AuthError> {
        self.inner.set(value)
    }
    fn delete(&self) -> Result<(), AuthError> {
        panic!("refresh cleanup must never call unconditional delete")
    }
    fn delete_if_unchanged(&self, expected: &str) -> Result<bool, AuthError> {
        self.comparing.send(()).unwrap();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| AuthError::Store("fixture compare gate timed out".into()))?;
        self.inner.delete_if_unchanged(expected)
    }
}

#[test]
fn rotation_between_recheck_and_conditional_delete_survives_and_is_reused() {
    let memory = Arc::new(MemorySecretStore::default());
    memory.set(OLD).unwrap();
    let (comparing, comparison_started) = mpsc::channel();
    let (release, finish_comparison) = mpsc::channel();
    let store = Arc::new(CompareGate {
        inner: memory.clone(),
        comparing,
        release: Mutex::new(finish_comparison),
    });
    let auth = auth(reply(None), store);
    let failed_refresh = std::thread::spawn(move || auth.valid_token());
    comparison_started
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    memory.set(NEW).unwrap();
    release.send(()).unwrap();
    assert_eq!(
        failed_refresh.join().unwrap().unwrap().unwrap().expose(),
        "new-access"
    );
    assert_eq!(memory.get().unwrap().as_deref(), Some(NEW));
}

#[test]
fn transient_network_failure_retains_session_and_existing_retry_policy() {
    let store = Arc::new(MemorySecretStore::default());
    store.set(OLD).unwrap();
    let http = Arc::new(ReplyHttp {
        replacement: None,
        network_error: true,
        calls: AtomicUsize::new(0),
    });
    let result = auth(http.clone(), store.clone()).valid_token();
    assert!(matches!(result, Err(AuthError::Network(_))));
    assert_eq!(http.calls.load(Ordering::SeqCst), 4);
    assert_eq!(store.get().unwrap().as_deref(), Some(OLD));
}
