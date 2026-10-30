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
