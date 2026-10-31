//! How fast the Player's event loop runs (#271).
//!
//! - **Visible and focused:** `ControlFlow::Poll` plus a redraw request every turn. Vsync in
//!   `present` sets the pace, so this does not spin.
//! - **Visible, not focused:** at most [`BACKGROUND_FPS`] frames a second (`WaitUntil`). The
//!   game keeps simulating at its fixed 60 Hz ticks (each frame runs the ticks it owes), so a
//!   game in a background window keeps going; it only draws less often.
//! - **Minimised or occluded:** `ControlFlow::Wait` and no redraw requests. Nothing is presented
//!   then, so nothing else would slow the loop down and it would spin a CPU core. The game is
//!   paused: no frames run, and the first frame after the window comes back starts from a fresh
//!   frame time instead of the time spent minimised. (Offline only: a networked client must keep
//!   ticking while minimised; that is an M2 follow-up.)
//! - **Automated runs** (`--fps`, `--frames`, `--capture`, `--autopilot`, `--input-script`)
//!   always run flat out, so a measurement or a scripted run is never throttled or paused.
use std::time::{Duration, Instant};
use winit::event_loop::ControlFlow;

/// Frames a second while the window is visible but not focused.
pub const BACKGROUND_FPS: u32 = 30;

/// What the window looks like to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    /// Minimised (or a zero-sized client area).
    pub minimised: bool,
    /// Completely hidden behind other windows (where the platform reports it).
    pub occluded: bool,
    /// Has keyboard focus.
    pub focused: bool,
}

/// How the loop runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// Every turn draws a frame.
    Full,
    /// A frame every 1 / [`BACKGROUND_FPS`] s.
    Background,
    /// No frames until the window is shown again.
    Paused,
}

impl Pace {
    /// The pace for a window in `state`; `automated` runs always get [`Pace::Full`].
    pub fn for_window(state: WindowState, automated: bool) -> Pace {
        if automated {
            Pace::Full
        } else if state.minimised || state.occluded {
            Pace::Paused
        } else if !state.focused {
            Pace::Background
        } else {
            Pace::Full
        }
    }

    /// The event loop's control flow after a frame drawn at `last_frame`.
    pub fn control_flow(self, last_frame: Instant) -> ControlFlow {
        match self {
            Pace::Full => ControlFlow::Poll,
            Pace::Background => ControlFlow::WaitUntil(last_frame + background_interval()),
            Pace::Paused => ControlFlow::Wait,
        }
    }

    /// True when a frame should be requested now.
    pub fn wants_frame(self, last_frame: Instant, now: Instant) -> bool {
        match self {
            Pace::Full => true,
            Pace::Background => now >= last_frame + background_interval(),
            Pace::Paused => false,
        }
    }
}

fn background_interval() -> Duration {
    Duration::from_secs(1) / BACKGROUND_FPS
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOWN: WindowState = WindowState {
        minimised: false,
        occluded: false,
        focused: true,
    };

    #[test]
    fn a_minimised_window_waits_and_draws_nothing() {
        let state = WindowState {
            minimised: true,
            focused: false,
            ..SHOWN
        };
        let pace = Pace::for_window(state, false);
        assert_eq!(pace, Pace::Paused);
        let t = Instant::now();
        assert_eq!(pace.control_flow(t), ControlFlow::Wait);
        assert!(!pace.wants_frame(t, t + Duration::from_secs(60)));
        // Minimised while still focused (Windows can report that) is paused too.
        let state = WindowState {
            minimised: true,
            ..SHOWN
        };
        assert_eq!(Pace::for_window(state, false), Pace::Paused);
        let state = WindowState {
            occluded: true,
            ..SHOWN
        };
        assert_eq!(Pace::for_window(state, false), Pace::Paused);
    }

    #[test]
    fn a_focused_window_runs_every_turn() {
        let pace = Pace::for_window(SHOWN, false);
        assert_eq!(pace, Pace::Full);
        let t = Instant::now();
        assert_eq!(pace.control_flow(t), ControlFlow::Poll);
        assert!(pace.wants_frame(t, t));
    }

    #[test]
    fn an_unfocused_window_draws_at_the_background_rate() {
        let state = WindowState {
            focused: false,
            ..SHOWN
        };
        let pace = Pace::for_window(state, false);
        assert_eq!(pace, Pace::Background);
        let t = Instant::now();
        let next = t + Duration::from_secs(1) / BACKGROUND_FPS;
        assert_eq!(pace.control_flow(t), ControlFlow::WaitUntil(next));
        assert!(!pace.wants_frame(t, t + Duration::from_millis(5)));
        assert!(pace.wants_frame(t, next));
    }

    #[test]
    fn automated_runs_are_never_throttled() {
        for minimised in [false, true] {
            for focused in [false, true] {
                let state = WindowState {
                    minimised,
                    occluded: minimised,
                    focused,
                };
                assert_eq!(Pace::for_window(state, true), Pace::Full);
            }
        }
    }
}
