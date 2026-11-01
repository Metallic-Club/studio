# Bounded transport polling

Drivers that need a receive-work limit use `Connection::poll_bounded(&mut out,
PollBudget { max_events, max_bytes })`. Both MemoryNet and QUIC implement the same
strict contract. Existing `poll(&mut out)` remains an explicit unrestricted drain;
calling it once per tick does not make it bounded. This primitive does not add a
runtime driver, wire format, admission or playable multiplayer.

Only events appended by this call are charged; existing output is preserved and
excluded. Reliable and Datagram payloads cost their buffer length. Closed costs
one event and the UTF-8 byte length of its ProtocolError/Kicked/TransportError
text; Normal and Timeout cost zero bytes. Framing, channel tags and allocation
capacity are not charged. The event limit independently bounds empty payloads.
Default PollBudget is zero events/zero bytes, not unlimited.

With zero events the call dequeues nothing, including a pending Closed. With
positive events and zero bytes, empty payloads and text-free closes can be
delivered. No byte overshoot is allowed: an indivisible event that does not fit
remains the FIFO head. `PollStop::ByteLimit { next_event_bytes }` reports its full
payload charge. A later call can grant enough room; alternatively the driver can
explicitly close. Repeated small budgets never drop or bypass reliable data or
the Closed behind it. Reports charge a deferred event only when it is appended.

The other stop reasons are Empty (nothing immediately ready), EventLimit (count
exhausted, without an extra dequeue probe), and Closed (delivered or already
locally/delivery closed). EventLimit does not promise that more work is queued.
Closed is delivered at most once and nothing follows it. Local `close()` discards
pending delivery even if a peer close is already queued. Transport LinkStats
continue counting network receipt, independently from per-call PollReport.

Memory polling acquires the shared mutex and removes at most the allowed number
of deque heads. Its manual `MemoryNet::advance` separately delivers flights and
runs timers for all connections; that work is outside this bound. QUIC polling
uses at most the allowed dequeue attempts even when background producers keep
the channel ready. It may retain one whole event outside the channel while
blocked by bytes (up to the protocol's existing incoming frame limit). Legacy
polling also flushes that retained head first.

This is not a wall-clock or total-memory guarantee: lock contention, caller output
growth, background network parsing/allocations and the manual clock remain
separate. Reuse output capacity. A runtime driver must actually call the bounded
method and separately cap connection accepts and its decoding/processing work.
No arbitrary tick budget is chosen by this library.

The new method is required, with no unrestricted default fallback. Existing
Connection callers keep their source API. External implementors of the public
trait must implement the new contract explicitly; this is not a claim that old
external implementations compile unchanged.
