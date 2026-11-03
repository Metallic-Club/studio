//! The `ScriptLanguage` trait every language backend implements, and the values that cross it.

use metallic_core::{Id, ListenerKey, PropId, ScriptError, ScriptValue, SignalId};
use thiserror::Error;

use crate::HostContext;

/// A compiled chunk inside one backend, chosen by that backend. The host only stores it and hands it back to
/// [`ScriptLanguage::spawn`]; it never names a chunk of another backend.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkId(pub u64);

/// A thread (coroutine, fiber, task...) inside one backend, chosen by that backend. Unique within the backend
/// until [`ScriptLanguage::release_thread`] frees it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ThreadId(pub u64);

/// Why a thread stopped without finishing. The host's scheduler decides when to resume it (the crate docs give the
/// rules); the variants stay language-neutral, so `task.wait` in Luau and its equivalent in another language yield
/// the same thing.
#[derive(Clone, Debug, PartialEq)]
pub enum YieldReason {
    /// Resume after `seconds` of game time (`task.wait(seconds)`), at the earliest at the next `Heartbeat`. The
    /// thread is resumed with one number: the game time that actually passed, never less than `seconds`.
    Wait {
        /// How long to sleep. Zero, negative and NaN mean "at the next `Heartbeat`".
        seconds: f64,
    },
    /// Resume with the arguments of the next event of a signal (`signal:Wait()`). The host connects a listener of
    /// its own and disconnects it after that one event.
    WaitSignal {
        /// The instance whose signal is awaited.
        target: Id,
        /// Which of its signals.
        signal: SignalRef,
    },
    /// Resume, with no arguments, at the end of the current resumption point (`task.defer(coroutine.running())`).
    Defer,
    /// Suspended for a reason only the backend understands (a coroutine yield to its own caller, for example).
    /// The host keeps the thread alive and never resumes it on its own.
    Other,
}

/// A signal of an instance, as [`YieldReason::WaitSignal`] names it. The backend resolves the name while the script
/// runs (`DataModel::signal_id`, `DataModel::prop_id`), so an unknown name raises its error at the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalRef {
    /// A declared signal (`part.Touched`).
    Signal(SignalId),
    /// The changed signal of one property (`part:GetPropertyChangedSignal("Name")`).
    PropertyChanged(PropId),
}

/// What a call into a backend ended with.
#[derive(Clone, Debug, PartialEq)]
pub enum Resume {
    /// The thread returned these values and is done. The host releases it.
    Finished(Vec<ScriptValue>),
    /// The thread is suspended and may be resumed later.
    Yielded(YieldReason),
    /// The thread failed and is done. The host reports the failure and releases the thread.
    Errored(ScriptFailure),
}

/// Why a thread failed. Line numbers are 1-based lines of the script's source file.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum ScriptFailure {
    /// The script raised an error, or an engine call it made did.
    #[error("{message}")]
    Runtime {
        /// The error message, without the script name or line prefix (the host adds those when reporting).
        message: String,
        /// The line that raised it, when the backend knows.
        line: Option<u32>,
        /// The backend's stack traceback, when it has one.
        traceback: Option<String>,
    },
    /// The thread ran past the deadline in [`HostContext::deadline`] and was interrupted.
    #[error("script exceeded its time budget (is there an infinite loop?)")]
    Timeout {
        /// The line it was running when interrupted, when the backend knows.
        line: Option<u32>,
    },
    /// The backend's memory limit (see [`ScriptLanguage::set_memory_limit`]) was reached.
    #[error("not enough memory")]
    OutOfMemory,
}

impl ScriptFailure {
    /// A [`ScriptFailure::Runtime`] with no traceback.
    pub fn runtime(message: impl Into<String>, line: Option<u32>) -> Self {
        Self::Runtime {
            message: message.into(),
            line,
            traceback: None,
        }
    }

    /// The source line the failure happened on, when known.
    pub fn line(&self) -> Option<u32> {
        match self {
            Self::Runtime { line, .. } | Self::Timeout { line } => *line,
            Self::OutOfMemory => None,
        }
    }

    /// The backend's traceback, when it has one.
    pub fn traceback(&self) -> Option<&str> {
        match self {
            Self::Runtime { traceback, .. } => traceback.as_deref(),
            Self::Timeout { .. } | Self::OutOfMemory => None,
        }
    }
}

impl From<ScriptError> for ScriptFailure {
    /// An engine call's error, raised at an unknown line. Backends that know the line build the failure with
    /// [`ScriptFailure::runtime`] instead.
    fn from(error: ScriptError) -> Self {
        Self::runtime(error.to_string(), None)
    }
}

/// A chunk that failed to compile.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{chunk}:{line}: {message}")]
pub struct CompileError {
    /// The chunk name the host passed to [`ScriptLanguage::compile`] (the source path).
    pub chunk: String,
    /// The 1-based line of the error.
    pub line: u32,
    /// The 1-based column of the error, when the compiler reports one.
    pub column: Option<u32>,
    /// The compiler's message.
    pub message: String,
}

/// A scripting language backend: Luau first (`metallic_luau`), others later.
///
/// The trait is object-safe: the host holds one `Box<dyn ScriptLanguage>` per language and picks it by the
/// script file's extension. Everything a backend knows about the engine reaches it through [`HostContext`] during
/// a call; between calls it keeps only its own handles ([`ChunkId`], [`ThreadId`]), instance [`Id`]s and
/// [`ListenerKey`]s, never a reference into the data model or the host.
///
/// # Lifecycle of a thread
///
/// [`spawn`](Self::spawn) or [`spawn_listener`](Self::spawn_listener) creates a thread without running it. The host
/// then calls [`resume`](Self::resume) until it returns [`Resume::Finished`] or [`Resume::Errored`], and finally
/// [`release_thread`](Self::release_thread). The host may also release a suspended thread at any time (its script
/// was destroyed or disabled); the backend then drops it without running it again.
///
/// # Budgets
///
/// Before every `spawn`, `spawn_listener` and `resume` the host sets [`HostContext::deadline`]. The backend checks
/// it at its safepoints (loop back-edges, calls) with [`Deadline::poll`](crate::Deadline::poll), which reads the
/// clock only once every few hundred polls, and returns [`ScriptFailure::Timeout`] once it has passed. Only the
/// offending thread fails; the backend stays usable.
///
/// # The `task` library and signals
///
/// The host owns scheduling, so a backend's `task` functions are thin:
///
/// - `task.spawn(f, ...)`: create a thread for `f`, resume it **at once, nested in the current call** (only the
///   backend can run code while it is being called), then hand the outcome to [`HostContext::spawned`]. The host
///   adopts the thread and schedules it by how it ended, as if it had resumed it.
/// - `task.defer(f, ...)`, `task.delay(t, f, ...)`: create a thread (or pass an existing one) and call
///   [`HostContext::defer`] or [`HostContext::delay`]. `defer` refuses past the re-entrancy limit; raise its error in
///   the calling thread.
/// - `task.wait(t)`: yield [`YieldReason::Wait`]. `signal:Wait()`: yield [`YieldReason::WaitSignal`].
/// - `task.cancel(thread)`: mark the thread dead and call [`HostContext::cancel`]. Refuse to cancel the running
///   thread (or one that is resuming another) yourself, with an error.
/// - `Connect`, `Once`, `Disconnect`: [`HostContext::connect`], [`HostContext::once`], [`HostContext::disconnect`].
///
/// [`Id`]: metallic_core::Id
pub trait ScriptLanguage {
    /// The language's name, such as `"luau"`, for messages.
    fn name(&self) -> &str;

    /// The source file extensions this backend runs, without the dot, such as `["luau"]`.
    fn extensions(&self) -> &[&str];

    /// Compiles `source`. `chunk_name` is the source path; it names the chunk in compile errors. The host caches
    /// the result, so one file is compiled once per host however many scripts run it.
    fn compile(&mut self, source: &str, chunk_name: &str) -> Result<ChunkId, CompileError>;

    /// Creates the main thread of the script [`HostContext::script`] from `chunk`, without running it. The thread
    /// should be named after [`HostContext::chunk_name`] (`path#uid`), so runtime errors and the profiler can tell
    /// two scripts that share a file apart.
    fn spawn(&mut self, chunk: ChunkId, ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure>;

    /// Creates a thread that will call the function the backend stored under `listener` (when it connected through
    /// [`HostContext::connect`]), without running it. The host resumes it with the event's arguments. Every
    /// delivery runs in its own thread, so any handler may yield.
    fn spawn_listener(&mut self, listener: ListenerKey, ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure>;

    /// Runs `thread` until it finishes, yields or fails. `args` are the thread's arguments on its first resume and
    /// the values the yield returns afterwards.
    fn resume(&mut self, thread: ThreadId, args: &[ScriptValue], ctx: &mut HostContext<'_>) -> Resume;

    /// Drops `thread`. It is never resumed again. Releasing an unknown or already released thread does nothing.
    fn release_thread(&mut self, thread: ThreadId);

    /// Drops the function stored under `listener`. The host has already disconnected it. Releasing an unknown key
    /// does nothing.
    ///
    /// For a `Once` connection ([`HostContext::once`]) the host calls this right after
    /// [`spawn_listener`](Self::spawn_listener) for its one delivery, so that thread must hold the function itself.
    fn release_listener(&mut self, listener: ListenerKey);

    /// The bytes the backend's VM currently uses.
    fn memory_used(&self) -> usize;

    /// Caps the VM's memory at `bytes` (`None` removes the cap). Allocations past it fail the thread that made them
    /// with [`ScriptFailure::OutOfMemory`].
    fn set_memory_limit(&mut self, bytes: Option<usize>);
}
