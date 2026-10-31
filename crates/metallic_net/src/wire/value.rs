//! [`NetValue`]: a property value or remote argument as it travels.

use std::fmt;

use metallic_core::{CFrame, Color3, InstanceId, UDim2, Value, ValueType, Vec3};
use serde::de::{self, DeserializeSeed, EnumAccess, SeqAccess, Unexpected, VariantAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use super::netid::NetId;

/// Deepest nesting of [`NetValue::Table`] a decoder accepts (a table directly in a message is depth 1). Encoders
/// refuse deeper tables, so script code converting a Luau table must stop at this depth too.
pub const MAX_TABLE_DEPTH: usize = 32;

/// A value on the wire: every [`Value`] type, plus what remote arguments need (`Vector2`, tables).
///
/// Strings are bytes (Luau strings need not be UTF-8). A reference names the instance by [`NetId`]; `Ref(None)` is a
/// nil reference or one to an instance the receiver cannot see. Numbers may be NaN or infinite on the wire (Luau
/// allows them as remote arguments); [`NetValue::to_value`] refuses them for properties.
///
/// Postcard is the only format: the variant order below is the wire tag and must not change.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum NetValue {
    /// `nil`.
    Nil,
    /// A boolean.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A string (bytes).
    Str(Vec<u8>),
    /// A 3D vector.
    Vector3(Vec3),
    /// A 2D vector (remote arguments only; no property has this type yet).
    Vector2([f64; 2]),
    /// A colour (0..=255 channels).
    Color3(Color3),
    /// A GUI size or position.
    UDim2(UDim2),
    /// An instance reference, or none.
    Ref(Option<NetId>),
    /// A Luau table as key/value pairs in a stable order (remote arguments only).
    Table(Vec<(NetValue, NetValue)>),
    /// A position and rotation (boxed, like [`Value::CFrame`], so the enum stays small).
    CFrame(Box<NetCFrame>),
}

/// A `CFrame` on the wire: the position and the rotation as a unit quaternion, all `f64` (56 bytes instead of the
/// 96 of the full matrix). Converting back gives the matrix to within about 1e-15 per component; CFrames are always
/// orthonormal, so nothing else is lost.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NetCFrame {
    /// Translation (studs).
    pub position: Vec3,
    /// Rotation as a quaternion `[x, y, z, w]` ([`CFrame::quaternion`]).
    pub rotation: [f64; 4],
}

impl NetCFrame {
    /// The wire form of a CFrame.
    pub fn from_cframe(cf: &CFrame) -> NetCFrame {
        NetCFrame {
            position: cf.position,
            rotation: cf.quaternion(),
        }
    }

    /// The CFrame, or `None` when a component is not finite or the quaternion is far from unit length (a hostile or
    /// corrupt value; a real one has length 1).
    pub fn to_cframe(&self) -> Option<CFrame> {
        let p = self.position;
        let len = self.rotation.iter().map(|c| c * c).sum::<f64>().sqrt();
        if !all_finite(&[p.x, p.y, p.z]) || !len.is_finite() || !(0.5..=2.0).contains(&len) {
            return None;
        }
        Some(CFrame::from_quaternion(p, self.rotation))
    }
}

impl NetValue {
    /// The wire form of a property value; `net` maps an instance to its NetId (`None`: the receiver cannot see it,
    /// so the reference travels as `Ref(None)`).
    pub fn from_value(v: &Value, net: impl Fn(InstanceId) -> Option<NetId>) -> NetValue {
        match v {
            Value::Nil => NetValue::Nil,
            Value::Bool(b) => NetValue::Bool(*b),
            Value::Number(n) => NetValue::Number(*n),
            Value::String(s) => NetValue::Str(s.as_bytes().to_vec()),
            Value::Vector3(v) => NetValue::Vector3(*v),
            Value::Color3(c) => NetValue::Color3(*c),
            Value::UDim2(u) => NetValue::UDim2(*u),
            Value::Ref(id) => NetValue::Ref(id.and_then(net)),
            Value::CFrame(cf) => NetValue::CFrame(Box::new(NetCFrame::from_cframe(cf))),
        }
    }

    /// The property value for a property of type `ty`; `local` maps a NetId to the local instance (`None`: not here
    /// yet, the reference becomes nil).
    ///
    /// `None` when the wire value does not fit the type, a number is NaN or infinite, or a string is not UTF-8.
    pub fn to_value(
        &self,
        ty: ValueType,
        local: impl Fn(NetId) -> Option<InstanceId>,
    ) -> Option<Value> {
        let value = match (self, ty) {
            (NetValue::Nil, ValueType::Nil) => Value::Nil,
            (NetValue::Bool(b), ValueType::Bool) => Value::Bool(*b),
            (NetValue::Number(n), ValueType::Number) if n.is_finite() => Value::Number(*n),
            (NetValue::Str(s), ValueType::String) => {
                Value::String(String::from_utf8(s.clone()).ok()?)
            }
            (NetValue::Vector3(v), ValueType::Vector3) if all_finite(&[v.x, v.y, v.z]) => {
                Value::Vector3(*v)
            }
            (NetValue::Color3(c), ValueType::Color3) if all_finite(&[c.r, c.g, c.b]) => {
                Value::Color3(*c)
            }
            (NetValue::UDim2(u), ValueType::UDim2)
                if all_finite(&[u.x_scale, u.x_offset, u.y_scale, u.y_offset]) =>
            {
                Value::UDim2(*u)
            }
            (NetValue::Ref(n), ValueType::Ref) => Value::Ref(n.and_then(local)),
            (NetValue::CFrame(cf), ValueType::CFrame) => Value::CFrame(Box::new(cf.to_cframe()?)),
            _ => return None,
        };
        Some(value)
    }

    /// Whether this value nests tables deeper than [`MAX_TABLE_DEPTH`] (stops looking once it is).
    pub(crate) fn too_deep(&self) -> bool {
        fn deeper_than(v: &NetValue, budget: usize) -> bool {
            match v {
                NetValue::Table(pairs) => {
                    budget == 0
                        || pairs
                            .iter()
                            .any(|(k, v)| deeper_than(k, budget - 1) || deeper_than(v, budget - 1))
                }
                _ => false,
            }
        }
        deeper_than(self, MAX_TABLE_DEPTH)
    }
}

fn all_finite(xs: &[f64]) -> bool {
    xs.iter().all(|x| x.is_finite())
}

/// Wire tags, in declaration order (the derived `Serialize` uses the same indices).
const VARIANTS: &[&str] = &[
    "Nil", "Bool", "Number", "Str", "Vector3", "Vector2", "Color3", "UDim2", "Ref", "Table",
    "CFrame",
];

// Deserialize is written by hand so table nesting is capped: a derived impl recurses once per level and a 64 KiB
// frame of nested tables would overflow the stack.
impl<'de> Deserialize<'de> for NetValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        ValueSeed { depth: 0 }.deserialize(d)
    }
}

/// Deserializes one value whose enclosing tables are `depth` deep.
#[derive(Clone, Copy)]
struct ValueSeed {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for ValueSeed {
    type Value = NetValue;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<NetValue, D::Error> {
        d.deserialize_enum("NetValue", VARIANTS, self)
    }
}

impl<'de> Visitor<'de> for ValueSeed {
    type Value = NetValue;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a NetValue")
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<NetValue, A::Error> {
        let (tag, v): (u32, _) = data.variant()?;
        Ok(match tag {
            0 => {
                v.unit_variant()?;
                NetValue::Nil
            }
            1 => NetValue::Bool(v.newtype_variant()?),
            2 => NetValue::Number(v.newtype_variant()?),
            3 => NetValue::Str(v.newtype_variant()?),
            4 => NetValue::Vector3(v.newtype_variant()?),
            5 => NetValue::Vector2(v.newtype_variant()?),
            6 => NetValue::Color3(v.newtype_variant()?),
            7 => NetValue::UDim2(v.newtype_variant()?),
            8 => NetValue::Ref(v.newtype_variant()?),
            9 => {
                if self.depth >= MAX_TABLE_DEPTH {
                    return Err(de::Error::custom("tables nested too deeply"));
                }
                NetValue::Table(v.newtype_variant_seed(TableSeed {
                    depth: self.depth + 1,
                })?)
            }
            10 => NetValue::CFrame(v.newtype_variant()?),
            other => {
                return Err(de::Error::invalid_value(
                    Unexpected::Unsigned(u64::from(other)),
                    &"a NetValue tag 0..=10",
                ));
            }
        })
    }
}

/// Deserializes a table's pairs at nesting `depth`.
#[derive(Clone, Copy)]
struct TableSeed {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for TableSeed {
    type Value = Vec<(NetValue, NetValue)>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for TableSeed {
    type Value = Vec<(NetValue, NetValue)>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a table of NetValue pairs")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        // The hint is bounded by the bytes left (postcard), and capped again here.
        let mut pairs = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(64));
        while let Some(pair) = seq.next_element_seed(PairSeed { depth: self.depth })? {
            pairs.push(pair);
        }
        Ok(pairs)
    }
}

/// Deserializes one `(key, value)` pair inside a table at nesting `depth`.
#[derive(Clone, Copy)]
struct PairSeed {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for PairSeed {
    type Value = (NetValue, NetValue);
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_tuple(2, self)
    }
}

impl<'de> Visitor<'de> for PairSeed {
    type Value = (NetValue, NetValue);

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a key/value pair")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let seed = ValueSeed { depth: self.depth };
        let key = seq
            .next_element_seed(seed)?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let value = seq
            .next_element_seed(seed)?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        Ok((key, value))
    }
}
