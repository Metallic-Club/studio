//! How property and attribute values are written in a scene file and read back.

use metallic_core::{CFrame, Color3, EnumItem, Id, PropType, Registry, Value, Vector2, Vector3};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// A property value as written. Untagged: the class registry knows each property's type, so the reader does too.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub(super) enum Written {
    /// A `boolean`.
    Bool(bool),
    /// An `integer`.
    Int(i64),
    /// A `number`.
    Number(f64),
    /// A `string`, or an enum item's name.
    Text(String),
    /// A `Vector2`: `[x, y]`.
    Vector2(Vector2),
    /// A `Vector3`: `[x, y, z]`.
    Vector3(Vector3),
    /// A `CFrame`: `[px, py, pz, qx, qy, qz, qw]` (its own serialized form).
    CFrame(CFrame),
    /// A `Color3`: `[r, g, b]`.
    Color3(Color3),
    /// A reference: the target's scene id.
    Ref(u64),
}

/// What [`write`] makes of a value.
pub(super) enum Encoded {
    /// The value to write.
    Value(Written),
    /// A reference to an instance that has no scene id: it is not saved.
    NotSaved,
}

/// Writes a stored property value. `scene_id` gives a referenced instance's id in the scene. A nil reference is a
/// property's default and never reaches here, but is written as not saved for safety.
pub(super) fn write(value: &Value, scene_id: impl Fn(Id) -> Option<u64>) -> Encoded {
    Encoded::Value(match value {
        Value::Bool(b) => Written::Bool(*b),
        Value::Int(i) => Written::Int(*i),
        Value::Number(n) => Written::Number(*n),
        Value::String(s) => Written::Text(s.to_string()),
        Value::Vector2(v) => Written::Vector2(*v),
        Value::Vector3(v) => Written::Vector3(*v),
        Value::CFrame(cf) => Written::CFrame(*cf),
        Value::Color3(c) => Written::Color3(*c),
        Value::Enum(item) => Written::Text(item.name().to_owned()),
        Value::Ref(target) => match target.and_then(scene_id) {
            Some(id) => Written::Ref(id),
            None => return Encoded::NotSaved,
        },
    })
}

/// A property value as read, before references are resolved.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Read {
    /// A value ready to store.
    Value(Value),
    /// A reference to the instance with this scene id.
    Ref(u64),
}

/// Reads a value of the declared type `ty`, or `None` when it does not fit: the wrong JSON kind, a number that is
/// not finite as the stored type, or an enum item `registry` does not have.
pub(super) fn read(registry: &Registry, ty: PropType, json: &Json) -> Option<Read> {
    let value = match ty {
        PropType::Bool => Value::Bool(json.as_bool()?),
        PropType::Int => Value::Int(json.as_i64()?),
        PropType::Number => Value::Number(json.as_f64()?),
        PropType::String => Value::String(json.as_str()?.into()),
        PropType::Vector2 => Value::Vector2(Vector2::deserialize(json).ok()?),
        PropType::Vector3 => Value::Vector3(Vector3::deserialize(json).ok()?),
        PropType::CFrame => Value::CFrame(CFrame::deserialize(json).ok()?),
        PropType::Color3 => Value::Color3(Color3::deserialize(json).ok()?),
        PropType::Enum(enum_type) => Value::Enum(registry.enum_item(enum_type, json.as_str()?)?),
        PropType::Ref(_) => {
            return match json {
                Json::Null => Some(Read::Value(Value::Ref(None))),
                other => other.as_u64().map(Read::Ref),
            };
        }
    };
    is_finite(&value).then_some(Read::Value(value))
}

/// An attribute value as written: tagged with its type, since attributes declare none (`{"Number": 3.0}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) enum Attribute {
    /// A `boolean`.
    Bool(bool),
    /// An `integer`.
    Int(i64),
    /// A `number`.
    Number(f64),
    /// A `string`.
    String(String),
    /// A `Vector2`.
    Vector2(Vector2),
    /// A `Vector3`.
    Vector3(Vector3),
    /// A `CFrame`.
    CFrame(CFrame),
    /// A `Color3`.
    Color3(Color3),
    /// An enum item: `{"enum_type": "Material", "name": "Plastic"}`.
    Enum(EnumItem),
}

impl Attribute {
    /// The attribute form of `value`; `None` for a reference, which attributes cannot hold.
    pub(super) fn from_value(value: &Value) -> Option<Self> {
        Some(match value {
            Value::Bool(b) => Self::Bool(*b),
            Value::Int(i) => Self::Int(*i),
            Value::Number(n) => Self::Number(*n),
            Value::String(s) => Self::String(s.to_string()),
            Value::Vector2(v) => Self::Vector2(*v),
            Value::Vector3(v) => Self::Vector3(*v),
            Value::CFrame(cf) => Self::CFrame(*cf),
            Value::Color3(c) => Self::Color3(*c),
            Value::Enum(item) => Self::Enum(item.clone()),
            Value::Ref(_) => return None,
        })
    }

    /// Reads an attribute value, or `None` when it is not one (an unknown tag, a number that is not finite).
    pub(super) fn read(json: &Json) -> Option<Value> {
        let value = match Self::deserialize(json).ok()? {
            Self::Bool(b) => Value::Bool(b),
            Self::Int(i) => Value::Int(i),
            Self::Number(n) => Value::Number(n),
            Self::String(s) => Value::String(s.into()),
            Self::Vector2(v) => Value::Vector2(v),
            Self::Vector3(v) => Value::Vector3(v),
            Self::CFrame(cf) => Value::CFrame(cf),
            Self::Color3(c) => Value::Color3(c),
            Self::Enum(item) => Value::Enum(item),
        };
        is_finite(&value).then_some(value)
    }
}

/// Whether every number in `value` is finite, as the data model requires. A JSON number too large for an `f32`
/// component reads as infinity, which this refuses.
fn is_finite(value: &Value) -> bool {
    match value {
        Value::Number(n) => n.is_finite(),
        Value::Vector2(v) => v.is_finite(),
        Value::Vector3(v) => v.is_finite(),
        Value::Color3(c) => c.r.is_finite() && c.g.is_finite() && c.b.is_finite(),
        // `CFrame`'s own reader refuses non-finite numbers.
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use metallic_core::glam::Quat;
    use serde_json::json;

    use super::*;

    fn registry() -> std::sync::Arc<Registry> {
        metallic_classes::shared_registry().unwrap()
    }

    fn round_trip(value: Value, ty: PropType) {
        let Encoded::Value(written) = write(&value, |_| Some(7)) else {
            panic!("{value:?} was not written");
        };
        let json = serde_json::to_value(&written).unwrap();
        assert_eq!(read(&registry(), ty, &json), Some(Read::Value(value)), "{json}");
    }

    #[test]
    fn every_value_type_reads_back() {
        round_trip(Value::Bool(true), PropType::Bool);
        round_trip(Value::Int(-3), PropType::Int);
        round_trip(Value::Number(0.1), PropType::Number);
        round_trip(Value::String("Scripts/Door.luau".into()), PropType::String);
        round_trip(Value::Vector2(Vector2::new(0.1, -2.0)), PropType::Vector2);
        round_trip(Value::Vector3(Vector3::new(0.1, 2.5, -3.0)), PropType::Vector3);
        round_trip(
            Value::CFrame(CFrame::from_parts(
                Vector3::new(1.0, 2.0, 3.0),
                Quat::from_rotation_y(0.7),
            )),
            PropType::CFrame,
        );
        round_trip(Value::Color3(Color3::new(0.2, 0.4, 0.6)), PropType::Color3);
        let plastic = registry().enum_item("Material", "Plastic").unwrap();
        round_trip(Value::Enum(plastic), PropType::Enum("Material"));
    }

    #[test]
    fn references_read_as_scene_ids() {
        let Encoded::Value(written) = write(&Value::Ref(Some(Id::new(4, 0))), |_| Some(12)) else {
            panic!("not written");
        };
        assert_eq!(serde_json::to_value(&written).unwrap(), json!(12));
        assert!(matches!(
            write(&Value::Ref(Some(Id::new(4, 0))), |_| None),
            Encoded::NotSaved
        ));
        let ty = PropType::Ref("Camera");
        assert_eq!(read(&registry(), ty, &json!(12)), Some(Read::Ref(12)));
        assert_eq!(read(&registry(), ty, &json!(null)), Some(Read::Value(Value::Ref(None))));
        assert_eq!(read(&registry(), ty, &json!("Camera")), None);
    }

    #[test]
    fn values_of_the_wrong_kind_are_refused() {
        let registry = registry();
        assert_eq!(read(&registry, PropType::Bool, &json!(1)), None);
        assert_eq!(read(&registry, PropType::Int, &json!(1.5)), None);
        assert_eq!(read(&registry, PropType::Vector3, &json!([1.0, 2.0])), None);
        assert_eq!(
            read(&registry, PropType::Vector3, &json!([1.0, 2.0, 1e39])),
            None,
            "overflows f32"
        );
        assert_eq!(read(&registry, PropType::Enum("Material"), &json!("Cheese")), None);
        assert_eq!(
            read(&registry, PropType::Number, &json!(2)),
            Some(Read::Value(Value::Number(2.0)))
        );
    }

    #[test]
    fn attributes_name_their_type() {
        let value = Value::Vector3(Vector3::new(1.0, 2.0, 3.0));
        let json = serde_json::to_value(Attribute::from_value(&value).unwrap()).unwrap();
        assert_eq!(json, json!({"Vector3": [1.0, 2.0, 3.0]}));
        assert_eq!(Attribute::read(&json), Some(value));
        assert_eq!(Attribute::read(&json!({"Instance": 3})), None);
        assert_eq!(Attribute::from_value(&Value::Ref(None)), None);
    }
}
