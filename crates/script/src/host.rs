//! `ScriptHost`: owns the backends, the chunk cache, and every script's threads and listeners.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use metallic_core::{ConnectionId, DataModel, Delivery, DmError, Id, ListenerKey, ScriptValue};
use thiserror::Error;

use crate::{
    ChunkId, CollectOutput, CompileError, Deadline, HostContext, HostRequest, OutputSink, OutputSource, Resume,
    ScriptFailure, ScriptLanguage, ThreadId, YieldReason,
};

/// The time one call into a backend may take before the thread is interrupted, unless
/// [`ScriptHost::set_resume_budget`] changes it. It is a backstop against runaway loops, not frame pacing: the
/// scheduler's per-phase budget (#45) keeps frames on time.
pub const DEFAULT_RESUME_BUDGET: Duration = Duration::from_secs(1);

/// A backend registered with a host, by registration order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LanguageId(usize);

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
    /// Created and not resumed yet (a thread a backend asked the host to adopt).
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
}

/// One listener: who owns it and its connection in the data model.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ListenerRecord {
    pub(crate) script: Id,
    pub(crate) language: LanguageId,
    pub(crate) connection: ConnectionId,
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
    /// Reused by [`ScriptHost::dispatch_signals`], so a drain allocates nothing once it has grown.
    deliveries: Vec<Delivery>,
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

    /// Resumes a suspended thread with `args` (the values its yield returns). The scheduler (#45) drives this; a
    /// failure is reported to the output and ends only this thread.
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
        let resume = self
            .call(dm, script, |backend, ctx| backend.resume(thread.thread, args, ctx))
            .ok_or(HostError::UnknownThread(thread))?;
        self.settle(script, thread, &resume);
        Ok(resume)
    }

    /// Delivers every queued signal event: drains [`DataModel::drain_deliveries`] and runs each listener in a new
    /// thread of its backend, with the event's arguments and a fresh deadline. A listener that fails is reported
    /// and does not stop the others; one that yields stays suspended for the scheduler. Returns how many listener
    /// threads ran.
    ///
    /// Events the listeners fire while running are delivered by the next call, as the data model's deferred
    /// delivery prescribes. The scheduler (#45) decides when in the frame this runs.
    pub fn dispatch_signals(&mut self, dm: &mut DataModel) -> usize {
        let mut deliveries = std::mem::take(&mut self.deliveries);
        dm.drain_deliveries(&mut deliveries);
        let mut ran = 0;
        for delivery in deliveries.drain(..) {
            // A listener reaped or disconnected since the event fired receives nothing.
            let Some(listener) = self.listeners.map.get(&delivery.listener).copied() else {
                continue;
            };
            if listener.connection != delivery.connection {
                continue;
            }
            let spawned = self.call(dm, listener.script, |backend, ctx| {
                backend.spawn_listener(delivery.listener, ctx)
            });
            match spawned {
                Some(Ok(thread)) => {
                    let thread = self.adopt(listener.script, listener.language, thread);
                    // The thread was adopted just above, so it is known.
                    let _ = self.resume_thread(dm, thread, &delivery.args);
                    ran += 1;
                }
                Some(Err(failure)) => self.report(listener.script, &failure),
                None => {}
            }
        }
        self.deliveries = deliveries;
        ran
    }

    /// Stops everything `script` started: releases its threads and disconnects and releases its listeners. The
    /// scheduler (#45) calls it when a script is destroyed, disabled or moved out of a service that runs scripts.
    /// Returns `false` when the script was not running.
    pub fn reap(&mut self, dm: &mut DataModel, script: Id) -> bool {
        let Some(record) = self.scripts.remove(&script) else {
            return false;
        };
        let backend = self.languages.get_mut(record.language.0);
        let mut backend = backend.map(|b| &mut **b);
        for thread in record.threads {
            self.threads.remove(&ThreadRef {
                language: record.language,
                thread,
            });
            if let Some(backend) = backend.as_deref_mut() {
                backend.release_thread(thread);
            }
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
        let mut ctx = HostContext::new(
            dm,
            Deadline::after(self.resume_budget),
            script,
            language,
            record,
            &mut self.listeners,
            &mut self.pending,
            &mut self.output,
        );
        let result = f(&mut **backend, &mut ctx);
        self.handle_requests(script, language);
        Some(result)
    }

    /// Handles the requests the last call left.
    fn handle_requests(&mut self, script: Id, language: LanguageId) {
        let mut pending = std::mem::take(&mut self.pending);
        for request in pending.drain(..) {
            match request {
                HostRequest::AdoptThread(thread) => {
                    self.adopt(script, language, thread);
                }
            }
        }
        self.pending = pending;
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
            },
        );
        thread_ref
    }

    /// Updates the thread's state after a resume: a finished or failed thread is released (and a failure
    /// reported), a yielded one remembers why.
    fn settle(&mut self, script: Id, thread: ThreadRef, resume: &Resume) {
        match resume {
            Resume::Yielded(reason) => {
                if let Some(record) = self.threads.get_mut(&thread) {
                    record.state = ThreadState::Yielded(reason.clone());
                }
                return;
            }
            Resume::Errored(failure) => self.report(script, failure),
            Resume::Finished(_) => {}
        }
        self.threads.remove(&thread);
        if let Some(record) = self.scripts.get_mut(&script) {
            record.threads.remove(&thread.thread);
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
