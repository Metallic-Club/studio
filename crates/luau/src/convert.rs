//! Conversions between Luau values and engine values, in one place. Pushing is in `ffi.rs` (`Push`,
//! `push_value`), because it touches the VM; reading is here, on top of [`Args`].

use std::ffi::c_int;
use std::sync::Arc;

use metallic_core::{CFrame, Color3, Id, PropType, ScriptValue, Value, Vector2};

use crate::ffi::{Args, EnumRef, Kind, Raise, tag};
use crate::shared::Shared;

/// How deep nested tables may be when passed to the engine.
const MAX_DEPTH: u32 = 16;

/// Value `idx` as an engine value: method arguments, return values, event arguments.
pub(crate) fn read_script(args: &Args<'_>, idx: c_int, shared: &Shared) -> Result<ScriptValue, Raise> {
    read_nested(args, idx, shared, 0)
}

fn read_nested(args: &Args<'_>, idx: c_int, shared: &Shared, depth: u32) -> Result<ScriptValue, Raise> {
    Ok(match args.kind(idx) {
        Kind::None | Kind::Nil => ScriptValue::Nil,
        Kind::Bool => ScriptValue::Bool(args.boolean(idx).unwrap_or(false)),
        Kind::Number => ScriptValue::Number(args.number(idx).unwrap_or(0.0)),
        Kind::String => {
            let bytes = args.bytes(idx).unwrap_or_default();
            ScriptValue::String(Arc::from(String::from_utf8_lossy(bytes).as_ref()))
        }
        Kind::Vector => ScriptValue::Vector3(args.vector(idx).unwrap_or_default()),
        Kind::Userdata(tag::INSTANCE) => args.payload::<Id>(idx).map_or(ScriptValue::Nil, ScriptValue::Instance),
        Kind::Userdata(tag::VECTOR2) => ScriptValue::Vector2(args.payload::<Vector2>(idx).unwrap_or_default()),
        Kind::Userdata(tag::CFRAME) => ScriptValue::CFrame(args.payload::<CFrame>(idx).unwrap_or_default()),
        Kind::Userdata(tag::COLOR3) => ScriptValue::Color3(args.payload::<Color3>(idx).unwrap_or_default()),
        Kind::Userdata(tag::ENUM_ITEM) => args
            .payload::<EnumRef>(idx)
            .and_then(|item| shared.enums.item(item))
            .map_or(ScriptValue::Nil, |item| ScriptValue::Enum(item.clone())),
        Kind::Table => {
            if depth >= MAX_DEPTH {
                return Err(Raise::from("tables passed to the engine are nested too deeply"));
            }
            read_table(args, idx, shared, depth + 1)?
        }
        kind => {
            return Err(Raise(format!(
                "a {} cannot be passed to the engine",
                crate::ffi::kind_name(kind)
            )));
        }
    })
}

/// A table as a list (when it has an array part) or a map with string keys.
fn read_table(args: &Args<'_>, idx: c_int, shared: &Shared, depth: u32) -> Result<ScriptValue, Raise> {
    let mut list = Vec::new();
    args.each_array(idx, |element| {
        list.push(read_nested(element, -1, shared, depth)?);
        Ok::<_, Raise>(())
    })?;
    if !list.is_empty() {
        return Ok(ScriptValue::List(list));
    }
    let mut map = Vec::new();
    args.each_pair(idx, |pair, key| {
        let Some(key) = key else {
            return Err(Raise::from("tables passed to the engine need string keys or a list"));
        };
        let key = Arc::from(key);
        map.push((key, read_nested(pair, -1, shared, depth)?));
        Ok(())
    })?;
    Ok(ScriptValue::Map(map))
}

/// Value `idx` as a value for a property of type `ty`, checked as `ScriptValue::to_value` checks it. The common
/// cases skip the intermediate `ScriptValue`.
pub(crate) fn read_value(args: &Args<'_>, idx: c_int, ty: &PropType, shared: &Shared) -> Result<Value, Raise> {
    let fast = match (ty, args.kind(idx)) {
        (PropType::Number, Kind::Number) => args.number(idx).map(Value::Number),
        (PropType::Bool, Kind::Bool) => args.boolean(idx).map(Value::Bool),
        (PropType::Vector3, Kind::Vector) => args.vector(idx).map(Value::Vector3),
        (PropType::CFrame, Kind::Userdata(tag::CFRAME)) => args.payload::<CFrame>(idx).map(Value::CFrame),
        (PropType::Color3, Kind::Userdata(tag::COLOR3)) => args.payload::<Color3>(idx).map(Value::Color3),
        (PropType::Vector2, Kind::Userdata(tag::VECTOR2)) => args.payload::<Vector2>(idx).map(Value::Vector2),
        (PropType::Ref(_), Kind::Userdata(tag::INSTANCE)) => args.payload::<Id>(idx).map(|id| Value::Ref(Some(id))),
        (PropType::Ref(_), Kind::Nil) => Some(Value::Ref(None)),
        (PropType::Enum(enum_type), Kind::Userdata(tag::ENUM_ITEM)) => args
            .payload::<EnumRef>(idx)
            .and_then(|item| shared.enums.item(item))
            .filter(|item| item.enum_type() == *enum_type)
            .map(|item| Value::Enum(item.clone())),
        _ => None,
    };
    match fast {
        Some(value) => Ok(value),
        None => Ok(read_script(args, idx, shared)?.to_value(ty)?),
    }
}
