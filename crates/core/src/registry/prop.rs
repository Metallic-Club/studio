//! `PropMeta` and what it is made of: everything the engine and Studio know about one property.

use std::fmt;
use std::ops::BitOr;

use crate::{DataModel, Id, PropType, ScriptError, Value};

/// A property's default value, written in a `const` class descriptor.
///
/// [`Value`] cannot be built in a `const` (strings and enum items are reference-counted), so descriptors use this
/// instead. The registry turns each default into a `Value` once, when it is built, and checks that it fits the
/// property's [`PropType`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DefaultValue {
    /// For a [`PropType::Bool`] property.
    Bool(bool),
    /// For a [`PropType::Int`] property.
    Int(i64),
    /// For a [`PropType::Number`] property.
    Number(f64),
    /// For a [`PropType::String`] property.
    Str(&'static str),
    /// For a [`PropType::Vector2`] property: `x`, `y`.
    Vector2(f32, f32),
    /// For a [`PropType::Vector3`] property: `x`, `y`, `z`.
    Vector3(f32, f32, f32),
    /// For a [`PropType::CFrame`] property: a position with no rotation. `CFrame(0.0, 0.0, 0.0)` is the identity.
    CFrame(f32, f32, f32),
    /// For a [`PropType::Color3`] property: `r`, `g`, `b` from 0 to 1.
    Color3(f32, f32, f32),
    /// For a [`PropType::Enum`] property: the item's name. The enum type comes from the property's type.
    Enum(&'static str),
    /// For a [`PropType::Ref`] property: no instance (nil). A reference cannot default to anything else.
    NilRef,
}

impl DefaultValue {
    /// The kind of value this is, named like [`PropType`]'s `Display` so that a mismatch reads well.
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "boolean",
            Self::Int(_) => "integer",
            Self::Number(_) => "number",
            Self::Str(_) => "string",
            Self::Vector2(..) => "Vector2",
            Self::Vector3(..) => "Vector3",
            Self::CFrame(..) => "CFrame",
            Self::Color3(..) => "Color3",
            Self::Enum(_) => "enum item",
            Self::NilRef => "nil reference",
        }
    }
}

/// What a property is, as a set of bits. Combine with `|` at runtime or [`PropFlags::union`] in a `const`.
///
/// The change flags (`TRANSFORM`, `VISUAL`, `PHYSICS`) decide which dirty bits a write sets, so the renderer and
/// physics only look at what changed. The others decide who may read, write and save the property.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PropFlags(u16);

impl PropFlags {
    /// No flags.
    pub const NONE: Self = Self(0);
    /// Writing it moves or resizes the instance.
    pub const TRANSFORM: Self = Self(1 << 0);
    /// Writing it changes how the instance looks.
    pub const VISUAL: Self = Self(1 << 1);
    /// Writing it may change collision or simulation.
    pub const PHYSICS: Self = Self(1 << 2);
    /// Play-mode state: never saved, and reset when Play stops.
    pub const RUNTIME: Self = Self(1 << 3);
    /// Not saved to scene files (derived from other properties, or saved by another one).
    pub const NO_SAVE: Self = Self(1 << 4);
    /// Scripts can read it but not write it.
    pub const READ_ONLY: Self = Self(1 << 5);
    /// Not shown in Studio's Properties panel. Scripts still see it.
    pub const HIDDEN: Self = Self(1 << 6);
    /// Re-declares an inherited property on purpose, replacing its metadata but keeping its [`PropId`] and type.
    ///
    /// [`PropId`]: crate::PropId
    pub const OVERRIDE: Self = Self(1 << 7);

    /// Every flag name with its bit, for `Debug`.
    const NAMES: [(Self, &'static str); 8] = [
        (Self::TRANSFORM, "TRANSFORM"),
        (Self::VISUAL, "VISUAL"),
        (Self::PHYSICS, "PHYSICS"),
        (Self::RUNTIME, "RUNTIME"),
        (Self::NO_SAVE, "NO_SAVE"),
        (Self::READ_ONLY, "READ_ONLY"),
        (Self::HIDDEN, "HIDDEN"),
        (Self::OVERRIDE, "OVERRIDE"),
    ];

    /// Both sets of flags.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any flag in `other` is set.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// The raw bits.
    pub const fn bits(self) -> u16 {
        self.0
    }
}

impl BitOr for PropFlags {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.union(other)
    }
}

/// Writes the set flags by name, such as `PropFlags(VISUAL | READ_ONLY)`.
impl fmt::Debug for PropFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PropFlags(")?;
        let mut first = true;
        for (flag, name) in Self::NAMES {
            if self.contains(flag) {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        if first {
            f.write_str("NONE")?;
        }
        f.write_str(")")
    }
}

/// Which control Studio's Properties panel uses for a property.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Widget {
    /// The usual control for the property's type.
    #[default]
    Default,
    /// A slider over the property's range (see [`PropMeta::range`]).
    Slider,
    /// A number field changed by dragging, `speed` units per pixel.
    Drag {
        /// Units per pixel dragged.
        speed: f32,
    },
    /// A text box with several lines.
    Multiline,
}

/// Reads a computed property: one whose value is derived from other state instead of being stored.
pub type Getter = fn(&DataModel, Id) -> Value;

/// Writes a computed property, usually by writing the properties it is derived from.
pub type Setter = fn(&mut DataModel, Id, Value) -> Result<(), ScriptError>;

/// The functions behind a computed property (such as `Position`, derived from `CFrame`).
#[derive(Clone, Copy, Debug)]
pub struct Computed {
    /// Reads the value.
    pub get: Getter,
    /// Writes the value, or `None` when the property can only be read.
    pub set: Option<Setter>,
}

/// Everything the engine and Studio know about one property: its type, default, flags and inspector hints.
///
/// Built in a `const` with a typed constructor and chained setters:
///
/// ```
/// use metallic_core::PropMeta;
///
/// static ANGLE: PropMeta = PropMeta::number("Angle", 90.0)
///     .group("Spotlight")
///     .range(1.0, 179.0)
///     .visual()
///     .desc("Full angle of the cone, in degrees.");
/// ```
#[derive(Clone, Copy, Debug)]
pub struct PropMeta {
    /// The name scripts and scene files use, such as `Brightness`.
    pub name: &'static str,
    /// The declared type. Every write is checked against it.
    pub ty: PropType,
    /// The value a new instance starts with.
    pub default: DefaultValue,
    /// Change, save and access flags.
    pub flags: PropFlags,
    /// The name Studio shows, or empty to show [`PropMeta::name`].
    pub label: &'static str,
    /// The Properties panel group, such as `Appearance`, or empty for the default group.
    pub group: &'static str,
    /// Sort key inside the group; lower comes first, ties keep declaration order.
    pub order: i16,
    /// The Properties panel control.
    pub widget: Widget,
    /// What the property does, for tooltips and the API docs.
    pub description: &'static str,
    /// `(min, max)`: writes are clamped into this range (per component for vectors).
    pub range: Option<(f64, f64)>,
    /// Older names that still work in scripts and scene files.
    pub aliases: &'static [&'static str],
    /// Set for a computed property, whose value comes from functions instead of storage.
    pub computed: Option<Computed>,
    /// For a computed property: the stored properties its value is derived from (see [`PropMeta::derived_from`]).
    pub derived_from: &'static [&'static str],
}

impl PropMeta {
    /// A property of type `ty` starting at `default`, with no flags or hints. The typed constructors below are
    /// usually clearer.
    pub const fn new(name: &'static str, ty: PropType, default: DefaultValue) -> Self {
        Self {
            name,
            ty,
            default,
            flags: PropFlags::NONE,
            label: "",
            group: "",
            order: 0,
            widget: Widget::Default,
            description: "",
            range: None,
            aliases: &[],
            computed: None,
            derived_from: &[],
        }
    }

    /// A `true` / `false` property.
    pub const fn boolean(name: &'static str, default: bool) -> Self {
        Self::new(name, PropType::Bool, DefaultValue::Bool(default))
    }

    /// A whole-number property.
    pub const fn int(name: &'static str, default: i64) -> Self {
        Self::new(name, PropType::Int, DefaultValue::Int(default))
    }

    /// A real-number property.
    pub const fn number(name: &'static str, default: f64) -> Self {
        Self::new(name, PropType::Number, DefaultValue::Number(default))
    }

    /// A text property.
    pub const fn string(name: &'static str, default: &'static str) -> Self {
        Self::new(name, PropType::String, DefaultValue::Str(default))
    }

    /// A `Vector2` property.
    pub const fn vector2(name: &'static str, x: f32, y: f32) -> Self {
        Self::new(name, PropType::Vector2, DefaultValue::Vector2(x, y))
    }

    /// A `Vector3` property.
    pub const fn vector3(name: &'static str, x: f32, y: f32, z: f32) -> Self {
        Self::new(name, PropType::Vector3, DefaultValue::Vector3(x, y, z))
    }

    /// A `CFrame` property starting at the identity (the origin, no rotation).
    pub const fn cframe(name: &'static str) -> Self {
        Self::new(name, PropType::CFrame, DefaultValue::CFrame(0.0, 0.0, 0.0))
    }

    /// A `Color3` property; components go from 0 to 1.
    pub const fn color3(name: &'static str, r: f32, g: f32, b: f32) -> Self {
        Self::new(name, PropType::Color3, DefaultValue::Color3(r, g, b))
    }

    /// A property holding an item of the enum `enum_type`, starting at the item named `item`.
    pub const fn enumeration(name: &'static str, enum_type: &'static str, item: &'static str) -> Self {
        Self::new(name, PropType::Enum(enum_type), DefaultValue::Enum(item))
    }

    /// A reference to an instance of `class` (or a subclass), starting at nil.
    pub const fn reference(name: &'static str, class: &'static str) -> Self {
        Self::new(name, PropType::Ref(class), DefaultValue::NilRef)
    }

    /// Sets the name Studio shows.
    pub const fn label(mut self, label: &'static str) -> Self {
        self.label = label;
        self
    }

    /// Sets the Properties panel group.
    pub const fn group(mut self, group: &'static str) -> Self {
        self.group = group;
        self
    }

    /// Sets the sort key inside the group.
    pub const fn order(mut self, order: i16) -> Self {
        self.order = order;
        self
    }

    /// Sets the Properties panel control.
    pub const fn widget(mut self, widget: Widget) -> Self {
        self.widget = widget;
        self
    }

    /// Sets the description shown in tooltips and the API docs.
    pub const fn desc(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Clamps writes into `min..=max`. The registry rejects `min > max` and a default outside the range.
    pub const fn range(mut self, min: f64, max: f64) -> Self {
        self.range = Some((min, max));
        self
    }

    /// Adds `flags` to the ones already set.
    pub const fn flags(mut self, flags: PropFlags) -> Self {
        self.flags = self.flags.union(flags);
        self
    }

    /// Adds [`PropFlags::TRANSFORM`].
    pub const fn transform(self) -> Self {
        self.flags(PropFlags::TRANSFORM)
    }

    /// Adds [`PropFlags::VISUAL`].
    pub const fn visual(self) -> Self {
        self.flags(PropFlags::VISUAL)
    }

    /// Adds [`PropFlags::PHYSICS`].
    pub const fn physics(self) -> Self {
        self.flags(PropFlags::PHYSICS)
    }

    /// Adds [`PropFlags::READ_ONLY`].
    pub const fn read_only(self) -> Self {
        self.flags(PropFlags::READ_ONLY)
    }

    /// Adds [`PropFlags::OVERRIDE`].
    pub const fn override_inherited(self) -> Self {
        self.flags(PropFlags::OVERRIDE)
    }

    /// Sets the older names that still work.
    pub const fn aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    /// Makes this a computed property: reads call `get`, writes call `set` (or fail when it is `None`).
    pub const fn computed(mut self, get: Getter, set: Option<Setter>) -> Self {
        self.computed = Some(Computed { get, set });
        self
    }

    /// Names the stored properties a computed property's value is derived from, such as `&["CFrame"]` for
    /// `Position`. When one of them changes on an instance something listens to, this property's `Changed` and
    /// property-changed signals fire too, right after the stored property's own, so
    /// `GetPropertyChangedSignal("Position")` fires when the `CFrame` moves.
    ///
    /// The registry checks each name: only a computed property may derive from others, and each source must be a
    /// stored (not computed) property of the same class or one it inherits.
    pub const fn derived_from(mut self, sources: &'static [&'static str]) -> Self {
        self.derived_from = sources;
        self
    }

    /// The name Studio shows: the label, or the name when no label is set.
    pub const fn display_name(&self) -> &'static str {
        if self.label.is_empty() { self.name } else { self.label }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_constructors_pair_type_and_default() {
        let cases = [
            (PropMeta::boolean("A", true), PropType::Bool, DefaultValue::Bool(true)),
            (PropMeta::int("B", 3), PropType::Int, DefaultValue::Int(3)),
            (PropMeta::number("C", 1.5), PropType::Number, DefaultValue::Number(1.5)),
            (PropMeta::string("D", "x"), PropType::String, DefaultValue::Str("x")),
            (
                PropMeta::vector2("E", 1.0, 2.0),
                PropType::Vector2,
                DefaultValue::Vector2(1.0, 2.0),
            ),
            (
                PropMeta::vector3("F", 1.0, 2.0, 3.0),
                PropType::Vector3,
                DefaultValue::Vector3(1.0, 2.0, 3.0),
            ),
            (
                PropMeta::cframe("G"),
                PropType::CFrame,
                DefaultValue::CFrame(0.0, 0.0, 0.0),
            ),
            (
                PropMeta::color3("H", 1.0, 0.5, 0.0),
                PropType::Color3,
                DefaultValue::Color3(1.0, 0.5, 0.0),
            ),
            (
                PropMeta::enumeration("I", "PartShape", "Block"),
                PropType::Enum("PartShape"),
                DefaultValue::Enum("Block"),
            ),
            (
                PropMeta::reference("J", "BasePart"),
                PropType::Ref("BasePart"),
                DefaultValue::NilRef,
            ),
        ];
        for (meta, ty, default) in cases {
            assert_eq!(meta.ty, ty, "{}", meta.name);
            assert_eq!(meta.default, default, "{}", meta.name);
        }
    }

    #[test]
    fn setters_chain_in_const() {
        const META: PropMeta = PropMeta::number("Angle", 90.0)
            .label("Cone angle")
            .group("Spotlight")
            .order(2)
            .widget(Widget::Drag { speed: 0.5 })
            .desc("Full angle of the cone.")
            .range(1.0, 179.0)
            .visual()
            .read_only()
            .aliases(&["angle"]);
        assert_eq!(META.display_name(), "Cone angle");
        assert_eq!(META.group, "Spotlight");
        assert_eq!(META.order, 2);
        assert_eq!(META.widget, Widget::Drag { speed: 0.5 });
        assert_eq!(META.range, Some((1.0, 179.0)));
        assert!(META.flags.contains(PropFlags::VISUAL | PropFlags::READ_ONLY));
        assert!(!META.flags.intersects(PropFlags::PHYSICS));
        assert_eq!(META.aliases, &["angle"]);
        assert_eq!(PropMeta::boolean("Enabled", true).display_name(), "Enabled");
    }

    #[test]
    fn flags_accumulate_and_debug_by_name() {
        let meta = PropMeta::cframe("CFrame")
            .transform()
            .physics()
            .flags(PropFlags::NO_SAVE);
        assert_eq!(
            meta.flags,
            PropFlags::TRANSFORM.union(PropFlags::PHYSICS).union(PropFlags::NO_SAVE)
        );
        assert_eq!(format!("{:?}", meta.flags), "PropFlags(TRANSFORM | PHYSICS | NO_SAVE)");
        assert_eq!(format!("{:?}", PropFlags::NONE), "PropFlags(NONE)");
        assert_eq!(PropFlags::HIDDEN.bits(), 1 << 6);
        assert!(
            PropMeta::int("X", 0)
                .override_inherited()
                .flags
                .contains(PropFlags::OVERRIDE)
        );
    }

    #[test]
    fn computed_properties_keep_their_functions() {
        fn get(_: &DataModel, _: Id) -> Value {
            Value::Number(1.0)
        }
        let meta = PropMeta::number("Mass", 0.0).computed(get, None);
        let computed = meta.computed.expect("computed");
        assert!(computed.set.is_none());
        assert!(meta.derived_from.is_empty());
        const DERIVED: PropMeta = PropMeta::number("Mass", 0.0)
            .computed(get, None)
            .derived_from(&["Size"]);
        assert_eq!(DERIVED.derived_from, &["Size"]);
    }

    #[test]
    fn default_kind_names_match_prop_type_names() {
        assert_eq!(DefaultValue::Int(1).kind_name(), PropType::Int.to_string());
        assert_eq!(DefaultValue::Number(1.0).kind_name(), PropType::Number.to_string());
        assert_eq!(DefaultValue::Str("").kind_name(), PropType::String.to_string());
        assert_eq!(DefaultValue::NilRef.kind_name(), "nil reference");
    }
}
