//! A minimal single-threaded event: connect handlers, fire with arguments.
//!
//! Connection identities are shared across all signal instances and argument types, and are
//! never reused after disconnect/drop. Atomic identity allocation permits independent signals
//! to connect on different threads; each individual signal still requires exclusive access.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(0);

fn allocate_connection(counter: &AtomicU64) -> ConnectionId {
    // Only uniqueness is shared; handler storage is local and needs no memory publication.
    // MAX is a terminal exhausted sentinel, never an issued ID. Failed attempts leave it set.
    ConnectionId(
        counter
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("signal connection ID space exhausted"),
    )
}

/// Opaque identity for one handler across all signals, including different argument types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(u64);

type Handler<Args> = Box<dyn FnMut(&Args) + Send>;

/// An event that calls its connected handlers in connection order.
pub struct Signal<Args> {
    handlers: Vec<(ConnectionId, Handler<Args>)>,
}

impl<Args> Default for Signal<Args> {
    fn default() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }
}

impl<Args> Signal<Args> {
    /// Adds a handler and returns the id to disconnect it with.
    ///
    /// # Panics
    /// Panics before registering the handler if the shared u64 identity space is exhausted.
    pub fn connect(&mut self, handler: impl FnMut(&Args) + Send + 'static) -> ConnectionId {
        let id = allocate_connection(&NEXT_CONNECTION);
        self.handlers.push((id, Box::new(handler)));
        id
    }
    /// Removes a handler; unknown ids, including IDs from other signals, are ignored.
    pub fn disconnect(&mut self, id: ConnectionId) {
        self.handlers.retain(|(c, _)| *c != id);
    }
    /// Calls every connected handler with `args`.
    pub fn fire(&mut self, args: &Args) {
        for (_, h) in &mut self.handlers {
            h(args);
        }
    }
    /// Number of connected handlers.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }
    /// True when no handler is connected.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustion_never_wraps_or_reissues_an_identity() {
        // An isolated counter exercises the real allocator boundary without exhausting or
        // resetting global state underneath independently running signal tests.
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_connection(&counter), ConnectionId(u64::MAX - 1));
        for _ in 0..2 {
            assert!(std::panic::catch_unwind(|| allocate_connection(&counter)).is_err());
            assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        }
    }
}
