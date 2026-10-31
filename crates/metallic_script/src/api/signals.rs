//! Signals (`:Connect`, `:Once`, `:Wait`, `RBXScriptConnection:Disconnect`) and how the host
//! fires them.
//!
//! A signal is identified by an instance and an [`EventKind`]. Each VM keeps its own
//! [`Hub`] of connected Lua functions and of threads parked in `:Wait()`. Firing takes a
//! snapshot of the live connections and waiters first, so a `Connect` or `Wait` made by a
//! handler does not run in the same dispatch, and a `Disconnect` made by a handler stops
//! later handlers of that dispatch from running. A `:Once` connection disconnects itself
//! just before its one call. Waiters resume after the handlers, with the same arguments,
//! under the normal resume budget; they are dropped with their script
//! ([`ApiState::stop_owner`]) or their instance ([`ApiState::prune_hub`]).
//!
//! Limits: a signal holds at most [`MAX_CONNECTIONS`] connections (and as many waiters); all
//! handlers of one `step` share one time budget (the rest wait, in order, for the next step);
//! a script whose handlers time out [`MAX_CONSECUTIVE_TIMEOUTS`] times in a row is
//! disconnected.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use metallic_core::InstanceId;
use mlua::{
    Function, Lua, MetaMethod, MultiValue, Table, Thread, UserData, UserDataFields,
    UserDataMethods, UserDataRef, Value,
};

use super::{ApiState, errors, lua_error};
use crate::task_lib::TIMEOUT_MSG;
use crate::vm::{STEP_BUDGET_SEC, format_error};

/// Most connections one signal accepts (and most threads waiting on it).
pub(crate) const MAX_CONNECTIONS: usize = 10_000;
/// Timeouts in a row after which a script's connections are removed.
const MAX_CONSECUTIVE_TIMEOUTS: u32 = 3;
/// Most handler calls kept waiting for a later step.
const MAX_DEFERRED: usize = 10_000;

/// The events scripts can connect to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EventKind {
    /// `BasePart.Touched(hit)`
    Touched,
    /// `BasePart.TouchEnded(hit)`
    TouchEnded,
    /// `Humanoid.Died()`
    Died,
    /// `Player.CharacterAdded(model)`
    CharacterAdded,
    /// `Players.PlayerAdded(player)`
    PlayerAdded,
    /// `RunService.Heartbeat(dt)`
    Heartbeat,
    /// `RemoteEvent.OnServerEvent(player, ...)`
    OnServerEvent,
    /// `RemoteEvent.OnClientEvent(...)`
    OnClientEvent,
    /// `InputAction.Pressed()`
    Pressed,
    /// `InputAction.Released()`
    Released,
    /// `InputAction.Triggered(value)`
    Triggered,
    /// `InputAction.StateChanged(value)`
    StateChanged,
    /// `UserInputService.LastInputTypeChanged(type)`
    LastInputTypeChanged,
    /// `Instance.AttributeChanged(name)` for id 0, `GetAttributeChangedSignal(name)` for the
    /// interned name id of that attribute (no arguments).
    AttributeChanged(u32),
    /// `CollectionService:GetInstanceAddedSignal(tag)`: the interned tag id; the instance is the
    /// argument.
    TagAdded(u32),
    /// `CollectionService:GetInstanceRemovedSignal(tag)`.
    TagRemoved(u32),
    /// A GUI element's pointer event (`Activated`, `MouseEnter`, ...).
    Gui(super::gui::GuiEventKind),
}

/// One signal: an event of one instance.
pub(crate) type SignalKey = (InstanceId, EventKind);

struct Connection {
    id: u64,
    func: Function,
    alive: Rc<Cell<bool>>,
    /// The owner id of the script that connected it (0 for none).
    owner: u64,
    /// Made by `:Once`: disconnected just before its first call.
    once: bool,
}

/// A connection as a dispatch saw it when it started.
struct Snapshot {
    id: u64,
    func: Function,
    alive: Rc<Cell<bool>>,
    owner: u64,
    once: bool,
}

/// A thread parked in `signal:Wait()`.
struct SignalWaiter {
    thread: Thread,
    /// The owner id of the script that waits (0 for none).
    owner: u64,
}

/// What a dispatch runs: a connected handler, or a waiting thread to resume.
enum Call {
    Handler(Function),
    Resume(Thread),
}

/// A handler call or waiter resume kept for the next step because this one ran out of budget.
pub(crate) struct Deferred {
    call: Call,
    alive: Rc<Cell<bool>>,
    owner: u64,
    args: Vec<Value>,
}

/// The connections and `:Wait()` threads of one VM.
#[derive(Default)]
pub(crate) struct Hub {
    connections: RefCell<HashMap<SignalKey, Vec<Connection>>>,
    waiters: RefCell<HashMap<SignalKey, Vec<SignalWaiter>>>,
    next_id: Cell<u64>,
}

impl Hub {
    fn connect(
        &self,
        key: SignalKey,
        func: Function,
        owner: u64,
        once: bool,
    ) -> mlua::Result<(u64, Rc<Cell<bool>>)> {
        let mut all = self.connections.borrow_mut();
        let list = all.entry(key).or_default();
        if list.len() >= MAX_CONNECTIONS {
            return Err(lua_error("too many connections"));
        }
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let alive = Rc::new(Cell::new(true));
        list.push(Connection {
            id,
            func,
            alive: alive.clone(),
            owner,
            once,
        });
        Ok((id, alive))
    }

    fn disconnect(&self, key: SignalKey, id: u64) {
        if let Some(list) = self.connections.borrow_mut().get_mut(&key) {
            list.retain(|c| c.id != id);
        }
    }

    /// How many handlers and `:Wait()` threads `key` has (both are calls a fire makes).
    fn count(&self, key: SignalKey) -> usize {
        self.connections.borrow().get(&key).map_or(0, Vec::len)
            + self.waiters.borrow().get(&key).map_or(0, Vec::len)
    }

    fn snapshot(&self, key: SignalKey) -> Vec<Snapshot> {
        self.connections
            .borrow()
            .get(&key)
            .map(|list| {
                list.iter()
                    .map(|c| Snapshot {
                        id: c.id,
                        func: c.func.clone(),
                        alive: c.alive.clone(),
                        owner: c.owner,
                        once: c.once,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Parks `thread` until `key` next fires.
    fn add_waiter(&self, key: SignalKey, thread: Thread, owner: u64) -> mlua::Result<()> {
        let mut all = self.waiters.borrow_mut();
        let list = all.entry(key).or_default();
        if list.len() >= MAX_CONNECTIONS {
            return Err(lua_error("too many threads waiting on this signal"));
        }
        list.push(SignalWaiter { thread, owner });
        Ok(())
    }

    /// Removes and returns the threads waiting on `key`.
    fn take_waiters(&self, key: SignalKey) -> Vec<SignalWaiter> {
        self.waiters.borrow_mut().remove(&key).unwrap_or_default()
    }

    /// Parks `waiters` on `key` again, ahead of any that started waiting since.
    fn restore_waiters(&self, key: SignalKey, waiters: impl Iterator<Item = SignalWaiter>) {
        let mut all = self.waiters.borrow_mut();
        let list = all.entry(key).or_default();
        let newer = std::mem::take(list);
        list.extend(waiters);
        list.extend(newer);
    }

    /// How many threads wait on any signal.
    fn waiter_count(&self) -> usize {
        self.waiters.borrow().values().map(Vec::len).sum()
    }

    /// Disconnects everything `owner` connected and drops its waiting threads.
    fn remove_owner(&self, owner: u64) {
        for list in self.connections.borrow_mut().values_mut() {
            list.retain(|c| {
                let keep = c.owner != owner;
                if !keep {
                    c.alive.set(false);
                }
                keep
            });
        }
        let mut waiters = self.waiters.borrow_mut();
        for list in waiters.values_mut() {
            list.retain(|w| w.owner != owner);
        }
        waiters.retain(|_, list| !list.is_empty());
    }

    /// Drops the connections and waiting threads of instances for which `is_alive` is false.
    pub(crate) fn prune(&self, is_alive: impl Fn(InstanceId) -> bool) {
        self.connections.borrow_mut().retain(|(id, _), list| {
            let keep = is_alive(*id);
            if !keep {
                for c in list.iter() {
                    c.alive.set(false);
                }
            }
            keep
        });
        self.waiters
            .borrow_mut()
            .retain(|(id, _), list| !list.is_empty() && is_alive(*id));
    }
}

/// Per-VM bookkeeping of the step budget, deferred handlers and timeout streaks.
#[derive(Default)]
pub(crate) struct Pacing {
    deadline: Cell<Option<Instant>>,
    deferred: RefCell<VecDeque<Deferred>>,
    streaks: RefCell<HashMap<u64, u32>>,
    warned_full: Cell<bool>,
    /// Set by `hold_step_deadline`: the next `begin_step` keeps the running deadline.
    keep_deadline: Cell<bool>,
    /// How many times a handler's arguments were built (a test hook for the dispatch cost).
    args_built: Cell<u64>,
}

impl ApiState {
    /// Starts the time budget of one `step`.
    pub(crate) fn begin_step(&self) {
        if self.pacing.keep_deadline.replace(false) {
            return;
        }
        self.pacing.deadline.set(Some(
            Instant::now() + Duration::from_secs_f64(STEP_BUDGET_SEC),
        ));
        self.pacing.warned_full.set(false);
    }

    /// Starts a time budget at once, dropping any held one (see `hold_step_deadline`).
    pub(crate) fn restart_step(&self) {
        self.pacing.keep_deadline.set(false);
        self.begin_step();
    }

    /// Lets the next `begin_step` continue this budget, so work done before the `step` of the
    /// same frame and the step itself share one budget.
    pub(crate) fn hold_step_deadline(&self) {
        self.pacing.keep_deadline.set(true);
    }

    /// Ends the step budget (calls outside `step`, like script start-up, are not limited by it).
    pub(crate) fn end_step(&self) {
        self.pacing.deadline.set(None);
    }

    /// How many times handler arguments were built since the VM started.
    pub(crate) fn args_built(&self) -> u64 {
        self.pacing.args_built.get()
    }

    /// How many threads wait in `signal:Wait()`.
    pub(crate) fn signal_waiters(&self) -> usize {
        self.hub.waiter_count()
    }

    fn over_budget(&self) -> bool {
        self.pacing
            .deadline
            .get()
            .is_some_and(|d| Instant::now() >= d)
    }

    /// Whether the lifecycle events (tag added or removed, attribute changed) of a log can be
    /// handed to `handlers` handlers now. They are never dropped: when this step is out of time
    /// or the deferred queue has no room for that many calls, the reader stays at the event and
    /// takes it up again in the next step, in order. The events wait in the world's log (which
    /// holds hundreds of thousands) instead of in the handler queue.
    pub(crate) fn lifecycle_room(&self, handlers: usize) -> bool {
        handlers == 0
            || (!self.over_budget()
                && self.pacing.deferred.borrow().len() + handlers <= MAX_DEFERRED)
    }

    /// How many calls a fire of `key` makes: its handlers plus the threads in `:Wait()`.
    pub(crate) fn handler_count(&self, key: SignalKey) -> usize {
        self.hub.count(key)
    }

    /// Runs handler calls and waiter resumes that an earlier step had no time for, oldest
    /// first.
    pub(crate) fn run_deferred(&self) {
        loop {
            if self.over_budget() {
                return;
            }
            let next = self.pacing.deferred.borrow_mut().pop_front();
            let Some(deferred) = next else { return };
            if deferred.alive.get() {
                self.run_call(deferred.call, deferred.owner, deferred.args);
            }
        }
    }

    /// Calls every handler connected to `key`, then resumes every thread waiting on it.
    /// `args` builds the arguments for one handler or waiter (called once per call, so
    /// tables can be copied afresh). Each handler runs in its own thread under the execution
    /// budget; its errors go to the output sink. Calls that do not fit in the step budget
    /// run in a later step.
    pub(crate) fn fire(&self, key: SignalKey, args: &dyn Fn(&Lua) -> mlua::Result<Vec<Value>>) {
        let handlers = self.hub.snapshot(key);
        let waiters = self.hub.take_waiters(key);
        if handlers.is_empty() && waiters.is_empty() {
            return;
        }
        if self.lua.try_upgrade().is_none() {
            return;
        }
        for handler in handlers {
            if !handler.alive.get() {
                continue;
            }
            let alive = if handler.once {
                // Disconnected before the call: `Connected` is false inside it and no later
                // fire sees it. The call itself still runs, even when it is deferred.
                handler.alive.set(false);
                self.hub.disconnect(key, handler.id);
                Rc::new(Cell::new(true))
            } else {
                handler.alive
            };
            if !self.dispatch_call(Call::Handler(handler.func), alive, handler.owner, args) {
                break;
            }
        }
        let mut waiters = waiters.into_iter();
        while let Some(waiter) = waiters.next() {
            // Its script may have stopped while the handlers ran.
            if !self.budget.can_resume_parked(&waiter.thread) {
                continue;
            }
            let call = Call::Resume(waiter.thread.clone());
            if !self.dispatch_call(call, Rc::new(Cell::new(true)), waiter.owner, args) {
                // No room to queue them: they keep waiting for the next fire.
                self.hub
                    .restore_waiters(key, std::iter::once(waiter).chain(waiters));
                break;
            }
        }
    }

    /// Runs one handler call now, or queues it when this step is out of time. `args` builds
    /// its arguments. Returns false when the queue is full and the rest should be dropped.
    pub(crate) fn dispatch(
        &self,
        func: Function,
        alive: Rc<Cell<bool>>,
        owner: u64,
        args: &dyn Fn(&Lua) -> mlua::Result<Vec<Value>>,
    ) -> bool {
        self.dispatch_call(Call::Handler(func), alive, owner, args)
    }

    fn dispatch_call(
        &self,
        call: Call,
        alive: Rc<Cell<bool>>,
        owner: u64,
        args: &dyn Fn(&Lua) -> mlua::Result<Vec<Value>>,
    ) -> bool {
        let Some(lua) = self.lua.try_upgrade() else {
            return false;
        };
        // Decide first, so a full queue costs no argument copying: once it is full the
        // rest of the handlers are dropped without building anything.
        let defer = self.over_budget();
        if defer && self.pacing.deferred.borrow().len() >= MAX_DEFERRED {
            if !self.pacing.warned_full.replace(true) {
                self.sink
                    .warn("too many script handlers are waiting; some calls were dropped");
            }
            return false;
        }
        self.pacing.args_built.set(self.pacing.args_built.get() + 1);
        let values = match args(&lua) {
            Ok(values) => values,
            Err(e) => {
                self.sink.error(&format_error(&e));
                return true;
            }
        };
        if defer {
            self.pacing.deferred.borrow_mut().push_back(Deferred {
                call,
                alive,
                owner,
                args: values,
            });
        } else {
            self.run_call(call, owner, values);
        }
        true
    }

    fn run_call(&self, call: Call, owner: u64, values: Vec<Value>) {
        match call {
            Call::Handler(func) => self.run_handler(func, owner, values),
            Call::Resume(thread) => self.resume_waiter(&thread, values),
        }
    }

    /// Resumes a thread parked in `signal:Wait()` with the fire's arguments, under the
    /// resume budget. A thread whose script stopped in the meantime stays put.
    fn resume_waiter(&self, thread: &Thread, values: Vec<Value>) {
        if !self.budget.can_resume_parked(thread) {
            return;
        }
        if let Err(e) = self.budget.resume(thread, MultiValue::from_vec(values)) {
            self.sink.error(&e.message);
        }
    }

    /// Runs one handler in its own thread; tracks consecutive timeouts of its script.
    fn run_handler(&self, func: Function, owner: u64, values: Vec<Value>) {
        let name = func.info().short_src.unwrap_or_default();
        let mut all = Vec::with_capacity(values.len() + 1);
        all.push(Value::Function(func));
        all.extend(values);
        let result =
            self.budget
                .call_as::<MultiValue>(owner, &self.spawn, MultiValue::from_vec(all));
        let timed_out = match result {
            Ok(_) => false,
            Err(e) => {
                self.sink.error(&e.message);
                e.message.ends_with(TIMEOUT_MSG)
            }
        };
        if owner == 0 {
            return;
        }
        if !timed_out {
            self.pacing.streaks.borrow_mut().remove(&owner);
            return;
        }
        let streak = {
            let mut streaks = self.pacing.streaks.borrow_mut();
            let n = streaks.entry(owner).or_insert(0);
            *n += 1;
            *n
        };
        if streak >= MAX_CONSECUTIVE_TIMEOUTS {
            self.stop_owner(owner);
            self.sink.error(&format!(
                "{name}: script disconnected after {MAX_CONSECUTIVE_TIMEOUTS} consecutive timeouts"
            ));
        }
    }

    /// Stops a script: removes its connections, queued calls, waits and sleeping threads.
    pub(crate) fn stop_owner(&self, owner: u64) {
        self.hub.remove_owner(owner);
        self.pacing
            .deferred
            .borrow_mut()
            .retain(|d| d.owner != owner);
        self.waiters
            .borrow_mut()
            .retain(|w| self.budget.owner_of(&w.thread) != Some(owner));
        self.pacing.streaks.borrow_mut().remove(&owner);
        self.require.release_owner(owner);
        self.drop_bound_actions(owner);
        self.budget.cancel_owner(owner);
    }

    /// Drops the connections and `:Wait()` threads of instances that no longer exist.
    pub(crate) fn prune_hub(&self) {
        if let Ok(world) = self.world() {
            self.hub.prune(|id| world.is_alive(id));
        }
    }
}

/// The Luau `RBXScriptSignal`.
pub(crate) struct LuaSignal {
    pub(crate) state: Rc<ApiState>,
    pub(crate) key: SignalKey,
}

/// `signal:Connect(fn)` and `signal:Once(fn)`.
fn connect_fn(lua: &Lua, once: bool) -> mlua::Result<Function> {
    errors::guarded_fn(
        lua,
        move |lua, (this, func): (UserDataRef<LuaSignal>, Function)| {
            let owner = this.state.budget.current_owner();
            let (id, alive) = this.state.hub.connect(this.key, func, owner, once)?;
            lua.create_userdata(LuaConnection {
                state: this.state.clone(),
                key: this.key,
                id,
                alive,
            })
        },
    )
}

/// `signal:Wait()`: Luau code, because only Luau code can yield its own thread. Outside a
/// yieldable thread it yields anyway, so it fails exactly like `task.wait` there.
fn wait_fn(lua: &Lua) -> mlua::Result<Function> {
    let register = errors::guarded_fn(
        lua,
        |_, (this, thread): (UserDataRef<LuaSignal>, Thread)| {
            let owner = this.state.budget.current_owner();
            this.state.budget.adopt(&thread)?;
            this.state.hub.add_waiter(this.key, thread, owner)
        },
    )?;
    let coroutine: Table = lua.globals().get("coroutine")?;
    lua.load(
        r#"
local register, yield, running, isyieldable = ...
return function(signal)
    if not isyieldable() then return yield() end
    register(signal, running())
    return yield()
end
"#,
    )
    .set_name("=[engine]")
    .call((
        register,
        coroutine.get::<Function>("yield")?,
        coroutine.get::<Function>("running")?,
        coroutine.get::<Function>("isyieldable")?,
    ))
}

impl UserData for LuaSignal {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "RBXScriptSignal");
        // `Connect`, `Once`, `Wait`; guarded, so a bad argument is a positioned script error.
        fields.add_meta_field_with(MetaMethod::Index, |lua| {
            let members = lua.create_table()?;
            members.set("Connect", connect_fn(lua, false)?)?;
            members.set("Once", connect_fn(lua, true)?)?;
            members.set("Wait", wait_fn(lua)?)?;
            Ok(members)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| {
            Ok("RBXScriptSignal".to_string())
        });
    }
}

/// The Luau `RBXScriptConnection`.
pub(crate) struct LuaConnection {
    state: Rc<ApiState>,
    key: SignalKey,
    id: u64,
    alive: Rc<Cell<bool>>,
}

impl UserData for LuaConnection {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "RBXScriptConnection");
        fields.add_field_method_get("Connected", |_, this| Ok(this.alive.get()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("Disconnect", |_, this, ()| {
            this.alive.set(false);
            this.state.hub.disconnect(this.key, this.id);
            Ok(())
        });
        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| {
            Ok("RBXScriptConnection".to_string())
        });
    }
}
