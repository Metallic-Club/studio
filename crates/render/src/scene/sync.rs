//! [`InstanceScene`]: the CPU side of the GPU scene. Drains the data model's changes into packed slots.

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, ConsumerId, DataModel, DirtyBits, DmError, Id};

use crate::cull::{Casters, CullStats, CullView, VisibleSet};

use super::components::{ClassBinding, ClassTable, MeshInput, RenderComponents};
use super::grid::{Grid, bounding_sphere};
use super::instance::{GpuInstance, MeshKind, affine_rows, pack_color, transparency_flags};
use super::lists::{DrawLists, NOT_LISTED, entry, entry_slot, list_of};
use super::slots::RegionSlots;
use super::{
    DEMOTE_REST, MAX_SHADOW_CHANGES, MAX_UPLOAD_GAP, ORIGIN_GRID, PROMOTE_MOVES_ANCHORED, PROMOTE_MOVES_UNANCHORED,
    PROMOTE_WINDOW, REST_CHECKS_PER_FRAME, Region, SHADOW_FLAGS, SlotRef, SyncStats, UploadRange, is_shadow_caster,
};

/// What the scene asks the data model to record for it (removals and `CREATED` are always recorded).
fn interest() -> DirtyBits {
    DirtyBits::TRANSFORM | DirtyBits::VISUAL | DirtyBits::CREATED | DirtyBits::STRUCTURE
}

/// Where an instance index of the data model has its slot, if anywhere.
#[derive(Clone, Copy, Debug)]
struct Entry {
    generation: u32,
    region: Region,
    /// [`Entry::NONE`] when the instance has no slot.
    slot: u32,
}

impl Entry {
    const NONE: u32 = u32::MAX;
    const EMPTY: Self = Self {
        generation: 0,
        region: Region::Static,
        slot: Self::NONE,
    };
}

/// The CPU side of the GPU-resident scene: which instance has which slot, and the packed instances themselves.
/// See the [module documentation](super) for the design.
///
/// Attach it to a data model with [`InstanceScene::attach`], then call [`InstanceScene::sync`] once per frame and
/// [`InstanceScene::coalesce`] before uploading (which [`super::GpuScene`] does). Once its vectors have grown to
/// the scene's working size, a frame allocates nothing.
#[derive(Debug)]
pub struct InstanceScene {
    consumer: ConsumerId,
    /// The data model this scene is attached to ([`DataModel::uid`]).
    dm_uid: u64,
    /// Only descendants of this instance are drawn (`Workspace`); `None` draws every renderable instance.
    world: Option<Id>,
    classes: ClassTable,
    /// Indexed by [`Id::index`].
    entries: Vec<Entry>,
    regions: [RegionSlots; 2],
    origin: Vec3,
    /// Counts syncs; the static/dynamic heuristic's clock.
    frame: u32,
    /// Where the next rest check in the dynamic region starts.
    rest_cursor: u32,
    /// Reused drain buffers.
    changes: Vec<(Id, DirtyBits)>,
    removed: Vec<Id>,
    /// The ranges [`InstanceScene::coalesce`] produced.
    uploads: Vec<UploadRange>,
    /// Which slots each draw covers (see [`super::lists`]).
    lists: DrawLists,
    /// Every listed instance's bounding sphere, for culling (see [`super::grid`]).
    grid: Grid,
    /// Changes whenever a coalesce changed any slot, so a view's cull can be kept while nothing changed.
    revision: u64,
    /// Instances that are not drawn but whose changes [`InstanceScene::sync`] reports (`Lighting`).
    watched: Vec<Id>,
    /// Static shadow casters that changed (see the module documentation).
    shadow: ShadowChanges,
}

/// The static region's shadow-caster changes since they were last taken: bounding spheres, origin-relative.
#[derive(Debug, Default)]
struct ShadowChanges {
    spheres: Vec<(Vec3, f32)>,
    /// Bumped when everything must count as changed.
    epoch: u64,
}

impl ShadowChanges {
    #[inline]
    fn note(&mut self, instance: &GpuInstance) {
        if !is_shadow_caster(instance) {
            return;
        }
        if self.spheres.len() >= MAX_SHADOW_CHANGES {
            self.everything();
        } else {
            self.spheres.push(bounding_sphere(instance));
        }
    }

    /// Everything changed: the epoch moves and the notes so far say nothing more.
    fn everything(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.spheres.clear();
    }
}

impl InstanceScene {
    /// Registers a change consumer on `dm` and queues every existing instance, so the first
    /// [`InstanceScene::sync`] places the whole scene. Only descendants of `world` are drawn (pass `Workspace`);
    /// with `None` every renderable instance is, wherever it is.
    ///
    /// Fails when `dm` has no free change consumer.
    pub fn attach(dm: &mut DataModel, world: Option<Id>, components: &RenderComponents) -> Result<Self, DmError> {
        let consumer = dm.register_consumer(interest())?;
        dm.mark_all(consumer);
        Ok(Self {
            consumer,
            dm_uid: dm.uid(),
            world,
            classes: ClassTable::new(dm.registry(), components),
            entries: Vec::new(),
            regions: Default::default(),
            origin: Vec3::ZERO,
            frame: 0,
            rest_cursor: 0,
            changes: Vec::new(),
            removed: Vec::new(),
            uploads: Vec::new(),
            lists: DrawLists::default(),
            grid: Grid::default(),
            revision: 0,
            watched: Vec::new(),
            shadow: ShadowChanges::default(),
        })
    }

    /// Reports `id`'s changes from now on: each [`InstanceScene::sync`] counts it in [`SyncStats::watched`] when
    /// one of its visual properties, its place in the tree or its existence changed since the last. For instances
    /// that are not drawn but say how to draw (the `Lighting` service), so they are read again only when they
    /// change, through the change consumer the scene already drains. A sync right after attaching (or
    /// [`InstanceScene::rebuild_all`]) reports every watched instance. Watching the same instance twice does
    /// nothing.
    pub fn watch(&mut self, id: Id) {
        if !self.watched.contains(&id) {
            self.watched.push(id);
        }
    }

    /// Unregisters the change consumer from `dm` (the data model it was attached to; another is left alone).
    pub fn detach(self, dm: &mut DataModel) {
        if self.is_attached_to(dm) {
            dm.unregister_consumer(self.consumer);
        }
    }

    /// Whether this scene mirrors `dm`.
    pub fn is_attached_to(&self, dm: &DataModel) -> bool {
        self.dm_uid == dm.uid()
    }

    /// Forgets every slot and queues every instance again, so the next [`InstanceScene::sync`] rebuilds the
    /// scene from scratch. The one full pass; nothing calls it per frame.
    pub fn rebuild_all(&mut self, dm: &mut DataModel) {
        if !self.is_attached_to(dm) {
            return;
        }
        for region in &mut self.regions {
            region.clear();
        }
        self.entries.fill(Entry::EMPTY);
        self.uploads.clear();
        self.lists.clear();
        self.grid.clear();
        self.revision = self.revision.wrapping_add(1);
        self.shadow.everything();
        self.rest_cursor = 0;
        dm.mark_all(self.consumer);
    }

    /// Applies what changed in `dm` since the last sync: removed instances free their slots, new ones (in the
    /// world) get one, changed ones are repacked in place, and the static/dynamic heuristic runs. Only the
    /// instances the change consumer names are touched. Does nothing when `dm` is not the data model the scene
    /// was attached to.
    pub fn sync(&mut self, dm: &mut DataModel) -> SyncStats {
        let mut stats = SyncStats::default();
        if !self.is_attached_to(dm) {
            return stats;
        }
        self.frame = self.frame.wrapping_add(1);

        dm.drain_removed(self.consumer, &mut self.removed);
        let removed = std::mem::take(&mut self.removed);
        for &id in &removed {
            if let Some(slot) = self.lookup(id) {
                self.release(id, slot);
                stats.removed += 1;
            }
        }
        self.removed = removed;
        self.removed.clear();

        dm.drain_changes(self.consumer, &mut self.changes);
        let changes = std::mem::take(&mut self.changes);
        for &(id, bits) in &changes {
            self.apply(dm, id, bits, &mut stats);
        }
        self.changes = changes;
        self.changes.clear();

        self.demote_resting(&mut stats);
        stats
    }

    /// Moves the scene origin near `camera` when the camera is more than [`ORIGIN_GRID`] metres from it (see the
    /// module documentation), recomputing every live instance's translation. Returns whether it moved.
    pub fn set_camera_position(&mut self, dm: &DataModel, camera: Vec3) -> bool {
        if !camera.is_finite() || (camera - self.origin).abs().max_element() <= ORIGIN_GRID {
            return false;
        }
        self.origin = (camera / ORIGIN_GRID).round() * ORIGIN_GRID;
        // Every cached shadow was drawn in the old coordinates.
        self.shadow.everything();
        for region in Region::ALL {
            for slot in 0..self.regions[region.index()].len() {
                let owner = self.regions[region.index()]
                    .meta
                    .get(slot as usize)
                    .and_then(|m| m.owner);
                let Some(owner) = owner else { continue };
                let cframe = self.binding(dm, owner).map_or(CFrame::IDENTITY, |binding| {
                    dm.cframe(owner, binding.props.cframe).unwrap_or(CFrame::IDENTITY)
                });
                let translation = cframe.position() - self.origin;
                let slots = &mut self.regions[region.index()];
                if let Some(instance) = slots.instances.get_mut(slot as usize) {
                    for (row, t) in instance.rows.iter_mut().zip(translation.to_array()) {
                        row[3] = t;
                    }
                }
                if slots.mark_moved(slot) {
                    self.grid.queue(entry(region, slot));
                }
                slots.mark_dirty(slot);
            }
        }
        true
    }

    /// The point translations are stored relative to. The shader adds `camera − origin`.
    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    /// The instance's slot, if it is drawn.
    pub fn slot_of(&self, id: Id) -> Option<SlotRef> {
        let entry = self.entries.get(id.index() as usize)?;
        (entry.slot != Entry::NONE && entry.generation == id.generation()).then_some(SlotRef {
            region: entry.region,
            slot: entry.slot,
        })
    }

    /// The packed instances of `region`, one per slot (free slots are [`GpuInstance::FREE`]). Its length is the
    /// number of slots in use, free ones included: draw (or cull) that many.
    pub fn instances(&self, region: Region) -> &[GpuInstance] {
        &self.regions[region.index()].instances
    }

    /// The instance in each slot of `region`, `None` for a free slot.
    pub fn owners(&self, region: Region) -> impl Iterator<Item = Option<Id>> + '_ {
        self.regions[region.index()].meta.iter().map(|m| m.owner)
    }

    /// How many instances are drawn from `region`.
    pub fn live(&self, region: Region) -> u32 {
        self.regions[region.index()].live()
    }

    /// Slots waiting for upload in `region`.
    pub fn dirty(&self, region: Region) -> usize {
        self.regions[region.index()].dirty_len()
    }

    /// Turns every dirty slot into upload ranges ([`InstanceScene::uploads`]) and clears the dirty lists. Within a
    /// region the ranges are sorted, adjacent slots share a range, and runs separated by at most
    /// [`MAX_UPLOAD_GAP`] clean slots are merged.
    ///
    /// The draw lists ([`InstanceScene::draw_list`]) are brought up to date first, from the same dirty slots, and
    /// slots that changed list are placed in the culling grid (moved ones wait for the next cull).
    pub fn coalesce(&mut self) {
        if Region::ALL.iter().any(|r| self.regions[r.index()].dirty_len() > 0) {
            self.revision = self.revision.wrapping_add(1);
        }
        self.update_lists();
        self.uploads.clear();
        for region in Region::ALL {
            self.regions[region.index()].coalesce(region, MAX_UPLOAD_GAP, &mut self.uploads);
        }
    }

    /// The ranges the last [`InstanceScene::coalesce`] produced.
    pub fn uploads(&self) -> &[UploadRange] {
        &self.uploads
    }

    /// Changes whenever a [`InstanceScene::coalesce`] changed any slot (or the scene was rebuilt): while it stays
    /// the same, a cull for the same view gives the same result.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Writes the instances `view` sees into `out`, as of the last [`InstanceScene::coalesce`]: per mesh kind the
    /// visible opaque entries, roughly front to back, and the visible see-through entries (see [`crate::cull`]).
    /// Allocates nothing once `out` has grown to the scene's lists.
    ///
    /// Instances that moved since the last cull are moved in the culling grid first (see [`super::grid`]), so
    /// that upkeep is part of the cull's time; further views of the same frame find nothing left to move.
    pub fn cull(&mut self, view: &CullView, out: &mut VisibleSet) -> CullStats {
        let start = std::time::Instant::now();
        self.apply_moves();
        let capacities = self.lists.opaque.iter().map(|list| list.entries.len());
        let mut stats = self
            .grid
            .cull(view, capacities, self.lists.transparent.entries.len(), out);
        stats.cpu_time = start.elapsed();
        out.stats = stats;
        stats
    }

    /// Like [`InstanceScene::cull`], for a shadow view (a cascade, [`crate::shadow`]): writes the opaque shadow
    /// casters `view` holds ([`super::is_shadow_caster`]), of both regions or the static one only, into `out`'s
    /// opaque lists, roughly nearest the light first when `view` looks along the light. The see-through list stays
    /// empty. Allocates nothing once `out` has grown to the scene's lists.
    pub fn cull_casters(&mut self, view: &CullView, casters: Casters, out: &mut VisibleSet) -> CullStats {
        let start = std::time::Instant::now();
        self.apply_moves();
        let capacities = self.lists.opaque.iter().map(|list| list.entries.len());
        let mut stats = self.grid.cull_casters(view, casters, capacities, out);
        stats.cpu_time = start.elapsed();
        out.stats = stats;
        stats
    }

    /// The bounding spheres (origin-relative) of the static shadow casters that changed since
    /// [`InstanceScene::clear_shadow_changes`]: where each one was and is, if it casts (see the module
    /// documentation). Only meaningful together with [`InstanceScene::shadow_epoch`].
    pub fn shadow_changes(&self) -> &[(Vec3, f32)] {
        &self.shadow.spheres
    }

    /// Changes whenever every shadow must count as changed: the scene was rebuilt, its origin moved, or more than
    /// [`MAX_SHADOW_CHANGES`] changes piled up.
    pub fn shadow_epoch(&self) -> u64 {
        self.shadow.epoch
    }

    /// Forgets the noted shadow-caster changes (they have been looked at). Keeps the memory.
    pub fn clear_shadow_changes(&mut self) {
        self.shadow.spheres.clear();
    }

    /// Instances in the culling grid: every one on a draw list.
    pub fn culling_items(&self) -> usize {
        self.grid.len()
    }

    /// Instances queued to move in the culling grid at the next cull.
    #[cfg(test)]
    pub(super) fn culling_queue(&self) -> usize {
        self.grid.queued()
    }

    /// Moves the queued instances' items in the culling grid to their current bounding spheres.
    fn apply_moves(&mut self) {
        let queued = self.grid.take_queue();
        for &moved in &queued {
            let (region, slot) = entry_slot(moved);
            let slots = &mut self.regions[region.index()];
            let (Some(instance), Some(meta)) = (slots.instances.get(slot as usize), slots.meta.get_mut(slot as usize))
            else {
                continue;
            };
            if !meta.moved {
                continue;
            }
            meta.moved = false;
            self.grid.update(region, slot, instance, meta.list);
        }
        self.grid.return_queue(queued);
    }

    /// The entries drawn as opaque instances of mesh `kind`, in no particular order: one per live slot whose
    /// instance is solid (see [`super::lists`] for the entry format). Current as of the last
    /// [`InstanceScene::coalesce`].
    pub fn draw_list(&self, kind: MeshKind) -> &[u32] {
        self.lists
            .opaque
            .get(usize::from(kind.0))
            .map_or(&[], |list| list.entries.as_slice())
    }

    /// How many opaque draw lists there are: one per mesh kind up to the highest kind drawn so far.
    pub fn draw_list_count(&self) -> usize {
        self.lists.opaque.len()
    }

    /// The entries of every partly see-through instance, unsorted (the renderer sorts them per view).
    pub fn transparent_list(&self) -> &[u32] {
        &self.lists.transparent.entries
    }

    /// Changes whenever [`InstanceScene::transparent_list`] or one of its instances changes, so a view knows when
    /// to sort again.
    pub fn transparent_revision(&self) -> u64 {
        self.lists.transparent_revision
    }

    /// The packed instance an entry names.
    pub fn entry_instance(&self, entry: u32) -> Option<&GpuInstance> {
        let (region, slot) = entry_slot(entry);
        self.regions[region.index()].instances.get(slot as usize)
    }

    /// The `Material` enum's items: the material table's rows, in order. Empty when the game has no such enum.
    pub fn material_names(&self) -> &'static [&'static str] {
        self.classes.materials()
    }

    /// Makes the draw lists also record changed instances (not only changed entries), for lists that hold copies
    /// of the instances (the vertex-buffer fallback).
    pub(crate) fn set_gather_lists(&mut self, gather: bool) {
        self.lists.set_gather(gather);
    }

    /// The opaque lists, for their upload.
    pub(crate) fn lists_mut(&mut self) -> &mut DrawLists {
        &mut self.lists
    }

    /// Puts every dirty slot's entry in the list its packed instance belongs in (see [`super::lists`]).
    fn update_lists(&mut self) {
        for region in Region::ALL {
            let dirty = self.regions[region.index()].take_dirty();
            for &slot in &dirty {
                self.relist_slot(region, slot);
            }
            self.regions[region.index()].return_dirty(dirty);
        }
    }

    fn relist_slot(&mut self, region: Region, slot: u32) {
        let slots = &self.regions[region.index()];
        let (Some(instance), Some(meta)) = (slots.instances.get(slot as usize), slots.meta.get(slot as usize)) else {
            return;
        };
        let want = list_of(instance);
        let (have, pos) = (meta.list, meta.list_pos);
        if want == have {
            if have != NOT_LISTED {
                self.lists.changed(have, pos);
            }
            return;
        }
        // A slot that changes list enters, leaves or changes list in the culling grid now; one that only moved
        // waits on the grid's queue for the next cull. (A queued slot stays queued: it is on the queue once, and
        // the update there finds nothing left to do.)
        self.grid.update(region, slot, instance, want);
        if have != NOT_LISTED
            && let Some(moved) = self.lists.remove(have, pos)
        {
            let (moved_region, moved_slot) = entry_slot(moved);
            if let Some(m) = self.regions[moved_region.index()].meta.get_mut(moved_slot as usize) {
                m.list_pos = pos;
            }
        }
        let pos = if want == NOT_LISTED {
            0
        } else {
            self.lists.push(want, entry(region, slot))
        };
        if let Some(meta) = self.regions[region.index()].meta.get_mut(slot as usize) {
            meta.list = want;
            meta.list_pos = pos;
        }
    }

    /// The live slot of `id`, dropping a slot left behind by an earlier instance with the same index.
    fn lookup(&mut self, id: Id) -> Option<SlotRef> {
        let entry = *self.entries.get(id.index() as usize)?;
        if entry.slot == Entry::NONE {
            return None;
        }
        let slot = SlotRef {
            region: entry.region,
            slot: entry.slot,
        };
        if entry.generation == id.generation() {
            return Some(slot);
        }
        // Not reachable through the data model's drains (removals are applied before changes), kept so a stale
        // slot can never be drawn for a new instance.
        self.release_slot(id.index(), slot);
        None
    }

    fn binding(&self, dm: &DataModel, id: Id) -> Option<ClassBinding> {
        self.classes.get(dm.class_of(id).ok()?).copied()
    }

    fn apply(&mut self, dm: &DataModel, id: Id, bits: DirtyBits, stats: &mut SyncStats) {
        let Some(binding) = self.binding(dm, id) else {
            // Only instances that are not drawn get here, so drawn ones never pay for the watch list.
            if self.watched.contains(&id) {
                stats.watched += 1;
            }
            return;
        };
        let current = self.lookup(id);
        if bits.intersects(DirtyBits::CREATED | DirtyBits::STRUCTURE) {
            let in_world = match self.world {
                Some(world) => dm.is_descendant_of(id, world).unwrap_or(false),
                None => true,
            };
            match (in_world, current) {
                (false, Some(slot)) => {
                    self.release(id, slot);
                    stats.removed += 1;
                }
                (false, None) => {}
                (true, None) => {
                    let slot = self.place(id, Region::Static);
                    self.write(dm, id, slot, &binding, true);
                    stats.added += 1;
                }
                (true, Some(slot)) => {
                    self.write(dm, id, slot, &binding, true);
                    stats.updated += 1;
                }
            }
            return;
        }
        let Some(mut slot) = current else {
            return;
        };
        let transform = bits.contains(DirtyBits::TRANSFORM);
        if transform {
            slot = self.track_motion(dm, id, slot, &binding, stats);
        }
        self.write(dm, id, slot, &binding, transform);
        stats.updated += 1;
    }

    /// Counts a transform change towards promotion (static region) or as activity (dynamic region). Returns the
    /// slot the instance is in afterwards.
    fn track_motion(
        &mut self,
        dm: &DataModel,
        id: Id,
        slot: SlotRef,
        binding: &ClassBinding,
        stats: &mut SyncStats,
    ) -> SlotRef {
        let frame = self.frame;
        let Some(meta) = self.regions[slot.region.index()].meta.get_mut(slot.slot as usize) else {
            return slot;
        };
        match slot.region {
            Region::Dynamic => {
                meta.since = frame;
                slot
            }
            Region::Static => {
                if frame.wrapping_sub(meta.since) >= PROMOTE_WINDOW {
                    meta.since = frame;
                    meta.moves = 0;
                }
                meta.moves = meta.moves.saturating_add(1);
                let moves = meta.moves;
                // Read only once the instance could be promoted: most static instances never get here.
                if moves < PROMOTE_MOVES_UNANCHORED {
                    return slot;
                }
                let anchored = binding.props.anchored.is_some_and(|p| dm.bool(id, p).unwrap_or(false));
                let needed = if anchored {
                    PROMOTE_MOVES_ANCHORED
                } else {
                    PROMOTE_MOVES_UNANCHORED
                };
                if moves < needed {
                    return slot;
                }
                stats.promoted += 1;
                self.migrate(id, slot, Region::Dynamic)
            }
        }
    }

    /// Checks up to [`REST_CHECKS_PER_FRAME`] dynamic slots, round-robin, and moves those at rest back to the
    /// static region. Their packed instance is copied as it is: nothing about it changed.
    fn demote_resting(&mut self, stats: &mut SyncStats) {
        let len = self.regions[Region::Dynamic.index()].len();
        if self.regions[Region::Dynamic.index()].live() == 0 {
            self.rest_cursor = 0;
            return;
        }
        let checks = REST_CHECKS_PER_FRAME.min(len);
        for _ in 0..checks {
            if self.rest_cursor >= len {
                self.rest_cursor = 0;
            }
            let slot = self.rest_cursor;
            self.rest_cursor += 1;
            let dynamic = &self.regions[Region::Dynamic.index()];
            let Some(meta) = dynamic.meta.get(slot as usize) else {
                continue;
            };
            let Some(owner) = meta.owner else { continue };
            if self.frame.wrapping_sub(meta.since) < DEMOTE_REST {
                continue;
            }
            let instance = dynamic
                .instances
                .get(slot as usize)
                .copied()
                .unwrap_or(GpuInstance::FREE);
            let from = SlotRef {
                region: Region::Dynamic,
                slot,
            };
            let to = self.migrate(owner, from, Region::Static);
            let statics = &mut self.regions[Region::Static.index()];
            if let Some(target) = statics.instances.get_mut(to.slot as usize) {
                *target = instance;
            }
            statics.mark_dirty(to.slot);
            // A static caster now: cached shadows that hold the static region get it.
            self.shadow.note(&instance);
            stats.demoted += 1;
        }
    }

    /// Moves `id` from `from` to a new slot in `to`; the caller writes the instance there.
    fn migrate(&mut self, id: Id, from: SlotRef, to: Region) -> SlotRef {
        if from.region == Region::Static
            && let Some(instance) = self.regions[from.region.index()].instances.get(from.slot as usize)
        {
            // Promoted: its shadow leaves the cached cascades, which hold the static region only.
            self.shadow.note(instance);
        }
        self.regions[from.region.index()].release(from.slot);
        self.place(id, to)
    }

    /// A new slot for `id` in `region`, recorded in the entry table.
    fn place(&mut self, id: Id, region: Region) -> SlotRef {
        let slot = self.regions[region.index()].alloc(id, self.frame);
        let index = id.index() as usize;
        if index >= self.entries.len() {
            // Grows only when the data model has grown: never in a steady frame.
            self.entries.resize(index + 1, Entry::EMPTY);
        }
        if let Some(entry) = self.entries.get_mut(index) {
            *entry = Entry {
                generation: id.generation(),
                region,
                slot,
            };
        }
        SlotRef { region, slot }
    }

    fn release(&mut self, id: Id, slot: SlotRef) {
        self.release_slot(id.index(), slot);
    }

    fn release_slot(&mut self, index: u32, slot: SlotRef) {
        if slot.region == Region::Static
            && let Some(instance) = self.regions[slot.region.index()].instances.get(slot.slot as usize)
        {
            // Its shadow leaves the cached cascades (removed, or promoted to the dynamic region).
            self.shadow.note(instance);
        }
        self.regions[slot.region.index()].release(slot.slot);
        if let Some(entry) = self.entries.get_mut(index as usize) {
            *entry = Entry::EMPTY;
        }
    }

    /// Packs `id` into `slot` from its current properties and queues the slot for upload. `transform` says whether
    /// the change record names a transform property (`CFrame`, `Position`, `Orientation`, `Size`: the `TRANSFORM`
    /// dirty bit), which moves the bounding sphere for the culling grid.
    fn write(&mut self, dm: &DataModel, id: Id, slot: SlotRef, binding: &ClassBinding, transform: bool) {
        let instance = pack(dm, id, binding, &self.classes, self.origin);
        let region = &mut self.regions[slot.region.index()];
        let mut moved = transform;
        if let Some(target) = region.instances.get_mut(slot.slot as usize) {
            // Besides the transform, only the mesh kind's own scale rule changes the box (a `Shape` change, which
            // always changes the kind); a recolour or a new material leaves the culling grid alone. Its item also
            // says whether it casts a shadow, so `CastShadow` updates it too.
            moved |= target.mesh_material >> 16 != instance.mesh_material >> 16;
            let shadow_flags = (target.flags ^ instance.flags) & SHADOW_FLAGS;
            moved |= shadow_flags & GpuInstance::CAST_SHADOW != 0;
            if slot.region == Region::Static && (moved || shadow_flags != 0) {
                // Its shadow may have changed: where it was and where it is now (see the module documentation).
                self.shadow.note(target);
                self.shadow.note(&instance);
            }
            *target = instance;
        }
        if moved && region.mark_moved(slot.slot) {
            self.grid.queue(entry(slot.region, slot.slot));
        }
        region.mark_dirty(slot.slot);
    }
}

/// The packed instance of `id`, read from the data model. Properties that cannot be read draw with defaults.
fn pack(dm: &DataModel, id: Id, binding: &ClassBinding, classes: &ClassTable, origin: Vec3) -> GpuInstance {
    let props = &binding.props;
    let cframe = dm.cframe(id, props.cframe).unwrap_or(CFrame::IDENTITY);
    let size = props.size.and_then(|p| dm.vector3(id, p).ok()).unwrap_or(Vec3::ONE);
    let (mesh, scale) = (binding.mesh)(&MeshInput { dm, id, props, size });
    let color = props.color.and_then(|p| dm.color3(id, p).ok()).unwrap_or(Color3::WHITE);
    let transparency = props
        .transparency
        .and_then(|p| dm.number(id, p).ok())
        .map_or(0.0, |t| t.clamp(0.0, 1.0) as f32);
    let material = props
        .material
        .and_then(|p| dm.enum_item(id, p).ok())
        .map_or(0, |item| classes.material_row(item.name()));
    let cast_shadow = props.cast_shadow.is_none_or(|p| dm.bool(id, p).unwrap_or(true));
    let mut flags = GpuInstance::LIVE | transparency_flags(transparency);
    if cast_shadow {
        flags |= GpuInstance::CAST_SHADOW;
    }
    GpuInstance {
        rows: affine_rows(cframe, scale, origin),
        color: pack_color(color),
        transparency,
        mesh_material: (u32::from(mesh.0) << 16) | u32::from(material),
        flags,
    }
}
