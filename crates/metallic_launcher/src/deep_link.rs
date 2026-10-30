//! Bounded `metallic://` parsing and canonical serialization.

use std::fmt;

/// Registered protocol scheme.
pub const SCHEME: &str = "metallic";
/// Maximum protocol input size in bytes, checked before parsing.
pub const MAX_URI_LEN: usize = 2048;

/// A link could not be parsed. No variant echoes input or a join ticket.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeepLinkError {
    /// Input exceeded the protocol limit.
    #[error("link is too long")]
    TooLong,
    /// The input did not use the registered scheme.
    #[error("not a Metallic link")]
    NotMetallic,
    /// No action matched the protocol grammar.
    #[error("unknown action")]
    UnknownAction,
    /// A required parameter was absent.
    #[error("missing {0}")]
    MissingParam(&'static str),
    /// A field failed validation.
    #[error("invalid {0}")]
    BadParam(&'static str),
    /// A known field appeared more than once.
    #[error("duplicate {0}")]
    DuplicateParam(&'static str),
}

/// A positive, canonical decimal place identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlaceId(u64);

impl PlaceId {
    /// Accepts 1-20 ASCII decimal digits, without a sign or leading zero, fitting `u64`.
    pub fn parse(text: &str) -> Result<Self, DeepLinkError> {
        if text.is_empty()
            || text.len() > 20
            || text.starts_with('0')
            || !text.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(DeepLinkError::BadParam("place"));
        }
        text.parse()
            .map(Self)
            .map_err(|_| DeepLinkError::BadParam("place"))
    }
}

impl fmt::Display for PlaceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// An opaque, short-lived join ticket. Debug output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct JoinTicket(String);

impl JoinTicket {
    /// Accepts 8-128 ASCII letters, digits, underscores or hyphens.
    pub fn parse(text: &str) -> Result<Self, DeepLinkError> {
        if !(8..=128).contains(&text.len())
            || !text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(DeepLinkError::BadParam("ticket"));
        }
        Ok(Self(text.to_owned()))
    }

    /// Exposes the ticket only for protocol serialization/transport. Never log this value.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for JoinTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JoinTicket(..)")
    }
}

/// A website-relative path, optionally including query and fragment text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SitePath(String);

impl SitePath {
    /// Accepts at most 512 bytes, starting with one slash, without backslashes or controls.
    pub fn parse(text: &str) -> Result<Self, DeepLinkError> {
        if text.len() > 512
            || !text.starts_with('/')
            || text.starts_with("//")
            || text.contains('\\')
            || text.chars().any(char::is_control)
        {
            return Err(DeepLinkError::BadParam("path"));
        }
        Ok(Self(text.to_owned()))
    }

    /// Returns the validated path text without further percent decoding.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated protocol action. Its ticket field remains redacted in Debug output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    /// Start the player for this place and opaque join ticket.
    Play {
        /// Positive place identifier.
        place: PlaceId,
        /// Single-use ticket supplied by the platform.
        ticket: JoinTicket,
    },
    /// Open Studio, optionally for a particular place.
    StudioOpen {
        /// Place to open when supplied.
        place: Option<PlaceId>,
    },
    /// Navigate to a path on the configured site.
    Open {
        /// Relative site path, or the site's root when absent.
        path: Option<SitePath>,
    },
    /// Request Studio installation.
    Install,
}

impl DeepLink {
    /// Parses a bounded URI, decoding query fields exactly once and dropping unknown fields.
    pub fn parse(uri: &str) -> Result<Self, DeepLinkError> {
        if uri.len() > MAX_URI_LEN {
            return Err(DeepLinkError::TooLong);
        }
        if !is_metallic(uri) {
            return Err(DeepLinkError::NotMetallic);
        }
        if uri.chars().any(|c| c.is_control() || c.is_whitespace()) || uri.contains('\\') {
            return Err(DeepLinkError::BadParam("uri"));
        }
        // Query values are decoded once, but the authority/action grammar is literal.
        // Do not let URL normalization erase userinfo, empty ports or dot segments.
        let raw_action = uri
            .get(SCHEME.len() + 1..)
            .and_then(|tail| tail.strip_prefix("//"))
            .and_then(|tail| tail.split(['?', '#']).next())
            .ok_or(DeepLinkError::UnknownAction)?;
        if !matches!(raw_action, "play" | "studio/open" | "open" | "install") {
            return Err(DeepLinkError::UnknownAction);
        }
        let url = url::Url::parse(uri).map_err(|_| DeepLinkError::BadParam("uri"))?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
        {
            return Err(DeepLinkError::BadParam("uri"));
        }
        let mut place = None;
        let mut ticket = None;
        let mut path = None;
        for (key, value) in url.query_pairs() {
            let target = match key.as_ref() {
                "place" => (&mut place, "place"),
                "ticket" => (&mut ticket, "ticket"),
                "path" => (&mut path, "path"),
                _ => continue,
            };
            if target.0.replace(value.into_owned()).is_some() {
                return Err(DeepLinkError::DuplicateParam(target.1));
            }
        }
        match (url.host_str(), url.path()) {
            (Some("play"), "") => Ok(Self::Play {
                place: PlaceId::parse(
                    place
                        .as_deref()
                        .ok_or(DeepLinkError::MissingParam("place"))?,
                )?,
                ticket: JoinTicket::parse(
                    ticket
                        .as_deref()
                        .ok_or(DeepLinkError::MissingParam("ticket"))?,
                )?,
            }),
            (Some("studio"), "/open") => Ok(Self::StudioOpen {
                place: place.as_deref().map(PlaceId::parse).transpose()?,
            }),
            (Some("open"), "") => Ok(Self::Open {
                path: path.as_deref().map(SitePath::parse).transpose()?,
            }),
            (Some("install"), "") => Ok(Self::Install),
            _ => Err(DeepLinkError::UnknownAction),
        }
    }

    /// Returns a canonical URI with lowercase scheme, fixed field order and encoded values.
    /// This is transport data containing the ticket; never log it.
    pub fn to_uri(&self) -> String {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        let action = match self {
            Self::Play { place, ticket } => {
                query
                    .append_pair("place", &place.to_string())
                    .append_pair("ticket", ticket.expose());
                "play"
            }
            Self::StudioOpen { place } => {
                if let Some(place) = place {
                    query.append_pair("place", &place.to_string());
                }
                "studio/open"
            }
            Self::Open { path } => {
                if let Some(path) = path {
                    query.append_pair("path", path.as_str());
                }
                "open"
            }
            Self::Install => "install",
        };
        let query = query.finish();
        if query.is_empty() {
            format!("{SCHEME}://{action}")
        } else {
            format!("{SCHEME}://{action}?{query}")
        }
    }
}

pub(crate) fn is_metallic(text: &str) -> bool {
    text.as_bytes()
        .get(..SCHEME.len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(SCHEME.as_bytes()))
        && text.as_bytes().get(SCHEME.len()) == Some(&b':')
}
