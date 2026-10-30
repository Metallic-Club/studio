//! A rate limit for Play links that arrive from outside the window (the single-instance pipe,
//! the protocol handler, a click in the page), so a page that sends many `metallic://play` links
//! cannot start many Players.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::error::LauncherError;

/// A token bucket: `burst` launches at once, then one more every `interval`.
#[derive(Debug)]
pub struct PlayLimiter {
    burst: u32,
    interval: Duration,
    tokens: f64,
    last: Option<Instant>,
}

impl PlayLimiter {
    /// The Launcher's setting: a burst of 3, then one launch per 2 seconds.
    pub fn standard() -> Self {
        Self::new(3, Duration::from_secs(2))
    }

    /// A limiter that allows `burst` launches (at least 1) and then one per `interval`.
    pub fn new(burst: u32, interval: Duration) -> Self {
        let burst = burst.max(1);
        PlayLimiter {
            burst,
            interval,
            tokens: f64::from(burst),
            last: None,
        }
    }

    /// Takes a launch now if one is left.
    pub fn allow(&mut self) -> bool {
        self.allow_at(Instant::now())
    }

    /// Like [`allow`](Self::allow) with the time given (for tests). An earlier time than the
    /// last one adds nothing.
    pub fn allow_at(&mut self, now: Instant) -> bool {
        if let Some(last) = self.last {
            let gained = now.saturating_duration_since(last).as_secs_f64()
                / self.interval.as_secs_f64().max(f64::MIN_POSITIVE);
            self.tokens = (self.tokens + gained).min(f64::from(self.burst));
        }
        if self.last.is_none_or(|last| now > last) {
            self.last = Some(now);
        }
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// One limiter shared by every way of starting a program: Play links (pipe, protocol handler,
/// page clicks, first-start argument), the bridge's `play` and Studio-open links, and the tray.
#[derive(Debug)]
pub struct LaunchGate(Mutex<PlayLimiter>);

impl LaunchGate {
    /// A gate that uses `limiter`.
    pub fn new(limiter: PlayLimiter) -> Self {
        LaunchGate(Mutex::new(limiter))
    }

    /// Ok when a launch may go ahead now; `Busy` otherwise.
    pub fn permit(&self) -> Result<(), LauncherError> {
        self.permit_at(Instant::now())
    }

    /// Like [`permit`](Self::permit) with the time given (for tests).
    pub fn permit_at(&self, now: Instant) -> Result<(), LauncherError> {
        let mut limiter = self
            .0
            .lock()
            .map_err(|_| LauncherError::Busy("launching"))?;
        if limiter.allow_at(now) {
            Ok(())
        } else {
            Err(LauncherError::Busy("too many launches in a short time"))
        }
    }
}
