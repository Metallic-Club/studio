//! `ScriptHost`: owns the backends, the chunk cache, and every script's threads and listeners.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use metallic_core::{ConnectionId, DataModel, Delivery, DmError, Id, ListenerKey, ScriptValue};
use thiserror::Error;

use crate::context::{EarlyWait, connect_signal, defer_depth_message};
use crate::scheduler::{Budget, Deferred, SCHEDULER_SOURCE, Scheduler, Sleeper, Slot, Waiter, Wake, sanitize_seconds};
use crate::{
    ChunkId, CollectOutput, CompileError, Deadline, HostContext, HostRequest, MAX_DEFER_DEPTH, OutputSink,
    OutputSource, Phase, Resume, ScriptFailure, ScriptLanguage, SignalRef, StepStats, ThreadId, YieldReason,
};

/// The time one call into a backend may take before the thread is interrupted, unless
/// [`ScriptHost::set_resume_budget`] changes it. It is a backstop against runaway loops, not frame pacing: the
/// per-phase budget ([`ScriptHost::set_phase_budget`]) keeps frames on time.
pub const DEFAULT_RESUME_BUDGET: Duration = Duration::from_secs(1);

/// A backend registered with a host, by registration order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LanguageId(pub(crate) usize);

impl LanguageId {
    /// The registration index: 0 for the first backend registered.
    pub fn index(self) -> usize {
        self.0
    }
}

/// A thread anywhere in the host: which backend runs it, and that backend's handle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ThreadRef {
    /// The backend that runs it.
    pub language: LanguageId,
    /// The backend's handle.
    pub thread: ThreadId,
}

/// Where a live thread stands, as the host last saw it.
#[derive(Clone, Debug, PartialEq)]
pub enum ThreadState {
    /// Created and not resumed yet (a thread a backend asked the host to adopt, defer or delay).
    Created,
    /// Suspended, waiting to be resumed for this reason.
    Yielded(YieldReason),
}

/// The outcome of [`ScriptHost::run_script`]: the script's main thread and how its first resume ended.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptRun {
    /// The main thread. After [`Resume::Finished`] or [`Resume::Errored`] it is already released.
    pub thread: ThreadRef,
    /// How the first resume ended. An error has already been reported to the output.
    pub resume: Resume,
}

/// Why the host could not do what was asked. Script failures that the host could report and recover from are
/// reported to the output as well.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum HostError {
    /// The source path has no extension, so no language can be chosen.
    #[error("{path}: a script file needs an extension naming its language, such as `.luau`")]
    NoExtension {
        /// The script's source path.
        path: String,
    },
    /// No registered backend runs this extension.
    #[error("{path}: no script language runs `.{extension}` files")]
    UnknownExtension {
        /// The script's source path.
        path: String,
        /// The extension, lower-case and without the dot.
        extension: String,
    },
    /// Two backends claim the same extension.
    #[error("`.{extension}` files are already run by {language}")]
    ExtensionTaken {
        /// The extension, lower-case and without the dot.
        extension: String,
        /// The backend that registered it first.
        language: String,
    },
    /// The script's source did not compile.
    #[error(transparent)]
    Compile(#[from] CompileError),
    /// The backend could not create a thread.
    #[error("could not start the script: {0}")]
    Spawn(ScriptFailure),
    /// The script instance does not exist.
    #[error(transparent)]
    DataModel(#[from] DmError),
    /// The thread is not one the host tracks (it finished, failed or was reaped).
    #[error("the thread {0:?} is not running")]
    UnknownThread(ThreadRef),
}

/// A chunk cache key: one compiled chunk per backend, source path and content.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct ChunkKey {
    language: LanguageId,
    path: String,
    /// A hash of the source text, plus its length to make a collision even less likely.
    content: (u64, usize),
}

/// What the host knows about one started script.
#[derive(Debug)]
pub(crate) struct ScriptRecord {
    /// The backend that runs it.
    pub(crate) language: LanguageId,
    /// The instance's full name when it started.
    pub(crate) name: Arc<str>,
    /// `path#uid`.
    pub(crate) chunk_name: Arc<str>,
    /// Every live thread it created.
    pub(crate) threads: HashSet<ThreadId>,
    /// Every listener it connected.
    pub(crate) listeners: HashSet<ListenerKey>,
}

/// One live thread.
#[derive(Debug)]
struct ThreadRecord {
    /// The script that owns it.
    script: Id,
    state: ThreadState,
    /// What the scheduler will resume it for.
    slot: Slot,
    /// What the scheduler will resume it with.
    wake: Wake,
}

/// One listener: who owns it and its connection in the data model.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ListenerRecord {
    pub(crate) script: Id,
    pub(crate) language: LanguageId,
    pub(crate) connection: ConnectionId,
    /// A `Once` connection: disconnected at its first delivery.
    pub(crate) once: bool,
}

/// Every listener, and the next key to hand out.
#[derive(Debug, Default)]
pub(crate) struct Listeners {
    pub(crate) map: HashMap<ListenerKey, ListenerRecord>,
    next: u64,
}

impl Listeners {
    /// A key no listener of this host has used.
    pub(crate) fn next_key(&mut self) -> ListenerKey {
        self.next += 1;
        ListenerKey(self.next)
    }
}

/// The language-neutral script host. See the crate docs for the design.
///
/// `O` is where output and errors go: [`CollectOutput`] by default, `Box<dyn OutputSink>` when the sink is chosen at
/// run time.
pub struct ScriptHost<O: OutputSink = CollectOutput> {
    /// The backends, by [`LanguageId`].
    languages: Vec<Box<dyn ScriptLanguage>>,
    /// Lower-case extension (no dot) to backend.
    by_extension: HashMap<String, LanguageId>,
    /// Compiled chunks, so a file is compiled once per host.
    chunks: HashMap<ChunkKey, ChunkId>,
    /// Started scripts.
    scripts: HashMap<Id, ScriptRecord>,
    /// Live threads.
    threads: HashMap<ThreadRef, ThreadRecord>,
    /// Live listeners.
    listeners: Listeners,
    /// Requests from the current call; empty between calls.
    pending: Vec<HostRequest>,
    /// Deliveries carried over from a step that ran out of budget, then the latest drain. Reused, so a drain
    /// allocates nothing once it has grown.
    deliveries: Vec<Delivery>,
    /// Sleepers, deferred threads, signal waiters and game time.
    sched: Scheduler,
    /// Waiter connections the current call made early (see [`HostContext::spawned`]); empty between calls.
    early_waits: Vec<EarlyWait>,
    output: O,
    resume_budget: Duration,
    memory_limit: Option<usize>,
}

impl ScriptHost<CollectOutput> {
    /// A host with no backends whose output is collected in memory.
    pub fn new() -> Self {
        Self::with_output(CollectOutput::new())
    }
}

impl Default for ScriptHost<CollectOutput> {
    fn default() -> Self {
        Self::new()
    }
}

impl<O: OutputSink> ScriptHost<O> {
    /// A host with no backends that writes output and errors to `output`.
    pub fn with_output(output: O) -> Self {
        Self {
            languages: Vec::new(),
            by_extension: HashMap::new(),
            chunks: HashMap::new(),
            scripts: HashMap::new(),
            threads: HashMap::new(),
            listeners: Listeners::default(),
            pending: Vec::new(),
            deliveries: Vec::new(),
            sched: Scheduler::default(),
            early_waits: Vec::new(),
            output,
            resume_budget: DEFAULT_RESUME_BUDGET,
            memory_limit: None,
        }
    }

    /// Adds a backend for every extension it lists. Refuses an extension another backend already runs, and then
    /// adds nothing. The host's memory limit applies to it at once.
    pub fn register_language(&mut self, mut language: Box<dyn ScriptLanguage>) -> Result<LanguageId, HostError> {
        let id = LanguageId(self.languages.len());
        let extensions: Vec<String> = language.extensions().iter().map(|ext| normalize(ext)).collect();
        for extension in &extensions {
            if let Some(taken) = self.by_extension.get(extension) {
                let language = self
                    .languages
                    .get(taken.0)
                    .map(|l| l.name().to_owned())
                    .unwrap_or_default();
                return Err(HostError::ExtensionTaken {
                    extension: extension.clone(),
                    language,
                });
            }
        }
        for extension in extensions {
            self.by_extension.insert(extension, id);
        }
        language.set_memory_limit(self.memory_limit);
        self.languages.push(language);
        Ok(id)
    }

    /// The backend that runs files at `path`, chosen by its extension.
    pub fn language_for_path(&self, path: &str) -> Result<LanguageId, HostError> {
        let Some(extension) = extension(path) else {
            return Err(HostError::NoExtension { path: path.to_owned() });
        };
        let extension = normalize(extension);
        match self.by_extension.get(&extension) {
            Some(id) => Ok(*id),
            None => Err(HostError::UnknownExtension {
                path: path.to_owned(),
                extension,
            }),
        }
    }

    /// The registered backend `id`.
    pub fn language(&self, id: LanguageId) -> Option<&dyn ScriptLanguage> {
        self.languages.get(id.0).map(|l| &**l)
    }

    /// Starts the script instance `script` from the file at `source_path` with the text `source`: picks the
    /// backend by extension, compiles (or reuses the cached chunk), creates the main thread and resumes it once.
    ///
    /// A script that is already running is reaped first, so running it again restarts it. Every failure is
    /// reported to the output with the script's name and line, and returned; nothing else is affected.
    pub fn run_script(
        &mut self,
        dm: &mut DataModel,
        script: Id,
        source_path: &str,
        source: &str,
    ) -> Result<ScriptRun, HostError> {
        let name: Arc<str> = dm.full_name(script)?.into();
        let reported = |host: &mut Self, chunk: &str, line: Option<u32>, error: HostError| {
            let from = OutputSource {
                script: &name,
                chunk,
                line,
            };
            host.output.error(&from, &error.to_string());
            error
        };
        let language = match self.language_for_path(source_path) {
            Ok(language) => language,
            Err(error) => return Err(reported(self, source_path, None, error)),
        };
        self.reap(dm, script);
        let chunk = match self.chunk(language, source_path, source) {
            Ok(chunk) => chunk,
            Err(error) => {
                let line = Some(error.line);
                return Err(reported(self, source_path, line, HostError::Compile(error)));
            }
        };
        let chunk_name: Arc<str> = format!("{source_path}#{}.{}", script.index(), script.generation()).into();
        self.scripts.insert(
            script,
            ScriptRecord {
                language,
                name: Arc::clone(&name),
                chunk_name: Arc::clone(&chunk_name),
                threads: HashSet::new(),
                listeners: HashSet::new(),
            },
        );
        let spawned = self.call(dm, script, |backend, ctx| backend.spawn(chunk, ctx));
        let thread = match spawned {
            Some(Ok(thread)) => thread,
            Some(Err(failure)) => {
                self.report(script, &failure);
                self.reap(dm, script);
                return Err(HostError::Spawn(failure));
            }
            // The record was inserted just above and the language exists, so this cannot happen.
            None => return Err(HostError::Spawn(ScriptFailure::runtime("the script vanished", None))),
        };
        let thread = self.adopt(script, language, thread);
        let resume = self.resume_thread(dm, thread, &[])?;
        Ok(ScriptRun { thread, resume })
    }

    /// Resumes a suspended thread now with `args` (the values its yield returns), outside the scheduler. Whatever
    /// the thread was scheduled for (a sleep, the deferred queue, a signal) is abandoned first. A failure is
    /// reported to the output and ends only this thread.
    pub fn resume_thread(
        &mut self,
        dm: &mut DataModel,
        thread: ThreadRef,
        args: &[ScriptValue],
    ) -> Result<Resume, HostError> {
        let Some(record) = self.threads.get(&thread) else {
            return Err(HostError::UnknownThread(thread));
        };
        let script = record.script;
        self.unschedule(dm, thread);
        self.resume_now(dm, script, thread, args)
            .ok_or(HostError::UnknownThread(thread))
    }

    /// Runs one frame phase: the engine loop calls it once per phase per frame, in the order of [`Phase::FRAME`],
    /// with the game time `now` in seconds (it never goes back). The crate docs give the full order; in short:
    ///
    /// 1. at [`Phase::Heartbeat`] only, the due sleepers (`task.wait`, `task.delay`), earliest wake time first and
    ///    in sleep order among equal times;
    /// 2. the deferred queue, until it is empty;
    /// 3. the signal events queued so far, one listener thread per delivery ([`ScriptHost::dispatch_signals`]);
    /// 4. the deferred queue again.
    ///
    /// The phase may spend [`ScriptHost::phase_budget`] on this. Past it, the remaining work stays queued in the
    /// same order and runs first at the next step that runs that kind of work, and a warning goes to the output
    /// (once per frame at most).
    pub fn step(&mut self, dm: &mut DataModel, phase: Phase, now: f64) -> StepStats {
        self.sched.advance(now);
        let mut budget = Budget::new(self.sched.phase_budget);
        let mut resumed = 0;
        if phase == Phase::Heartbeat {
            self.sched.frame += 1;
            resumed += self.resume_sleepers(dm, &mut budget);
        }
        resumed += self.flush_deferred(dm, &mut budget);
        resumed += self.dispatch(dm, &mut budget);
        resumed += self.flush_deferred(dm, &mut budget);
        self.finish_step(phase, resumed, &budget)
    }

    /// The resumption point of waiting threads alone: steps 1 and 2 of a [`Phase::Heartbeat`] [`step`], with no
    /// signal dispatch. For engine loops that dispatch signals elsewhere, and for tests.
    ///
    /// [`step`]: ScriptHost::step
    pub fn resume_waiting(&mut self, dm: &mut DataModel, now: f64) -> StepStats {
        self.sched.advance(now);
        self.sched.frame += 1;
        let mut budget = Budget::new(self.sched.phase_budget);
        let mut resumed = self.resume_sleepers(dm, &mut budget);
        resumed += self.flush_deferred(dm, &mut budget);
        self.finish_step(Phase::Heartbeat, resumed, &budget)
    }

    /// Delivers every queued signal event, with no time budget: deliveries a budgeted [`step`] carried over first,
    /// then [`DataModel::drain_deliveries`]. Each delivery to a script listener runs in a new thread of its backend,
    /// with the event's arguments; a `Once` listener is disconnected first. A delivery to a thread in
    /// `signal:Wait()` resumes that thread with the arguments. A listener that fails is reported and does not stop
    /// the others; one that yields is scheduled like any thread. Returns how many threads ran.
    ///
    /// It does not run the deferred work the listeners queue; [`step`] does, right after. Events the listeners fire
    /// while running are delivered by the next call, as the data model's deferred delivery prescribes.
    ///
    /// [`step`]: ScriptHost::step
    pub fn dispatch_signals(&mut self, dm: &mut DataModel) -> usize {
        self.dispatch(dm, &mut Budget::unlimited())
    }

    /// Stops a thread for good (`task.cancel` from the engine's side): it leaves whatever it was scheduled for and
    /// is released. Returns `false` when the host does not track it.
    pub fn cancel(&mut self, dm: &mut DataModel, thread: ThreadRef) -> bool {
        if !self.threads.contains_key(&thread) {
            return false;
        }
        self.unschedule(dm, thread);
        self.forget(dm, thread);
        true
    }

    /// Stops everything `script` started: releases its threads (unscheduling its sleepers, deferred threads and
    /// signal waiters, whose connections are disconnected) and disconnects and releases its listeners. The engine
    /// calls it when a script is destroyed, disabled or moved out of a service that runs scripts. Returns `false`
    /// when the script was not running.
    pub fn reap(&mut self, dm: &mut DataModel, script: Id) -> bool {
        let Some(record) = self.scripts.remove(&script) else {
            return false;
        };
        let backend = self.languages.get_mut(record.language.0);
        let mut backend = backend.map(|b| &mut **b);
        let mut unscheduled = false;
        for thread in record.threads {
            let thread_ref = ThreadRef {
                language: record.language,
                thread,
            };
            if let Some(gone) = self.threads.remove(&thread_ref) {
                unscheduled |= gone.slot != Slot::Idle;
                drop_slot(&mut self.sched, dm, gone.slot);
            }
            if let Some(backend) = backend.as_deref_mut() {
                backend.release_thread(thread);
            }
        }
        if unscheduled {
            // Their queue entries are stale now; drop them so the queues hold only live work.
            let threads = &self.threads;
            self.sched
                .sleepers
                .retain(|s| is_current(threads, s.thread, Slot::Sleeping(s.seq)));
            self.sched
                .deferred
                .retain(|d| is_current(threads, d.thread, Slot::Deferred(d.seq)));
        }
        for key in record.listeners {
            // A key another script already disconnected (`HostContext::disconnect`) is gone from the table, and
            // its backend has dropped the function.
            let Some(listener) = self.listeners.map.remove(&key) else {
                continue;
            };
            dm.disconnect(listener.connection);
            if let Some(backend) = backend.as_deref_mut() {
                backend.release_listener(key);
            }
        }
        true
    }

    /// Whether `script` has been started and not reaped.
    pub fn is_running(&self, script: Id) -> bool {
        self.scripts.contains_key(&script)
    }

    /// The live threads of `script`.
    pub fn threads_of(&self, script: Id) -> impl Iterator<Item = ThreadRef> + '_ {
        self.scripts.get(&script).into_iter().flat_map(|record| {
            record.threads.iter().map(move |&thread| ThreadRef {
                language: record.language,
                thread,
            })
        })
    }

    /// Where a live thread stands, or `None` when the host does not track it.
    pub fn thread_state(&self, thread: ThreadRef) -> Option<&ThreadState> {
        self.threads.get(&thread).map(|record| &record.state)
    }

    /// How many threads are alive in every backend.
    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }

    /// How many listeners are connected, across every script.
    pub fn listener_count(&self) -> usize {
        self.listeners.map.len()
    }

    /// The time budget of one call into a backend.
    pub fn resume_budget(&self) -> Duration {
        self.resume_budget
    }

    /// Sets the time budget of one call into a backend (see [`DEFAULT_RESUME_BUDGET`]).
    pub fn set_resume_budget(&mut self, budget: Duration) {
        self.resume_budget = budget;
    }

    /// The time one [`step`](ScriptHost::step) may spend resuming threads.
    pub fn phase_budget(&self) -> Duration {
        self.sched.phase_budget
    }

    /// Sets the time one [`step`](ScriptHost::step) may spend resuming threads (see
    /// [`DEFAULT_PHASE_BUDGET`](crate::DEFAULT_PHASE_BUDGET); `Duration::MAX` is no limit). A step always resumes
    /// at least one thread when one is due, so even a zero budget makes progress, one thread per step.
    pub fn set_phase_budget(&mut self, budget: Duration) {
        self.sched.phase_budget = budget;
    }

    /// The game time of the last step, in seconds.
    pub fn now(&self) -> f64 {
        self.sched.now
    }

    /// How many threads sleep in `task.wait` or wait for their `task.delay`.
    pub fn sleeper_count(&self) -> usize {
        self.sched.live_sleepers
    }

    /// How many threads wait in the deferred queue.
    pub fn deferred_count(&self) -> usize {
        self.sched.live_deferred
    }

    /// How many threads wait on a signal (`signal:Wait()`).
    pub fn waiter_count(&self) -> usize {
        self.sched.waiters.len()
    }

    /// Caps every backend's memory at `bytes` (`None` removes the cap), now and for backends registered later.
    pub fn set_memory_limit(&mut self, bytes: Option<usize>) {
        self.memory_limit = bytes;
        for language in &mut self.languages {
            language.set_memory_limit(bytes);
        }
    }

    /// The bytes every backend uses together.
    pub fn memory_used(&self) -> usize {
        self.languages.iter().map(|l| l.memory_used()).sum()
    }

    /// The output sink.
    pub fn output(&self) -> &O {
        &self.output
    }

    /// The output sink, to drain or reconfigure it.
    pub fn output_mut(&mut self) -> &mut O {
        &mut self.output
    }

    /// The cached chunk for this file, compiling it on a miss.
    fn chunk(&mut self, language: LanguageId, path: &str, source: &str) -> Result<ChunkId, CompileError> {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        let key = ChunkKey {
            language,
            path: path.to_owned(),
            content: (hasher.finish(), source.len()),
        };
        if let Some(chunk) = self.chunks.get(&key) {
            return Ok(*chunk);
        }
        let Some(backend) = self.languages.get_mut(language.0) else {
            return Err(CompileError {
                chunk: path.to_owned(),
                line: 1,
                column: None,
                message: "the language is not registered".to_owned(),
            });
        };
        let chunk = backend.compile(source, path)?;
        self.chunks.insert(key, chunk);
        Ok(chunk)
    }

    /// Runs `f` with `script`'s backend and a fresh context, then handles what the backend requested. `None` when
    /// the script is not running.
    fn call<R>(
        &mut self,
        dm: &mut DataModel,
        script: Id,
        f: impl FnOnce(&mut dyn ScriptLanguage, &mut HostContext<'_>) -> R,
    ) -> Option<R> {
        let record = self.scripts.get_mut(&script)?;
        let language = record.language;
        let backend = self.languages.get_mut(language.0)?;
        let result = {
            let mut ctx = HostContext::new(
                dm,
                Deadline::after(self.resume_budget),
                script,
                language,
                record,
                &mut self.listeners,
                &mut self.pending,
                &mut self.output,
                self.sched.depth,
                &mut self.early_waits,
            );
            f(&mut **backend, &mut ctx)
        };
        self.handle_requests(dm, script, language);
        Some(result)
    }

    /// Resumes `thread` of `script` with `args` and settles how it ended. `None` when the script is not running.
    fn resume_now(
        &mut self,
        dm: &mut DataModel,
        script: Id,
        thread: ThreadRef,
        args: &[ScriptValue],
    ) -> Option<Resume> {
        let resume = self.call(dm, script, |backend, ctx| backend.resume(thread.thread, args, ctx))?;
        self.settle(dm, script, thread, &resume);
        Some(resume)
    }

    /// Resumes a thread the scheduler just took off a queue, with what it was scheduled to get.
    fn resume_scheduled(&mut self, dm: &mut DataModel, thread: ThreadRef) {
        let Some(record) = self.threads.get_mut(&thread) else {
            return;
        };
        record.slot = Slot::Idle;
        let wake = std::mem::replace(&mut record.wake, Wake::Nothing);
        let script = record.script;
        match wake {
            Wake::Nothing => self.resume_now(dm, script, thread, &[]),
            Wake::Elapsed { since, at_least } => {
                let elapsed = (self.sched.now - since).max(at_least);
                self.resume_now(dm, script, thread, &[ScriptValue::Number(elapsed)])
            }
            Wake::Args(args) => self.resume_now(dm, script, thread, &args),
        };
    }

    /// Resumes the due sleepers, earliest wake time first, then in sleep order. Threads that went to sleep during
    /// this pass wait for the next one, so `task.wait(0)` always lasts until the next `Heartbeat`.
    fn resume_sleepers(&mut self, dm: &mut DataModel, budget: &mut Budget) -> usize {
        let first_new = self.sched.peek_ticket();
        let now = self.sched.now;
        let mut ran = 0;
        // Every sleeper scheduled during the pass has a wake time of at least `now` and a ticket of at least
        // `first_new`, so once the top is one of them, no older due sleeper is left behind it.
        while let Some(&top) = self.sched.sleepers.peek() {
            if top.wake > now || top.seq >= first_new {
                break;
            }
            if !is_current(&self.threads, top.thread, Slot::Sleeping(top.seq)) {
                self.sched.sleepers.pop();
                continue;
            }
            if !budget.next() {
                break;
            }
            self.sched.sleepers.pop();
            self.sched.live_sleepers = self.sched.live_sleepers.saturating_sub(1);
            self.resume_scheduled(dm, top.thread);
            ran += 1;
        }
        ran
    }

    /// Resumes deferred threads in the order they were deferred, including the ones they defer, until the queue is
    /// empty or the budget runs out.
    fn flush_deferred(&mut self, dm: &mut DataModel, budget: &mut Budget) -> usize {
        let mut ran = 0;
        while let Some(&entry) = self.sched.deferred.front() {
            if !is_current(&self.threads, entry.thread, Slot::Deferred(entry.seq)) {
                self.sched.deferred.pop_front();
                continue;
            }
            if !budget.next() {
                break;
            }
            self.sched.deferred.pop_front();
            self.sched.live_deferred = self.sched.live_deferred.saturating_sub(1);
            let outer = std::mem::replace(&mut self.sched.depth, entry.depth);
            self.resume_scheduled(dm, entry.thread);
            self.sched.depth = outer;
            ran += 1;
        }
        ran
    }

    /// Delivers carried-over deliveries, then a new drain, while the budget lasts. What is left stays for the next
    /// call, in order.
    fn dispatch(&mut self, dm: &mut DataModel, budget: &mut Budget) -> usize {
        let mut deliveries = std::mem::take(&mut self.deliveries);
        dm.drain_deliveries(&mut deliveries);
        let mut ran = 0;
        let mut done = 0;
        while let Some(delivery) = deliveries.get(done) {
            let key = delivery.listener;
            // A listener reaped, disconnected or already fired (`Once`, `Wait`) since the event fired receives
            // nothing.
            if let Some(waiter) = self.sched.waiters.get(&key).copied() {
                if waiter.connection == delivery.connection {
                    if !budget.next() {
                        break;
                    }
                    self.sched.waiters.remove(&key);
                    dm.disconnect(waiter.connection);
                    if let Some(record) = self.threads.get_mut(&waiter.thread) {
                        record.slot = Slot::Idle;
                        record.wake = Wake::Nothing;
                        let script = record.script;
                        self.resume_now(dm, script, waiter.thread, &delivery.args);
                        ran += 1;
                    }
                }
            } else if let Some(listener) = self.listeners.map.get(&key).copied()
                && listener.connection == delivery.connection
            {
                if !budget.next() {
                    break;
                }
                if self.run_listener(dm, key, listener, &delivery.args) {
                    ran += 1;
                }
            }
            done += 1;
        }
        deliveries.drain(..done);
        self.deliveries = deliveries;
        ran
    }

    /// Runs one delivery to a script listener in a new thread. A `Once` listener is disconnected first and released
    /// once its thread exists. Returns whether the thread ran.
    fn run_listener(
        &mut self,
        dm: &mut DataModel,
        key: ListenerKey,
        listener: ListenerRecord,
        args: &[ScriptValue],
    ) -> bool {
        if listener.once {
            self.listeners.map.remove(&key);
            if let Some(record) = self.scripts.get_mut(&listener.script) {
                record.listeners.remove(&key);
            }
            dm.disconnect(listener.connection);
        }
        let spawned = self.call(dm, listener.script, |backend, ctx| backend.spawn_listener(key, ctx));
        if listener.once
            && let Some(backend) = self.languages.get_mut(listener.language.0)
        {
            backend.release_listener(key);
        }
        match spawned {
            Some(Ok(thread)) => {
                let thread = self.adopt(listener.script, listener.language, thread);
                self.resume_now(dm, listener.script, thread, args);
                true
            }
            Some(Err(failure)) => {
                self.report(listener.script, &failure);
                false
            }
            None => false,
        }
    }

    /// Warns about a step that ran out of budget (once per frame), and sums the step up.
    fn finish_step(&mut self, phase: Phase, resumed: usize, budget: &Budget) -> StepStats {
        if budget.exhausted && self.sched.warned_frame != Some(self.sched.frame) {
            self.sched.warned_frame = Some(self.sched.frame);
            let from = OutputSource {
                script: SCHEDULER_SOURCE,
                chunk: SCHEDULER_SOURCE,
                line: None,
            };
            let message = format!(
                "{} used up its {:?} script budget; the threads still due run next frame",
                phase.name(),
                self.sched.phase_budget
            );
            self.output.warn(&from, &message);
        }
        StepStats {
            resumed,
            carried_over: budget.exhausted,
        }
    }

    /// Handles the requests the last call left, in order.
    fn handle_requests(&mut self, dm: &mut DataModel, script: Id, language: LanguageId) {
        let mut pending = std::mem::take(&mut self.pending);
        for request in pending.drain(..) {
            match request {
                HostRequest::AdoptThread(thread) => {
                    if !self.threads.contains_key(&ThreadRef { language, thread }) {
                        self.adopt(script, language, thread);
                    }
                }
                HostRequest::Spawned { thread, resume } => {
                    let (thread, owner) = self.claim(dm, script, language, thread);
                    self.settle(dm, owner, thread, &resume);
                }
                HostRequest::Defer { thread, args } => {
                    let (thread, owner) = self.claim(dm, script, language, thread);
                    self.schedule_defer(dm, owner, thread, wake_with(args));
                }
                HostRequest::Delay { thread, seconds, args } => {
                    let (thread, _) = self.claim(dm, script, language, thread);
                    self.schedule_sleep(thread, seconds, wake_with(args));
                }
                HostRequest::Cancel(thread) => {
                    self.cancel(dm, ThreadRef { language, thread });
                }
            }
        }
        self.pending = pending;
        // An early connection whose thread did not end up waiting (cancelled in the same call) is not needed.
        for wait in self.early_waits.drain(..) {
            dm.disconnect(wait.connection);
        }
    }

    /// The thread a request names, and its owner: a tracked thread leaves what it was scheduled for; an unknown one
    /// is adopted as `script`'s.
    fn claim(&mut self, dm: &mut DataModel, script: Id, language: LanguageId, thread: ThreadId) -> (ThreadRef, Id) {
        let thread_ref = ThreadRef { language, thread };
        match self.threads.get(&thread_ref) {
            Some(record) => {
                let owner = record.script;
                self.unschedule(dm, thread_ref);
                (thread_ref, owner)
            }
            None => (self.adopt(script, language, thread), script),
        }
    }

    /// Records `thread` as a live, not yet resumed thread of `script`.
    fn adopt(&mut self, script: Id, language: LanguageId, thread: ThreadId) -> ThreadRef {
        let thread_ref = ThreadRef { language, thread };
        if let Some(record) = self.scripts.get_mut(&script) {
            record.threads.insert(thread);
        }
        self.threads.insert(
            thread_ref,
            ThreadRecord {
                script,
                state: ThreadState::Created,
                slot: Slot::Idle,
                wake: Wake::Nothing,
            },
        );
        thread_ref
    }

    /// Updates the thread after a resume: a finished or failed thread is released (and a failure reported), a
    /// yielded one remembers why and is scheduled for it.
    fn settle(&mut self, dm: &mut DataModel, script: Id, thread: ThreadRef, resume: &Resume) {
        let reason = match resume {
            Resume::Yielded(reason) => reason,
            Resume::Errored(failure) => {
                self.report(script, failure);
                self.forget(dm, thread);
                return;
            }
            Resume::Finished(_) => {
                self.forget(dm, thread);
                return;
            }
        };
        let Some(record) = self.threads.get_mut(&thread) else {
            // Cancelled or reaped while it ran: the backend must not keep it.
            if let Some(backend) = self.languages.get_mut(thread.language.0) {
                backend.release_thread(thread.thread);
            }
            return;
        };
        record.state = ThreadState::Yielded(reason.clone());
        match *reason {
            YieldReason::Wait { seconds } => {
                self.unschedule(dm, thread);
                let wake = Wake::Elapsed {
                    since: self.sched.now,
                    at_least: sanitize_seconds(seconds),
                };
                self.schedule_sleep(thread, seconds, wake);
            }
            YieldReason::Defer => {
                self.unschedule(dm, thread);
                self.schedule_defer(dm, script, thread, Wake::Nothing);
            }
            YieldReason::WaitSignal { target, signal } => {
                self.unschedule(dm, thread);
                self.schedule_signal(dm, script, thread, target, signal);
            }
            // A thread may have deferred or delayed itself before yielding to its own caller; that stands.
            YieldReason::Other => {}
        }
    }

    /// Puts an unscheduled thread to sleep for `seconds` of game time.
    fn schedule_sleep(&mut self, thread: ThreadRef, seconds: f64, wake: Wake) {
        let Some(record) = self.threads.get_mut(&thread) else {
            return;
        };
        let seq = self.sched.ticket();
        record.slot = Slot::Sleeping(seq);
        record.wake = wake;
        self.sched.sleepers.push(Sleeper {
            wake: self.sched.now + sanitize_seconds(seconds),
            seq,
            thread,
        });
        self.sched.live_sleepers += 1;
    }

    /// Queues an unscheduled thread at the end of the deferred queue, one level deeper than the running code. Past
    /// [`MAX_DEFER_DEPTH`] the thread errors instead.
    fn schedule_defer(&mut self, dm: &mut DataModel, script: Id, thread: ThreadRef, wake: Wake) {
        let depth = self.sched.depth + 1;
        if depth > MAX_DEFER_DEPTH {
            self.report(script, &ScriptFailure::runtime(defer_depth_message(), None));
            self.forget(dm, thread);
            return;
        }
        let Some(record) = self.threads.get_mut(&thread) else {
            return;
        };
        let seq = self.sched.ticket();
        record.slot = Slot::Deferred(seq);
        record.wake = wake;
        self.sched.deferred.push_back(Deferred { seq, thread, depth });
        self.sched.live_deferred += 1;
    }

    /// Parks an unscheduled thread until the next event of `signal` on `target`, through a host-internal
    /// connection. A signal that cannot be connected (the instance is gone) fails the thread.
    fn schedule_signal(&mut self, dm: &mut DataModel, script: Id, thread: ThreadRef, target: Id, signal: SignalRef) {
        let early = self.early_waits.iter().position(|w| w.thread == thread.thread);
        let connected = match early.map(|at| self.early_waits.swap_remove(at)) {
            Some(early) => Ok((early.listener, early.connection)),
            None => {
                let key = self.listeners.next_key();
                connect_signal(dm, target, signal, key).map(|connection| (key, connection))
            }
        };
        let (key, connection) = match connected {
            Ok(connected) => connected,
            Err(error) => {
                self.report(script, &ScriptFailure::runtime(error.to_string(), None));
                self.forget(dm, thread);
                return;
            }
        };
        match self.threads.get_mut(&thread) {
            Some(record) => {
                record.slot = Slot::Signal(key);
                self.sched.waiters.insert(key, Waiter { thread, connection });
            }
            None => {
                dm.disconnect(connection);
            }
        }
    }

    /// Takes a thread out of whatever it is scheduled for. Queue entries go stale and are skipped; a signal waiter
    /// is disconnected.
    fn unschedule(&mut self, dm: &mut DataModel, thread: ThreadRef) {
        let Some(record) = self.threads.get_mut(&thread) else {
            return;
        };
        let slot = std::mem::replace(&mut record.slot, Slot::Idle);
        record.wake = Wake::Nothing;
        if slot == Slot::Idle {
            return;
        }
        drop_slot(&mut self.sched, dm, slot);
        // Cancelling many sleepers leaves stale entries behind; rebuild before they outnumber the live ones.
        let threads = &self.threads;
        if self.sched.sleepers_need_compacting() {
            self.sched
                .sleepers
                .retain(|s| is_current(threads, s.thread, Slot::Sleeping(s.seq)));
        }
        if self.sched.deferred_need_compacting() {
            self.sched
                .deferred
                .retain(|d| is_current(threads, d.thread, Slot::Deferred(d.seq)));
        }
    }

    /// Stops tracking a thread and releases it in its backend.
    fn forget(&mut self, dm: &mut DataModel, thread: ThreadRef) {
        if let Some(record) = self.threads.remove(&thread) {
            drop_slot(&mut self.sched, dm, record.slot);
            if let Some(script) = self.scripts.get_mut(&record.script) {
                script.threads.remove(&thread.thread);
            }
        }
        if let Some(backend) = self.languages.get_mut(thread.language.0) {
            backend.release_thread(thread.thread);
        }
    }

    /// Reports a failure of `script` to the output.
    fn report(&mut self, script: Id, failure: &ScriptFailure) {
        let Some(record) = self.scripts.get(&script) else {
            return;
        };
        let from = OutputSource {
            script: &record.name,
            chunk: &record.chunk_name,
            line: failure.line(),
        };
        let message = match failure.traceback() {
            Some(traceback) => format!("{failure}\n{traceback}"),
            None => failure.to_string(),
        };
        self.output.error(&from, &message);
    }
}

/// The extension of the file at `path`, without the dot, if its file name has one.
fn extension(path: &str) -> Option<&str> {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match file.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => Some(extension),
        _ => None,
    }
}

/// Whether `thread` is still scheduled with `slot`, so the queue entry naming that slot is current.
fn is_current(threads: &HashMap<ThreadRef, ThreadRecord>, thread: ThreadRef, slot: Slot) -> bool {
    threads.get(&thread).is_some_and(|record| record.slot == slot)
}

/// Updates the scheduler for a thread leaving `slot`: the live counts drop, and a signal waiter's connection is
/// disconnected.
fn drop_slot(sched: &mut Scheduler, dm: &mut DataModel, slot: Slot) {
    match slot {
        Slot::Idle => {}
        Slot::Sleeping(_) => sched.live_sleepers = sched.live_sleepers.saturating_sub(1),
        Slot::Deferred(_) => sched.live_deferred = sched.live_deferred.saturating_sub(1),
        Slot::Signal(key) => {
            if let Some(waiter) = sched.waiters.remove(&key) {
                dm.disconnect(waiter.connection);
            }
        }
    }
}

/// What a `task.defer` or `task.delay` thread is resumed with.
fn wake_with(args: Vec<ScriptValue>) -> Wake {
    if args.is_empty() {
        Wake::Nothing
    } else {
        Wake::Args(args)
    }
}

/// An extension as the host stores it: lower-case, without a leading dot.
fn normalize(extension: &str) -> String {
    extension.trim_start_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_come_from_the_file_name() {
        assert_eq!(extension("Scripts/Door.luau"), Some("luau"));
        assert_eq!(extension("Scripts\\Door.client.luau"), Some("luau"));
        assert_eq!(extension("Scripts.v2/Door"), None);
        assert_eq!(extension("Scripts/.luau"), None);
        assert_eq!(extension("Door."), None);
        assert_eq!(normalize(".LUAU"), "luau");
    }
}
