//! Instance storage: a generational slot map of compact instances with sparse property values.
//!
//! The layout is sized for 100,000-part scenes (issue #14 budget: at most 256 bytes per `Part` with four
//! non-default properties, inline and heap together; `tests/alloc.rs` measures it):
//!
//! - a slot is 72 bytes: the generation, the parent's slot index, the position among the parent's children, the
//!   class, per-consumer dirty bits, and a [`Body`] whose pointers are only paid for when used;
//! - an instance with its class's name stores no name (one null pointer); a renamed one boxes a shared string;
//! - only non-default property values are stored, in a `Vec` sorted by [`PropId`] that grows one entry at a time
//!   (four values are 160 bytes: `PropId` + `Value` is 40 bytes with alignment). Removing an entry keeps the
//!   capacity, so a property flipping between its default and another value (a `bool`) never allocates again;
//! - children and attributes sit behind `Option<Box<..>>`, one pointer each for the many instances without any.

use std::sync::Arc;

use super::MAX_CONSUMERS;
use crate::{ClassId, Id, PropId, Value};

/// One user attribute: its name and value.
pub(super) type Attribute = (Arc<str>, Value);

/// `Slot::parent` when there is no parent.
const NO_PARENT: u32 = u32::MAX;

/// The parts of a live instance that own memory.
#[derive(Clone, Debug)]
#[expect(
    clippy::box_collection,
    reason = "a boxed Vec is one pointer, not three, for the many instances with no children or attributes"
)]
pub(super) struct Body {
    /// `None` while the instance has its class's name (the data model keeps that one shared per class), so most
    /// instances pay one pointer and no allocation for their name.
    pub(super) name: Option<Box<Arc<str>>>,
    /// Non-default values only, sorted by `PropId`.
    pub(super) props: Vec<(PropId, Value)>,
    /// In order. `None` until the first child arrives.
    pub(super) children: Option<Box<Vec<Id>>>,
    /// In insertion order. `None` until the first attribute is set.
    pub(super) attributes: Option<Box<Vec<Attribute>>>,
}

impl Body {
    /// A body with the class's name and nothing else.
    pub(super) fn new() -> Self {
        Self {
            name: None,
            props: Vec::new(),
            children: None,
            attributes: None,
        }
    }

    /// The stored value of `prop`, if it is not at its default.
    #[inline]
    pub(super) fn prop(&self, prop: PropId) -> Option<&Value> {
        // A linear scan beats a binary search on the handful of entries an instance stores.
        for (p, value) in &self.props {
            if *p == prop {
                return Some(value);
            }
            if *p > prop {
                break;
            }
        }
        None
    }

    /// Stores `value` for `prop`: overwrites an existing entry in place, or inserts one, growing the list by
    /// exactly one entry when it is full (memory matters more than insert speed: entries are added rarely).
    #[inline]
    pub(super) fn store(&mut self, prop: PropId, value: Value) {
        match self.props.binary_search_by_key(&prop, |(p, _)| *p) {
            Ok(i) => {
                if let Some(entry) = self.props.get_mut(i) {
                    entry.1 = value;
                }
            }
            Err(at) => {
                if self.props.len() == self.props.capacity() {
                    self.props.reserve_exact(1);
                }
                self.props.insert(at, (prop, value));
            }
        }
    }

    /// Removes the entry for `prop`, so it reads as its default again. The capacity is kept.
    pub(super) fn remove(&mut self, prop: PropId) {
        if let Ok(i) = self.props.binary_search_by_key(&prop, |(p, _)| *p) {
            self.props.remove(i);
        }
    }

    /// A copy for a clone: the same name, values and attributes, exact-size, and no children.
    pub(super) fn copy_without_children(&self) -> Self {
        let mut props = Vec::with_capacity(self.props.len());
        props.extend_from_slice(&self.props);
        Self {
            name: self.name.clone(),
            props,
            children: None,
            attributes: self.attributes.clone(),
        }
    }

    /// The children, in order.
    pub(super) fn children(&self) -> &[Id] {
        self.children.as_deref().map_or(&[], Vec::as_slice)
    }

    /// The children list, created on first use.
    pub(super) fn children_mut(&mut self) -> &mut Vec<Id> {
        self.children.get_or_insert_with(Box::default)
    }
}

/// One entry of the slot map. A free slot has no body and keeps its generation, so old ids stay stale.
#[derive(Debug)]
pub(super) struct Slot {
    pub(super) generation: u32,
    /// The parent's slot index, or `NO_PARENT`. A parent outlives its children (destroying it destroys them),
    /// so the index always names a live slot and its generation completes the id.
    parent: u32,
    /// The position in the parent's children, so walking the tree never searches a sibling list.
    pub(super) sibling: u32,
    pub(super) class: ClassId,
    /// Per change consumer: what changed since that consumer last drained.
    pub(super) dirty: [u8; MAX_CONSUMERS],
    pub(super) body: Option<Body>,
}

/// The slot map: slots, the free list and the live count.
#[derive(Debug, Default)]
pub(super) struct Slots {
    slots: Vec<Slot>,
    /// Free slot indices, reused last-in first-out.
    free: Vec<u32>,
    live: usize,
}

impl Slots {
    /// How many instances are alive.
    pub(super) fn live(&self) -> usize {
        self.live
    }

    /// Room for `additional` more instances without growing the slot table.
    pub(super) fn reserve(&mut self, additional: usize) {
        self.slots.reserve(additional.saturating_sub(self.free.len()));
    }

    /// The live slot `id` names, or `None` when it is stale or unknown.
    #[inline]
    pub(super) fn get(&self, id: Id) -> Option<&Slot> {
        let slot = self.slots.get(id.index() as usize)?;
        (slot.generation == id.generation() && slot.body.is_some()).then_some(slot)
    }

    /// The live slot `id` names, mutably.
    #[inline]
    pub(super) fn get_mut(&mut self, id: Id) -> Option<&mut Slot> {
        let slot = self.slots.get_mut(id.index() as usize)?;
        (slot.generation == id.generation() && slot.body.is_some()).then_some(slot)
    }

    /// The body of the live instance `id`.
    #[inline]
    pub(super) fn body(&self, id: Id) -> Option<&Body> {
        self.get(id).and_then(|s| s.body.as_ref())
    }

    /// The body of the live instance `id`, mutably.
    #[inline]
    pub(super) fn body_mut(&mut self, id: Id) -> Option<&mut Body> {
        self.get_mut(id).and_then(|s| s.body.as_mut())
    }

    /// Whether `id` names a live instance.
    #[inline]
    pub(super) fn contains(&self, id: Id) -> bool {
        self.get(id).is_some()
    }

    /// The parent of the live instance `id`.
    #[inline]
    pub(super) fn parent(&self, id: Id) -> Option<Id> {
        let index = self.get(id)?.parent;
        if index == NO_PARENT {
            return None;
        }
        let parent = self.slots.get(index as usize)?;
        Some(Id::new(index, parent.generation))
    }

    /// Records that `id` is the child at position `sibling` of `parent`. The caller updates the children list.
    pub(super) fn link(&mut self, id: Id, parent: Id, sibling: u32) {
        if let Some(slot) = self.get_mut(id) {
            slot.parent = parent.index();
            slot.sibling = sibling;
        }
    }

    /// Clears `id`'s parent, returning the old parent and the position among its children. The caller updates
    /// the children list.
    pub(super) fn unlink(&mut self, id: Id) -> Option<(Id, u32)> {
        let parent = self.parent(id)?;
        let slot = self.get_mut(id)?;
        slot.parent = NO_PARENT;
        Some((parent, slot.sibling))
    }

    /// Adds an instance with the given body, no parent and clean dirty bits.
    pub(super) fn insert(&mut self, class: ClassId, body: Body) -> Id {
        self.live += 1;
        if let Some(index) = self.free.pop()
            && let Some(slot) = self.slots.get_mut(index as usize)
        {
            slot.parent = NO_PARENT;
            slot.sibling = 0;
            slot.class = class;
            slot.dirty = [0; MAX_CONSUMERS];
            slot.body = Some(body);
            return Id::new(index, slot.generation);
        }
        // Slot indices are u32; four billion live instances is far past any scene this engine targets.
        let index = u32::try_from(self.slots.len()).unwrap_or(NO_PARENT - 1);
        self.slots.push(Slot {
            generation: 0,
            parent: NO_PARENT,
            sibling: 0,
            class,
            dirty: [0; MAX_CONSUMERS],
            body: Some(body),
        });
        Id::new(index, 0)
    }

    /// Frees the slot of `id`, moving its generation on so every copy of `id` becomes stale. A slot whose
    /// generation would wrap is retired instead of reused, so an id is never handed out twice.
    pub(super) fn remove(&mut self, id: Id) -> Option<Body> {
        let slot = self.get_mut(id)?;
        let body = slot.body.take();
        slot.parent = NO_PARENT;
        slot.dirty = [0; MAX_CONSUMERS];
        let next = slot.generation.checked_add(1);
        if let Some(next) = next {
            slot.generation = next;
            self.free.push(id.index());
        }
        self.live -= 1;
        body
    }

    /// Every live instance's id, in slot order.
    #[cfg(test)]
    pub(super) fn ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            let index = u32::try_from(i).ok()?;
            slot.body.is_some().then(|| Id::new(index, slot.generation))
        })
    }

    /// Calls `f` with every live instance's id and slot, in slot order.
    pub(super) fn for_each_live_mut(&mut self, mut f: impl FnMut(Id, &mut Slot)) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.body.is_some()
                && let Ok(index) = u32::try_from(i)
            {
                f(Id::new(index, slot.generation), slot);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_is_compact() {
        assert!(size_of::<Slot>() <= 72, "Slot is {} bytes", size_of::<Slot>());
        assert_eq!(size_of::<(PropId, Value)>(), 40);
    }

    #[test]
    fn freed_slots_are_reused_with_a_new_generation() {
        let mut slots = Slots::default();
        let a = slots.insert(ClassId::new(0), Body::new());
        assert!(slots.remove(a).is_some());
        assert!(!slots.contains(a));
        assert!(slots.remove(a).is_none());
        let b = slots.insert(ClassId::new(0), Body::new());
        assert_eq!(b.index(), a.index());
        assert_ne!(b, a);
        assert!(slots.get(a).is_none());
        assert_eq!(slots.live(), 1);
        assert_eq!(slots.ids().collect::<Vec<_>>(), vec![b]);
    }

    #[test]
    fn parent_links_carry_the_generation() {
        let mut slots = Slots::default();
        let parent = slots.insert(ClassId::new(0), Body::new());
        let child = slots.insert(ClassId::new(0), Body::new());
        assert_eq!(slots.parent(child), None);
        slots.link(child, parent, 3);
        assert_eq!(slots.parent(child), Some(parent));
        assert_eq!(slots.unlink(child), Some((parent, 3)));
        assert_eq!(slots.parent(child), None);
        assert_eq!(slots.unlink(child), None);
    }

    #[test]
    fn props_stay_sorted_and_sparse() {
        let mut b = Body::new();
        b.store(PropId::new(5), Value::Bool(true));
        b.store(PropId::new(1), Value::Int(1));
        b.store(PropId::new(3), Value::Int(3));
        b.store(PropId::new(3), Value::Int(4));
        let keys: Vec<u16> = b.props.iter().map(|(p, _)| p.raw()).collect();
        assert_eq!(keys, vec![1, 3, 5]);
        assert_eq!(b.props.capacity(), 3, "grows one entry at a time");
        assert_eq!(b.prop(PropId::new(3)), Some(&Value::Int(4)));
        b.remove(PropId::new(1));
        b.remove(PropId::new(9));
        assert_eq!(b.props.len(), 2);
        assert_eq!(b.prop(PropId::new(1)), None);
        b.store(PropId::new(1), Value::Int(1));
        assert_eq!(b.props.capacity(), 3, "a removed entry's room is reused");
    }
}
