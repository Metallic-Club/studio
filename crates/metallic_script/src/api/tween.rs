//! `TweenService`, `TweenInfo` and `Tween`, with Roblox semantics.
//!
//! `TweenService:Create(instance, info, goals)` makes a [`LuaTween`]; `Play` starts it from the
//! properties' values at that moment. Every playing tween of a VM advances once per host step,
//! before `RunService.Heartbeat` fires, by the step's `dt`: no wall clock, so a headless host
//! gives the same values on every run. Timeline, as in `metallic_anim::TweenInfo`: `DelayTime`
//! once, then `RepeatCount + 1` cycles of `Time` seconds (twice that with `Reverses`, which ends
//! back at the start); a negative `RepeatCount` repeats forever. Curves are
//! `metallic_anim::ease`, Roblox's 11 styles by 3 directions.
//!
//! Tweenable: number, `Vector3`, `Color3`, `UDim2`, `CFrame` (a part's `CFrame`, a model's
//! `WorldPivot` and CFrame properties) and bool, which keeps its start value until the tween
//! ends and then snaps. Playing a tween cancels every other playing tween of the same instance
//! that animates one of its properties (a part's `CFrame` counts as `Position` and
//! `Orientation`). `Completed(playbackState)` fires once when a tween finishes, is cancelled or
//! is overridden, during the next step. Advancing allocates nothing (a CFrame property other
//! than a part's `CFrame` or a model's `WorldPivot` boxes its value on every write, as the
//! property store requires).

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use metallic_anim::{EasingDirection, EasingStyle, PlaybackState, ease};
use metallic_core::classes::{find_property, is_subclass};
use metallic_core::{
    CFrame, Color3, CoreError, InstanceId, UDim2, Value as CoreValue, ValueType, Vec3, World,
    transform,
};
use mlua::{
    AnyUserData, IntoLua, Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods,
    UserDataRef, Value,
};

use super::cframe::cframe;
use super::datatypes::lua_to_value;
use super::input::enum_item;
use super::instance::LuaInstance;
use super::signals::{EventKind, LuaSignal};
use super::{ApiState, errors, lua_error};

/// `Enum.EasingStyle` item names, in Roblox order (the item values are the indices).
pub(crate) const EASING_STYLES: &[&str] = &[
    "Linear",
    "Sine",
    "Back",
    "Quad",
    "Quart",
    "Quint",
    "Bounce",
    "Elastic",
    "Exponential",
    "Circular",
    "Cubic",
];
/// `Enum.EasingDirection` item names.
pub(crate) const EASING_DIRECTIONS: &[&str] = &["In", "Out", "InOut"];
/// `Enum.PlaybackState` item names.
pub(crate) const PLAYBACK_STATES: &[&str] = &[
    "Begin",
    "Delayed",
    "Playing",
    "Paused",
    "Completed",
    "Cancelled",
];

/// Most tweens one VM plays at once; `Play` beyond that is an error.
pub(crate) const MAX_PLAYING_TWEENS: usize = 10_000;

fn state_name(state: PlaybackState) -> &'static str {
    match state {
        PlaybackState::Begin => "Begin",
        PlaybackState::Delayed => "Delayed",
        PlaybackState::Playing => "Playing",
        PlaybackState::Paused => "Paused",
        PlaybackState::Completed => "Completed",
        PlaybackState::Cancelled => "Cancelled",
    }
}

fn style_named(name: &str) -> Option<EasingStyle> {
    EasingStyle::ALL.into_iter().find(|s| s.label() == name)
}

fn direction_named(name: &str) -> Option<EasingDirection> {
    EasingDirection::ALL.into_iter().find(|d| d.label() == name)
}

/// The values of a `TweenInfo`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Info {
    time: f64,
    style: EasingStyle,
    direction: EasingDirection,
    repeat_count: i32,
    reverses: bool,
    delay: f64,
}

impl Default for Info {
    /// `TweenInfo.new()`: 1 s, Quad, Out, no repeat, no reverse, no delay.
    fn default() -> Self {
        Info {
            time: 1.0,
            style: EasingStyle::Quad,
            direction: EasingDirection::Out,
            repeat_count: 0,
            reverses: false,
            delay: 0.0,
        }
    }
}

impl Info {
    /// Linear progress (`0` start, `1` goal) at `elapsed` seconds after `Play`, and whether
    /// the whole timeline is over. Same timeline as `metallic_anim::TweenInfo::sample`, in f64
    /// so a long-running repeat keeps its precision.
    fn progress(&self, elapsed: f64) -> (f64, bool) {
        let t = elapsed - self.delay;
        let end = if self.reverses { 0.0 } else { 1.0 };
        let cycle = if self.reverses {
            2.0 * self.time
        } else {
            self.time
        };
        if cycle <= 0.0 {
            return (end, self.repeat_count >= 0);
        }
        if self.repeat_count >= 0 && t >= cycle * (f64::from(self.repeat_count) + 1.0) {
            return (end, true);
        }
        let in_cycle = t.max(0.0).rem_euclid(cycle);
        let linear = if in_cycle <= self.time {
            in_cycle / self.time
        } else {
            1.0 - (in_cycle - self.time) / self.time
        };
        (linear, false)
    }

    fn eased(&self, linear: f64) -> f64 {
        f64::from(ease(self.style, self.direction, linear as f32))
    }
}

/// Where a tween writes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    /// A stored property.
    Property(&'static str),
    /// A part's `CFrame` (its `Position` and `Orientation`).
    PartCFrame,
    /// A model's `WorldPivot`.
    WorldPivot,
}

impl Target {
    /// True when writing one of them changes the other.
    fn overlaps(self, other: Target) -> bool {
        use Target::*;
        match (self, other) {
            (PartCFrame, PartCFrame) | (WorldPivot, WorldPivot) => true,
            (PartCFrame, Property(p)) | (Property(p), PartCFrame) => {
                p == "Position" || p == "Orientation"
            }
            (Property(a), Property(b)) => a == b,
            _ => false,
        }
    }
}

/// A tweenable value.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TweenValue {
    Number(f64),
    Vector3(Vec3),
    Color3(Color3),
    UDim2(UDim2),
    CFrame(CFrame),
    Bool(bool),
}

fn mix(a: f64, b: f64, t: f64) -> f64 {
    // Exact at both ends (unlike `a + (b - a) * t`).
    a * (1.0 - t) + b * t
}

impl TweenValue {
    fn from_core(value: CoreValue) -> Option<TweenValue> {
        Some(match value {
            CoreValue::Number(n) => TweenValue::Number(n),
            CoreValue::Vector3(v) => TweenValue::Vector3(v),
            CoreValue::Color3(c) => TweenValue::Color3(c),
            CoreValue::UDim2(u) => TweenValue::UDim2(u),
            CoreValue::CFrame(cf) => TweenValue::CFrame(*cf),
            CoreValue::Bool(b) => TweenValue::Bool(b),
            _ => return None,
        })
    }

    fn to_core(self) -> CoreValue {
        match self {
            TweenValue::Number(n) => CoreValue::Number(n),
            TweenValue::Vector3(v) => CoreValue::Vector3(v),
            TweenValue::Color3(c) => CoreValue::Color3(c),
            TweenValue::UDim2(u) => CoreValue::UDim2(u),
            TweenValue::CFrame(cf) => CoreValue::CFrame(Box::new(cf)),
            TweenValue::Bool(b) => CoreValue::Bool(b),
        }
    }

    /// The value at eased progress `t` from `self` to `goal` (bools keep `self`).
    fn lerp(self, goal: TweenValue, t: f64) -> TweenValue {
        match (self, goal) {
            (TweenValue::Number(a), TweenValue::Number(b)) => TweenValue::Number(mix(a, b, t)),
            (TweenValue::Vector3(a), TweenValue::Vector3(b)) => TweenValue::Vector3(Vec3::new(
                mix(a.x, b.x, t),
                mix(a.y, b.y, t),
                mix(a.z, b.z, t),
            )),
            (TweenValue::Color3(a), TweenValue::Color3(b)) => TweenValue::Color3(Color3::new(
                mix(a.r, b.r, t),
                mix(a.g, b.g, t),
                mix(a.b, b.b, t),
            )),
            (TweenValue::UDim2(a), TweenValue::UDim2(b)) => TweenValue::UDim2(UDim2 {
                x_scale: mix(a.x_scale, b.x_scale, t),
                x_offset: mix(a.x_offset, b.x_offset, t),
                y_scale: mix(a.y_scale, b.y_scale, t),
                y_offset: mix(a.y_offset, b.y_offset, t),
            }),
            (TweenValue::CFrame(a), TweenValue::CFrame(b)) => TweenValue::CFrame(a.lerp(b, t)),
            _ => self,
        }
    }
}

/// One animated property.
#[derive(Debug, Clone, Copy)]
struct Prop {
    target: Target,
    goal: TweenValue,
    /// Read from the instance by `Play`.
    start: TweenValue,
    /// A part's `Size`, clamped like a script assignment.
    clamp_size: bool,
}

fn read(world: &World, id: InstanceId, target: Target) -> Result<TweenValue, CoreError> {
    match target {
        Target::Property(name) => {
            TweenValue::from_core(world.get(id, name)?).ok_or(CoreError::InvalidInstance)
        }
        Target::PartCFrame => transform::part_cframe(world, id).map(TweenValue::CFrame),
        Target::WorldPivot => transform::world_pivot(world, id).map(TweenValue::CFrame),
    }
}

fn write(
    world: &mut World,
    id: InstanceId,
    prop: &Prop,
    value: TweenValue,
) -> Result<(), CoreError> {
    match (prop.target, value) {
        (Target::PartCFrame, TweenValue::CFrame(cf)) => transform::set_part_cframe(world, id, cf),
        (Target::WorldPivot, TweenValue::CFrame(cf)) => transform::set_world_pivot(world, id, cf),
        (Target::Property(name), TweenValue::Vector3(v)) if prop.clamp_size => {
            world.set(id, name, CoreValue::Vector3(transform::clamp_part_size(v)))
        }
        (Target::Property(name), value) => world.set(id, name, value.to_core()),
        _ => Err(CoreError::InvalidInstance),
    }
}

/// A tween's state, shared by its Lua object and the playing list.
#[derive(Debug)]
pub(crate) struct TweenData {
    id: u64,
    instance: InstanceId,
    info: Info,
    props: Vec<Prop>,
    elapsed: f64,
    state: PlaybackState,
    /// False once the Lua object was collected: nobody can play it again.
    lua_alive: bool,
}

impl TweenData {
    fn running(&self) -> bool {
        matches!(self.state, PlaybackState::Playing | PlaybackState::Delayed)
    }

    /// Advances by `dt` and writes the properties. Returns false when the tween stopped (it
    /// completed, or its instance is gone and it was cancelled).
    fn advance(&mut self, world: &mut World, dt: f64) -> bool {
        if !self.running() {
            return false;
        }
        self.elapsed += dt;
        if self.elapsed < self.info.delay {
            self.state = PlaybackState::Delayed;
            return true;
        }
        self.state = PlaybackState::Playing;
        let (linear, finished) = self.info.progress(self.elapsed);
        let eased = self.info.eased(linear);
        for prop in &self.props {
            let value = match (finished, prop.start) {
                (true, _) if self.info.reverses => prop.start,
                (true, _) => prop.goal,
                (false, TweenValue::Bool(_)) => continue,
                (false, start) => start.lerp(prop.goal, eased),
            };
            if write(world, self.instance, prop, value).is_err() {
                self.state = PlaybackState::Cancelled;
                self.elapsed = 0.0;
                return false;
            }
        }
        if finished {
            self.state = PlaybackState::Completed;
            return false;
        }
        true
    }
}

type Shared = Rc<RefCell<TweenData>>;

/// The tweens of one VM.
#[derive(Default)]
pub(crate) struct Tweens {
    next_id: Cell<u64>,
    /// Tweens that are playing or delayed, in `Play` order.
    playing: RefCell<Vec<Shared>>,
    /// `Completed` events to fire at the next step.
    pending: RefCell<Vec<(Shared, PlaybackState)>>,
    /// The events being fired (kept to reuse its capacity).
    firing: RefCell<Vec<(Shared, PlaybackState)>>,
    /// Tweens whose Lua object was collected while idle: their connections are dropped.
    collected: RefCell<Vec<u64>>,
    service: OnceCell<AnyUserData>,
    /// `Play`, `Pause`, `Cancel`, `Destroy` and `IsA` of a `Tween`.
    tween_methods: OnceCell<Table>,
    /// `Create` and `GetValue` of the service.
    service_methods: OnceCell<Table>,
}

/// The signal key of a tween's `Completed`: carried by `RunService`, which is never destroyed.
fn completed_key(state: &ApiState, id: u64) -> Option<(InstanceId, EventKind)> {
    let carrier = state.world().ok()?.root("RunService")?;
    Some((carrier, EventKind::TweenCompleted(id)))
}

impl ApiState {
    /// Advances every playing tween by `dt` seconds, then fires the `Completed` events of
    /// tweens that stopped since the last call. Called by the host once per step.
    pub(crate) fn advance_tweens(self: &Rc<Self>, dt: f64) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        if let Ok(mut world) = self.world.try_borrow_mut() {
            let mut pending = self.tweens.pending.borrow_mut();
            self.tweens.playing.borrow_mut().retain(|tween| {
                let mut data = tween.borrow_mut();
                let keep = data.advance(&mut world, dt);
                if !keep {
                    pending.push((tween.clone(), data.state));
                }
                keep
            });
        }
        let carrier = self.world().ok().and_then(|w| w.root("RunService"));
        // Swap buffers, so a handler that plays or cancels tweens queues for the next step and
        // both buffers keep their capacity.
        let mut batch = std::mem::take(&mut *self.tweens.firing.borrow_mut());
        std::mem::swap(&mut batch, &mut *self.tweens.pending.borrow_mut());
        for (tween, playback) in batch.drain(..) {
            let (id, lua_alive) = {
                let data = tween.borrow();
                (data.id, data.lua_alive)
            };
            let Some(carrier) = carrier else { continue };
            let key = (carrier, EventKind::TweenCompleted(id));
            let name = state_name(playback);
            self.fire(key, &|_| Ok(vec![self.enum_value("PlaybackState", name)]));
            if !lua_alive && !tween.borrow().running() {
                self.hub.remove_key(key);
            }
        }
        *self.tweens.firing.borrow_mut() = batch;
        let mut collected = std::mem::take(&mut *self.tweens.collected.borrow_mut());
        if let Some(carrier) = carrier {
            for &id in &collected {
                self.hub
                    .remove_key((carrier, EventKind::TweenCompleted(id)));
            }
        }
        collected.clear();
        let mut slot = self.tweens.collected.borrow_mut();
        if slot.is_empty() {
            *slot = collected;
        }
    }

    fn queue_completed(&self, tween: &Shared, state: PlaybackState) {
        self.tweens
            .pending
            .borrow_mut()
            .push((tween.clone(), state));
    }
}

/// An enum item, looked up in the global `Enum` table when it reaches Lua.
struct EnumName(&'static str, &'static str);

/// The VM's `Enum` table, for [`EnumName`] (set by [`install_service`]).
struct EnumTable(Table);

impl IntoLua for EnumName {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let enums = lua
            .app_data_ref::<EnumTable>()
            .ok_or_else(|| lua_error("the Enum table is not installed"))?;
        let group: Table = enums.0.raw_get(self.0)?;
        group.raw_get(self.1)
    }
}

/// `TweenInfo` as Luau userdata.
#[derive(Clone, Copy)]
pub(crate) struct LuaTweenInfo(Info);

impl UserData for LuaTweenInfo {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "TweenInfo");
        fields.add_field_method_get("Time", |_, this| Ok(this.0.time));
        fields.add_field_method_get("RepeatCount", |_, this| Ok(this.0.repeat_count));
        fields.add_field_method_get("Reverses", |_, this| Ok(this.0.reverses));
        fields.add_field_method_get("DelayTime", |_, this| Ok(this.0.delay));
        fields.add_field_method_get("EasingStyle", |_, this| {
            Ok(EnumName("EasingStyle", this.0.style.label()))
        });
        fields.add_field_method_get("EasingDirection", |_, this| {
            Ok(EnumName("EasingDirection", this.0.direction.label()))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let i = this.0;
            Ok(format!(
                "Time:{} DelayTime:{} RepeatCount:{} Reverses:{} EasingDirection:{} EasingStyle:{}",
                i.time,
                i.delay,
                i.repeat_count,
                i.reverses,
                i.direction.label(),
                i.style.label()
            ))
        });
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: Value| {
            Ok(match other {
                Value::UserData(ud) => ud.borrow::<LuaTweenInfo>().is_ok_and(|o| o.0 == this.0),
                _ => false,
            })
        });
    }
}

fn enum_arg<T>(
    value: &Value,
    group: &str,
    position: usize,
    parse: fn(&str) -> Option<T>,
) -> mlua::Result<Option<T>> {
    if value.is_nil() {
        return Ok(None);
    }
    match enum_item(value) {
        Some((g, name, _)) if g == group => Ok(parse(name)),
        _ => Err(lua_error(format!(
            "TweenInfo.new: argument {position} must be an Enum.{group} item"
        ))),
    }
}

fn number_arg(value: &Value, position: usize) -> mlua::Result<Option<f64>> {
    let n = match value {
        Value::Nil => return Ok(None),
        Value::Integer(i) => *i as f64,
        Value::Number(n) => *n,
        _ => {
            return Err(lua_error(format!(
                "TweenInfo.new: argument {position} must be a number"
            )));
        }
    };
    if !n.is_finite() {
        return Err(lua_error(format!(
            "TweenInfo.new: argument {position} must be finite"
        )));
    }
    Ok(Some(n))
}

/// The global `TweenInfo` table (`TweenInfo.new`).
pub(crate) fn constructor(lua: &Lua) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set(
        "new",
        errors::guarded_fn(lua, |_, args: mlua::Variadic<Value>| {
            let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Nil);
            let mut info = Info::default();
            if let Some(t) = number_arg(&arg(0), 1)? {
                info.time = t.max(0.0);
            }
            if let Some(s) = enum_arg(&arg(1), "EasingStyle", 2, style_named)? {
                info.style = s;
            }
            if let Some(d) = enum_arg(&arg(2), "EasingDirection", 3, direction_named)? {
                info.direction = d;
            }
            if let Some(r) = number_arg(&arg(3), 4)? {
                info.repeat_count = r.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            }
            match arg(4) {
                Value::Nil => {}
                Value::Boolean(b) => info.reverses = b,
                _ => return Err(lua_error("TweenInfo.new: argument 5 must be a boolean")),
            }
            if let Some(d) = number_arg(&arg(5), 6)? {
                info.delay = d.max(0.0);
            }
            Ok(LuaTweenInfo(info))
        })?,
    )?;
    table.set_readonly(true);
    Ok(table)
}

/// A Luau `Tween`.
pub(crate) struct LuaTween {
    state: Rc<ApiState>,
    data: Shared,
}

impl Drop for LuaTween {
    fn drop(&mut self) {
        // Collected, possibly during a GC step inside another borrow: never panic here.
        let Ok(mut data) = self.data.try_borrow_mut() else {
            return;
        };
        data.lua_alive = false;
        if !data.running()
            && let Ok(mut collected) = self.state.tweens.collected.try_borrow_mut()
        {
            collected.push(data.id);
        }
    }
}

impl LuaTween {
    fn play(&self) -> mlua::Result<()> {
        let state = &self.state;
        {
            let mut data = self.data.borrow_mut();
            match data.state {
                PlaybackState::Playing | PlaybackState::Delayed => return Ok(()),
                PlaybackState::Paused => {}
                PlaybackState::Begin | PlaybackState::Completed | PlaybackState::Cancelled => {
                    // Like Roblox, a tween starts from the values the properties have now.
                    let world = state.world()?;
                    let instance = data.instance;
                    for prop in &mut data.props {
                        prop.start = read(&world, instance, prop.target).map_err(|_| {
                            lua_error("Tween:Play: the tweened instance has been destroyed")
                        })?;
                    }
                    data.elapsed = 0.0;
                }
            }
            data.state = if data.elapsed < data.info.delay {
                PlaybackState::Delayed
            } else {
                PlaybackState::Playing
            };
        }
        let mut playing = state.tweens.playing.borrow_mut();
        if playing.len() >= MAX_PLAYING_TWEENS {
            self.data.borrow_mut().state = PlaybackState::Paused;
            return Err(lua_error("Tween:Play: too many tweens are playing"));
        }
        // A newer tween overrides older ones on the same properties; those are cancelled.
        let mine = self.data.borrow();
        playing.retain(|other| {
            if Rc::ptr_eq(other, &self.data) {
                return true;
            }
            let mut o = other.borrow_mut();
            let clash = o.instance == mine.instance
                && o.props
                    .iter()
                    .any(|a| mine.props.iter().any(|b| a.target.overlaps(b.target)));
            if clash {
                o.state = PlaybackState::Cancelled;
                o.elapsed = 0.0;
                drop(o);
                state.queue_completed(other, PlaybackState::Cancelled);
            }
            !clash
        });
        drop(mine);
        playing.push(self.data.clone());
        Ok(())
    }

    fn pause(&self) {
        let mut data = self.data.borrow_mut();
        if data.running() {
            data.state = PlaybackState::Paused;
            drop(data);
            self.unlist();
        }
    }

    fn cancel(&self) {
        let mut data = self.data.borrow_mut();
        if matches!(
            data.state,
            PlaybackState::Playing | PlaybackState::Delayed | PlaybackState::Paused
        ) {
            data.state = PlaybackState::Cancelled;
            data.elapsed = 0.0;
            drop(data);
            self.unlist();
            self.state
                .queue_completed(&self.data, PlaybackState::Cancelled);
        }
    }

    fn destroy(&self) {
        {
            let mut data = self.data.borrow_mut();
            if data.running() || data.state == PlaybackState::Paused {
                data.state = PlaybackState::Cancelled;
                data.elapsed = 0.0;
            }
        }
        self.unlist();
        let id = self.data.borrow().id;
        if let Some(key) = completed_key(&self.state, id) {
            self.state.hub.remove_key(key);
        }
    }

    fn unlist(&self) {
        self.state
            .tweens
            .playing
            .borrow_mut()
            .retain(|t| !Rc::ptr_eq(t, &self.data));
    }

    fn index(&self, lua: &Lua, key: &str) -> mlua::Result<Value> {
        let state = &self.state;
        Ok(match key {
            "ClassName" | "Name" => Value::String(lua.create_string("Tween")?),
            "PlaybackState" => {
                EnumName("PlaybackState", state_name(self.data.borrow().state)).into_lua(lua)?
            }
            "TweenInfo" => LuaTweenInfo(self.data.borrow().info).into_lua(lua)?,
            "Instance" => {
                let id = self.data.borrow().instance;
                let alive = state.world()?.is_alive(id);
                state.wrap_opt(lua, alive.then_some(id))?
            }
            "Completed" => {
                let id = self.data.borrow().id;
                let key = completed_key(state, id)
                    .ok_or_else(|| lua_error("the world has no RunService"))?;
                Value::UserData(lua.create_userdata(LuaSignal {
                    state: state.clone(),
                    key,
                })?)
            }
            other => member(state.tweens.tween_methods.get(), other, "Tween")?,
        })
    }
}

/// Method `key` from `methods`, or the "not a valid member" error.
fn member(methods: Option<&Table>, key: &str, class: &str) -> mlua::Result<Value> {
    match methods.map(|m| m.raw_get::<Value>(key)).transpose()? {
        Some(Value::Nil) | None => {
            Err(lua_error(format!("{key} is not a valid member of {class}")))
        }
        Some(f) => Ok(f),
    }
}

impl UserData for LuaTween {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "Instance");
        fields.add_meta_field_with(MetaMethod::Index, |lua| {
            errors::guarded_fn(lua, |lua, (this, key): (UserDataRef<LuaTween>, String)| {
                this.index(lua, &key)
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok("Tween".to_string()));
    }
}

/// The target and goal of goal `key` for an instance of `class`, and whether it is a part's
/// clamped `Size`.
fn goal(class: &'static str, key: &str, value: &Value) -> mlua::Result<(Target, TweenValue, bool)> {
    let cframe_goal = |name: &str| {
        cframe(value).map(TweenValue::CFrame).ok_or_else(|| {
            lua_error(format!(
                "TweenService:Create: property {name} expects a CFrame, got {}",
                value.type_name()
            ))
        })
    };
    if key == "CFrame" && is_subclass(class, "BasePart") {
        return Ok((Target::PartCFrame, cframe_goal(key)?, false));
    }
    if key == "WorldPivot" && is_subclass(class, "Model") {
        return Ok((Target::WorldPivot, cframe_goal(key)?, false));
    }
    let def = find_property(class, key)
        .filter(|d| !d.hidden)
        .ok_or_else(|| {
            lua_error(format!(
                "TweenService:Create: no property named '{key}' for a {class}"
            ))
        })?;
    let tweenable = matches!(
        def.ty,
        ValueType::Number
            | ValueType::Vector3
            | ValueType::Color3
            | ValueType::UDim2
            | ValueType::CFrame
            | ValueType::Bool
    );
    if !tweenable {
        return Err(lua_error(format!(
            "TweenService:Create: property '{key}' is a {}, which cannot be tweened",
            def.ty.name()
        )));
    }
    let converted = lua_to_value(value, def.ty, key)?;
    let goal = TweenValue::from_core(converted).ok_or_else(|| lua_error("not tweenable"))?;
    let clamp_size = key == "Size" && is_subclass(class, "BasePart");
    Ok((Target::Property(def.name), goal, clamp_size))
}

/// The Luau `TweenService`.
pub(crate) struct LuaTweenService {
    state: Rc<ApiState>,
}

impl LuaTweenService {
    fn create(&self, instance: &Value, info: &Value, goals: &Value) -> mlua::Result<LuaTween> {
        let state = &self.state;
        let instance = match instance {
            Value::UserData(ud) => ud.borrow::<LuaInstance>().ok().map(|i| i.id),
            _ => None,
        }
        .ok_or_else(|| lua_error("TweenService:Create: argument 1 must be an Instance"))?;
        let info = match info {
            Value::UserData(ud) => ud.borrow::<LuaTweenInfo>().ok().map(|i| i.0),
            _ => None,
        }
        .ok_or_else(|| lua_error("TweenService:Create: argument 2 must be a TweenInfo"))?;
        let Value::Table(goals) = goals else {
            return Err(lua_error(
                "TweenService:Create: argument 3 must be a table of property goals",
            ));
        };
        let world = state.world()?;
        let class = world
            .class_name(instance)
            .ok_or_else(|| lua_error("TweenService:Create: the instance has been destroyed"))?;
        let mut props = Vec::new();
        for pair in goals.pairs::<Value, Value>() {
            let (key, value) = pair?;
            let Value::String(key) = key else {
                return Err(lua_error(
                    "TweenService:Create: goal keys must be property names",
                ));
            };
            let key = key.to_str()?;
            let (target, goal, clamp_size) = goal(class, &key, &value)?;
            let start = read(&world, instance, target).map_err(|_| {
                lua_error(format!(
                    "TweenService:Create: cannot read property '{}'",
                    &*key
                ))
            })?;
            props.push(Prop {
                target,
                goal,
                start,
                clamp_size,
            });
        }
        drop(world);
        let id = state.tweens.next_id.get();
        state.tweens.next_id.set(id + 1);
        Ok(LuaTween {
            state: state.clone(),
            data: Rc::new(RefCell::new(TweenData {
                id,
                instance,
                info,
                props,
                elapsed: 0.0,
                state: PlaybackState::Begin,
                lua_alive: true,
            })),
        })
    }
}

impl UserData for LuaTweenService {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "Instance");
        fields.add_meta_field_with(MetaMethod::Index, |lua| {
            errors::guarded_fn(
                lua,
                |lua, (this, key): (UserDataRef<LuaTweenService>, String)| match key.as_str() {
                    "ClassName" | "Name" => Ok(Value::String(lua.create_string("TweenService")?)),
                    other => member(
                        this.state.tweens.service_methods.get(),
                        other,
                        "TweenService",
                    ),
                },
            )
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| {
            Ok("TweenService".to_string())
        });
    }
}

fn easing_args(style: &Value, direction: &Value) -> mlua::Result<(EasingStyle, EasingDirection)> {
    let style = match enum_item(style) {
        Some(("EasingStyle", name, _)) => style_named(name),
        _ => None,
    }
    .ok_or_else(|| {
        lua_error("TweenService:GetValue: argument 2 must be an Enum.EasingStyle item")
    })?;
    let direction = match enum_item(direction) {
        Some(("EasingDirection", name, _)) => direction_named(name),
        _ => None,
    }
    .ok_or_else(|| {
        lua_error("TweenService:GetValue: argument 3 must be an Enum.EasingDirection item")
    })?;
    Ok((style, direction))
}

/// Creates the `TweenService` object of `state` (once, so `GetService` returns the same one)
/// and the method tables of tweens and the service.
pub(crate) fn install_service(lua: &Lua, state: &Rc<ApiState>) -> mlua::Result<()> {
    let tween = lua.create_table()?;
    tween.raw_set(
        "Play",
        errors::guarded_fn(lua, |_, this: UserDataRef<LuaTween>| this.play())?,
    )?;
    tween.raw_set(
        "Pause",
        errors::guarded_fn(lua, |_, this: UserDataRef<LuaTween>| {
            this.pause();
            Ok(())
        })?,
    )?;
    tween.raw_set(
        "Cancel",
        errors::guarded_fn(lua, |_, this: UserDataRef<LuaTween>| {
            this.cancel();
            Ok(())
        })?,
    )?;
    tween.raw_set(
        "Destroy",
        errors::guarded_fn(lua, |_, this: UserDataRef<LuaTween>| {
            this.destroy();
            Ok(())
        })?,
    )?;
    tween.raw_set(
        "IsA",
        errors::guarded_fn(lua, |_, (_, class): (UserDataRef<LuaTween>, String)| {
            Ok(matches!(class.as_str(), "Tween" | "TweenBase" | "Instance"))
        })?,
    )?;
    let service = lua.create_table()?;
    service.raw_set(
        "Create",
        errors::guarded_fn(
            lua,
            |_,
             (this, instance, info, goals): (
                UserDataRef<LuaTweenService>,
                Value,
                Value,
                Value,
            )| this.create(&instance, &info, &goals),
        )?,
    )?;
    service.raw_set(
        "GetValue",
        errors::guarded_fn(
            lua,
            |_, (_, alpha, style, direction): (UserDataRef<LuaTweenService>, f64, Value, Value)| {
                let (style, direction) = easing_args(&style, &direction)?;
                Ok(f64::from(ease(style, direction, alpha as f32)))
            },
        )?,
    )?;
    lua.set_app_data(EnumTable(state.enums.table.clone()));
    let _ = state.tweens.tween_methods.set(tween);
    let _ = state.tweens.service_methods.set(service);
    let object = lua.create_userdata(LuaTweenService {
        state: state.clone(),
    })?;
    let _ = state.tweens.service.set(object);
    Ok(())
}

/// The `TweenService` object, for `game:GetService("TweenService")`.
pub(crate) fn service(state: &ApiState, name: &str) -> Option<Value> {
    if name != "TweenService" {
        return None;
    }
    state.tweens.service.get().cloned().map(Value::UserData)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(time: f64, repeat_count: i32, reverses: bool, delay: f64) -> Info {
        Info {
            time,
            repeat_count,
            reverses,
            delay,
            ..Info::default()
        }
    }

    #[test]
    fn timeline_matches_metallic_anim() {
        let cases = [
            info(1.0, 0, false, 0.0),
            info(0.5, 2, true, 0.25),
            info(2.0, -1, false, 1.0),
            info(0.0, 0, false, 0.0),
        ];
        for i in cases {
            let anim = metallic_anim::TweenInfo {
                time: i.time as f32,
                easing_style: i.style,
                easing_direction: i.direction,
                repeat_count: i.repeat_count,
                reverses: i.reverses,
                delay_time: i.delay as f32,
            };
            for step in 0..200 {
                let t = step as f64 * 0.037;
                // `advance` only samples once the delay is over.
                if t > i.delay {
                    let (linear, finished) = i.progress(t);
                    let sample = anim.sample(t as f32);
                    assert_eq!(finished, sample.finished, "{i:?} at {t}");
                    assert!(
                        (i.eased(linear) as f32 - sample.alpha).abs() < 1e-4,
                        "{i:?} at {t}: {} vs {}",
                        i.eased(linear),
                        sample.alpha
                    );
                }
            }
        }
    }

    #[test]
    fn lerp_is_exact_at_both_ends() {
        let a = TweenValue::Number(0.1);
        let b = TweenValue::Number(0.3);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        let a = TweenValue::Bool(false);
        assert_eq!(a.lerp(TweenValue::Bool(true), 0.9), a);
    }

    #[test]
    fn part_cframe_overlaps_position_and_orientation() {
        assert!(Target::PartCFrame.overlaps(Target::Property("Position")));
        assert!(Target::Property("Orientation").overlaps(Target::PartCFrame));
        assert!(!Target::PartCFrame.overlaps(Target::Property("Size")));
        assert!(Target::Property("Size").overlaps(Target::Property("Size")));
    }
}
