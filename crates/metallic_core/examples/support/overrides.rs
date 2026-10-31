//! Storage-component prototypes only; this module does not replace World validation or events.
use metallic_core::Value;
use std::collections::BTreeMap;

/// Representative tokens; the final token belongs to Player rather than Part.
pub const KEYS: [&str; 9] = [
    "Position",
    "Orientation",
    "Size",
    "Anchored",
    "CanCollide",
    "Material",
    "Shape",
    "Name",
    "RespawnLocation",
];

/// Trusted storage operations; class/type validation and events remain outside this prototype.
pub trait Overrides: Default + Clone {
    /// Stable report label.
    const LABEL: &'static str;
    /// Read an explicitly stored override.
    fn stored(&self, key: usize) -> Option<&Value>;
    /// Insert or replace a valid schema token.
    fn insert(&mut self, key: usize, value: Value);
    /// Remove a stored token, if present.
    fn remove(&mut self, key: usize);
    /// Number of explicit overrides.
    fn len(&self) -> usize;

    /// Borrow stored data or the shared default for a valid token.
    fn read<'a>(&'a self, key: usize, defaults: &'a [Value]) -> &'a Value {
        self.stored(key).unwrap_or(&defaults[key])
    }

    /// Resolve a known name; unknown names are absent rather than creating tokens.
    fn read_name<'a>(&'a self, name: &str, defaults: &'a [Value]) -> Option<&'a Value> {
        KEYS.iter()
            .position(|key| *key == name)
            .map(|key| self.read(key, defaults))
    }

    /// Store a non-default value, removing overrides that equal the default.
    fn set(&mut self, key: usize, value: Value, defaults: &[Value]) {
        if value == defaults[key] {
            self.remove(key);
        } else {
            self.insert(key, value);
        }
    }
}

#[derive(Default, Clone)]
/// Current production-style key/value container, excluding the owning Node.
pub struct NamedMap(BTreeMap<String, Value>);
impl Overrides for NamedMap {
    const LABEL: &'static str = "named_btree";
    fn stored(&self, key: usize) -> Option<&Value> {
        self.0.get(KEYS[key])
    }
    fn insert(&mut self, key: usize, value: Value) {
        self.0.insert(KEYS[key].to_owned(), value);
    }
    fn remove(&mut self, key: usize) {
        self.0.remove(KEYS[key]);
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

#[derive(Default, Clone)]
/// B-tree keyed by compact property IDs, retaining owned Value payloads.
pub struct KeyMap(BTreeMap<u16, Value>);
impl Overrides for KeyMap {
    const LABEL: &'static str = "id_btree";
    fn stored(&self, key: usize) -> Option<&Value> {
        self.0.get(&(key as u16))
    }
    fn insert(&mut self, key: usize, value: Value) {
        self.0.insert(key as u16, value);
    }
    fn remove(&mut self, key: usize) {
        self.0.remove(&(key as u16));
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

#[derive(Default, Clone)]
/// Sorted numeric-key vector, retaining owned Value payloads.
pub struct SortedValues(Vec<(u16, Value)>);
impl Overrides for SortedValues {
    const LABEL: &'static str = "id_sorted_vec";
    fn stored(&self, key: usize) -> Option<&Value> {
        self.0
            .binary_search_by_key(&(key as u16), |(key, _)| *key)
            .ok()
            .map(|index| &self.0[index].1)
    }
    fn insert(&mut self, key: usize, value: Value) {
        let key = key as u16;
        match self.0.binary_search_by_key(&key, |(key, _)| *key) {
            Ok(index) => self.0[index].1 = value,
            Err(index) => self.0.insert(index, (key, value)),
        }
    }
    fn remove(&mut self, key: usize) {
        if let Ok(index) = self.0.binary_search_by_key(&(key as u16), |(key, _)| *key) {
            self.0.remove(index);
        }
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::{Vec3, World};

    fn check<S: Overrides>() {
        let mut world = World::new_empty();
        let target = world.create("Part", None).unwrap();
        let defaults = vec![Value::Nil; KEYS.len()];
        let mut values = S::default();
        assert_eq!(values.read_name("unknown", &defaults), None);
        assert_eq!(values.read(0, &defaults), &Value::Nil);
        values.set(7, Value::String("owned name".into()), &defaults);
        values.set(0, Value::Vector3(Vec3::new(1.25, -0.5, 1e12)), &defaults);
        values.set(8, Value::Ref(Some(target)), &defaults);
        let mut cloned = values.clone();
        cloned.set(7, Value::String("changed clone".into()), &defaults);
        assert_eq!(
            values.read(7, &defaults),
            &Value::String("owned name".into())
        );
        assert_eq!(
            cloned.read(7, &defaults),
            &Value::String("changed clone".into())
        );
        assert_eq!(cloned.read(8, &defaults), &Value::Ref(Some(target)));
        assert_eq!(
            values.read(0, &defaults),
            &Value::Vector3(Vec3::new(1.25, -0.5, 1e12))
        );
        values.set(0, Value::Number(42.0), &defaults);
        assert_eq!(values.len(), 3, "updating must not duplicate a key");
        values.set(0, Value::Nil, &defaults);
        assert_eq!(values.stored(0), None);
        assert_eq!(values.read(0, &defaults), &Value::Nil);
        values.remove(0);
        assert_eq!(values.len(), 2, "removing an absent key is harmless");
    }

    #[test]
    fn named_map_preserves_component_semantics() {
        check::<NamedMap>();
    }
    #[test]
    fn id_map_preserves_component_semantics() {
        check::<KeyMap>();
    }
    #[test]
    fn sorted_vector_preserves_component_semantics() {
        check::<SortedValues>();
    }
}
