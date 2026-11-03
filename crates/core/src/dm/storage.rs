//! Instance storage: a generational slot map of compact instances with sparse property values.
//!
//! The layout is sized for 100,000-part scenes (issue #14 budget: at most 256 bytes per `Part` with four
//! non-default properties, inline and heap together; `tests/alloc.rs` measures it):
//!
//! - a slot is 80 bytes: the generation, the parent's slot index, the previous and next siblings' slot indices, the
//!   class, one byte of dirty bits per change consumer ([`MAX_CONSUMERS`], eight), the signal "watched" flag, the
//!   pin count (see `pins.rs`), and a [`Body`] whose pointers are only paid for when used;
//! - an instance with its class's name stores no name (one null pointer); a renamed one boxes a shared string;
//! - only non-default property values are stored, in a `Vec` sorted by [`PropId`] that grows one entry at a time
//!   (four values are 160 bytes: `PropId` + `Value` is 40 bytes with alignment). Removing an entry keeps the
//!   capacity, so a property flipping between its default and another value (a `bool`) never allocates again;
//! - children and attributes sit behind `Option<Box<..>>`, one pointer each for the many instances without any.
//!
//! # Children: an intrusive sibling list
//!
//! Children are a doubly linked list threaded through the slots (`prev` and `next` above), and a parent keeps only
//! a [`ChildList`]: the first and last child and the count. Appending, removing any child and stepping to a
//! sibling are all O(1) and never allocate, so clearing a folder of 100,000 parts one by one is linear, and order
//! is insertion order. The links fit in what a position-in-parent index and padding used to take, so they cost
//! the slot nothing, and a parent no longer pays 8 bytes per child for an id vector. Iterating children chases links
//! instead of reading a contiguous array; parts created together sit in neighbouring slots, so that walk stays
//! mostly sequential.

use std::sync::Arc;

use super::MAX_CONSUMERS;
use crate::{ClassId, Id, PropId, Value};

/// One user attribute: its name and value.
pub(super) type Attribute = (Arc<str>, Value);

/// A slot link (`parent`, `prev`, `next`, a list's ends) that names no slot.
pub(super) const NONE: u32 = u32::MAX;

/// A parent's children: the ends of the sibling list threaded through their slots, and its length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ChildList {
    /// The first child's slot index, or [`NONE`].
    pub(super) first: u32,
    /// The last child's slot index, or [`NONE`].
    pub(super) last: u32,
    /// How many children there are.
    pub(super) len: u32,
}

impl ChildList {
    /// No children.
    pub(super) const EMPTY: Self = Self {
        first: NONE,
        last: NONE,
        len: 0,
    };
}

/// The parts of a live instance that own memory.
#[derive(Clone, Debug)]
#[expect(
    clippy::box_collection,
    reason = "a boxed Vec is one pointer, not three, for the many instances with no attributes"
)]
pub(super) struct Body {
    /// `None` while the instance has its class's name (the data model keeps that one shared per class), so most
    /// instances pay one pointer and no allocation for their name.
    pub(super) name: Option<Box<Arc<str>>>,
    /// Non-default values only, sorted by `PropId`.
    pub(super) props: Vec<(PropId, Value)>,
    /// The ends of the child list. `None` until the first child arrives; kept (possibly empty) after that, so a
    /// container that empties and refills does not allocate again.
    pub(super) children: Option<Box<ChildList>>,
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

    /// The ends and length of the child list (empty when there are no children).
    #[inline]
    pub(super) fn child_list(&self) -> ChildList {
        self.children.as_deref().copied().unwrap_or(ChildList::EMPTY)
    }
}

/// One entry of the slot map. A free slot has no body and keeps its generation, so old ids stay stale.
#[derive(Debug)]
pub(super) struct Slot {
    pub(super) generation: u32,
    /// The parent's slot index, or [`NONE`]. A parent outlives its children (destroying it destroys them),
    /// so the index always names a live slot and its generation completes the id.
    parent: u32,
    /// The previous and next siblings' slot indices, or [`NONE`] at either end of the parent's child list. Like
    /// `parent`, they always name live slots: a child leaves the list before its slot is freed.
    prev: u32,
    next: u32,
    pub(super) class: ClassId,
    /// Per change consumer: what changed since that consumer last drained. With eight consumers the fixed fields
    /// take 28 bytes, padded to 32 before the 48-byte body: four consumers fitted in 72 bytes, eight take 80.
    pub(super) dirty: [u8; MAX_CONSUMERS],
    /// Whether any signal connection listens to this instance, so an unwatched change costs one check. It sits
    /// in what was padding.
    pub(super) watched: bool,
    /// How many outside holders pinned the instance, up to `u8::MAX`; `pins.rs` counts the rest. It sits in what
    /// was padding.
    pub(super) pins: u8,
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

    /// The id of the slot at `index`, a link read from a live slot (so the slot is live too), or `None` for
    /// [`NONE`].
    #[inline]
    pub(super) fn id_at(&self, index: u32) -> Option<Id> {
        let slot = self.slots.get(index as usize)?;
        Some(Id::new(index, slot.generation))
    }

    /// The id and the previous and next sibling links of the slot at `index`, for walking a child list.
    #[inline]
    pub(super) fn siblings_at(&self, index: u32) -> Option<(Id, u32, u32)> {
        let slot = self.slots.get(index as usize)?;
        Some((Id::new(index, slot.generation), slot.prev, slot.next))
    }

    /// The parent of the live instance `id`.
    #[inline]
    pub(super) fn parent(&self, id: Id) -> Option<Id> {
        self.id_at(self.get(id)?.parent)
    }

    /// The first child of the live instance `id`.
    #[inline]
    pub(super) fn first_child(&self, id: Id) -> Option<Id> {
        self.id_at(self.body(id)?.children.as_deref()?.first)
    }

    /// The last child of the live instance `id`.
    #[inline]
    pub(super) fn last_child(&self, id: Id) -> Option<Id> {
        self.id_at(self.body(id)?.children.as_deref()?.last)
    }

    /// The sibling after the live instance `id`.
    #[inline]
    pub(super) fn next_sibling(&self, id: Id) -> Option<Id> {
        self.id_at(self.get(id)?.next)
    }

    /// The sibling before the live instance `id`.
    #[inline]
    pub(super) fn prev_sibling(&self, id: Id) -> Option<Id> {
        self.id_at(self.get(id)?.prev)
    }

    /// Appends `child`, which must have no parent, to the end of `parent`'s children. Allocates only for a
    /// parent's first child ever. Does nothing when either is stale.
    pub(super) fn append(&mut self, child: Id, parent: Id) {
        debug_assert!(self.parent(child).is_none(), "append of an instance that has a parent");
        if !self.contains(child) {
            return;
        }
        let Some(body) = self.body_mut(parent) else {
            return;
        };
        let list = body.children.get_or_insert_with(|| Box::new(ChildList::EMPTY));
        let c = child.index();
        let prev = list.last;
        list.last = c;
        if prev == NONE {
            list.first = c;
        }
        list.len += 1;
        if prev != NONE
            && let Some(slot) = self.slots.get_mut(prev as usize)
        {
            slot.next = c;
        }
        if let Some(slot) = self.slots.get_mut(c as usize) {
            slot.parent = parent.index();
            slot.prev = prev;
            slot.next = NONE;
        }
    }

    /// Takes `id` out of its parent's children, closing the gap between its siblings, and returns the old parent.
    /// O(1) wherever the child sits.
    pub(super) fn unlink(&mut self, id: Id) -> Option<Id> {
        let slot = self.get_mut(id)?;
        let (p, prev, next) = (slot.parent, slot.prev, slot.next);
        if p == NONE {
            return None;
        }
        slot.parent = NONE;
        slot.prev = NONE;
        slot.next = NONE;
        if prev != NONE
            && let Some(s) = self.slots.get_mut(prev as usize)
        {
            s.next = next;
        }
        if next != NONE
            && let Some(s) = self.slots.get_mut(next as usize)
        {
            s.prev = prev;
        }
        let parent = self.slots.get_mut(p as usize)?;
        if let Some(list) = parent.body.as_mut().and_then(|b| b.children.as_deref_mut()) {
            if prev == NONE {
                list.first = next;
            }
            if next == NONE {
                list.last = prev;
            }
            list.len = list.len.saturating_sub(1);
        }
        Some(Id::new(p, parent.generation))
    }

    /// Adds an instance with the given body, no parent and clean dirty bits.
    pub(super) fn insert(&mut self, class: ClassId, body: Body) -> Id {
        self.live += 1;
        if let Some(index) = self.free.pop()
            && let Some(slot) = self.slots.get_mut(index as usize)
        {
            slot.parent = NONE;
            slot.prev = NONE;
            slot.next = NONE;
            slot.class = class;
            slot.dirty = [0; MAX_CONSUMERS];
            slot.watched = false;
            slot.pins = 0;
            slot.body = Some(body);
            return Id::new(index, slot.generation);
        }
        // Slot indices are u32; four billion live instances is far past any scene this engine targets.
        let index = u32::try_from(self.slots.len()).unwrap_or(NONE - 1);
        self.slots.push(Slot {
            generation: 0,
            parent: NONE,
            prev: NONE,
            next: NONE,
            class,
            dirty: [0; MAX_CONSUMERS],
            watched: false,
            pins: 0,
            body: Some(body),
        });
        Id::new(index, 0)
    }

    /// Frees the slot of `id`, moving its generation on so every copy of `id` becomes stale. A slot whose
    /// generation would wrap is retired instead of reused, so an id is never handed out twice.
    ///
    /// The caller has taken `id` out of its parent's children first, unless the parent is freed with it (a
    /// destroyed subtree), so no live slot links to a freed one.
    pub(super) fn remove(&mut self, id: Id) -> Option<Body> {
        let slot = self.get_mut(id)?;
        let body = slot.body.take();
        slot.parent = NONE;
        slot.prev = NONE;
        slot.next = NONE;
        slot.dirty = [0; MAX_CONSUMERS];
        slot.watched = false;
        slot.pins = 0;
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
        assert!(size_of::<Slot>() <= 80, "Slot is {} bytes", size_of::<Slot>());
        assert_eq!(size_of::<Body>(), 48);
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
        slots.append(child, parent);
        assert_eq!(slots.parent(child), Some(parent));
        assert_eq!(slots.unlink(child), Some(parent));
        assert_eq!(slots.parent(child), None);
        assert_eq!(slots.unlink(child), None);
    }

    /// The child list from first to last and from last to first, checking every link agrees.
    fn chain(slots: &Slots, parent: Id) -> Vec<Id> {
        let list = slots.body(parent).map_or(ChildList::EMPTY, Body::child_list);
        let mut forward = Vec::new();
        let mut x = slots.first_child(parent);
        while let Some(id) = x {
            assert_eq!(slots.parent(id), Some(parent));
            forward.push(id);
            x = slots.next_sibling(id);
        }
        let mut backward = Vec::new();
        let mut x = slots.last_child(parent);
        while let Some(id) = x {
            backward.push(id);
            x = slots.prev_sibling(id);
        }
        backward.reverse();
        assert_eq!(forward, backward);
        assert_eq!(forward.len(), list.len as usize);
        forward
    }

    #[test]
    fn sibling_links_stay_consistent() {
        let mut slots = Slots::default();
        let parent = slots.insert(ClassId::new(0), Body::new());
        let kids: Vec<Id> = (0..5).map(|_| slots.insert(ClassId::new(0), Body::new())).collect();
        assert!(chain(&slots, parent).is_empty());
        for &k in &kids {
            slots.append(k, parent);
        }
        assert_eq!(chain(&slots, parent), kids);
        // Middle, front, back.
        slots.unlink(kids[2]);
        slots.unlink(kids[0]);
        slots.unlink(kids[4]);
        assert_eq!(chain(&slots, parent), [kids[1], kids[3]]);
        slots.append(kids[0], parent);
        assert_eq!(chain(&slots, parent), [kids[1], kids[3], kids[0]]);
        for &k in &[kids[1], kids[3], kids[0]] {
            slots.unlink(k);
        }
        assert!(chain(&slots, parent).is_empty());
        assert_eq!(slots.body(parent).map(Body::child_list), Some(ChildList::EMPTY));
        // The emptied list keeps its box, so refilling it does not allocate.
        assert!(slots.body(parent).is_some_and(|b| b.children.is_some()));
        slots.append(kids[4], parent);
        assert_eq!(chain(&slots, parent), [kids[4]]);
        // A stale parent or child is ignored.
        assert!(slots.remove(kids[2]).is_some());
        slots.append(kids[2], parent);
        assert_eq!(chain(&slots, parent), [kids[4]]);
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
