//! A passive host-only deadline timer. The worker never owns or accesses Lua state.

use std::cell::RefCell;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Instant;

struct State {
    deadline: Option<Instant>,
    next_check: Option<Instant>,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    expired: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// One sleeping worker per VM, with a single atomic flag for the interrupt's hot path.
pub(crate) struct Watchdog {
    shared: Arc<Shared>,
    worker: RefCell<Option<JoinHandle<()>>>,
}

impl Watchdog {
    pub(crate) fn new() -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                deadline: None,
                next_check: None,
                stopped: false,
            }),
            changed: Condvar::new(),
            expired: AtomicBool::new(false),
        });
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("metallic-script-budget".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || run(state))?;
        Ok(Self {
            shared,
            worker: RefCell::new(Some(worker)),
        })
    }

    /// A new outer budget replaces the previous generation under the worker's mutex.
    pub(crate) fn arm(&self, deadline: Instant) -> bool {
        let mut state = self.shared.lock();
        if state.stopped {
            return false;
        }
        state.deadline = Some(deadline);
        self.shared
            .expired
            .store(Instant::now() >= deadline, Ordering::Release);
        // A pending earlier wake-up can check a later budget too. Keeping that wake-up on
        // disarm avoids waking the worker for every short callback in a busy frame.
        let wake = state.next_check.is_none_or(|next| deadline < next);
        if wake {
            state.next_check = Some(deadline);
        }
        drop(state);
        if wake {
            self.shared.changed.notify_one();
        }
        true
    }

    pub(crate) fn disarm(&self) {
        let mut state = self.shared.lock();
        state.deadline = None;
        if !state.stopped {
            self.shared.expired.store(false, Ordering::Release);
        }
    }

    pub(crate) fn expired(&self) -> bool {
        self.shared.expired.load(Ordering::Acquire)
    }

    /// Sparse clock checks latch expiration too, so a caught timeout never grants more execution.
    pub(crate) fn expire(&self) {
        self.shared.expired.store(true, Ordering::Release);
    }

    pub(crate) fn stop(&self) {
        {
            let mut state = self.shared.lock();
            state.stopped = true;
            state.deadline = None;
            self.shared.expired.store(true, Ordering::Release);
        }
        self.shared.changed.notify_one();
        // The worker needs the state mutex while exiting: never retain it across join.
        if let Some(worker) = self.worker.borrow_mut().take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(shared: Arc<Shared>) {
    let mut state = shared.lock();
    loop {
        if state.stopped {
            return;
        }
        let now = Instant::now();
        if state.deadline.is_some_and(|deadline| now >= deadline) {
            shared.expired.store(true, Ordering::Release);
            state.next_check = None;
        } else if state.next_check.is_some_and(|next| now >= next) {
            state.next_check = state.deadline;
        }
        state = match state.next_check {
            Some(next) => match shared
                .changed
                .wait_timeout(state, next.saturating_duration_since(now))
            {
                Ok((state, _)) => state,
                Err(poisoned) => poisoned.into_inner().0,
            },
            None => match shared.changed.wait(state) {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            },
        };
        // Re-read the current generation after every wake. A timeout for an old deadline
        // must not expire a newly armed budget, even when the wake was already pending.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn old_wake_does_not_expire_a_later_budget() {
        let watchdog = Watchdog::new().unwrap();
        assert!(watchdog.arm(Instant::now() + Duration::from_millis(10)));
        watchdog.disarm();
        assert!(watchdog.arm(Instant::now() + Duration::from_secs(30)));
        thread::sleep(Duration::from_millis(30));
        assert!(!watchdog.expired());
        watchdog.stop();
    }

    #[test]
    fn expiration_is_latched_until_the_next_outer_budget() {
        let watchdog = Watchdog::new().unwrap();
        assert!(watchdog.arm(Instant::now()));
        assert!(watchdog.expired());
        watchdog.disarm();
        assert!(!watchdog.expired());
        assert!(watchdog.arm(Instant::now() + Duration::from_secs(30)));
        assert!(!watchdog.expired());
    }

    #[test]
    fn stop_joins_a_waiting_worker_without_waiting_for_its_deadline() {
        let watchdog = Watchdog::new().unwrap();
        assert!(watchdog.arm(Instant::now() + Duration::from_secs(60)));
        let started = Instant::now();
        watchdog.stop();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!watchdog.arm(Instant::now() + Duration::from_secs(30)));
        watchdog.stop();
    }
}
