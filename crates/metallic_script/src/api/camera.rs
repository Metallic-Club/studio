//! `workspace.CurrentCamera`, the `Camera` projection methods and the `Ray` datatype.
//!
//! The camera is an instance (class `Camera`, under the Workspace) with `CFrame`,
//! `FieldOfView` and `CameraType`; the game publishes its own camera there while `CameraType`
//! is `Custom` and uses the script's values while it is `Scriptable` or `Fixed`. The methods
//! here are pure maths over those properties and the viewport the host reported
//! ([`ScriptHost::set_viewport_size`](crate::ScriptHost::set_viewport_size)).
//!
//! Conventions (as in Roblox): the field of view is vertical, in degrees; viewport pixels start
//! at the top left; the camera looks along its CFrame's `LookVector` (-Z). `Screen` variants
//! take coordinates measured below the GUI inset, `Viewport` variants do not.

use metallic_core::{CFrame, InstanceId, Value as CoreValue, Vec3, World};
use mlua::{
    Function, Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods, UserDataRef, Value,
};

use super::datatypes::{LuaVector3, vector};
use super::instance::LuaInstance;
use super::{ApiState, errors, lua_error};

/// The viewport size used until the host reports one.
pub(crate) const DEFAULT_VIEWPORT: [f64; 2] = [1280.0, 720.0];

/// The Luau `Ray`: an origin and a direction (any length).
#[derive(Clone, Copy)]
pub(crate) struct LuaRay {
    pub(crate) origin: Vec3,
    pub(crate) direction: Vec3,
}

fn vec3_arg(v: &Value, what: &str) -> mlua::Result<Vec3> {
    vector(v).ok_or_else(|| {
        lua_error(format!(
            "invalid argument ({what}): Vector3 expected, got {}",
            v.type_name()
        ))
    })
}

impl UserData for LuaRay {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "Ray");
        fields.add_field_method_get("Origin", |_, this| Ok(LuaVector3(this.origin)));
        fields.add_field_method_get("Direction", |_, this| Ok(LuaVector3(this.direction)));
        fields.add_field_method_get("Unit", |_, this| {
            Ok(LuaRay {
                origin: this.origin,
                direction: this.direction.unit(),
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("ClosestPoint", |_, this, point: Value| {
            let point = vec3_arg(&point, "point")?;
            Ok(LuaVector3(this.closest_point(point)))
        });
        methods.add_method("Distance", |_, this, point: Value| {
            let point = vec3_arg(&point, "point")?;
            Ok((this.closest_point(point) - point).magnitude())
        });
        methods.add_meta_function(MetaMethod::Eq, |_, (a, b): (Value, Value)| {
            let ray = |v: &Value| match v {
                Value::UserData(ud) => ud.borrow::<LuaRay>().ok().map(|r| (r.origin, r.direction)),
                _ => None,
            };
            Ok(match (ray(&a), ray(&b)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            })
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "{{{}, {}, {}}}, {{{}, {}, {}}}",
                this.origin.x,
                this.origin.y,
                this.origin.z,
                this.direction.x,
                this.direction.y,
                this.direction.z
            ))
        });
    }
}

impl LuaRay {
    /// The point of the ray's line closest to `point` (the line is bounded at the origin, like
    /// Roblox's: points behind the origin map to the origin).
    fn closest_point(&self, point: Vec3) -> Vec3 {
        let len2 = self.direction.x * self.direction.x
            + self.direction.y * self.direction.y
            + self.direction.z * self.direction.z;
        if len2 <= 0.0 {
            return self.origin;
        }
        let to = point - self.origin;
        let t =
            (to.x * self.direction.x + to.y * self.direction.y + to.z * self.direction.z) / len2;
        self.origin + self.direction * t.clamp(0.0, 1.0)
    }
}

/// `Ray.new(origin, direction)`.
pub(crate) fn ray_constructor(lua: &Lua) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set(
        "new",
        errors::guarded_fn(lua, |_, (origin, direction): (Value, Value)| {
            Ok(LuaRay {
                origin: vec3_arg(&origin, "origin")?,
                direction: vec3_arg(&direction, "direction")?,
            })
        })?,
    )?;
    table.set_readonly(true);
    Ok(table)
}

/// The camera `workspace.CurrentCamera` returns: a `Camera` under the Workspace, the one named
/// `Camera` first.
pub(crate) fn current_camera(world: &World) -> Option<InstanceId> {
    let workspace = world.root("Workspace")?;
    let cameras = || {
        world
            .children(workspace)
            .iter()
            .copied()
            .filter(|&c| world.class_name(c) == Some("Camera"))
    };
    cameras()
        .find(|&c| matches!(world.get_ref(c, "Name"), Ok(CoreValue::String(n)) if n == "Camera"))
        .or_else(|| cameras().next())
}

/// The camera's pose, field of view and the viewport, as the projection maths needs them.
struct View {
    pose: CFrame,
    tan_half_fov: f64,
    size: [f64; 2],
    inset: f64,
}

impl View {
    fn read(state: &ApiState, camera: &LuaInstance) -> mlua::Result<View> {
        camera.expect_class("Camera", "ViewportPointToRay")?;
        let world = state.world()?;
        let pose = match world.get_ref(camera.id, "CFrame") {
            Ok(CoreValue::CFrame(cf)) => **cf,
            _ => CFrame::IDENTITY,
        };
        let fov = match world.get_ref(camera.id, "FieldOfView") {
            Ok(CoreValue::Number(n)) => *n,
            _ => 70.0,
        };
        let size = state.shared.viewport_size();
        Ok(View {
            pose,
            tan_half_fov: (fov.to_radians() / 2.0).tan(),
            size,
            inset: state.shared.gui_inset.get(),
        })
    }

    fn aspect(&self) -> f64 {
        self.size[0] / self.size[1]
    }

    /// The ray through viewport pixel `(x, y)`: starts `depth` studs along it from the camera.
    fn ray(&self, x: f64, y: f64, depth: f64) -> LuaRay {
        let nx = x / self.size[0] * 2.0 - 1.0;
        let ny = 1.0 - y / self.size[1] * 2.0;
        let local = Vec3::new(
            nx * self.tan_half_fov * self.aspect(),
            ny * self.tan_half_fov,
            -1.0,
        );
        let direction = self.pose.vector_to_world(local).unit();
        LuaRay {
            origin: self.pose.position + direction * depth,
            direction,
        }
    }

    /// Where `point` is on the viewport: `(x, y, depth)` and whether it is in front of the
    /// camera and inside the viewport. Behind the camera, `depth` is negative.
    fn project(&self, point: Vec3) -> (Vec3, bool) {
        let c = self.pose.point_to_object(point);
        let depth = -c.z;
        let d = depth.abs().max(1e-9);
        let nx = c.x / (d * self.tan_half_fov * self.aspect());
        let ny = c.y / (d * self.tan_half_fov);
        let x = (nx * 0.5 + 0.5) * self.size[0];
        let y = (0.5 - ny * 0.5) * self.size[1];
        let on_screen =
            depth > 0.0 && (0.0..=self.size[0]).contains(&x) && (0.0..=self.size[1]).contains(&y);
        (Vec3::new(x, y, depth), on_screen)
    }
}

fn number(v: &Value, what: &str) -> mlua::Result<f64> {
    let n = match v {
        Value::Number(n) => *n,
        Value::Integer(i) => *i as f64,
        other => {
            return Err(lua_error(format!(
                "invalid argument ({what}): number expected, got {}",
                other.type_name()
            )));
        }
    };
    if n.is_finite() {
        Ok(n)
    } else {
        Err(lua_error(format!(
            "invalid argument ({what}): must be finite"
        )))
    }
}

/// `ViewportPointToRay`, `ScreenPointToRay`, `WorldToViewportPoint` and `WorldToScreenPoint`.
pub(crate) fn methods(lua: &Lua) -> mlua::Result<Vec<(&'static str, Function)>> {
    let to_ray = |screen: bool| {
        lua.create_function(
            move |_, (this, x, y, depth): (UserDataRef<LuaInstance>, Value, Value, Option<Value>)| {
                let view = View::read(&this.state, &this)?;
                let x = number(&x, "x")?;
                let y = number(&y, "y")?;
                let depth = match depth {
                    None | Some(Value::Nil) => 0.0,
                    Some(d) => number(&d, "depth")?,
                };
                let y = if screen { y + view.inset } else { y };
                Ok(view.ray(x, y, depth))
            },
        )
    };
    let to_point = |screen: bool| {
        lua.create_function(move |_, (this, point): (UserDataRef<LuaInstance>, Value)| {
            let view = View::read(&this.state, &this)?;
            let point = vec3_arg(&point, "worldPoint")?;
            let (mut at, on) = view.project(point);
            if screen {
                at.y -= view.inset;
            }
            Ok((LuaVector3(at), on))
        })
    };
    Ok(vec![
        ("ViewportPointToRay", errors::guarded(lua, to_ray(false)?)?),
        ("ScreenPointToRay", errors::guarded(lua, to_ray(true)?)?),
        (
            "WorldToViewportPoint",
            errors::guarded(lua, to_point(false)?)?,
        ),
        ("WorldToScreenPoint", errors::guarded(lua, to_point(true)?)?),
    ])
}
