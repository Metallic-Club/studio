//! Instance ids.

/// A handle to an instance in the data model: a slot index plus the generation of that slot.
///
/// When an instance is destroyed its slot's generation moves on, so an id kept from before never equals the id of a
/// newer instance that reuses the slot. The data model uses that to detect stale ids.
///
/// Ids are runtime-only and have no serde impl: scene files use their own stable ids.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Id {
    index: u32,
    generation: u32,
}

impl Id {
    /// Makes an id from a slot index and that slot's generation. Only the data model's slot map should need this.
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// The slot this id points at.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The generation of the slot when this id was handed out.
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_id_differs_from_newer_id_in_same_slot() {
        let old = Id::new(7, 1);
        let new = Id::new(7, 2);
        assert_ne!(old, new);
        assert_eq!(old.index(), new.index());
        assert_eq!(new.generation(), 2);
    }
}
