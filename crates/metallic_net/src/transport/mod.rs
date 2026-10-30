//! Connections between a server and its clients: QUIC over UDP, and an in-memory network with a manual clock for
//! deterministic tests. Both carry the same channels, enforce the same limits and are polled once per tick.
//!
//! Channels (reliable, research 3.1): `Control` is one bidirectional stream opened by the client; `World` one
//! long-lived server-to-client stream; `Remote` one long-lived client-to-server stream; `Bulk` one server-to-client
//! stream per payload (so large payloads never hold up each other). Datagrams are unreliable and unordered.

mod cert;
mod conditioner;
pub mod limits;
mod memory;
mod quic;

use std::net::SocketAddr;
use std::time::Duration;

pub use conditioner::LinkConditioner;
pub use memory::MemoryNet;
pub use quic::{QuicClient, QuicServer};

use crate::NetError;
use limits::{MAX_CLIENT_FRAME, MAX_SERVER_FRAME};

/// A reliable, ordered channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Handshake, acks and control messages, both ways.
    Control,
    /// Replication events, server to client.
    World,
    /// Remote calls, client to server.
    Remote,
    /// Large payloads (join snapshot, assets), server to client; payloads are not ordered among each other.
    Bulk,
}

impl Channel {
    /// Tag byte that starts each QUIC stream of this channel.
    pub(crate) fn tag(self) -> u8 {
        match self {
            Channel::Control => 0,
            Channel::World => 1,
            Channel::Remote => 2,
            Channel::Bulk => 3,
        }
    }

    pub(crate) fn from_tag(tag: u8) -> Option<Channel> {
        match tag {
            0 => Some(Channel::Control),
            1 => Some(Channel::World),
            2 => Some(Channel::Remote),
            3 => Some(Channel::Bulk),
            _ => None,
        }
    }
}

/// Something that arrived on a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// A whole reliable frame.
    Reliable {
        /// Channel it came on.
        channel: Channel,
        /// The frame's bytes.
        frame: Vec<u8>,
    },
    /// An unreliable datagram.
    Datagram(Vec<u8>),
    /// The connection closed; nothing follows.
    Closed(CloseReason),
}

/// Why a connection closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// Closed on purpose by either side.
    Normal,
    /// The peer was silent for the idle timeout, or the client sent no Hello in time.
    Timeout,
    /// The peer broke the protocol (oversize frame, bad stream, undecodable message).
    ProtocolError(String),
    /// The server removed the player.
    Kicked(String),
    /// The transport failed (handshake, I/O, peer reset).
    TransportError(String),
}

impl CloseReason {
    /// QUIC application close code and reason text.
    pub(crate) fn to_wire(&self) -> (u32, &str) {
        match self {
            CloseReason::Normal => (0, ""),
            CloseReason::Timeout => (1, ""),
            CloseReason::ProtocolError(s) => (2, s),
            CloseReason::Kicked(s) => (3, s),
            CloseReason::TransportError(s) => (4, s),
        }
    }

    pub(crate) fn from_wire(code: u64, text: &[u8]) -> CloseReason {
        let text = String::from_utf8_lossy(text).into_owned();
        match code {
            0 => CloseReason::Normal,
            1 => CloseReason::Timeout,
            2 => CloseReason::ProtocolError(text),
            3 => CloseReason::Kicked(text),
            4 => CloseReason::TransportError(text),
            _ => CloseReason::TransportError(format!("close code {code}: {text}")),
        }
    }
}

/// A connection to one peer. Non-blocking: the game loop calls [`Connection::poll`] once per tick.
///
/// After [`Connection::close`] the connection reports nothing more. Dropping a connection without closing it is
/// abrupt: QUIC sends a bare close, the in-memory network models a crashed peer (the other side times out).
pub trait Connection: Send {
    /// Queues a reliable frame. Fails if the connection is closed, the channel goes the other way or the frame is
    /// over [`MAX_SERVER_FRAME`](limits::MAX_SERVER_FRAME). The receiver closes on frames over its own limit.
    fn send_reliable(&mut self, channel: Channel, frame: Vec<u8>) -> Result<(), NetError>;
    /// Sends an unreliable datagram of at most [`MAX_DATAGRAM`](limits::MAX_DATAGRAM) bytes.
    fn send_datagram(&mut self, bytes: Vec<u8>) -> Result<(), NetError>;
    /// Appends everything that arrived since the last poll to `out`.
    fn poll(&mut self, out: &mut Vec<Incoming>);
    /// Current round-trip time estimate.
    fn rtt(&self) -> Duration;
    /// Traffic counters.
    fn stats(&self) -> LinkStats;
    /// Closes the connection and tells the peer why.
    fn close(&mut self, reason: CloseReason);
    /// The peer's address, if the transport has one.
    fn remote_addr(&self) -> Option<SocketAddr>;
}

/// Hands out connections from clients.
pub trait Listener: Send {
    /// The next connection that finished connecting, if any.
    fn accept(&mut self) -> Option<Box<dyn Connection>>;
}

/// Traffic counters of one connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    /// Bytes sent (QUIC: UDP payload bytes; memory: frame and datagram bytes).
    pub bytes_sent: u64,
    /// Bytes received, counted like `bytes_sent`.
    pub bytes_received: u64,
    /// Incoming datagrams dropped (over the rate limit or the size limit, or the queue was full).
    pub datagrams_dropped: u64,
    /// Incoming datagrams handed to `poll`.
    pub datagrams_received: u64,
}

/// Accepts only sequence numbers newer than the newest seen so far, across the `u16` wrap: `seq` is newer when
/// `seq.wrapping_sub(newest) as i16 > 0`. Use it to drop stale or duplicate datagrams.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sequencer {
    newest: Option<u16>,
}

impl Sequencer {
    /// Whether `seq` is newer than everything accepted before; remembers it if so.
    pub fn accept(&mut self, seq: u16) -> bool {
        let newer = match self.newest {
            None => true,
            Some(newest) => (seq.wrapping_sub(newest) as i16) > 0,
        };
        if newer {
            self.newest = Some(seq);
        }
        newer
    }
}

/// Which end of a connection this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Client,
    Server,
}

impl Side {
    pub(crate) fn may_send(self, channel: Channel) -> bool {
        match self {
            Side::Client => matches!(channel, Channel::Control | Channel::Remote),
            Side::Server => !matches!(channel, Channel::Remote),
        }
    }

    /// Largest reliable frame this side accepts from its peer.
    pub(crate) fn max_incoming_frame(self) -> usize {
        match self {
            Side::Client => MAX_SERVER_FRAME,
            Side::Server => MAX_CLIENT_FRAME,
        }
    }

    /// The checks every `send_reliable` runs before queueing.
    pub(crate) fn check_send(self, channel: Channel, len: usize) -> Result<(), NetError> {
        if !self.may_send(channel) {
            return Err(NetError::WrongDirection(channel));
        }
        if len > MAX_SERVER_FRAME {
            return Err(NetError::FrameTooLarge {
                size: len,
                max: MAX_SERVER_FRAME,
            });
        }
        Ok(())
    }
}
