//! The tree: parents, children, names, searches and destruction.
//!
//! Each slot stores its position among its parent's children, so a depth-first walk moves to the next sibling in
//! constant time and needs no stack: [`Descendants`] never allocates.

use std::iter::FusedIterator;
use std::sync::Arc;

use super::{DataModel, DirtyBits, DmError, Notice};
use crate::{ClassId, Id};

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
    /// The instance's parent, or `None` when it has none.
    pub fn parent(&self, id: Id) -> Result<Option<Id>, DmError> {
        self.slot(id)?;
        Ok(self.slots.parent(id))
    }

    /// The instance's children, in order.
    pub fn children(&self, id: Id) -> Result<&[Id], DmError> {
        Ok(self.body(id)?.children())
    }

    /// Every descendant of the instance, depth-first in child order. See [`Descendants`].
    pub fn descendants(&self, id: Id) -> Result<Descendants<'_>, DmError> {
        Ok(Descendants {
            dm: self,
            root: id,
            next: self.body(id)?.children().first().copied(),
        })
    }

    /// The live instance's name: its own, or its class's.
    fn name_ref(&self, id: Id) -> Result<&Arc<str>, DmError> {
        let slot = self.slot(id)?;
        if let Some(name) = slot.body.as_ref().and_then(|b| b.name.as_deref()) {
            return Ok(name);
        }
        self.classes
            .get(slot.class.index())
            .map(|c| &c.name)
            .ok_or(DmError::UnknownClass(slot.class.raw()))
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
        self.changes.mark(slot, id, DirtyBits::PROPERTY);
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
                return Err(DmError::NoInstance(parent));
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
        if !self.slots.contains(id) {
            return;
        }
        let Some(body) = self.slots.body_mut(parent) else {
            return;
        };
        let children = body.children_mut();
        let sibling = u32::try_from(children.len()).unwrap_or(u32::MAX);
        children.push(id);
        self.slots.link(id, parent, sibling);
    }

    /// Removes `id` from its parent's children, renumbering the later siblings.
    fn detach(&mut self, id: Id) {
        let Some((parent, at)) = self.slots.unlink(id) else {
            return;
        };
        let at = at as usize;
        let Some(body) = self.slots.body_mut(parent) else {
            return;
        };
        let children = body.children_mut();
        if at < children.len() {
            children.remove(at);
        }
        let len = children.len();
        for i in at..len {
            let Some(&sibling) = self.slots.body(parent).and_then(|b| b.children().get(i)) else {
                break;
            };
            if let Some(slot) = self.slots.get_mut(sibling) {
                slot.sibling = u32::try_from(i).unwrap_or(u32::MAX);
            }
        }
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
    /// descendant.
    pub fn is_descendant_of(&self, id: Id, ancestor: Id) -> Result<bool, DmError> {
        self.slot(id)?;
        self.slot(ancestor)?;
        Ok(self.is_ancestor(ancestor, id))
    }

    /// The instance after `current` in a depth-first walk of `root`'s subtree.
    pub(super) fn next_in_subtree(&self, root: Id, current: Id) -> Option<Id> {
        match self.slots.body(current).and_then(|b| b.children().first()) {
            Some(&child) => Some(child),
            None => self.next_after(root, current),
        }
    }

    /// The instance after `current`'s whole subtree in a depth-first walk of `root`'s subtree.
    pub(super) fn next_after(&self, root: Id, current: Id) -> Option<Id> {
        let mut x = current;
        while x != root {
            let slot = self.slots.get(x)?;
            let parent = self.slots.parent(x)?;
            let siblings = self.slots.body(parent)?.children();
            if let Some(&next) = siblings.get(slot.sibling as usize + 1) {
                return Some(next);
            }
            x = parent;
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
            Ok(self.children(id)?.iter().copied().find(|&x| matches(self, x)))
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
    /// detached and every slot freed: ids held anywhere become stale, every change consumer gets them from
    /// [`DataModel::drain_removed`], and references to them in other instances read as nil.
    pub fn destroy(&mut self, id: Id) -> Result<(), DmError> {
        let class = self.slot(id)?.class;
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
            self.detach(id);
            for &x in &subtree {
                if let Some(slot) = self.slots.get(x) {
                    self.changes.removed(slot, x);
                }
                self.slots.remove(x);
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

    /// Destroys every child of the instance (`ClearAllChildren`), last child first so each removal is cheap even
    /// under a parent with many children. Services under the root are kept.
    pub fn destroy_children(&mut self, id: Id) -> Result<(), DmError> {
        let mut i = self.children(id)?.len();
        while i > 0 {
            i -= 1;
            let Some(&child) = self.slots.body(id).and_then(|b| b.children().get(i)) else {
                // A hook removed several children; continue from what is left.
                i = self.slots.body(id).map_or(0, |b| b.children().len());
                continue;
            };
            match self.destroy(child) {
                Ok(()) | Err(DmError::IsService { .. }) => {}
                Err(other) => return Err(other),
            }
        }
        Ok(())
    }
}
