//! Property and attribute values on the wire: every [`Value`] kind with its exact bits, references by [`NetId`].

use metallic_core::{CFrame, Color3, Id, PropType, Registry, Value, Vector2, Vector3};

use super::scene::NetId;
use crate::codec::{CodecError, Reader, Writer};

/// The longest text value (a `String` property, an attribute, a name), in bytes.
pub const MAX_STRING: usize = 256 << 10;

/// A value as it travels: a reference names a [`NetId`], not a local id.
#[derive(Clone, Debug, PartialEq)]
pub enum WireValue {
    /// A boolean.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A real number.
    Number(f64),
    /// Text.
    String(String),
    /// A 2D vector.
    Vector2(Vector2),
    /// A 3D vector.
    Vector3(Vector3),
    /// A position and rotation, as its seven stored numbers.
    CFrame([f32; 7]),
    /// A colour.
    Color3(Color3),
    /// An enum item: its enum's name and its own.
    Enum(String, String),
    /// A reference, or nil.
    Ref(Option<NetId>),
}

impl WireValue {
    /// `value` as it travels, references through `net` (an instance with no net id is sent as nil).
    pub fn of(value: &Value, net: impl Fn(Id) -> Option<NetId>) -> Self {
        match value {
            Value::Bool(b) => Self::Bool(*b),
            Value::Int(i) => Self::Int(*i),
            Value::Number(n) => Self::Number(*n),
            Value::String(s) => Self::String(s.to_string()),
            Value::Vector2(v) => Self::Vector2(*v),
            Value::Vector3(v) => Self::Vector3(*v),
            Value::CFrame(cf) => Self::CFrame(cf.to_array()),
            Value::Color3(c) => Self::Color3(*c),
            Value::Enum(item) => Self::Enum(item.enum_type().to_owned(), item.name().to_owned()),
            Value::Ref(target) => Self::Ref(target.and_then(net)),
        }
    }

    /// The local value, references through `local`, enum items from `registry`. `None` when it cannot be one: a
    /// rotation that is not a rotation, an enum item that does not exist.
    pub fn to_value(&self, registry: &Registry, local: impl Fn(NetId) -> Option<Id>) -> Option<Value> {
        Some(match self {
            Self::Bool(b) => Value::Bool(*b),
            Self::Int(i) => Value::Int(*i),
            Self::Number(n) => Value::Number(*n),
            Self::String(s) => Value::String(s.as_str().into()),
            Self::Vector2(v) => Value::Vector2(*v),
            Self::Vector3(v) => Value::Vector3(*v),
            Self::CFrame(parts) => Value::CFrame(CFrame::from_array(*parts)?),
            Self::Color3(c) => Value::Color3(*c),
            Self::Enum(ty, name) => Value::Enum(registry.enum_item(ty, name)?),
            Self::Ref(target) => Value::Ref(target.and_then(local)),
        })
    }

    /// Whether it can be stored in a property of type `ty`.
    pub fn fits(&self, ty: &PropType) -> bool {
        matches!(
            (self, ty),
            (Self::Bool(_), PropType::Bool)
                | (Self::Int(_), PropType::Int)
                | (Self::Number(_), PropType::Number)
                | (Self::String(_), PropType::String)
                | (Self::Vector2(_), PropType::Vector2)
                | (Self::Vector3(_), PropType::Vector3)
                | (Self::CFrame(_), PropType::CFrame)
                | (Self::Color3(_), PropType::Color3)
                | (Self::Ref(_), PropType::Ref(_))
        ) || matches!((self, ty), (Self::Enum(a, _), PropType::Enum(b)) if a == b)
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        match self {
            Self::Bool(b) => {
                w.u8(0);
                w.bool(*b);
            }
            Self::Int(i) => {
                w.u8(1);
                w.i64(*i);
            }
            Self::Number(n) => {
                w.u8(2);
                w.f64(*n);
            }
            Self::String(s) => {
                w.u8(3);
                w.str(s);
            }
            Self::Vector2(v) => {
                w.u8(4);
                w.f32(v.x);
                w.f32(v.y);
            }
            Self::Vector3(v) => {
                w.u8(5);
                w.f32(v.x);
                w.f32(v.y);
                w.f32(v.z);
            }
            Self::CFrame(parts) => {
                w.u8(6);
                for part in parts {
                    w.f32(*part);
                }
            }
            Self::Color3(c) => {
                w.u8(7);
                w.f32(c.r);
                w.f32(c.g);
                w.f32(c.b);
            }
            Self::Enum(ty, name) => {
                w.u8(8);
                w.str(ty);
                w.str(name);
            }
            Self::Ref(target) => {
                w.u8(9);
                w.option(*target, |w, id| w.u64(id.0));
            }
        }
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        Ok(match r.u8()? {
            0 => Self::Bool(r.bool()?),
            1 => Self::Int(r.i64()?),
            2 => Self::Number(finite64(r.f64()?)?),
            3 => Self::String(r.str("a text value", MAX_STRING)?.to_owned()),
            4 => Self::Vector2(Vector2::new(finite(r.f32()?)?, finite(r.f32()?)?)),
            5 => Self::Vector3(Vector3::new(finite(r.f32()?)?, finite(r.f32()?)?, finite(r.f32()?)?)),
            6 => {
                let mut parts = [0.0; 7];
                for part in &mut parts {
                    *part = finite(r.f32()?)?;
                }
                Self::CFrame(parts)
            }
            7 => Self::Color3(Color3 {
                r: finite(r.f32()?)?,
                g: finite(r.f32()?)?,
                b: finite(r.f32()?)?,
            }),
            8 => Self::Enum(
                r.str("an enum", 128)?.to_owned(),
                r.str("an enum item", 128)?.to_owned(),
            ),
            9 => Self::Ref(r.option(|r| r.u64().map(NetId))?),
            tag => return Err(CodecError::BadTag { what: "a value", tag }),
        })
    }
}

fn finite(value: f32) -> Result<f32, CodecError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CodecError::Invalid("a number that is not finite"))
    }
}

fn finite64(value: f64) -> Result<f64, CodecError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CodecError::Invalid("a number that is not finite"))
    }
}
