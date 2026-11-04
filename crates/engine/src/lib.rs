//! The engine runtime: the one crate that knows every subsystem, and the one apps depend on.
//!
//! Owns: the game loop (fixed physics step plus render step), firing `RunService`'s events, and the wiring of the
//! registry, scripting backends, renderer, physics and project loading.
//!
//! Features: `render`, `physics` and `luau` (all on by default). A dedicated server builds with
//! `--no-default-features --features physics,luau`, which builds no window stack (no winit). See
//! `docs/adr/0001-project-layout.md`.
//!
//! # Running a game
//!
//! [`Engine`] owns the game ([`DataModel`](metallic_core::DataModel)), the script host (with the Luau backend under
//! `luau`), the game clock and, under `render`, the window and the [`RenderHook`]. [`EngineConfig`] sets the run
//! context, the tick rate, the catch-up cap, headless or windowed, and where script sources come from.
//!
//! ```
//! # #[cfg(feature = "luau")] {
//! use metallic_engine::{Engine, EngineConfig};
//! use metallic_script::{MemorySources, RunContext};
//!
//! let config = EngineConfig::new(RunContext::Server).with_sources(MemorySources::new());
//! let mut engine = Engine::new(config)?;
//! engine.start()?; // the scripts' top-level code runs here
//! engine.run_headless(60); // one second of game time at 60 Hz
//! assert!((engine.game_time() - 1.0).abs() < 1e-6);
//! # }
//! # Ok::<(), metallic_engine::EngineError>(())
//! ```
//!
//! Headless engines are driven by [`Engine::tick`] (one frame of one fixed step: servers and tests),
//! [`Engine::run_headless`] or [`Engine::frame`] (one frame of a real time step). Windowed engines run in the
//! platform's event loop through [`run_windowed`] ([`WindowedApp`]), which times frames with the wall clock and
//! feeds the input queue.
//!
//! # The frame
//!
//! Every frame runs these steps in this order, which is the order `metallic_script`'s crate docs set for the
//! script phases. "Fire" means RunService's event of that name is fired on the data model just before the script
//! host's step of the same name, so its listeners run inside that step, with the frame's time step as `deltaTime`.
//!
//! | # | Step | What runs | Profiling scope |
//! |---|---|---|---|
//! | 1 | input | the events queued since the last frame become this frame's ([`Engine::input_events`]); `UserInputService` takes them and fires its events (only while the game runs); `MouseBehavior` reaches the window | `input` |
//! | 2 | `RenderStepped` | clients and standalone games only (not [`RunContext::Server`]): fire, then step | `RenderStepped` |
//! | 3 | `PreSimulation` | fire, then step | `PreSimulation` |
//! | 4 | simulation | `N` fixed steps of the [`Simulation`] hook (physics is M7; [`NoSimulation`] until then) | `Simulation` |
//! | 5 | `PostSimulation` | fire, then step | `PostSimulation` |
//! | 6 | `Heartbeat` | fire, then step; due `task.wait`/`task.delay` threads resume here, before the listeners | `Heartbeat` |
//! | 7 | render | the [`RenderHook`], when windowed with a window attached | `render` |
//!
//! Steps 2 to 6 run only while the game runs ([`RunState::Running`]). A stopped or paused game still takes input
//! and draws, so Studio's edit mode and pause keep a live viewport.
//!
//! # Time
//!
//! - **Game time** moves by each frame's real time step, clamped to [`EngineConfig::max_frame_time`] (250 ms), only
//!   while the game runs. It is the `now` of every script step, so `time()` and `task.wait` in scripts use it, and it
//!   never goes back. A headless [`Engine::tick`] moves it by exactly one fixed step.
//! - **The simulation** runs at a fixed rate ([`EngineConfig::tick_rate`], 60 Hz) from an accumulator: a frame runs
//!   as many whole steps as game time owes it, at most [`EngineConfig::max_catch_up`] (8). Steps past the cap are
//!   dropped rather than carried, so a stall slows the game down instead of making every later frame longer (the
//!   spiral of death). Time is kept in whole nanoseconds and the step is rounded up (60 Hz is 16,666,667 ns), so a
//!   frame of one step is always exactly one step, and `tick_rate` ticks always cover a second.
//! - **Every phase gets the frame's time step** as `deltaTime` (the real time step, clamped), not the simulated
//!   time: at 144 fps most frames run no simulation step, yet scripts still see the time that passed.
//! - **Pause and step** (Studio, M6): [`Engine::pause`] freezes game time and skips steps 2 to 6;
//!   [`Engine::step_frame`] runs one whole frame of one fixed step while paused; [`Engine::resume`] goes on.
//!
//! # `RunService`
//!
//! The class lives in `metallic_classes` (`services::run_service`). The engine writes its hidden run flags
//! (`IsStudio`, `IsRunning`, `IsClient`, `IsServer`) when it is made and when the game starts or stops: a
//! [`RunContext::Local`] game is both client and server. Its four events are fired only while something is
//! connected to `RunService` ([`DataModel::is_watched`](metallic_core::DataModel::is_watched)), so a game whose scripts
//! do not use them pays one flag check per phase.
//!
//! # `UserInputService`
//!
//! The class lives in `metallic_classes` (`services::user_input_service`, with `input::input_object` and
//! `input::key_code`), and so does the code that turns input into its state and events (`input::feed`). The engine's
//! part, under `render`, is the input step: it translates the platform's events (`metallic_platform::KeyCode` to
//! `Enum.KeyCode` through the one table [`metallic_classes::with_key_codes`] keeps), and applies the cursor state
//! scripts set to the window: `MouseBehavior` `LockCenter` and `LockCurrentPosition` ask for
//! `CursorGrab::Lock`, after moving the cursor to the point to hold (where the platform can only confine the
//! cursor, the engine moves it back at each input step), `Default` for `None`, and `MouseIconEnabled` shows or
//! hides the cursor. Each is applied once per change, and again to a new window; a lock again when focus returns.
//!
//! - **Mouse movement is free when unwatched:** it is summed per frame, `InputChanged` fires once per frame and
//!   only while something listens to it ([`DataModel::is_signal_watched`](metallic_core::DataModel::is_signal_watched)),
//!   so with nothing connected 1,000 motion events create no instance and allocate nothing (`tests/input_alloc.rs`).
//! - **One input object per press**, made only while something listens and destroyed after `End`; a press shorter
//!   than a frame ends in the next frame, so `InputBegan` sees `Begin`. Mouse movement and the wheel reuse one
//!   object each (`metallic_classes::input::feed` has the rules).
//! - **A headless build** (no `render`) has no input: the service exists and nothing feeds it.
//!
//! Drawing is not done here: the renderer (`metallic_render`, #75 to #77) implements [`RenderHook`] and is
//! installed with [`Engine::set_render_hook`]. The engine calls [`RenderHook::render`] once per frame, last, with a
//! [`RenderFrame`]: the data model (mutable, so the renderer can drain its change consumer) and the frame's
//! [`FrameStats`] (game time, the interpolation factor `alpha`, ...). That call is where the main thread builds
//! the frame packet (ADR 0008 §11). [`RenderHook::attach`], [`RenderHook::detach`] and [`RenderHook::resize`]
//! follow the window's life. Headless engines never call it, and without the `render` feature it does not exist.
//! [`SceneRenderer`] is the renderer's hook: a thin adapter that keeps `metallic_render`'s GPU-resident scene in
//! step with the game and draws its parts from `Workspace.CurrentCamera`.
//!
//! # Profiling
//!
//! The steps above run inside `profiling` crate scopes, and every frame ends with `profiling::finish_frame!()`.
//! With no back end the macros expand to nothing, so they cost nothing. `--features profile-with-puffin` or
//! `--features profile-with-tracy` picks a back end; the app that turns one on also starts the profiler (puffin:
//! `puffin::set_scopes_on(true)` and a `puffin_http` server; Tracy: `tracy_client::Client::start()`).
//!
//! Independently of those, a frame can measure its own script and simulation wall time
//! ([`FrameStats::script_cpu`], [`FrameStats::simulation_cpu`]; [`Engine::set_cpu_timing`], on by default for
//! windowed engines). [`SceneRenderer`] adds its render time, the GPU timings and the draws, and keeps them in
//! [`perf::PerfStats`]: the numbers of the performance overlay ([`SceneRenderer::set_overlay`]).
//!
//! # Stress scenes
//!
//! [`stress`] builds the scenes every renderer change is measured against (ADR 0008 §10), by name and seed, and,
//! under `render`, runs one in a window for a number of frames and reports the frame statistics as JSON
//! ([`stress::run_bench`]; `cargo run --release -p metallic_engine --example stress -- --scene parts_100k`).

mod clock;
mod config;
mod engine;
mod error;
#[cfg(feature = "render")]
pub mod graphics;
mod hooks;
#[cfg(feature = "render")]
mod input;
mod output;
#[cfg(feature = "render")]
pub mod perf;
#[cfg(feature = "render")]
mod scene_renderer;
pub mod stress;
#[cfg(feature = "render")]
mod user_input;
#[cfg(feature = "render")]
mod windowed;

pub use config::{DEFAULT_MAX_CATCH_UP, DEFAULT_MAX_FRAME_TIME, DEFAULT_TICK_RATE, EngineConfig, EngineMode};
pub use engine::{Engine, RunState};
pub use error::EngineError;
pub use hooks::{FrameStats, NoSimulation, Simulation};
#[cfg(feature = "render")]
pub use hooks::{RenderFrame, RenderHook, RenderHookError};
#[cfg(feature = "render")]
pub use input::InputEvent;
pub use output::StdOutput;
#[cfg(feature = "render")]
pub use scene_renderer::SceneRenderer;
#[cfg(feature = "render")]
pub use windowed::{WindowedApp, WindowedOptions, run_windowed};

/// Re-exported so apps and tests name the same script host types as the engine.
pub use metallic_script::{MemorySources, RunContext, SourceLoader};

// The crates whose types the engine's API takes and returns (the data model and its values, output sinks, windows
// and the event loop), re-exported because apps depend on the engine alone (ADR 0001): the Player builds its game's
// instances and drives the platform's event loop through these.
pub use metallic_core;
pub use metallic_platform;
// The renderer too (with a window), so apps name its graphics settings through the engine.
#[cfg(feature = "render")]
pub use metallic_render;
pub use metallic_script;
