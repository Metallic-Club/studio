//! Tweens with Roblox `TweenService` semantics: [`TweenInfo`] and [`Tween`].
//!
//! Timeline of one tween: `delay_time`, then `repeat_count + 1` cycles. A cycle is one pass of
//! `time` seconds from start to goal, followed (when `reverses`) by a pass of `time` seconds back
//! to the start. So with `reverses` a tween ends where it began. `repeat_count = -1` (any
//! negative value) repeats forever. The delay happens once, before the first cycle (assistant's
//! call, matching the Roblox reference text "time of delay until the tween begins").

use crate::easing::{EasingDirection, EasingStyle, ease};
use crate::value::Animatable;
use serde::{Deserialize, Serialize};

/// How a tween moves. Defaults match Roblox's `TweenInfo.new()`: 1 s, Quad, Out, no repeat,
/// no reverse, no delay.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TweenInfo {
    /// Seconds for one pass from start to goal.
    pub time: f32,
    /// Curve shape.
    pub easing_style: EasingStyle,
    /// Which end eases.
    pub easing_direction: EasingDirection,
    /// Extra cycles after the first; negative = forever.
    pub repeat_count: i32,
    /// Play back to the start after each forward pass.
    pub reverses: bool,
    /// Seconds to wait before starting.
    pub delay_time: f32,
}

impl Default for TweenInfo {
    fn default() -> Self {
        Self {
            time: 1.0,
            easing_style: EasingStyle::Quad,
            easing_direction: EasingDirection::Out,
            repeat_count: 0,
            reverses: false,
            delay_time: 0.0,
        }
    }
}

/// Where a tween is at a moment in time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TweenSample {
    /// Eased progress from start (`0`) to goal (`1`); may overshoot with Back or Elastic.
    pub alpha: f32,
    /// True once the whole timeline (all repeats) is over.
    pub finished: bool,
}

impl TweenInfo {
    /// A tween of `time` seconds with default easing.
    pub fn new(time: f32) -> Self {
        Self {
            time,
            ..Self::default()
        }
    }

    /// Same info with another easing.
    pub fn with_easing(mut self, style: EasingStyle, direction: EasingDirection) -> Self {
        self.easing_style = style;
        self.easing_direction = direction;
        self
    }

    /// Same info with repeats and reversing.
    pub fn with_repeat(mut self, repeat_count: i32, reverses: bool) -> Self {
        self.repeat_count = repeat_count;
        self.reverses = reverses;
        self
    }

    /// Same info with a start delay.
    pub fn with_delay(mut self, delay_time: f32) -> Self {
        self.delay_time = delay_time;
        self
    }

    /// Seconds of one cycle (doubled when reversing).
    pub fn cycle_time(&self) -> f32 {
        let t = self.time.max(0.0);
        if self.reverses { 2.0 * t } else { t }
    }

    /// Total length including the delay, or `None` when it repeats forever.
    pub fn total_duration(&self) -> Option<f32> {
        if self.repeat_count < 0 {
            return None;
        }
        Some(self.delay_time.max(0.0) + self.cycle_time() * (self.repeat_count as f32 + 1.0))
    }

    /// Progress at `elapsed` seconds since `play`.
    pub fn sample(&self, elapsed: f32) -> TweenSample {
        let e = if elapsed.is_finite() { elapsed } else { 0.0 };
        let t = e - self.delay_time.max(0.0);
        if t <= 0.0 {
            return TweenSample {
                alpha: 0.0,
                finished: false,
            };
        }
        let pass = self.time.max(0.0);
        let cycle = self.cycle_time();
        let end_alpha = if self.reverses { 0.0 } else { 1.0 };
        if cycle <= 0.0 {
            return TweenSample {
                alpha: end_alpha,
                finished: self.repeat_count >= 0,
            };
        }
        if let Some(total) = self.total_duration()
            && e >= total
        {
            return TweenSample {
                alpha: end_alpha,
                finished: true,
            };
        }
        let in_cycle = t.rem_euclid(cycle);
        let linear = if in_cycle <= pass {
            in_cycle / pass
        } else {
            1.0 - (in_cycle - pass) / pass
        };
        TweenSample {
            alpha: ease(self.easing_style, self.easing_direction, linear),
            finished: false,
        }
    }
}

/// Playback state of a [`Tween`]; names follow Roblox's `Enum.PlaybackState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlaybackState {
    /// Created, not played yet.
    #[default]
    Begin,
    /// Waiting out `delay_time`.
    Delayed,
    /// Moving.
    Playing,
    /// Paused; `play` resumes.
    Paused,
    /// Reached the end.
    Completed,
    /// Stopped by `cancel`; `play` starts over.
    Cancelled,
}

/// A tween of one value from `start` to `goal`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tween<T: Animatable> {
    /// Timing and easing.
    pub info: TweenInfo,
    start: T,
    goal: T,
    elapsed: f32,
    state: PlaybackState,
    value: T,
}

impl<T: Animatable> Tween<T> {
    /// A tween that has not started. Like Roblox, the start value is the property's value when
    /// the tween is created (call [`Tween::set_start`] to change it before playing).
    pub fn new(info: TweenInfo, start: T, goal: T) -> Self {
        Self {
            info,
            start,
            goal,
            elapsed: 0.0,
            state: PlaybackState::Begin,
            value: start,
        }
    }

    /// Replaces the start value.
    pub fn set_start(&mut self, start: T) {
        self.start = start;
        if self.state == PlaybackState::Begin {
            self.value = start;
        }
    }

    /// Starts, resumes after `pause`, or restarts after `cancel` or completion.
    pub fn play(&mut self) {
        match self.state {
            PlaybackState::Paused | PlaybackState::Playing | PlaybackState::Delayed => {}
            _ => self.elapsed = 0.0,
        }
        self.state = if self.elapsed < self.info.delay_time {
            PlaybackState::Delayed
        } else {
            PlaybackState::Playing
        };
    }

    /// Pauses in place.
    pub fn pause(&mut self) {
        if matches!(self.state, PlaybackState::Playing | PlaybackState::Delayed) {
            self.state = PlaybackState::Paused;
        }
    }

    /// Stops and forgets progress. The value stays where it is (no snap back).
    pub fn cancel(&mut self) {
        if self.state != PlaybackState::Completed {
            self.state = PlaybackState::Cancelled;
            self.elapsed = 0.0;
        }
    }

    /// Advances by `dt` seconds (only while playing or delayed) and returns the current value.
    pub fn update(&mut self, dt: f32) -> T {
        if !matches!(self.state, PlaybackState::Playing | PlaybackState::Delayed) {
            return self.value;
        }
        self.elapsed += if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let s = self.info.sample(self.elapsed);
        self.value = T::interpolate(self.start, self.goal, s.alpha);
        self.state = if s.finished {
            PlaybackState::Completed
        } else if self.elapsed < self.info.delay_time {
            PlaybackState::Delayed
        } else {
            PlaybackState::Playing
        };
        self.value
    }

    /// Current value.
    pub fn value(&self) -> T {
        self.value
    }

    /// Current state.
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Seconds since play (excluding paused time).
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }
}
