//! One region's slots: the packed instances, who owns each slot, the free list and the dirty list.

use metallic_core::Id;

use super::instance::GpuInstance;
use super::lists::NOT_LISTED;
use super::{Region, UploadRange};

/// Bookkeeping for one slot, next to its packed instance.
#[derive(Clone, Copy, Debug)]
pub(super) struct SlotMeta {
    /// The instance in the slot; `None` when the slot is free.
    pub(super) owner: Option<Id>,
    /// Static region: frames with a transform change in the current window. Unused in the dynamic region.
    pub(super) moves: u16,
    /// Static region: the frame the current window started. Dynamic region: the last frame the transform changed.
    pub(super) since: u32,
    /// Queued on the dirty list.
    pub(super) dirty: bool,
    /// The packed transform changed and the slot is queued for the culling grid, which moves its item at the next
    /// cull ([`super::grid`]). Cleared when the grid has the item's current sphere.
    pub(super) moved: bool,
    /// The draw list the slot's entry is in ([`NOT_LISTED`] when none). Kept when the slot is freed or reused,
    /// until the next coalesce takes the entry out of the list.
    pub(super) list: u16,
    /// The entry's position in that list.
    pub(super) list_pos: u32,
}

impl SlotMeta {
    const FREE: Self = Self {
        owner: None,
        moves: 0,
        since: 0,
        dirty: false,
        moved: false,
        list: NOT_LISTED,
        list_pos: 0,
    };
}

/// One region of the scene: a dense array of slots, each keeping its index until its instance leaves.
///
/// Freed slots go on a free list and are reused before the array grows; nothing is ever compacted, so freeing one
/// slot uploads one slot. `instances` is the packed source the uploads copy from: only dirty slots are repacked,
/// and nothing compares it against anything.
#[derive(Debug, Default)]
pub(super) struct RegionSlots {
    pub(super) instances: Vec<GpuInstance>,
    pub(super) meta: Vec<SlotMeta>,
    free: Vec<u32>,
    dirty: Vec<u32>,
    live: u32,
}

impl RegionSlots {
    /// Slots in use, free ones included: everything below this index may be live.
    pub(super) fn len(&self) -> u32 {
        // The data model's ids are `u32` indices, so a region never holds more than `u32::MAX` slots.
        u32::try_from(self.instances.len()).unwrap_or(u32::MAX)
    }

    pub(super) fn live(&self) -> u32 {
        self.live
    }

    /// A slot for `owner`, reusing a free one first. The caller writes the instance and marks it dirty.
    pub(super) fn alloc(&mut self, owner: Id, frame: u32) -> u32 {
        self.live += 1;
        let meta = SlotMeta {
            owner: Some(owner),
            since: frame,
            ..SlotMeta::FREE
        };
        if let Some(slot) = self.free.pop()
            && let Some(m) = self.meta.get_mut(slot as usize)
        {
            // Freed earlier this frame, the slot may still be queued (keep it queued once) and still listed (the
            // next coalesce moves the entry).
            *m = SlotMeta {
                dirty: m.dirty,
                moved: m.moved,
                list: m.list,
                list_pos: m.list_pos,
                ..meta
            };
            return slot;
        }
        let slot = self.len();
        self.instances.push(GpuInstance::FREE);
        self.meta.push(meta);
        slot
    }

    /// Frees `slot`: it is zeroed (so it reads as not live on the GPU), uploaded, and reused by a later `alloc`.
    pub(super) fn release(&mut self, slot: u32) {
        let i = slot as usize;
        let (Some(instance), Some(meta)) = (self.instances.get_mut(i), self.meta.get_mut(i)) else {
            return;
        };
        if meta.owner.is_none() {
            return;
        }
        *instance = GpuInstance::FREE;
        *meta = SlotMeta {
            dirty: meta.dirty,
            moved: meta.moved,
            list: meta.list,
            list_pos: meta.list_pos,
            ..SlotMeta::FREE
        };
        self.live -= 1;
        self.free.push(slot);
        self.mark_dirty(slot);
    }

    /// Notes that `slot`'s packed transform changed, so the next cull moves its item in the culling grid. Returns
    /// whether it was not noted yet (the caller then queues it on the grid, once). A slot on no draw list has no
    /// item yet: the coalesce that lists it places it where it is then.
    #[inline]
    pub(super) fn mark_moved(&mut self, slot: u32) -> bool {
        match self.meta.get_mut(slot as usize) {
            Some(meta) if !meta.moved && meta.list != NOT_LISTED => {
                meta.moved = true;
                true
            }
            _ => false,
        }
    }

    /// Queues `slot` for upload (once, however often it changes before the upload).
    #[inline]
    pub(super) fn mark_dirty(&mut self, slot: u32) {
        if let Some(meta) = self.meta.get_mut(slot as usize)
            && !meta.dirty
        {
            meta.dirty = true;
            self.dirty.push(slot);
        }
    }

    pub(super) fn dirty_len(&self) -> usize {
        self.dirty.len()
    }

    /// Lends out the slots queued for upload (in the order they were queued), to walk them while changing other
    /// slots' bookkeeping. Give them back with [`RegionSlots::return_dirty`] before anything is marked dirty.
    pub(super) fn take_dirty(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.dirty)
    }

    /// Puts back what [`RegionSlots::take_dirty`] lent out.
    pub(super) fn return_dirty(&mut self, dirty: Vec<u32>) {
        self.dirty = dirty;
    }

    /// Turns the dirty slots into ranges, appended to `out`: sorted, adjacent slots merged, and runs separated
    /// by at most `max_gap` clean slots merged too (re-uploading a few unchanged slots costs less than another
    /// copy). Clears the dirty list.
    pub(super) fn coalesce(&mut self, region: Region, max_gap: u32, out: &mut Vec<UploadRange>) {
        self.dirty.sort_unstable();
        let mut run: Option<UploadRange> = None;
        for &slot in &self.dirty {
            if let Some(meta) = self.meta.get_mut(slot as usize) {
                meta.dirty = false;
            }
            match run.as_mut() {
                Some(r) if slot - (r.first + r.count) <= max_gap => r.count = slot - r.first + 1,
                _ => {
                    out.extend(run.take());
                    run = Some(UploadRange {
                        region,
                        first: slot,
                        count: 1,
                    });
                }
            }
        }
        out.extend(run);
        self.dirty.clear();
    }

    /// Drops every slot (for a full rebuild). Keeps the vectors' memory.
    pub(super) fn clear(&mut self) {
        self.instances.clear();
        self.meta.clear();
        self.free.clear();
        self.dirty.clear();
        self.live = 0;
    }
}
