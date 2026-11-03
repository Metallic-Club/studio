//! Change tracking: subsystems drain what changed instead of re-reading every instance.
//!
//! Render, physics, the script host, Studio and replication each register a consumer, up to [`MAX_CONSUMERS`]. Every
//! instance keeps one byte of [`DirtyBits`] per consumer; when that byte goes from zero to non-zero the instance's id is pushed onto the
//! consumer's dirty list, so a drain visits only changed instances, each once. Destroyed instances go onto a
//! separate removed list. Draining moves entries into a caller-owned `Vec`, so once the vectors have grown nothing
//! allocates.

use std::fmt;
use std::ops::{BitAnd, BitOr, BitOrAssign};

use super::storage::Dirty;
use super::{DataModel, DmError, MAX_CONSUMERS};
use crate::{Id, PropFlags};

/// What changed about an instance since a consumer last drained it. Combine with `|`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DirtyBits(u8);

impl DirtyBits {
    /// Nothing.
    pub const NONE: Self = Self(0);
    /// A property flagged [`PropFlags::TRANSFORM`] changed.
    pub const TRANSFORM: Self = Self(1 << 0);
    /// A property flagged [`PropFlags::VISUAL`] changed.
    pub const VISUAL: Self = Self(1 << 1);
    /// A property flagged [`PropFlags::PHYSICS`] changed.
    pub const PHYSICS: Self = Self(1 << 2);
    /// The instance or one of its ancestors moved in the tree.
    pub const STRUCTURE: Self = Self(1 << 3);
    /// The instance is new since the last drain (or [`DataModel::mark_all`] asked for everything).
    pub const CREATED: Self = Self(1 << 4);
    /// Any property, the name or an attribute changed. Every write sets it, on top of the kind bits above, so
    /// replication and Studio see changes that neither rendering nor physics care about.
    pub const PROPERTY: Self = Self(1 << 5);
    /// Every bit.
    pub const ALL: Self = Self(0b11_1111);

    const NAMES: [(Self, &'static str); 6] = [
        (Self::TRANSFORM, "TRANSFORM"),
        (Self::VISUAL, "VISUAL"),
        (Self::PHYSICS, "PHYSICS"),
        (Self::STRUCTURE, "STRUCTURE"),
        (Self::CREATED, "CREATED"),
        (Self::PROPERTY, "PROPERTY"),
    ];

    /// The bits a write to a property with `flags` sets.
    pub const fn from_prop_flags(flags: PropFlags) -> Self {
        let mut bits = Self::PROPERTY.0;
        if flags.contains(PropFlags::TRANSFORM) {
            bits |= Self::TRANSFORM.0;
        }
        if flags.contains(PropFlags::VISUAL) {
            bits |= Self::VISUAL.0;
        }
        if flags.contains(PropFlags::PHYSICS) {
            bits |= Self::PHYSICS.0;
        }
        Self(bits)
    }

    /// Whether every bit in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any bit in `other` is set.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether no bit is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }
}

impl BitOr for DirtyBits {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for DirtyBits {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl BitAnd for DirtyBits {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

/// Writes the set bits by name, such as `DirtyBits(TRANSFORM | PROPERTY)`.
impl fmt::Debug for DirtyBits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DirtyBits(")?;
        let mut first = true;
        for (bit, name) in Self::NAMES {
            if self.contains(bit) {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        if first {
            f.write_str("NONE")?;
        }
        f.write_str(")")
    }
}

/// A subsystem registered to drain changes, from [`DataModel::register_consumer`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ConsumerId(u8);

impl ConsumerId {
    /// The consumer's index, below [`MAX_CONSUMERS`].
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// One consumer's queues.
#[derive(Debug, Default)]
struct Consumer {
    /// Only these bits are recorded; `CREATED` always is. `NONE` marks a free entry
    /// ([`DataModel::unregister_consumer`]), which records nothing.
    interest: DirtyBits,
    /// Ids whose dirty byte went from zero to non-zero since the last drain. May hold ids destroyed since.
    dirty: Vec<Id>,
    removed: Vec<Id>,
}

/// Every consumer's queues.
#[derive(Debug, Default)]
pub(super) struct Changes {
    consumers: Vec<Consumer>,
}

impl Changes {
    /// Records `bits` in `dirty`, the bits of the live instance `id`, for every consumer interested in them.
    #[inline]
    pub(super) fn mark(&mut self, dirty: &mut Dirty, id: Id, bits: DirtyBits) {
        for (consumer, dirty) in self.consumers.iter_mut().zip(dirty.iter_mut()) {
            let bits = (bits & consumer.interest).0;
            if bits != 0 {
                if *dirty == 0 {
                    consumer.dirty.push(id);
                }
                *dirty |= bits;
            }
        }
    }

    /// Records that the live instance `id`, whose bits are `dirty`, is being destroyed. A consumer that never
    /// drained the instance's creation is not told: for it, the instance never existed.
    pub(super) fn removed(&mut self, dirty: &Dirty, id: Id) {
        for (consumer, dirty) in self.consumers.iter_mut().zip(dirty.iter()) {
            if dirty & DirtyBits::CREATED.0 == 0 && !consumer.interest.is_empty() {
                consumer.removed.push(id);
            }
        }
    }
}

impl DataModel {
    /// Registers a subsystem that drains changes, recording only the bits in `interest` for it (`CREATED` and
    /// removals are always recorded). At most [`MAX_CONSUMERS`] can be registered at once; the place of one
    /// unregistered since is reused.
    ///
    /// Instances that already exist are not queued; call [`DataModel::mark_all`] to receive them once.
    pub fn register_consumer(&mut self, interest: DirtyBits) -> Result<ConsumerId, DmError> {
        let consumer = Consumer {
            interest: interest | DirtyBits::CREATED,
            ..Consumer::default()
        };
        let consumers = &mut self.changes.consumers;
        if let Some((index, free)) = consumers.iter_mut().enumerate().find(|(_, c)| c.interest.is_empty())
            && let Ok(index) = u8::try_from(index)
        {
            *free = consumer;
            return Ok(ConsumerId(index));
        }
        let index = u8::try_from(consumers.len())
            .ok()
            .filter(|&i| usize::from(i) < MAX_CONSUMERS)
            .ok_or(DmError::TooManyConsumers { max: MAX_CONSUMERS })?;
        consumers.push(consumer);
        Ok(ConsumerId(index))
    }

    /// Unregisters `consumer`, for a subsystem that stops (a script host whose game ends): nothing is recorded for
    /// it any more, what it had not drained is dropped, and its place can be registered again. Its id must not be
    /// used afterwards, since a later registration may get the same one. Does nothing for an id that is not
    /// registered.
    ///
    /// Clears the consumer's byte in every live instance: one pass over their dirty bits.
    pub fn unregister_consumer(&mut self, consumer: ConsumerId) {
        let index = consumer.index();
        let Some(c) = self.changes.consumers.get_mut(index) else {
            return;
        };
        if c.interest.is_empty() {
            return;
        }
        *c = Consumer::default();
        self.slots.for_each_live_dirty_mut(|_, dirty| {
            if let Some(dirty) = dirty.get_mut(index) {
                *dirty = 0;
            }
        });
    }

    /// Appends every instance that changed since `consumer` last drained, with what changed, to `out`, and clears
    /// them. Each instance appears once. Instances destroyed since are skipped: they are in
    /// [`DataModel::drain_removed`] instead.
    ///
    /// Allocation-free once `out` and the internal list have grown to their working size.
    pub fn drain_changes(&mut self, consumer: ConsumerId, out: &mut Vec<(Id, DirtyBits)>) {
        let Some(c) = self.changes.consumers.get_mut(consumer.index()) else {
            return;
        };
        out.reserve(c.dirty.len());
        for id in c.dirty.drain(..) {
            // A destroyed instance's bits are stale: check liveness before reading them.
            if self.slots.contains(id)
                && let Some(dirty) = self.slots.dirty_at_mut(id.index())
                && let Some(dirty) = dirty.get_mut(consumer.index())
                && *dirty != 0
            {
                out.push((id, DirtyBits(*dirty)));
                *dirty = 0;
            }
        }
    }

    /// Appends every instance destroyed since `consumer` last drained to `out`, and clears them. Their ids are
    /// stale by now; use them to drop mirrored state.
    pub fn drain_removed(&mut self, consumer: ConsumerId, out: &mut Vec<Id>) {
        if let Some(c) = self.changes.consumers.get_mut(consumer.index()) {
            out.append(&mut c.removed);
        }
    }

    /// Queues every live instance for `consumer` with `CREATED | STRUCTURE`, for a subsystem that registers after
    /// the scene was built or needs a full resynchronisation.
    pub fn mark_all(&mut self, consumer: ConsumerId) {
        let Some(c) = self.changes.consumers.get_mut(consumer.index()) else {
            return;
        };
        if c.interest.is_empty() {
            return;
        }
        let bits = (DirtyBits::CREATED | DirtyBits::STRUCTURE) & c.interest;
        c.dirty.reserve(self.slots.live());
        let ids = &mut c.dirty;
        self.slots.for_each_live_dirty_mut(|id, dirty| {
            if let Some(dirty) = dirty.get_mut(consumer.index()) {
                if *dirty == 0 {
                    ids.push(id);
                }
                *dirty |= bits.0;
            }
        });
    }

    /// Counts structural changes (creation, re-parenting, destruction). A subsystem that caches something derived
    /// from the whole tree can compare it instead of draining.
    pub fn structure_revision(&self) -> u64 {
        self.structure_revision
    }
}
