//! `workspace:Raycast`, `RaycastParams` and `RaycastResult`.
//!
//! Roblox semantics: the length of the direction is the maximum distance in studs, the first
//! part hit comes back as a `RaycastResult` (nil for no hit), and the filter is a list of
//! instances whose descendants count too. The physics itself is behind
//! [`SceneQuery`](crate::SceneQuery); this module turns Luau values into a
//! [`SceneRay`](crate::SceneRay) and the answer into a result.
//!
//! Not supported, accepted and ignored: `IgnoreWater` (there is no water), `CollisionGroup`
//! (the physics has no collision groups yet) and `BruteForceAllSlow`.

use std::cell::RefCell;
use std::rc::Rc;

use metallic_core::{InstanceId, Value as CoreValue, Vec3};
use mlua::{
    Function, Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods, UserDataRef,
    Value,
};

use super::datatypes::{LuaVector3, vector};
use super::input::enum_item;
use super::instance::LuaInstance;
use super::{ApiState, errors, lua_error};
use crate::scene::SceneRay;

/// Longest ray, in studs; a longer direction is shortened to this.
pub(crate) const MAX_RAY_LENGTH: f64 = 15_000.0;
/// Most parts a filter may expand to (the listed instances and all their descendants).
pub(crate) const MAX_FILTER_PARTS: usize = 100_000;
/// Most instances one `FilterDescendantsInstances` assignment takes.
const MAX_FILTER_LIST: usize = 10_000;

/// The settings of a `RaycastParams`.
#[derive(Clone)]
struct Params {
    filter: Vec<InstanceId>,
    include: bool,
    ignore_water: bool,
    collision_group: String,
    respect_can_collide: bool,
    brute_force: bool,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            filter: Vec::new(),
            include: false,
            ignore_water: false,
            collision_group: "Default".to_string(),
            respect_can_collide: false,
            brute_force: false,
        }
    }
}

/// The Luau `RaycastParams`.
pub(crate) struct LuaRaycastParams {
    state: Rc<ApiState>,
    params: RefCell<Params>,
}

/// Instances out of one Luau instance or an array of them.
fn instance_ids(value: &Value, what: &str) -> mlua::Result<Vec<InstanceId>> {
    let one = |v: &Value| -> mlua::Result<InstanceId> {
        match v {
            Value::UserData(ud) => ud
                .borrow::<LuaInstance>()
                .map(|i| i.id)
                .map_err(|_| lua_error(format!("{what} must be Instances"))),
            other => Err(lua_error(format!(
                "{what} must be Instances (got a {})",
                other.type_name()
            ))),
        }
    };
    match value {
        Value::Table(t) => {
            let mut out = Vec::new();
            for item in t.sequence_values::<Value>() {
                if out.len() >= MAX_FILTER_LIST {
                    return Err(lua_error(format!(
                        "{what} lists more than {MAX_FILTER_LIST} instances"
                    )));
                }
                out.push(one(&item?)?);
            }
            Ok(out)
        }
        other => Ok(vec![one(other)?]),
    }
}

impl UserData for LuaRaycastParams {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "RaycastParams");
        fields.add_field_method_get("FilterDescendantsInstances", |lua, this| {
            let ids = this.params.borrow().filter.clone();
            let alive: Vec<InstanceId> = {
                let world = this.state.world()?;
                ids.into_iter().filter(|&id| world.is_alive(id)).collect()
            };
            let table = lua.create_table()?;
            for (i, id) in alive.into_iter().enumerate() {
                table.set(i + 1, this.state.wrap(lua, id)?)?;
            }
            Ok(table)
        });
        fields.add_field_method_set("FilterDescendantsInstances", |_, this, value: Value| {
            let ids = match &value {
                Value::Nil => Vec::new(),
                other => instance_ids(other, "FilterDescendantsInstances")?,
            };
            this.params.borrow_mut().filter = ids;
            Ok(())
        });
        fields.add_field_method_get("FilterType", |_, this| {
            let name = if this.params.borrow().include {
                "Include"
            } else {
                "Exclude"
            };
            Ok(this.state.enum_value("RaycastFilterType", name))
        });
        fields.add_field_method_set("FilterType", |_, this, value: Value| {
            match enum_item(&value) {
                Some(("RaycastFilterType", name, _)) => {
                    this.params.borrow_mut().include = name == "Include";
                    Ok(())
                }
                _ => Err(lua_error(
                    "FilterType must be an Enum.RaycastFilterType (Include or Exclude)",
                )),
            }
        });
        fields.add_field_method_get("IgnoreWater", |_, this| {
            Ok(this.params.borrow().ignore_water)
        });
        fields.add_field_method_set("IgnoreWater", |_, this, value: bool| {
            this.params.borrow_mut().ignore_water = value;
            Ok(())
        });
        fields.add_field_method_get("RespectCanCollide", |_, this| {
            Ok(this.params.borrow().respect_can_collide)
        });
        fields.add_field_method_set("RespectCanCollide", |_, this, value: bool| {
            this.params.borrow_mut().respect_can_collide = value;
            Ok(())
        });
        fields.add_field_method_get("BruteForceAllSlow", |_, this| {
            Ok(this.params.borrow().brute_force)
        });
        fields.add_field_method_set("BruteForceAllSlow", |_, this, value: bool| {
            this.params.borrow_mut().brute_force = value;
            Ok(())
        });
        fields.add_field_method_get("CollisionGroup", |_, this| {
            Ok(this.params.borrow().collision_group.clone())
        });
        fields.add_field_method_set("CollisionGroup", |_, this, value: String| {
            this.params.borrow_mut().collision_group = value;
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("AddToFilter", |_, this, value: Value| {
            let ids = instance_ids(&value, "AddToFilter")?;
            let mut params = this.params.borrow_mut();
            if params.filter.len() + ids.len() > MAX_FILTER_LIST {
                return Err(lua_error(format!(
                    "the filter lists more than {MAX_FILTER_LIST} instances"
                )));
            }
            params.filter.extend(ids);
            Ok(())
        });
        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| {
            Ok("RaycastParams".to_string())
        });
    }
}

/// The Luau `RaycastResult`.
pub(crate) struct LuaRaycastResult {
    state: Rc<ApiState>,
    instance: InstanceId,
    position: Vec3,
    normal: Vec3,
    distance: f64,
    material: String,
}

impl UserData for LuaRaycastResult {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "RaycastResult");
        fields.add_field_method_get("Instance", |lua, this| {
            this.state.wrap_opt(lua, Some(this.instance))
        });
        fields.add_field_method_get("Position", |_, this| Ok(LuaVector3(this.position)));
        fields.add_field_method_get("Normal", |_, this| Ok(LuaVector3(this.normal)));
        fields.add_field_method_get("Distance", |_, this| Ok(this.distance));
        fields.add_field_method_get("Material", |_, this| {
            Ok(this.state.enum_value("Material", &this.material))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| {
            Ok("RaycastResult".to_string())
        });
    }
}

/// `RaycastParams.new()`.
pub(crate) fn constructor(lua: &Lua, state: &Rc<ApiState>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let state = state.clone();
    table.set(
        "new",
        errors::guarded_fn(lua, move |_, ()| {
            Ok(LuaRaycastParams {
                state: state.clone(),
                params: RefCell::new(Params::default()),
            })
        })?,
    )?;
    table.set_readonly(true);
    Ok(table)
}

/// The listed instances and everything below them, without repeats, in a deterministic order.
fn expand(state: &ApiState, listed: &[InstanceId]) -> mlua::Result<Vec<InstanceId>> {
    let world = state.world()?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &root in listed {
        if !world.is_alive(root) {
            continue;
        }
        let mut stack = vec![root];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if out.len() >= MAX_FILTER_PARTS {
                return Err(lua_error(format!(
                    "the filter covers more than {MAX_FILTER_PARTS} instances"
                )));
            }
            out.push(cur);
            stack.extend(world.children(cur).iter().rev().copied());
        }
    }
    Ok(out)
}

/// `workspace:Raycast(origin, direction, params?)`.
pub(crate) fn methods(lua: &Lua) -> mlua::Result<Vec<(&'static str, Function)>> {
    let raycast = lua.create_function(
        |lua,
         (this, origin, direction, params): (
            UserDataRef<LuaInstance>,
            Value,
            Value,
            Option<Value>,
        )| {
            this.expect_class("Workspace", "Raycast")?;
            let state = this.state.clone();
            let arg = |v: &Value, n: usize| {
                vector(v).ok_or_else(|| {
                    lua_error(format!(
                        "invalid argument #{n} to 'Raycast' (Vector3 expected, got {})",
                        v.type_name()
                    ))
                })
            };
            let origin = arg(&origin, 2)?;
            let mut direction = arg(&direction, 3)?;
            let finite = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
            if !finite(origin) || !finite(direction) {
                return Err(lua_error("Raycast: origin and direction must be finite"));
            }
            let p = match &params {
                None | Some(Value::Nil) => Params::default(),
                Some(Value::UserData(ud)) => ud
                    .borrow::<LuaRaycastParams>()
                    .map(|p| p.params.borrow().clone())
                    .map_err(|_| {
                        lua_error("invalid argument #4 to 'Raycast' (RaycastParams expected)")
                    })?,
                Some(other) => {
                    return Err(lua_error(format!(
                        "invalid argument #4 to 'Raycast' (RaycastParams expected, got {})",
                        other.type_name()
                    )));
                }
            };
            let length = direction.magnitude();
            if length <= 0.0 {
                return Ok(Value::Nil);
            }
            if length > MAX_RAY_LENGTH {
                direction = direction.unit() * MAX_RAY_LENGTH;
            }
            let Some(scene) = state.shared.scene.borrow().clone() else {
                return Ok(Value::Nil);
            };
            let parts = expand(&state, &p.filter)?;
            // Include with nothing listed can hit nothing.
            if p.include && parts.is_empty() {
                return Ok(Value::Nil);
            }
            let hit = scene.raycast(&SceneRay {
                origin,
                direction,
                include: p.include,
                parts,
                respect_can_collide: p.respect_can_collide,
            });
            let Some(hit) = hit else {
                return Ok(Value::Nil);
            };
            let material = match state.world()?.get_ref(hit.instance, "Material") {
                Ok(CoreValue::String(s)) => s.clone(),
                _ => String::new(),
            };
            Ok(Value::UserData(lua.create_userdata(LuaRaycastResult {
                state,
                instance: hit.instance,
                position: hit.position,
                normal: hit.normal,
                distance: hit.distance,
                material,
            })?))
        },
    )?;
    Ok(vec![("Raycast", errors::guarded(lua, raycast)?)])
}
