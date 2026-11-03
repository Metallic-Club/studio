//! The task scheduler's state: frame phases, the sleeper heap, the deferred queue and the signal waiters.
//!
//! [`ScriptHost`](crate::ScriptHost) drives it (the crate docs give the rules); this module only keeps the queues
//! and their bookkeeping, so the ordering rules live in one small place.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::time::Duration;

use metallic_core::{ConnectionId, Id, ListenerKey, ScriptValue};

use crate::{Deadline, ThreadRef};

/// How many deferred resumes deep `task.defer` may go within one resumption point, as in Roblox. Deferred work
/// that defers more work is fine; a chain that would go deeper errors the thread calling `task.defer`.
pub const MAX_DEFER_DEPTH: u32 = 80;

/// The time one frame phase may spend resuming threads, unless [`ScriptHost::set_phase_budget`] changes it. Half
/// a 60 Hz frame: past it, the rest of the phase's due work waits for the next frame.
///
/// [`ScriptHost::set_phase_budget`]: crate::ScriptHost::set_phase_budget
pub const DEFAULT_PHASE_BUDGET: Duration = Duration::from_millis(8);

/// The output name of the scheduler's own warnings.
pub(crate) const SCHEDULER_SOURCE: &str = "TaskScheduler";

/// A frame phase: an entry point of the engine loop into [`ScriptHost::step`](crate::ScriptHost::step). The
/// engine calls each once per frame, in the order of [`Phase::FRAME`]. RunService's events of the same names come
/// with the `RunService` class; the engine fires them on the data model just before the matching step, so their
/// listeners run in that step.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Phase {
    /// Before rendering (clients only; a server never calls it).
    RenderStepped,
    /// Before the physics step.
    PreSimulation,
    /// After the physics step.
    PostSimulation,
    /// The end of the frame. Due `task.wait` and `task.delay` threads resume here.
    Heartbeat,
}

impl Phase {
    /// Every phase, in the order the engine calls them within a frame.
    pub const FRAME: [Self; 4] = [
        Self::RenderStepped,
        Self::PreSimulation,
        Self::PostSimulation,
        Self::Heartbeat,
    ];

    /// The phase's name, as RunService's event calls it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::RenderStepped => "RenderStepped",
            Self::PreSimulation => "PreSimulation",
            Self::PostSimulation => "PostSimulation",
            Self::Heartbeat => "Heartbeat",
        }
    }
}

/// What one [`ScriptHost::step`](crate::ScriptHost::step) did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StepStats {
    /// How many threads it resumed: sleepers, deferred threads, signal waiters and listener threads (not the
    /// threads they `task.spawn` nested).
    pub resumed: usize,
    /// Whether the phase budget ran out, so due work carried over.
    pub carried_over: bool,
}

/// What a live thread is scheduled for. Queue entries name the ticket they were pushed with; an entry whose
/// thread's slot no longer matches it is stale and skipped, so cancelling never searches a queue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Slot {
    /// Not scheduled: running, `Created`, or suspended for the backend's own reasons.
    Idle,
    /// In the sleeper heap with this ticket.
    Sleeping(u64),
    /// In the deferred queue with this ticket.
    Deferred(u64),
    /// Waiting on a signal through this host-internal listener.
    Signal(ListenerKey),
    /// Waiting in `require` for this module to finish loading.
    Require(Id),
    /// Waiting in an `Invoke` for the callback thread of this ticket to return.
    Invoke(u64),
    /// Waiting in an `Invoke`, with this ticket, for a function to be assigned to the callback.
    Callback(u64),
}

/// What a scheduled thread is resumed with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Wake {
    /// No arguments.
    Nothing,
    /// The game time since `since`, never less than `at_least` (`task.wait`).
    Elapsed {
        /// When it started waiting.
        since: f64,
        /// How long it asked to wait.
        at_least: f64,
    },
    /// These arguments (`task.defer` and `task.delay`).
    Args(Vec<ScriptValue>),
}

/// A sleeping thread. The heap pops the earliest wake time first and, among equal times, the earliest sleeper.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sleeper {
    pub(crate) wake: f64,
    pub(crate) seq: u64,
    pub(crate) thread: ThreadRef,
}

impl Ord for Sleeper {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed: `BinaryHeap` is a max-heap.
        other.wake.total_cmp(&self.wake).then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Sleeper {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Sleeper {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Sleeper {}

/// A deferred thread, and how many deferred resumes deep it will run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Deferred {
    pub(crate) seq: u64,
    pub(crate) thread: ThreadRef,
    pub(crate) depth: u32,
}

/// A thread waiting on a signal (`signal:Wait()`), through the host-internal connection.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Waiter {
    pub(crate) thread: ThreadRef,
    pub(crate) connection: ConnectionId,
}

/// The scheduler's queues and clocks.
#[derive(Debug)]
pub(crate) struct Scheduler {
    /// Game time as of the last step; it never goes back.
    pub(crate) now: f64,
    /// The next ticket. Tickets order sleepers with equal wake times and tell stale queue entries apart.
    next_seq: u64,
    pub(crate) sleepers: BinaryHeap<Sleeper>,
    /// Sleepers whose entry is current (the heap also holds stale ones until they surface).
    pub(crate) live_sleepers: usize,
    pub(crate) deferred: VecDeque<Deferred>,
    /// Deferred entries that are current.
    pub(crate) live_deferred: usize,
    /// Host-internal listeners of `signal:Wait()`.
    pub(crate) waiters: HashMap<ListenerKey, Waiter>,
    /// How many deferred resumes deep the running code is.
    pub(crate) depth: u32,
    /// Heartbeats so far.
    pub(crate) frame: u64,
    /// The frame of the last budget warning.
    pub(crate) warned_frame: Option<u64>,
    pub(crate) phase_budget: Duration,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            now: 0.0,
            next_seq: 0,
            sleepers: BinaryHeap::new(),
            live_sleepers: 0,
            deferred: VecDeque::new(),
            live_deferred: 0,
            waiters: HashMap::new(),
            depth: 0,
            frame: 0,
            warned_frame: None,
            phase_budget: DEFAULT_PHASE_BUDGET,
        }
    }
}

impl Scheduler {
    /// A new ticket.
    pub(crate) fn ticket(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    /// The ticket the next schedule will get. Work scheduled from now on has a ticket at least this.
    pub(crate) fn peek_ticket(&self) -> u64 {
        self.next_seq
    }

    /// Moves game time to `now`. Time never goes back, and NaN is ignored.
    pub(crate) fn advance(&mut self, now: f64) {
        if now > self.now {
            self.now = now;
        }
    }

    /// Whether the heap holds so many stale entries that it is worth rebuilding.
    pub(crate) fn sleepers_need_compacting(&self) -> bool {
        self.sleepers.len() > 2 * self.live_sleepers + 64
    }

    /// Whether the deferred queue holds so many stale entries that it is worth compacting.
    pub(crate) fn deferred_need_compacting(&self) -> bool {
        self.deferred.len() > 2 * self.live_deferred + 64
    }
}

/// A sleep length as the scheduler uses it: zero for negative and NaN.
pub(crate) fn sanitize_seconds(seconds: f64) -> f64 {
    if seconds > 0.0 { seconds } else { 0.0 }
}

/// The time budget of one step. It always lets the first resume of the step through, so a tiny budget still makes
/// progress, and checks the clock before each later one.
#[derive(Debug)]
pub(crate) struct Budget {
    deadline: Deadline,
    started: bool,
    pub(crate) exhausted: bool,
}

impl Budget {
    /// A budget of `limit` from now.
    pub(crate) fn new(limit: Duration) -> Self {
        Self {
            deadline: Deadline::after(limit),
            started: false,
            exhausted: false,
        }
    }

    /// A budget that never runs out.
    pub(crate) fn unlimited() -> Self {
        Self {
            deadline: Deadline::unlimited(),
            started: false,
            exhausted: false,
        }
    }

    /// Whether one more resume fits. Once it says no, it keeps saying no.
    pub(crate) fn next(&mut self) -> bool {
        if self.exhausted {
            return false;
        }
        if self.started && self.deadline.check_now() {
            self.exhausted = true;
            return false;
        }
        self.started = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LanguageId, ThreadId};

    fn sleeper(wake: f64, seq: u64) -> Sleeper {
        Sleeper {
            wake,
            seq,
            thread: ThreadRef {
                language: LanguageId(0),
                thread: ThreadId(seq),
            },
        }
    }

    #[test]
    fn the_heap_pops_by_wake_time_then_sleep_order() {
        let mut heap = BinaryHeap::new();
        for (wake, seq) in [(2.0, 0), (1.0, 1), (1.0, 2), (0.5, 3), (2.0, 4), (1.0, 5)] {
            heap.push(sleeper(wake, seq));
        }
        let order: Vec<u64> = std::iter::from_fn(|| heap.pop()).map(|s| s.seq).collect();
        assert_eq!(order, vec![3, 1, 2, 5, 0, 4]);
    }

    #[test]
    fn a_budget_always_lets_one_resume_through() {
        let mut budget = Budget::new(Duration::ZERO);
        assert!(budget.next());
        assert!(!budget.next());
        assert!(budget.exhausted);
        assert!(!budget.next());
        let mut budget = Budget::unlimited();
        assert!((0..1000).all(|_| budget.next()));
    }

    #[test]
    fn time_never_goes_back() {
        let mut scheduler = Scheduler::default();
        scheduler.advance(2.0);
        scheduler.advance(1.0);
        scheduler.advance(f64::NAN);
        assert_eq!(scheduler.now, 2.0);
        assert_eq!(sanitize_seconds(-1.0), 0.0);
        assert_eq!(sanitize_seconds(f64::NAN), 0.0);
        assert_eq!(sanitize_seconds(0.25), 0.25);
    }
}
