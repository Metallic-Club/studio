//! `clone_tree`: deep copies of a subtree.

use super::{DataModel, DirtyBits, DmError};
use crate::Id;
use crate::Value;
use crate::registry::hash::FxMap;

impl DataModel {
    /// Whether the instance may be copied: its `Archivable` property, or `true` for a class without one.
    pub(super) fn archivable(&self, id: Id) -> bool {
        let prop = self
            .slots
            .get(id)
            .and_then(|s| self.classes.get(s.class.index()))
            .and_then(|c| c.archivable);
        prop.is_none_or(|p| self.bool(id, p).unwrap_or(true))
    }

    /// Deep-copies the instance and its subtree (`Clone` in scripts). The copy has no parent.
    ///
    /// Returns `None` when the instance's `Archivable` is false; descendants whose `Archivable` is false are left
    /// out together with their own subtrees. Name, stored properties and attributes are copied. References to
    /// instances inside the copied subtree point at their copies; references to anything else keep pointing at
    /// the original (Roblox semantics), and references to destroyed instances become nil. Each copy is marked
    /// `CREATED`, and its class behaviours' `on_created` run, parent first, once every copy holds its final values
    /// but before the copies are joined into a tree (a new instance never has a parent during `on_created`).
    pub fn clone_tree(&mut self, id: Id) -> Result<Option<Id>, DmError> {
        let class = self.slot(id)?.class;
        self.check_movable(id, class)?;
        if !self.archivable(id) {
            return Ok(None);
        }

        // Pass 1: copy each archivable instance, parent first, remembering which copy each one goes under.
        // `(copy, parent copy)`, parent first.
        let mut order: Vec<(Id, Option<Id>)> = Vec::new();
        let mut copies: FxMap<Id, Id> = FxMap::default();
        let mut current = Some(id);
        while let Some(source) = current {
            if source != id && !self.archivable(source) {
                current = self.next_after(id, source);
                continue;
            }
            let class = self.slot(source)?.class;
            let parent = self.slots.parent(source);
            let body = self.body(source)?.copy_without_children();
            let copy = self.spawn_body(class, body);
            let parent_copy = if source == id {
                None
            } else {
                parent.and_then(|p| copies.get(&p)).copied()
            };
            order.push((copy, parent_copy));
            copies.insert(source, copy);
            current = self.next_in_subtree(id, source);
        }

        // Pass 2: point references inside the subtree at the copies; drop references to destroyed instances.
        for &(copy, _) in &order {
            let mut i = 0;
            while let Some((prop, value)) = self.slots.body(copy).and_then(|b| b.props.get(i)) {
                let Value::Ref(Some(target)) = *value else {
                    i += 1;
                    continue;
                };
                let prop = *prop;
                if let Some(&mapped) = copies.get(&target) {
                    if let Some(entry) = self.slots.body_mut(copy).and_then(|b| b.props.get_mut(i)) {
                        entry.1 = Value::Ref(Some(mapped));
                    }
                    i += 1;
                } else if self.slots.contains(target) {
                    i += 1;
                } else if let Some(body) = self.slots.body_mut(copy) {
                    // A reference's default is always nil, so dropping the entry makes it read nil.
                    body.remove(prop);
                }
            }
        }

        // Pass 3: `on_created` runs before the copy has a parent, as for any new instance.
        for &(copy, _) in &order {
            if let Some(class) = self.slots.get(copy).map(|s| s.class) {
                self.run_hooks(copy, class, |b, dm| b.on_created(dm, copy));
            }
        }

        // Pass 4: rebuild the tree among the copies, in the original child order.
        for &(copy, parent_copy) in &order {
            if let Some(parent_copy) = parent_copy
                && self.slots.contains(copy)
                && self.slots.contains(parent_copy)
            {
                self.attach(copy, parent_copy);
                self.mark(copy, DirtyBits::STRUCTURE);
            }
        }
        self.structure_revision += 1;
        Ok(order
            .first()
            .map(|&(copy, _)| copy)
            .filter(|&copy| self.slots.contains(copy)))
    }
}
