//! A test-only "language" that drives the host through the same trait a real one uses (ADR 0003).
//!
//! A program is a list of statements, one per line (`#` starts a comment). Values are numbers, `true`, `false`,
//! `nil`, `"quoted strings"`, bare words (strings), `$var` (a variable), `@n` (the thread's n-th resume argument),
//! and `%script`, `%game` or `%<Service>` (instances). `-> var` at the end of `new`, `call`, `get`, `require` and
//! the `task` statements stores the result. A `call` of a method that invokes a callback (`Invoke`) yields to the
//! host and stores the first value it returns (all of them as a tuple when there are several).
//!
//! A `<body>` is one or more statements separated by ` ; `, run by a new thread (a listener or a task). A task's
//! body starts with a copy of its creator's variables, and its `-> var` names the thread, not a statement of the
//! body.
//!
//! ```text
//! new <Class> [-> var]                     create an instance (`Instance.new`)
//! call <target> <Method> [args...] [-> var] call a method through `MethodCall`
//! set <target> <Prop> <value>              write a property
//! get <target> <Prop> [-> var]             read a property
//! connect <var> <target> <Signal> <body>   connect; the listener runs <body>; <var> holds the key (`_` ignores it)
//! once <var> <target> <Signal> <body>      like connect, for one event (`Once`)
//! disconnect <var>                         disconnect the key in <var>
//! require <target> [-> var]                require a ModuleScript
//! oninvoke <target> <Callback> <body>      assign <body> to a callback member; it returns what its `return` gives
//! clearcallback <target> <Callback>        unassign a callback member
//! waitsignal <target> <Signal>             yield until the signal fires (`signal:Wait()`)
//! waitchanged <target> <Prop>              yield until the property changes (`GetPropertyChangedSignal():Wait()`)
//! task.spawn [args... |] <body> [-> var]   run <body> in a new thread now, nested (`task.spawn`)
//! task.defer [args... |] <body> [-> var]   run <body> at the end of the resumption point (`task.defer`)
//! task.delay <s> [args... |] <body> [-> var] run <body> after <s> seconds (`task.delay`)
//! task.cancel <var>                        cancel the thread in <var>
//! print <words...> / warn <words...>       write to the output
//! record [values...]                       log the values (or, without any, the thread's resume arguments)
//! wait <seconds> / defer / yield           yield with `Wait`, `Defer` or `Other`
//! fork                                     create an empty thread and ask the host to adopt it
//! error <words...>                         fail with this message
//! spin                                     loop until the deadline passes
//! return [values...]                       finish with these values
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use metallic_core::{Args, Id, ListenerKey, ScriptError, ScriptValue};

use crate::{
    ChunkId, CompileError, HostContext, HostRequest, ModuleKey, Resume, ScriptFailure, ScriptLanguage, SignalRef,
    ThreadId, YieldReason,
};

/// The memory a mock thread "uses", so memory limits can be tested.
pub(crate) const THREAD_BYTES: usize = 1024;

/// What the mock did, shared with the test after the backend is boxed into a host.
#[derive(Debug, Default)]
pub(crate) struct MockLog {
    /// The chunk names passed to `compile`, in order.
    pub(crate) compiles: Vec<String>,
    /// `(script, chunk name)` of every main thread spawned.
    pub(crate) spawned: Vec<(Id, String)>,
    /// Threads released by the host.
    pub(crate) released_threads: Vec<ThreadId>,
    /// Listeners released by the host.
    pub(crate) released_listeners: Vec<ListenerKey>,
    /// `(script, values)` of every `record` statement.
    pub(crate) records: Vec<(Id, Vec<ScriptValue>)>,
    /// The last memory limit the host set.
    pub(crate) memory_limit: Option<usize>,
    /// The module of every module thread spawned.
    pub(crate) modules_spawned: Vec<Id>,
    /// Module values released by the host.
    pub(crate) released_modules: Vec<ModuleKey>,
    /// How many functions the backend holds under listener keys.
    pub(crate) held_listeners: usize,
    /// How many module values the backend holds.
    pub(crate) held_modules: usize,
}

impl MockLog {
    /// The first value of every record, as text: the labels of `record <label>` in run order.
    pub(crate) fn labels(&self) -> Vec<String> {
        self.records
            .iter()
            .map(|(_, values)| match values.first() {
                Some(ScriptValue::String(text)) => text.to_string(),
                Some(other) => format!("{other:?}"),
                None => String::new(),
            })
            .collect()
    }
}

/// A token of a statement.
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Quoted(String),
}

/// One parsed statement and the line it came from.
#[derive(Clone, Debug)]
struct Stmt {
    line: u32,
    op: Op,
}

/// Which `task` function starts a body.
#[derive(Clone, Copy, Debug)]
enum TaskKind {
    Spawn,
    Defer,
    Delay(f64),
}

#[derive(Clone, Debug)]
enum Op {
    New {
        class: String,
        store: Option<String>,
    },
    Call {
        target: Token,
        method: String,
        args: Vec<Token>,
        store: Option<String>,
    },
    Set {
        target: Token,
        prop: String,
        value: Token,
    },
    Get {
        target: Token,
        prop: String,
        store: Option<String>,
    },
    Connect {
        var: String,
        target: Token,
        signal: String,
        once: bool,
        body: Rc<[Stmt]>,
    },
    Disconnect {
        var: String,
    },
    Require {
        target: Token,
        store: Option<String>,
    },
    SetCallback {
        target: Token,
        callback: String,
        /// `None` clears it.
        body: Option<Rc<[Stmt]>>,
    },
    WaitSignal {
        target: Token,
        name: String,
        property: bool,
    },
    Task {
        kind: TaskKind,
        args: Vec<Token>,
        body: Rc<[Stmt]>,
        store: Option<String>,
    },
    Cancel {
        var: String,
    },
    Print(String),
    Warn(String),
    Record(Vec<Token>),
    Yield(YieldReason),
    Fork,
    Error(String),
    Spin,
    Return(Vec<Token>),
}

/// A live mock thread.
struct MockThread {
    program: Rc<[Stmt]>,
    pc: usize,
    vars: HashMap<String, ScriptValue>,
    /// The arguments of the latest resume.
    args: Vec<ScriptValue>,
    /// The variable the values of the next resume go to (a `call` that invoked a callback).
    pending_store: Option<String>,
    /// The line of the statement it last yielded at.
    yield_line: Option<u32>,
}

impl MockThread {
    fn new(program: Rc<[Stmt]>) -> Self {
        Self {
            program,
            pc: 0,
            vars: HashMap::new(),
            args: Vec::new(),
            pending_store: None,
            yield_line: None,
        }
    }
}

/// The mock backend. It runs `.mock` files.
pub(crate) struct MockLanguage {
    log: Rc<RefCell<MockLog>>,
    chunks: Vec<Rc<[Stmt]>>,
    threads: HashMap<ThreadId, MockThread>,
    listeners: HashMap<ListenerKey, Rc<[Stmt]>>,
    next_thread: u64,
    memory_limit: Option<usize>,
    /// Module values by key.
    modules: HashMap<ModuleKey, ScriptValue>,
    /// Threads loading a module, and the key its value goes under.
    module_threads: HashMap<ThreadId, ModuleKey>,
    /// The extension it claims.
    extensions: &'static [&'static str],
    /// Its name.
    name: &'static str,
}

impl MockLanguage {
    /// A backend and the log it writes to.
    pub(crate) fn new() -> (Self, Rc<RefCell<MockLog>>) {
        Self::named("mock", &["mock"])
    }

    /// A backend with another name and extension, for tests with two languages.
    pub(crate) fn named(name: &'static str, extensions: &'static [&'static str]) -> (Self, Rc<RefCell<MockLog>>) {
        let log = Rc::new(RefCell::new(MockLog::default()));
        let language = Self {
            log: Rc::clone(&log),
            chunks: Vec::new(),
            threads: HashMap::new(),
            listeners: HashMap::new(),
            next_thread: 0,
            memory_limit: None,
            modules: HashMap::new(),
            module_threads: HashMap::new(),
            extensions,
            name,
        };
        (language, log)
    }

    /// Publishes how many functions it holds under listener keys (connections and callbacks) and module values.
    fn sync_held(&self) {
        let mut log = self.log.borrow_mut();
        log.held_listeners = self.listeners.len();
        log.held_modules = self.modules.len();
    }

    /// Creates a thread running `program`, if the memory limit allows another one.
    fn new_thread(&mut self, program: Rc<[Stmt]>) -> Result<ThreadId, ScriptFailure> {
        if self
            .memory_limit
            .is_some_and(|limit| self.memory_used() + THREAD_BYTES > limit)
        {
            return Err(ScriptFailure::OutOfMemory);
        }
        self.next_thread += 1;
        let id = ThreadId(self.next_thread);
        self.threads.insert(id, MockThread::new(program));
        Ok(id)
    }

    /// Runs `thread` until it yields, returns or fails. The thread is taken out of the table while it runs, so a
    /// `task.spawn` can run another thread nested, as a real VM does.
    fn run_thread(&mut self, thread: ThreadId, args: &[ScriptValue], ctx: &mut HostContext<'_>) -> Resume {
        let Some(mut state) = self.threads.remove(&thread) else {
            return Resume::Errored(ScriptFailure::runtime("cannot resume a dead thread", None));
        };
        state.args = args.to_vec();
        if let Some(var) = state.pending_store.take() {
            let value = match args {
                [one] => one.clone(),
                many => ScriptValue::Tuple(many.to_vec()),
            };
            state.vars.insert(var, value);
        }
        let program = Rc::clone(&state.program);
        let mut resume = Resume::Finished(Vec::new());
        while let Some(stmt) = program.get(state.pc) {
            state.pc += 1;
            // Every statement is a safepoint, like a VM's loop back-edges and calls.
            if ctx.deadline.poll() {
                resume = Resume::Errored(ScriptFailure::Timeout { line: Some(stmt.line) });
                break;
            }
            let flow = match self.exec(stmt, &mut state, ctx) {
                Ok(flow) => flow,
                Err(error) => Flow::Fail(ScriptFailure::runtime(error.to_string(), Some(stmt.line))),
            };
            match flow {
                Flow::Next => continue,
                Flow::Yield(reason) => {
                    state.yield_line = Some(stmt.line);
                    resume = Resume::Yielded(reason);
                }
                Flow::Return(values) => resume = Resume::Finished(values),
                Flow::Fail(failure) => resume = Resume::Errored(failure),
            }
            break;
        }
        // Kept until the host releases it, as a VM keeps a dead coroutine object.
        self.threads.insert(thread, state);
        self.finish_module(thread, resume)
    }

    /// A module thread that finished keeps its one value; any other count fails it.
    fn finish_module(&mut self, thread: ThreadId, resume: Resume) -> Resume {
        let Resume::Finished(values) = resume else {
            return resume;
        };
        let Some(key) = self.module_threads.remove(&thread) else {
            return Resume::Finished(values);
        };
        match <[ScriptValue; 1]>::try_from(values) {
            Ok([value]) => {
                self.modules.insert(key, value);
                self.sync_held();
                Resume::Finished(Vec::new())
            }
            Err(_) => Resume::Errored(ScriptFailure::runtime(
                "Module code did not return exactly one value",
                None,
            )),
        }
    }

    /// Runs one statement.
    fn exec(&mut self, stmt: &Stmt, state: &mut MockThread, ctx: &mut HostContext<'_>) -> Result<Flow, ScriptError> {
        match &stmt.op {
            Op::New { class, store } => {
                let Some(class_id) = ctx.registry().class_id(class) else {
                    return Err(ScriptError::runtime(format!("unable to create an instance of {class}")));
                };
                let id = ctx
                    .dm
                    .create(class_id)
                    .map_err(|e| ScriptError::runtime(e.to_string()))?;
                store_in(state, store, ScriptValue::Instance(id));
            }
            Op::Call {
                target,
                method,
                args,
                store,
            } => {
                let this = instance(state, ctx, target)?;
                let args: Vec<ScriptValue> = args
                    .iter()
                    .map(|a| value(state, ctx, a))
                    .collect::<Result<Vec<_>, _>>()?;
                let class = ctx.dm.class_of(this).map_err(dm_error)?;
                if let Some(callback) = ctx.registry().method(class, method).and_then(|m| m.invokes) {
                    state.pending_store.clone_from(store);
                    return Ok(Flow::Yield(YieldReason::InvokeCallback {
                        target: this,
                        callback,
                        args,
                    }));
                }
                let result = ctx.call_method(this, method, Args::from(args))?;
                store_in(state, store, result);
            }
            Op::Require { target, store } => {
                let module = instance(state, ctx, target)?;
                match ctx.require(module)? {
                    Some(key) => {
                        let value = self.modules.get(&key).cloned().unwrap_or_default();
                        store_in(state, store, value);
                    }
                    None => {
                        // Asked again when resumed, as a real backend's `require` does.
                        state.pc -= 1;
                        return Ok(Flow::Yield(YieldReason::Require { module }));
                    }
                }
            }
            Op::SetCallback { target, callback, body } => {
                let id = instance(state, ctx, target)?;
                match body {
                    Some(body) => {
                        let key = ctx.set_callback(id, callback)?;
                        self.listeners.insert(key, Rc::clone(body));
                        self.sync_held();
                    }
                    None => ctx.clear_callback(id, callback)?,
                }
            }
            Op::Set { target, prop, value: v } => {
                let id = instance(state, ctx, target)?;
                let v = value(state, ctx, v)?;
                ctx.set_property(id, prop, &v)?;
            }
            Op::Get { target, prop, store } => {
                let id = instance(state, ctx, target)?;
                let v = ctx.get_property(id, prop)?;
                store_in(state, store, v);
            }
            Op::Connect {
                var,
                target,
                signal,
                once,
                body,
            } => {
                let id = instance(state, ctx, target)?;
                let connection = if *once {
                    ctx.once(id, signal)?
                } else {
                    ctx.connect(id, signal)?
                };
                self.listeners.insert(connection.listener, Rc::clone(body));
                self.sync_held();
                state
                    .vars
                    .insert(var.clone(), ScriptValue::Number(connection.listener.0 as f64));
            }
            Op::Disconnect { var } => {
                let key = ListenerKey(handle(state, var)?);
                if ctx.disconnect(key) {
                    self.listeners.remove(&key);
                    self.sync_held();
                }
            }
            Op::WaitSignal { target, name, property } => {
                let target = instance(state, ctx, target)?;
                let signal = if *property {
                    SignalRef::PropertyChanged(ctx.dm.prop_id(target, name).map_err(dm_error)?)
                } else {
                    SignalRef::Signal(ctx.dm.signal_id(target, name).map_err(dm_error)?)
                };
                return Ok(Flow::Yield(YieldReason::WaitSignal { target, signal }));
            }
            Op::Task {
                kind,
                args,
                body,
                store,
            } => {
                let args = args
                    .iter()
                    .map(|a| value(state, ctx, a))
                    .collect::<Result<Vec<_>, _>>()?;
                let child = self
                    .new_thread(Rc::clone(body))
                    .map_err(|failure| ScriptError::runtime(failure.to_string()))?;
                // The body sees the creator's variables, as a closure sees its upvalues.
                if let Some(child_state) = self.threads.get_mut(&child) {
                    child_state.vars = state.vars.clone();
                }
                store_in(state, store, ScriptValue::Number(child.0 as f64));
                match kind {
                    TaskKind::Spawn => {
                        let resume = self.run_thread(child, &args, ctx);
                        ctx.spawned(child, resume);
                    }
                    TaskKind::Defer => {
                        if let Err(error) = ctx.defer(child, args) {
                            // The child never runs; the calling thread raises the error.
                            self.threads.remove(&child);
                            return Err(error);
                        }
                    }
                    TaskKind::Delay(seconds) => ctx.delay(*seconds, child, args),
                }
            }
            Op::Cancel { var } => ctx.cancel(ThreadId(handle(state, var)?)),
            Op::Print(text) => ctx.print(text),
            Op::Warn(text) => ctx.warn(text),
            Op::Record(values) => {
                let values = if values.is_empty() {
                    state.args.clone()
                } else {
                    values.iter().map(|v| value(state, ctx, v)).collect::<Result<_, _>>()?
                };
                self.log.borrow_mut().records.push((ctx.script(), values));
            }
            Op::Yield(reason) => return Ok(Flow::Yield(reason.clone())),
            Op::Fork => {
                let child = self
                    .new_thread(Rc::from(Vec::new()))
                    .map_err(|failure| ScriptError::runtime(failure.to_string()))?;
                ctx.request(HostRequest::AdoptThread(child));
            }
            Op::Error(message) => return Ok(Flow::Fail(ScriptFailure::runtime(message, Some(stmt.line)))),
            Op::Spin => {
                while !ctx.deadline.poll() {
                    std::hint::spin_loop();
                }
                return Ok(Flow::Fail(ScriptFailure::Timeout { line: Some(stmt.line) }));
            }
            Op::Return(values) => {
                let values = values.iter().map(|v| value(state, ctx, v)).collect::<Result<_, _>>()?;
                return Ok(Flow::Return(values));
            }
        }
        Ok(Flow::Next)
    }
}

/// What one statement did.
enum Flow {
    Next,
    Yield(YieldReason),
    Return(Vec<ScriptValue>),
    Fail(ScriptFailure),
}

impl ScriptLanguage for MockLanguage {
    fn name(&self) -> &str {
        self.name
    }

    fn extensions(&self) -> &[&str] {
        self.extensions
    }

    fn compile(&mut self, source: &str, chunk_name: &str) -> Result<ChunkId, CompileError> {
        self.log.borrow_mut().compiles.push(chunk_name.to_owned());
        let mut program = Vec::new();
        for (index, text) in source.lines().enumerate() {
            let line = u32::try_from(index + 1).unwrap_or(u32::MAX);
            let tokens = tokenize(text);
            if tokens.is_empty() {
                continue;
            }
            let op = parse(&tokens, line).map_err(|message| CompileError {
                chunk: chunk_name.to_owned(),
                line,
                column: Some(1 + (text.len() - text.trim_start().len()) as u32),
                message,
            })?;
            program.push(Stmt { line, op });
        }
        self.chunks.push(program.into());
        Ok(ChunkId(self.chunks.len() as u64 - 1))
    }

    fn spawn(&mut self, chunk: ChunkId, ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure> {
        let Some(program) = usize::try_from(chunk.0).ok().and_then(|i| self.chunks.get(i)) else {
            return Err(ScriptFailure::runtime("unknown chunk", None));
        };
        let thread = self.new_thread(Rc::clone(program))?;
        self.log
            .borrow_mut()
            .spawned
            .push((ctx.script(), ctx.chunk_name().to_owned()));
        Ok(thread)
    }

    fn spawn_listener(&mut self, listener: ListenerKey, _ctx: &mut HostContext<'_>) -> Result<ThreadId, ScriptFailure> {
        let Some(body) = self.listeners.get(&listener) else {
            return Err(ScriptFailure::runtime("unknown listener", None));
        };
        self.new_thread(Rc::clone(body))
    }

    fn spawn_module(
        &mut self,
        chunk: ChunkId,
        key: ModuleKey,
        ctx: &mut HostContext<'_>,
    ) -> Result<ThreadId, ScriptFailure> {
        let Some(program) = usize::try_from(chunk.0).ok().and_then(|i| self.chunks.get(i)) else {
            return Err(ScriptFailure::runtime("unknown chunk", None));
        };
        let thread = self.new_thread(Rc::clone(program))?;
        self.module_threads.insert(thread, key);
        self.log.borrow_mut().modules_spawned.push(ctx.script());
        Ok(thread)
    }

    fn release_module(&mut self, key: ModuleKey) {
        self.modules.remove(&key);
        self.sync_held();
        self.log.borrow_mut().released_modules.push(key);
    }

    fn resume(&mut self, thread: ThreadId, args: &[ScriptValue], ctx: &mut HostContext<'_>) -> Resume {
        self.run_thread(thread, args, ctx)
    }

    fn resume_error(&mut self, thread: ThreadId, message: &str, _ctx: &mut HostContext<'_>) -> Resume {
        // The mock has no `pcall`: the error ends the thread at the statement it waited in.
        let Some(state) = self.threads.get_mut(&thread) else {
            return Resume::Errored(ScriptFailure::runtime("cannot resume a dead thread", None));
        };
        state.pc = state.program.len();
        self.module_threads.remove(&thread);
        Resume::Errored(ScriptFailure::runtime(message, state.yield_line))
    }

    fn release_thread(&mut self, thread: ThreadId) {
        self.threads.remove(&thread);
        self.module_threads.remove(&thread);
        self.log.borrow_mut().released_threads.push(thread);
    }

    fn release_listener(&mut self, listener: ListenerKey) {
        self.listeners.remove(&listener);
        self.sync_held();
        self.log.borrow_mut().released_listeners.push(listener);
    }

    fn memory_used(&self) -> usize {
        self.threads.len() * THREAD_BYTES
    }

    fn set_memory_limit(&mut self, bytes: Option<usize>) {
        self.memory_limit = bytes;
        self.log.borrow_mut().memory_limit = bytes;
    }
}

fn store_in(state: &mut MockThread, store: &Option<String>, value: ScriptValue) {
    if let Some(var) = store {
        state.vars.insert(var.clone(), value);
    }
}

/// The number a variable holds, as a key or thread handle.
fn handle(state: &MockThread, var: &str) -> Result<u64, ScriptError> {
    match state.vars.get(var) {
        Some(ScriptValue::Number(n)) => Ok(*n as u64),
        _ => Err(ScriptError::runtime(format!("${var} is not a handle"))),
    }
}

fn dm_error(error: metallic_core::DmError) -> ScriptError {
    ScriptError::runtime(error.to_string())
}

/// Evaluates a value token.
fn value(state: &MockThread, ctx: &HostContext<'_>, token: &Token) -> Result<ScriptValue, ScriptError> {
    let word = match token {
        Token::Quoted(text) => return Ok(ScriptValue::from(text.as_str())),
        Token::Word(word) => word.as_str(),
    };
    if let Some(var) = word.strip_prefix('$') {
        return Ok(state.vars.get(var).cloned().unwrap_or_default());
    }
    if let Some(n) = word.strip_prefix('@').and_then(|n| n.parse::<usize>().ok()) {
        return Ok(n
            .checked_sub(1)
            .and_then(|i| state.args.get(i))
            .cloned()
            .unwrap_or_default());
    }
    if let Some(name) = word.strip_prefix('%') {
        let id = match name {
            "script" => Some(ctx.script()),
            "game" => ctx.dm.root(),
            service => ctx.registry().class_id(service).and_then(|class| ctx.dm.service(class)),
        };
        return id
            .map(ScriptValue::Instance)
            .ok_or_else(|| ScriptError::runtime(format!("{name} is not a valid service")));
    }
    Ok(match word {
        "nil" => ScriptValue::Nil,
        "true" => ScriptValue::Bool(true),
        "false" => ScriptValue::Bool(false),
        _ => match word.parse::<f64>() {
            Ok(n) => ScriptValue::Number(n),
            Err(_) => ScriptValue::from(word),
        },
    })
}

/// Evaluates a token that must be an instance.
fn instance(state: &MockThread, ctx: &HostContext<'_>, token: &Token) -> Result<Id, ScriptError> {
    match value(state, ctx, token)? {
        ScriptValue::Instance(id) => Ok(id),
        other => Err(ScriptError::runtime(format!(
            "attempt to index {} with a member",
            other.type_name()
        ))),
    }
}

/// Splits a line into words and `"quoted strings"`, dropping a `#` comment.
fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '#' {
            break;
        } else if c == '"' {
            chars.next();
            let quoted: String = chars.by_ref().take_while(|&c| c != '"').collect();
            tokens.push(Token::Quoted(quoted));
        } else {
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                word.push(c);
                chars.next();
            }
            tokens.push(Token::Word(word));
        }
    }
    tokens
}

/// The text of a token, quoted or not.
fn text(token: &Token) -> &str {
    match token {
        Token::Word(text) | Token::Quoted(text) => text,
    }
}

/// Whether a token is the bare word `word`.
fn is_word(token: &Token, word: &str) -> bool {
    matches!(token, Token::Word(w) if w == word)
}

/// Splits off a trailing `-> var`.
fn split_store(tokens: &[Token]) -> (&[Token], Option<String>) {
    match tokens {
        [rest @ .., Token::Word(arrow), Token::Word(var)] if arrow == "->" => (rest, Some(var.clone())),
        _ => (tokens, None),
    }
}

fn words(tokens: &[Token]) -> String {
    tokens.iter().map(text).collect::<Vec<_>>().join(" ")
}

/// Parses a body: statements separated by `;`.
fn parse_body(tokens: &[Token], line: u32) -> Result<Rc<[Stmt]>, String> {
    let mut body = Vec::new();
    for part in tokens.split(|t| is_word(t, ";")) {
        if part.is_empty() {
            return Err("empty statement in a body".to_owned());
        }
        body.push(Stmt {
            line,
            op: parse(part, line)?,
        });
    }
    Ok(body.into())
}

/// Parses `[args... |] <body>` of a `task` statement.
fn parse_task(kind: TaskKind, tokens: &[Token], store: Option<String>, line: u32) -> Result<Op, String> {
    let (args, body) = match tokens.iter().position(|t| is_word(t, "|")) {
        Some(bar) => (tokens[..bar].to_vec(), &tokens[bar + 1..]),
        None => (Vec::new(), tokens),
    };
    if body.is_empty() {
        return Err("a task needs a body".to_owned());
    }
    Ok(Op::Task {
        kind,
        args,
        body: parse_body(body, line)?,
        store,
    })
}

/// Parses one statement, or says why it is not one.
fn parse(tokens: &[Token], line: u32) -> Result<Op, String> {
    let Some((Token::Word(keyword), rest)) = tokens.split_first() else {
        return Err("expected a statement".to_owned());
    };
    let (args, store) = split_store(rest);
    let op = match (keyword.as_str(), args) {
        ("new", [class]) => Op::New {
            class: text(class).to_owned(),
            store,
        },
        ("call", [target, method, args @ ..]) => Op::Call {
            target: target.clone(),
            method: text(method).to_owned(),
            args: args.to_vec(),
            store,
        },
        ("set", [target, prop, value]) if store.is_none() => Op::Set {
            target: target.clone(),
            prop: text(prop).to_owned(),
            value: value.clone(),
        },
        ("get", [target, prop]) => Op::Get {
            target: target.clone(),
            prop: text(prop).to_owned(),
            store,
        },
        ("connect" | "once", _) => match rest {
            [var, target, signal, body @ ..] if !body.is_empty() => Op::Connect {
                var: text(var).to_owned(),
                target: target.clone(),
                signal: text(signal).to_owned(),
                once: keyword == "once",
                body: parse_body(body, line)?,
            },
            _ => return Err(format!("{keyword} needs a variable, a target, a signal and a body")),
        },
        ("disconnect", [var]) => Op::Disconnect {
            var: text(var).trim_start_matches('$').to_owned(),
        },
        ("require", [target]) => Op::Require {
            target: target.clone(),
            store,
        },
        ("oninvoke", _) => match rest {
            [target, callback, body @ ..] if !body.is_empty() => Op::SetCallback {
                target: target.clone(),
                callback: text(callback).to_owned(),
                body: Some(parse_body(body, line)?),
            },
            _ => return Err("oninvoke needs a target, a callback and a body".to_owned()),
        },
        ("clearcallback", [target, callback]) if store.is_none() => Op::SetCallback {
            target: target.clone(),
            callback: text(callback).to_owned(),
            body: None,
        },
        ("waitsignal" | "waitchanged", [target, name]) => Op::WaitSignal {
            target: target.clone(),
            name: text(name).to_owned(),
            property: keyword == "waitchanged",
        },
        ("task.spawn", body) => parse_task(TaskKind::Spawn, body, store, line)?,
        ("task.defer", body) => parse_task(TaskKind::Defer, body, store, line)?,
        ("task.delay", [seconds, body @ ..]) => {
            let seconds = text(seconds)
                .parse()
                .map_err(|_| format!("task.delay needs a number of seconds, got {}", text(seconds)))?;
            parse_task(TaskKind::Delay(seconds), body, store, line)?
        }
        ("task.cancel", [var]) => Op::Cancel {
            var: text(var).trim_start_matches('$').to_owned(),
        },
        ("print", _) => Op::Print(words(rest)),
        ("warn", _) => Op::Warn(words(rest)),
        ("record", _) => Op::Record(rest.to_vec()),
        ("wait", [seconds]) => Op::Yield(YieldReason::Wait {
            seconds: text(seconds)
                .parse()
                .map_err(|_| format!("wait needs a number of seconds, got {}", text(seconds)))?,
        }),
        ("defer", []) => Op::Yield(YieldReason::Defer),
        ("yield", []) => Op::Yield(YieldReason::Other),
        ("fork", []) => Op::Fork,
        ("error", _) => Op::Error(words(rest)),
        ("spin", []) => Op::Spin,
        ("return", _) => Op::Return(rest.to_vec()),
        (keyword, _) => return Err(format!("unexpected '{keyword}'")),
    };
    Ok(op)
}
