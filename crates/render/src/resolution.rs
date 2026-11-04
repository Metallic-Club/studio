//! Render scale and dynamic resolution (ADR 0008 §8).
//!
//! The scene can be drawn at a **render scale**: into a texture of `scale ×` the target's size, which the upscale
//! pass ([`crate::PassId::Upscale`]) then draws over the whole target with bilinear filtering, before the UI is
//! drawn at the target's own resolution. A scale of 1 with dynamic resolution off draws the scene straight into the
//! target, as without a render scale: no texture, no upscale pass.
//!
//! **Dynamic resolution** changes the scale from frame to frame to hold the GPU frame time at a target
//! ([`DynamicResolution`]). The scene textures are made once at [`ResolutionSettings::max_scale`] and a frame draws
//! into the top-left corner of them (its viewport and scissor), so a new scale never makes a texture; the upscale
//! pass reads only that corner. The controller reads the GPU timestamps the renderer already takes
//! ([`crate::FrameTimings`]), which arrive one to three frames late, so it remembers the scale each frame was drawn
//! at and damps its steps.
//!
//! The upscaler is one pass behind one seam (`upscale.rs`): a better one (FSR 1 class, #85) replaces its shader and
//! pipeline and nothing else.

/// How the scene's resolution is chosen: a fixed render scale, or dynamic resolution between two scales.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolutionSettings {
    /// The render scale: the scene is drawn at this fraction of the target's width and height (0.5 draws a quarter
    /// of the pixels; above 1 supersamples). Clamped to [`MIN_SCALE`]..=[`MAX_SCALE`]. With dynamic resolution on,
    /// the scale it starts from.
    pub scale: f32,
    /// Change the scale every frame to hold the GPU frame time at [`ResolutionSettings::target_ms`].
    pub dynamic: bool,
    /// The GPU frame time dynamic resolution holds, in milliseconds.
    pub target_ms: f32,
    /// The lowest scale dynamic resolution goes to.
    pub min_scale: f32,
    /// The highest scale dynamic resolution goes to: the size the scene textures are made at.
    pub max_scale: f32,
}

/// The lowest render scale (ADR 0008 §8: 50 to 200%).
pub const MIN_SCALE: f32 = 0.5;
/// The highest render scale.
pub const MAX_SCALE: f32 = 2.0;

impl ResolutionSettings {
    /// The target's own resolution: no render scale, no dynamic resolution.
    pub const NATIVE: Self = Self {
        scale: 1.0,
        dynamic: false,
        target_ms: 1000.0 / 60.0,
        min_scale: MIN_SCALE,
        max_scale: 1.0,
    };

    /// A fixed render scale.
    pub fn fixed(scale: f32) -> Self {
        Self { scale, ..Self::NATIVE }
    }

    /// Dynamic resolution holding `target_ms` of GPU time a frame, between `min_scale` and `max_scale`.
    pub fn dynamic(target_ms: f32, min_scale: f32, max_scale: f32) -> Self {
        Self {
            scale: max_scale,
            dynamic: true,
            target_ms,
            min_scale,
            max_scale,
        }
    }

    /// The scale range in use: `(min, max)`, clamped to [`MIN_SCALE`]..=[`MAX_SCALE`]. Without dynamic resolution
    /// both are the fixed scale.
    pub fn range(&self) -> (f32, f32) {
        let clamp = |scale: f32| {
            if scale.is_finite() {
                scale.clamp(MIN_SCALE, MAX_SCALE)
            } else {
                1.0
            }
        };
        if self.dynamic {
            let max = clamp(self.max_scale);
            (clamp(self.min_scale).min(max), max)
        } else {
            let scale = clamp(self.scale);
            (scale, scale)
        }
    }

    /// Whether the scene is drawn into its own texture and upscaled: a scale other than 1, or dynamic resolution.
    pub fn scaled(&self) -> bool {
        self.dynamic || self.range().1 != 1.0
    }

    /// The size the scene textures are made at for a `width` × `height` target: the highest scale's.
    pub fn texture_size(&self, width: u32, height: u32) -> (u32, u32) {
        scaled_size(width, height, self.range().1)
    }
}

impl Default for ResolutionSettings {
    /// [`ResolutionSettings::NATIVE`].
    fn default() -> Self {
        Self::NATIVE
    }
}

/// The scene's resolution in a frame ([`crate::Renderer::resolution_stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResolutionStats {
    /// The render scale the frame was drawn at (1 without a render scale).
    pub scale: f32,
    /// The width the scene was drawn at, in pixels.
    pub width: u32,
    /// The height the scene was drawn at, in pixels.
    pub height: u32,
    /// Whether dynamic resolution is on.
    pub dynamic: bool,
    /// The GPU frame time dynamic resolution holds, in milliseconds.
    pub target_ms: f32,
    /// The lowest scale in use (the fixed scale without dynamic resolution).
    pub min_scale: f32,
    /// The highest scale in use.
    pub max_scale: f32,
    /// The GPU time of the newest timed frame dynamic resolution used, in milliseconds (0 without it).
    pub gpu_ms: f32,
}

/// `width` × `height` at `scale`, rounded, at least 1 × 1.
pub fn scaled_size(width: u32, height: u32, scale: f32) -> (u32, u32) {
    let side = |n: u32| ((n as f32 * scale).round() as u32).max(1);
    (side(width), side(height))
}

/// How far from the target a frame may be before the scale changes: noise in the timings does not move it.
const DEADBAND: f32 = 0.03;
/// The share of the way to the scale the last timing asks for that one step goes: the timings are a few frames
/// late, so a full step would overshoot.
const GAIN: f32 = 0.25;
/// Smaller changes are not made, so the upscale's parameters are not rewritten for a change nobody sees.
const MIN_STEP: f32 = 0.005;
/// Frames whose scale is remembered: more than the timings can be late ([`crate::TIMING_RING_SIZE`] frames).
const HISTORY: usize = 8;

/// Dynamic resolution's controller: the scale for the next frame, from the GPU time of the frames before.
///
/// A frame's GPU time is taken to grow with its pixels (the shadow maps do not, so it overestimates how much a step
/// saves, which only makes the steps smaller). From a timed frame drawn at scale `s` that took `t` ms, the scale that
/// would take the target `T` is `s · √(T / t)`; each step goes [`GAIN`] of the way there from the current scale,
/// within the settings' range, and none is taken while `t` is within 3% of `T`. Each timed frame is used once.
/// Nothing allocates.
#[derive(Clone, Debug)]
pub struct DynamicResolution {
    settings: ResolutionSettings,
    scale: f32,
    /// The scale of the last [`HISTORY`] frames drawn, by frame index.
    history: [(u64, f32); HISTORY],
    /// The newest timed frame used.
    last_timed: Option<u64>,
    /// The GPU time of the newest timed frame used, in milliseconds.
    last_gpu_ms: f32,
}

impl DynamicResolution {
    /// A controller for `settings`, at its starting scale.
    pub fn new(settings: ResolutionSettings) -> Self {
        let mut controller = Self {
            // Not dynamic yet, so `set_settings` starts from the settings' scale.
            settings: ResolutionSettings::NATIVE,
            scale: 1.0,
            history: [(u64::MAX, 1.0); HISTORY],
            last_timed: None,
            last_gpu_ms: 0.0,
        };
        controller.set_settings(settings);
        controller
    }

    /// Changes the settings. Dynamic resolution keeps its current scale within the new range (and starts from the
    /// settings' scale when it was off); a fixed scale is taken as it is.
    pub fn set_settings(&mut self, settings: ResolutionSettings) {
        let (min, max) = settings.range();
        self.scale = if settings.dynamic {
            let start = if self.settings.dynamic {
                self.scale
            } else {
                settings.scale
            };
            if start.is_finite() { start.clamp(min, max) } else { max }
        } else {
            max
        };
        self.settings = settings;
    }

    /// The settings.
    pub fn settings(&self) -> &ResolutionSettings {
        &self.settings
    }

    /// The scale the next frame is drawn at.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The GPU time of the newest timed frame used, in milliseconds (0 before the first).
    pub fn last_gpu_ms(&self) -> f32 {
        self.last_gpu_ms
    }

    /// Notes that frame `frame` is drawn at the current scale.
    pub fn drawing(&mut self, frame: u64) {
        self.history[(frame % HISTORY as u64) as usize] = (frame, self.scale);
    }

    /// Takes in the GPU time of frame `frame` (`gpu_ms` milliseconds) and returns whether the scale changed. A frame
    /// already used, one not drawn through [`DynamicResolution::drawing`] or too old to be remembered, or a time that
    /// is not positive, changes nothing; so does a fixed scale.
    pub fn observe(&mut self, frame: u64, gpu_ms: f32) -> bool {
        if !self.settings.dynamic || self.last_timed.is_some_and(|last| last >= frame) {
            return false;
        }
        self.last_timed = Some(frame);
        let (drawn, drawn_scale) = self.history[(frame % HISTORY as u64) as usize];
        if drawn != frame || !(gpu_ms > 0.0 && gpu_ms.is_finite()) {
            return false;
        }
        self.last_gpu_ms = gpu_ms;
        let target = self.settings.target_ms;
        if target.is_nan() || target <= 0.0 || (gpu_ms - target).abs() <= DEADBAND * target {
            return false;
        }
        let (min, max) = self.settings.range();
        let wanted = (drawn_scale * (target / gpu_ms).sqrt()).clamp(min, max);
        let delta = wanted - self.scale;
        if delta.abs() < MIN_STEP {
            return false;
        }
        // At least the smallest step, never past the scale wanted.
        let step = (GAIN * delta).abs().max(MIN_STEP).copysign(delta);
        let next = (self.scale + step).clamp(min, max);
        // Near a limit, step onto it rather than creep towards it.
        let next = if next - min < MIN_STEP {
            min
        } else if max - next < MIN_STEP {
            max
        } else {
            next
        };
        let changed = next != self.scale;
        self.scale = next;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GPU whose frame takes `fixed` ms plus `per_pixel` ms at scale 1, in proportion to the pixels.
    fn gpu(fixed: f32, per_pixel: f32, scale: f32) -> f32 {
        fixed + per_pixel * scale * scale
    }

    /// Runs `frames` frames whose timings arrive `latency` frames late, and returns the scales drawn.
    fn run(controller: &mut DynamicResolution, frames: u64, latency: u64, cost: impl Fn(f32) -> f32) -> Vec<f32> {
        let mut drawn = Vec::new();
        for frame in 0..frames {
            if frame >= latency {
                let timed = frame - latency;
                let ms = cost(drawn[timed as usize]);
                controller.observe(timed, ms);
            }
            controller.drawing(frame);
            drawn.push(controller.scale());
        }
        drawn
    }

    #[test]
    fn it_settles_on_the_scale_that_takes_the_target() {
        let mut controller = DynamicResolution::new(ResolutionSettings::dynamic(4.0, 0.5, 1.0));
        // 1 ms that does not scale and 6 ms that does: 4 ms at a scale of √0.5 ≈ 0.707.
        let drawn = run(&mut controller, 200, 3, |s| gpu(1.0, 6.0, s));
        let last = *drawn.last().unwrap();
        let time = gpu(1.0, 6.0, last);
        assert!((time - 4.0).abs() <= 0.04 * 4.0 + 0.01, "settled at {last} ({time} ms)");
        // It does not oscillate once settled: the last 50 frames stay within a step of each other.
        let tail = &drawn[150..];
        let (lo, hi) = tail
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)));
        assert!(hi - lo < 0.02, "{lo}..{hi}");
    }

    #[test]
    fn it_stays_within_its_range() {
        let mut controller = DynamicResolution::new(ResolutionSettings::dynamic(1.0, 0.6, 0.9));
        // Too slow at every scale: it goes to the minimum and stays.
        let drawn = run(&mut controller, 100, 2, |s| gpu(2.0, 8.0, s));
        assert_eq!(*drawn.last().unwrap(), 0.6);
        // Fast at every scale: it goes to the maximum.
        let drawn = run(&mut controller, 300, 2, |s| gpu(0.0, 0.1, s));
        assert_eq!(*drawn.last().unwrap(), 0.9);
    }

    #[test]
    fn each_timed_frame_counts_once_and_a_fixed_scale_never_moves() {
        let mut controller = DynamicResolution::new(ResolutionSettings::dynamic(4.0, 0.5, 1.0));
        controller.drawing(0);
        assert!(controller.observe(0, 8.0));
        let scale = controller.scale();
        assert!(!controller.observe(0, 8.0), "the same frame again");
        assert!(!controller.observe(5, 8.0), "a frame never drawn");
        assert_eq!(controller.scale(), scale);

        let mut fixed = DynamicResolution::new(ResolutionSettings::fixed(0.75));
        fixed.drawing(0);
        assert!(!fixed.observe(0, 100.0));
        assert_eq!(fixed.scale(), 0.75);
    }

    #[test]
    fn ranges_and_sizes() {
        assert!(!ResolutionSettings::NATIVE.scaled());
        assert!(ResolutionSettings::fixed(0.5).scaled());
        assert_eq!(ResolutionSettings::fixed(0.1).range(), (MIN_SCALE, MIN_SCALE));
        assert_eq!(ResolutionSettings::fixed(f32::NAN).range(), (1.0, 1.0));
        let dynamic = ResolutionSettings::dynamic(2.0, 0.9, 0.7);
        assert_eq!(dynamic.range(), (0.7, 0.7), "a minimum above the maximum");
        assert_eq!(ResolutionSettings::fixed(0.75).texture_size(1920, 1080), (1440, 810));
        assert_eq!(scaled_size(1, 1, 0.5), (1, 1));
    }
}
