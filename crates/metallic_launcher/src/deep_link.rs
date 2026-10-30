//! `metallic://` deep links.
//!
//! Grammar (scheme case-insensitive, unknown query keys ignored, a repeated known key is an
//! error, values percent-decoded exactly once):
//!
//! - `metallic://play?place=<id>&ticket=<t>`
//! - `metallic://studio/open[?place=<id>]`
//! - `metallic://open[?path=<site path>]`
//! - `metallic://install`
//!
//! A link can come from any web page, email or second process, so [`DeepLink::parse`] treats
//! it as hostile: it is length-limited, must be printable ASCII without spaces (anything else must be
//! percent-encoded), and every
//! value is checked against a narrow shape. Errors never echo the input, and a
//! [`JoinTicket`] never shows in `Debug`.

use std::fmt;

use url::Url;

/// The URI scheme the Launcher registers.
pub const SCHEME: &str = "metallic";
/// Longest accepted link, in bytes.
pub const MAX_URI_LEN: usize = 2048;

const MAX_SITE_PATH_LEN: usize = 512;

/// Why a link was refused. No variant repeats the input.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeepLinkError {
    /// The link is longer than [`MAX_URI_LEN`].
    #[error("the link is too long")]
    TooLong,
    /// The link uses another scheme.
    #[error("the link is not a metallic:// link")]
    NotMetallic,
    /// The link names no known action, or is malformed.
    #[error("the link names no known action")]
    UnknownAction,
    /// A required query value is missing.
    #[error("the link has no `{0}` value")]
    MissingParam(&'static str),
    /// A query value has the wrong shape.
    #[error("the link has a bad `{0}` value")]
    BadParam(&'static str),
    /// A known query key appears more than once.
    #[error("the link repeats `{0}`")]
    DuplicateParam(&'static str),
}

/// A Metallic place id: decimal, 1 to 20 digits, no sign, no leading zero, fits `u64`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceId(u64);

impl PlaceId {
    /// Parses the decimal text of a place id.
    pub fn parse(text: &str) -> Option<PlaceId> {
        let bytes = text.as_bytes();
        let shape_ok = (1..=20).contains(&bytes.len())
            && bytes.iter().all(u8::is_ascii_digit)
            && bytes[0] != b'0';
        if !shape_ok {
            return None;
        }
        text.parse().ok().map(PlaceId)
    }

    /// The number.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PlaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The one-time ticket that lets the Player join a place: `[A-Za-z0-9_-]{8,128}`.
/// `Debug` prints `JoinTicket(..)`.
#[derive(Clone, PartialEq, Eq)]
pub struct JoinTicket(String);

impl JoinTicket {
    /// Checks the shape of `text` and wraps it.
    pub fn parse(text: &str) -> Option<JoinTicket> {
        let ok = (8..=128).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        ok.then(|| JoinTicket(text.to_string()))
    }

    /// The ticket text. Pass it on; never log it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for JoinTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JoinTicket(..)")
    }
}

/// Percent-decodes `text` once. Bytes that do not form valid UTF-8 become U+FFFD.
fn percent_decode_once(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// True when `text` (a path with an optional query) names the site's `/auth/` pages once it is
/// percent-decoded (repeatedly) and its `.` and `..` segments are resolved, or cannot be
/// resolved safely. The sign-in handoff page lives there and a link must never load it.
fn reaches_auth_pages(text: &str) -> bool {
    let path = text.split(['?', '#']).next().unwrap_or("");
    let mut decoded = path.to_string();
    for _ in 0..8 {
        let next = percent_decode_once(&decoded);
        if next == decoded {
            let mut stack: Vec<String> = Vec::new();
            for segment in decoded.split('/').skip(1) {
                match segment.to_lowercase().as_str() {
                    "." | "" => {}
                    ".." => {
                        if stack.pop().is_none() {
                            return true;
                        }
                    }
                    other => stack.push(other.to_string()),
                }
            }
            return decoded.contains('\\')
                || decoded.chars().any(char::is_control)
                || stack.first().is_some_and(|first| first == "auth");
        }
        decoded = next;
    }
    // Still changing after 8 rounds: refuse.
    true
}

/// A path on the Metallic site: starts with `/`, not `//`, no `\`, no control characters,
/// at most 512 bytes. It can never name another host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SitePath(String);

impl SitePath {
    /// Checks `text` and wraps it.
    pub fn new(text: &str) -> Result<SitePath, DeepLinkError> {
        let ok = text.starts_with('/')
            && !text.starts_with("//")
            && text.len() <= MAX_SITE_PATH_LEN
            && !text.contains('\\')
            && !text.chars().any(|c| {
                c.is_control()
                    || matches!(
                        c,
                        '\u{200B}'..='\u{200F}'
                            | '\u{202A}'..='\u{202E}'
                            | '\u{2066}'..='\u{2069}'
                            | '\u{FEFF}'
                    )
            });
        if ok && !reaches_auth_pages(text) {
            Ok(SitePath(text.to_string()))
        } else {
            Err(DeepLinkError::BadParam("path"))
        }
    }

    /// The path text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a `metallic://` link asks the Launcher to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeepLink {
    /// Start the Player and join a place.
    Play {
        /// The place to join.
        place: PlaceId,
        /// The one-time join ticket.
        ticket: JoinTicket,
    },
    /// Open Metallic Studio, optionally on a place.
    StudioOpen {
        /// The place to open, if any.
        place: Option<PlaceId>,
    },
    /// Show the Launcher, optionally on a page of the site.
    Open {
        /// The site page to show, if any.
        path: Option<SitePath>,
    },
    /// Install the Player and Studio.
    Install,
}

/// The four actions, before their values are checked.
#[derive(Clone, Copy)]
enum Action {
    Play,
    StudioOpen,
    Open,
    Install,
}

fn set_once(
    slot: &mut Option<String>,
    name: &'static str,
    value: String,
) -> Result<(), DeepLinkError> {
    if slot.is_some() {
        return Err(DeepLinkError::DuplicateParam(name));
    }
    *slot = Some(value);
    Ok(())
}

fn parse_place(text: &Option<String>) -> Result<Option<PlaceId>, DeepLinkError> {
    match text {
        Some(text) => PlaceId::parse(text)
            .map(Some)
            .ok_or(DeepLinkError::BadParam("place")),
        None => Ok(None),
    }
}

impl DeepLink {
    /// Parses and validates a link.
    pub fn parse(uri: &str) -> Result<DeepLink, DeepLinkError> {
        if uri.len() > MAX_URI_LEN {
            return Err(DeepLinkError::TooLong);
        }
        let (scheme, _) = uri.split_once(':').ok_or(DeepLinkError::NotMetallic)?;
        if !scheme.eq_ignore_ascii_case(SCHEME) {
            return Err(DeepLinkError::NotMetallic);
        }
        // Plain ASCII without spaces or control characters; everything else must be %-encoded.
        // (The url crate would silently drop tabs and newlines.)
        if !uri.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err(DeepLinkError::UnknownAction);
        }
        let url = Url::parse(uri).map_err(|_| DeepLinkError::UnknownAction)?;
        if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
            return Err(DeepLinkError::UnknownAction);
        }
        let host = url
            .host_str()
            .ok_or(DeepLinkError::UnknownAction)?
            .to_ascii_lowercase();
        let path = url.path();
        let no_path = path.is_empty() || path == "/";
        let action = match host.as_str() {
            "play" if no_path => Action::Play,
            "studio" if path == "/open" || path == "/open/" => Action::StudioOpen,
            "open" if no_path => Action::Open,
            "install" if no_path => Action::Install,
            _ => return Err(DeepLinkError::UnknownAction),
        };

        let mut place = None;
        let mut ticket = None;
        let mut site_path = None;
        for (key, value) in url.query_pairs() {
            let value = value.into_owned();
            match (action, key.as_ref()) {
                (Action::Play | Action::StudioOpen, "place") => {
                    set_once(&mut place, "place", value)?
                }
                (Action::Play, "ticket") => set_once(&mut ticket, "ticket", value)?,
                (Action::Open, "path") => set_once(&mut site_path, "path", value)?,
                _ => {}
            }
        }
        match action {
            Action::Play => {
                let place = parse_place(&place)?.ok_or(DeepLinkError::MissingParam("place"))?;
                let ticket = ticket.ok_or(DeepLinkError::MissingParam("ticket"))?;
                let ticket = JoinTicket::parse(&ticket).ok_or(DeepLinkError::BadParam("ticket"))?;
                Ok(DeepLink::Play { place, ticket })
            }
            Action::StudioOpen => Ok(DeepLink::StudioOpen {
                place: parse_place(&place)?,
            }),
            Action::Open => Ok(DeepLink::Open {
                path: site_path.as_deref().map(SitePath::new).transpose()?,
            }),
            Action::Install => Ok(DeepLink::Install),
        }
    }

    /// The canonical link: lowercase scheme, fixed key order, encoded values. This is what is
    /// handed on to other programs, never the text that was received.
    pub fn to_uri(&self) -> String {
        let query = |pairs: &[(&str, &str)]| {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            for (key, value) in pairs {
                serializer.append_pair(key, value);
            }
            serializer.finish()
        };
        match self {
            DeepLink::Play { place, ticket } => format!(
                "{SCHEME}://play?{}",
                query(&[("place", &place.to_string()), ("ticket", ticket.expose())])
            ),
            DeepLink::StudioOpen { place: None } => format!("{SCHEME}://studio/open"),
            DeepLink::StudioOpen { place: Some(place) } => format!(
                "{SCHEME}://studio/open?{}",
                query(&[("place", &place.to_string())])
            ),
            DeepLink::Open { path: None } => format!("{SCHEME}://open"),
            DeepLink::Open { path: Some(path) } => {
                format!("{SCHEME}://open?{}", query(&[("path", path.as_str())]))
            }
            DeepLink::Install => format!("{SCHEME}://install"),
        }
    }
}
