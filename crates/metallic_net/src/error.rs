//! Errors returned by `metallic_net`.

use crate::transport::Channel;

/// What went wrong in the network layer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// A datagram is larger than [`MAX_DATAGRAM`](crate::transport::limits::MAX_DATAGRAM) or than the path carries.
    #[error("datagram of {size} bytes is over the {max}-byte limit")]
    DatagramTooLarge {
        /// Size of the refused datagram.
        size: usize,
        /// Largest size allowed.
        max: usize,
    },
    /// A reliable frame is larger than any peer accepts.
    #[error("frame of {size} bytes is over the {max}-byte limit")]
    FrameTooLarge {
        /// Size of the refused frame.
        size: usize,
        /// Largest size allowed.
        max: usize,
    },
    /// This side may not send on the channel (a client on `World` or `Bulk`, a server on `Remote`).
    #[error("{0:?} cannot be sent from this side")]
    WrongDirection(Channel),
    /// The connection is closed.
    #[error("connection closed")]
    Closed,
    /// Connecting did not finish in time.
    #[error("timed out")]
    Timeout,
    /// The handshake failed: wrong certificate pin, wrong ALPN or refused by the server.
    #[error("connection failed: {0}")]
    Connect(String),
    /// A socket or async runtime error.
    #[error("i/o error: {0}")]
    Io(String),
    /// The certificate or TLS configuration could not be built.
    #[error("tls setup failed: {0}")]
    Tls(String),
}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::Io(e.to_string())
    }
}
