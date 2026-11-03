//! The language-neutral scripting host.
//!
//! Owns: the `ScriptLanguage` trait that language backends implement, the task scheduler, signal connections, time
//! and memory budgets, the registry-driven binding description and the API dump.
//!
//! Must not depend on: any specific language or VM, or the render, physics, project or platform crates.
//! See `docs/adr/0003-scripting.md`.
//!
//! # Who owns what
//!
//! - A **backend** ([`ScriptLanguage`], one per language: Luau in `metallic_luau`, more later) owns its VM, its
//!   compiled chunks, its threads and the functions stored under listener keys. It hands the host opaque
//!   handles ([`ChunkId`], [`ThreadId`]) and never keeps a reference into the engine between calls.
//! - The **host** ([`ScriptHost`]) owns the backends (chosen by the script file's extension), the chunk cache (one
//!   compile per file and content), and the bookkeeping that ties every thread and listener to the script that
//!   created it, so [`ScriptHost::reap`] can stop all of it. It decides when code runs and with which time budget,
//!   and reports every failure to an [`OutputSink`] with the script's name and line.
//! - The **data model** (`metallic_core`) owns instances and connections. A connection names an opaque
//!   [`ListenerKey`](metallic_core::ListenerKey) the host chose; core never stores a closure.
//!
//! # The call flow
//!
//! [`ScriptHost::run_script`] picks the backend, compiles (or reuses) the chunk, asks the backend to
//! [`spawn`](ScriptLanguage::spawn) the main thread and [`resume`](ScriptLanguage::resume)s it once. During every
//! call the backend sees a [`HostContext`]: the data model, the running script, a fresh [`Deadline`], the output,
//! and the host's connection table. Engine calls go through `MethodCall` and property reads and writes on the data
//! model, exactly as for any other language. A resume ends in a [`Resume`]: finished, yielded with a
//! [`YieldReason`], or errored with a [`ScriptFailure`]. Signal events are queued by the data model and delivered
//! by [`ScriptHost::dispatch_signals`], which runs each listener in a new thread of its backend. From then on the
//! scheduler below decides when each thread runs again.
//!
//! # The frame
//!
//! The engine loop calls [`ScriptHost::step`] once per [`Phase`] per frame, in this order, with the game time
//! `now` in seconds (from the engine's clock, never the wall clock, so runs are reproducible; it never goes back):
//!
//! 1. `RenderStepped` (clients only), 2. `PreSimulation`, then physics, 3. `PostSimulation`, 4. `Heartbeat`.
//!
//! RunService's events of those names come with the `RunService` class: the engine fires each on the data model
//! just before the matching step, so their listeners run in that step. Every step does, in order:
//!
//! 1. **`Heartbeat` only: due sleepers.** Threads in `task.wait` and `task.delay` whose wake time has come, earliest
//!    wake time first and, among equal times, in the order they went to sleep. A thread that goes to sleep during
//!    this pass waits for the next `Heartbeat`, even for `task.wait(0)`.
//! 2. **Deferred work.** The deferred queue, oldest first, until it is empty: work deferred by deferred work runs in
//!    the same pass.
//! 3. **Signals.** Every event queued so far (one drain of the data model): each delivery to a listener runs in a
//!    new thread, each delivery to a thread in `signal:Wait()` resumes it. Events those threads fire wait for the
//!    next step.
//! 4. **Deferred work** again, which the listeners just queued.
//!
//! [`ScriptHost::resume_waiting`] is steps 1 and 2 alone. [`ScriptHost::run_script`], [`ScriptHost::resume_thread`]
//! and [`ScriptHost::dispatch_signals`] run code at once but are not resumption points: deferred work they queue
//! runs at the next step.
//!
//! # Scheduling rules (the `task` library)
//!
//! - **`task.spawn(f, ...)`** runs `f` at once, nested in the caller, until it first yields; then the caller goes on.
//!   The backend runs it (only it can, mid-call) and reports with [`HostContext::spawned`]; from there the host
//!   schedules the thread like any other.
//! - **`task.defer(f, ...)`** ([`HostContext::defer`], or a thread yielding [`YieldReason::Defer`]) queues the
//!   thread at the end of the deferred queue, run at the end of the current resumption point (step 2 or 4 above).
//!   A deferred thread that defers more runs one level deeper; past [`MAX_DEFER_DEPTH`] (80, as in Roblox) the
//!   `task.defer` call errors in the calling thread, and only there.
//! - **`task.delay(t, f, ...)`** ([`HostContext::delay`]) and **`task.wait(t)`** ([`YieldReason::Wait`]) sleep until
//!   `now + t`, and wake at the first `Heartbeat` at or after it (never the one that is running). `task.wait`
//!   returns the time that actually passed, never less than `t`; `task.delay` passes `f` its arguments.
//! - **`task.cancel(thread)`** ([`HostContext::cancel`], [`ScriptHost::cancel`]) unschedules the thread, wherever it
//!   waits, and releases it.
//! - **`Connect`, `Once`, `Disconnect`** ([`HostContext::connect`], [`HostContext::once`],
//!   [`HostContext::disconnect`]): every delivery runs the listener in its own thread, so any handler may yield; a
//!   handler error ends only that thread and is reported. A `Once` connection is disconnected before its listener
//!   runs, and further events already queued for it are dropped.
//! - **`signal:Wait()`** ([`YieldReason::WaitSignal`]): the host connects a listener of its own, and on the next
//!   event disconnects it and resumes the thread with the event's arguments.
//! - **Errors** from any thread are reported to the [`OutputSink`] with the script's full name and line, and end
//!   only that thread.
//! - **Resuming a thread by hand** ([`ScriptHost::resume_thread`]) or spawning it again abandons whatever it was
//!   scheduled for; a thread waits for one thing at a time.
//!
//! # Budgets
//!
//! Each call into a backend has its own deadline ([`ScriptHost::set_resume_budget`], a backstop against runaway
//! loops). Each step also has a time budget ([`ScriptHost::set_phase_budget`], [`DEFAULT_PHASE_BUDGET`]), checked
//! between resumes: once it is spent, the remaining work stays queued in the same order and runs first at the next
//! step that runs that kind of work (sleepers at the next `Heartbeat`, deferred threads and signal deliveries at the
//! next step), and the output gets one warning per frame at most. A step always resumes at least one due thread, so
//! even a zero budget makes progress.
//!
//! # Reaping
//!
//! [`ScriptHost::reap`] stops a script: its threads leave the sleeper heap, the deferred queue and the signal
//! waiters (whose connections are disconnected), and are released with its listeners. The engine calls it when a
//! script is destroyed, disabled, or moved out of a service that runs scripts.
//!
//! **Known gap:** when the instance whose signal a thread `Wait`s on is destroyed, the data model drops the
//! connection without telling the host, so the thread stays parked (and counted in
//! [`ScriptHost::waiter_count`]) until its script is reaped. Core will report dropped connections (#52).
//!
//! # Performance
//!
//! Sleepers live in a binary heap keyed by (wake time, sleep order), so a step looks only at due threads: a frame
//! with 5,000 sleepers and nothing due costs a few hundred nanoseconds and allocates nothing. Cancelling leaves a
//! stale heap or queue entry that is skipped (and compacted away once stale entries outnumber live ones), so it never
//! searches. Resuming allocates nothing beyond the backend's own work. `benches/scheduler.rs` measures it.
//!
//! # Why it stays language-neutral
//!
//! Every value crossing the trait is a `ScriptValue`, every engine call a `MethodCall`, every wait a
//! [`YieldReason`]. The host owns the semantics (when threads resume, what a connection is, when a script is
//! stopped), so a second language behaves like the first by construction. The test-only mock backend in this crate
//! drives the whole host through the same trait without any VM: if something could only be done through Luau, the
//! trait would be wrong (ADR 0003).
//!
//! # Not here yet
//!
//! The script classes (#47) feed [`ScriptHost::run_script`] and call [`ScriptHost::reap`] automatically; the engine
//! loop and RunService (M3) call [`ScriptHost::step`].

mod context;
mod deadline;
mod host;
mod language;
mod output;
mod scheduler;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod scheduler_tests;
#[cfg(test)]
mod tests;

pub use context::{Connection, HostContext, HostRequest};
pub use deadline::Deadline;
pub use host::{DEFAULT_RESUME_BUDGET, HostError, LanguageId, ScriptHost, ScriptRun, ThreadRef, ThreadState};
pub use language::{ChunkId, CompileError, Resume, ScriptFailure, ScriptLanguage, SignalRef, ThreadId, YieldReason};
pub use output::{CollectOutput, OutputLevel, OutputLine, OutputSink, OutputSource};
pub use scheduler::{DEFAULT_PHASE_BUDGET, MAX_DEFER_DEPTH, Phase, StepStats};
