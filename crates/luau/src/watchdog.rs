//! The time budget without a per-safepoint cost: a watchdog thread installs Luau's interrupt only once a call
//! has overrun its deadline.
//!
//! # Why
//!
//! Luau checks `lua_callbacks(L)->interrupt` at every safepoint (loop back-edge, call, return). With no callback
//! the check is a load, a compare and a branch that is never taken: in the interpreter (`VM_INTERRUPT` in
//! `VM/src/lvmexecute.cpp`) and in native code (`IrCmd::INTERRUPT`, lowered in `CodeGen/src/IrLoweringX64.cpp` and
//! `IrLoweringA64.cpp`). With a callback, every safepoint calls it. A call costs a few nanoseconds, which was 14% of
//! an interpreted loop iteration and doubled a native one (#64). So, while a call is within its budget, no
//! interrupt is installed at all.
//!
//! # How
//!
//! Each call into the backend ([`FrameGuard`](crate::ffi::FrameGuard)) arms the watchdog with its deadline and
//! disarms it when it ends. One thread per process sleeps until the earliest armed deadline; when one passes it
//! installs the interrupt in that VM ([`InterruptSlot::install`]). The interrupt then sees the passed
//! [`Deadline`] and stops the thread, exactly as when it was installed all the time. Disarming removes it again.
//!
//! Luau allows this: `lua.h` documents that "interrupt is safe to set from an arbitrary thread" (on
//! `lua_Callbacks`), and native code expects it, re-reading the callback in its interrupt handler because "it may
//! be nullptr in case the update raced with the check" (`emitInterrupt` in `CodeGen/src/EmitCommonX64.cpp` and
//! `CodeGenA64.cpp`). [`InterruptSlot`] has the memory-level argument.
//!
//! The watchdog writes into a VM only while a call into it is armed, and the backend, which owns the VM, is
//! borrowed by that call for the whole time, so the VM cannot be freed under the write. Arming, disarming and the
//! write all happen under one lock, so once disarming returns the watchdog cannot write into that VM any more.
//!
//! # What it costs
//!
//! Arming and disarming each lock an uncontended mutex, and arming reads the clock once; neither makes a system
//! call in the common case. The thread does not wake per call: it sleeps until the earliest armed deadline, and
//! arming only wakes it when the new deadline is earlier than the time it will wake anyway. While calls keep coming
//! it wakes about once per budget; when none came since it last woke, it sleeps until the next one.
//!
//! An overrun is seen once the thread wakes, so the precision is the operating system's timer (about a
//! millisecond on Linux, up to about 16 ms on Windows) rather than microseconds. The budget is a backstop against
//! runaway loops (a second by default), so that is plenty.
//!
//! # Without a thread
//!
//! Where no thread can be started, [`Budget`] installs the interrupt for good instead, and every safepoint polls
//! the deadline as it always did.

use std::cell::Cell;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use metallic_script::Deadline;

use crate::ffi::InterruptSlot;

/// The shortest the watchdog lingers after a call, so a budget of zero cannot make it spin.
const MIN_LINGER: Duration = Duration::from_millis(1);

/// One armed call.
struct Entry {
    /// Tells this arming apart from others of the same VM (calls can nest).
    id: u64,
    /// The call's deadline.
    at: Instant,
    /// The VM to interrupt.
    slot: InterruptSlot,
    /// Whether the watchdog has installed the interrupt for this call.
    fired: bool,
}

/// What the watchdog and the calls share, under [`Watchdog::state`].
struct State {
    /// Every armed call, in no order (there are rarely more than a few).
    armed: Vec<Entry>,
    /// The next [`Entry::id`].
    next_id: u64,
    /// When the thread will wake by itself, or `None` while it sleeps until woken.
    wake_at: Option<Instant>,
    /// How long the thread sleeps when nothing is armed but calls came since it last woke: the last budget.
    linger: Duration,
    /// Whether a call was armed since the thread last woke.
    armed_since_wake: bool,
}

/// The process's watchdog: the armed calls and the condition its thread sleeps on.
pub(crate) struct Watchdog {
    state: Mutex<State>,
    wake: Condvar,
}

/// The one watchdog. Its thread starts on first use.
static WATCHDOG: Watchdog = Watchdog {
    state: Mutex::new(State {
        armed: Vec::new(),
        next_id: 0,
        wake_at: None,
        linger: MIN_LINGER,
        armed_since_wake: false,
    }),
    wake: Condvar::new(),
};

/// Whether the watchdog's thread is running, decided once.
static STARTED: OnceLock<bool> = OnceLock::new();

/// The watchdog, starting its thread the first time. `None` where no thread can be started.
fn watchdog() -> Option<&'static Watchdog> {
    let started = *STARTED.get_or_init(|| {
        thread::Builder::new()
            .name("luau-watchdog".to_owned())
            .spawn(|| WATCHDOG.run())
            .is_ok()
    });
    started.then_some(&WATCHDOG)
}

impl Watchdog {
    /// The shared state. Nothing panics while holding it, but a poisoned lock is still usable: the state is
    /// consistent after every statement.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The thread: installs the interrupt for every armed call past its deadline, then sleeps until the next
    /// deadline.
    fn run(&self) {
        let mut state = self.lock();
        loop {
            let now = Instant::now();
            for entry in state.armed.iter_mut().filter(|entry| !entry.fired && entry.at <= now) {
                entry.slot.install();
                entry.fired = true;
            }
            let next = state
                .armed
                .iter()
                .filter(|entry| !entry.fired)
                .map(|entry| entry.at)
                .min();
            let sleep = match next {
                Some(at) => Some(at.saturating_duration_since(now)),
                None if state.armed_since_wake => Some(state.linger),
                None => None,
            };
            state.armed_since_wake = false;
            state.wake_at = sleep.and_then(|sleep| now.checked_add(sleep));
            // Waking early or spuriously is harmless: the loop checks the deadlines again.
            state = match sleep {
                Some(sleep) => {
                    self.wake
                        .wait_timeout(state, sleep)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                None => self.wake.wait(state).unwrap_or_else(PoisonError::into_inner),
            };
        }
    }

    /// Arms the watchdog for a call into the VM of `slot` that must end by `at`, until the returned guard drops.
    fn arm(&'static self, slot: InterruptSlot, at: Instant) -> Armed {
        let now = Instant::now();
        let mut state = self.lock();
        let id = state.next_id;
        state.next_id += 1;
        state.linger = at.saturating_duration_since(now).max(MIN_LINGER);
        state.armed_since_wake = true;
        let fired = at <= now;
        if fired {
            slot.install();
        } else if state.wake_at.is_none_or(|wake_at| at < wake_at) {
            state.wake_at = Some(at);
            self.wake.notify_one();
        }
        state.armed.push(Entry { id, at, slot, fired });
        Armed { watchdog: self, id }
    }
}

/// A call the watchdog watches. Dropping it disarms the watchdog and removes the interrupt if it was installed.
pub(crate) struct Armed {
    watchdog: &'static Watchdog,
    id: u64,
}

impl Drop for Armed {
    fn drop(&mut self) {
        let mut state = self.watchdog.lock();
        let Some(index) = state.armed.iter().position(|entry| entry.id == self.id) else {
            return;
        };
        let entry = state.armed.swap_remove(index);
        // A call nested in this one (same VM) that has overrun too keeps its interrupt.
        if entry.fired && !state.armed.iter().any(|other| other.fired && other.slot == entry.slot) {
            entry.slot.remove();
        }
    }
}

/// One backend's time budget: whether it is enforced, and how.
pub(crate) struct Budget {
    /// The VM's interrupt.
    slot: InterruptSlot,
    /// The watchdog, or `None` when the interrupt is installed for good instead.
    watchdog: Option<&'static Watchdog>,
    /// Whether the budget is enforced (see [`crate::LuauLanguage::set_time_budget_enabled`]).
    enabled: Cell<bool>,
}

impl Budget {
    /// The budget of the VM whose interrupt is `slot`, enforced.
    pub(crate) fn new(slot: InterruptSlot) -> Self {
        let watchdog = watchdog();
        if watchdog.is_none() {
            slot.install();
        }
        Self {
            slot,
            watchdog,
            enabled: Cell::new(true),
        }
    }

    /// Turns enforcing on or off. Only between calls.
    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
        if self.watchdog.is_none() {
            if enabled {
                self.slot.install();
            } else {
                self.slot.remove();
            }
        }
    }

    /// Arms the watchdog for a call that must end by `deadline`, until the returned guard drops. `None` when there
    /// is nothing to watch: no limit, enforcing off, or no watchdog (the interrupt is installed already).
    pub(crate) fn arm(&self, deadline: &Deadline) -> Option<Armed> {
        let watchdog = self.watchdog?;
        if !self.enabled.get() {
            return None;
        }
        Some(watchdog.arm(self.slot, deadline.instant()?))
    }

    /// Whether a watchdog thread runs (otherwise the interrupt is installed for good).
    #[cfg(test)]
    pub(crate) fn has_watchdog(&self) -> bool {
        self.watchdog.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use metallic_core::ScriptValue;
    use metallic_script::{Resume, ScriptFailure, ScriptHost};

    use super::*;
    use crate::LuauLanguage;

    /// Rounds per stress thread.
    const ROUNDS: u32 = 500;

    /// A small deterministic generator (xorshift64), so a failing seed can be replayed.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// Many resumes against budgets from 0 to 2 ms, so the watchdog fires at random moments: before the script
    /// starts, in its loop, while it returns, after it finished. Every run must finish with the right value or time
    /// out at a line of its loop, and the interrupt must be gone after every call. Several VMs run at once, so arms,
    /// disarms and the thread's writes interleave.
    fn stress(seed: u64) {
        let mut dm = metallic_classes::new_game().unwrap_or_else(|e| panic!("{e}"));
        let luau = LuauLanguage::new(dm.registry().clone()).expect("Luau VM");
        let slot = luau.shared.budget.slot;
        let watched = luau.shared.budget.has_watchdog();
        let mut host = ScriptHost::new();
        host.register_language(Box::new(luau)).expect("register");
        let registry = dm.registry().clone();
        let script_class = registry.class_id("Script").expect("Script");
        let workspace = registry
            .class_id("Workspace")
            .and_then(|class| dm.service(class))
            .expect("Workspace");
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (mut finished, mut loops_stopped, mut runaways_stopped) = (0, 0, 0);
        for round in 0..ROUNDS {
            let budget = Duration::from_micros(rng.next() % 2000);
            host.set_resume_budget(budget);
            let runaway = rng.next().is_multiple_of(4);
            // From one iteration to 16 million (tens of milliseconds even in native code), so loops end before,
            // around and long after the watchdog fires.
            let iterations = 1u64 << (rng.next() % 25);
            let source = if runaway {
                "local n = 0\nwhile true do\n    n += 1\nend\n".to_owned()
            } else {
                format!("local n = 0\nfor i = 1, {iterations} do\n    n += i % 3\nend\nreturn n\n")
            };
            let script = dm.create(script_class).expect("script");
            dm.set_parent(script, Some(workspace)).expect("parent");
            let started = Instant::now();
            let run = host
                .run_script(&mut dm, script, "Stress.luau", &source)
                .unwrap_or_else(|e| panic!("seed {seed} round {round}: {e}"));
            let took = started.elapsed();
            match run.resume {
                Resume::Finished(values) if !runaway => {
                    // Each full 1, 2, 0 of `i % 3` adds 3; a remainder of 1 or 2 adds 1 or 1 + 2.
                    let expected = (iterations / 3 * 3 + [0, 1, 3][(iterations % 3) as usize]) as f64;
                    assert!(
                        matches!(values.as_slice(), [ScriptValue::Number(n)] if *n == expected),
                        "seed {seed} round {round}: {values:?}, expected {expected}"
                    );
                    finished += 1;
                }
                // The loop's lines; 5 is the `return`, also a safepoint.
                Resume::Errored(ScriptFailure::Timeout { line: Some(2..=5) }) if !runaway => loops_stopped += 1,
                Resume::Errored(ScriptFailure::Timeout { line: Some(2..=4) }) => runaways_stopped += 1,
                other => panic!("seed {seed} round {round} (runaway: {runaway}): {other:?}"),
            }
            // An overrun is seen within the OS timer's resolution; a lost wake-up would show as seconds.
            assert!(
                took < budget + Duration::from_secs(1),
                "seed {seed} round {round}: {took:?} for a budget of {budget:?}"
            );
            if watched {
                assert!(
                    !slot.is_installed(),
                    "seed {seed} round {round}: the interrupt outlived its call"
                );
            }
            dm.destroy(script).expect("destroy");
        }
        assert!(
            finished > 0 && loops_stopped > 0 && runaways_stopped > 0,
            "seed {seed}: {finished} finished, {loops_stopped} loops and {runaways_stopped} runaways stopped"
        );
        // The VM is still fine, and nothing is left armed that could stop a script within budget.
        host.set_resume_budget(Duration::from_secs(60));
        let script = dm.create(script_class).expect("script");
        dm.set_parent(script, Some(workspace)).expect("parent");
        let run = host
            .run_script(
                &mut dm,
                script,
                "After.luau",
                "local n = 0\nfor i = 1, 1e6 do n += 1 end\nreturn n",
            )
            .expect("compiles");
        assert!(
            matches!(&run.resume, Resume::Finished(values) if values.as_slice() == [ScriptValue::Number(1e6)]),
            "{:?}",
            run.resume
        );
    }

    #[test]
    fn the_watchdog_stops_runaway_scripts_at_random_moments_and_always_cleans_up() {
        let threads: Vec<_> = (1..=4).map(|seed| thread::spawn(move || stress(seed))).collect();
        for thread in threads {
            thread.join().expect("a stress thread failed");
        }
    }

    #[test]
    fn a_short_deadline_wakes_the_watchdog_early() {
        let Some(watchdog) = watchdog() else {
            return;
        };
        let (long_lua, short_lua) = (mlua::Lua::new(), mlua::Lua::new());
        let long_slot = InterruptSlot::of(&long_lua).expect("slot");
        let short_slot = InterruptSlot::of(&short_lua).expect("slot");
        let now = Instant::now();
        let long = watchdog.arm(long_slot, now + Duration::from_secs(60));
        let short = watchdog.arm(short_slot, now + Duration::from_millis(5));
        let fired = (0..2000).any(|_| {
            thread::sleep(Duration::from_millis(1));
            short_slot.is_installed()
        });
        assert!(fired, "the watchdog slept past a deadline armed after a later one");
        assert!(!long_slot.is_installed());
        drop(short);
        assert!(!short_slot.is_installed(), "disarming removes the interrupt");
        drop(long);
        assert!(!long_slot.is_installed());
    }

    #[test]
    fn a_deadline_already_past_fires_at_once_and_unlimited_never_arms() {
        let lua = mlua::Lua::new();
        let slot = InterruptSlot::of(&lua).expect("slot");
        let budget = Budget::new(slot);
        if !budget.has_watchdog() {
            return;
        }
        assert!(budget.arm(&Deadline::unlimited()).is_none());
        let armed = budget.arm(&Deadline::at(Some(Instant::now())));
        assert!(slot.is_installed());
        drop(armed);
        assert!(!slot.is_installed());
        budget.set_enabled(false);
        assert!(budget.arm(&Deadline::at(Some(Instant::now()))).is_none());
        assert!(!slot.is_installed());
    }
}
