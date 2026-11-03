//! `Ty` and `Param`: the types of method parameters, method returns and signal parameters.
//!
//! A type is written once, in a class file, and read by everything that shows or checks a signature: the API dump,
//! the generated Luau definitions, editor autocomplete, docs and other language bindings. It is built in `const`s
//! like the rest of a descriptor, so nested types are `&'static` references: `Ty::Optional(&Ty::Class("Instance"))`.
//! Class and enum names are plain strings, checked when the registry is built.
//!
//! [`Ty`]'s `Display` writes Luau type syntax (`Instance?`, `{Instance}`, `{[string]: any}`, `...any`, `()`), so a
//! signature reads the same in docs, error messages and the generated definitions. Other languages map the variants
//! themselves.

use std::fmt;

/// The type of a value a method takes or returns, or a signal passes.
///
/// Where each variant may appear (the registry rejects the rest):
///
/// - [`Ty::Variadic`] only as the whole type of a method's or signal's last parameter;
/// - [`Ty::Tuple`] only as the whole return type of a method;
/// - every other variant anywhere, including inside [`Ty::Optional`], [`Ty::Array`] and [`Ty::Map`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Ty {
    /// A boolean (`boolean`).
    Bool,
    /// A whole number. Luau has one number type, so it is written `number`; the variant keeps the distinction for
    /// languages that have integers.
    Int,
    /// A number (`number`).
    Number,
    /// A string (`string`).
    String,
    /// A `Vector2`.
    Vector2,
    /// A `Vector3`.
    Vector3,
    /// A `CFrame`.
    CFrame,
    /// A `Color3`.
    Color3,
    /// An item of the named enum, such as `Enum.Material`.
    Enum(&'static str),
    /// An instance of the named class or one of its subclasses, such as `BasePart`. Never nil; use
    /// [`Ty::Optional`] for that.
    Class(&'static str),
    /// Any value, nil included (`any`).
    Any,
    /// Only nil (`nil`).
    Nil,
    /// The type or nil (`T?`).
    Optional(&'static Ty),
    /// A list of values of one type (`{T}`).
    Array(&'static Ty),
    /// A table from keys of the first type to values of the second (`{[K]: V}`).
    Map(&'static Ty, &'static Ty),
    /// Any number of values of the type, zero included (`...T`). Only as the last parameter.
    Variadic(&'static Ty),
    /// Several values at once (`(A, B)`). Only as a method's return; `Tuple(&[])` ([`Ty::NOTHING`]) returns nothing.
    Tuple(&'static [Ty]),
    /// A function, such as a callback (`(...any) -> ...any`).
    Function,
}

impl Ty {
    /// No values at all: the return type of a method that returns nothing, written `()`. Set it with
    /// [`MethodDesc::returns_nothing`](super::MethodDesc::returns_nothing).
    pub const NOTHING: Self = Self::Tuple(&[]);

    /// Whether nil is a valid value of this type: `any`, `nil` and `T?`.
    pub const fn accepts_nil(&self) -> bool {
        matches!(self, Self::Any | Self::Nil | Self::Optional(_))
    }
}

/// Writes the type in Luau syntax: `boolean`, `Enum.Material`, `BasePart?`, `{Instance}`, `{[string]: any}`,
/// `...any`, `(CFrame, boolean)`, `()`.
impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => f.write_str("boolean"),
            Self::Int | Self::Number => f.write_str("number"),
            Self::String => f.write_str("string"),
            Self::Vector2 => f.write_str("Vector2"),
            Self::Vector3 => f.write_str("Vector3"),
            Self::CFrame => f.write_str("CFrame"),
            Self::Color3 => f.write_str("Color3"),
            Self::Enum(name) => write!(f, "Enum.{name}"),
            Self::Class(name) => f.write_str(name),
            Self::Any => f.write_str("any"),
            Self::Nil => f.write_str("nil"),
            // `?` binds tighter than `->`, so a function type needs parentheses.
            Self::Optional(inner @ Self::Function) => write!(f, "({inner})?"),
            Self::Optional(inner) => write!(f, "{inner}?"),
            Self::Array(item) => write!(f, "{{{item}}}"),
            Self::Map(key, value) => write!(f, "{{[{key}]: {value}}}"),
            Self::Variadic(item) => write!(f, "...{item}"),
            Self::Tuple(items) => {
                f.write_str("(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str(")")
            }
            Self::Function => f.write_str("(...any) -> ...any"),
        }
    }
}

/// One parameter of a method or a signal.
///
/// ```
/// use metallic_core::{Param, Ty};
///
/// const RECURSIVE: Param = Param::new("recursive", Ty::Bool).optional();
/// assert_eq!(RECURSIVE.to_string(), "recursive: boolean?");
/// assert_eq!(Param::new("args", Ty::Variadic(&Ty::Any)).to_string(), "...: any");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Param {
    /// The parameter's name, for docs and autocomplete. A variadic parameter's name is not shown in Luau.
    pub name: &'static str,
    /// The type of the value.
    pub ty: Ty,
    /// The caller may leave it out (and pass nil). Optional parameters come after every required one.
    pub optional: bool,
}

impl Param {
    /// A required parameter `name` of type `ty`.
    pub const fn new(name: &'static str, ty: Ty) -> Self {
        Self {
            name,
            ty,
            optional: false,
        }
    }

    /// Lets the caller leave it out.
    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// Writes the parameter as in a Luau declaration: `name: string`, `recursive: boolean?` (optional), `...: any`
/// (variadic).
impl fmt::Display for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ty {
            Ty::Variadic(item) => write!(f, "...: {item}"),
            Ty::Function if self.optional => write!(f, "{}: ({})?", self.name, Ty::Function),
            ty if self.optional && !ty.accepts_nil() => write!(f, "{}: {ty}?", self.name),
            ty => write!(f, "{}: {ty}", self.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_render_as_luau() {
        let cases: &[(Ty, &str)] = &[
            (Ty::Bool, "boolean"),
            (Ty::Int, "number"),
            (Ty::Number, "number"),
            (Ty::String, "string"),
            (Ty::Vector2, "Vector2"),
            (Ty::Vector3, "Vector3"),
            (Ty::CFrame, "CFrame"),
            (Ty::Color3, "Color3"),
            (Ty::Enum("Material"), "Enum.Material"),
            (Ty::Class("BasePart"), "BasePart"),
            (Ty::Any, "any"),
            (Ty::Nil, "nil"),
            (Ty::Optional(&Ty::Class("BasePart")), "BasePart?"),
            (Ty::Array(&Ty::Class("Instance")), "{Instance}"),
            (Ty::Map(&Ty::String, &Ty::Any), "{[string]: any}"),
            (Ty::Variadic(&Ty::Any), "...any"),
            (Ty::NOTHING, "()"),
            (Ty::Tuple(&[Ty::CFrame, Ty::Bool]), "(CFrame, boolean)"),
            (Ty::Function, "(...any) -> ...any"),
            (Ty::Optional(&Ty::Function), "((...any) -> ...any)?"),
            (Ty::Array(&Ty::Optional(&Ty::Enum("Material"))), "{Enum.Material?}"),
        ];
        for (ty, expected) in cases {
            assert_eq!(ty.to_string(), *expected, "{ty:?}");
        }
    }

    #[test]
    fn params_render_as_luau() {
        assert_eq!(Param::new("name", Ty::String).to_string(), "name: string");
        assert_eq!(
            Param::new("recursive", Ty::Bool).optional().to_string(),
            "recursive: boolean?"
        );
        // Already nil-able: no second `?`.
        let parent = Param::new("parent", Ty::Optional(&Ty::Class("Instance"))).optional();
        assert_eq!(parent.to_string(), "parent: Instance?");
        assert_eq!(Param::new("value", Ty::Any).optional().to_string(), "value: any");
        assert_eq!(Param::new("args", Ty::Variadic(&Ty::Any)).to_string(), "...: any");
        assert_eq!(
            Param::new("callback", Ty::Function).optional().to_string(),
            "callback: ((...any) -> ...any)?"
        );
    }
}
