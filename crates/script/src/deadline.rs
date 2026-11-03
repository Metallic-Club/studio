//! `Deadline`: the time budget of one call into a backend, checked cheaply.

use std::time::{Duration, Instant};

/// How many [`Deadline::poll`] calls share one clock read. Reading the clock at every safepoint made hot loops
/// 2.4–3.1× slower in the archived engine; one read per 256 polls is noise, and a safepoint every few
/// instructions still notices an overrun within microseconds.
const POLLS_PER_CLOCK_READ: u32 = 256;

/// The moment a call into a backend must stop by. The host sets a fresh one in [`HostContext::deadline`] before
/// every call; the backend polls it at its safepoints.
///
/// [`HostContext::deadline`]: crate::HostContext::deadline
#[derive(Clone, Debug)]
pub struct Deadline {
    /// When the call must stop, or `None` for no limit.
    at: Option<Instant>,
    /// Polls left before the next clock read.
    countdown: u32,
    /// Whether the deadline was seen to have passed. Once set it stays set.
    expired: bool,
}

impl Deadline {
    /// A deadline that never passes.
    pub fn unlimited() -> Self {
        Self::at(None)
    }

    /// A deadline `budget` from now. A budget too large to represent is no limit.
    pub fn after(budget: Duration) -> Self {
        Self::at(Instant::now().checked_add(budget))
    }

    /// A deadline at `at`, or no limit for `None`.
    pub fn at(at: Option<Instant>) -> Self {
        Self {
            at,
            countdown: POLLS_PER_CLOCK_READ,
            expired: false,
        }
    }

    /// When the call must stop, or `None` for no limit. Backends whose VM interrupt cannot reach the
    /// [`HostContext`](crate::HostContext) copy this into their own state at the start of a call.
    pub fn instant(&self) -> Option<Instant> {
        self.at
    }

    /// Whether the deadline has passed, reading the clock only once every few hundred polls. Call it at every
    /// safepoint; it is a decrement and a branch the rest of the time.
    #[inline]
    pub fn poll(&mut self) -> bool {
        if self.expired {
            return true;
        }
        self.countdown -= 1;
        if self.countdown > 0 {
            return false;
        }
        self.countdown = POLLS_PER_CLOCK_READ;
        self.check_now()
    }

    /// Whether the deadline has passed, reading the clock now. For a backend that checks rarely (once per
    /// instruction batch, from a watchdog...), where the counter in [`Deadline::poll`] is not needed.
    pub fn check_now(&mut self) -> bool {
        if !self.expired {
            self.expired = self.at.is_some_and(|at| Instant::now() >= at);
        }
        self.expired
    }

    /// Whether a previous poll or check saw the deadline pass. Reads no clock.
    pub fn is_expired(&self) -> bool {
        self.expired
    }
}

impl Default for Deadline {
    /// No limit.
    fn default() -> Self {
        Self::unlimited()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_expires() {
        let mut deadline = Deadline::unlimited();
        for _ in 0..10 * POLLS_PER_CLOCK_READ {
            assert!(!deadline.poll());
        }
        assert!(!deadline.check_now());
        assert_eq!(deadline.instant(), None);
    }

    #[test]
    fn a_past_deadline_is_seen_within_one_clock_interval() {
        let mut deadline = Deadline::at(Some(Instant::now()));
        let polls = (1..=POLLS_PER_CLOCK_READ).find(|_| deadline.poll());
        assert_eq!(polls, Some(POLLS_PER_CLOCK_READ));
        assert!(deadline.is_expired());
        // It stays expired without reading the clock again.
        assert!(deadline.poll());
    }

    #[test]
    fn a_future_deadline_does_not_expire_early() {
        let mut deadline = Deadline::after(Duration::from_secs(3600));
        for _ in 0..4 * POLLS_PER_CLOCK_READ {
            assert!(!deadline.poll());
        }
        assert!(!deadline.check_now());
        assert!(Deadline::after(Duration::MAX).instant().is_none());
    }
}
