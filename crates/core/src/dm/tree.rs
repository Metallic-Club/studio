//! The tree: parents, children, names, searches and destruction.
//!
//! Children are a sibling list threaded through the slots (see `storage.rs`), so adding or removing a child is O(1)
//! wherever it sits, and a depth-first walk moves to the next sibling in constant time and needs no stack:
//! [`Children`] and [`Descendants`] never allocate.

use std::iter::FusedIterator;
use std::sync::Arc;

use super::storage::Slots;
use super::{DataModel, DirtyBits, DmError, Notice};
use crate::{ClassId, Id};

/// The children of an instance, in order (the order they were parented in), from [`DataModel::children`]. Never
/// allocates; knows its length and walks from either end.
#[derive(Clone, Debug)]
pub struct Children<'a> {
    slots: &'a Slots,
    /// The slot index of the next child from the front, and from the back.
    front: u32,
    back: u32,
    /// How many children are left; the two ends meet when it reaches zero.
    len: usize,
}

impl Children<'_> {
    /// Whether no children are left.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Iterator for Children<'_> {
    type Item = Id;

    #[inline]
    fn next(&mut self) -> Option<Id> {
        if self.len == 0 {
            return None;
        }
        let (id, _, next) = self.slots.siblings_at(self.front)?;
        self.front = next;
        self.len -= 1;
        Some(id)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl DoubleEndedIterator for Children<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Id> {
        if self.len == 0 {
            return None;
        }
        let (id, prev, _) = self.slots.siblings_at(self.back)?;
        self.back = prev;
        self.len -= 1;
        Some(id)
    }
}

impl ExactSizeIterator for Children<'_> {}

impl FusedIterator for Children<'_> {}

/// The descendants of an instance, depth-first in child order (parents before their children). The instance
/// itself is not included. Never allocates.
#[derive(Clone, Debug)]
pub struct Descendants<'a> {
    dm: &'a DataModel,
    root: Id,
    next: Option<Id>,
}

impl Iterator for Descendants<'_> {
    type Item = Id;

    fn next(&mut self) -> Option<Id> {
        let current = self.next?;
        self.next = self.dm.next_in_subtree(self.root, current);
        Some(current)
    }
}

impl FusedIterator for Descendants<'_> {}

impl DataModel {
    /// The instance's parent, or `None` when it has none (always, for a destroyed instance that can still be read).
    pub fn parent(&self, id: Id) -> Result<Option<Id>, DmError> {
        self.view(id)?;
        Ok(self.slots.parent(id))
    }

    /// The instance's children, in order. See [`Children`]. A destroyed instance that can still be read has none.
    pub fn children(&self, id: Id) -> Result<Children<'_>, DmError> {
        let list = self.view(id)?.body.child_list();
        Ok(Children {
            slots: &self.slots,
            front: list.first,
            back: list.last,
            len: list.len as usize,
        })
    }

    /// How many children the instance has.
    pub fn child_count(&self, id: Id) -> Result<usize, DmError> {
        Ok(self.view(id)?.body.child_list().len as usize)
    }

    /// Every descendant of the instance, depth-first in child order. See [`Descendants`].
    pub fn descendants(&self, id: Id) -> Result<Descendants<'_>, DmError> {
        self.view(id)?;
        Ok(Descendants {
            dm: self,
            root: id,
            next: self.slots.first_child(id),
        })
    }

    /// The instance's name: its own, or its class's.
    fn name_ref(&self, id: Id) -> Result<&Arc<str>, DmError> {
        let view = self.view(id)?;
        if let Some(name) = view.body.name.as_deref() {
            return Ok(name);
        }
        self.classes
            .get(view.class.index())
            .map(|c| &c.name)
            .ok_or(DmError::UnknownClass(view.class.raw()))
    }

    /// The instance's name.
    pub fn name(&self, id: Id) -> Result<&str, DmError> {
        Ok(self.name_ref(id)?)
    }

    /// The instance's name as a shared string, for a computed `Name` property. Cloning it never allocates.
    pub fn shared_name(&self, id: Id) -> Result<Arc<str>, DmError> {
        Ok(Arc::clone(self.name_ref(id)?))
    }

    /// Renames the instance. Renaming to the current name does nothing; renaming back to the class's name frees
    /// the stored one.
    pub fn set_name(&mut self, id: Id, name: Arc<str>) -> Result<(), DmError> {
        self.slot(id)?;
        if **self.name_ref(id)? == *name {
            return Ok(());
        }
        let is_class_name = self
            .slots
            .get(id)
            .and_then(|s| self.classes.get(s.class.index()))
            .is_some_and(|c| c.name == name);
        let Some(slot) = self.slots.get_mut(id) else {
            return Err(DmError::NoInstance(id));
        };
        let Some(body) = slot.body.as_mut() else {
            return Err(DmError::NoInstance(id));
        };
        body.name = (!is_class_name).then(|| Box::new(name));
        // Live: its slot was just looked up.
        if let Some(dirty) = self.slots.dirty_at_mut(id.index()) {
            self.changes.mark(dirty, id, DirtyBits::PROPERTY);
        }
        self.notify(Notice::NameChanged { id });
        Ok(())
    }

    /// Moves the instance under `new_parent`, or out of the tree with `None`. This is the only way an instance
    /// changes parent.
    ///
    /// Refuses the root, services, a stale `new_parent`, and a new parent that is the instance itself or one of
    /// its descendants. Moving to the current parent does nothing. Otherwise the instance is removed from the old
    /// parent's children and appended to the new parent's, its whole subtree is marked `STRUCTURE`, each class
    /// behaviour's `on_parent_changed` runs, and the move is reported for signals.
    pub fn set_parent(&mut self, id: Id, new_parent: Option<Id>) -> Result<(), DmError> {
        let class = self.slot(id)?.class;
        let old = self.slots.parent(id);
        self.check_movable(id, class)?;
        if let Some(parent) = new_parent {
            if !self.slots.contains(parent) {
                return Err(self.not_live(parent));
            }
            if parent == id || self.is_ancestor(id, parent) {
                return Err(DmError::WouldCreateCycle {
                    name: self.name(id)?.to_owned(),
                });
            }
        }
        if old == new_parent {
            return Ok(());
        }
        self.detach(id);
        if let Some(parent) = new_parent {
            self.attach(id, parent);
        }
        self.structure_revision += 1;
        self.mark_subtree(id, DirtyBits::STRUCTURE);
        self.run_hooks(id, class, |b, dm| b.on_parent_changed(dm, id, old, new_parent));
        self.notify(Notice::ParentChanged {
            id,
            old,
            new: new_parent,
        });
        Ok(())
    }

    /// Refuses the root and services, which never move, clone or die.
    pub(super) fn check_movable(&self, id: Id, class: ClassId) -> Result<(), DmError> {
        if self.root == Some(id) {
            return Err(DmError::IsRoot {
                class: self.class_label(class),
            });
        }
        if self.is_service_class(class) {
            return Err(DmError::IsService {
                class: self.class_label(class),
            });
        }
        Ok(())
    }

    /// Appends `id` (which has no parent) to `parent`'s children.
    pub(super) fn attach(&mut self, id: Id, parent: Id) {
        self.slots.append(id, parent);
    }

    /// Removes `id` from its parent's children, in constant time.
    fn detach(&mut self, id: Id) {
        self.slots.unlink(id);
    }

    /// Whether `ancestor` is above `id` in the tree.
    fn is_ancestor(&self, ancestor: Id, id: Id) -> bool {
        let mut current = self.slots.parent(id);
        while let Some(x) = current {
            if x == ancestor {
                return true;
            }
            current = self.slots.parent(x);
        }
        false
    }

    /// Whether `ancestor` is above the instance in the tree (`IsDescendantOf`). An instance is not its own
    /// descendant, and a destroyed instance that can still be read is nobody's descendant or ancestor.
    pub fn is_descendant_of(&self, id: Id, ancestor: Id) -> Result<bool, DmError> {
        self.view(id)?;
        self.view(ancestor)?;
        Ok(self.is_ancestor(ancestor, id))
    }

    /// The instance after `current` in a depth-first walk of `root`'s subtree.
    pub(super) fn next_in_subtree(&self, root: Id, current: Id) -> Option<Id> {
        match self.slots.first_child(current) {
            Some(child) => Some(child),
            None => self.next_after(root, current),
        }
    }

    /// The instance after `current`'s whole subtree in a depth-first walk of `root`'s subtree.
    pub(super) fn next_after(&self, root: Id, current: Id) -> Option<Id> {
        let mut x = current;
        while x != root {
            if let Some(next) = self.slots.next_sibling(x) {
                return Some(next);
            }
            x = self.slots.parent(x)?;
        }
        None
    }

    /// Marks `root` and every descendant with `bits`.
    fn mark_subtree(&mut self, root: Id, bits: DirtyBits) {
        let mut current = Some(root);
        while let Some(x) = current {
            self.mark(x, bits);
            current = self.next_in_subtree(root, x);
        }
    }

    /// The first child (or, with `recursive`, descendant in depth-first order) named `name`.
    pub fn find_first_child(&self, id: Id, name: &str, recursive: bool) -> Result<Option<Id>, DmError> {
        self.find(id, recursive, |dm, x| dm.name(x).is_ok_and(|n| n == name))
    }

    /// The first child (or descendant) whose class is exactly `class`.
    pub fn find_first_child_of_class(&self, id: Id, class: ClassId, recursive: bool) -> Result<Option<Id>, DmError> {
        self.find(id, recursive, |dm, x| dm.slots.get(x).is_some_and(|s| s.class == class))
    }

    /// The first child (or descendant) whose class is `class` or inherits from it.
    pub fn find_first_child_which_is_a(&self, id: Id, class: ClassId, recursive: bool) -> Result<Option<Id>, DmError> {
        self.find(id, recursive, |dm, x| {
            dm.slots.get(x).is_some_and(|s| dm.registry.is_a(s.class, class))
        })
    }

    fn find(&self, id: Id, recursive: bool, matches: impl Fn(&Self, Id) -> bool) -> Result<Option<Id>, DmError> {
        if recursive {
            Ok(self.descendants(id)?.find(|&x| matches(self, x)))
        } else {
            Ok(self.children(id)?.find(|&x| matches(self, x)))
        }
    }

    /// The dotted path of names from the top of the tree down to the instance, without the root, like Roblox's
    /// `GetFullName`: `Workspace.Model.Part`. The root's own full name is its name; an instance outside the tree
    /// starts from its top-most ancestor.
    pub fn full_name(&self, id: Id) -> Result<String, DmError> {
        let mut names: Vec<&str> = Vec::new();
        let mut current = Some(id);
        while let Some(x) = current {
            if self.root == Some(x) && x != id {
                break;
            }
            names.push(self.name(x)?);
            current = self.parent(x)?;
        }
        names.reverse();
        Ok(names.join("."))
    }

    /// Destroys the instance and its whole subtree.
    ///
    /// Refuses the root and services. First every instance in the subtree, parent first, is reported as
    /// destroying and its class behaviours' `on_destroying` run while the tree is intact. Then the subtree is
    /// detached (reported for signals as a move out of the tree: `ChildRemoved`, `AncestryChanged`, ...) and
    /// every slot freed: ids held anywhere become stale, every change consumer gets them from
    /// [`DataModel::drain_removed`], and references to them in other instances read as nil. An instance that is
    /// pinned, or any instance while something is connected anywhere, leaves a snapshot that keeps it readable
    /// (see [`DataModel::pin`]).
    ///
    /// Destroying a destroyed instance that can still be read does nothing, as in Roblox.
    pub fn destroy(&mut self, id: Id) -> Result<(), DmError> {
        let class = match self.slots.get(id) {
            Some(slot) => slot.class,
            None if self.is_destroyed(id) => return Ok(()),
            None => return Err(DmError::NoInstance(id)),
        };
        self.check_movable(id, class)?;

        let mut subtree = std::mem::take(&mut self.scratch);
        self.collect_subtree(id, &mut subtree);
        for &x in &subtree {
            if let Some(class) = self.slots.get(x).map(|s| s.class) {
                self.notify(Notice::Destroying { id: x });
                self.run_hooks(x, class, |b, dm| b.on_destroying(dm, x));
            }
        }
        // Hooks may have changed the subtree (or destroyed it); free what is there now.
        if self.slots.contains(id) {
            self.collect_subtree(id, &mut subtree);
            let old = self.slots.parent(id);
            self.detach(id);
            if old.is_some() {
                // Leaving the tree, reported while the subtree is still intact so its events can walk it.
                self.notify(Notice::ParentChanged { id, old, new: None });
            }
            // While anything is connected, every destroyed instance stays readable until its events are delivered
            // (a `ChildRemoved` handler is handed the child), as graves did; otherwise only pinned ones do.
            let grave = self.any_connection();
            for &x in &subtree {
                let Some(slot) = self.slots.get(x) else {
                    continue;
                };
                if let Some(dirty) = self.slots.dirty_at(x.index()) {
                    self.changes.removed(dirty, x);
                }
                if slot.pins == 0 && !grave {
                    self.slots.remove(x);
                    continue;
                }
                let (class, pins) = (slot.class, slot.pins);
                if let Some(body) = self.slots.remove(x) {
                    self.keep_snapshot(x, class, body, pins, grave);
                }
            }
            self.structure_revision += 1;
        }
        subtree.clear();
        self.scratch = subtree;
        Ok(())
    }

    /// Replaces `out` with `root` and its descendants, parent first.
    fn collect_subtree(&self, root: Id, out: &mut Vec<Id>) {
        out.clear();
        let mut current = Some(root);
        while let Some(x) = current {
            out.push(x);
            current = self.next_in_subtree(root, x);
        }
    }

    /// Destroys every child of the instance (`ClearAllChildren`), last child first. Services under the root are
    /// kept. A destroyed instance that can still be read has no children, so this does nothing.
    pub fn destroy_children(&mut self, id: Id) -> Result<(), DmError> {
        if self.view(id)?.body.child_list().len == 0 {
            return Ok(());
        }
        let mut current = self.slots.last_child(id);
        while let Some(child) = current {
            let prev = self.slots.prev_sibling(child);
            match self.destroy(child) {
                Ok(()) | Err(DmError::IsService { .. }) => {}
                Err(other) => return Err(other),
            }
            current = match prev {
                Some(p) if self.slots.parent(p) == Some(id) => Some(p),
                // A hook moved or destroyed the previous child; continue from what is left.
                Some(_) => self.slots.last_child(id),
                None => None,
            };
        }
        Ok(())
    }
}
