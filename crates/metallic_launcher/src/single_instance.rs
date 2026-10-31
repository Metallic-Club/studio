//! One Launcher at a time. A second start hands its request to the first and exits.
//!
//! This module holds the message format and the traits; the operating-system part (a named
//! pipe on Windows) is in `platform`. A message is a `u32` little-endian length, then one tag
//! byte (`F` focus, `L` link), then for a link the UTF-8 text. The length counts the tag and
//! the text, never more than [`MAX_HANDOFF_BYTES`], and is checked before the body is read.
//! The first Launcher parses every link again with `DeepLink::parse`; nothing here trusts it.

use std::path::Path;

use crate::error::LauncherError;

/// The longest message body (tag and text) a Launcher will read.
pub const MAX_HANDOFF_BYTES: usize = 4096;

/// What a second Launcher asks the first one to do.
#[derive(Clone, PartialEq, Eq)]
pub enum HandoffMessage {
    /// Show the window.
    Focus,
    /// Handle this `metallic://` link (not yet checked).
    Link(String),
}

impl std::fmt::Debug for HandoffMessage {
    // A link can carry a join ticket, so its text is never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandoffMessage::Focus => f.write_str("Focus"),
            HandoffMessage::Link(_) => f.write_str("Link(..)"),
        }
    }
}

/// Turns a message into its bytes: length, tag, text.
pub fn encode(message: &HandoffMessage) -> Vec<u8> {
    let mut body = Vec::new();
    match message {
        HandoffMessage::Focus => body.push(b'F'),
        HandoffMessage::Link(text) => {
            body.push(b'L');
            body.extend_from_slice(text.as_bytes());
        }
    }
    // A body this large is refused by `decode` and by the senders; the length field saturates.
    let length = u32::try_from(body.len()).unwrap_or(u32::MAX);
    let mut bytes = Vec::with_capacity(4 + body.len());
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&body);
    bytes
}

/// Reads the length field of a message and refuses one above [`MAX_HANDOFF_BYTES`] (or zero).
/// A reader calls this on the first four bytes, before it reads the body.
pub fn body_length(header: [u8; 4]) -> Result<usize, LauncherError> {
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 || length > MAX_HANDOFF_BYTES {
        return Err(LauncherError::BadHandoff("the length is not acceptable"));
    }
    Ok(length)
}

/// Reads one whole message. Anything that is not exactly a valid message is refused.
pub fn decode(bytes: &[u8]) -> Result<HandoffMessage, LauncherError> {
    let (header, body) = bytes
        .split_first_chunk::<4>()
        .ok_or(LauncherError::BadHandoff("the message is too short"))?;
    let length = body_length(*header)?;
    if body.len() != length {
        return Err(LauncherError::BadHandoff("the length does not match"));
    }
    match body.split_first() {
        Some((b'F', [])) => Ok(HandoffMessage::Focus),
        Some((b'L', text)) => std::str::from_utf8(text)
            .map(|text| HandoffMessage::Link(text.to_string()))
            .map_err(|_| LauncherError::BadHandoff("the text is not UTF-8")),
        _ => Err(LauncherError::BadHandoff("unknown message kind")),
    }
}

/// The first Launcher's end: waits for messages from later starts.
pub trait HandoffListener: Send {
    /// Blocks until one message arrives. An error means one bad connection or message; the
    /// caller may call again.
    fn accept(&mut self) -> Result<HandoffMessage, LauncherError>;
}

/// A later start's end: sends one message to the first Launcher.
pub trait HandoffSender {
    /// Sends `message` and returns once the first Launcher has read it.
    fn send(&mut self, message: &HandoffMessage) -> Result<(), LauncherError>;
}

/// The result of asking for the right to be the only Launcher.
pub enum Acquired {
    /// This is the first Launcher; listen for later starts.
    Primary(Box<dyn HandoffListener>),
    /// Another Launcher of this user runs; hand it the request.
    Secondary(Box<dyn HandoffSender>),
}

/// Whether the program at the other end of the pipe (`server`, its image path) is a Launcher
/// like this one (`own`): the same file name, ignoring case. The pipe is already limited to
/// this user; this also keeps a link (which may hold a join ticket) away from some other
/// program of the user's that took the pipe name. An installed copy and a development build
/// live in different folders, so only the name is compared.
pub fn same_program(server: &Path, own: &Path) -> bool {
    match (server.file_name(), own.file_name()) {
        (Some(server), Some(own)) => server.eq_ignore_ascii_case(own),
        _ => false,
    }
}
