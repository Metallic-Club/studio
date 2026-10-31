//! The Roblox-named Luau API: `game`, `workspace`, `Instance`, the datatypes (with `CFrame`), services,
//! signals and `RemoteEvent`.
//!
//! Every VM gets one [`ApiState`] that the userdata and functions of this module share. It
//! holds the world, the signal hub and the threads parked in `WaitForChild`. Nothing here
//! keeps a `World` borrow while Lua code runs: handlers may change the world, so the host
//! collects what to call, releases the borrow and only then calls into Lua.

mod cframe;
mod datatypes;
mod errors;
mod input;
mod instance;
mod pivot;
mod remote;
mod services;
mod signals;

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use metallic_core::{DestroyedInstance, InstanceId, World};
use mlua::{Error, Function, Table, WeakLua};

use crate::vm::{Budget, Context, OutputSink, ScriptError, ScriptVm, format_error};

pub use input::{
    BoundResult, InputActionEvent, InputDevice, InputEventKind, InputValue, KEY_CODES,
    MAX_ACTION_EVENTS_PER_STEP, TouchButtonInfo, USER_INPUT_TYPES,
};
pub(crate) use remote::{QueuedRemote, RemoteTarget};
pub(crate) use signals::{EventKind, Hub, Pacing};

/// State shared by the server VM and the client VM of one [`ScriptHost`](crate::ScriptHost).
#[derive(Default)]
pub(crate) struct HostShared {
    /// Remote events fired since the last `step`, delivered by the next one.
    pub(crate) remotes: RefCell<Vec<QueuedRemote>>,
    /// Messages and bytes queued since the last delivery, for the server (0) and client (1) VM.
    pub(crate) queued: Cell<[(usize, usize); 2]>,
    /// Players whose `PlayerAdded` was delivered (what `GetPlayers` returns).
    pub(crate) joined: RefCell<HashSet<InstanceId>>,
    /// The player of the client VM (`Players.LocalPlayer`).
    pub(crate) local_player: Cell<Option<InstanceId>>,
    /// Input events, action states and device flags pushed by the host.
    pub(crate) input: input::InputShared,
    /// What instances destroyed during this step were (from [`World::take_destroyed`]), so a
    /// destroyed instance stays readable. Emptied at the end of every step: by then each Luau
    /// object of one holds its own copy, which goes when the object is collected.
    pub(crate) destroyed: RefCell<HashMap<InstanceId, Rc<DestroyedInstance>>>,
    /// The previous step's table, kept one more step: an instance destroyed by a script during a
    /// step reaches scripts again in the next one (a `TouchEnded` of its character's limbs).
    pub(crate) destroyed_before: RefCell<HashMap<InstanceId, Rc<DestroyedInstance>>>,
    /// Reused buffer for [`World::take_destroyed`].
    pub(crate) taken: RefCell<Vec<DestroyedInstance>>,
}

/// Everything the API functions of one VM need.
pub(crate) struct ApiState {
    pub(crate) world: Rc<RefCell<World>>,
    pub(crate) context: Context,
    pub(crate) budget: Budget,
    pub(crate) sink: Rc<dyn OutputSink>,
    pub(crate) shared: Rc<HostShared>,
    pub(crate) hub: Hub,
    pub(crate) pacing: Pacing,
    pub(crate) lua: WeakLua,
    /// Weak-valued table `instance id -> userdata`, so one instance is one Lua object.
    cache: Table,
    /// `task.spawn`: every handler runs in its own thread, so it may yield.
    spawn: Function,
    /// The VM clock, set by the host on every step.
    pub(crate) now: Rc<Cell<f64>>,
    pub(crate) waiters: Rc<RefCell<Vec<instance::Waiter>>>,
    /// Instance methods by name (`FindFirstChild`, `GetPlayers`, ...).
    methods: HashMap<&'static str, Function>,
    /// `Enum` and its items.
    enums: input::Enums,
    /// Actions bound with `ContextActionService:BindAction`, by name.
    bound: RefCell<HashMap<String, input::Bound>>,
    /// `Cancel` calls for actions unbound while pressed.
    cancels: RefCell<Vec<input::PendingCancel>>,
    /// Hidden actions to destroy once the world is free: `(action, context)`.
    pending_destroy: RefCell<Vec<(InstanceId, InstanceId)>>,
    /// Wraps a bound handler so its `Sink`/`Pass` result reaches the host.
    bind_wrapper: Function,
    /// `UserInputService` and `ContextActionService`.
    input_services: input::InputServices,
}

impl ApiState {
    /// Read access to the world; an error (not a panic) if the host has it borrowed.
    pub(crate) fn world(&self) -> mlua::Result<Ref<'_, World>> {
        self.world
            .try_borrow()
            .map_err(|_| Error::runtime("the world is busy"))
    }

    /// Write access to the world.
    pub(crate) fn world_mut(&self) -> mlua::Result<RefMut<'_, World>> {
        self.world
            .try_borrow_mut()
            .map_err(|_| Error::runtime("the world is busy"))
    }
}

fn setup_failed(e: Error) -> ScriptError {
    ScriptError {
        message: format!("script API setup failed: {}", format_error(&e)),
    }
}

/// Installs the whole API into `vm` and returns the state the host drives it with.
pub(crate) fn install(
    vm: &ScriptVm,
    world: Rc<RefCell<World>>,
    shared: Rc<HostShared>,
    sink: Rc<dyn OutputSink>,
) -> Result<Rc<ApiState>, ScriptError> {
    build(vm, world, shared, sink).map_err(setup_failed)
}

fn build(
    vm: &ScriptVm,
    world: Rc<RefCell<World>>,
    shared: Rc<HostShared>,
    sink: Rc<dyn OutputSink>,
) -> mlua::Result<Rc<ApiState>> {
    let lua = vm.lua();
    errors::install(lua)?;
    let cache = lua.create_table()?;
    let weak_values = lua.create_table()?;
    weak_values.set("__mode", "v")?;
    cache.set_metatable(Some(weak_values))?;
    let spawn: Function = lua.globals().get::<Table>("task")?.get("spawn")?;

    let now = Rc::new(Cell::new(0.0));
    let waiters = Rc::new(RefCell::new(Vec::new()));
    let mut methods = HashMap::new();
    let mut raw = instance::methods(lua, &waiters, &now)?;
    raw.extend(services::methods(lua)?);
    raw.extend(remote::methods(lua)?);
    raw.extend(pivot::methods(lua)?);
    raw.extend(input::action_methods(lua)?);
    for (name, function) in raw {
        // `WaitForChild` is Luau code that calls guarded functions itself.
        let function = if name == "WaitForChild" {
            function
        } else {
            errors::guarded(lua, function)?
        };
        methods.insert(name, function);
    }

    let enums = input::Enums::install(lua)?;
    let bind_wrapper = input::bind_wrapper(lua, &enums)?;
    let state = Rc::new(ApiState {
        world,
        context: vm.context(),
        budget: vm.budget(),
        sink,
        shared,
        hub: Hub::default(),
        pacing: Pacing::default(),
        lua: lua.weak(),
        cache,
        spawn,
        now,
        waiters,
        methods,
        enums,
        bound: RefCell::new(HashMap::new()),
        cancels: RefCell::new(Vec::new()),
        pending_destroy: RefCell::new(Vec::new()),
        bind_wrapper,
        input_services: input::InputServices::new(),
    });
    input::install_services(lua, &state)?;

    let set = |name: &str, value: mlua::Value| vm.set_global(name, value);
    let to_err = |e: ScriptError| Error::runtime(e.message);
    set(
        "Instance",
        mlua::Value::Table(instance::constructor(lua, &state)?),
    )
    .map_err(to_err)?;
    set("CFrame", mlua::Value::Table(cframe::constructor(lua)?)).map_err(to_err)?;
    for (name, table) in datatypes::constructors(lua)? {
        set(name, mlua::Value::Table(table)).map_err(to_err)?;
    }
    set("Enum", mlua::Value::Table(state.enums.table.clone())).map_err(to_err)?;
    set("game", mlua::Value::UserData(services::game(lua, &state)?)).map_err(to_err)?;
    let workspace = state.world()?.root("Workspace");
    let workspace = match workspace {
        Some(id) => mlua::Value::UserData(state.wrap(lua, id)?),
        None => mlua::Value::Nil,
    };
    set("workspace", workspace).map_err(to_err)?;
    Ok(state)
}

/// Makes a Lua error from anything printable.
pub(crate) fn lua_error(message: impl std::fmt::Display) -> Error {
    Error::runtime(message.to_string())
}
