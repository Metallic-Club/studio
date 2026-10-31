//! Shared game-thread delivery accounting; neither transport performs background work here.
use super::{CloseReason, Incoming};

/// Strict limits for one [`Connection::poll_bounded`](super::Connection::poll_bounded) call.
///
/// Payload bytes mean Reliable/Datagram buffer length, or the UTF-8 length of a Closed reason's
/// text (Normal/Timeout cost zero). Channel tags, allocation capacity and wire framing are not
/// charged; the independent event limit bounds empty payloads and per-event overhead. Existing
/// output entries are excluded. Default is zero events/zero bytes, not an unrestricted budget.
///
/// This bounds delivered payload and queue-dequeue attempts, not wall-clock time, background
/// network reads/allocations, lock contention or [`MemoryNet::advance`](super::MemoryNet::advance).
/// Reuse a sufficiently sized output buffer to avoid growth allocations. QUIC may retain one
/// whole event outside its channel when that event exceeds the remaining byte budget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollBudget {
    /// Maximum number of events to append; zero never consumes a pending event.
    pub max_events: usize,
    /// Maximum payload bytes to append; zero still admits empty payloads and text-free closes.
    pub max_bytes: usize,
}

impl PollBudget {
    pub(super) const UNRESTRICTED: Self = Self {
        max_events: usize::MAX,
        max_bytes: usize::MAX,
    };
}

/// Why a bounded receive call stopped. A limit does not assert that more events are available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollStop {
    /// No event was immediately available when checked.
    Empty,
    /// The event budget was used, or was zero. No extra dequeue probes are made.
    EventLimit,
    /// The next whole event exceeds the remaining byte budget and is retained at the head.
    /// Raise a later call's budget or explicitly close; small budgets never force an overshoot.
    ByteLimit {
        /// Full payload charge of the pending event, not just its excess over remaining bytes.
        next_event_bytes: usize,
    },
    /// Closed was delivered, or the connection was already locally/delivery closed.
    Closed,
}

/// Events and payload bytes appended by this call only; retained events are not charged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollReport {
    /// Number of appended events, including a delivered Closed.
    pub events: usize,
    /// Sum of appended payload charges, using [`PollBudget`]'s accounting.
    pub bytes: usize,
    /// Why the operation stopped.
    pub stop: PollStop,
}

#[derive(Default)]
pub(super) struct ReceiveState {
    pending: Option<Incoming>,
    closed: bool,
}

impl ReceiveState {
    pub(super) fn is_closed(&self) -> bool {
        self.closed
    }

    pub(super) fn close(&mut self) {
        self.pending = None;
        self.closed = true;
    }

    pub(super) fn poll(
        &mut self,
        out: &mut Vec<Incoming>,
        budget: PollBudget,
        mut next: impl FnMut() -> Option<Incoming>,
    ) -> PollReport {
        let mut report = PollReport {
            events: 0,
            bytes: 0,
            stop: PollStop::Closed,
        };
        if self.closed {
            return report;
        }
        loop {
            if report.events == budget.max_events {
                report.stop = PollStop::EventLimit;
                return report;
            }
            let Some(event) = self.pending.take().or_else(&mut next) else {
                report.stop = PollStop::Empty;
                return report;
            };
            let bytes = payload_bytes(&event);
            // Subtraction is safe: only events that fit have been charged. This also avoids
            // overflow when callers explicitly request usize::MAX.
            if bytes > budget.max_bytes - report.bytes {
                self.pending = Some(event);
                report.stop = PollStop::ByteLimit {
                    next_event_bytes: bytes,
                };
                return report;
            }
            let closed = matches!(event, Incoming::Closed(_));
            out.push(event);
            report.events += 1;
            report.bytes += bytes;
            if closed {
                self.closed = true;
                report.stop = PollStop::Closed;
                return report;
            }
        }
    }
}

fn payload_bytes(event: &Incoming) -> usize {
    match event {
        Incoming::Reliable { frame, .. } | Incoming::Datagram(frame) => frame.len(),
        Incoming::Closed(
            CloseReason::ProtocolError(text)
            | CloseReason::Kicked(text)
            | CloseReason::TransportError(text),
        ) => text.len(),
        Incoming::Closed(CloseReason::Normal | CloseReason::Timeout) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn an_always_ready_producer_cannot_extend_the_call() {
        let mut state = ReceiveState::default();
        let mut probes = 0;
        let mut out = vec![Incoming::Datagram(vec![99; 50])];
        let report = state.poll(
            &mut out,
            PollBudget {
                max_events: 7,
                max_bytes: 0,
            },
            || {
                probes += 1;
                assert!(probes <= 7, "producer fetched past count limit");
                Some(Incoming::Datagram(Vec::new()))
            },
        );
        assert_eq!(probes, 7);
        assert_eq!(out.len(), 8);
        assert_eq!(
            report,
            PollReport {
                events: 7,
                bytes: 0,
                stop: PollStop::EventLimit
            }
        );
    }

    #[test]
    fn a_blocked_head_and_close_are_not_skipped_or_charged_twice() {
        let mut state = ReceiveState::default();
        let mut queue = VecDeque::from([
            Incoming::Datagram(vec![1; 5]),
            Incoming::Closed(CloseReason::Kicked("é".into())),
            Incoming::Datagram(vec![9]),
        ]);
        let mut out = Vec::new();
        for _ in 0..2 {
            assert_eq!(
                state.poll(
                    &mut out,
                    PollBudget {
                        max_events: 8,
                        max_bytes: 4
                    },
                    || queue.pop_front()
                ),
                PollReport {
                    events: 0,
                    bytes: 0,
                    stop: PollStop::ByteLimit {
                        next_event_bytes: 5
                    }
                }
            );
        }
        assert_eq!(queue.len(), 2);
        let report = state.poll(
            &mut out,
            PollBudget {
                max_events: 8,
                max_bytes: 5,
            },
            || queue.pop_front(),
        );
        assert_eq!(
            report,
            PollReport {
                events: 1,
                bytes: 5,
                stop: PollStop::ByteLimit {
                    next_event_bytes: 2
                }
            }
        );
        assert_eq!(
            state
                .poll(&mut out, PollBudget::default(), || panic!("zero probes"))
                .stop,
            PollStop::EventLimit
        );
        assert_eq!(
            state.poll(
                &mut out,
                PollBudget {
                    max_events: 1,
                    max_bytes: 2
                },
                || queue.pop_front()
            ),
            PollReport {
                events: 1,
                bytes: 2,
                stop: PollStop::Closed
            }
        );
        assert_eq!(
            state
                .poll(&mut out, PollBudget::UNRESTRICTED, || panic!(
                    "nothing after close"
                ))
                .stop,
            PollStop::Closed
        );
        assert_eq!(out.len(), 2);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn local_close_discards_a_blocked_event() {
        let mut state = ReceiveState::default();
        let mut out = Vec::new();
        state.poll(
            &mut out,
            PollBudget {
                max_events: 1,
                max_bytes: 0,
            },
            || Some(Incoming::Datagram(vec![1])),
        );
        state.close();
        assert!(state.pending.is_none());
        assert_eq!(
            state
                .poll(&mut out, PollBudget::UNRESTRICTED, || panic!("closed"))
                .stop,
            PollStop::Closed
        );
        assert!(out.is_empty());
    }
}
