//! The draw lists: which slots each draw covers, kept up to date from the dirty slots alone (ADR 0008 §4).
//!
//! Every live, drawn slot is an **entry** in exactly one list: the opaque list of its mesh kind, or the
//! transparent list. An entry is the slot index with [`DYNAMIC_ENTRY`] set for the dynamic region, so one list
//! spans both regions and a mesh kind is one draw whatever region its instances are in. Lists are dense and
//! unordered: an entry is appended when its slot starts being drawn and swap-removed when it stops, so a change
//! costs O(1) and nothing is ever scanned or compacted. Each slot records its list and position
//! ([`super::slots::SlotMeta`]), so the swap-remove can fix the moved entry's position.
//!
//! Lists are updated in [`super::InstanceScene::coalesce`], from the slots that are about to be uploaded anyway:
//! a slot's list follows from its packed instance (live, hidden, translucent, mesh kind), so no other bookkeeping
//! in the sync needs to know about lists.
//!
//! Each list keeps the positions whose entry changed since the last upload, so only those are uploaded. On the
//! vertex-buffer fallback the GPU list holds copies of the instances rather than slot ids (vertex buffers cannot
//! be indexed indirectly), so there a changed instance also marks its position ([`DrawLists::set_gather`]).
//!
//! Culling (#79) replaces "every live slot of the kind" with "every visible slot of the kind" in the same format:
//! a list of entries per mesh kind, drawn as one instanced draw each.

use std::sync::atomic::{AtomicU64, Ordering};

use super::Region;
use super::instance::GpuInstance;

/// The list of a slot that is in none ([`super::slots::SlotMeta::list`]).
pub(crate) const NOT_LISTED: u16 = u16::MAX;
/// The list index of the transparent list. Opaque lists are indexed by mesh kind, which stays below it.
pub(crate) const TRANSPARENT_LIST: u16 = u16::MAX - 1;

/// A revision no other change, in any scene of the process, has had: a view that sorted one scene's transparent
/// list can never mistake another scene's for it.
fn next_revision() -> u64 {
    static REVISION: AtomicU64 = AtomicU64::new(1);
    REVISION.fetch_add(1, Ordering::Relaxed)
}

/// Set in an entry whose slot is in the dynamic region; the low 31 bits are the slot.
pub const DYNAMIC_ENTRY: u32 = 1 << 31;

/// The entry for `slot` of `region`.
#[inline]
pub fn entry(region: Region, slot: u32) -> u32 {
    match region {
        Region::Static => slot,
        Region::Dynamic => slot | DYNAMIC_ENTRY,
    }
}

/// The region and slot an entry names.
#[inline]
pub fn entry_slot(entry: u32) -> (Region, u32) {
    let region = if entry & DYNAMIC_ENTRY == 0 {
        Region::Static
    } else {
        Region::Dynamic
    };
    (region, entry & !DYNAMIC_ENTRY)
}

/// The list a packed instance belongs in: none when its slot is free or it is fully transparent, the transparent
/// list when it is partly see-through, otherwise the opaque list of its mesh kind.
#[inline]
pub(crate) fn list_of(instance: &GpuInstance) -> u16 {
    let flags = instance.flags;
    if flags & GpuInstance::LIVE == 0 || flags & GpuInstance::HIDDEN != 0 {
        NOT_LISTED
    } else if flags & GpuInstance::TRANSLUCENT != 0 {
        TRANSPARENT_LIST
    } else {
        match instance.mesh().0 {
            kind if kind < TRANSPARENT_LIST => kind,
            // The two highest kinds would collide with the list markers; there are never that many kinds.
            _ => NOT_LISTED,
        }
    }
}

/// One draw list: its entries, and the positions changed since the last upload.
#[derive(Debug, Default)]
pub(crate) struct DrawList {
    pub(crate) entries: Vec<u32>,
    /// Positions to upload. May repeat, and may lie past the end after removals; the upload sorts and trims them.
    pub(crate) dirty: Vec<u32>,
}

impl DrawList {
    /// Appends `entry`, returning its position.
    fn push(&mut self, entry: u32) -> u32 {
        // A region never holds more than `u32::MAX` slots, so neither does a list.
        let pos = u32::try_from(self.entries.len()).unwrap_or(u32::MAX);
        self.entries.push(entry);
        self.dirty.push(pos);
        pos
    }

    /// Swap-removes the entry at `pos`, returning the entry moved into `pos`, if any.
    fn swap_remove(&mut self, pos: u32) -> Option<u32> {
        let last = self.entries.pop()?;
        let slot = self.entries.get_mut(pos as usize)?;
        *slot = last;
        self.dirty.push(pos);
        Some(last)
    }
}

/// Every draw list of a scene: one opaque list per mesh kind and one transparent list.
#[derive(Debug, Default)]
pub(crate) struct DrawLists {
    /// Indexed by mesh kind. Grows when a new kind first appears.
    pub(crate) opaque: Vec<DrawList>,
    pub(crate) transparent: DrawList,
    /// Changed instances also mark their position (the vertex-buffer fallback, whose lists hold copies).
    gather: bool,
    /// Changes whenever the transparent list's entries or any of their instances change.
    pub(crate) transparent_revision: u64,
}

impl DrawLists {
    pub(crate) fn set_gather(&mut self, gather: bool) {
        self.gather = gather;
    }

    pub(crate) fn list_mut(&mut self, list: u16) -> Option<&mut DrawList> {
        if list == TRANSPARENT_LIST {
            Some(&mut self.transparent)
        } else {
            self.opaque.get_mut(usize::from(list))
        }
    }

    /// Appends `entry` to `list` (growing the opaque lists for a new kind), returning its position.
    pub(crate) fn push(&mut self, list: u16, entry: u32) -> u32 {
        if list == TRANSPARENT_LIST {
            self.transparent_revision = next_revision();
        } else if usize::from(list) >= self.opaque.len() {
            // Only when a mesh kind is drawn for the first time.
            self.opaque.resize_with(usize::from(list) + 1, DrawList::default);
        }
        self.list_mut(list).map_or(0, |l| l.push(entry))
    }

    /// Takes the entry at `pos` out of `list`; returns the entry that moved into `pos`, whose slot must be told.
    pub(crate) fn remove(&mut self, list: u16, pos: u32) -> Option<u32> {
        if list == TRANSPARENT_LIST {
            self.transparent_revision = next_revision();
        }
        self.list_mut(list)?.swap_remove(pos)
    }

    /// The instance listed at `pos` of `list` changed, but not its list.
    pub(crate) fn changed(&mut self, list: u16, pos: u32) {
        if list == TRANSPARENT_LIST {
            self.transparent_revision = next_revision();
        } else if self.gather
            && let Some(l) = self.opaque.get_mut(usize::from(list))
        {
            l.dirty.push(pos);
        }
    }

    /// Empties every list (for a full rebuild). Keeps the memory.
    pub(crate) fn clear(&mut self) {
        for list in &mut self.opaque {
            list.entries.clear();
            list.dirty.clear();
        }
        self.transparent.entries.clear();
        self.transparent.dirty.clear();
        self.transparent_revision = next_revision();
    }
}

#[cfg(test)]
mod tests {
    use super::super::instance::MeshKind;
    use super::*;

    fn instance(kind: MeshKind, flags: u32) -> GpuInstance {
        GpuInstance {
            mesh_material: u32::from(kind.0) << 16,
            flags,
            ..GpuInstance::default()
        }
    }

    #[test]
    fn entries_name_region_and_slot() {
        assert_eq!(entry_slot(entry(Region::Static, 7)), (Region::Static, 7));
        assert_eq!(entry_slot(entry(Region::Dynamic, 7)), (Region::Dynamic, 7));
        assert_ne!(entry(Region::Static, 7), entry(Region::Dynamic, 7));
    }

    #[test]
    fn the_list_follows_the_flags() {
        let live = GpuInstance::LIVE;
        assert_eq!(list_of(&GpuInstance::FREE), NOT_LISTED);
        assert_eq!(list_of(&instance(MeshKind::WEDGE, live)), MeshKind::WEDGE.0);
        assert_eq!(
            list_of(&instance(MeshKind::BALL, live | GpuInstance::TRANSLUCENT)),
            TRANSPARENT_LIST
        );
        assert_eq!(
            list_of(&instance(MeshKind::BALL, live | GpuInstance::HIDDEN)),
            NOT_LISTED
        );
    }

    #[test]
    fn swap_remove_reports_the_moved_entry() {
        let mut lists = DrawLists::default();
        for e in 0..4 {
            assert_eq!(lists.push(1, e), e);
        }
        assert_eq!(lists.opaque.len(), 2);
        assert_eq!(lists.remove(1, 1), Some(3));
        assert_eq!(lists.opaque[1].entries, [0, 3, 2]);
        // Removing the last entry moves nothing.
        assert_eq!(lists.remove(1, 2), None);
        assert_eq!(lists.opaque[1].entries, [0, 3]);
    }

    #[test]
    fn data_changes_mark_positions_only_when_gathering() {
        let mut lists = DrawLists::default();
        lists.push(0, 5);
        lists.opaque[0].dirty.clear();
        lists.changed(0, 0);
        assert!(lists.opaque[0].dirty.is_empty());
        lists.set_gather(true);
        lists.changed(0, 0);
        assert_eq!(lists.opaque[0].dirty, [0]);
        let before = lists.transparent_revision;
        lists.push(TRANSPARENT_LIST, 9);
        lists.changed(TRANSPARENT_LIST, 0);
        assert_ne!(lists.transparent_revision, before);
    }
}
