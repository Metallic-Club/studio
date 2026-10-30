//! Protocol limits (spec section 7.3). The server side of every transport enforces them.

use std::collections::VecDeque;
use std::time::Duration;

/// Largest datagram payload either side may send, in bytes.
pub const MAX_DATAGRAM: usize = 1200;
/// Largest reliable frame a client may send, in bytes; a larger one closes the connection with a protocol error.
pub const MAX_CLIENT_FRAME: usize = 64 * 1024;
/// Largest reliable frame a server may send, in bytes (also the most any side will send).
pub const MAX_SERVER_FRAME: usize = 4 << 20;
/// Datagrams a server accepts from one client in any one second; the excess is dropped and counted.
pub const CLIENT_DATAGRAMS_PER_SEC: usize = 120;
/// Streams a client may have open at once: its Control stream plus three unidirectional streams.
pub const CLIENT_MAX_STREAMS: u32 = 4;
/// A connection that hears nothing from its peer for this long closes with `CloseReason::Timeout`.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(10);
/// A client must send its first Control frame (the Hello) this soon after connecting, or the server closes it.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
/// QUIC application protocol; the protocol major version is part of it.
pub const ALPN: &[u8] = b"metallic/1";

/// A side sends a keep-alive when it has sent nothing for this long, so quiet connections stay open.
pub(crate) const KEEP_ALIVE: Duration = Duration::from_secs(2);

/// Sliding one-second window over accepted datagrams.
#[derive(Debug, Default)]
pub(crate) struct DatagramLimiter {
    accepted: VecDeque<Duration>,
}

impl DatagramLimiter {
    /// Whether a datagram arriving at `now` (any monotonic clock) is within the rate; records it if so.
    pub(crate) fn allow(&mut self, now: Duration) -> bool {
        while self
            .accepted
            .front()
            .is_some_and(|&t| now.saturating_sub(t) >= Duration::from_secs(1))
        {
            self.accepted.pop_front();
        }
        if self.accepted.len() >= CLIENT_DATAGRAMS_PER_SEC {
            return false;
        }
        self.accepted.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_slides() {
        let mut l = DatagramLimiter::default();
        let ms = Duration::from_millis;
        assert_eq!((0..200).filter(|&i| l.allow(ms(i))).count(), 120);
        assert!(!l.allow(ms(999)));
        assert!(l.allow(ms(1000)), "the first datagram left the window");
        assert!(!l.allow(ms(1000)));
    }
}
