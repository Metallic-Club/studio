use super::*;
use crate::updater::{HttpError, HttpResponse, UpdateChecker, UpdateInfo, UreqClient};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

const TEST_TOKEN: &str = "fixture-token";
const PACKAGE: &str = "MetallicStudio-0.2.0-full.nupkg";

fn asset() -> VelopackAsset {
    VelopackAsset {
        PackageId: "MetallicStudio".into(),
        Version: "0.2.0".into(),
        Type: "Full".into(),
        FileName: PACKAGE.into(),
        ..Default::default()
    }
}

fn metadata() -> serde_json::Value {
    serde_json::json!({"tag_name": "v0.2.0-stable", "draft": false, "assets": [
        {"id": 1, "name": "releases.stable.json", "url": "https://untrusted.invalid/feed"},
        {"id": 2, "name": PACKAGE, "browser_download_url": "https://untrusted.invalid/package"}
    ]})
}

fn feed() -> String {
    let mut old = asset();
    old.Version = "0.1.0".into();
    old.FileName = "old-history-not-in-this-release.nupkg".into();
    serde_json::to_string(&VelopackAssetFeed {
        Assets: vec![old, asset()],
    })
    .unwrap()
}

struct FakeHttp(Mutex<VecDeque<HttpResponse>>);
impl HttpClient for FakeHttp {
    fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| HttpError::new(TEST_TOKEN))
    }
    fn post_form(
        &self,
        _: &str,
        _: &[(&str, &str)],
        _: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        panic!("update source must not post")
    }
}

fn source(responses: Vec<HttpResponse>) -> TaggedGithubSource {
    TaggedGithubSource::new(
        Channel::Stable,
        Version::new(0, 2, 0),
        Token::new(TEST_TOKEN.into()),
        "https://api.example.invalid".into(),
        Arc::new(FakeHttp(Mutex::new(responses.into()))),
    )
}
fn ok(body: impl ToString) -> HttpResponse {
    HttpResponse {
        status: 200,
        body: body.to_string(),
    }
}
fn read_feed(source: &TaggedGithubSource) -> Result<VelopackAssetFeed, Error> {
    source.get_release_feed("stable", &velopack::bundle::Manifest::default(), "fixture")
}

#[test]
fn feed_is_pinned_to_selected_version_and_channel() {
    let source = source(vec![ok(metadata()), ok(feed())]);
    assert!(
        source
            .get_release_feed("dev", &Default::default(), "fixture")
            .is_err()
    );
    let result = read_feed(&source).unwrap();
    assert_eq!(result.Assets.len(), 1);
    assert_eq!(result.Assets[0].Version, "0.2.0");
    assert_eq!(result.Assets[0].FileName, PACKAGE);
}

#[test]
fn wrong_tag_draft_and_invalid_metadata_are_rejected() {
    let mut wrong_tag = metadata();
    wrong_tag["tag_name"] = "v0.2.0-dev".into();
    let mut draft = metadata();
    draft["draft"] = true.into();
    for body in [wrong_tag.to_string(), draft.to_string(), "not json".into()] {
        assert!(read_feed(&source(vec![ok(body)])).is_err());
    }
}

#[test]
fn unavailable_and_ambiguous_assets_cannot_resolve() {
    let mut missing = metadata();
    missing["assets"] = serde_json::json!([]);
    let mut zero = metadata();
    zero["assets"][0]["id"] = 0.into();
    let mut duplicate = metadata();
    duplicate["assets"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"id": 3,"name":"releases.stable.json"}));
    for body in [missing, zero, duplicate] {
        assert!(read_feed(&source(vec![ok(body)])).is_err());
    }
    let mut missing_package = metadata();
    missing_package["assets"].as_array_mut().unwrap().pop();
    assert!(read_feed(&source(vec![ok(missing_package), ok(feed())])).is_err());
}

#[test]
fn malformed_or_wrong_version_feed_does_not_offer_an_update() {
    for body in [
        "not json".to_string(),
        "{}".into(),
        "{\"Assets\":[]}".into(),
    ] {
        assert!(read_feed(&source(vec![ok(metadata()), ok(body)])).is_err());
    }
    let mut wrong_version = asset();
    wrong_version.Version = "0.3.0".into();
    let body = serde_json::to_string(&VelopackAssetFeed {
        Assets: vec![wrong_version],
    })
    .unwrap();
    assert!(read_feed(&source(vec![ok(metadata()), ok(body)])).is_err());
}

#[test]
fn delta_entries_never_replace_a_verified_full_update() {
    let mut delta = asset();
    delta.Type = "Delta".into();
    delta.FileName = "pruned-delta.nupkg".into();
    let body = serde_json::to_string(&VelopackAssetFeed {
        Assets: vec![asset(), delta],
    })
    .unwrap();
    let result = read_feed(&source(vec![ok(metadata()), ok(body)])).unwrap();
    assert_eq!(result.Assets.len(), 1);
    assert_eq!(result.Assets[0].Type, "Full");
    // Even an attached target delta cannot establish the chain from a skipped base.
    let mut attached = metadata();
    attached["assets"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"id": 3,"name":"pruned-delta.nupkg"}));
    let mut delta = asset();
    delta.Type = "Delta".into();
    delta.FileName = "pruned-delta.nupkg".into();
    let delta_only = serde_json::to_string(&VelopackAssetFeed {
        Assets: vec![delta.clone()],
    })
    .unwrap();
    assert!(read_feed(&source(vec![ok(attached.clone()), ok(delta_only)])).is_err());
    let with_full = serde_json::to_string(&VelopackAssetFeed {
        Assets: vec![asset(), delta],
    })
    .unwrap();
    let result = read_feed(&source(vec![ok(attached), ok(with_full)])).unwrap();
    assert_eq!(result.Assets.len(), 1);
    assert_eq!(result.Assets[0].Type, "Full");
}

#[test]
fn source_errors_hide_response_and_transport_secrets() {
    for status in [401, 403, 404, 500] {
        let error = read_feed(&source(vec![HttpResponse {
            status,
            body: TEST_TOKEN.into(),
        }]))
        .unwrap_err();
        assert!(!format!("{error:?} {error}").contains(TEST_TOKEN));
    }
    let error = read_feed(&source(vec![])).unwrap_err();
    assert!(!format!("{error:?} {error}").contains(TEST_TOKEN));
}

#[test]
fn foreign_version_package_is_rejected_before_network_or_file_write() {
    let source = source(vec![]);
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("package");
    let mut wrong = asset();
    wrong.Version = "0.3.0".into();
    assert!(
        source
            .download_release_entry(&wrong, &target, None)
            .is_err()
    );
    assert!(!target.exists());
}

/// Loopback-only server whose accept/read/join paths are bounded even when a test fails.
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    fn new(routes: HashMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let server_requests = requests.clone();
        let server_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            while !server_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Accepted Windows sockets can inherit the listener's mode.
                        // Reads below use timeouts, not nonblocking readiness polls.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 16_384 {
                            if stream.read(&mut byte).unwrap_or(0) == 0 {
                                break;
                            }
                            request.push(byte[0]);
                        }
                        let request = String::from_utf8(request).unwrap();
                        let path = request.split_whitespace().nth(1).unwrap_or("");
                        let body = routes.get(path);
                        let status = if body.is_some() {
                            "200 OK"
                        } else {
                            "404 Not Found"
                        };
                        let body = body.map(Vec::as_slice).unwrap_or(b"not found");
                        let header = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        server_requests.lock().unwrap().push(request);
                        let _ = stream
                            .write_all(header.as_bytes())
                            .and_then(|_| stream.write_all(body));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

#[test]
fn stable_download_resolves_beneath_fifteen_newer_dev_releases() {
    let mut releases: Vec<_> = (1..=15)
        .rev()
        .map(|patch| {
            serde_json::json!({
                "tag_name": format!("v0.3.{patch}-dev"), "draft": false, "assets": []
            })
        })
        .collect();
    releases.push(metadata());
    let binary = b"package\0with\xffbinary\x80bytes";
    let server = Server::new(HashMap::from([
        (
            format!("/repos/{RELEASE_REPO}/releases?per_page=50&page=1"),
            serde_json::to_vec(&releases).unwrap(),
        ),
        (
            format!("/repos/{RELEASE_REPO}/releases/tags/v0.2.0-stable"),
            serde_json::to_vec(&metadata()).unwrap(),
        ),
        (
            format!("/repos/{RELEASE_REPO}/releases/assets/1"),
            feed().into_bytes(),
        ),
        (
            format!("/repos/{RELEASE_REPO}/releases/assets/2"),
            binary.to_vec(),
        ),
    ]));
    let checker = UpdateChecker::new(Channel::Stable, Arc::new(Token::new(TEST_TOKEN.into())))
        .with_api_base(server.base.clone())
        .unwrap()
        .with_current_version(Version::new(0, 1, 0))
        .with_http(Arc::new(UreqClient::new(Duration::from_secs(2))));
    let update = checker
        .check()
        .unwrap_or_else(|error| {
            let paths: Vec<String> = server
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| request.lines().next().unwrap_or("").to_string())
                .collect();
            panic!("local discovery failed: {error}; requests: {paths:?}");
        })
        .unwrap();
    assert_eq!(
        update,
        UpdateInfo {
            version: Version::new(0, 2, 0),
            required: false
        }
    );
    let source = checker.update_source(&update).unwrap();
    let feed = read_feed(&source).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("package");
    let (sender, receiver) = std::sync::mpsc::channel();
    source
        .download_release_entry(&feed.Assets[0], &target, Some(sender))
        .unwrap();
    assert_eq!(std::fs::read(target).unwrap(), binary);
    assert_eq!(receiver.try_iter().last(), Some(100));
    let requests = server.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        4,
        "metadata is reused for the package transfer"
    );
    assert!(
        !requests
            .iter()
            .any(|request| request.contains("per_page=10"))
    );
    assert!(requests.iter().all(|request| {
        request
            .to_lowercase()
            .contains(&format!("authorization: bearer {TEST_TOKEN}"))
    }));
    assert!(requests[2..].iter().all(|request| {
        request
            .to_lowercase()
            .contains("accept: application/octet-stream")
    }));
}
