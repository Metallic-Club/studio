//! Bounded, single-thread CPU scopes and explicit frame-budget accounting.
//!
//! [`FrameProfiler`] records scopes with a monotonic [`Clock`]. Budget totals use exclusive time:
//! nested calls are charged once, to the innermost subsystem. Trace samples retain inclusive time.
//! Missing scopes are [`Measurement::Unavailable`], never a fabricated zero or GPU measurement.
//!
//! ```
//! use metallic_profiling::{FrameProfiler, Subsystem, SystemClock, cpu_scope};
//! let mut profiler = FrameProfiler::<SystemClock, 64>::default();
//! let frame = profiler.begin_frame();
//! {
//!     cpu_scope!(frame, Subsystem::Scripts, "scripts.step");
//!     std::hint::black_box(2 + 2);
//! }
//! let report = frame.finish();
//! assert_eq!(report.scope_count(Subsystem::Scripts), 1);
//! ```
//!
//! Storage is fixed by the sample capacity and [`MAX_SCOPE_DEPTH`]. The default collector performs
//! no heap allocation while recording; optional external backends have their own storage/overhead.
//! Enable either `puffin` or `tracy`, not both, to export [`cpu_scope!`] through the `profiling`
//! crate. Puffin capture remains off until `set_puffin_enabled` is called (with that feature).
//! No UI, network server or GPU query is created by the default collector. Tracy uses manual
//! lifetime and localhost-only connections; a host must call `start_tracy` before using any engine
//! or third-party Tracy instrumentation. That explicit call can start background threads and
//! accept local profiler connections; dropping its handle does not shut down the upstream client.
//!
//! The runtime is currently a scaffold. This crate supplies instrumentation points, not automatic
//! measurements of unimplemented game stages. Worker threads need separate collectors; this API
//! does not merge concurrent scopes. Default recording is always active; opt out by not creating
//! a frame or entering its scopes.
#![warn(missing_docs)]

mod budgets;

pub use budgets::{BudgetCheck, BudgetReport, BudgetState, BudgetTable};
use std::cell::RefCell;
use std::time::{Duration, Instant};

#[cfg(all(feature = "puffin", feature = "tracy"))]
compile_error!("Select at most one profiling backend: puffin or tracy.");

#[doc(hidden)]
pub use profiling as backend;

/// Maximum simultaneously open scopes per frame; deeper scopes mark attribution incomplete.
pub const MAX_SCOPE_DEPTH: usize = 64;

/// Main-thread CPU stages, ordered like the performance research budget table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Subsystem {
    /// Input and window events (network receive in the proposed server table).
    Input,
    /// All script VMs and callbacks.
    Scripts,
    /// Physics and character simulation.
    Physics,
    /// World-to-render-resource synchronization.
    RenderSync,
    /// CPU culling, command encoding and submission; never GPU execution time.
    RenderCpu,
    /// In-game UI processing, excluding Studio panels.
    GameUi,
    /// Replication work when implemented.
    Replication,
    /// Profiling and diagnostic work explicitly instrumented by the caller.
    Diagnostics,
}

impl Subsystem {
    /// Number of CPU stages in reports and budget tables.
    pub const COUNT: usize = 8;
    /// All CPU stages in report order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Input,
        Self::Scripts,
        Self::Physics,
        Self::RenderSync,
        Self::RenderCpu,
        Self::GameUi,
        Self::Replication,
        Self::Diagnostics,
    ];
    /// Stable label for text reports; not a dynamically allocated name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Scripts => "scripts",
            Self::Physics => "physics",
            Self::RenderSync => "render_sync",
            Self::RenderCpu => "render_cpu",
            Self::GameUi => "game_ui",
            Self::Replication => "replication",
            Self::Diagnostics => "diagnostics",
        }
    }
}

/// Availability and completeness of one duration; incomplete values must not certify a budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measurement {
    /// No scope for this stage was entered in the frame.
    Unavailable,
    /// The scope(s) were measured, including a legitimate zero-duration sample.
    Complete(Duration),
    /// Observed time is partial or unreliable; inspect the report's error counters.
    Incomplete(Duration),
}

/// Monotonic timestamp source. Tests can supply explicit timestamps instead of sleeping.
pub trait Clock {
    /// Elapsed time since an arbitrary, fixed origin; observations should never decrease.
    fn now(&self) -> Duration;
}

impl<C: Clock + ?Sized> Clock for &C {
    fn now(&self) -> Duration {
        (*self).now()
    }
}

/// Production monotonic CPU clock, backed by [`Instant`].
#[derive(Debug)]
pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// A completed scope. Samples are retained in completion order, not start order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeSample {
    /// Static source label; owned script names or user input must not be allocated here.
    pub name: &'static str,
    /// CPU stage charged for the scope's exclusive duration.
    pub subsystem: Subsystem,
    /// Start offset relative to the beginning of this frame.
    pub start: Duration,
    /// Elapsed duration including nested scopes; sample inclusives may overlap.
    pub inclusive: Duration,
    /// Duration excluding all immediately nested scopes, regardless of subsystem.
    pub exclusive: Duration,
    /// Nesting level, with zero identifying an outermost scope.
    pub depth: usize,
}

#[derive(Clone, Copy)]
struct ActiveScope {
    token: u64,
    name: &'static str,
    subsystem: Subsystem,
    start: Duration,
    children: Duration,
}

struct FrameState<const N: usize> {
    samples: [Option<ScopeSample>; N],
    sample_count: usize,
    stack: [Option<ActiveScope>; MAX_SCOPE_DEPTH],
    depth: usize,
    next_token: u64,
    seen: [bool; Subsystem::COUNT],
    totals: [Duration; Subsystem::COUNT],
    calls: [u64; Subsystem::COUNT],
    root_time: Duration,
    last_tick: Duration,
    dropped: u64,
    rejected: u64,
    clock_errors: u64,
    nesting_errors: u64,
}

impl<const N: usize> FrameState<N> {
    fn new(start: Duration) -> Self {
        Self {
            samples: [None; N],
            sample_count: 0,
            stack: [None; MAX_SCOPE_DEPTH],
            depth: 0,
            next_token: 0,
            seen: [false; Subsystem::COUNT],
            totals: [Duration::ZERO; Subsystem::COUNT],
            calls: [0; Subsystem::COUNT],
            root_time: Duration::ZERO,
            last_tick: start,
            dropped: 0,
            rejected: 0,
            clock_errors: 0,
            nesting_errors: 0,
        }
    }

    fn observe(&mut self, now: Duration) {
        if now < self.last_tick {
            self.clock_errors = self.clock_errors.saturating_add(1);
        }
        self.last_tick = self.last_tick.max(now);
    }

    fn add_time(&mut self, lhs: Duration, rhs: Duration) -> Duration {
        match lhs.checked_add(rhs) {
            Some(sum) => sum,
            None => {
                self.clock_errors = self.clock_errors.saturating_add(1);
                Duration::MAX
            }
        }
    }
}

/// Factory for non-overlapping frames, with a compile-time trace capacity (default 256 samples).
///
/// A frame exclusively borrows its profiler. Scope guards borrow the frame, so ordinary Rust
/// lifetimes prevent finishing a frame while its guards remain live. Deliberately forgotten guards
/// are detected by [`FrameReport::unclosed_scopes`]. Dropping a frame without finishing discards it.
pub struct FrameProfiler<C = SystemClock, const N: usize = 256> {
    clock: C,
}

impl<C: Clock, const N: usize> FrameProfiler<C, N> {
    /// Use an explicit clock; the clock is never read by construction.
    pub fn new(clock: C) -> Self {
        Self { clock }
    }

    /// Begin a fresh frame. No samples or accounting carry over from an earlier frame.
    pub fn begin_frame(&mut self) -> Frame<'_, C, N> {
        let start = self.clock.now();
        Frame {
            profiler: self,
            start,
            state: RefCell::new(FrameState::new(start)),
        }
    }
}

impl<C: Clock + Default, const N: usize> Default for FrameProfiler<C, N> {
    fn default() -> Self {
        Self::new(C::default())
    }
}

/// One CPU frame. Single-threaded interior mutability permits nested lexical scopes.
pub struct Frame<'p, C, const N: usize> {
    profiler: &'p mut FrameProfiler<C, N>,
    start: Duration,
    state: RefCell<FrameState<N>>,
}

impl<'p, C: Clock, const N: usize> Frame<'p, C, N> {
    /// Enter a CPU-only scope, closed when the returned guard is dropped.
    ///
    /// Use [`cpu_scope!`] to additionally export a lexical scope to an optional backend. Guards
    /// must close in reverse opening order. Capacity or order errors mark attribution incomplete
    /// instead of panicking or reporting a successful budget.
    #[must_use = "the guard must stay alive for the work being measured"]
    pub fn scope(&self, subsystem: Subsystem, name: &'static str) -> Scope<'_, 'p, C, N> {
        let now = self.profiler.clock.now();
        let mut state = self.state.borrow_mut();
        state.observe(now);
        state.seen[subsystem as usize] = true;
        let token = state.next_token.checked_add(1);
        if state.depth == MAX_SCOPE_DEPTH || token.is_none() {
            state.rejected = state.rejected.saturating_add(1);
            return Scope {
                frame: self,
                token: None,
            };
        }
        let token = token.unwrap_or_default();
        state.next_token = token;
        let depth = state.depth;
        state.stack[depth] = Some(ActiveScope {
            token,
            name,
            subsystem,
            start: now,
            children: Duration::ZERO,
        });
        state.depth += 1;
        Scope {
            frame: self,
            token: Some(token),
        }
    }

    fn close_scope(&self, token: u64) {
        let now = self.profiler.clock.now();
        let mut state = self.state.borrow_mut();
        state.observe(now);
        let depth = state.depth.saturating_sub(1);
        let active = state.stack[depth].filter(|active| state.depth > 0 && active.token == token);
        let Some(active) = active else {
            state.nesting_errors = state.nesting_errors.saturating_add(1);
            state.stack.fill(None);
            state.depth = 0;
            return;
        };
        state.stack[depth] = None;
        state.depth = depth;
        let Some(inclusive) = now.checked_sub(active.start) else {
            state.clock_errors = state.clock_errors.saturating_add(1);
            return;
        };
        let Some(exclusive) = inclusive.checked_sub(active.children) else {
            state.clock_errors = state.clock_errors.saturating_add(1);
            return;
        };
        if depth == 0 {
            let previous = state.root_time;
            state.root_time = state.add_time(previous, inclusive);
        } else if let Some(mut parent) = state.stack[depth - 1] {
            parent.children = state.add_time(parent.children, inclusive);
            state.stack[depth - 1] = Some(parent);
        }
        let index = active.subsystem as usize;
        let previous = state.totals[index];
        state.totals[index] = state.add_time(previous, exclusive);
        state.calls[index] = state.calls[index].saturating_add(1);
        if state.sample_count == N {
            state.dropped = state.dropped.saturating_add(1);
        } else {
            let index = state.sample_count;
            state.samples[index] = Some(ScopeSample {
                name: active.name,
                subsystem: active.subsystem,
                start: active.start.saturating_sub(self.start),
                inclusive,
                exclusive,
                depth,
            });
            state.sample_count += 1;
        }
    }

    /// Close the frame and return owned, bounded data. No frame history is retained internally.
    ///
    /// This also marks the frame boundary in an enabled external backend. Frame elapsed time
    /// includes work outside scopes; its budget is independent of trace capacity or attribution.
    pub fn finish(self) -> FrameReport<N> {
        let report = self.finish_interval();
        finish_backend_frame();
        report
    }

    /// Finish only this CPU interval, without marking a full external profiler frame.
    ///
    /// Use for a simulation phase that excludes rendering/GUI. The application can separately
    /// call [`finish_backend_frame`] at its actual displayed-frame boundary.
    pub fn finish_interval(self) -> FrameReport<N> {
        let now = self.profiler.clock.now();
        let mut state = self.state.into_inner();
        state.observe(now);
        let elapsed = now.saturating_sub(self.start);
        if state.root_time > elapsed {
            state.clock_errors = state.clock_errors.saturating_add(1);
        }
        FrameReport {
            elapsed,
            samples: state.samples,
            sample_count: state.sample_count,
            seen: state.seen,
            totals: state.totals,
            calls: state.calls,
            root_time: state.root_time,
            dropped: state.dropped,
            rejected: state.rejected,
            clock_errors: state.clock_errors,
            nesting_errors: state.nesting_errors,
            unclosed: state.depth,
        }
    }
}

/// Mark the host application's actual external profiler frame boundary.
///
/// This does not manufacture CPU samples. It is a no-op without a backend, and safely skips an
/// uninitialized Tracy client. Hosts should mark once after their complete frame, including GUI.
pub fn finish_backend_frame() {
    #[cfg(not(feature = "tracy"))]
    profiling::finish_frame!();
    #[cfg(feature = "tracy")]
    if let Some(client) = profiling::tracy_client::Client::running() {
        client.frame_mark();
    }
}

/// RAII CPU scope guard. Keep it until the end of the measured work; dropping closes it.
pub struct Scope<'s, 'p, C: Clock, const N: usize> {
    frame: &'s Frame<'p, C, N>,
    token: Option<u64>,
}

impl<C: Clock, const N: usize> Drop for Scope<'_, '_, C, N> {
    fn drop(&mut self) {
        if let Some(token) = self.token {
            self.frame.close_scope(token);
        }
    }
}

/// Completed CPU data for one frame; no GPU timing or cross-thread aggregation is implied.
pub struct FrameReport<const N: usize = 256> {
    elapsed: Duration,
    samples: [Option<ScopeSample>; N],
    sample_count: usize,
    seen: [bool; Subsystem::COUNT],
    totals: [Duration; Subsystem::COUNT],
    calls: [u64; Subsystem::COUNT],
    root_time: Duration,
    dropped: u64,
    rejected: u64,
    clock_errors: u64,
    nesting_errors: u64,
    unclosed: usize,
}

impl<const N: usize> FrameReport<N> {
    fn attribution_complete(&self) -> bool {
        self.clock_errors == 0
            && self.nesting_errors == 0
            && self.rejected == 0
            && self.unclosed == 0
    }

    /// Wall-clock CPU frame duration, including unscoped time; not the sum of subsystem timings.
    pub fn frame_time(&self) -> Measurement {
        if self.clock_errors == 0 {
            Measurement::Complete(self.elapsed)
        } else {
            Measurement::Incomplete(self.elapsed)
        }
    }

    /// Exclusive CPU time for a stage. Missing stages remain unavailable, even on an empty frame.
    pub fn subsystem_time(&self, subsystem: Subsystem) -> Measurement {
        let index = subsystem as usize;
        if !self.seen[index] {
            Measurement::Unavailable
        } else if self.attribution_complete() {
            Measurement::Complete(self.totals[index])
        } else {
            Measurement::Incomplete(self.totals[index])
        }
    }

    /// Time outside all outermost scopes; unavailable when attribution cannot be trusted.
    pub fn unscoped_time(&self) -> Option<Duration> {
        self.attribution_complete()
            .then(|| self.elapsed.saturating_sub(self.root_time))
    }

    /// Retained completed scope samples, in completion order. Iteration does not allocate.
    pub fn samples(&self) -> impl Iterator<Item = &ScopeSample> {
        self.samples[..self.sample_count]
            .iter()
            .filter_map(Option::as_ref)
    }

    /// Number of completed scopes charged to this stage, including samples dropped from the trace.
    pub fn scope_count(&self, subsystem: Subsystem) -> u64 {
        self.calls[subsystem as usize]
    }
    /// Trace samples omitted because capacity was reached; budget totals still include them.
    pub fn dropped_samples(&self) -> u64 {
        self.dropped
    }
    /// Scopes refused because nesting capacity or scope-token space was exhausted.
    pub fn rejected_scopes(&self) -> u64 {
        self.rejected
    }
    /// Non-monotonic timestamp or invalid duration-arithmetic observations.
    pub fn clock_errors(&self) -> u64 {
        self.clock_errors
    }
    /// Scope closures that did not match the latest opening; attribution becomes incomplete.
    pub fn nesting_errors(&self) -> u64 {
        self.nesting_errors
    }
    /// Scopes still open at finish, normally caused by deliberately forgotten guards.
    pub fn unclosed_scopes(&self) -> usize {
        self.unclosed
    }

    /// Assess explicit budgets; unavailable or incomplete data never passes a stage's budget.
    pub fn check_budgets(&self, budgets: &BudgetTable) -> BudgetReport {
        BudgetReport::new(self, budgets)
    }
}

/// Record a lexical CPU scope and optionally export that same lifetime to puffin or Tracy.
///
/// The label must be a string literal. The macro expands to guard bindings in the caller's block;
/// put it in braces when measuring only part of a function. Repeated/nested calls are supported.
#[macro_export]
macro_rules! cpu_scope {
    ($frame:expr, $subsystem:expr, $name:literal) => {
        let _metallic_cpu_scope = $frame.scope($subsystem, $name);
        $crate::backend_scope!($name);
    };
}

/// Export one lexical scope without a frame collector, for subsystem entrypoints.
///
/// This is a no-op without a backend. Puffin capture must be enabled explicitly. Tracy retains an
/// optional span guard in the caller's block and is inactive until a client has been started.
/// Labels are static literals; no script source, dynamic names or other user data are exported.
#[cfg(not(feature = "tracy"))]
#[macro_export]
macro_rules! backend_scope {
    ($name:literal) => {
        $crate::backend::scope!($name);
    };
}

/// Export a lexical scope to a running Tracy client, retaining its guard until block exit.
#[cfg(feature = "tracy")]
#[macro_export]
macro_rules! backend_scope {
    ($name:literal) => {
        let _metallic_backend_scope = $crate::backend::tracy_client::Client::running()
            .map(|client| client.span($crate::backend::tracy_client::span_location!($name), 0));
    };
}

/// Enable or disable optional puffin capture; CPU budget recording is unaffected.
#[cfg(feature = "puffin")]
pub fn set_puffin_enabled(enabled: bool) {
    profiling::puffin::set_scopes_on(enabled);
}

/// Explicitly start optional Tracy profiling with localhost-only access.
///
/// Call at host startup, before any engine/third-party profiling instrumentation, and retain the
/// returned handle. The upstream client has process lifetime here: dropping the handle does not
/// stop its threads or listener. This crate exposes no shutdown function; upstream explicit
/// shutdown is unsafe while any thread can still instrument work. Never call from ordinary tests.
#[cfg(feature = "tracy")]
pub fn start_tracy() -> profiling::tracy_client::Client {
    profiling::tracy_client::Client::start()
}
