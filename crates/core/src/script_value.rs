//! `ScriptValue` and `ScriptError`: the language-neutral boundary between the engine and every scripting backend.

use std::sync::Arc;

use thiserror::Error;

use crate::{CFrame, Color3, EnumItem, Id, PropType, Value, Vector2, Vector3};

/// A value passed into or out of a script, in any scripting language. Backends translate it to and from their own
/// native values; the engine never sees a VM type.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ScriptValue {
    /// No value.
    #[default]
    Nil,
    /// `true` or `false`.
    Bool(bool),
    /// A number. Scripts have one number type; integer properties are checked on conversion.
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
    /// A live instance.
    Instance(Id),
    /// An item of an engine enum.
    Enum(EnumItem),
    /// An ordered list (an array-like table in Luau).
    List(Vec<ScriptValue>),
    /// String keys to values, in a stable order (a dictionary-like table in Luau).
    Map(Vec<(Arc<str>, ScriptValue)>),
    /// Several values at once, such as a method's multiple return values.
    Tuple(Vec<ScriptValue>),
}

impl ScriptValue {
    /// The type name shown to game developers in error messages: `nil`, `boolean`, `number`, `Vector3`, ...
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Bool(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Vector2(_) => "Vector2",
            Self::Vector3(_) => "Vector3",
            Self::CFrame(_) => "CFrame",
            Self::Color3(_) => "Color3",
            Self::Instance(_) => "Instance",
            Self::Enum(_) => "EnumItem",
            Self::List(_) => "list",
            Self::Map(_) => "map",
            Self::Tuple(_) => "tuple",
        }
    }

    /// Whether this is [`ScriptValue::Nil`].
    pub fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }

    /// Converts a script value into a value for a property of type `ty`, for property writes and method arguments.
    ///
    /// The coercions are: a `Number` becomes an `Int` only when it is a whole number in range; `Nil` becomes
    /// `Ref(None)`; a `String` is accepted for an enum property as the item of that name (the registry checks the
    /// name exists). Everything else must already be the right kind.
    pub fn to_value(&self, ty: &PropType) -> Result<Value, ScriptError> {
        let value = match (ty, self) {
            (PropType::Bool, Self::Bool(b)) => Value::Bool(*b),
            (PropType::Int, Self::Number(n)) => {
                Value::Int(number_to_int(*n).ok_or_else(|| ScriptError::wrong_type(ty.to_string(), n.to_string()))?)
            }
            (PropType::Number, Self::Number(n)) => Value::Number(*n),
            (PropType::String, Self::String(s)) => Value::String(s.clone()),
            (PropType::Vector2, Self::Vector2(v)) => Value::Vector2(*v),
            (PropType::Vector3, Self::Vector3(v)) => Value::Vector3(*v),
            (PropType::CFrame, Self::CFrame(cf)) => Value::CFrame(*cf),
            (PropType::Color3, Self::Color3(c)) => Value::Color3(*c),
            (PropType::Enum(enum_type), Self::Enum(item)) if item.enum_type() == *enum_type => {
                Value::Enum(item.clone())
            }
            (PropType::Enum(enum_type), Self::String(name)) => Value::Enum(EnumItem::new(*enum_type, name.clone())),
            (PropType::Ref(_), Self::Instance(id)) => Value::Ref(Some(*id)),
            (PropType::Ref(_), Self::Nil) => Value::Ref(None),
            _ => return Err(ScriptError::wrong_type(ty.to_string(), self.describe())),
        };
        Ok(value)
    }

    /// What this value is, for the "got ..." half of an error. Enum items name their enum so that a mix-up between
    /// two enums is visible.
    fn describe(&self) -> String {
        match self {
            Self::Enum(item) => format!("Enum.{}", item.enum_type()),
            other => other.type_name().to_owned(),
        }
    }
}

/// The whole number `n` holds exactly, if any. Scripts only have `f64` numbers, so integer properties and arguments
/// go through this.
pub(crate) fn number_to_int(n: f64) -> Option<i64> {
    // `i64::MAX as f64` rounds up to 2^63, which is out of range, hence the strict upper bound.
    let in_range = n >= i64::MIN as f64 && n < i64::MAX as f64;
    (in_range && n.fract() == 0.0).then_some(n as i64)
}

impl From<Value> for ScriptValue {
    /// Hands a stored property value to a script: integers become numbers and a nil reference becomes `Nil`.
    fn from(value: Value) -> Self {
        match value {
            Value::Bool(b) => Self::Bool(b),
            Value::Int(i) => Self::Number(i as f64),
            Value::Number(n) => Self::Number(n),
            Value::String(s) => Self::String(s),
            Value::Vector2(v) => Self::Vector2(v),
            Value::Vector3(v) => Self::Vector3(v),
            Value::CFrame(cf) => Self::CFrame(cf),
            Value::Color3(c) => Self::Color3(c),
            Value::Enum(item) => Self::Enum(item),
            Value::Ref(Some(id)) => Self::Instance(id),
            Value::Ref(None) => Self::Nil,
        }
    }
}

impl From<bool> for ScriptValue {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<f64> for ScriptValue {
    fn from(n: f64) -> Self {
        Self::Number(n)
    }
}

impl From<&str> for ScriptValue {
    fn from(s: &str) -> Self {
        Self::String(s.into())
    }
}

impl From<Arc<str>> for ScriptValue {
    fn from(s: Arc<str>) -> Self {
        Self::String(s)
    }
}

impl From<Vector2> for ScriptValue {
    fn from(v: Vector2) -> Self {
        Self::Vector2(v)
    }
}

impl From<Vector3> for ScriptValue {
    fn from(v: Vector3) -> Self {
        Self::Vector3(v)
    }
}

impl From<CFrame> for ScriptValue {
    fn from(cf: CFrame) -> Self {
        Self::CFrame(cf)
    }
}

impl From<Color3> for ScriptValue {
    fn from(c: Color3) -> Self {
        Self::Color3(c)
    }
}

impl From<Id> for ScriptValue {
    fn from(id: Id) -> Self {
        Self::Instance(id)
    }
}

impl From<EnumItem> for ScriptValue {
    fn from(item: EnumItem) -> Self {
        Self::Enum(item)
    }
}

/// An error raised into a script by an engine call. Messages are written for game developers, in the style of
/// `bad argument #1 (expected number, got string)`.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum ScriptError {
    /// An argument of a call had the wrong type or was missing.
    #[error("bad argument #{index} (expected {expected}, got {got})")]
    BadArgument {
        /// The argument's position as the script writer counts it, starting at 1 (the receiver of a method call is
        /// not counted).
        index: usize,
        /// What the argument should have been, such as `number` or `Enum.Material`.
        expected: String,
        /// What was passed, such as `string`, `nil` or `no value`.
        got: String,
    },
    /// A value had the wrong type, outside an argument list (for example a property assignment).
    #[error("expected {expected}, got {got}")]
    WrongType {
        /// What the value should have been.
        expected: String,
        /// What was given.
        got: String,
    },
    /// Any other failure, with a message for the game developer.
    #[error("{0}")]
    Runtime(String),
}

impl ScriptError {
    /// A [`ScriptError::BadArgument`]. `index` is 1-based, as shown to the script writer.
    pub fn bad_argument(index: usize, expected: impl Into<String>, got: impl Into<String>) -> Self {
        Self::BadArgument {
            index,
            expected: expected.into(),
            got: got.into(),
        }
    }

    /// A [`ScriptError::WrongType`].
    pub fn wrong_type(expected: impl Into<String>, got: impl Into<String>) -> Self {
        Self::WrongType {
            expected: expected.into(),
            got: got.into(),
        }
    }

    /// A [`ScriptError::Runtime`] with a free-form message.
    pub fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime(message.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_reach_scripts_unchanged_except_int_and_nil_ref() {
        let id = Id::new(4, 2);
        let item = EnumItem::new("Material", "Plastic");
        let cases = [
            (Value::Bool(true), ScriptValue::Bool(true)),
            (Value::Int(-7), ScriptValue::Number(-7.0)),
            (Value::Number(0.5), ScriptValue::Number(0.5)),
            (Value::String("a".into()), ScriptValue::String("a".into())),
            (Value::Vector2(Vector2::X), ScriptValue::Vector2(Vector2::X)),
            (Value::Vector3(Vector3::Y), ScriptValue::Vector3(Vector3::Y)),
            (Value::CFrame(CFrame::IDENTITY), ScriptValue::CFrame(CFrame::IDENTITY)),
            (Value::Color3(Color3::WHITE), ScriptValue::Color3(Color3::WHITE)),
            (Value::Enum(item.clone()), ScriptValue::Enum(item)),
            (Value::Ref(Some(id)), ScriptValue::Instance(id)),
            (Value::Ref(None), ScriptValue::Nil),
        ];
        for (value, expected) in cases {
            assert_eq!(ScriptValue::from(value), expected);
        }
    }

    #[test]
    fn every_value_round_trips_through_a_script() {
        let values = [
            (Value::Bool(false), PropType::Bool),
            (Value::Int(42), PropType::Int),
            (Value::Number(-1.25), PropType::Number),
            (Value::String("Part".into()), PropType::String),
            (Value::Vector2(Vector2::new(1.0, 2.0)), PropType::Vector2),
            (Value::Vector3(Vector3::new(1.0, 2.0, 3.0)), PropType::Vector3),
            (
                Value::CFrame(CFrame::from_euler_xyz_deg(10.0, 20.0, 30.0)),
                PropType::CFrame,
            ),
            (Value::Color3(Color3::new(0.1, 0.2, 0.3)), PropType::Color3),
            (
                Value::Enum(EnumItem::new("Material", "Wood")),
                PropType::Enum("Material"),
            ),
            (Value::Ref(Some(Id::new(9, 1))), PropType::Ref("BasePart")),
            (Value::Ref(None), PropType::Ref("BasePart")),
        ];
        for (value, ty) in values {
            let back = ScriptValue::from(value.clone()).to_value(&ty).expect("round trip");
            assert_eq!(back, value);
            assert!(back.prop_type_matches(&ty));
        }
    }

    #[test]
    fn integral_numbers_coerce_to_int() {
        assert_eq!(ScriptValue::Number(3.0).to_value(&PropType::Int), Ok(Value::Int(3)));
        assert_eq!(ScriptValue::Number(-0.0).to_value(&PropType::Int), Ok(Value::Int(0)));
        let err = ScriptValue::Number(1.5)
            .to_value(&PropType::Int)
            .expect_err("fractional");
        assert_eq!(err.to_string(), "expected integer, got 1.5");
        for n in [f64::NAN, f64::INFINITY, 1e19, -1e19] {
            assert!(ScriptValue::Number(n).to_value(&PropType::Int).is_err(), "{n}");
        }
    }

    #[test]
    fn strings_coerce_to_enum_items() {
        let value = ScriptValue::from("Neon").to_value(&PropType::Enum("Material"));
        assert_eq!(value, Ok(Value::Enum(EnumItem::new("Material", "Neon"))));
    }

    #[test]
    fn enum_of_another_type_is_rejected() {
        let shape = ScriptValue::Enum(EnumItem::new("Shape", "Ball"));
        let err = shape.to_value(&PropType::Enum("Material")).expect_err("wrong enum");
        assert_eq!(err.to_string(), "expected Enum.Material, got Enum.Shape");
    }

    #[test]
    fn wrong_kinds_are_rejected() {
        let cases = [
            (ScriptValue::from("1"), PropType::Number, "expected number, got string"),
            (ScriptValue::Number(1.0), PropType::Bool, "expected boolean, got number"),
            (ScriptValue::Nil, PropType::Vector3, "expected Vector3, got nil"),
            (
                ScriptValue::Number(1.0),
                PropType::Ref("BasePart"),
                "expected BasePart, got number",
            ),
            (
                ScriptValue::Instance(Id::new(0, 0)),
                PropType::String,
                "expected string, got Instance",
            ),
            (
                ScriptValue::Tuple(vec![]),
                PropType::CFrame,
                "expected CFrame, got tuple",
            ),
            (ScriptValue::Bool(true), PropType::Int, "expected integer, got boolean"),
        ];
        for (value, ty, message) in cases {
            assert_eq!(value.to_value(&ty).expect_err("wrong kind").to_string(), message);
        }
    }

    #[test]
    fn number_to_int_bounds() {
        assert_eq!(number_to_int(-9_223_372_036_854_775_808.0), Some(i64::MIN));
        assert_eq!(number_to_int(9_223_372_036_854_775_808.0), None);
        assert_eq!(number_to_int(1e15), Some(1_000_000_000_000_000));
    }

    #[test]
    fn error_messages_read_well() {
        assert_eq!(
            ScriptError::bad_argument(1, "number", "string").to_string(),
            "bad argument #1 (expected number, got string)"
        );
        assert_eq!(ScriptError::runtime("Part is locked").to_string(), "Part is locked");
    }
}
