//! The preview overlay: live values of a gesture that is not committed yet (spec 2.1, #213).
//!
//! A tool calls [`PreviewOverlay::set`] every frame of a drag. The overlay remembers the value
//! each (instance, property) had before the gesture and the value it shows now. On release,
//! [`PreviewOverlay::commit`] turns it into **one** [`Command::SetProperties`] for `History`; on
//! Esc, [`PreviewOverlay::cancel`] puts every original value back. `History` is not touched until
//! commit.
//!
//! The overlay writes through to the edit world, and [`PreviewOverlay::get`] reads it first.
//! Writing through means the renderer, picking and Properties (which all read the world through
//! its change journal) show the preview without each knowing about it. Nothing else reads the
//! edit world during a gesture: scripts and physics run on Play's copy, and Studio refuses edits
//! while playing. After `cancel` the world holds exactly the values it had before the gesture.
//!
//! Cost per frame is one hash lookup and one `World::set` per changed value; the property name is
//! a shared [`PropName`], so a 500-part drag allocates nothing per frame after its first frame.
use std::collections::HashMap;

use metallic_core::{CoreError, InstanceId, Value, World};

use crate::commands::{Command, PropName};

#[derive(Debug, Clone)]
struct Entry {
    id: InstanceId,
    prop: PropName,
    original: Value,
    value: Value,
}

/// Live values of the gesture in progress.
#[derive(Debug, Default)]
pub struct PreviewOverlay {
    entries: Vec<Entry>,
    index: HashMap<(InstanceId, PropName), usize>,
}

impl PreviewOverlay {
    /// An empty overlay.
    pub fn new() -> PreviewOverlay {
        PreviewOverlay::default()
    }

    /// True while a gesture holds preview values.
    pub fn is_active(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Number of (instance, property) pairs previewed.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is previewed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Shows `value` for `prop` of `id`. The first call for a pair remembers the world's value.
    /// On error (unknown property, wrong type) nothing changes.
    pub fn set(
        &mut self,
        world: &mut World,
        id: InstanceId,
        prop: &PropName,
        value: Value,
    ) -> Result<(), CoreError> {
        let key = (id, prop.clone());
        if let Some(&i) = self.index.get(&key) {
            let e = &mut self.entries[i];
            if e.value == value {
                return Ok(());
            }
            world.set(id, prop, value.clone())?;
            e.value = value;
            return Ok(());
        }
        let original = world.get(id, prop)?;
        if original == value {
            // Remember the pair anyway so `get` and `commit` see a stable set; costs nothing.
            self.index.insert(key, self.entries.len());
            self.entries.push(Entry {
                id,
                prop: prop.clone(),
                original,
                value,
            });
            return Ok(());
        }
        world.set(id, prop, value.clone())?;
        self.index.insert(key, self.entries.len());
        self.entries.push(Entry {
            id,
            prop: prop.clone(),
            original,
            value,
        });
        Ok(())
    }

    /// The previewed value of `prop` of `id`, else the world's.
    pub fn get(&self, world: &World, id: InstanceId, prop: &str) -> Option<Value> {
        let key = (id, PropName::from(prop));
        match self.index.get(&key) {
            Some(&i) => Some(self.entries[i].value.clone()),
            None => world.get(id, prop).ok(),
        }
    }

    /// The value `prop` of `id` had before the gesture, if the gesture touched it.
    pub fn original(&self, id: InstanceId, prop: &str) -> Option<&Value> {
        let key = (id, PropName::from(prop));
        self.index.get(&key).map(|&i| &self.entries[i].original)
    }

    /// Ends the gesture: the changed pairs become one `SetProperties` named `name` (the world
    /// already holds the new values; pass it to `History::record`). `None` when nothing changed.
    pub fn commit(&mut self, name: String) -> Option<Command> {
        self.index.clear();
        let changes: Vec<_> = self
            .entries
            .drain(..)
            .filter(|e| e.original != e.value)
            .map(|e| (e.id, e.prop, e.original, e.value))
            .collect();
        (!changes.is_empty()).then_some(Command::SetProperties { name, changes })
    }

    /// Ends the gesture without a step: every original value goes back into the world.
    pub fn cancel(&mut self, world: &mut World) {
        self.index.clear();
        for e in self.entries.drain(..).rev() {
            if e.original != e.value {
                let _ = world.set(e.id, &e.prop, e.original);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::Vec3;

    #[test]
    fn set_get_commit_and_cancel() {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let p = w.create("Part", Some(ws)).unwrap();
        let pos: PropName = "Position".into();
        let mut o = PreviewOverlay::new();
        o.set(&mut w, p, &pos, Value::Vector3(Vec3::new(1.0, 0.0, 0.0)))
            .unwrap();
        o.set(&mut w, p, &pos, Value::Vector3(Vec3::new(2.0, 0.0, 0.0)))
            .unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(
            o.get(&w, p, "Position"),
            Some(Value::Vector3(Vec3::new(2.0, 0.0, 0.0)))
        );
        assert!(o.set(&mut w, p, &"Nope".into(), Value::Bool(true)).is_err());
        o.cancel(&mut w);
        assert_eq!(
            w.get(p, "Position").unwrap(),
            Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
        );
        assert!(!o.is_active());
        o.set(&mut w, p, &pos, Value::Vector3(Vec3::new(3.0, 0.0, 0.0)))
            .unwrap();
        let Some(Command::SetProperties { changes, .. }) = o.commit("Move 1 part".into()) else {
            panic!()
        };
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].2, Value::Vector3(Vec3::new(0.0, 0.0, 0.0)));
    }
}
