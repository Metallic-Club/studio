//! Wire format (spec sections 3.3, 4.5, 7.3): reliable messages in frames, datagrams, bit packing, quantization and
//! the values that travel in them.
//!
//! Every decoder here takes bytes from a peer that may be hostile. Decoders return [`WireError`] on any truncation,
//! garbage or oversize input and never panic; lengths are checked against the bytes actually present before anything
//! is allocated, and nesting is capped ([`MAX_TABLE_DEPTH`]).
//!
//! - Reliable frames ([`frame`]): `[kind: u8][source: u8][varint body length][postcard body]` carrying a
//!   [`ClientMsg`] or a [`ServerMsg`].
//! - Datagrams ([`datagram`]): a 3-byte header (protocol minor, kind, sequence), then a bit-packed snapshot or input
//!   packet, or a postcard unreliable remote call.

pub mod bits;
pub mod datagram;
pub mod frame;
pub mod msg;
pub mod netid;
pub mod quant;
pub mod value;
pub mod varint;

pub use bits::{BitReader, BitWriter};
pub use datagram::{
    DatagramKind, EntityRot, EntityState, InputCmd, InputPacket, MAX_INPUT_CMDS,
    MAX_UNRELIABLE_ARGS, OwnState, SnapshotPacket, UnreliableRemotePacket, datagram_kind,
    decode_input, decode_snapshot, decode_unreliable_remote, encode_input, encode_snapshot,
    encode_unreliable_remote,
};
pub use frame::{Decoded, decode_client, decode_server, encode_client, encode_server};
pub use msg::{
    ClientMsg, Create, MAX_JOIN_SNAPSHOT, PROTOCOL_MINOR, ServerMsg, compress_join, decompress_join,
};
pub use netid::{FIRST_DYNAMIC_NET_ID, NetId, service_net_id};
pub use quant::{
    QuantPos, QuantY, dequantize_orientation, dequantize_position, dequantize_velocity,
    quantize_orientation, quantize_position, quantize_velocity,
};
pub use value::{MAX_TABLE_DEPTH, NetCFrame, NetValue};
pub use varint::{read_i64, read_u64, write_i64, write_u64, zigzag_decode, zigzag_encode};

/// Why bytes could not be encoded or decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    /// The bytes end before the message does.
    #[error("message is truncated")]
    Truncated,
    /// A datagram kind this build does not know (reliable frames report unknown kinds as
    /// [`Decoded::UnknownKind`] instead).
    #[error("unknown message kind {0}")]
    UnknownKind(u8),
    /// The input or output is over a protocol size limit.
    #[error("message is over the size limit")]
    TooLarge,
    /// A field holds a value the protocol does not allow.
    #[error("bad value: {0}")]
    BadValue(&'static str),
    /// The postcard body did not decode (or encode).
    #[error("postcard: {0}")]
    Postcard(String),
}

impl From<postcard::Error> for WireError {
    fn from(e: postcard::Error) -> Self {
        match e {
            postcard::Error::DeserializeUnexpectedEnd => WireError::Truncated,
            other => WireError::Postcard(other.to_string()),
        }
    }
}

/// Decodes a whole postcard body: trailing bytes are an error.
pub(crate) fn postcard_exact<'a, T: serde::Deserialize<'a>>(
    bytes: &'a [u8],
) -> Result<T, WireError> {
    let (value, rest) = postcard::take_from_bytes(bytes)?;
    if rest.is_empty() {
        Ok(value)
    } else {
        Err(WireError::BadValue("trailing bytes after the message body"))
    }
}
