//! The Luau `Instance`: a light userdata around an [`InstanceId`], member lookup (methods,
//! events, registry properties, children), `Instance.new` and `WaitForChild`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use metallic_core::classes::{find_property, is_subclass};
use metallic_core::transform;
use metallic_core::{CoreError, InstanceId, Value as CoreValue};
use mlua::{
    AnyUserData, Function, Lua, MetaMethod, Table, Thread, UserData, UserDataFields,
    UserDataMethods, UserDataRef, Value,
};

use super::cframe::{LuaCFrame, cframe};
use super::datatypes::lua_to_value;
use super::errors;
use super::signals::{EventKind, LuaSignal};
use super::{ApiState, lua_error};
use crate::vm::format_error;

/// How long `WaitForChild` without a timeout waits before it warns (like Roblox).
const INFINITE_YIELD_WARN_SEC: f64 = 5.0;

/// Methods that exist only on some classes: `(method, class)`; a method listed more than once
/// exists on each of those classes. Everything else in the method table is available on every
/// instance.
const CLASS_METHODS: &[(&str, &str)] = &[
    ("GetService", "DataModel"),
    ("GetPlayers", "Players"),
    ("GetPlayerFromCharacter", "Players"),
    ("IsServer", "RunService"),
    ("IsClient", "RunService"),
    ("TakeDamage", "Humanoid"),
    ("FireServer", "RemoteEvent"),
    ("FireClient", "RemoteEvent"),
    ("FireAllClients", "RemoteEvent"),
    ("GetState", "InputAction"),
    ("GetHoldProgress", "InputAction"),
    // PVInstance methods: BasePart and Model (PVInstance itself is not a registry class).
    ("GetPivot", "BasePart"),
    ("GetPivot", "Model"),
    ("PivotTo", "BasePart"),
    ("PivotTo", "Model"),
    ("Resize", "BasePart"),
    ("ScaleTo", "Model"),
    ("GetScale", "Model"),
    ("GetBoundingBox", "Model"),
    ("GetExtentsSize", "Model"),
];

/// The event named `name` on an instance of `class`, if there is one.
fn event_kind(class: &str, name: &str) -> Option<EventKind> {
    match name {
        "Touched" if is_subclass(class, "BasePart") => Some(EventKind::Touched),
        "Died" if class == "Humanoid" => Some(EventKind::Died),
        "CharacterAdded" if class == "Player" => Some(EventKind::CharacterAdded),
        "PlayerAdded" if class == "Players" => Some(EventKind::PlayerAdded),
        "Heartbeat" if class == "RunService" => Some(EventKind::Heartbeat),
        "OnServerEvent" if class == "RemoteEvent" => Some(EventKind::OnServerEvent),
        "OnClientEvent" if class == "RemoteEvent" => Some(EventKind::OnClientEvent),
        "Pressed" if class == "InputAction" => Some(EventKind::Pressed),
        "Released" if class == "InputAction" => Some(EventKind::Released),
        "Triggered" if class == "InputAction" => Some(EventKind::Triggered),
        "StateChanged" if class == "InputAction" => Some(EventKind::StateChanged),
        _ => None,
    }
}

/// A thread parked in `WaitForChild`.
pub(crate) struct Waiter {
    pub(crate) thread: Thread,
    parent: InstanceId,
    name: String,
    started: f64,
    deadline: Option<f64>,
    warned: bool,
}

/// The Luau value of one instance. Cheap: an id plus the VM's shared state.
pub(crate) struct LuaInstance {
    pub(crate) id: InstanceId,
    pub(crate) state: Rc<ApiState>,
    /// Last known name, so a destroyed instance can still be named in errors.
    name_hint: RefCell<String>,
}

fn core_error(e: CoreError) -> mlua::Error {
    lua_error(e)
}

impl LuaInstance {
    fn live(&self) -> mlua::Result<()> {
        if self.state.world()?.is_alive(self.id) {
            Ok(())
        } else {
            Err(lua_error(format!(
                "{} has been destroyed",
                self.name_hint.borrow()
            )))
        }
    }

    fn class(&self) -> mlua::Result<&'static str> {
        self.state
            .world()?
            .class_name(self.id)
            .ok_or_else(|| lua_error(format!("{} has been destroyed", self.name_hint.borrow())))
    }

    /// Errors unless this instance is (a subclass of) `class`.
    pub(crate) fn expect_class(&self, class: &str, method: &str) -> mlua::Result<()> {
        let actual = self.class()?;
        if is_subclass(actual, class) {
            Ok(())
        } else {
            Err(not_a_member(method, actual))
        }
    }

    /// The current name, or the last known one once destroyed.
    pub(crate) fn name(&self) -> String {
        match self.state.world() {
            Ok(w) if w.is_alive(self.id) => match w.get(self.id, "Name") {
                Ok(CoreValue::String(s)) => s,
                _ => self.name_hint.borrow().clone(),
            },
            _ => self.name_hint.borrow().clone(),
        }
    }
}

fn not_a_member(member: &str, class: &str) -> mlua::Error {
    lua_error(format!("{member} is not a valid member of {class}"))
}

impl ApiState {
    /// The Luau object of `id`; asking twice gives the same object.
    pub(crate) fn wrap(self: &Rc<Self>, lua: &Lua, id: InstanceId) -> mlua::Result<AnyUserData> {
        // Keyed by the slot index (exact in a Lua number); the stored id tells generations apart.
        let slot = (id.to_bits() & 0xffff_ffff) as f64;
        if let Value::UserData(ud) = self.cache.get::<Value>(slot)?
            && ud.borrow::<LuaInstance>().is_ok_and(|i| i.id == id)
        {
            return Ok(ud);
        }
        let name = match self.world()?.get(id, "Name") {
            Ok(CoreValue::String(s)) => s,
            _ => String::new(),
        };
        let ud = lua.create_userdata(LuaInstance {
            id,
            state: self.clone(),
            name_hint: RefCell::new(name),
        })?;
        self.cache.set(slot, ud.clone())?;
        Ok(ud)
    }

    /// Like [`ApiState::wrap`], nil for `None`.
    pub(crate) fn wrap_opt(
        self: &Rc<Self>,
        lua: &Lua,
        id: Option<InstanceId>,
    ) -> mlua::Result<Value> {
        match id {
            Some(id) => Ok(Value::UserData(self.wrap(lua, id)?)),
            None => Ok(Value::Nil),
        }
    }

    /// Resumes `WaitForChild` threads whose child appeared or whose timeout passed, and
    /// warns once about waits without a timeout that last too long.
    pub(crate) fn wake_waiters(self: &Rc<Self>) {
        let waiting: Vec<Waiter> = std::mem::take(&mut *self.waiters.borrow_mut());
        if waiting.is_empty() {
            return;
        }
        let Some(lua) = self.lua.try_upgrade() else {
            return;
        };
        let now = self.now.get();
        let mut still_waiting = Vec::new();
        for mut waiter in waiting {
            let found = match self.world() {
                Ok(w) if !w.is_alive(waiter.parent) => continue,
                Ok(w) => w.find_first_child(waiter.parent, &waiter.name),
                Err(_) => {
                    still_waiting.push(waiter);
                    continue;
                }
            };
            let answer = match found {
                Some(child) => self.wrap(&lua, child).map(Value::UserData),
                None if waiter.deadline.is_some_and(|d| now >= d) => Ok(Value::Nil),
                None => {
                    if waiter.deadline.is_none()
                        && !waiter.warned
                        && now - waiter.started >= INFINITE_YIELD_WARN_SEC
                    {
                        waiter.warned = true;
                        let owner = self
                            .world()
                            .map(|w| w.full_name(waiter.parent))
                            .unwrap_or_default();
                        self.sink.warn(&format!(
                            "Infinite yield possible on '{owner}:WaitForChild(\"{}\")'",
                            waiter.name
                        ));
                    }
                    still_waiting.push(waiter);
                    continue;
                }
            };
            let result = answer
                .map_err(|e| crate::vm::ScriptError {
                    message: format_error(&e),
                })
                .and_then(|v| self.budget.resume(&waiter.thread, v));
            if let Err(e) = result {
                self.sink.error(&e.message);
            }
        }
        // Waits registered while threads resumed are already in the list.
        self.waiters.borrow_mut().extend(still_waiting);
    }

    fn get_property(self: &Rc<Self>, lua: &Lua, id: InstanceId, key: &str) -> mlua::Result<Value> {
        let value = self.world()?.get(id, key).map_err(core_error)?;
        self.value_to_lua(lua, value)
    }
}

impl UserData for LuaInstance {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "Instance");
        fields.add_meta_field_with(MetaMethod::Index, |lua| {
            errors::guarded_fn(
                lua,
                |lua, (this, key): (UserDataRef<LuaInstance>, Value)| index(lua, &this, key),
            )
        });
        fields.add_meta_field_with(MetaMethod::NewIndex, |lua| {
            errors::guarded_fn(
                lua,
                |_, (this, key, value): (UserDataRef<LuaInstance>, Value, Value)| {
                    new_index(&this, key, &value)
                },
            )
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.name()));
    }
}

/// Owns the Lua string guard for the entire member lookup. Invalid UTF-8 keeps
/// the existing replacement-character lookup/error behavior via an owned fallback.
enum MemberName {
    Borrowed(mlua::BorrowedStr),
    Lossy(String),
}

impl AsRef<str> for MemberName {
    fn as_ref(&self) -> &str {
        match self {
            Self::Borrowed(value) => value.as_ref(),
            Self::Lossy(value) => value.as_str(),
        }
    }
}

fn member_name(key: &Value) -> mlua::Result<MemberName> {
    match key {
        Value::String(s) => {
            // Diagnostic-only switch lets one warmed VM alternate the exact old
            // conversion and the proposal; no branch is present in production.
            #[cfg(test)]
            if read_probe::owned_baseline() {
                return Ok(MemberName::Lossy(
                    String::from_utf8_lossy(&s.as_bytes()).into_owned(),
                ));
            }
            Ok(match s.to_str() {
                Ok(value) => MemberName::Borrowed(value),
                Err(_) => MemberName::Lossy(String::from_utf8_lossy(&s.as_bytes()).into_owned()),
            })
        }
        other => Err(lua_error(format!(
            "an Instance member name must be a string, got {}",
            other.type_name()
        ))),
    }
}

#[cfg(test)]
mod read_probe;

fn index(lua: &Lua, this: &LuaInstance, key: Value) -> mlua::Result<Value> {
    let member = member_name(&key)?;
    let key = member.as_ref();
    this.live()?;
    let state = &this.state;
    let class = this.class()?;
    match key {
        "ClassName" => return Ok(Value::String(lua.create_string(class)?)),
        "Parent" => {
            let parent = state.world()?.parent(this.id);
            return state.wrap_opt(lua, parent);
        }
        "CFrame" if is_subclass(class, "BasePart") => {
            let cf = super::cframe::part_cframe(&*state.world()?, this.id).map_err(core_error)?;
            return Ok(Value::UserData(lua.create_userdata(LuaCFrame(cf))?));
        }
        "WorldPivot" if is_subclass(class, "Model") => {
            let cf = transform::world_pivot(&*state.world()?, this.id).map_err(core_error)?;
            return Ok(Value::UserData(lua.create_userdata(LuaCFrame(cf))?));
        }
        "LocalPlayer" if class == "Players" => {
            let player = match state.context {
                crate::vm::Context::Client => state.shared.local_player.get(),
                crate::vm::Context::Server => None,
            };
            return state.wrap_opt(lua, player);
        }
        _ => {}
    }
    state.check_input_member(class, key)?;
    if let Some(method) = state.methods.get(key) {
        let mut owners = CLASS_METHODS
            .iter()
            .filter(|(name, _)| *name == key)
            .peekable();
        let allowed = owners.peek().is_none() || owners.any(|(_, owner)| is_subclass(class, owner));
        if allowed {
            return Ok(Value::Function(method.clone()));
        }
    }
    if let Some(kind) = event_kind(class, key) {
        use crate::vm::Context;
        match (kind, state.context) {
            (EventKind::OnServerEvent, Context::Client) => {
                return Err(lua_error("OnServerEvent can only be used from the server"));
            }
            (EventKind::OnClientEvent, Context::Server) => {
                return Err(lua_error("OnClientEvent can only be used from the client"));
            }
            _ => {}
        }
        return Ok(Value::UserData(lua.create_userdata(LuaSignal {
            state: state.clone(),
            key: (this.id, kind),
        })?));
    }
    if find_property(class, key).is_some_and(|d| !d.hidden) {
        return state.get_property(lua, this.id, key);
    }
    let child = state.world()?.find_first_child(this.id, key);
    match child {
        Some(child) => Ok(Value::UserData(state.wrap(lua, child)?)),
        None => Err(not_a_member(key, class)),
    }
}

fn new_index(this: &LuaInstance, key: Value, value: &Value) -> mlua::Result<()> {
    let member = member_name(&key)?;
    let key = member.as_ref();
    this.live()?;
    let class = this.class()?;
    match key {
        "ClassName" => Err(lua_error("ClassName is read-only")),
        "Parent" => set_parent(this, value),
        "CFrame" if is_subclass(class, "BasePart") => {
            let cf = cframe(value).ok_or_else(|| {
                lua_error(format!(
                    "invalid value for property CFrame: expected CFrame, got {}",
                    value.type_name()
                ))
            })?;
            super::cframe::set_part_cframe(&mut *this.state.world_mut()?, this.id, cf)
                .map_err(core_error)
        }
        "WorldPivot" if is_subclass(class, "Model") => {
            let cf = cframe(value).ok_or_else(|| {
                lua_error(format!(
                    "invalid value for property WorldPivot: expected CFrame, got {}",
                    value.type_name()
                ))
            })?;
            transform::set_world_pivot(&mut *this.state.world_mut()?, this.id, cf)
                .map_err(core_error)
        }
        _ => {
            let Some(def) = find_property(class, key).filter(|d| !d.hidden) else {
                return if this.state.methods.contains_key(key) || event_kind(class, key).is_some() {
                    Err(lua_error(format!("{key} is read-only")))
                } else {
                    Err(not_a_member(key, class))
                };
            };
            let converted = lua_to_value(value, def.ty, key)?;
            this.state
                .world_mut()?
                .set(this.id, key, converted)
                .map_err(core_error)?;
            if key == "Name"
                && let Value::String(s) = value
            {
                *this.name_hint.borrow_mut() = String::from_utf8_lossy(&s.as_bytes()).into_owned();
            }
            Ok(())
        }
    }
}

fn set_parent(this: &LuaInstance, value: &Value) -> mlua::Result<()> {
    let parent = match value {
        Value::Nil => None,
        Value::UserData(ud) => match ud.borrow::<LuaInstance>() {
            Ok(p) => {
                p.live()?;
                Some(p.id)
            }
            Err(_) => return Err(bad_parent(value)),
        },
        other => return Err(bad_parent(other)),
    };
    let mut world = this.state.world_mut()?;
    if world.is_builtin(this.id) {
        return Err(lua_error(format!(
            "The Parent property of {} is locked",
            this.name()
        )));
    }
    world.set_parent(this.id, parent).map_err(core_error)
}

fn bad_parent(value: &Value) -> mlua::Error {
    lua_error(format!(
        "invalid value for property Parent: expected Instance, got {}",
        value.type_name()
    ))
}

/// Methods every instance has, and the Lua-side `WaitForChild`.
pub(crate) fn methods(
    lua: &Lua,
    waiters: &Rc<RefCell<Vec<Waiter>>>,
    now: &Rc<Cell<f64>>,
) -> mlua::Result<Vec<(&'static str, Function)>> {
    let find = lua.create_function(
        |lua, (this, name, recursive): (UserDataRef<LuaInstance>, String, Option<bool>)| {
            this.live()?;
            let world = this.state.world()?;
            let found = if recursive == Some(true) {
                descendants(&world, this.id).into_iter().find(
                    |&d| matches!(world.get(d, "Name"), Ok(CoreValue::String(ref s)) if *s == name),
                )
            } else {
                world.find_first_child(this.id, &name)
            };
            drop(world);
            this.state.wrap_opt(lua, found)
        },
    )?;
    let children = lua.create_function(|lua, this: UserDataRef<LuaInstance>| {
        this.live()?;
        let ids = this.state.world()?.children(this.id).to_vec();
        id_list(lua, &this.state, &ids)
    })?;
    let all_descendants = lua.create_function(|lua, this: UserDataRef<LuaInstance>| {
        this.live()?;
        let ids = {
            let world = this.state.world()?;
            descendants(&world, this.id)
        };
        id_list(lua, &this.state, &ids)
    })?;
    let is_a = lua.create_function(|_, (this, class): (UserDataRef<LuaInstance>, String)| {
        this.live()?;
        let world = this.state.world()?;
        Ok(if class == "PVInstance" {
            world.is_a(this.id, "BasePart") || world.is_a(this.id, "Model")
        } else {
            world.is_a(this.id, &class)
        })
    })?;
    let destroy = lua.create_function(|_, this: UserDataRef<LuaInstance>| {
        let mut world = this.state.world_mut()?;
        if !world.is_alive(this.id) {
            return Ok(());
        }
        if world.is_builtin(this.id) {
            return Err(lua_error(format!("{} cannot be destroyed", this.name())));
        }
        if let Ok(CoreValue::String(s)) = world.get(this.id, "Name") {
            *this.name_hint.borrow_mut() = s;
        }
        world.destroy(this.id);
        Ok(())
    })?;
    let clone = lua.create_function(|lua, this: UserDataRef<LuaInstance>| {
        this.live()?;
        let copy = {
            let mut world = this.state.world_mut()?;
            if world.is_builtin(this.id) {
                return Err(lua_error(format!("{} cannot be cloned", this.name())));
            }
            world.clone_subtree(this.id).map_err(core_error)?
        };
        this.state.wrap(lua, copy)
    })?;

    let (waiters, clock) = (waiters.clone(), now.clone());
    let register = lua.create_function(
        move |_,
              (this, name, timeout, thread): (
            UserDataRef<LuaInstance>,
            String,
            Option<f64>,
            Thread,
        )| {
            this.live()?;
            let started = clock.get();
            waiters.borrow_mut().push(Waiter {
                thread,
                parent: this.id,
                name,
                started,
                deadline: timeout.map(|t| started + t.max(0.0)),
                warned: false,
            });
            Ok(())
        },
    )?;
    let coroutine: Table = lua.globals().get("coroutine")?;
    // Written in Luau because only Luau code can yield its own thread.
    let wait_for_child: Function = lua
        .load(
            r#"
local register, yield, running = ...
return function(inst, name, timeout)
    local found = inst:FindFirstChild(name)
    if found ~= nil then return found end
    register(inst, name, timeout, running())
    return yield()
end
"#,
        )
        .set_name("=[engine]")
        .call((
            errors::guarded(lua, register)?,
            coroutine.get::<Function>("yield")?,
            coroutine.get::<Function>("running")?,
        ))?;

    Ok(vec![
        ("FindFirstChild", find),
        ("GetChildren", children),
        ("GetDescendants", all_descendants),
        ("IsA", is_a),
        ("Destroy", destroy),
        ("Clone", clone),
        ("WaitForChild", wait_for_child),
    ])
}

/// All descendants of `id`, parents before their children, in tree order.
fn descendants(world: &metallic_core::World, id: InstanceId) -> Vec<InstanceId> {
    let mut out = Vec::new();
    let mut stack: Vec<InstanceId> = world.children(id).iter().rev().copied().collect();
    while let Some(cur) = stack.pop() {
        out.push(cur);
        stack.extend(world.children(cur).iter().rev().copied());
    }
    out
}

fn id_list(lua: &Lua, state: &Rc<ApiState>, ids: &[InstanceId]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (i, &id) in ids.iter().enumerate() {
        table.set(i + 1, state.wrap(lua, id)?)?;
    }
    Ok(table)
}

/// The global `Instance` table (`Instance.new`).
pub(crate) fn constructor(lua: &Lua, state: &Rc<ApiState>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let state = state.clone();
    table.set(
        "new",
        errors::guarded_fn(lua, move |lua, (class, parent): (String, Option<Value>)| {
            if super::services::is_service(&class) {
                return Err(lua_error(format!(
                    "Unable to create an Instance of type \"{class}\""
                )));
            }
            let parent = match parent {
                None | Some(Value::Nil) => None,
                Some(Value::UserData(ud)) => match ud.borrow::<LuaInstance>() {
                    Ok(p) => {
                        p.live()?;
                        Some(p.id)
                    }
                    Err(_) => return Err(lua_error("Instance.new: parent must be an Instance")),
                },
                Some(_) => return Err(lua_error("Instance.new: parent must be an Instance")),
            };
            let created = state.world_mut()?.create(&class, parent);
            let id = created.map_err(|e| match e {
                CoreError::UnknownClass(_) => {
                    lua_error(format!("Unable to create an Instance of type \"{class}\""))
                }
                other => core_error(other),
            })?;
            state.wrap(lua, id)
        })?,
    )?;
    table.set_readonly(true);
    Ok(table)
}
