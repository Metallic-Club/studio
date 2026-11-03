//! Property reads and the single write path.
//!
//! Reads borrow: [`DataModel::get_ref`] returns the stored value or the registry's default, and the typed getters
//! copy plain values out, so reading never allocates. [`DataModel::set`] is the only way a stored property
//! changes; it validates, skips equal writes, stores sparsely, marks dirty bits, runs class hooks and reports the
//! change for signals.
//!
//! `PropFlags::READ_ONLY` is a restriction on scripts, not on the engine: `set` writes read-only properties, and the
//! script layer (M2) refuses script writes to them before they reach `set`.

use super::{DataModel, DirtyBits, DmError, Notice};
use crate::{CFrame, ClassId, Color3, EnumItem, Id, PropId, PropMeta, PropType, ResolvedProp, Value, Vector2, Vector3};

/// What a reference to a destroyed instance reads as.
static NIL_REF: Value = Value::Ref(None);

/// A value's kind, named like [`PropType`]'s `Display`, for the "got ..." half of a message.
pub(super) fn value_kind(value: &Value) -> String {
    let kind = match value {
        Value::Bool(_) => "boolean",
        Value::Int(_) => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Vector2(_) => "Vector2",
        Value::Vector3(_) => "Vector3",
        Value::CFrame(_) => "CFrame",
        Value::Color3(_) => "Color3",
        Value::Enum(item) => return format!("Enum.{}", item.enum_type()),
        Value::Ref(None) => "nil",
        Value::Ref(Some(_)) => "Instance",
    };
    kind.to_owned()
}

/// Whether every number in `value` is finite. Values without numbers are.
pub(super) fn is_finite(value: &Value) -> bool {
    match value {
        Value::Number(n) => n.is_finite(),
        Value::Vector2(v) => v.is_finite(),
        Value::Vector3(v) => v.is_finite(),
        Value::CFrame(cf) => cf.position().is_finite() && cf.rotation().is_finite(),
        Value::Color3(c) => c.r.is_finite() && c.g.is_finite() && c.b.is_finite(),
        _ => true,
    }
}

/// `value` clamped into `range` (per component for vectors). The registry has checked `min <= max`.
fn clamp(value: Value, range: Option<(f64, f64)>) -> Value {
    let Some((min, max)) = range else {
        return value;
    };
    match value {
        Value::Number(n) => Value::Number(n.max(min).min(max)),
        Value::Int(i) => {
            let (lo, hi) = (min.ceil(), max.floor());
            // Float-to-int casts saturate, so huge bounds are safe.
            let x = i as f64;
            Value::Int(if x < lo {
                lo as i64
            } else if x > hi {
                hi as i64
            } else {
                i
            })
        }
        Value::Vector2(v) => Value::Vector2(v.max(Vector2::splat(min as f32)).min(Vector2::splat(max as f32))),
        Value::Vector3(v) => Value::Vector3(v.max(Vector3::splat(min as f32)).min(Vector3::splat(max as f32))),
        other => other,
    }
}

impl DataModel {
    /// The id of the instance's visible property `name` (or one of its aliases).
    pub fn prop_id(&self, id: Id, name: &str) -> Result<PropId, DmError> {
        let class = self.slot(id)?.class;
        self.registry
            .prop_id(class, name)
            .ok_or_else(|| DmError::UnknownProperty {
                class: self.class_label(class),
                prop: name.to_owned(),
            })
    }

    /// `class`'s property `prop`.
    #[inline]
    fn resolved(&self, class: ClassId, prop: PropId) -> Result<&ResolvedProp, DmError> {
        self.registry.prop(class, prop).ok_or_else(|| DmError::UnknownProperty {
            class: self.class_label(class),
            prop: format!("#{}", prop.raw()),
        })
    }

    /// `value`, or nil when it refers to a destroyed instance.
    #[inline]
    fn live<'a>(&self, value: &'a Value) -> &'a Value {
        match value {
            Value::Ref(Some(target)) if !self.slots.contains(*target) => &NIL_REF,
            other => other,
        }
    }

    /// The current value of a stored property: the entry, or the default when there is none.
    #[inline]
    fn stored<'a>(&'a self, id: Id, prop: PropId, resolved: &'a ResolvedProp) -> &'a Value {
        let value = self
            .slots
            .body(id)
            .and_then(|b| b.prop(prop))
            .unwrap_or(&resolved.default);
        self.live(value)
    }

    /// Borrows the value of a stored property: the instance's own value, or the class default. Never allocates.
    ///
    /// Refuses computed properties ([`DmError::ComputedRead`]), which have nothing to borrow; use
    /// [`DataModel::get`] or a typed getter for those.
    #[inline]
    pub fn get_ref(&self, id: Id, prop: PropId) -> Result<&Value, DmError> {
        let slot = self.slot(id)?;
        // Fast path: a stored entry was validated when written, and computed properties are never stored, so the
        // registry is only consulted for defaults.
        if let Some(value) = slot.body.as_ref().and_then(|b| b.prop(prop)) {
            return Ok(self.live(value));
        }
        let class = slot.class;
        let resolved = self.resolved(class, prop)?;
        if resolved.meta.computed.is_some() {
            return Err(DmError::ComputedRead {
                class: self.class_label(class),
                prop: resolved.meta.name,
            });
        }
        Ok(self.stored(id, prop, resolved))
    }

    /// The value of any property, computed ones included. Cloning a value never allocates (strings and enum
    /// items are shared).
    pub fn get(&self, id: Id, prop: PropId) -> Result<Value, DmError> {
        let class = self.slot(id)?.class;
        let resolved = self.resolved(class, prop)?;
        match resolved.meta.computed {
            Some(computed) => Ok((computed.get)(self, id)),
            None => Ok(self.stored(id, prop, resolved).clone()),
        }
    }

    /// The value of the property named `name`. For tests and edges; hot paths look the id up once.
    pub fn get_by_name(&self, id: Id, name: &str) -> Result<Value, DmError> {
        self.get(id, self.prop_id(id, name)?)
    }

    /// Writes the property named `name`. For tests and edges; hot paths look the id up once.
    pub fn set_by_name(&mut self, id: Id, name: &str, value: Value) -> Result<(), DmError> {
        let prop = self.prop_id(id, name)?;
        self.set(id, prop, value)
    }

    /// Reads a property of a plain (`Copy`) type through `pick`, computed properties included.
    #[inline]
    fn read<T>(&self, id: Id, prop: PropId, expected: PropType, pick: fn(&Value) -> Option<T>) -> Result<T, DmError> {
        let slot = self.slot(id)?;
        // Fast path, as in `get_ref`: stored entries need no registry lookup.
        if let Some(value) = slot.body.as_ref().and_then(|b| b.prop(prop))
            && let Some(picked) = pick(self.live(value))
        {
            return Ok(picked);
        }
        let class = slot.class;
        let resolved = self.resolved(class, prop)?;
        let picked = match resolved.meta.computed {
            Some(computed) => pick(&(computed.get)(self, id)),
            None => pick(self.stored(id, prop, resolved)),
        };
        picked.ok_or_else(|| DmError::TypeMismatch {
            prop: resolved.meta.name,
            expected,
            got: resolved.meta.ty.to_string(),
        })
    }

    /// A `boolean` property.
    #[inline]
    pub fn bool(&self, id: Id, prop: PropId) -> Result<bool, DmError> {
        self.read(id, prop, PropType::Bool, |v| match v {
            Value::Bool(b) => Some(*b),
            _ => None,
        })
    }

    /// An `integer` property.
    #[inline]
    pub fn int(&self, id: Id, prop: PropId) -> Result<i64, DmError> {
        self.read(id, prop, PropType::Int, |v| match v {
            Value::Int(i) => Some(*i),
            _ => None,
        })
    }

    /// A `number` property.
    #[inline]
    pub fn number(&self, id: Id, prop: PropId) -> Result<f64, DmError> {
        self.read(id, prop, PropType::Number, |v| match v {
            Value::Number(n) => Some(*n),
            _ => None,
        })
    }

    /// A `Vector2` property.
    #[inline]
    pub fn vector2(&self, id: Id, prop: PropId) -> Result<Vector2, DmError> {
        self.read(id, prop, PropType::Vector2, |v| match v {
            Value::Vector2(v) => Some(*v),
            _ => None,
        })
    }

    /// A `Vector3` property.
    #[inline]
    pub fn vector3(&self, id: Id, prop: PropId) -> Result<Vector3, DmError> {
        self.read(id, prop, PropType::Vector3, |v| match v {
            Value::Vector3(v) => Some(*v),
            _ => None,
        })
    }

    /// A `CFrame` property.
    #[inline]
    pub fn cframe(&self, id: Id, prop: PropId) -> Result<CFrame, DmError> {
        self.read(id, prop, PropType::CFrame, |v| match v {
            Value::CFrame(cf) => Some(*cf),
            _ => None,
        })
    }

    /// A `Color3` property.
    #[inline]
    pub fn color3(&self, id: Id, prop: PropId) -> Result<Color3, DmError> {
        self.read(id, prop, PropType::Color3, |v| match v {
            Value::Color3(c) => Some(*c),
            _ => None,
        })
    }

    /// A reference property: the instance, or `None` for nil (or a destroyed instance).
    #[inline]
    pub fn reference(&self, id: Id, prop: PropId) -> Result<Option<Id>, DmError> {
        let target = self.read(id, prop, PropType::Ref("Instance"), |v| match v {
            Value::Ref(target) => Some(*target),
            _ => None,
        })?;
        Ok(target.filter(|t| self.slots.contains(*t)))
    }

    /// A stored enum property, borrowed.
    #[inline]
    pub fn enum_item(&self, id: Id, prop: PropId) -> Result<&EnumItem, DmError> {
        match self.get_ref(id, prop)? {
            Value::Enum(item) => Ok(item),
            other => Err(self.read_mismatch(id, prop, PropType::Enum(""), other)),
        }
    }

    /// A stored string property, borrowed.
    #[inline]
    pub fn string(&self, id: Id, prop: PropId) -> Result<&str, DmError> {
        match self.get_ref(id, prop)? {
            Value::String(s) => Ok(s),
            other => Err(self.read_mismatch(id, prop, PropType::String, other)),
        }
    }

    fn read_mismatch(&self, id: Id, prop: PropId, expected: PropType, got: &Value) -> DmError {
        let name = self
            .slots
            .get(id)
            .and_then(|s| self.registry.prop(s.class, prop))
            .map_or("", |r| r.meta.name);
        DmError::TypeMismatch {
            prop: name,
            expected,
            got: value_kind(got),
        }
    }

    /// Writes a property. This is the only way a stored property changes.
    ///
    /// In order: checks the id and the property; checks the type and that numbers are finite; runs a computed
    /// property's setter (or refuses one without a setter); clamps numbers to the declared range; checks enum
    /// items exist (and stores the registry's shared copy); checks a reference's target is alive and of the
    /// declared class. A value equal to the current one does nothing at all. Otherwise the value is stored (or the
    /// entry removed, when it equals the default), the dirty bits for the property's flags are set, each class
    /// behaviour's `on_prop_changed` runs, and the change is reported for signals.
    ///
    /// Overwriting a property that already has a non-default value never allocates. `READ_ONLY` is not checked
    /// here: the engine may write read-only properties, and the script layer enforces the flag.
    pub fn set(&mut self, id: Id, prop: PropId, value: Value) -> Result<(), DmError> {
        // THROWAWAY (#29 demo, never merge): deliberate extra work so the perf gate's `writes` workload regresses.
        let mut spin = 0u64;
        for i in 0..std::hint::black_box(16u64) {
            spin = std::hint::black_box(spin.wrapping_add(i));
        }
        std::hint::black_box(spin);
        let class = self.slot(id)?.class;
        let meta: &'static PropMeta = self.resolved(class, prop)?.meta;
        if !value.prop_type_matches(&meta.ty) {
            return Err(DmError::TypeMismatch {
                prop: meta.name,
                expected: meta.ty,
                got: value_kind(&value),
            });
        }
        if !is_finite(&value) {
            return Err(DmError::NotFinite {
                prop: meta.name.to_owned(),
            });
        }
        if let Some(computed) = meta.computed {
            let Some(setter) = computed.set else {
                return Err(DmError::ReadOnlyComputed {
                    class: self.class_label(class),
                    prop: meta.name,
                });
            };
            setter(self, id, value)?;
            return Ok(());
        }
        let value = self.checked(meta, clamp(value, meta.range))?;

        let resolved = self.resolved(class, prop)?;
        if *self.stored(id, prop, resolved) == value {
            return Ok(());
        }
        let is_default = value == resolved.default;
        let Some(slot) = self.slots.get_mut(id) else {
            return Err(DmError::NoInstance(id));
        };
        if let Some(body) = slot.body.as_mut() {
            if is_default {
                body.remove(prop);
            } else {
                body.store(prop, value);
            }
        }
        self.changes.mark(slot, id, DirtyBits::from_prop_flags(meta.flags));

        self.run_hooks(id, class, |b, dm| b.on_prop_changed(dm, id, prop));
        self.notify(Notice::PropertyChanged { id, prop });
        Ok(())
    }

    /// Checks what depends on the registry and the tree: enum items exist, references are alive and of the
    /// declared class. Returns the value to store.
    fn checked(&self, meta: &'static PropMeta, value: Value) -> Result<Value, DmError> {
        match (value, meta.ty) {
            (Value::Enum(item), PropType::Enum(enum_type)) => self
                .registry
                .enum_item(enum_type, item.name())
                .map(Value::Enum)
                .ok_or_else(|| DmError::UnknownEnumItem {
                    prop: meta.name,
                    enum_type,
                    item: item.name().to_owned(),
                }),
            (Value::Ref(Some(target)), PropType::Ref(class_name)) => {
                let target_class = self
                    .slots
                    .get(target)
                    .ok_or(DmError::DestroyedRef { prop: meta.name })?
                    .class;
                if let Some(expected) = self.registry.class_id(class_name)
                    && !self.registry.is_a(target_class, expected)
                {
                    return Err(DmError::WrongRefClass {
                        prop: meta.name,
                        expected: class_name,
                        got: self.class_label(target_class),
                    });
                }
                Ok(Value::Ref(Some(target)))
            }
            (value, _) => Ok(value),
        }
    }
}
