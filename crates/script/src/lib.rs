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
//! by [`ScriptHost::dispatch_signals`], which runs each listener in a new thread of its backend.
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
//! The scheduler (#45) adds the `task` library, resumption order and per-phase budgets on top of
//! [`ScriptHost::resume_thread`], [`ThreadState`] and [`HostRequest`]; the script classes (#47) feed
//! [`ScriptHost::run_script`] automatically.

mod context;
mod deadline;
mod host;
mod language;
mod output;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub use context::{Connection, HostContext, HostRequest};
pub use deadline::Deadline;
pub use host::{DEFAULT_RESUME_BUDGET, HostError, LanguageId, ScriptHost, ScriptRun, ThreadRef, ThreadState};
pub use language::{ChunkId, CompileError, Resume, ScriptFailure, ScriptLanguage, ThreadId, YieldReason};
pub use output::{CollectOutput, OutputLevel, OutputLine, OutputSink, OutputSource};
