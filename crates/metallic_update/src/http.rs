//! HTTP plumbing shared by the update check and the GitHub sign-in: the [`HttpClient`] trait (so
//! tests can inject answers), its `ureq` implementation, and the redacting [`Token`] wrapper.

use std::fmt;
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
}

/// [`HttpClient`] backed by `ureq`.
pub struct UreqClient {
    agent: ureq::Agent,
}

impl UreqClient {
    /// Creates a client whose requests give up after `timeout`.
    pub fn new(timeout: Duration) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent }
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
}
