//! A test-only "language" that drives the host through the same trait a real one uses (ADR 0003).
//!
//! A program is a list of statements, one per line (`#` starts a comment). Values are numbers, `true`, `false`,
//! `nil`, `"quoted strings"`, bare words (strings), `$var` (a variable), and `%script`, `%game` or `%<Service>`
//! (instances). `-> var` at the end of `new`, `call` and `get` stores the result.
//!
//! ```text
//! new <Class> [-> var]                     create an instance (`Instance.new`)
//! call <target> <Method> [args...] [-> var] call a method through `MethodCall`
//! set <target> <Prop> <value>              write a property
//! get <target> <Prop> [-> var]             read a property
//! connect <var> <target> <Signal> <stmt>   connect; the listener runs <stmt>; <var> holds the key (`_` ignores it)
//! disconnect <var>                         disconnect the key in <var>
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
    ChunkId, CompileError, HostContext, HostRequest, Resume, ScriptFailure, ScriptLanguage, ThreadId, YieldReason,
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
        body: Rc<[Stmt]>,
    },
    Disconnect {
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
}

impl MockThread {
    fn new(program: Rc<[Stmt]>) -> Self {
        Self {
            program,
            pc: 0,
            vars: HashMap::new(),
            args: Vec::new(),
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
}

impl MockLanguage {
    /// A backend and the log it writes to.
    pub(crate) fn new() -> (Self, Rc<RefCell<MockLog>>) {
        let log = Rc::new(RefCell::new(MockLog::default()));
        let language = Self {
            log: Rc::clone(&log),
            chunks: Vec::new(),
            threads: HashMap::new(),
            listeners: HashMap::new(),
            next_thread: 0,
            memory_limit: None,
        };
        (language, log)
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

    /// Runs `thread` until it yields, returns or fails. Threads it forks are left in `forks`.
    fn run_thread(
        &mut self,
        thread: ThreadId,
        args: &[ScriptValue],
        ctx: &mut HostContext<'_>,
        forks: &mut Vec<Rc<[Stmt]>>,
    ) -> Resume {
        let Some(state) = self.threads.get_mut(&thread) else {
            return Resume::Errored(ScriptFailure::runtime("cannot resume a dead thread", None));
        };
        state.args = args.to_vec();
        let program = Rc::clone(&state.program);
        while let Some(stmt) = program.get(state.pc) {
            state.pc += 1;
            // Every statement is a safepoint, like a VM's loop back-edges and calls.
            if ctx.deadline.poll() {
                return Resume::Errored(ScriptFailure::Timeout { line: Some(stmt.line) });
            }
            match run(stmt, state, ctx, &self.log, &mut self.listeners, forks) {
                Flow::Next => {}
                Flow::Yield(reason) => return Resume::Yielded(reason),
                Flow::Return(values) => return Resume::Finished(values),
                Flow::Fail(failure) => return Resume::Errored(failure),
            }
        }
        Resume::Finished(Vec::new())
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
        "mock"
    }

    fn extensions(&self) -> &[&str] {
        &["mock"]
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

    fn resume(&mut self, thread: ThreadId, args: &[ScriptValue], ctx: &mut HostContext<'_>) -> Resume {
        let mut forks = Vec::new();
        let resume = self.run_thread(thread, args, ctx, &mut forks);
        for program in forks {
            match self.new_thread(program) {
                Ok(child) => ctx.request(HostRequest::AdoptThread(child)),
                Err(failure) => return Resume::Errored(failure),
            }
        }
        resume
    }

    fn release_thread(&mut self, thread: ThreadId) {
        self.threads.remove(&thread);
        self.log.borrow_mut().released_threads.push(thread);
    }

    fn release_listener(&mut self, listener: ListenerKey) {
        self.listeners.remove(&listener);
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

/// Runs one statement. A `fork` leaves the child's program in `forks` for the caller to create.
fn run(
    stmt: &Stmt,
    state: &mut MockThread,
    ctx: &mut HostContext<'_>,
    log: &Rc<RefCell<MockLog>>,
    listeners: &mut HashMap<ListenerKey, Rc<[Stmt]>>,
    forks: &mut Vec<Rc<[Stmt]>>,
) -> Flow {
    match exec(stmt, state, ctx, log, listeners, forks) {
        Ok(flow) => flow,
        Err(error) => Flow::Fail(ScriptFailure::runtime(error.to_string(), Some(stmt.line))),
    }
}

fn exec(
    stmt: &Stmt,
    state: &mut MockThread,
    ctx: &mut HostContext<'_>,
    log: &Rc<RefCell<MockLog>>,
    listeners: &mut HashMap<ListenerKey, Rc<[Stmt]>>,
    forks: &mut Vec<Rc<[Stmt]>>,
) -> Result<Flow, ScriptError> {
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
            let args: Args = args
                .iter()
                .map(|a| value(state, ctx, a))
                .collect::<Result<Vec<_>, _>>()?
                .into();
            let result = ctx.call_method(this, method, args)?;
            store_in(state, store, result);
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
            body,
        } => {
            let id = instance(state, ctx, target)?;
            let connection = ctx.connect(id, signal)?;
            listeners.insert(connection.listener, Rc::clone(body));
            state
                .vars
                .insert(var.clone(), ScriptValue::Number(connection.listener.0 as f64));
        }
        Op::Disconnect { var } => {
            let Some(ScriptValue::Number(key)) = state.vars.get(var) else {
                return Err(ScriptError::runtime(format!("${var} is not a connection")));
            };
            let key = ListenerKey(*key as u64);
            if ctx.disconnect(key) {
                listeners.remove(&key);
            }
        }
        Op::Print(text) => ctx.print(text),
        Op::Warn(text) => ctx.warn(text),
        Op::Record(values) => {
            let values = if values.is_empty() {
                state.args.clone()
            } else {
                values.iter().map(|v| value(state, ctx, v)).collect::<Result<_, _>>()?
            };
            log.borrow_mut().records.push((ctx.script(), values));
        }
        Op::Yield(reason) => return Ok(Flow::Yield(reason.clone())),
        Op::Fork => forks.push(Rc::from(Vec::new())),
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

fn store_in(state: &mut MockThread, store: &Option<String>, value: ScriptValue) {
    if let Some(var) = store {
        state.vars.insert(var.clone(), value);
    }
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
        ("connect", _) => match rest {
            [var, target, signal, body @ ..] if !body.is_empty() => Op::Connect {
                var: text(var).to_owned(),
                target: target.clone(),
                signal: text(signal).to_owned(),
                body: Rc::from(vec![Stmt {
                    line,
                    op: parse(body, line)?,
                }]),
            },
            _ => return Err("connect needs a variable, a target, a signal and a statement".to_owned()),
        },
        ("disconnect", [var]) => Op::Disconnect {
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
