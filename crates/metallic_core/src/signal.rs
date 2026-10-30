//! A minimal single-threaded event: connect handlers, fire with arguments.

/// Identifies one connected handler so it can be disconnected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(u64);

type Handler<Args> = Box<dyn FnMut(&Args)>;

/// An event that calls its connected handlers in connection order.
pub struct Signal<Args> {
    next: u64,
    handlers: Vec<(ConnectionId, Handler<Args>)>,
}

impl<Args> Default for Signal<Args> {
    fn default() -> Self {
        Self {
            next: 0,
            handlers: Vec::new(),
        }
    }
}

impl<Args> Signal<Args> {
    /// Adds a handler and returns the id to disconnect it with.
    pub fn connect(&mut self, handler: impl FnMut(&Args) + 'static) -> ConnectionId {
        let id = ConnectionId(self.next);
        self.next += 1;
        self.handlers.push((id, Box::new(handler)));
        id
    }
    /// Removes a handler; unknown ids are ignored.
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
