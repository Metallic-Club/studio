//! Opens the real window. Ignored by default: run it by hand or at night with
//! `cargo test -p metallic_launcher -- --ignored shell_loads_dev_url_and_exits`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PAGE: &str = "<!doctype html><html><head><title>test</title></head><body style=\"background:#222\">\
<script>window.addEventListener('load', function () { fetch('/loaded'); });</script></body></html>";

#[test]
#[ignore = "opens a window"]
fn shell_loads_dev_url_and_exits() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let port = listener.local_addr().expect("local address").port();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    {
        let seen = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 4096];
                let Ok(n) = stream.read(&mut buffer) else {
                    continue;
                };
                let request = String::from_utf8_lossy(&buffer[..n]).to_string();
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                seen.lock().expect("lock").push(path.clone());
                let (content_type, body) = if path == "/" {
                    ("text/html; charset=utf-8", PAGE)
                } else {
                    ("text/plain", "ok")
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
    }

    let config_dir = tempfile::tempdir().expect("temp dir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_Metallic"))
        .args([
            "--dev",
            "--dev-url",
            &format!("http://127.0.0.1:{port}"),
            "--exit-after-load",
        ])
        .env("METALLIC_LAUNCHER_CONFIG_DIR", config_dir.path())
        .spawn()
        .expect("start Metallic");

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            panic!("Metallic did not exit within 20 seconds");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success(), "exit status: {status}");
    assert!(
        seen.lock().expect("lock").iter().any(|p| p == "/loaded"),
        "the page never asked for /loaded: {:?}",
        seen.lock().expect("lock")
    );
}

/// Serves `page_for(path)` (or a plain "ok") on a free local port and records every requested path.
fn serve_pages(
    page_for: impl Fn(&str) -> Option<String> + Send + 'static,
) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let port = listener.local_addr().expect("local address").port();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 4096];
            // The web view may open connections ahead of time and send nothing on them.
            let Ok(n) = stream.read(&mut buffer) else {
                continue;
            };
            if n == 0 {
                continue;
            }
            let request = String::from_utf8_lossy(&buffer[..n]).to_string();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            log.lock().expect("lock").push(path.clone());
            let (content_type, body) = match page_for(&path) {
                Some(page) => ("text/html; charset=utf-8", page),
                None => ("text/plain", "ok".to_string()),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (port, seen)
}

/// Starts Metallic on the development address and waits for it to leave after the first load.
fn run_launcher_on(port: u16) {
    let config_dir = tempfile::tempdir().expect("temp dir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_Metallic"))
        .args([
            "--dev",
            "--dev-url",
            &format!("http://127.0.0.1:{port}"),
            "--exit-after-load",
        ])
        .env("METALLIC_LAUNCHER_CONFIG_DIR", config_dir.path())
        .spawn()
        .expect("start Metallic");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            panic!("Metallic did not exit within 20 seconds");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success(), "exit status: {status}");
}

/// The page on the development address is the site as far as the bridge goes: `getInfo` works.
#[test]
#[ignore = "opens a window"]
fn dev_page_bridge_get_info() {
    let page = "<!doctype html><html><head><title>dev</title></head><body style=\"background:#222\"><script>\
        window.metallic.invoke('getInfo').then(\
          function (info) { fetch('/info?v=' + info.launcherVersion + '&b=' + info.bridgeVersion); },\
          function (e) { fetch('/info-error?code=' + e.code); });\
        </script></body></html>"
        .to_string();
    let (port, seen) = serve_pages(move |path| (path == "/").then(|| page.clone()));
    run_launcher_on(port);
    let seen = seen.lock().expect("lock").clone();
    let expected = format!("/info?v={}&b=1", env!("CARGO_PKG_VERSION"));
    assert!(
        seen.contains(&expected),
        "no getInfo answer; requests: {seen:?}"
    );
}

/// Pages outside the site get no bridge. A second local server (not the development address)
/// cannot be shown: the navigation policy blocks plain `http` to other hosts, in the window and
/// in a frame, so its page never runs (the web view still sends the request before the block, so
/// the test looks for the page's own report, not for the request). A frame of the development
/// page itself loads but has no `window.metallic`.
#[test]
#[ignore = "opens a window"]
fn other_origin_page_has_no_bridge() {
    let (other_port, other_seen) = serve_pages(|_| {
        Some("<script>fetch('/report?metallic=' + typeof window.metallic);</script>".to_string())
    });
    let main_page = format!(
        "<!doctype html><html><head><title>dev</title></head><body style=\"background:#222\">\
         <iframe src=\"/frame\"></iframe><iframe src=\"http://127.0.0.1:{other_port}/\"></iframe>\
         <script>fetch('/top?metallic=' + typeof window.metallic);\
         setTimeout(function () {{ location.href = 'http://127.0.0.1:{other_port}/'; }}, 200);</script>\
         </body></html>"
    );
    let frame_page =
        "<script>fetch('/frame-report?metallic=' + typeof window.metallic);</script>".to_string();
    let (port, seen) = serve_pages(move |path| match path {
        "/" => Some(main_page.clone()),
        "/frame" => Some(frame_page.clone()),
        _ => None,
    });
    run_launcher_on(port);
    let seen = seen.lock().expect("lock").clone();
    assert!(
        seen.contains(&"/top?metallic=object".to_string()),
        "{seen:?}"
    );
    assert!(
        seen.contains(&"/frame-report?metallic=undefined".to_string()),
        "the frame has a bridge or never loaded: {seen:?}"
    );
    let other_seen = other_seen.lock().expect("lock").clone();
    assert!(
        !other_seen.iter().any(|path| path.starts_with("/report")),
        "the other server's page ran: {other_seen:?}"
    );
}
