//! Reliable frames: `[kind: u8][source: u8][varint body length][postcard body]`.
//!
//! `source` is 0 from a server or client directly; an edge relay later stamps which server a frame came from. A
//! frame of a kind this build does not know decodes as [`Decoded::UnknownKind`] (it is still length-checked): a
//! client skips it and logs the kind once (spec 3.3); a server closes the connection (spec 7.3, "known kind").

use super::WireError;
use super::msg::{ClientMsg, ServerMsg};
use super::varint::{read_u64, write_u64};
use crate::transport::limits::{MAX_CLIENT_FRAME, MAX_SERVER_FRAME};

/// A decoded frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Decoded<T> {
    /// A message this build knows.
    Msg {
        /// The frame's source byte.
        source: u8,
        /// The message.
        msg: T,
    },
    /// A well-formed frame of an unknown kind; skip it.
    UnknownKind(u8),
}

fn encode(kind: u8, body: &[u8], max: usize) -> Result<Vec<u8>, WireError> {
    let mut out = Vec::with_capacity(body.len() + 12);
    out.push(kind);
    out.push(0);
    write_u64(&mut out, body.len() as u64);
    out.extend_from_slice(body);
    if out.len() > max {
        return Err(WireError::TooLarge);
    }
    Ok(out)
}

/// Splits a frame into kind, source and body after checking its size and length field.
fn split(bytes: &[u8], max: usize) -> Result<(u8, u8, &[u8]), WireError> {
    if bytes.len() > max {
        return Err(WireError::TooLarge);
    }
    let (&kind, rest) = bytes.split_first().ok_or(WireError::Truncated)?;
    let (&source, rest) = rest.split_first().ok_or(WireError::Truncated)?;
    let (len, used) = read_u64(rest)?;
    let body = rest.get(used..).ok_or(WireError::Truncated)?;
    if len > max as u64 {
        return Err(WireError::TooLarge);
    }
    match (body.len() as u64).cmp(&len) {
        std::cmp::Ordering::Less => Err(WireError::Truncated),
        std::cmp::Ordering::Greater => Err(WireError::BadValue("trailing bytes after the frame")),
        std::cmp::Ordering::Equal => Ok((kind, source, body)),
    }
}

/// Encodes a server message as a frame with source 0.
///
/// Errors: [`WireError::TooLarge`] over [`MAX_SERVER_FRAME`], [`WireError::BadValue`] for tables nested deeper than
/// [`MAX_TABLE_DEPTH`](super::MAX_TABLE_DEPTH).
pub fn encode_server(m: &ServerMsg) -> Result<Vec<u8>, WireError> {
    let (kind, body) = m.to_body()?;
    encode(kind, &body, MAX_SERVER_FRAME)
}

/// Decodes one frame from a server. Refuses frames over [`MAX_SERVER_FRAME`] before reading them; the body must
/// decode completely.
pub fn decode_server(bytes: &[u8]) -> Result<Decoded<ServerMsg>, WireError> {
    let (kind, source, body) = split(bytes, MAX_SERVER_FRAME)?;
    Ok(match ServerMsg::from_body(kind, body)? {
        Some(msg) => Decoded::Msg { source, msg },
        None => Decoded::UnknownKind(kind),
    })
}

/// Encodes a client message as a frame with source 0.
///
/// Errors: [`WireError::TooLarge`] over [`MAX_CLIENT_FRAME`] (the server would close the connection; a remote call
/// reports "remote payload too large"), [`WireError::BadValue`] for tables nested too deeply.
pub fn encode_client(m: &ClientMsg) -> Result<Vec<u8>, WireError> {
    let (kind, body) = m.to_body()?;
    encode(kind, &body, MAX_CLIENT_FRAME)
}

/// Decodes one frame from a client. Refuses frames over [`MAX_CLIENT_FRAME`] (64 KiB) before reading them; the body
/// must decode completely. The source byte is returned as sent: a client has no reason to send anything but 0, so the
/// server may refuse or ignore other values.
pub fn decode_client(bytes: &[u8]) -> Result<Decoded<ClientMsg>, WireError> {
    let (kind, source, body) = split(bytes, MAX_CLIENT_FRAME)?;
    Ok(match ClientMsg::from_body(kind, body)? {
        Some(msg) => Decoded::Msg { source, msg },
        None => Decoded::UnknownKind(kind),
    })
}
