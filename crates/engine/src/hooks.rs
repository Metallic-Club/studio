//! The seams the loop calls into for work other crates do: the fixed simulation step (physics, M7) and, with the
//! `render` feature, drawing (the renderer, #75 to #77).

use std::time::Duration;

use metallic_core::DataModel;

/// What the engine reports about a frame: returned by [`Engine::frame`](crate::Engine::frame) and handed to the
/// render hook.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    /// The frame's number, from 1. Counts every frame, paused and stopped ones too.
    pub frame: u64,
    /// How far game time moved this frame, in seconds: the `deltaTime` RunService's events passed. Zero when the
    /// game is paused or stopped.
    pub dt: f64,
    /// Game time after the frame, in seconds: what `time()` returns in scripts.
    pub game_time: f64,
    /// Fixed simulation steps run this frame.
    pub sim_steps: u32,
    /// Simulation steps that were due but dropped by the catch-up cap.
    pub dropped_steps: u32,
    /// How far game time is into the next simulation step, from 0 to just under 1: what a renderer interpolates
    /// moving things by.
    pub alpha: f64,
    /// Whether the script phases and the simulation ran (false while paused or stopped).
    pub ran: bool,
    /// Script threads the frame's phases resumed.
    pub resumed: usize,
}

/// The fixed-step simulation: the engine calls [`Simulation::step`] zero or more times per frame, between
/// `PreSimulation` and `PostSimulation`, as the accumulator decides (see [`EngineConfig::max_catch_up`]).
///
/// Physics (M7) implements it; until then the engine runs [`NoSimulation`].
///
/// [`EngineConfig::max_catch_up`]: crate::EngineConfig::max_catch_up
pub trait Simulation {
    /// Advances the simulation by one fixed step of `dt`.
    fn step(&mut self, dm: &mut DataModel, dt: Duration);
}

/// A simulation that does nothing: the engine's default until physics arrives (M7).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSimulation;

impl Simulation for NoSimulation {
    fn step(&mut self, _dm: &mut DataModel, _dt: Duration) {}
}

#[cfg(feature = "render")]
pub use render::{RenderFrame, RenderHook, RenderHookError};

#[cfg(feature = "render")]
mod render {
    use metallic_core::DataModel;
    use metallic_platform::{Size, Window};

    use super::FrameStats;

    /// Why a render hook could not attach to a window: the renderer's own error, boxed so no renderer type leaks
    /// into the engine's API.
    pub type RenderHookError = Box<dyn std::error::Error + Send + Sync>;

    /// What the render hook gets once per frame, after `Heartbeat`.
    #[derive(Debug)]
    pub struct RenderFrame<'a> {
        /// The game, after this frame's scripts and simulation. Mutable so the renderer can drain its change
        /// consumer (only what changed is re-uploaded, ADR 0004); it must not change the game's state.
        pub dm: &'a mut DataModel,
        /// The frame's numbers: game time, interpolation factor, ...
        pub stats: FrameStats,
    }

    /// The renderer's seam: how the engine drives drawing without knowing how it is done.
    ///
    /// The engine calls [`RenderHook::render`] once per frame as the frame's last step, only when it is windowed
    /// and has a window attached. That call is where the main thread builds the frame packet (dirty slots, camera,
    /// lights; ADR 0008 §11) from the data model; whether it then encodes on the spot or hands the packet to a
    /// render thread is the renderer's business. The other calls follow the window's life: [`RenderHook::attach`]
    /// when a window is there to draw into (at start, and again when a mobile app comes back to the foreground),
    /// [`RenderHook::detach`] when the surface must go (suspend, exit), [`RenderHook::resize`] when the drawable
    /// area changes.
    pub trait RenderHook {
        /// A window to draw into: create the surface. A [`Window`] is cheap to clone and implements the
        /// `raw-window-handle` traits, which is what `wgpu` needs.
        fn attach(&mut self, window: &Window) -> Result<(), RenderHookError> {
            let _ = window;
            Ok(())
        }

        /// The surface is no longer valid (suspend, exit): drop it. Nothing is drawn until the next
        /// [`RenderHook::attach`].
        fn detach(&mut self) {}

        /// The window's drawable area is now `size` physical pixels at `scale_factor` physical pixels per logical
        /// one.
        fn resize(&mut self, size: Size, scale_factor: f64) {
            let _ = (size, scale_factor);
        }

        /// Draws the frame.
        fn render(&mut self, frame: &mut RenderFrame<'_>);
    }
}
