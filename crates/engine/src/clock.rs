//! The game clock: game time, and the fixed-step accumulator that decides how many simulation steps a frame runs.
//!
//! Everything is kept in whole nanoseconds ([`Duration`]), not `f64` seconds, so a frame of exactly one fixed step
//! always runs exactly one step: summing `1.0 / 60.0` sixty times in floating point need not reach `1.0`, and the
//! accumulator would then run zero steps one frame and two the next.

use std::time::Duration;

/// What one frame of the clock gives the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Advance {
    /// How far game time moved: the real time step, clamped to the longest frame.
    pub(crate) dt: Duration,
    /// How many fixed simulation steps to run.
    pub(crate) steps: u32,
    /// Steps that were due but skipped because the frame hit the catch-up cap.
    pub(crate) dropped: u32,
}

/// Game time and the simulation accumulator.
#[derive(Clone, Debug)]
pub(crate) struct Clock {
    /// One simulation step.
    fixed: Duration,
    /// The most steps one frame may run.
    max_catch_up: u32,
    /// The longest real time step a frame takes into account.
    max_frame: Duration,
    /// Game time since the engine was made: the sum of every frame's clamped time step.
    game: Duration,
    /// Simulation time owed: always less than one step between frames.
    acc: Duration,
}

impl Clock {
    /// A clock at game time zero that steps `tick_rate` times per second.
    ///
    /// The step is rounded **up** to whole nanoseconds (60 Hz is 16,666,667 ns), so `tick_rate` steps always cover
    /// at least a second and a `task.wait(1)` started at a step's time wakes after `tick_rate` steps, not one more.
    pub(crate) fn new(tick_rate: u32, max_catch_up: u32, max_frame: Duration) -> Self {
        let fixed = Duration::from_nanos(1_000_000_000u64.div_ceil(u64::from(tick_rate.max(1))));
        Self {
            fixed,
            max_catch_up,
            max_frame,
            game: Duration::ZERO,
            acc: Duration::ZERO,
        }
    }

    /// One simulation step.
    pub(crate) fn fixed_step(&self) -> Duration {
        self.fixed
    }

    /// Game time since the engine was made.
    pub(crate) fn game_time(&self) -> Duration {
        self.game
    }

    /// How far into the next simulation step game time is, from 0 (on a step) to just under 1: what a renderer
    /// interpolates by.
    pub(crate) fn alpha(&self) -> f64 {
        self.acc.as_secs_f64() / self.fixed.as_secs_f64()
    }

    /// Moves game time by `real` (clamped to the longest frame) and takes the whole steps now owed, at most the
    /// catch-up cap. Steps over the cap are dropped, not carried: carrying them is how a slow frame turns into a
    /// slower one (the spiral of death).
    pub(crate) fn advance(&mut self, real: Duration) -> Advance {
        let dt = real.min(self.max_frame);
        self.game += dt;
        self.acc += dt;
        let due = self.acc.as_nanos() / self.fixed.as_nanos();
        let due = u32::try_from(due).unwrap_or(u32::MAX);
        let steps = due.min(self.max_catch_up);
        // `due` steps fit in `acc`, so this never underflows; what remains is less than one step.
        self.acc = self.acc.saturating_sub(self.fixed.saturating_mul(due));
        Advance {
            dt,
            steps,
            dropped: due - steps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> Clock {
        Clock::new(60, 8, Duration::from_millis(250))
    }

    #[test]
    fn sixty_hz_rounds_the_step_up() {
        let clock = clock();
        assert_eq!(clock.fixed_step(), Duration::from_nanos(16_666_667));
        assert!(clock.fixed_step() * 60 >= Duration::from_secs(1));
    }

    #[test]
    fn a_frame_of_one_step_runs_one_step_every_time() {
        let mut clock = clock();
        let step = clock.fixed_step();
        for _ in 0..10_000 {
            let advance = clock.advance(step);
            assert_eq!((advance.steps, advance.dropped), (1, 0));
        }
        assert_eq!(clock.game_time(), step * 10_000);
        assert_eq!(clock.alpha(), 0.0);
    }

    #[test]
    fn short_frames_accumulate_into_steps() {
        let mut clock = clock();
        let quarter = clock.fixed_step() / 4;
        let steps: Vec<u32> = (0..8).map(|_| clock.advance(quarter).steps).collect();
        // 16,666,667 / 4 rounds down, so the fourth quarter falls 3 ns short of a step.
        assert_eq!(steps, [0, 0, 0, 0, 1, 0, 0, 0]);
        assert!(clock.alpha() > 0.5 && clock.alpha() < 1.0);
    }

    #[test]
    fn a_stall_runs_at_most_the_catch_up_cap() {
        let mut clock = Clock::new(60, 5, Duration::from_secs(2));
        let advance = clock.advance(Duration::from_secs(1));
        assert_eq!(advance.steps, 5);
        assert_eq!(
            advance.dropped, 54,
            "60 steps were due, minus the 5 run, minus the remainder"
        );
        assert!(clock.alpha() < 1.0, "the dropped steps are not carried");
        // The next normal frame is normal again.
        let step = clock.fixed_step();
        assert!(clock.advance(step).steps <= 1);
    }

    #[test]
    fn a_long_frame_is_clamped() {
        let mut clock = clock();
        let advance = clock.advance(Duration::from_secs(10));
        assert_eq!(advance.dt, Duration::from_millis(250));
        assert_eq!(clock.game_time(), Duration::from_millis(250));
        assert_eq!(advance.steps, 8);
        assert_eq!(advance.dropped, 6);
    }
}
