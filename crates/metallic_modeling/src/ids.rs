//! Stable element ids and the tombstoned arena that stores elements.
//!
//! Ids are never reused: removing an element leaves an empty slot, so an id always names the same element for the
//! whole life of a [`crate::Mesh`], including across undo and redo. [`crate::Mesh::compacted`] renumbers when the
//! tombstones should be dropped (for example before saving).

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);

        impl $name {
            /// The slot index of this id.
            #[must_use]
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id_type!(
    /// Stable id of a vertex.
    VertId
);
id_type!(
    /// Stable id of an edge.
    EdgeId
);
id_type!(
    /// Stable id of a face (polygon).
    FaceId
);

/// Slot storage with tombstones. Iteration is always in id order, so every algorithm on top is deterministic.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Arena<T> {
    slots: Vec<Option<T>>,
    live: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            live: 0,
        }
    }
}

impl<T> Arena<T> {
    /// The id the next pushed element gets.
    pub(crate) fn next_index(&self) -> u32 {
        // Meshes never get near 4 billion slots; saturate instead of panicking.
        u32::try_from(self.slots.len()).unwrap_or(u32::MAX)
    }

    /// Number of live elements.
    pub(crate) fn len(&self) -> usize {
        self.live
    }

    /// Number of slots, live or tombstoned.
    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn get(&self, index: u32) -> Option<&T> {
        self.slots.get(index as usize).and_then(Option::as_ref)
    }

    /// Puts `value` into slot `index` (growing as needed) and returns what was there.
    pub(crate) fn set(&mut self, index: u32, value: Option<T>) -> Option<T> {
        let i = index as usize;
        if i >= self.slots.len() {
            self.slots.resize_with(i + 1, || None);
        }
        let added = value.is_some();
        let old = std::mem::replace(&mut self.slots[i], value);
        if old.is_some() {
            self.live -= 1;
        }
        if added {
            self.live += 1;
        }
        old
    }

    /// Live elements in id order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (u32, &T)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|v| (i as u32, v)))
    }
}
