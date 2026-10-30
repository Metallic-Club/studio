//! HTTP plumbing shared by the update check and the GitHub sign-in: the [`HttpClient`] trait (so
//! tests can inject answers), its `ureq` implementation, and the redacting [`Token`] wrapper.

use std::fmt;
use std::io::Read;
use std::time::Duration;

/// A GitHub access token. `Debug` and `Display` print a placeholder, never the value.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Wraps a token value.
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The raw token. Use only to build an `Authorization` header; never log it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted token>")
    }
}

/// Supplies the signed-in user's token when one is needed.
pub trait TokenProvider: Send + Sync {
    /// The current token, or `None` when nobody is signed in.
    fn token(&self) -> Option<Token>;
}

impl TokenProvider for Token {
    fn token(&self) -> Option<Token> {
        Some(self.clone())
    }
}

/// A finished HTTP exchange. Any status code is a response; only transport problems are errors.
///
/// `Debug` omits the body, because the sign-in answers carry tokens.
#[derive(Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body as text.
    pub body: String,
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body", &format_args!("<{} bytes, hidden>", self.body.len()))
            .finish()
    }
}

/// A transport-level HTTP failure (no connection, timeout, bad TLS). Holds a short description
/// that never contains request headers.
#[derive(Clone, Debug)]
pub struct HttpError(String);

impl HttpError {
    /// Creates an error from a description.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HttpError {}

/// The few HTTP calls the updater and sign-in need. Implemented by [`UreqClient`]; tests inject their own.
pub trait HttpClient: Send + Sync {
    /// GET `url` with extra `headers`.
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, HttpError>;

    /// POST `form` (`application/x-www-form-urlencoded`) to `url` with extra `headers`.
    fn post_form(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError>;

    /// GET `url` with extra `headers` and write the body to `out` as it arrives, calling
    /// `progress` with the number of bytes written so far. Returns the total. A status outside
    /// 200 to 299 is an error. Redirects are followed, but an `Authorization` header is sent
    /// only to the host the request started at. If `out` fails, the download stops and the
    /// error is returned.
    ///
    /// The default answers "streaming not supported"; [`UreqClient`] implements it.
    fn get_to_writer(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        out: &mut dyn std::io::Write,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, HttpError> {
        let _ = (url, headers, out, progress);
        Err(HttpError::new("streaming not supported"))
    }
}

/// How many redirects a streamed download follows.
const MAX_DOWNLOAD_REDIRECTS: usize = 5;

/// `scheme://authority` of an address, lower-cased, or `None` for an address without one or
/// with user information in it.
fn origin_of(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), authority.to_ascii_lowercase()))
}

/// True for `https`, and for plain `http` only to this PC (tests).
fn scheme_allowed(scheme: &str, authority: &str) -> bool {
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    scheme == "https" || (scheme == "http" && matches!(host, "127.0.0.1" | "localhost" | "::1"))
}

/// The address a redirect `location` points to, given the address it came from.
fn resolve_redirect(from: &str, location: &str) -> Option<String> {
    if location.contains("://") {
        return Some(location.to_string());
    }
    let (scheme, authority) = origin_of(from)?;
    location
        .starts_with('/')
        .then(|| format!("{scheme}://{authority}{location}"))
}

/// [`HttpClient`] backed by `ureq`.
pub struct UreqClient {
    agent: ureq::Agent,
    /// For streamed downloads: no overall time limit (a big file takes a while), redirects are
    /// handled here so the `Authorization` header can be dropped when the host changes.
    download_agent: ureq::Agent,
}

impl UreqClient {
    /// Creates a client whose requests give up after `timeout`.
    pub fn new(timeout: Duration) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .into();
        let download_agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(None)
            .timeout_connect(Some(timeout))
            .timeout_recv_response(Some(timeout))
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            agent,
            download_agent,
        }
    }
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(20))
    }
}

fn read_response(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<HttpResponse, HttpError> {
    let mut response = result.map_err(|e| HttpError::new(e.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| HttpError::new(e.to_string()))?;
    Ok(HttpResponse { status, body })
}

/// Percent-encodes one form field (RFC 3986 unreserved characters stay as they are).
fn form_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

impl HttpClient for UreqClient {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        read_response(request.call())
    }

    fn post_form(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let body = form
            .iter()
            .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let mut request = self
            .agent
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        read_response(request.send(body))
    }

    fn get_to_writer(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        out: &mut dyn std::io::Write,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, HttpError> {
        let first = origin_of(url).ok_or_else(|| HttpError::new("bad download address"))?;
        let mut current = url.to_string();
        // Once the request has left the first host, the credentials stay dropped.
        let mut send_credentials = true;
        for _ in 0..=MAX_DOWNLOAD_REDIRECTS {
            let here = origin_of(&current).ok_or_else(|| HttpError::new("bad redirect address"))?;
            if !scheme_allowed(&here.0, &here.1) {
                return Err(HttpError::new("refused a download over plain http"));
            }
            if here != first {
                send_credentials = false;
            }
            let mut request = self.download_agent.get(&current);
            for (name, value) in headers {
                if send_credentials || !name.eq_ignore_ascii_case("authorization") {
                    request = request.header(*name, *value);
                }
            }
            let mut response = request.call().map_err(|e| HttpError::new(e.to_string()))?;
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| resolve_redirect(&current, value))
                    .ok_or_else(|| HttpError::new("bad redirect"))?;
                current = location;
                continue;
            }
            if !(200..300).contains(&status) {
                return Err(HttpError::new(format!("HTTP status {status}")));
            }
            let mut reader = response.body_mut().as_reader();
            let mut buffer = vec![0u8; 64 * 1024];
            let mut total: u64 = 0;
            loop {
                let read = reader
                    .read(&mut buffer)
                    .map_err(|e| HttpError::new(e.to_string()))?;
                if read == 0 {
                    return Ok(total);
                }
                out.write_all(&buffer[..read])
                    .map_err(|e| HttpError::new(e.to_string()))?;
                total += read as u64;
                progress(total);
            }
        }
        Err(HttpError::new("too many redirects"))
    }
}
