//! `Value` and `PropType`: what a property stores and what it is declared to store.

use std::fmt;
use std::sync::Arc;

use crate::{CFrame, Color3, EnumItem, Id, Vector2, Vector3};

/// A property value as the data model stores it. Every variant is `Copy` or reference-counted, so cloning is cheap.
///
/// There is deliberately no serde impl: writing instance references and enum items needs scene context (stable ids
/// and the registry), so the scene format defines how a `Value` is saved.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `true` or `false`.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A real number.
    Number(f64),
    /// Text.
    String(Arc<str>),
    /// A 2D vector.
    Vector2(Vector2),
    /// A 3D vector.
    Vector3(Vector3),
    /// A position and rotation.
    CFrame(CFrame),
    /// An sRGB colour.
    Color3(Color3),
    /// An item of an engine enum.
    Enum(EnumItem),
    /// A reference to another instance, or `None` for no instance (nil).
    Ref(Option<Id>),
}

impl Value {
    /// Whether this value has the kind `ty` declares.
    ///
    /// The check is strict: an `Int` does not match `Number` (coercion happens when a script value is converted, in
    /// [`ScriptValue::to_value`](crate::ScriptValue::to_value)). An enum item matches only its own enum type. A
    /// reference matches any `Ref` type, because checking the class of the target needs the data model.
    pub fn prop_type_matches(&self, ty: &PropType) -> bool {
        match (self, ty) {
            (Self::Enum(item), PropType::Enum(enum_type)) => item.enum_type() == *enum_type,
            (Self::Bool(_), PropType::Bool)
            | (Self::Int(_), PropType::Int)
            | (Self::Number(_), PropType::Number)
            | (Self::String(_), PropType::String)
            | (Self::Vector2(_), PropType::Vector2)
            | (Self::Vector3(_), PropType::Vector3)
            | (Self::CFrame(_), PropType::CFrame)
            | (Self::Color3(_), PropType::Color3)
            | (Self::Ref(_), PropType::Ref(_)) => true,
            _ => false,
        }
    }
}

/// The declared type of a property, one per [`Value`] kind. Names are `&'static str` because they come from the
/// static class descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropType {
    /// [`Value::Bool`].
    Bool,
    /// [`Value::Int`].
    Int,
    /// [`Value::Number`].
    Number,
    /// [`Value::String`].
    String,
    /// [`Value::Vector2`].
    Vector2,
    /// [`Value::Vector3`].
    Vector3,
    /// [`Value::CFrame`].
    CFrame,
    /// [`Value::Color3`].
    Color3,
    /// [`Value::Enum`] of the named enum type, such as `Material`.
    Enum(&'static str),
    /// [`Value::Ref`] to an instance of the named class (or a subclass), such as `BasePart`.
    Ref(&'static str),
}

/// Writes the type as a game developer reads it in an error message: `number`, `Enum.Material`, `BasePart`.
impl fmt::Display for PropType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => f.write_str("boolean"),
            Self::Int => f.write_str("integer"),
            Self::Number => f.write_str("number"),
            Self::String => f.write_str("string"),
            Self::Vector2 => f.write_str("Vector2"),
            Self::Vector3 => f.write_str("Vector3"),
            Self::CFrame => f.write_str("CFrame"),
            Self::Color3 => f.write_str("Color3"),
            Self::Enum(enum_type) => write!(f, "Enum.{enum_type}"),
            Self::Ref(class) => f.write_str(class),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_of_each() -> Vec<(Value, PropType)> {
        vec![
            (Value::Bool(true), PropType::Bool),
            (Value::Int(3), PropType::Int),
            (Value::Number(1.5), PropType::Number),
            (Value::String("hi".into()), PropType::String),
            (Value::Vector2(Vector2::ONE), PropType::Vector2),
            (Value::Vector3(Vector3::ONE), PropType::Vector3),
            (Value::CFrame(CFrame::IDENTITY), PropType::CFrame),
            (Value::Color3(Color3::WHITE), PropType::Color3),
            (
                Value::Enum(EnumItem::new("Material", "Plastic")),
                PropType::Enum("Material"),
            ),
            (Value::Ref(Some(Id::new(1, 0))), PropType::Ref("BasePart")),
        ]
    }

    #[test]
    fn each_value_matches_only_its_own_type() {
        let cases = one_of_each();
        for (i, (value, _)) in cases.iter().enumerate() {
            for (j, (_, ty)) in cases.iter().enumerate() {
                assert_eq!(value.prop_type_matches(ty), i == j, "{value:?} against {ty:?}");
            }
        }
    }

    #[test]
    fn enum_must_match_its_enum_type() {
        let item = Value::Enum(EnumItem::new("Material", "Plastic"));
        assert!(!item.prop_type_matches(&PropType::Enum("Shape")));
    }

    #[test]
    fn nil_ref_matches_any_ref_type() {
        assert!(Value::Ref(None).prop_type_matches(&PropType::Ref("Instance")));
        assert!(Value::Ref(Some(Id::new(0, 0))).prop_type_matches(&PropType::Ref("Model")));
    }

    #[test]
    fn fits_the_per_instance_budget() {
        // Every property of every instance is a `Value`; 100k-part scenes need it to stay this small.
        assert!(size_of::<Value>() <= 32, "Value is {} bytes", size_of::<Value>());
    }

    #[test]
    fn prop_type_display_reads_well() {
        assert_eq!(PropType::Int.to_string(), "integer");
        assert_eq!(PropType::Enum("Material").to_string(), "Enum.Material");
        assert_eq!(PropType::Ref("BasePart").to_string(), "BasePart");
    }
}
