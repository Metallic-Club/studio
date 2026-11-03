//! One region's slots: the packed instances, who owns each slot, the free list and the dirty list.

use metallic_core::Id;

use super::instance::GpuInstance;
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
}

impl SlotMeta {
    const FREE: Self = Self {
        owner: None,
        moves: 0,
        since: 0,
        dirty: false,
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
            moves: 0,
            since: frame,
            dirty: false,
        };
        if let Some(slot) = self.free.pop()
            && let Some(m) = self.meta.get_mut(slot as usize)
        {
            // Freed earlier this frame, the slot may still be queued: keep it queued once.
            *m = SlotMeta { dirty: m.dirty, ..meta };
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
        let dirty = meta.dirty;
        *meta = SlotMeta {
            dirty,
            ..SlotMeta::FREE
        };
        self.live -= 1;
        self.free.push(slot);
        self.mark_dirty(slot);
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
