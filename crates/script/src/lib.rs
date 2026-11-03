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
//! # Running a game
//!
//! [`ScriptHost::start_game`] runs the game's scripts: every enabled script of the [`RunContext`]'s class
//! (`Script` on a server, `LocalScript` on a client, both in a standalone [`RunContext::Local`] game) inside a
//! service flagged `runs_scripts`. A script's `Source` is a project-relative path, read through a
//! [`SourceLoader`] ([`MemorySources`] until projects arrive in M4); an empty `Source` runs nothing. Scripts start
//! in tree order: the services in the order they sit under the root, and within each, depth-first (a parent before
//! its children, children in order).
//!
//! From then on the host follows each script's lifecycle, through a change consumer and connections of its own
//! (neither reaches a backend):
//!
//! - **disabled** (`Enabled = false`), **destroyed**, or **moved out** of every service that runs scripts (parented
//!   elsewhere or to nil): the script is reaped;
//! - **re-enabled**, or **moved back** into such a service: it starts again, from the top, as a new run;
//! - **added** at runtime into such a service (created there, moved there, cloned there) while enabled: it starts.
//!
//! A change takes effect at the next signal dispatch (step 3 of a frame step, or [`ScriptHost::dispatch_signals`]),
//! after the change's own events: a script that disables another keeps running until then, and so does the other.
//! Scripts that start at the same dispatch start in the order they were first created or moved since the previous
//! one, then the re-enabled ones in the order they were enabled.
//!
//! **Cost.** A running game registers one change consumer with the data model (`STRUCTURE` interest, plus the
//! `CREATED` and removals every consumer gets; [`ScriptHost::stop_game`] unregisters it) and connects
//! `GetPropertyChangedSignal("Enabled")` on each script inside a service that runs scripts; it never connects to
//! parts or other instances. Property writes cost what they cost without a game. Each instance created or moved
//! (with its descendants, which a move marks too) costs one dirty-list entry, drained at the next dispatch with one
//! class test; each destroyed one costs a removed-list entry. No signal event is queued for any of them. A
//! destroyed script is reported by its `Enabled` watch, whose connection is dropped with it.
//!
//! # `require`
//!
//! `require(moduleScript)` ([`HostContext::require`], [`YieldReason::Require`]) runs the module's chunk once per
//! host (one host is one context), in a thread of its own whose script is the module; the module must return
//! exactly one value, which its backend keeps under a [`ModuleKey`]. Every later `require` returns that same value
//! (for Luau, the same table). Modules stay loaded until [`ScriptHost::stop_game`].
//!
//! - **A module that yields while loading** (`task.wait` at its top level) makes every requirer wait, as in Roblox:
//!   the first `require` starts the load, later ones queue behind it, and all resume in the order they came once
//!   the module returns.
//! - **Errors** in the module are reported once, as the module's, and every requirer (now and later) raises
//!   `Requested module experienced an error while loading: Module.Name:line: message`.
//! - **Cycles** raise in the requirer that would close the cycle:
//!   `Requested module was required recursively: Workspace.A requires Workspace.B requires Workspace.A`.
//! - **Languages:** a module runs in the backend of its file's extension, and only scripts of that language may
//!   require it; requiring across languages is a clear error (calls between languages come later, ADR 0003).
//!
//! # Callbacks and `BindableFunction`
//!
//! A callback member (`BindableFunction.OnInvoke`, declared with core's `CallbackDesc`) holds a script function:
//! [`HostContext::set_callback`] records it per (instance, callback) under a [`ListenerKey`](metallic_core::ListenerKey)
//! the backend stores the function under, owned by the script that assigned it. A method marked
//! `invokes_callback` (`Invoke`) yields [`YieldReason::InvokeCallback`]; the host runs the function in a new thread of
//! its owner with the arguments, and resumes the caller with its results, so the caller waits while the function
//! yields. An error in the function is raised in the caller (`Script.Name:line: message`), not reported on its own.
//!
//! **`Invoke` with no function assigned waits** until a script assigns one (as in Roblox), so scripts may start in
//! any order. It fails instead when the instance is destroyed while it waits. A function is released when it is
//! replaced or cleared, when its script is reaped, or when its instance is destroyed.
//!
//! # Reaping
//!
//! [`ScriptHost::reap`] stops a script: its threads leave the sleeper heap, the deferred queue and the signal
//! waiters (whose connections are disconnected), and are released with its listeners and its callback functions.
//! The host calls it itself while a game runs (see above). A thread that was answering an `Invoke` makes that
//! `Invoke` fail; one that was loading a module makes the module fail.
//!
//! # Connections dropped with their instances
//!
//! The data model drops the connections of a destroyed instance on its own, after delivering its last events
//! (`Destroying`, ...). After every dispatch that delivered all queued events, the host drains those reports
//! ([`DataModel::drain_dropped_connections`](metallic_core::DataModel::drain_dropped_connections)) and releases what
//! it kept for each: the listener record and the backend's function, so destroying connected instances leaks
//! nothing. **A thread in `signal:Wait()` on a destroyed instance** is resumed with the error
//! `the instance whose signal this thread was waiting on was destroyed`, raised where it waits (it may catch it).
//! Roblox leaves such a thread parked forever; that leaks, so the host does not.
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
//! The engine loop and RunService (M3) call [`ScriptHost::step`]; projects (M4) bring the real [`SourceLoader`];
//! client and server contexts (M10) split [`RunContext::Local`].

pub mod api_dump;
mod context;
mod deadline;
mod game;
mod host;
mod language;
mod output;
mod scheduler;

#[cfg(test)]
mod game_tests;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod scheduler_tests;
#[cfg(test)]
mod tests;

pub use api_dump::ApiDump;
pub use context::{Connection, HostContext, HostRequest};
pub use deadline::Deadline;
pub use game::{LoadError, MemorySources, RunContext, SourceLoader};
pub use host::{DEFAULT_RESUME_BUDGET, HostError, LanguageId, ScriptHost, ScriptRun, ThreadRef, ThreadState};
pub use language::{
    ChunkId, CompileError, ModuleKey, Resume, ScriptFailure, ScriptLanguage, SignalRef, ThreadId, YieldReason,
};
pub use output::{CollectOutput, OutputLevel, OutputLine, OutputSink, OutputSource};
pub use scheduler::{DEFAULT_PHASE_BUDGET, MAX_DEFER_DEPTH, Phase, StepStats};
