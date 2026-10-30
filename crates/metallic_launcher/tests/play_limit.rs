//! Tests for the rate limit on Play links that arrive from outside the window.

use std::time::{Duration, Instant};

use metallic_launcher::PlayLimiter;

#[test]
fn a_small_burst_passes_then_links_are_dropped() {
    let mut limiter = PlayLimiter::new(2, Duration::from_secs(2));
    let t0 = Instant::now();
    assert!(limiter.allow_at(t0));
    assert!(limiter.allow_at(t0));
    assert!(!limiter.allow_at(t0));
    assert!(!limiter.allow_at(t0 + Duration::from_millis(500)));
}

#[test]
fn one_launch_comes_back_every_interval() {
    let mut limiter = PlayLimiter::new(2, Duration::from_secs(2));
    let t0 = Instant::now();
    assert!(limiter.allow_at(t0));
    assert!(limiter.allow_at(t0));
    assert!(!limiter.allow_at(t0 + Duration::from_millis(1900)));
    assert!(limiter.allow_at(t0 + Duration::from_secs(2)));
    assert!(!limiter.allow_at(t0 + Duration::from_secs(2)));
    assert!(limiter.allow_at(t0 + Duration::from_secs(4)));
}

#[test]
fn a_rejected_link_does_not_push_the_next_one_back() {
    let mut limiter = PlayLimiter::new(1, Duration::from_secs(2));
    let t0 = Instant::now();
    assert!(limiter.allow_at(t0));
    for ms in [100, 600, 1200, 1900] {
        assert!(!limiter.allow_at(t0 + Duration::from_millis(ms)));
    }
    assert!(limiter.allow_at(t0 + Duration::from_secs(2)));
}

#[test]
fn idle_time_never_builds_more_than_the_burst() {
    let mut limiter = PlayLimiter::new(3, Duration::from_secs(2));
    let t0 = Instant::now() + Duration::from_secs(1000);
    for _ in 0..3 {
        assert!(limiter.allow_at(t0));
    }
    assert!(!limiter.allow_at(t0));
}

#[test]
fn a_clock_that_steps_back_does_not_panic_or_refill() {
    let mut limiter = PlayLimiter::new(1, Duration::from_secs(2));
    let t0 = Instant::now() + Duration::from_secs(10);
    assert!(limiter.allow_at(t0));
    assert!(!limiter.allow_at(t0 - Duration::from_secs(5)));
}

#[test]
fn one_gate_covers_every_launch_path() {
    use metallic_launcher::{LaunchGate, LauncherError};
    // Player links, the bridge's play command and Studio-open links all ask the same gate.
    let gate = LaunchGate::new(PlayLimiter::new(2, Duration::from_secs(2)));
    let t0 = Instant::now();
    assert!(gate.permit_at(t0).is_ok()); // a Play link
    assert!(gate.permit_at(t0).is_ok()); // a Studio link
    assert!(matches!(gate.permit_at(t0), Err(LauncherError::Busy(_)))); // the bridge's play
    assert!(gate.permit_at(t0 + Duration::from_secs(2)).is_ok());
}
