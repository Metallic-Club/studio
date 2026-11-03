//! `Args`: the arguments of an engine call made from a script.

use std::sync::Arc;

use crate::script_value::number_to_int;
use crate::{CFrame, Color3, EnumItem, Id, ScriptError, ScriptValue, Vector2, Vector3};

/// The arguments of a method or function call made from a script, without the receiver.
///
/// The typed accessors take a 0-based index and return [`ScriptError::BadArgument`] with the 1-based position the
/// script writer sees, for example `bad argument #1 (expected number, got string)`. A missing argument reads as
/// `got no value`. The `opt_*` accessors return `Ok(None)` for `nil` or a missing argument, and still fail on a
/// value of the wrong type.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    values: Vec<ScriptValue>,
}

impl Args {
    /// Wraps the values a script passed.
    pub fn new(values: Vec<ScriptValue>) -> Self {
        Self { values }
    }

    /// How many arguments were passed (trailing `nil`s included, if the backend kept them).
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no arguments were passed.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The raw argument at `i`, or `None` when it is missing.
    pub fn get(&self, i: usize) -> Option<&ScriptValue> {
        self.values.get(i)
    }

    /// All arguments, in order.
    pub fn as_slice(&self) -> &[ScriptValue] {
        &self.values
    }

    /// Takes the arguments back out.
    pub fn into_vec(self) -> Vec<ScriptValue> {
        self.values
    }

    /// Argument `i` as a number.
    pub fn number(&self, i: usize) -> Result<f64, ScriptError> {
        self.required(i, "number", self.opt_number(i))
    }

    /// Argument `i` as a number, or `None` when it is `nil` or missing.
    pub fn opt_number(&self, i: usize) -> Result<Option<f64>, ScriptError> {
        self.optional(i, "number", |v| match v {
            ScriptValue::Number(n) => Some(*n),
            _ => None,
        })
    }

    /// Argument `i` as a whole number. A number with a fractional part, or out of `i64` range, is an error.
    pub fn int(&self, i: usize) -> Result<i64, ScriptError> {
        self.required(i, "integer", self.opt_int(i))
    }

    /// Argument `i` as a whole number, or `None` when it is `nil` or missing.
    pub fn opt_int(&self, i: usize) -> Result<Option<i64>, ScriptError> {
        match self.values.get(i) {
            Some(ScriptValue::Number(n)) => number_to_int(*n)
                .map(Some)
                .ok_or_else(|| ScriptError::bad_argument(i + 1, "integer", n.to_string())),
            _ => self.optional(i, "integer", |_| None),
        }
    }

    /// Argument `i` as a string.
    pub fn string(&self, i: usize) -> Result<Arc<str>, ScriptError> {
        self.required(i, "string", self.opt_string(i))
    }

    /// Argument `i` as a string, or `None` when it is `nil` or missing.
    pub fn opt_string(&self, i: usize) -> Result<Option<Arc<str>>, ScriptError> {
        self.optional(i, "string", |v| match v {
            ScriptValue::String(s) => Some(s.clone()),
            _ => None,
        })
    }

    /// Argument `i` as a boolean.
    pub fn bool(&self, i: usize) -> Result<bool, ScriptError> {
        self.required(i, "boolean", self.opt_bool(i))
    }

    /// Argument `i` as a boolean, or `None` when it is `nil` or missing.
    pub fn opt_bool(&self, i: usize) -> Result<Option<bool>, ScriptError> {
        self.optional(i, "boolean", |v| match v {
            ScriptValue::Bool(b) => Some(*b),
            _ => None,
        })
    }

    /// Argument `i` as a `Vector2`.
    pub fn vector2(&self, i: usize) -> Result<Vector2, ScriptError> {
        self.required(i, "Vector2", self.opt_vector2(i))
    }

    /// Argument `i` as a `Vector2`, or `None` when it is `nil` or missing.
    pub fn opt_vector2(&self, i: usize) -> Result<Option<Vector2>, ScriptError> {
        self.optional(i, "Vector2", |v| match v {
            ScriptValue::Vector2(v) => Some(*v),
            _ => None,
        })
    }

    /// Argument `i` as a `Vector3`.
    pub fn vector3(&self, i: usize) -> Result<Vector3, ScriptError> {
        self.required(i, "Vector3", self.opt_vector3(i))
    }

    /// Argument `i` as a `Vector3`, or `None` when it is `nil` or missing.
    pub fn opt_vector3(&self, i: usize) -> Result<Option<Vector3>, ScriptError> {
        self.optional(i, "Vector3", |v| match v {
            ScriptValue::Vector3(v) => Some(*v),
            _ => None,
        })
    }

    /// Argument `i` as a `CFrame`.
    pub fn cframe(&self, i: usize) -> Result<CFrame, ScriptError> {
        self.required(i, "CFrame", self.opt_cframe(i))
    }

    /// Argument `i` as a `CFrame`, or `None` when it is `nil` or missing.
    pub fn opt_cframe(&self, i: usize) -> Result<Option<CFrame>, ScriptError> {
        self.optional(i, "CFrame", |v| match v {
            ScriptValue::CFrame(cf) => Some(*cf),
            _ => None,
        })
    }

    /// Argument `i` as a `Color3`.
    pub fn color3(&self, i: usize) -> Result<Color3, ScriptError> {
        self.required(i, "Color3", self.opt_color3(i))
    }

    /// Argument `i` as a `Color3`, or `None` when it is `nil` or missing.
    pub fn opt_color3(&self, i: usize) -> Result<Option<Color3>, ScriptError> {
        self.optional(i, "Color3", |v| match v {
            ScriptValue::Color3(c) => Some(*c),
            _ => None,
        })
    }

    /// Argument `i` as an instance. Whether the instance is still alive is the data model's check.
    pub fn instance(&self, i: usize) -> Result<Id, ScriptError> {
        self.required(i, "Instance", self.opt_instance(i))
    }

    /// Argument `i` as an instance, or `None` when it is `nil` or missing.
    pub fn opt_instance(&self, i: usize) -> Result<Option<Id>, ScriptError> {
        self.optional(i, "Instance", |v| match v {
            ScriptValue::Instance(id) => Some(*id),
            _ => None,
        })
    }

    /// Argument `i` as an enum item of any enum type. Accepting a string for a known enum type is done by
    /// [`ScriptValue::to_value`].
    pub fn enum_item(&self, i: usize) -> Result<EnumItem, ScriptError> {
        self.required(i, "EnumItem", self.opt_enum_item(i))
    }

    /// Argument `i` as an enum item, or `None` when it is `nil` or missing.
    pub fn opt_enum_item(&self, i: usize) -> Result<Option<EnumItem>, ScriptError> {
        self.optional(i, "EnumItem", |v| match v {
            ScriptValue::Enum(item) => Some(item.clone()),
            _ => None,
        })
    }

    /// Reads an optional argument: `nil` or missing is `None`, a value `pick` rejects is a bad argument.
    fn optional<T>(
        &self,
        i: usize,
        expected: &str,
        pick: impl FnOnce(&ScriptValue) -> Option<T>,
    ) -> Result<Option<T>, ScriptError> {
        match self.values.get(i) {
            None | Some(ScriptValue::Nil) => Ok(None),
            Some(value) => pick(value)
                .map(Some)
                .ok_or_else(|| ScriptError::bad_argument(i + 1, expected, value.type_name())),
        }
    }

    /// Turns an optional read into a required one: `None` becomes "got nil" or "got no value".
    fn required<T>(&self, i: usize, expected: &str, read: Result<Option<T>, ScriptError>) -> Result<T, ScriptError> {
        read?.ok_or_else(|| {
            let got = if i < self.values.len() { "nil" } else { "no value" };
            ScriptError::bad_argument(i + 1, expected, got)
        })
    }
}

impl From<Vec<ScriptValue>> for Args {
    fn from(values: Vec<ScriptValue>) -> Self {
        Self::new(values)
    }
}

impl FromIterator<ScriptValue> for Args {
    fn from_iter<I: IntoIterator<Item = ScriptValue>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> Args {
        Args::new(vec![
            ScriptValue::Number(2.0),
            ScriptValue::from("Part"),
            ScriptValue::Nil,
            ScriptValue::Bool(true),
            ScriptValue::Number(2.5),
            ScriptValue::Instance(Id::new(3, 1)),
            ScriptValue::Enum(EnumItem::new("Material", "Plastic")),
            ScriptValue::Vector3(Vector3::ONE),
            ScriptValue::CFrame(CFrame::IDENTITY),
            ScriptValue::Color3(Color3::WHITE),
            ScriptValue::Vector2(Vector2::ONE),
        ])
    }

    fn message<T: std::fmt::Debug>(result: Result<T, ScriptError>) -> String {
        result.expect_err("should fail").to_string()
    }

    #[test]
    fn accessors_read_matching_values() {
        let a = args();
        assert_eq!(a.number(0), Ok(2.0));
        assert_eq!(a.int(0), Ok(2));
        assert_eq!(a.string(1).as_deref(), Ok("Part"));
        assert_eq!(a.bool(3), Ok(true));
        assert_eq!(a.instance(5), Ok(Id::new(3, 1)));
        assert_eq!(a.enum_item(6), Ok(EnumItem::new("Material", "Plastic")));
        assert_eq!(a.vector3(7), Ok(Vector3::ONE));
        assert_eq!(a.cframe(8), Ok(CFrame::IDENTITY));
        assert_eq!(a.color3(9), Ok(Color3::WHITE));
        assert_eq!(a.vector2(10), Ok(Vector2::ONE));
        assert_eq!(a.len(), 11);
        assert!(!a.is_empty());
    }

    #[test]
    fn optional_accessors_treat_nil_and_missing_as_none() {
        let a = args();
        assert_eq!(a.opt_bool(2), Ok(None));
        assert_eq!(a.opt_number(99), Ok(None));
        assert_eq!(a.opt_int(2), Ok(None));
        assert_eq!(a.opt_instance(2), Ok(None));
        assert_eq!(a.opt_bool(3), Ok(Some(true)));
        assert_eq!(
            a.opt_string(1).map(|s| s.map(|s| s.to_string())),
            Ok(Some("Part".to_owned()))
        );
    }

    #[test]
    fn wrong_type_messages_use_one_based_positions() {
        let a = args();
        assert_eq!(message(a.number(1)), "bad argument #2 (expected number, got string)");
        assert_eq!(message(a.string(0)), "bad argument #1 (expected string, got number)");
        assert_eq!(
            message(a.instance(1)),
            "bad argument #2 (expected Instance, got string)"
        );
        assert_eq!(
            message(a.enum_item(1)),
            "bad argument #2 (expected EnumItem, got string)"
        );
        assert_eq!(message(a.vector3(8)), "bad argument #9 (expected Vector3, got CFrame)");
        assert_eq!(
            message(a.opt_color3(0)),
            "bad argument #1 (expected Color3, got number)"
        );
    }

    #[test]
    fn nil_and_missing_required_arguments() {
        let a = args();
        assert_eq!(message(a.bool(2)), "bad argument #3 (expected boolean, got nil)");
        assert_eq!(
            message(a.cframe(11)),
            "bad argument #12 (expected CFrame, got no value)"
        );
        assert_eq!(
            message(Args::default().number(0)),
            "bad argument #1 (expected number, got no value)"
        );
    }

    #[test]
    fn int_rejects_fractions() {
        let a = args();
        assert_eq!(message(a.int(4)), "bad argument #5 (expected integer, got 2.5)");
        assert_eq!(message(a.opt_int(1)), "bad argument #2 (expected integer, got string)");
    }

    #[test]
    fn builds_from_vec_and_iterator() {
        let from_vec = Args::from(vec![ScriptValue::Nil, ScriptValue::Bool(false)]);
        let collected: Args = [ScriptValue::Nil, ScriptValue::Bool(false)].into_iter().collect();
        assert_eq!(from_vec, collected);
        assert_eq!(collected.get(1), Some(&ScriptValue::Bool(false)));
        assert_eq!(collected.as_slice().len(), 2);
        assert_eq!(collected.into_vec().len(), 2);
    }
}
