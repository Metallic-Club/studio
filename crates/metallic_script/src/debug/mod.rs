//! The script debugger (task 15): breakpoints, pause and step, and inspection of a paused
//! game's threads.
//!
//! # How a pause works
//!
//! Studio runs the game on its UI thread, so a paused script must not block: it *breaks*.
//! Luau's `lua_break` suspends the running thread like a yield, and the resume that ran it
//! returns `LUA_BREAK` to the scheduler, which keeps the thread instead of finishing it.
//! Breaks travel outwards: a coroutine that breaks inside `coroutine.resume` breaks its
//! resumer (Luau's own rule), and a `task.spawn` thread that breaks breaks the thread that
//! spawned it as soon as `task.spawn` returns. The paused threads form a chain, innermost
//! first; Continue resumes them in that order, each with a fresh execution budget, so the
//! deadline never runs while the game is paused.
//!
//! Breaks come from Luau's debug callbacks: `debugbreak` (a `BREAK` opcode patched in by
//! [`lua_breakpoint`](mlua::ffi::lua_breakpoint)), `debugstep` (single-step mode, for
//! stepping and Pause), `debugprotectederror` (break on unhandled errors) and
//! `debuginterrupt` (a child coroutine broke). The callbacks are installed only while the
//! debugger is *armed* (breakpoints set, break on error, a step or pause pending), so a
//! game without breakpoints runs exactly as it does without a debugger.
//!
//! While anything is paused the host runs no script code: steps return early, signal
//! handlers are queued and resumes are deferred until the chain has been continued.

mod api;
mod breakpoints;
mod inspect;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CStr, c_int};
use std::rc::Weak;

use metallic_core::InstanceId;
use mlua::{Lua, MultiValue, Thread, Value, ffi};

use crate::task_lib::Scheduler;

pub(crate) use breakpoints::BreakpointSet;

/// What a breakpoint does when its line runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Breakpoint {
    /// Pauses every time.
    Plain,
    /// Pauses when the Luau expression is truthy in the paused frame (an expression that
    /// errors also pauses, after a warning).
    Condition(String),
    /// Prints the message without pausing; `{expr}` parts are evaluated in the frame.
    Log(String),
}

/// Why the game is paused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PauseReason {
    /// A breakpoint was hit.
    Breakpoint,
    /// A step finished.
    Step,
    /// [`Debugger::pause`] was called.
    Pause,
    /// An error no `pcall` handles (with break on error on). The text is the error message.
    Error(String),
}

/// One frame of a paused call stack, innermost first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The script whose code this is (None for code the host cannot attribute).
    pub script: Option<InstanceId>,
    /// The chunk name (the script's full name).
    pub chunk: String,
    /// The function name (`(main)` for the script body, `(anonymous)` when unnamed).
    pub function: String,
    /// The line being run.
    pub line: u32,
}

/// Whether the game runs or is paused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugState {
    /// Scripts run.
    Running,
    /// Scripts, physics and characters are stopped.
    Paused {
        /// Why.
        reason: PauseReason,
        /// The paused call stack, innermost first (empty after [`Debugger::pause`] when no
        /// script was running).
        stack: Vec<Frame>,
    },
}

/// Which variables of a frame to list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The frame's local variables in scope at the paused line.
    Locals,
    /// The frame function's upvalues.
    Upvalues,
    /// The script's own globals (including `script`).
    Globals,
}

/// Names a table or Instance shown in the variables tree, for [`Debugger::expand`]. Valid
/// until the game runs again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VarHandle(pub(crate) u32);

/// One row of the variables tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarNode {
    /// The variable name or table key.
    pub name: String,
    /// The value as text.
    pub display: String,
    /// Set for tables and Instances, which can be expanded.
    pub children: Option<VarHandle>,
}

/// How a paused thread got paused.
#[derive(Clone, Debug)]
pub(crate) enum How {
    /// In a debug hook at an instruction (breakpoint or step): resuming runs that
    /// instruction's hook again, which must be skipped once.
    Hook { depth: c_int, line: c_int },
    /// In a C function (`coroutine.resume` or `task.spawn`) after an inner thread broke.
    Call,
    /// At an unhandled error: the thread is not resumed again, the error is reported.
    Error(String),
}

/// Who resumes a paused thread when the game continues.
#[derive(Clone)]
pub(crate) enum Resumer {
    /// Resumed by the scheduler (a script, task, handler or waiter thread): its results
    /// are read like those of any scheduler resume.
    Scheduler(Thread, u64),
    /// A coroutine resumed by `coroutine.resume`: its results stay on its stack for the
    /// resumer's continuation.
    Coroutine,
}

/// One paused thread.
pub(crate) struct Broken {
    pub(crate) state: *mut ffi::lua_State,
    pub(crate) resumer: Resumer,
    pub(crate) how: How,
    pub(crate) frames: Vec<inspect::FrameSnap>,
    /// Local writes to apply in the hook that runs first on resume (top frame only).
    pub(crate) writes: Vec<(c_int, String, Value)>,
}

/// A requested step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepKind {
    Into,
    Over,
    Out,
}

#[derive(Clone, Copy)]
struct Step {
    kind: StepKind,
    thread: *mut ffi::lua_State,
    depth: c_int,
    line: c_int,
    /// The stepped thread finished or yielded: stop in whatever runs next.
    done: bool,
}

/// The hook-skip and local writes of the thread being resumed from a hook break.
struct Resumed {
    state: *mut ffi::lua_State,
    depth: c_int,
    line: c_int,
    writes: Vec<(c_int, String, Value)>,
}

/// The debugger state of one VM. Owned by its [`Scheduler`].
pub(crate) struct DebugCore {
    /// Main state of the VM (key of the hook registry), null before install.
    main: Cell<*mut ffi::lua_State>,
    armed: Cell<bool>,
    pub(crate) breakpoints: BreakpointSet,
    break_on_error: Cell<bool>,
    /// >0 while the debugger itself runs Lua (conditions, watches): no breaks then.
    suppress: Cell<u32>,
    step: Cell<Option<Step>>,
    /// Break at the next instruction any thread runs (Pause, or a step with no stack).
    break_next: Cell<bool>,
    /// Paused threads, innermost first. Non-empty means paused.
    paused: RefCell<Vec<Broken>>,
    reason: RefCell<Option<PauseReason>>,
    /// Threads in `lua_resume` now, innermost last, and whether the scheduler resumed them.
    active: RefCell<Vec<(*mut ffi::lua_State, bool)>>,
    /// The next `lua_resume` is the scheduler's.
    rust_resume: Cell<bool>,
    resumed: RefCell<Option<Resumed>>,
    /// Resumes asked for while paused, run once the game continues.
    deferred: RefCell<VecDeque<(Thread, Vec<Value>)>>,
    /// Values shown in the variables tree, by [`VarHandle`].
    pub(crate) handles: RefCell<Vec<Value>>,
    /// `pcall` and `xpcall`, to tell handled errors from unhandled ones.
    pcalls: Cell<[usize; 2]>,
}

thread_local! {
    /// Hook lookup: VM main state -> its scheduler.
    static CORES: RefCell<Vec<(usize, Weak<Scheduler>)>> = const { RefCell::new(Vec::new()) };
}

fn with_sched(l: *mut ffi::lua_State, f: impl FnOnce(&Scheduler)) {
    let main = unsafe { ffi::lua_mainthread(l) } as usize;
    let sched = CORES.with(|c| {
        c.borrow()
            .iter()
            .find(|(k, _)| *k == main)
            .and_then(|(_, w)| w.upgrade())
    });
    if let Some(s) = sched {
        f(&s);
    }
}

/// The outcome of one resume.
pub(crate) enum Outcome {
    /// Finished or yielded with these values.
    Done(MultiValue),
    /// Paused.
    Break,
}

impl Outcome {
    /// The values of a finished or yielded resume (none after a break).
    pub(crate) fn values(self) -> MultiValue {
        match self {
            Outcome::Done(values) => values,
            Outcome::Break => MultiValue::new(),
        }
    }
}

impl DebugCore {
    pub(crate) fn new() -> Self {
        DebugCore {
            main: Cell::new(std::ptr::null_mut()),
            armed: Cell::new(false),
            breakpoints: BreakpointSet::default(),
            break_on_error: Cell::new(false),
            suppress: Cell::new(0),
            step: Cell::new(None),
            break_next: Cell::new(false),
            paused: RefCell::new(Vec::new()),
            reason: RefCell::new(None),
            active: RefCell::new(Vec::new()),
            rust_resume: Cell::new(false),
            resumed: RefCell::new(None),
            deferred: RefCell::new(VecDeque::new()),
            handles: RefCell::new(Vec::new()),
            pcalls: Cell::new([0; 2]),
        }
    }

    /// Registers the hooks of `lua` for `sched` (once, at VM setup).
    pub(crate) fn install(sched: &std::rc::Rc<Scheduler>, lua: &Lua) -> mlua::Result<()> {
        let mut main = std::ptr::null_mut();
        let globals = lua.globals();
        let pcall: mlua::Function = globals.get("pcall")?;
        let xpcall: mlua::Function = globals.get("xpcall")?;
        sched
            .debug
            .pcalls
            .set([pcall.to_pointer() as usize, xpcall.to_pointer() as usize]);
        // SAFETY: only reads the main state pointer.
        unsafe {
            lua.exec_raw::<()>((), |st| main = ffi::lua_mainthread(st))?;
        }
        sched.debug.main.set(main);
        let weak = std::rc::Rc::downgrade(sched);
        CORES.with(|c| {
            let mut c = c.borrow_mut();
            c.retain(|(k, w)| *k != main as usize && w.strong_count() > 0);
            c.push((main as usize, weak));
        });
        Ok(())
    }

    /// Forgets everything (VM shutdown): paused threads are dropped, not resumed.
    pub(crate) fn clear(&self) {
        self.set_callbacks(false);
        self.armed.set(false);
        self.paused.borrow_mut().clear();
        self.deferred.borrow_mut().clear();
        self.handles.borrow_mut().clear();
        self.resumed.borrow_mut().take();
        self.active.borrow_mut().clear();
        self.breakpoints.clear();
        self.step.set(None);
        self.break_next.set(false);
        self.reason.borrow_mut().take();
        let main = self.main.replace(std::ptr::null_mut()) as usize;
        if main != 0 {
            CORES.with(|c| c.borrow_mut().retain(|(k, _)| *k != main));
        }
    }

    /// True while hooks are installed (resumes then go through [`raw_resume`]).
    pub(crate) fn armed(&self) -> bool {
        self.armed.get()
    }

    /// True while any thread of this VM is paused.
    pub(crate) fn is_paused(&self) -> bool {
        !self.paused.borrow().is_empty()
    }

    /// Installs or removes the hooks to match the configuration. Called only while no
    /// script runs.
    pub(crate) fn rearm(&self) {
        let want = !self.breakpoints.is_empty()
            || self.break_on_error.get()
            || self.step.get().is_some()
            || self.break_next.get()
            || self.is_paused();
        if want != self.armed.get() {
            self.armed.set(want);
            self.set_callbacks(want);
            if !want {
                self.active.borrow_mut().clear();
            }
        }
    }

    fn set_callbacks(&self, on: bool) {
        let main = self.main.get();
        if main.is_null() {
            return;
        }
        // SAFETY: the callbacks struct lives as long as the state; mlua owns only
        // `interrupt`, `userthread` and `userdata`.
        unsafe {
            let cb = ffi::lua_callbacks(main);
            (*cb).debugbreak = on.then_some(hook_break as _);
            (*cb).debugstep = on.then_some(hook_step as _);
            (*cb).debuginterrupt = on.then_some(hook_interrupt as _);
            (*cb).debugprotectederror = on.then_some(hook_error as _);
            (*cb).preresume = on.then_some(hook_preresume as _);
            (*cb).postresume = on.then_some(hook_postresume as _);
        }
    }

    pub(crate) fn set_break_on_error(&self, on: bool) {
        self.break_on_error.set(on);
        self.rearm();
    }

    /// Why and where the VM is paused (None while running).
    pub(crate) fn pause_info(&self) -> Option<(PauseReason, Vec<inspect::FrameInfo>)> {
        let paused = self.paused.borrow();
        if paused.is_empty() {
            return None;
        }
        let reason = self
            .reason
            .borrow()
            .clone()
            .unwrap_or(PauseReason::Breakpoint);
        let frames = paused
            .iter()
            .flat_map(|b| b.frames.iter().map(|f| f.info.clone()))
            .collect();
        Some((reason, frames))
    }

    /// Breaks at the next instruction any script runs (Pause, or a step with no stack).
    pub(crate) fn set_break_next(&self, on: bool) {
        self.break_next.set(on);
        self.rearm();
    }

    /// Queues a resume asked for while paused.
    pub(crate) fn defer(&self, thread: Thread, args: Vec<Value>) {
        self.deferred.borrow_mut().push_back((thread, args));
    }

    /// The scheduler's resume of `thread` returned a break: it resumes it again later.
    pub(crate) fn scheduler_break(&self, thread: &Thread, owner: u64) {
        let ptr = thread.to_pointer() as *mut ffi::lua_State;
        let mut paused = self.paused.borrow_mut();
        match paused.iter_mut().find(|b| b.state == ptr) {
            Some(b) => b.resumer = Resumer::Scheduler(thread.clone(), owner),
            None => paused.push(Broken {
                state: ptr,
                resumer: Resumer::Scheduler(thread.clone(), owner),
                how: How::Call,
                frames: Vec::new(),
                writes: Vec::new(),
            }),
        }
    }

    /// Continues the paused threads, innermost first, then the deferred resumes. Stops
    /// when something breaks again. `step` is applied to the innermost thread.
    pub(crate) fn resume(&self, s: &Scheduler, step: Option<StepKind>) {
        if !self.is_paused() {
            if step.is_some() {
                self.set_break_next(true);
            }
            return;
        }
        self.handles.borrow_mut().clear();
        self.reason.borrow_mut().take();
        self.step.set(step.and_then(|kind| {
            let paused = self.paused.borrow();
            let first = paused.first()?;
            let (depth, line) = match first.how {
                How::Hook { depth, line } => (depth, line),
                // SAFETY: a paused thread is alive while it is in the chain.
                How::Call => (unsafe { ffi::lua_stackdepth(first.state) }, -1),
                How::Error(_) => return None,
            };
            Some(Step {
                kind,
                thread: first.state,
                depth,
                line,
                done: false,
            })
        }));
        self.rearm();
        loop {
            let mut rest = std::mem::take(&mut *self.paused.borrow_mut());
            if rest.is_empty() {
                break;
            }
            let entry = rest.remove(0);
            let broke = self.run_entry(s, entry);
            let mut paused = self.paused.borrow_mut();
            paused.extend(rest);
            if broke {
                drop(paused);
                self.rearm();
                return;
            }
        }
        while !self.is_paused() {
            let next = self.deferred.borrow_mut().pop_front();
            let Some((thread, args)) = next else { break };
            if let Err(e) = s.resume_with_budget(&thread, args) {
                s.sink.error(&e);
            }
        }
        if !self.is_paused() && self.step.get().is_some_and(|st| st.done) {
            self.step.set(None);
        }
        self.rearm();
    }

    /// Resumes one paused thread; true if it (or something it ran) broke again.
    fn run_entry(&self, s: &Scheduler, entry: Broken) -> bool {
        let Ok(lua) = s.lua() else { return false };
        if let How::Error(message) = &entry.how {
            s.sink.error(message);
            if let Resumer::Scheduler(thread, _) = &entry.resumer {
                s.forget(thread);
            }
            self.mark_step_done(entry.state);
            return false;
        }
        if let How::Hook { depth, line } = entry.how {
            *self.resumed.borrow_mut() = Some(Resumed {
                state: entry.state,
                depth,
                line,
                writes: entry.writes,
            });
        }
        let resumer = entry.resumer.clone();
        let broke = match &entry.resumer {
            Resumer::Scheduler(thread, owner) => {
                if s.begin_resume(thread).is_err() {
                    self.resumed.borrow_mut().take();
                    return false;
                }
                let (raw, timeout) = s.as_owner(*owner, || {
                    s.budgeted(|| raw_resume(&lua, self, thread, Vec::new()))
                });
                match raw {
                    Ok(Outcome::Break) => true,
                    raw => {
                        if let Some(msg) = timeout {
                            s.cancel_thread(thread);
                            s.sink.error(&msg);
                        } else if let Err(e) = s.after_resume(thread, raw.map(Outcome::values)) {
                            s.sink.error(&e);
                        }
                        false
                    }
                }
            }
            Resumer::Coroutine => {
                let (status, _) = s.budgeted(|| raw_resume_keep(&lua, self, entry.state));
                status == ffi::LUA_BREAK
            }
        };
        self.resumed.borrow_mut().take();
        if broke {
            let mut paused = self.paused.borrow_mut();
            if let Some(b) = paused.iter_mut().find(|b| b.state == entry.state) {
                b.resumer = resumer;
            }
        } else {
            self.mark_step_done(entry.state);
        }
        broke
    }

    fn mark_step_done(&self, state: *mut ffi::lua_State) {
        if let Some(mut step) = self.step.get()
            && step.thread == state
        {
            step.done = true;
            self.step.set(Some(step));
        }
    }

    /// Whether `l` may break now: every thread from it down to the one the scheduler
    /// resumed is yieldable (a break never crosses a C call or a host `pcall`).
    fn breakable(&self, l: *mut ffi::lua_State) -> bool {
        if self.suppress.get() > 0 {
            return false;
        }
        let active = self.active.borrow();
        if active.last().is_none_or(|(top, _)| *top != l) {
            return false;
        }
        for &(state, by_rust) in active.iter().rev() {
            // SAFETY: threads in `lua_resume` are alive.
            if unsafe { ffi::lua_isyieldable(state) } == 0 {
                return false;
            }
            if by_rust {
                return true;
            }
        }
        false
    }

    /// Records that `l` pauses here and breaks it.
    ///
    /// # Safety
    /// `l` is running in a debug hook or C function and [`DebugCore::breakable`] holds.
    unsafe fn pause_here(
        &self,
        s: &Scheduler,
        l: *mut ffi::lua_State,
        how: How,
        reason: PauseReason,
    ) {
        let frames = match s.lua() {
            Ok(lua) => unsafe { inspect::capture(&lua, self, l) },
            Err(_) => Vec::new(),
        };
        self.paused.borrow_mut().push(Broken {
            state: l,
            resumer: Resumer::Coroutine,
            how,
            frames,
            writes: Vec::new(),
        });
        self.reason.borrow_mut().get_or_insert(reason);
        self.step.set(None);
        self.break_next.set(false);
        unsafe {
            ffi::lua_break(l);
        }
    }

    /// The first hook on a thread resumed from a hook break: applies queued local writes
    /// and says whether this is the re-run of the paused instruction (to skip).
    unsafe fn take_resumed(
        &self,
        s: &Scheduler,
        l: *mut ffi::lua_State,
        depth: c_int,
        line: c_int,
    ) -> bool {
        let rec = {
            let mut resumed = self.resumed.borrow_mut();
            match resumed.as_ref() {
                Some(r) if r.state == l => resumed.take(),
                _ => None,
            }
        };
        let Some(rec) = rec else { return false };
        if let Ok(lua) = s.lua() {
            for (level, name, value) in rec.writes {
                unsafe { inspect::write_local(&lua, l, level, &name, value) };
            }
        }
        rec.depth == depth && rec.line == line
    }

    /// Whether the thread being resumed should single-step.
    fn wants_step(&self, l: *mut ffi::lua_State) -> bool {
        self.break_next.get()
            || self.step.get().is_some()
            || self
                .resumed
                .borrow()
                .as_ref()
                .is_some_and(|r| r.state == l && !r.writes.is_empty())
    }
}

fn source_of(ar: &ffi::lua_Debug) -> String {
    if ar.short_src.is_null() {
        return String::new();
    }
    // SAFETY: Luau fills `short_src` with a NUL-terminated string.
    unsafe { CStr::from_ptr(ar.short_src) }
        .to_string_lossy()
        .into_owned()
}

/// Whether level 0 of `l` is engine code or C (never a place to stop a step).
unsafe fn in_engine(l: *mut ffi::lua_State) -> bool {
    let mut ar = unsafe { std::mem::zeroed::<ffi::lua_Debug>() };
    if unsafe { ffi::lua_getinfo(l, 0, c"s".as_ptr(), &mut ar) } == 0 {
        return true;
    }
    let src = source_of(&ar);
    src == "[engine]" || src == "[C]"
}

unsafe extern "C-unwind" fn hook_break(l: *mut ffi::lua_State, ar: *mut ffi::lua_Debug) {
    let line = unsafe { (*ar).currentline };
    with_sched(l, |s| unsafe { on_break(s, l, line) });
}

unsafe fn on_break(s: &Scheduler, l: *mut ffi::lua_State, line: c_int) {
    let d = &s.debug;
    if d.suppress.get() > 0 {
        return;
    }
    let depth = unsafe { ffi::lua_stackdepth(l) };
    if unsafe { d.take_resumed(s, l, depth, line) } {
        return;
    }
    let Ok(lua) = s.lua() else { return };
    let pause = unsafe { breakpoints::check_hit(&lua, s, l, line) };
    if pause && d.breakable(l) {
        unsafe { d.pause_here(s, l, How::Hook { depth, line }, PauseReason::Breakpoint) };
    }
}

unsafe extern "C-unwind" fn hook_step(l: *mut ffi::lua_State, ar: *mut ffi::lua_Debug) {
    let line = unsafe { (*ar).currentline };
    with_sched(l, |s| unsafe { on_step(s, l, line) });
}

unsafe fn on_step(s: &Scheduler, l: *mut ffi::lua_State, line: c_int) {
    let d = &s.debug;
    if d.suppress.get() > 0 {
        return;
    }
    let depth = unsafe { ffi::lua_stackdepth(l) };
    if unsafe { d.take_resumed(s, l, depth, line) } {
        return;
    }
    let reason = if d.break_next.get() {
        PauseReason::Pause
    } else {
        let Some(step) = d.step.get() else { return };
        let stop = if step.done {
            true
        } else if step.thread == l {
            match step.kind {
                StepKind::Into => depth != step.depth || line != step.line,
                StepKind::Over => depth < step.depth || (depth == step.depth && line != step.line),
                StepKind::Out => depth < step.depth,
            }
        } else {
            step.kind == StepKind::Into
        };
        if !stop {
            return;
        }
        PauseReason::Step
    };
    if unsafe { in_engine(l) } || !d.breakable(l) {
        return;
    }
    unsafe { d.pause_here(s, l, How::Hook { depth, line }, reason) };
}

unsafe extern "C-unwind" fn hook_interrupt(l: *mut ffi::lua_State, _ar: *mut ffi::lua_Debug) {
    // A coroutine resumed by `l` broke; Luau breaks `l` right after this hook.
    with_sched(l, |s| {
        let frames = match s.lua() {
            Ok(lua) => unsafe { inspect::capture(&lua, &s.debug, l) },
            Err(_) => Vec::new(),
        };
        s.debug.paused.borrow_mut().push(Broken {
            state: l,
            resumer: Resumer::Coroutine,
            how: How::Call,
            frames,
            writes: Vec::new(),
        });
    });
}

unsafe extern "C-unwind" fn hook_error(l: *mut ffi::lua_State) {
    with_sched(l, |s| unsafe { on_error(s, l) });
}

unsafe fn on_error(s: &Scheduler, l: *mut ffi::lua_State) {
    let d = &s.debug;
    if !d.break_on_error.get() || d.suppress.get() > 0 || s.timeout_loc.borrow().is_some() {
        return;
    }
    if !unsafe { unhandled(d, l) } || !d.breakable(l) {
        return;
    }
    let mut len = 0;
    let p = unsafe { ffi::lua_tolstring(l, -1, &mut len) };
    let message = if p.is_null() {
        "error".to_string()
    } else {
        // SAFETY: Luau strings are valid for `len` bytes.
        let bytes = unsafe { std::slice::from_raw_parts(p.cast::<u8>(), len) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    unsafe {
        d.pause_here(
            s,
            l,
            How::Error(message.clone()),
            PauseReason::Error(message),
        )
    };
}

/// True when the only protected call on `l`'s stack is the scheduler's guard (the bottom
/// `xpcall`): no script `pcall` would handle the error.
unsafe fn unhandled(d: &DebugCore, l: *mut ffi::lua_State) -> bool {
    let [pcall, xpcall] = d.pcalls.get();
    let depth = unsafe { ffi::lua_stackdepth(l) };
    let mut found = 0;
    let mut at = -1;
    for level in 0..depth {
        let mut ar = unsafe { std::mem::zeroed::<ffi::lua_Debug>() };
        if unsafe { ffi::lua_checkstack(l, 1) } == 0
            || unsafe { ffi::lua_getinfo(l, level, c"f".as_ptr(), &mut ar) } == 0
        {
            break;
        }
        let f = unsafe { ffi::lua_topointer(l, -1) } as usize;
        unsafe { ffi::lua_pop(l, 1) };
        if f == pcall || f == xpcall {
            found += 1;
            at = level;
        }
    }
    found == 1 && at == depth - 2
}

unsafe extern "C-unwind" fn hook_preresume(l: *mut ffi::lua_State) {
    with_sched(l, |s| {
        let by_rust = s.debug.rust_resume.replace(false);
        s.debug.active.borrow_mut().push((l, by_rust));
    });
}

unsafe extern "C-unwind" fn hook_postresume(l: *mut ffi::lua_State) {
    with_sched(l, |s| {
        let mut active = s.debug.active.borrow_mut();
        if let Some(i) = active.iter().rposition(|(t, _)| *t == l) {
            active.truncate(i);
        }
    });
}

/// `task.spawn`'s helper: breaks the spawning thread after the spawned one broke, so the
/// spawner pauses before it runs on. Does nothing where a break is not possible.
pub(crate) unsafe extern "C-unwind" fn break_spawner(l: *mut ffi::lua_State) -> c_int {
    let mut brk = false;
    with_sched(l, |s| {
        let d = &s.debug;
        if d.is_paused() && d.breakable(l) {
            let frames = match s.lua() {
                Ok(lua) => unsafe { inspect::capture(&lua, d, l) },
                Err(_) => Vec::new(),
            };
            d.paused.borrow_mut().push(Broken {
                state: l,
                resumer: Resumer::Coroutine,
                how: How::Call,
                frames,
                writes: Vec::new(),
            });
            brk = true;
        }
    });
    if brk { unsafe { ffi::lua_break(l) } } else { 0 }
}

/// Resumes `thread` with `args` (the debugger's resume; mlua's `Thread::resume` treats a
/// break as an error). Values come back like `Thread::resume`'s.
pub(crate) fn raw_resume(
    lua: &Lua,
    d: &DebugCore,
    thread: &Thread,
    args: Vec<Value>,
) -> mlua::Result<Outcome> {
    let co = thread.to_pointer() as *mut ffi::lua_State;
    // SAFETY: `co` is the live thread behind `thread`.
    let status = unsafe { ffi::lua_status(co) };
    if status != ffi::LUA_BREAK && thread.status() != mlua::thread::ThreadStatus::Resumable {
        return Err(mlua::Error::CoroutineUnresumable);
    }
    let step = d.wants_step(co);
    let mut result = ffi::LUA_OK;
    // SAFETY: the closure moves the arguments to `co`, resumes it and moves its results
    // (or error object) back; nothing else touches either stack meanwhile.
    let values: MultiValue = unsafe {
        lua.exec_raw(MultiValue::from_vec(args), |st| {
            let nargs = ffi::lua_gettop(st);
            if ffi::lua_checkstack(co, nargs + 1) == 0 {
                result = ffi::LUA_ERRMEM;
                ffi::lua_settop(st, 0);
                return;
            }
            ffi::lua_xmove(st, co, nargs);
            ffi::lua_singlestep(co, c_int::from(step));
            d.rust_resume.set(true);
            result = ffi::lua_resume_(co, st, nargs);
            d.rust_resume.set(false);
            match result {
                ffi::LUA_OK | ffi::LUA_YIELD => {
                    let n = ffi::lua_gettop(co);
                    if ffi::lua_checkstack(st, n + 1) != 0 {
                        ffi::lua_xmove(co, st, n);
                    }
                }
                ffi::LUA_BREAK => {}
                _ => ffi::lua_xmove(co, st, 1),
            }
        })?
    };
    match result {
        ffi::LUA_OK | ffi::LUA_YIELD => Ok(Outcome::Done(values)),
        ffi::LUA_BREAK => Ok(Outcome::Break),
        code => Err(match values.into_iter().next() {
            Some(Value::Error(e)) => *e,
            Some(Value::String(s)) if code == ffi::LUA_ERRMEM => {
                mlua::Error::MemoryError(s.to_string_lossy())
            }
            Some(Value::String(s)) => mlua::Error::RuntimeError(s.to_string_lossy()),
            Some(other) => mlua::Error::RuntimeError(format!("{} error", other.type_name())),
            None => mlua::Error::RuntimeError("unknown error".into()),
        }),
    }
}

/// Resumes the paused coroutine `co` and leaves its results on its stack, for the
/// `coroutine.resume` continuation of its resumer. Returns the resume status.
fn raw_resume_keep(lua: &Lua, d: &DebugCore, co: *mut ffi::lua_State) -> c_int {
    let step = d.wants_step(co);
    let mut result = ffi::LUA_ERRRUN;
    // SAFETY: `co` is kept alive by its paused resumer; only its own stack is touched.
    let ran = unsafe {
        lua.exec_raw::<()>((), |st| {
            ffi::lua_singlestep(co, c_int::from(step));
            d.rust_resume.set(true);
            result = ffi::lua_resume_(co, st, 0);
            d.rust_resume.set(false);
        })
    };
    if ran.is_err() {
        return ffi::LUA_ERRRUN;
    }
    result
}

/// The host-level debugger of a [`ScriptHost`](crate::ScriptHost): breakpoints by script,
/// pause and step, and inspection of the paused frames. Get it with
/// [`ScriptHost::debugger`](crate::ScriptHost::debugger).
///
/// Breakpoints are kept by the host and apply to every start of the game. While the game
/// is paused, [`ScriptHost::is_paused`](crate::ScriptHost::is_paused) is true and
/// [`ScriptHost::step`](crate::ScriptHost::step) runs nothing; the runtime then also stops
/// physics and characters.
pub struct Debugger<'a> {
    pub(crate) host: &'a mut crate::ScriptHost,
}
