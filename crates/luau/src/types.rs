//! The data types: `Vector3` (Luau's native `vector`), `Vector2`, `CFrame`, `Color3` and `EnumItem`, with Roblox's
//! constructors, members and operators. Each is a set of C functions; `library.rs` puts them into metatables and
//! library tables.
//!
//! # Why `Vector3` is Luau's `vector`
//!
//! A native vector is a value, not an object: creating one allocates nothing, `+ - * /` and `.X/.Y/.Z` run inside
//! the VM, and `Vector3.new(x, y, z)` compiles to the vector builtin (the compiler is told `Vector3.new` is the
//! vector constructor). The rest of the API hangs off the vector metatable (`.Magnitude`, `:Dot()`...), and
//! `typeof` reports `"Vector3"`. Differences from Roblox: `type(v)` is `"vector"`, components are 32-bit floats
//! (as in the engine), and lower-case `.x` also works.
//!
//! Angles follow Roblox: `CFrame.Angles` and `CFrame.fromOrientation` take radians, `ToOrientation` returns radians.
//! (Properties such as `Orientation` stay in degrees, as in Roblox.)

use std::ffi::c_int;

use metallic_core::glam::{EulerRot, Mat3, Quat};
use metallic_core::{CFrame, Color3, Vector2, Vector3};

use crate::ffi::{Args, Call, EnumRef, Kind, Out, Push, Raise, Ret, lua_fn};

// ---------------------------------------------------------------------------------------------------------------
// Argument helpers. `first` is the stack index of the user's argument #1 (2 for methods, where 1 is `self`).
// ---------------------------------------------------------------------------------------------------------------

fn bad(args: &Args<'_>, idx: c_int, first: c_int, f: &str, expected: &str) -> Raise {
    Raise(format!(
        "bad argument #{} to '{f}' ({expected} expected, got {})",
        idx - first + 1,
        args.type_name(idx)
    ))
}

fn number(args: &Args<'_>, idx: c_int, first: c_int, f: &str) -> Result<f64, Raise> {
    args.number(idx).ok_or_else(|| bad(args, idx, first, f, "number"))
}

fn number_or(args: &Args<'_>, idx: c_int, first: c_int, f: &str, default: f64) -> Result<f64, Raise> {
    match args.kind(idx) {
        Kind::None | Kind::Nil => Ok(default),
        _ => number(args, idx, first, f),
    }
}

fn float(args: &Args<'_>, idx: c_int, first: c_int, f: &str) -> Result<f32, Raise> {
    number(args, idx, first, f).map(|n| n as f32)
}

fn vector3(args: &Args<'_>, idx: c_int, first: c_int, f: &str) -> Result<Vector3, Raise> {
    args.vector(idx).ok_or_else(|| bad(args, idx, first, f, "Vector3"))
}

fn vector2(args: &Args<'_>, idx: c_int, first: c_int, f: &str) -> Result<Vector2, Raise> {
    args.payload::<Vector2>(idx)
        .ok_or_else(|| bad(args, idx, first, f, "Vector2"))
}

fn cframe(args: &Args<'_>, idx: c_int, first: c_int, f: &str) -> Result<CFrame, Raise> {
    args.payload::<CFrame>(idx)
        .ok_or_else(|| bad(args, idx, first, f, "CFrame"))
}

fn color3(args: &Args<'_>, idx: c_int, first: c_int, f: &str) -> Result<Color3, Raise> {
    args.payload::<Color3>(idx)
        .ok_or_else(|| bad(args, idx, first, f, "Color3"))
}

fn key<'a>(args: &Args<'a>) -> Result<&'a str, Raise> {
    args.str(2)
        .ok_or_else(|| Raise(format!("invalid member key of type {}", args.type_name(2))))
}

fn not_member(key: &str, type_name: &str) -> Raise {
    Raise(format!("{key} is not a valid member of {type_name}"))
}

fn one(push: Push) -> Ret {
    Ok(Out::One(push))
}

fn num(n: f32) -> Push {
    Push::Number(f64::from(n))
}

/// The method name of a `__namecall`, or of a method closure (upvalue 1).
fn method_name<'a>(args: &Args<'a>) -> Result<&'a str, Raise> {
    args.namecall()
        .or_else(|| args.upvalue_str_atom(1))
        .and_then(|name| name.str())
        .ok_or_else(|| Raise::from("no method name"))
}

/// Roblox's `FuzzyEq` default tolerance.
const FUZZY_EPSILON: f64 = 1e-5;

// ---------------------------------------------------------------------------------------------------------------
// Vector3 (native vector).
// ---------------------------------------------------------------------------------------------------------------

/// `Vector3` methods, for `__index` (as function values) and the method table.
pub(crate) const VECTOR3_METHODS: &[&str] = &[
    "Dot", "Cross", "Lerp", "Min", "Max", "Abs", "Floor", "Ceil", "FuzzyEq", "Angle",
];

lua_fn!(pub(crate) vector3_new => vector3_new_body);
lua_fn!(pub(crate) vector3_index => vector3_index_body);
lua_fn!(pub(crate) vector3_method => vector3_method_body);

fn vector3_new_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let x = number_or(a, 1, 1, "new", 0.0)?;
    let y = number_or(a, 2, 1, "new", 0.0)?;
    let z = number_or(a, 3, 1, "new", 0.0)?;
    one(Push::Vector3(Vector3::new(x as f32, y as f32, z as f32)))
}

fn vector3_index_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let v = vector3(a, 1, 1, "index")?;
    let key = key(a)?;
    match key {
        "X" | "x" => one(num(v.x)),
        "Y" | "y" => one(num(v.y)),
        "Z" | "z" => one(num(v.z)),
        "Magnitude" => one(num(v.length())),
        // Like Roblox, the zero vector's unit is NaN.
        "Unit" => one(Push::Vector3(v / v.length())),
        _ => match VECTOR3_METHODS.iter().find(|m| **m == key) {
            Some(name) => one(Push::Field {
                table: call.shared()?.refs.vector3_methods,
                name,
            }),
            None => Err(not_member(key, "Vector3")),
        },
    }
}

fn vector3_method_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let name = method_name(a)?;
    let v = a
        .vector(1)
        .ok_or_else(|| Raise(format!("Expected ':' not '.' calling member function {name}")))?;
    match name {
        "Dot" => one(num(v.dot(vector3(a, 2, 2, name)?))),
        "Cross" => one(Push::Vector3(v.cross(vector3(a, 2, 2, name)?))),
        "Lerp" => {
            let goal = vector3(a, 2, 2, name)?;
            let alpha = float(a, 3, 2, name)?;
            one(Push::Vector3(v + (goal - v) * alpha))
        }
        "Min" | "Max" => {
            let mut out = v;
            for i in 2..=a.len() {
                let other = vector3(a, i, 2, name)?;
                out = if name == "Min" { out.min(other) } else { out.max(other) };
            }
            one(Push::Vector3(out))
        }
        "Abs" => one(Push::Vector3(v.abs())),
        "Floor" => one(Push::Vector3(v.floor())),
        "Ceil" => one(Push::Vector3(v.ceil())),
        "FuzzyEq" => {
            let other = vector3(a, 2, 2, name)?;
            let epsilon = number_or(a, 3, 2, name, FUZZY_EPSILON)? as f32;
            one(Push::Bool(v.abs_diff_eq(other, epsilon)))
        }
        "Angle" => {
            let other = vector3(a, 2, 2, name)?;
            let angle = v.angle_between(other);
            let signed = match a.vector(3) {
                Some(axis) if v.cross(other).dot(axis) < 0.0 => -angle,
                _ => angle,
            };
            one(num(signed))
        }
        _ => Err(not_member(name, "Vector3")),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Vector2 (userdata).
// ---------------------------------------------------------------------------------------------------------------

/// `Vector2` methods.
pub(crate) const VECTOR2_METHODS: &[&str] = &["Dot", "Cross", "Lerp", "Min", "Max", "Abs", "Floor", "Ceil", "FuzzyEq"];

lua_fn!(pub(crate) vector2_new => vector2_new_body);
lua_fn!(pub(crate) vector2_index => vector2_index_body);
lua_fn!(pub(crate) vector2_method => vector2_method_body);
lua_fn!(pub(crate) vector2_add => vector2_add_body);
lua_fn!(pub(crate) vector2_sub => vector2_sub_body);
lua_fn!(pub(crate) vector2_mul => vector2_mul_body);
lua_fn!(pub(crate) vector2_div => vector2_div_body);
lua_fn!(pub(crate) vector2_unm => vector2_unm_body);
lua_fn!(pub(crate) vector2_eq => vector2_eq_body);
lua_fn!(pub(crate) vector2_to_string => vector2_to_string_body);

fn vector2_new_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let x = number_or(a, 1, 1, "new", 0.0)?;
    let y = number_or(a, 2, 1, "new", 0.0)?;
    one(Push::Vector2(Vector2::new(x as f32, y as f32)))
}

fn vector2_index_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let v = vector2(a, 1, 1, "index")?;
    let key = key(a)?;
    match key {
        "X" => one(num(v.x)),
        "Y" => one(num(v.y)),
        "Magnitude" => one(num(v.length())),
        "Unit" => one(Push::Vector2(v / v.length())),
        _ => match VECTOR2_METHODS.iter().find(|m| **m == key) {
            Some(name) => one(Push::Field {
                table: call.shared()?.refs.vector2_methods,
                name,
            }),
            None => Err(not_member(key, "Vector2")),
        },
    }
}

fn vector2_method_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let name = method_name(a)?;
    let v = a
        .payload::<Vector2>(1)
        .ok_or_else(|| Raise(format!("Expected ':' not '.' calling member function {name}")))?;
    match name {
        "Dot" => one(num(v.dot(vector2(a, 2, 2, name)?))),
        "Cross" => one(num(v.perp_dot(vector2(a, 2, 2, name)?))),
        "Lerp" => {
            let goal = vector2(a, 2, 2, name)?;
            let alpha = float(a, 3, 2, name)?;
            one(Push::Vector2(v + (goal - v) * alpha))
        }
        "Min" | "Max" => {
            let mut out = v;
            for i in 2..=a.len() {
                let other = vector2(a, i, 2, name)?;
                out = if name == "Min" { out.min(other) } else { out.max(other) };
            }
            one(Push::Vector2(out))
        }
        "Abs" => one(Push::Vector2(v.abs())),
        "Floor" => one(Push::Vector2(v.floor())),
        "Ceil" => one(Push::Vector2(v.ceil())),
        "FuzzyEq" => {
            let other = vector2(a, 2, 2, name)?;
            let epsilon = number_or(a, 3, 2, name, FUZZY_EPSILON)? as f32;
            one(Push::Bool(v.abs_diff_eq(other, epsilon)))
        }
        _ => Err(not_member(name, "Vector2")),
    }
}

/// An arithmetic operand: a `Vector2` or a number.
fn v2_operand(a: &Args<'_>, idx: c_int, op: &str) -> Result<Vector2, Raise> {
    if let Some(v) = a.payload::<Vector2>(idx) {
        return Ok(v);
    }
    match a.number(idx) {
        Some(n) => Ok(Vector2::splat(n as f32)),
        None => Err(Raise(format!(
            "attempt to perform arithmetic ({op}) on {} and {}",
            a.type_name(1),
            a.type_name(2)
        ))),
    }
}

fn vector2_add_body(call: &Call<'_>) -> Ret {
    let (a, b) = (vector2(&call.args, 1, 1, "add")?, vector2(&call.args, 2, 1, "add")?);
    one(Push::Vector2(a + b))
}

fn vector2_sub_body(call: &Call<'_>) -> Ret {
    let (a, b) = (vector2(&call.args, 1, 1, "sub")?, vector2(&call.args, 2, 1, "sub")?);
    one(Push::Vector2(a - b))
}

fn vector2_mul_body(call: &Call<'_>) -> Ret {
    let a = v2_operand(&call.args, 1, "mul")?;
    let b = v2_operand(&call.args, 2, "mul")?;
    one(Push::Vector2(a * b))
}

fn vector2_div_body(call: &Call<'_>) -> Ret {
    let a = v2_operand(&call.args, 1, "div")?;
    let b = v2_operand(&call.args, 2, "div")?;
    one(Push::Vector2(a / b))
}

fn vector2_unm_body(call: &Call<'_>) -> Ret {
    one(Push::Vector2(-vector2(&call.args, 1, 1, "unm")?))
}

fn vector2_eq_body(call: &Call<'_>) -> Ret {
    let a = call.args.payload::<Vector2>(1);
    let b = call.args.payload::<Vector2>(2);
    one(Push::Bool(a.is_some() && a == b))
}

fn vector2_to_string_body(call: &Call<'_>) -> Ret {
    let v = vector2(&call.args, 1, 1, "tostring")?;
    one(Push::Text(format!("{}, {}", v.x, v.y)))
}

// ---------------------------------------------------------------------------------------------------------------
// CFrame (userdata).
// ---------------------------------------------------------------------------------------------------------------

/// `CFrame` methods.
pub(crate) const CFRAME_METHODS: &[&str] = &[
    "Inverse",
    "Lerp",
    "ToOrientation",
    "ToEulerAnglesXYZ",
    "ToEulerAnglesYXZ",
    "GetComponents",
    "components",
    "ToWorldSpace",
    "ToObjectSpace",
    "PointToWorldSpace",
    "PointToObjectSpace",
    "VectorToWorldSpace",
    "VectorToObjectSpace",
    "FuzzyEq",
];

lua_fn!(pub(crate) cframe_new => cframe_new_body);
lua_fn!(pub(crate) cframe_angles => cframe_angles_body);
lua_fn!(pub(crate) cframe_from_orientation => cframe_from_orientation_body);
lua_fn!(pub(crate) cframe_look_at => cframe_look_at_body);
lua_fn!(pub(crate) cframe_from_axis_angle => cframe_from_axis_angle_body);
lua_fn!(pub(crate) cframe_index => cframe_index_body);
lua_fn!(pub(crate) cframe_method => cframe_method_body);
lua_fn!(pub(crate) cframe_mul => cframe_mul_body);
lua_fn!(pub(crate) cframe_add => cframe_add_body);
lua_fn!(pub(crate) cframe_sub => cframe_sub_body);
lua_fn!(pub(crate) cframe_eq => cframe_eq_body);
lua_fn!(pub(crate) cframe_to_string => cframe_to_string_body);

/// The rotation matrix's rows, as Roblox lists them (`R00 R01 R02`, ...).
fn rows(cf: CFrame) -> [f32; 9] {
    let m = Mat3::from_quat(cf.rotation());
    [
        m.x_axis.x, m.y_axis.x, m.z_axis.x, m.x_axis.y, m.y_axis.y, m.z_axis.y, m.x_axis.z, m.y_axis.z, m.z_axis.z,
    ]
}

/// The twelve numbers of `GetComponents` and `tostring`.
fn components(cf: CFrame) -> [f32; 12] {
    let p = cf.position();
    let r = rows(cf);
    [p.x, p.y, p.z, r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7], r[8]]
}

fn cframe_new_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let cf = match a.len() {
        0 => CFrame::IDENTITY,
        1 => CFrame::new(vector3(a, 1, 1, "new")?),
        2 => {
            let (eye, target) = (vector3(a, 1, 1, "new")?, vector3(a, 2, 1, "new")?);
            CFrame::look_at(eye, target, Vector3::Y)
        }
        3 => CFrame::new(Vector3::new(
            float(a, 1, 1, "new")?,
            float(a, 2, 1, "new")?,
            float(a, 3, 1, "new")?,
        )),
        7 => {
            let p = Vector3::new(float(a, 1, 1, "new")?, float(a, 2, 1, "new")?, float(a, 3, 1, "new")?);
            let q = Quat::from_xyzw(
                float(a, 4, 1, "new")?,
                float(a, 5, 1, "new")?,
                float(a, 6, 1, "new")?,
                float(a, 7, 1, "new")?,
            );
            CFrame::from_parts(p, q)
        }
        12 => {
            let mut n = [0.0f32; 12];
            for (i, slot) in n.iter_mut().enumerate() {
                *slot = float(a, i as c_int + 1, 1, "new")?;
            }
            let m = Mat3::from_cols(
                Vector3::new(n[3], n[6], n[9]),
                Vector3::new(n[4], n[7], n[10]),
                Vector3::new(n[5], n[8], n[11]),
            );
            CFrame::from_parts(Vector3::new(n[0], n[1], n[2]), Quat::from_mat3(&m))
        }
        n => return Err(Raise(format!("invalid number of arguments to 'CFrame.new': {n}"))),
    };
    one(Push::CFrame(cf))
}

/// `CFrame.Angles(rx, ry, rz)` (also `fromEulerAnglesXYZ`): radians, applied Z, then Y, then X.
fn cframe_angles_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let (x, y, z) = (
        number_or(a, 1, 1, "Angles", 0.0)? as f32,
        number_or(a, 2, 1, "Angles", 0.0)? as f32,
        number_or(a, 3, 1, "Angles", 0.0)? as f32,
    );
    one(Push::CFrame(CFrame::from_rotation(Quat::from_euler(
        EulerRot::XYZ,
        x,
        y,
        z,
    ))))
}

/// `CFrame.fromOrientation(rx, ry, rz)` (also `fromEulerAnglesYXZ`): radians, applied Z, then X, then Y.
fn cframe_from_orientation_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let (x, y, z) = (
        number_or(a, 1, 1, "fromOrientation", 0.0)? as f32,
        number_or(a, 2, 1, "fromOrientation", 0.0)? as f32,
        number_or(a, 3, 1, "fromOrientation", 0.0)? as f32,
    );
    one(Push::CFrame(CFrame::from_rotation(Quat::from_euler(
        EulerRot::YXZ,
        y,
        x,
        z,
    ))))
}

fn cframe_look_at_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let eye = vector3(a, 1, 1, "lookAt")?;
    let target = vector3(a, 2, 1, "lookAt")?;
    let up = match a.kind(3) {
        Kind::None | Kind::Nil => Vector3::Y,
        _ => vector3(a, 3, 1, "lookAt")?,
    };
    one(Push::CFrame(CFrame::look_at(eye, target, up)))
}

fn cframe_from_axis_angle_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let axis = vector3(a, 1, 1, "fromAxisAngle")?;
    let angle = float(a, 2, 1, "fromAxisAngle")?;
    let rotation = axis
        .try_normalize()
        .map_or(Quat::IDENTITY, |axis| Quat::from_axis_angle(axis, angle));
    one(Push::CFrame(CFrame::from_rotation(rotation)))
}

fn cframe_index_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let cf = cframe(a, 1, 1, "index")?;
    let key = key(a)?;
    let p = cf.position();
    match key {
        "Position" | "p" => one(Push::Vector3(p)),
        "X" => one(num(p.x)),
        "Y" => one(num(p.y)),
        "Z" => one(num(p.z)),
        "LookVector" => one(Push::Vector3(cf.look_vector())),
        "RightVector" | "XVector" => one(Push::Vector3(cf.right_vector())),
        "UpVector" | "YVector" => one(Push::Vector3(cf.up_vector())),
        "ZVector" => one(Push::Vector3(-cf.look_vector())),
        "Rotation" => one(Push::CFrame(CFrame::from_rotation(cf.rotation()))),
        _ => match CFRAME_METHODS.iter().find(|m| **m == key) {
            Some(name) => one(Push::Field {
                table: call.shared()?.refs.cframe_methods,
                name,
            }),
            None => Err(not_member(key, "CFrame")),
        },
    }
}

/// Euler angles as three numbers (radians).
fn angles(v: Vector3) -> Ret {
    Ok(Out::Many(vec![
        Push::Number(f64::from(v.x.to_radians())),
        Push::Number(f64::from(v.y.to_radians())),
        Push::Number(f64::from(v.z.to_radians())),
    ]))
}

fn cframe_method_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let name = method_name(a)?;
    let cf = a
        .payload::<CFrame>(1)
        .ok_or_else(|| Raise(format!("Expected ':' not '.' calling member function {name}")))?;
    match name {
        "Inverse" => one(Push::CFrame(cf.inverse())),
        "Lerp" => {
            let goal = cframe(a, 2, 2, name)?;
            let alpha = float(a, 3, 2, name)?;
            one(Push::CFrame(cf.lerp(goal, alpha)))
        }
        "ToOrientation" | "ToEulerAnglesYXZ" => angles(cf.to_orientation_deg()),
        "ToEulerAnglesXYZ" => angles(cf.to_euler_xyz_deg()),
        "GetComponents" | "components" => Ok(Out::Many(components(cf).into_iter().map(num).collect())),
        "ToWorldSpace" => one(Push::CFrame(cf * cframe(a, 2, 2, name)?)),
        "ToObjectSpace" => one(Push::CFrame(cf.inverse() * cframe(a, 2, 2, name)?)),
        "PointToWorldSpace" => one(Push::Vector3(cf.transform_point(vector3(a, 2, 2, name)?))),
        "PointToObjectSpace" => one(Push::Vector3(cf.inverse().transform_point(vector3(a, 2, 2, name)?))),
        "VectorToWorldSpace" => one(Push::Vector3(cf.transform_vector(vector3(a, 2, 2, name)?))),
        "VectorToObjectSpace" => one(Push::Vector3(cf.inverse().transform_vector(vector3(a, 2, 2, name)?))),
        "FuzzyEq" => {
            let other = cframe(a, 2, 2, name)?;
            let epsilon = number_or(a, 3, 2, name, FUZZY_EPSILON)? as f32;
            one(Push::Bool(cf.approx_eq(other, epsilon)))
        }
        _ => Err(not_member(name, "CFrame")),
    }
}

/// `cf * cf` (compose) and `cf * v` (transform a point).
fn cframe_mul_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let cf = cframe(a, 1, 1, "mul")?;
    if let Some(other) = a.payload::<CFrame>(2) {
        return one(Push::CFrame(cf * other));
    }
    if let Some(v) = a.vector(2) {
        return one(Push::Vector3(cf * v));
    }
    Err(Raise(format!(
        "attempt to perform arithmetic (mul) on CFrame and {}",
        a.type_name(2)
    )))
}

fn cframe_add_body(call: &Call<'_>) -> Ret {
    let cf = cframe(&call.args, 1, 1, "add")?;
    let v = vector3(&call.args, 2, 1, "add")?;
    one(Push::CFrame(CFrame::from_parts(cf.position() + v, cf.rotation())))
}

fn cframe_sub_body(call: &Call<'_>) -> Ret {
    let cf = cframe(&call.args, 1, 1, "sub")?;
    let v = vector3(&call.args, 2, 1, "sub")?;
    one(Push::CFrame(CFrame::from_parts(cf.position() - v, cf.rotation())))
}

fn cframe_eq_body(call: &Call<'_>) -> Ret {
    let a = call.args.payload::<CFrame>(1);
    let b = call.args.payload::<CFrame>(2);
    one(Push::Bool(a.is_some() && a == b))
}

fn cframe_to_string_body(call: &Call<'_>) -> Ret {
    let cf = cframe(&call.args, 1, 1, "tostring")?;
    let text = components(cf).iter().map(f32::to_string).collect::<Vec<_>>().join(", ");
    one(Push::Text(text))
}

// ---------------------------------------------------------------------------------------------------------------
// Color3 (userdata).
// ---------------------------------------------------------------------------------------------------------------

/// `Color3` methods.
pub(crate) const COLOR3_METHODS: &[&str] = &["Lerp", "ToHex", "ToHSV"];

lua_fn!(pub(crate) color3_new => color3_new_body);
lua_fn!(pub(crate) color3_from_rgb => color3_from_rgb_body);
lua_fn!(pub(crate) color3_from_hex => color3_from_hex_body);
lua_fn!(pub(crate) color3_from_hsv => color3_from_hsv_body);
lua_fn!(pub(crate) color3_index => color3_index_body);
lua_fn!(pub(crate) color3_method => color3_method_body);
lua_fn!(pub(crate) color3_eq => color3_eq_body);
lua_fn!(pub(crate) color3_to_string => color3_to_string_body);

fn color3_new_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let r = number_or(a, 1, 1, "new", 0.0)? as f32;
    let g = number_or(a, 2, 1, "new", 0.0)? as f32;
    let b = number_or(a, 3, 1, "new", 0.0)? as f32;
    one(Push::Color3(Color3::new(r, g, b)))
}

fn color3_from_rgb_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let r = number_or(a, 1, 1, "fromRGB", 0.0)? as f32;
    let g = number_or(a, 2, 1, "fromRGB", 0.0)? as f32;
    let b = number_or(a, 3, 1, "fromRGB", 0.0)? as f32;
    one(Push::Color3(Color3::new(r / 255.0, g / 255.0, b / 255.0)))
}

fn color3_from_hex_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let hex = a.str(1).ok_or_else(|| bad(a, 1, 1, "fromHex", "string"))?;
    match Color3::from_hex(hex) {
        Some(c) => one(Push::Color3(c)),
        None => Err(Raise(format!("Unable to convert characters to hex value: \"{hex}\""))),
    }
}

fn color3_from_hsv_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let h = number(a, 1, 1, "fromHSV")?;
    let s = number(a, 2, 1, "fromHSV")?;
    let v = number(a, 3, 1, "fromHSV")?;
    let h = (h.rem_euclid(1.0)) * 6.0;
    let sector = h.floor();
    let f = h - sector;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match sector as u8 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    one(Push::Color3(Color3::new(r as f32, g as f32, b as f32)))
}

fn color3_index_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let c = color3(a, 1, 1, "index")?;
    let key = key(a)?;
    match key {
        "R" => one(num(c.r)),
        "G" => one(num(c.g)),
        "B" => one(num(c.b)),
        _ => match COLOR3_METHODS.iter().find(|m| **m == key) {
            Some(name) => one(Push::Field {
                table: call.shared()?.refs.color3_methods,
                name,
            }),
            None => Err(not_member(key, "Color3")),
        },
    }
}

fn color3_method_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let name = method_name(a)?;
    let c = a
        .payload::<Color3>(1)
        .ok_or_else(|| Raise(format!("Expected ':' not '.' calling member function {name}")))?;
    match name {
        "Lerp" => {
            let goal = color3(a, 2, 2, name)?;
            let alpha = float(a, 3, 2, name)?;
            one(Push::Color3(c.lerp(goal, alpha)))
        }
        "ToHex" => one(Push::Text(c.to_hex())),
        "ToHSV" => {
            let (r, g, b) = (f64::from(c.r), f64::from(c.g), f64::from(c.b));
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let delta = max - min;
            let h = if delta == 0.0 {
                0.0
            } else if max == r {
                ((g - b) / delta).rem_euclid(6.0) / 6.0
            } else if max == g {
                ((b - r) / delta + 2.0) / 6.0
            } else {
                ((r - g) / delta + 4.0) / 6.0
            };
            let s = if max == 0.0 { 0.0 } else { delta / max };
            Ok(Out::Many(vec![Push::Number(h), Push::Number(s), Push::Number(max)]))
        }
        _ => Err(not_member(name, "Color3")),
    }
}

fn color3_eq_body(call: &Call<'_>) -> Ret {
    let a = call.args.payload::<Color3>(1);
    let b = call.args.payload::<Color3>(2);
    one(Push::Bool(a.is_some() && a == b))
}

fn color3_to_string_body(call: &Call<'_>) -> Ret {
    let c = color3(&call.args, 1, 1, "tostring")?;
    one(Push::Text(format!("{}, {}, {}", c.r, c.g, c.b)))
}

// ---------------------------------------------------------------------------------------------------------------
// EnumItem (userdata, one cached object per item).
// ---------------------------------------------------------------------------------------------------------------

lua_fn!(pub(crate) enum_item_index => enum_item_index_body);
lua_fn!(pub(crate) enum_item_eq => enum_item_eq_body);
lua_fn!(pub(crate) enum_item_to_string => enum_item_to_string_body);

fn enum_item_index_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let item = a
        .payload::<EnumRef>(1)
        .ok_or_else(|| bad(a, 1, 1, "index", "EnumItem"))?;
    let key = key(a)?;
    let shared = call.shared()?;
    let desc = shared
        .enums
        .desc(item)
        .ok_or_else(|| Raise::from("unknown enum item"))?;
    match key {
        "Name" => one(Push::Static(
            desc.items
                .get(usize::from(item.item_index))
                .copied()
                .unwrap_or_default(),
        )),
        "Value" => one(Push::Number(f64::from(
            desc.number(usize::from(item.item_index)).unwrap_or_default(),
        ))),
        "EnumType" => one(Push::Static(desc.name)),
        _ => Err(not_member(key, &format!("\"Enum.{}\"", desc.name))),
    }
}

fn enum_item_eq_body(call: &Call<'_>) -> Ret {
    let a = call.args.payload::<EnumRef>(1);
    let b = call.args.payload::<EnumRef>(2);
    one(Push::Bool(a.is_some() && a == b))
}

fn enum_item_to_string_body(call: &Call<'_>) -> Ret {
    let a = &call.args;
    let item = a
        .payload::<EnumRef>(1)
        .ok_or_else(|| bad(a, 1, 1, "tostring", "EnumItem"))?;
    let shared = call.shared()?;
    let text = shared
        .enums
        .item(item)
        .map_or_else(|| "EnumItem".to_owned(), ToString::to_string);
    one(Push::Text(text))
}
