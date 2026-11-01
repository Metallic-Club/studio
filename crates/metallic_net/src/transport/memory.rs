//! In-memory network with a shared manual clock, for deterministic tests.
//!
//! Nothing moves until [`MemoryNet::advance`]. Each connection has its own [`LinkConditioner`], applied in both
//! directions. Reliable frames lost with probability `loss` are re-sent after one round trip, so they arrive late but
//! in order per channel (`Bulk` payloads are independent of each other, as on QUIC); datagrams are lost or reordered
//! by jitter and never re-sent. The server side enforces the limits of [`limits`](super::limits), and both sides
//! send keep-alives and close after the idle timeout, as on QUIC. The same seeds give the same arrivals.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::conditioner::Rng;
use super::limits::{DatagramLimiter, HELLO_TIMEOUT, IDLE_TIMEOUT, KEEP_ALIVE, MAX_DATAGRAM};
use super::receive::ReceiveState;
use super::{
    Channel, CloseReason, Connection, Incoming, LinkConditioner, LinkStats, Listener, PollBudget,
    PollReport, Side,
};
use crate::NetError;

/// A simulated network: one listener, any number of client connections, one manual clock.
#[derive(Clone)]
pub struct MemoryNet {
    state: Arc<Mutex<NetState>>,
}

impl MemoryNet {
    /// An empty network at time zero. `seed` (with each connection's own seed) drives every random choice.
    pub fn new(seed: u64) -> MemoryNet {
        MemoryNet {
            state: Arc::new(Mutex::new(NetState {
                now: Duration::ZERO,
                seed,
                ends: Vec::new(),
                links: Vec::new(),
                flights: BinaryHeap::new(),
                next_order: 0,
                accept_queue: VecDeque::new(),
            })),
        }
    }

    /// The server's listener. Every listener of one network shares the same queue of new connections.
    pub fn listener(&self) -> Box<dyn Listener> {
        Box::new(MemoryListener {
            state: self.state.clone(),
        })
    }

    /// Connects a new client. The server side is ready to accept at once; the client must send its first Control
    /// frame within [`HELLO_TIMEOUT`].
    pub fn connect(&self, conditioner: LinkConditioner) -> Box<dyn Connection> {
        let mut state = lock(&self.state);
        let link = state.links.len();
        let seed = state.seed
            ^ conditioner.seed.rotate_left(17)
            ^ (link as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        state.links.push(Link {
            cond: conditioner,
            rng: Rng::new(seed),
        });
        let now = state.now;
        state.ends.push(End::new(Side::Client, now));
        state.ends.push(End::new(Side::Server, now));
        let server = 2 * link + 1;
        state.ends[server].hello_deadline = Some(now + HELLO_TIMEOUT);
        state.accept_queue.push_back(server);
        Box::new(MemoryConnection {
            state: self.state.clone(),
            end: 2 * link,
        })
    }

    /// Moves the clock forward by `dt`, delivering packets and running timers in time order.
    pub fn advance(&self, dt: Duration) {
        lock(&self.state).advance(dt);
    }
}

fn lock(state: &Mutex<NetState>) -> MutexGuard<'_, NetState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

struct NetState {
    now: Duration,
    seed: u64,
    /// Connection ends: `2 * link` is the client, `2 * link + 1` the server.
    ends: Vec<End>,
    links: Vec<Link>,
    flights: BinaryHeap<Flight>,
    next_order: u64,
    accept_queue: VecDeque<usize>,
}

struct Link {
    cond: LinkConditioner,
    rng: Rng,
}

struct End {
    side: Side,
    /// False once the connection object was dropped.
    alive: bool,
    /// True once closed; nothing more is reported or sent.
    closed: bool,
    inbox: VecDeque<Incoming>,
    receive: ReceiveState,
    stats: LinkStats,
    last_recv: Duration,
    last_sent: Duration,
    /// Server only: when a client that sent no Control frame yet gets closed.
    hello_deadline: Option<Duration>,
    limiter: DatagramLimiter,
    /// Latest arrival scheduled per ordered channel (Control, World, Remote), so later frames never overtake.
    ordered_tail: [Duration; 3],
}

impl End {
    fn new(side: Side, now: Duration) -> End {
        End {
            side,
            alive: true,
            closed: false,
            inbox: VecDeque::new(),
            receive: ReceiveState::default(),
            stats: LinkStats::default(),
            last_recv: now,
            last_sent: now,
            hello_deadline: None,
            limiter: DatagramLimiter::default(),
            ordered_tail: [now; 3],
        }
    }

    fn live(&self) -> bool {
        self.alive && !self.closed
    }
}

enum Packet {
    Reliable(Channel, Vec<u8>),
    Datagram(Vec<u8>),
    Close(CloseReason),
    KeepAlive,
}

struct Flight {
    at: Duration,
    order: u64,
    to: usize,
    packet: Packet,
}

impl PartialEq for Flight {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Flight {}
impl PartialOrd for Flight {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Flight {
    /// Earliest first in a max-heap; equal times in send order.
    fn cmp(&self, other: &Self) -> Ordering {
        (other.at, other.order).cmp(&(self.at, self.order))
    }
}

impl NetState {
    fn push(&mut self, at: Duration, to: usize, packet: Packet) {
        self.next_order += 1;
        self.flights.push(Flight {
            at,
            order: self.next_order,
            to,
            packet,
        });
    }

    /// Schedules a reliable packet from `from`: re-sent after one round trip each time it is lost.
    fn send_reliable_packet(&mut self, from: usize, ordered: Option<usize>, packet: Packet) {
        let link = &mut self.links[from / 2];
        let mut at = self.now;
        while link.cond.lose(&mut link.rng) {
            at += link.cond.round_trip();
        }
        at += link.cond.sample_delay(&mut link.rng);
        if let Some(slot) = ordered {
            let tail = &mut self.ends[from].ordered_tail[slot];
            at = at.max(*tail);
            *tail = at;
        }
        self.ends[from].last_sent = self.now;
        self.push(at, from ^ 1, packet);
    }

    /// Schedules an unreliable packet from `from`.
    fn send_unreliable_packet(&mut self, from: usize, packet: Packet) {
        self.ends[from].last_sent = self.now;
        let link = &mut self.links[from / 2];
        if link.cond.lose(&mut link.rng) {
            return;
        }
        let at = self.now + link.cond.sample_delay(&mut link.rng);
        self.push(at, from ^ 1, packet);
    }

    /// Closes `end` and tells the peer; `report` also puts `Closed` in this end's own inbox.
    fn shut(&mut self, end: usize, reason: CloseReason, report: bool) {
        if self.ends[end].closed {
            return;
        }
        self.ends[end].closed = true;
        if report {
            self.ends[end]
                .inbox
                .push_back(Incoming::Closed(reason.clone()));
        } else {
            self.ends[end].inbox.clear();
        }
        self.send_reliable_packet(end, None, Packet::Close(reason));
    }

    fn deliver(&mut self, flight: Flight) {
        let now = self.now;
        let to = flight.to;
        let end = &mut self.ends[to];
        if !end.live() {
            return;
        }
        end.last_recv = now;
        match flight.packet {
            Packet::KeepAlive => {}
            Packet::Close(reason) => {
                end.closed = true;
                end.inbox.push_back(Incoming::Closed(reason));
            }
            Packet::Datagram(bytes) => {
                if end.side == Side::Server && !end.limiter.allow(now) {
                    end.stats.datagrams_dropped += 1;
                } else {
                    end.stats.datagrams_received += 1;
                    end.stats.bytes_received += bytes.len() as u64;
                    end.inbox.push_back(Incoming::Datagram(bytes));
                }
            }
            Packet::Reliable(channel, frame) => {
                let max = end.side.max_incoming_frame();
                if frame.len() > max {
                    let reason = format!(
                        "frame of {} bytes is over the {max}-byte limit",
                        frame.len()
                    );
                    self.shut(to, CloseReason::ProtocolError(reason), true);
                    return;
                }
                end.stats.bytes_received += frame.len() as u64;
                if channel == Channel::Control {
                    end.hello_deadline = None;
                }
                end.inbox.push_back(Incoming::Reliable { channel, frame });
            }
        }
    }

    /// Earliest timer of any live end.
    fn next_timer(&self) -> Option<Duration> {
        self.ends
            .iter()
            .filter(|e| e.live())
            .flat_map(|e| {
                [
                    Some(e.last_recv + IDLE_TIMEOUT),
                    Some(e.last_sent + KEEP_ALIVE),
                    e.hello_deadline,
                ]
            })
            .flatten()
            .min()
    }

    fn run_timers(&mut self) {
        let now = self.now;
        for id in 0..self.ends.len() {
            let end = &mut self.ends[id];
            if !end.live() {
                continue;
            }
            if end.hello_deadline.is_some_and(|d| d <= now) {
                self.shut(id, CloseReason::Timeout, true);
            } else if end.last_recv + IDLE_TIMEOUT <= now {
                // Like QUIC's idle timeout: silent, each side decides on its own.
                end.closed = true;
                end.inbox.push_back(Incoming::Closed(CloseReason::Timeout));
            } else if end.last_sent + KEEP_ALIVE <= now {
                self.send_unreliable_packet(id, Packet::KeepAlive);
            }
        }
    }

    fn advance(&mut self, dt: Duration) {
        let target = self.now + dt;
        loop {
            let next_flight = self.flights.peek().map(|f| f.at);
            let next = match (next_flight, self.next_timer()) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => match a.or(b) {
                    Some(t) => t,
                    None => break,
                },
            };
            if next > target {
                break;
            }
            self.now = self.now.max(next);
            while self.flights.peek().is_some_and(|f| f.at <= self.now) {
                if let Some(flight) = self.flights.pop() {
                    self.deliver(flight);
                }
            }
            self.run_timers();
        }
        self.now = target;
    }
}

struct MemoryListener {
    state: Arc<Mutex<NetState>>,
}

impl Listener for MemoryListener {
    fn accept(&mut self) -> Option<Box<dyn Connection>> {
        let end = lock(&self.state).accept_queue.pop_front()?;
        Some(Box::new(MemoryConnection {
            state: self.state.clone(),
            end,
        }))
    }
}

struct MemoryConnection {
    state: Arc<Mutex<NetState>>,
    end: usize,
}

impl Connection for MemoryConnection {
    fn send_reliable(&mut self, channel: Channel, frame: Vec<u8>) -> Result<(), NetError> {
        let mut state = lock(&self.state);
        let end = &mut state.ends[self.end];
        if end.closed {
            return Err(NetError::Closed);
        }
        end.side.check_send(channel, frame.len())?;
        end.stats.bytes_sent += frame.len() as u64;
        let ordered = match channel {
            Channel::Control => Some(0),
            Channel::World => Some(1),
            Channel::Remote => Some(2),
            Channel::Bulk => None,
        };
        state.send_reliable_packet(self.end, ordered, Packet::Reliable(channel, frame));
        Ok(())
    }

    fn send_datagram(&mut self, bytes: Vec<u8>) -> Result<(), NetError> {
        let mut state = lock(&self.state);
        let end = &mut state.ends[self.end];
        if end.closed {
            return Err(NetError::Closed);
        }
        if bytes.len() > MAX_DATAGRAM {
            return Err(NetError::DatagramTooLarge {
                size: bytes.len(),
                max: MAX_DATAGRAM,
            });
        }
        end.stats.bytes_sent += bytes.len() as u64;
        state.send_unreliable_packet(self.end, Packet::Datagram(bytes));
        Ok(())
    }

    fn poll(&mut self, out: &mut Vec<Incoming>) {
        self.poll_bounded(out, PollBudget::UNRESTRICTED);
    }

    fn poll_bounded(&mut self, out: &mut Vec<Incoming>, budget: PollBudget) -> PollReport {
        let mut state = lock(&self.state);
        let End { receive, inbox, .. } = &mut state.ends[self.end];
        receive.poll(out, budget, || inbox.pop_front())
    }

    fn rtt(&self) -> Duration {
        lock(&self.state).links[self.end / 2].cond.round_trip()
    }

    fn stats(&self) -> LinkStats {
        lock(&self.state).ends[self.end].stats
    }

    fn close(&mut self, reason: CloseReason) {
        let mut state = lock(&self.state);
        state.shut(self.end, reason, false);
        // A peer may already have closed while an oversized head was retained. Local close
        // still discards all pending delivery, including that earlier peer-close event.
        state.ends[self.end].receive.close();
        state.ends[self.end].inbox.clear();
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        None
    }
}

impl Drop for MemoryConnection {
    fn drop(&mut self) {
        let mut state = lock(&self.state);
        let end = &mut state.ends[self.end];
        end.alive = false;
        end.inbox = VecDeque::new();
        end.receive.close();
    }
}
