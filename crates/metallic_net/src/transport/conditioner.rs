//! Link conditioner: added latency, jitter and loss, for tests and Studio's network simulation.
//!
//! The in-memory network applies it to every packet. On QUIC it wraps the server's UDP socket, so QUIC itself sees
//! the delay and the loss (and retransmits, measures RTT and controls congestion as on a real bad link).

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fmt;
use std::io::{self, IoSliceMut};
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use quinn::udp::{EcnCodepoint, RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, UdpPoller};
use tokio::time::{Instant, Sleep};

/// How to degrade a link. The default is a perfect link.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinkConditioner {
    /// One-way delay added to every packet (at most 10 s is applied).
    pub latency: Duration,
    /// Each packet's delay varies by up to this much either way (never below zero), which reorders packets
    /// (at most 10 s is applied).
    pub jitter: Duration,
    /// Chance that a packet is lost, `0..=1` (at most 0.99 is applied, so reliable frames still arrive).
    pub loss: f32,
    /// Seed of the random choices, so a run can be repeated.
    pub seed: u64,
}

/// Largest latency or jitter the conditioner applies.
pub(crate) const MAX_DELAY: Duration = Duration::from_secs(10);
/// Largest loss chance the conditioner applies, so reliable frames always get through eventually.
pub(crate) const MAX_LOSS: f64 = 0.99;

impl LinkConditioner {
    /// Round trip the link adds (twice the latency).
    pub(crate) fn round_trip(&self) -> Duration {
        self.latency.min(MAX_DELAY) * 2
    }

    /// Loss chance clamped to `0..=MAX_LOSS`; NaN counts as no loss.
    pub(crate) fn loss_chance(&self) -> f64 {
        if self.loss.is_nan() {
            0.0
        } else {
            f64::from(self.loss).clamp(0.0, MAX_LOSS)
        }
    }

    /// One packet's delay: latency plus a uniform jitter offset, at least zero.
    pub(crate) fn sample_delay(&self, rng: &mut Rng) -> Duration {
        let latency = self.latency.min(MAX_DELAY);
        let jitter = self.jitter.min(MAX_DELAY);
        if jitter.is_zero() {
            return latency;
        }
        let offset = (rng.next_f64() * 2.0 - 1.0) * jitter.as_secs_f64();
        Duration::try_from_secs_f64((latency.as_secs_f64() + offset).max(0.0)).unwrap_or(latency)
    }

    /// Whether the next packet is lost.
    pub(crate) fn lose(&self, rng: &mut Rng) -> bool {
        let chance = self.loss_chance();
        chance > 0.0 && rng.next_f64() < chance
    }
}

/// Small deterministic generator (SplitMix64).
#[derive(Clone, Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub(crate) fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A UDP socket that delays and drops packets in both directions.
pub(crate) struct ConditionedSocket {
    inner: Arc<dyn AsyncUdpSocket>,
    cond: LinkConditioner,
    state: Mutex<HeldState>,
}

struct HeldState {
    rng: Rng,
    /// Received packets waiting for their delivery time.
    held: BinaryHeap<Held>,
    order: u64,
    timer: Option<Pin<Box<Sleep>>>,
    scratch: Vec<u8>,
}

struct Held {
    at: Instant,
    order: u64,
    from: SocketAddr,
    ecn: Option<EcnCodepoint>,
    dst_ip: Option<IpAddr>,
    data: Vec<u8>,
}

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Held {}
impl PartialOrd for Held {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Held {
    /// Earliest first in a max-heap.
    fn cmp(&self, other: &Self) -> Ordering {
        (other.at, other.order).cmp(&(self.at, self.order))
    }
}

impl ConditionedSocket {
    pub(crate) fn new(inner: Arc<dyn AsyncUdpSocket>, cond: LinkConditioner) -> ConditionedSocket {
        ConditionedSocket {
            inner,
            cond,
            state: Mutex::new(HeldState {
                rng: Rng::new(cond.seed),
                held: BinaryHeap::new(),
                order: 0,
                timer: None,
                scratch: vec![0; u16::MAX as usize],
            }),
        }
    }
}

impl fmt::Debug for ConditionedSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConditionedSocket")
            .field("cond", &self.cond)
            .finish_non_exhaustive()
    }
}

impl AsyncUdpSocket for ConditionedSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        self.inner.clone().create_io_poller()
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let segment = transmit
            .segment_size
            .unwrap_or(transmit.contents.len())
            .max(1);
        for packet in transmit.contents.chunks(segment) {
            if self.cond.lose(&mut state.rng) {
                continue;
            }
            let delay = self.cond.sample_delay(&mut state.rng);
            let (destination, ecn, src_ip, data) = (
                transmit.destination,
                transmit.ecn,
                transmit.src_ip,
                packet.to_vec(),
            );
            let inner = self.inner.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                let transmit = Transmit {
                    destination,
                    ecn,
                    contents: &data,
                    segment_size: None,
                    src_ip,
                };
                // A send that would block counts as one more lost packet.
                let _ = inner.try_send(&transmit);
            });
        }
        Ok(())
    }

    fn poll_recv(
        &self,
        cx: &mut Context,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let state = &mut *guard;
        // Take everything the real socket has, deciding loss and delivery time for each packet.
        loop {
            let mut one_meta = [RecvMeta::default()];
            let polled = {
                let mut one_buf = [IoSliceMut::new(&mut state.scratch)];
                self.inner.poll_recv(cx, &mut one_buf, &mut one_meta)
            };
            match polled {
                Poll::Ready(Ok(count)) => {
                    let now = Instant::now();
                    for m in one_meta.iter().take(count) {
                        let stride = m.stride.max(1);
                        let mut start = 0;
                        while start < m.len {
                            let end = (start + stride).min(m.len);
                            if !self.cond.lose(&mut state.rng) {
                                let at = now + self.cond.sample_delay(&mut state.rng);
                                state.order += 1;
                                state.held.push(Held {
                                    at,
                                    order: state.order,
                                    from: m.addr,
                                    ecn: m.ecn,
                                    dst_ip: m.dst_ip,
                                    data: state.scratch[start..end].to_vec(),
                                });
                            }
                            start = end;
                        }
                    }
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => break,
            }
        }
        // Hand out the packets that are due.
        let now = Instant::now();
        let mut filled = 0;
        while filled < bufs.len().min(meta.len()) {
            if !state.held.peek().is_some_and(|h| h.at <= now) {
                break;
            }
            let Some(packet) = state.held.pop() else {
                break;
            };
            let len = packet.data.len().min(bufs[filled].len());
            bufs[filled][..len].copy_from_slice(&packet.data[..len]);
            meta[filled] = RecvMeta {
                addr: packet.from,
                len,
                stride: len,
                ecn: packet.ecn,
                dst_ip: packet.dst_ip,
            };
            filled += 1;
        }
        if filled > 0 {
            return Poll::Ready(Ok(filled));
        }
        // Wake up when the next held packet is due.
        if let Some(next) = state.held.peek().map(|h| h.at) {
            let timer = state
                .timer
                .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(next)));
            timer.as_mut().reset(next);
            if timer.as_mut().poll(cx).is_ready() {
                cx.waker().wake_by_ref();
            }
        }
        Poll::Pending
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_stay_in_range_and_repeat() {
        let cond = LinkConditioner {
            latency: Duration::from_millis(40),
            jitter: Duration::from_millis(10),
            loss: 0.25,
            seed: 9,
        };
        let mut a = Rng::new(cond.seed);
        let mut b = Rng::new(cond.seed);
        let mut lost = 0;
        for _ in 0..1000 {
            let d = cond.sample_delay(&mut a);
            assert!(
                d >= Duration::from_millis(30) && d <= Duration::from_millis(50),
                "{d:?}"
            );
            assert_eq!(d, cond.sample_delay(&mut b));
            if cond.lose(&mut a) {
                lost += 1;
            }
            cond.lose(&mut b);
        }
        assert!((200..300).contains(&lost), "{lost}");
        let nan = LinkConditioner {
            loss: f32::NAN,
            ..cond
        };
        assert!(!nan.lose(&mut a));
    }

    #[test]
    fn absurd_values_are_clamped() {
        let cond = LinkConditioner {
            latency: Duration::MAX,
            jitter: Duration::MAX,
            loss: 1.0,
            seed: 1,
        };
        let mut rng = Rng::new(1);
        for _ in 0..100 {
            assert!(cond.sample_delay(&mut rng) <= 2 * MAX_DELAY);
        }
        assert_eq!(cond.loss_chance(), MAX_LOSS);
        assert_eq!(cond.round_trip(), 2 * MAX_DELAY);
    }
}
