//! Pins and snapshots: what stays readable of a destroyed instance.
//!
//! Destroying an instance frees its slot and moves the slot's generation on (see `storage.rs`), so every copy of
//! its id goes stale. Scripts still hold such ids (a Luau variable, a handler's argument) and expect to read them,
//! as in Roblox: `print(part.Name, "was destroyed")`. So the data model keeps a destroyed instance's last state, a
//! [`Snapshot`] (its class and its body: name, stored property values and attributes, without children), keyed by
//! the old id, in two cases:
//!
//! - **It is pinned.** [`DataModel::pin`] counts an outside holder; a script backend pins an instance while a script
//!   value refers to it and unpins it when that value is collected. The snapshot lives until the last pin goes.
//! - **Its events are being delivered.** When anything is connected anywhere, every destroyed instance is kept
//!   until the drain after the one that delivers its events begins (the "grave" window, `signals.rs`), so a
//!   `ChildRemoved` handler can read the child even though no script held it. A handler that keeps the instance
//!   pins it, and the snapshot then outlives the window.
//!
//! An instance destroyed with no pin while nothing is connected leaves nothing behind: that destroy costs one more
//! byte check than before, and the perf gate's `destroy` workload measures exactly that case.
//!
//! # What a snapshot answers
//!
//! The old id resolves to the snapshot **for reads only**: class, `IsA`, name, every property (stored values, class
//! defaults, computed ones derived from them), attributes. `Parent` is nil, there are no children or descendants,
//! and the full name is the name alone. Writes, moves, renames, clones, connections and fired signals fail with
//! [`DmError::Destroyed`]; destroying it again (or clearing its children) does nothing, as in Roblox. A recycled
//! slot is never confused with the snapshot: the new instance has a newer generation, so the old id keeps reading
//! the snapshot and the new id the new instance. [`DataModel::contains`] stays `false` for the old id.
//!
//! # What a pin costs
//!
//! A live instance's count is one byte in its slot, in what was padding, so it does not grow the slot. The rare
//! instance with more than 255 holders counts the rest in a side table. Pinning and unpinning a live instance never
//! allocates below that.

use std::sync::atomic::{AtomicU64, Ordering};

use super::storage::Body;
use super::{DataModel, DmError};
use crate::registry::hash::FxMap;
use crate::{ClassId, Id};

/// The last state of a destroyed instance.
#[derive(Debug)]
pub(super) struct Snapshot {
    /// Its class.
    pub(super) class: ClassId,
    /// Its name, stored values and attributes as they were at the end; never any children.
    pub(super) body: Body,
    /// Pins still held on it.
    pins: u32,
    /// Whether it is still in its grave window (its events may be being delivered).
    grave: bool,
}

/// Every snapshot, and the pin counts that do not fit a slot.
#[derive(Debug, Default)]
pub(super) struct Pins {
    snapshots: FxMap<Id, Snapshot>,
    /// Pins of live instances beyond the slot's `u8::MAX`.
    pub(super) overflow: FxMap<Id, u32>,
    /// Snapshots kept for event delivery and the drain count when each was made, oldest first.
    graves: Vec<(Id, u64)>,
}

/// What a read sees of an instance: its class and body, live or from its snapshot.
#[derive(Clone, Copy)]
pub(super) struct View<'a> {
    /// The class.
    pub(super) class: ClassId,
    /// The body.
    pub(super) body: &'a Body,
}

/// The next data model's [`DataModel::uid`]. Starts at 1, so 0 can mean "none".
static NEXT_UID: AtomicU64 = AtomicU64::new(1);

/// A number no other data model of this process has.
pub(super) fn next_uid() -> u64 {
    NEXT_UID.fetch_add(1, Ordering::Relaxed)
}

impl DataModel {
    /// A number that no other data model made by this process has, for holders that pin instances (a script
    /// backend) and must give each pin back to the data model it came from.
    pub fn uid(&self) -> u64 {
        self.uid
    }

    /// Pins the instance for an outside holder, such as a script value that refers to it. While it is pinned, a
    /// destroyed instance stays readable (see [`DataModel::is_destroyed`]); each pin is given back with
    /// [`DataModel::unpin`].
    ///
    /// Pinning a destroyed instance that is still readable pins its snapshot. Returns whether the pin was taken:
    /// `false` for an id that names neither a live instance nor a readable destroyed one, which needs no unpin.
    pub fn pin(&mut self, id: Id) -> bool {
        if let Some(slot) = self.slots.get_mut(id) {
            if slot.pins < u8::MAX {
                slot.pins += 1;
            } else {
                let extra = self.pins.overflow.entry(id).or_insert(0);
                *extra = extra.saturating_add(1);
            }
            return true;
        }
        match self.pins.snapshots.get_mut(&id) {
            Some(snapshot) => {
                snapshot.pins = snapshot.pins.saturating_add(1);
                true
            }
            None => false,
        }
    }

    /// Gives back one pin taken with [`DataModel::pin`]. A destroyed instance whose last pin goes (and whose events
    /// have been delivered) stops being readable, and its snapshot is freed. Returns whether there was a pin to give
    /// back.
    pub fn unpin(&mut self, id: Id) -> bool {
        if let Some(slot) = self.slots.get_mut(id) {
            if slot.pins == u8::MAX
                && let Some(extra) = self.pins.overflow.get_mut(&id)
            {
                *extra -= 1;
                if *extra == 0 {
                    self.pins.overflow.remove(&id);
                }
                return true;
            }
            if slot.pins == 0 {
                return false;
            }
            slot.pins -= 1;
            return true;
        }
        let Some(snapshot) = self.pins.snapshots.get_mut(&id) else {
            return false;
        };
        if snapshot.pins == 0 {
            return false;
        }
        snapshot.pins -= 1;
        if snapshot.pins == 0 && !snapshot.grave {
            self.pins.snapshots.remove(&id);
        }
        true
    }

    /// Whether `id` names a destroyed instance that can still be read (it is pinned, or its events are being
    /// delivered). Such an id answers reads and refuses writes with [`DmError::Destroyed`].
    pub fn is_destroyed(&self, id: Id) -> bool {
        !self.slots.contains(id) && self.pins.snapshots.contains_key(&id)
    }

    /// How many destroyed instances can still be read: the snapshots the data model holds.
    pub fn snapshot_count(&self) -> usize {
        self.pins.snapshots.len()
    }

    /// The class and body of `id`, live or destroyed but still readable.
    #[inline]
    pub(super) fn view(&self, id: Id) -> Result<View<'_>, DmError> {
        match self.slots.get(id) {
            Some(slot) => match &slot.body {
                Some(body) => Ok(View {
                    class: slot.class,
                    body,
                }),
                None => Err(DmError::NoInstance(id)),
            },
            None => self.snapshot_view(id),
        }
    }

    /// The [`View`] of a destroyed instance, or [`DmError::NoInstance`].
    #[cold]
    #[inline(never)]
    pub(super) fn snapshot_view(&self, id: Id) -> Result<View<'_>, DmError> {
        self.snapshot(id)
            .map(|s| View {
                class: s.class,
                body: &s.body,
            })
            .ok_or(DmError::NoInstance(id))
    }

    /// The snapshot of the destroyed instance `id`, if it can still be read.
    #[inline]
    pub(super) fn snapshot(&self, id: Id) -> Option<&Snapshot> {
        if self.pins.snapshots.is_empty() {
            return None;
        }
        self.pins.snapshots.get(&id)
    }

    /// Why a call that needs the live instance `id` cannot have it: [`DmError::Destroyed`] when it is destroyed but
    /// still readable, [`DmError::NoInstance`] otherwise.
    #[cold]
    #[inline(never)]
    pub(super) fn not_live(&self, id: Id) -> DmError {
        if self.snapshot(id).is_some() {
            DmError::Destroyed(id)
        } else {
            DmError::NoInstance(id)
        }
    }

    /// Keeps the last state of `id`, whose slot was just freed with `pins` pins on it (the slot's count; the side
    /// table's share is added here). `grave` keeps it until its events are delivered too.
    #[cold]
    #[inline(never)]
    pub(super) fn keep_snapshot(&mut self, id: Id, class: ClassId, mut body: Body, pins: u8, grave: bool) {
        let mut pins = u32::from(pins);
        if pins == u32::from(u8::MAX) {
            pins = pins.saturating_add(self.pins.overflow.remove(&id).unwrap_or(0));
        }
        if pins == 0 && !grave {
            return;
        }
        // The children were destroyed with it; the list would name freed slots.
        body.children = None;
        if grave {
            self.pins.graves.push((id, self.signals_epoch()));
        }
        self.pins.snapshots.insert(
            id,
            Snapshot {
                class,
                body,
                pins,
                grave,
            },
        );
    }

    /// Ends the grave window of the snapshots made before drain `epoch`, freeing those nothing pins. Called when that
    /// drain begins.
    pub(super) fn expire_graves(&mut self, epoch: u64) {
        let Pins { snapshots, graves, .. } = &mut self.pins;
        if graves.is_empty() {
            return;
        }
        graves.retain(|&(id, made)| {
            if made >= epoch {
                return true;
            }
            if let Some(snapshot) = snapshots.get_mut(&id) {
                snapshot.grave = false;
                if snapshot.pins == 0 {
                    snapshots.remove(&id);
                }
            }
            false
        });
    }
}
