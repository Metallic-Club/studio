//! The spatial structure culling walks (ADR 0008 §3): a loose grid of every drawn instance's bounding sphere.
//!
//! Each instance on a draw list is an **item** in exactly one place: the grid cell its centre is in (cells are
//! [`CELL_SIZE`] metres on a side, in the scene's origin-relative coordinates, created as instances arrive and kept
//! when they empty), or the **large** list when its sphere is wider than a cell. A cell is *loose*: it bounds
//! its items by its box grown by the largest radius it holds, so an item never has to be split or placed twice.
//! Large items (floors, walls) are tested one by one, against their own box.
//!
//! The grid is kept current from the scene's own changes, never by a scan:
//!
//! - a slot that enters, leaves or changes draw list is placed, removed or relabelled in
//!   [`super::InstanceScene::coalesce`], with the draw lists;
//! - a slot whose packed transform changed is **queued** once (a flag on the slot and an entry here,
//!   [`Grid::queue`]), and the next cull moves its item before it walks the grid. A recolour queues nothing. The
//!   upkeep of moving parts is therefore part of the cull's time and costs nothing in frames that do not cull.
//!
//! Moving an item is O(1): one still inside its cell's box is rewritten in place without working out a cell key,
//! one that crosses into another cell is swap-removed and appended (the item moved into its place is told through
//! its slot). So static and moving instances live in the same structure, and the 1% of parts that move each frame
//! cost a few writes, not a refit.
//!
//! A cull ([`super::InstanceScene::cull`]) tests every non-empty cell's loose box against the view and skips cells
//! outside it (or beyond the view distance). It then takes the cells left and the large items in view nearest
//! first: every item of a cell inside the frustum passes the planes, the items of a cell crossing one are tested
//! one by one, and distance and size are only tested where even the cell's smallest item at its farthest corner
//! would not pass.
//!
//! Each item of a round kind (Ball, Cylinder) also keeps the **level of detail** the camera's view last drew it at
//! ([`crate::lod`]), so the next cull chooses with hysteresis; moving the item keeps it. A cell notes whether it has
//! held such an item since it was last empty, and a cell that has not is culled exactly as without levels, so a scene
//! of blocks pays nothing for them.

use std::collections::HashMap;

use metallic_core::glam::{IVec3, Vec3};

use super::Region;
use super::instance::GpuInstance;
use super::lists::{DYNAMIC_ENTRY, NOT_LISTED, TRANSPARENT_LIST, entry, entry_slot};
use crate::cull::{Casters, Containment, CullStats, CullView, VisibleSet};
use crate::lod::{self, NEW};

/// The side of a grid cell, in metres. Parts are mostly a few metres across; a cell holds a few dozen of them in a
/// dense scene, few enough to test one by one when the cell crosses the view's edge, and the cells of a 1.6 km city
/// are a few thousand box tests.
pub const CELL_SIZE: f32 = 32.0;

/// Items with a larger bounding radius go on the large list (a cell's loose box grows by its largest radius).
pub(super) const LARGE_RADIUS: f32 = CELL_SIZE;

/// A cull's order entries: the index of a cell, or of a large item ([`LARGE`]), for a cell whether its items
/// must be tested against the planes and for distance and size, and whether the view chooses levels of detail for
/// the cell's items (or the large item) at all ([`WITH_LEVELS`]).
const LARGE: u32 = 1 << 31;
const TEST_PLANES: u32 = 1 << 30;
const TEST_SIZE: u32 = 1 << 29;
/// Decided per cell while ordering, so nothing about levels stays live across the pass that lists the items: with
/// the view's two level flags held across it, the item loop ran out of callee-saved registers in the perf gate's
/// LTO'd build and reloaded two of its values from the stack after every listed item (#138, `docs/perf.md`).
const WITH_LEVELS: u32 = 1 << 28;
const INDEX: u32 = WITH_LEVELS - 1;

/// Where a slot's item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Place {
    /// The cell index, [`Place::LARGE`] for the large list, or [`Place::NONE`].
    cell: u32,
    /// The position in the cell's (or the large list's) items.
    pos: u32,
}

impl Place {
    const NONE: u32 = u32::MAX;
    const LARGE: u32 = u32::MAX - 1;
    const EMPTY: Self = Self {
        cell: Self::NONE,
        pos: 0,
    };
}

/// One drawn instance as culling sees it: its bounding sphere, its entry, its list, whether it casts a sun shadow
/// ([`super::is_shadow_caster`]), which shadow views ([`Casters`]) cull on, its shape's class of levels of detail
/// ([`lod::lod_class`]; 0 for kinds with one level), and the level the camera's view last drew it at
/// ([`lod::NEW`] before the first). 24 bytes, as before levels: the class and the caster share a byte.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Item {
    pub(crate) center: Vec3,
    pub(crate) radius: f32,
    pub(crate) entry: u32,
    pub(crate) list: u16,
    /// [`CASTER`], and the class of levels from bit 1 up.
    pub(crate) flags: u8,
    pub(crate) level: u8,
}

/// [`Item::flags`]: the instance casts a sun shadow.
const CASTER: u8 = 1;

impl Item {
    #[inline]
    fn caster(&self) -> bool {
        self.flags & CASTER != 0
    }

    #[inline]
    fn class(&self) -> u8 {
        self.flags >> 1
    }
}

const _: () = assert!(size_of::<Item>() == 24);

/// A large item, with the half extents of its axis-aligned box (much tighter than the sphere for a wall or floor).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Large {
    item: Item,
    half: Vec3,
}

#[derive(Debug)]
struct Cell {
    key: IVec3,
    /// The largest radius of any item the cell has held since it was last empty.
    reach: f32,
    /// No item the cell holds is smaller than this (the smallest since it was last empty; removals leave it).
    smallest: f32,
    /// The cell has held an item with levels of detail since it was last empty (removals leave it).
    levels: bool,
    items: Vec<Item>,
}

/// The loose grid; see the [module documentation](self).
#[derive(Debug, Default)]
pub(crate) struct Grid {
    cells: Vec<Cell>,
    /// Cell index by key. Only looked up when an item enters a cell, never by a cull.
    index: HashMap<IVec3, u32>,
    large: Vec<Large>,
    /// Per region, by slot: where the slot's item is.
    places: [Vec<Place>; 2],
    /// Entries whose transform changed since the last cull ([`Grid::queue`]), moved at the next one.
    queued: Vec<u32>,
}

/// The bounding sphere of a packed instance: the centre of its unit mesh (the translation) and half the diagonal
/// of the stretched, turned unit cube every mesh fits in. The rows' columns are the cube's axes, at right angles
/// (rotation times a per-axis scale), so the diagonal is the root of their squared lengths.
#[inline]
pub(crate) fn bounding_sphere(instance: &GpuInstance) -> (Vec3, f32) {
    let r = &instance.rows;
    let center = Vec3::new(r[0][3], r[1][3], r[2][3]);
    let squared: f32 = r
        .iter()
        .map(|row| row[0] * row[0] + row[1] * row[1] + row[2] * row[2])
        .sum();
    (center, 0.5 * squared.sqrt())
}

/// The half extents of a packed instance's axis-aligned box: per world axis, half the sum of the cube's axes'
/// reach along it.
#[inline]
pub(super) fn half_extents(instance: &GpuInstance) -> Vec3 {
    let r = &instance.rows;
    let reach = |row: &[f32; 4]| 0.5 * (row[0].abs() + row[1].abs() + row[2].abs());
    Vec3::new(reach(&r[0]), reach(&r[1]), reach(&r[2]))
}

pub(super) fn key_of(center: Vec3) -> IVec3 {
    // Saturates far outside any real world; such items are on the large list anyway (not finite) or share an edge
    // cell, which stays correct (the cell's box is computed from its key, and its items from their own spheres).
    (center / CELL_SIZE).floor().as_ivec3()
}

impl Grid {
    /// Forgets every item, keeping the cells and their memory.
    pub(crate) fn clear(&mut self) {
        for cell in &mut self.cells {
            cell.items.clear();
            cell.reach = 0.0;
            cell.smallest = f32::INFINITY;
            cell.levels = false;
        }
        self.large.clear();
        for places in &mut self.places {
            places.fill(Place::EMPTY);
        }
        self.queued.clear();
    }

    /// Queues `entry`, whose packed transform changed, to be moved at the next cull. The caller queues an entry
    /// once until then ([`super::slots::SlotMeta::moved`]).
    #[inline]
    pub(crate) fn queue(&mut self, entry: u32) {
        self.queued.push(entry);
    }

    /// Lends out the queued entries, to move them while changing the grid. Give them back with
    /// [`Grid::return_queue`], which empties them and keeps the memory.
    pub(crate) fn take_queue(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.queued)
    }

    /// How many entries are queued.
    #[cfg(test)]
    pub(super) fn queued(&self) -> usize {
        self.queued.len()
    }

    /// Puts back what [`Grid::take_queue`] lent out, emptied.
    pub(crate) fn return_queue(&mut self, mut queued: Vec<u32>) {
        queued.clear();
        self.queued = queued;
    }

    /// Instances in the grid.
    pub(crate) fn len(&self) -> usize {
        self.cells.iter().map(|c| c.items.len()).sum::<usize>() + self.large.len()
    }

    /// [`Grid::update`] for an item that starts from no level of detail.
    #[cfg(test)]
    pub(crate) fn update_new(&mut self, region: Region, slot: u32, instance: &GpuInstance, list: u16) {
        self.update(region, slot, instance, list, NEW);
    }

    /// The level of detail the camera's view last drew `slot`'s item at ([`NEW`] when it has none).
    pub(crate) fn level(&self, region: Region, slot: u32) -> u8 {
        self.level_at(self.place(region, slot))
    }

    /// Brings `slot`'s item in line with its packed `instance` and the draw `list` it is on ([`NOT_LISTED`]
    /// takes it out). An item that enters the grid starts at level of detail `level` ([`NEW`], or the level of the
    /// slot it came from in another region); one already in it keeps its own.
    // Kept out of line: the sync calls it only when a slot changes list, and inlined it would bloat that hot path.
    #[inline(never)]
    pub(crate) fn update(&mut self, region: Region, slot: u32, instance: &GpuInstance, list: u16, level: u8) {
        let place = self.place(region, slot);
        if list == NOT_LISTED {
            if place.cell != Place::NONE {
                self.remove(place);
                self.set_place(region, slot, Place::EMPTY);
            }
            return;
        }
        let (center, radius) = bounding_sphere(instance);
        let class = lod::lod_class(instance.mesh());
        let caster = list != TRANSPARENT_LIST && super::is_shadow_caster(instance);
        let mut item = Item {
            center,
            radius,
            entry: entry(region, slot),
            list,
            flags: u8::from(caster) | (class << 1),
            level: NEW,
        };
        // The common case for a moving part: still in its cell. Its own box is tested without working out a key,
        // and a sphere that is not finite fails the test and takes the long way.
        if place.cell < Place::LARGE
            && radius <= LARGE_RADIUS
            && let Some(cell) = self.cells.get_mut(place.cell as usize)
        {
            let inside = center - cell.key.as_vec3() * CELL_SIZE;
            if inside.cmpge(Vec3::ZERO).all() && inside.cmplt(Vec3::splat(CELL_SIZE)).all() {
                if radius > cell.reach {
                    cell.reach = radius;
                }
                if radius < cell.smallest {
                    cell.smallest = radius;
                }
                if class != 0 {
                    cell.levels = true;
                }
                if let Some(target) = cell.items.get_mut(place.pos as usize) {
                    // The level it was drawn at stays, for the hysteresis.
                    item.level = target.level;
                    *target = item;
                }
                return;
            }
        }
        item.level = if place.cell == Place::NONE {
            level
        } else {
            self.level_at(place)
        };
        let large = !(radius <= LARGE_RADIUS && center.is_finite());
        if large {
            let large_item = Large {
                item,
                half: half_extents(instance),
            };
            if place.cell == Place::LARGE {
                if let Some(target) = self.large.get_mut(place.pos as usize) {
                    *target = large_item;
                }
                return;
            }
            if place.cell != Place::NONE {
                self.remove(place);
            }
            let pos = u32::try_from(self.large.len()).unwrap_or(u32::MAX);
            self.large.push(large_item);
            self.set_place(
                region,
                slot,
                Place {
                    cell: Place::LARGE,
                    pos,
                },
            );
            return;
        }
        let key = key_of(center);
        if place.cell != Place::NONE
            && place.cell != Place::LARGE
            && let Some(cell) = self.cells.get_mut(place.cell as usize)
            && cell.key == key
        {
            cell.reach = cell.reach.max(radius);
            cell.smallest = cell.smallest.min(radius);
            cell.levels |= class != 0;
            if let Some(target) = cell.items.get_mut(place.pos as usize) {
                *target = item;
            }
            return;
        }
        if place.cell != Place::NONE {
            self.remove(place);
        }
        let index = match self.index.get(&key) {
            Some(&index) => index,
            None => {
                // A cell no item has been in before: grows the grid once, then it stays.
                let index = u32::try_from(self.cells.len()).unwrap_or(u32::MAX);
                self.cells.push(Cell {
                    key,
                    reach: 0.0,
                    smallest: f32::INFINITY,
                    levels: false,
                    items: Vec::new(),
                });
                self.index.insert(key, index);
                index
            }
        };
        let Some(cell) = self.cells.get_mut(index as usize) else {
            return;
        };
        cell.reach = cell.reach.max(radius);
        cell.smallest = cell.smallest.min(radius);
        cell.levels |= class != 0;
        let pos = u32::try_from(cell.items.len()).unwrap_or(u32::MAX);
        cell.items.push(item);
        self.set_place(region, slot, Place { cell: index, pos });
    }

    /// The level the item at `place` was last drawn at; [`NEW`] when there is none.
    fn level_at(&self, place: Place) -> u8 {
        let item = if place.cell == Place::LARGE {
            self.large.get(place.pos as usize).map(|l| &l.item)
        } else {
            self.cells
                .get(place.cell as usize)
                .and_then(|cell| cell.items.get(place.pos as usize))
        };
        item.map_or(NEW, |item| item.level)
    }

    fn place(&self, region: Region, slot: u32) -> Place {
        self.places[region.index()]
            .get(slot as usize)
            .copied()
            .unwrap_or(Place::EMPTY)
    }

    fn set_place(&mut self, region: Region, slot: u32, place: Place) {
        let places = &mut self.places[region.index()];
        if slot as usize >= places.len() {
            // Grows only with the region.
            places.resize(slot as usize + 1, Place::EMPTY);
        }
        if let Some(p) = places.get_mut(slot as usize) {
            *p = place;
        }
    }

    /// Swap-removes the item at `place`, telling the item moved into its position.
    fn remove(&mut self, place: Place) {
        let moved = if place.cell == Place::LARGE {
            swap_remove(&mut self.large, place.pos).map(|l| l.item.entry)
        } else {
            let Some(cell) = self.cells.get_mut(place.cell as usize) else {
                return;
            };
            let moved = swap_remove(&mut cell.items, place.pos).map(|item| item.entry);
            if cell.items.is_empty() {
                cell.reach = 0.0;
                cell.smallest = f32::INFINITY;
                cell.levels = false;
            }
            moved
        };
        if let Some(moved) = moved {
            let (region, slot) = entry_slot(moved);
            self.set_place(region, slot, place);
        }
    }

    /// Writes what `view` sees into `out` (see [`crate::cull`]), returning what the cull did (without its time).
    /// `capacities` is each opaque list's full length and `transparent` the see-through list's, so `out` never
    /// grows during the cull. A camera's view notes the level it draws each round item at, for the next cull's
    /// hysteresis ([`crate::lod`]).
    pub(crate) fn cull(
        &mut self,
        view: &CullView,
        capacities: impl ExactSizeIterator<Item = usize>,
        transparent: usize,
        out: &mut VisibleSet,
    ) -> CullStats {
        self.cull_filtered::<VIEW>(view, capacities, transparent, out)
    }

    /// Like [`Grid::cull`], for a shadow view: only the opaque instances that cast shadows, of both regions or of
    /// the static region alone (see [`Casters`]). The see-through list stays empty. A shadow view's levels need no
    /// memory ([`crate::lod`]), so it changes none.
    pub(crate) fn cull_casters(
        &mut self,
        view: &CullView,
        casters: Casters,
        capacities: impl ExactSizeIterator<Item = usize>,
        out: &mut VisibleSet,
    ) -> CullStats {
        match casters {
            Casters::All => self.cull_filtered::<ALL_CASTERS>(view, capacities, 0, out),
            Casters::Static => self.cull_filtered::<STATIC_CASTERS>(view, capacities, 0, out),
        }
    }

    /// The cull of [`Grid::cull`] and [`Grid::cull_casters`]: `MODE` is [`VIEW`] (every instance) or which casters
    /// a shadow view keeps. A constant, so the camera's cull compiles to the same code as before shadows.
    fn cull_filtered<const MODE: u8>(
        &mut self,
        view: &CullView,
        capacities: impl ExactSizeIterator<Item = usize>,
        transparent: usize,
        out: &mut VisibleSet,
    ) -> CullStats {
        let mut stats = CullStats::default();
        let levels = view.lod.enabled();
        out.begin(capacities, transparent, self.cells.len() + self.large.len(), levels);
        let half_cell = Vec3::splat(CELL_SIZE * 0.5);
        // First the cells and large items in view, with their depth...
        for (index, cell) in (0u32..).zip(&self.cells) {
            let count = u32::try_from(cell.items.len()).unwrap_or(u32::MAX);
            if count == 0 {
                continue;
            }
            stats.cells += 1;
            stats.candidates += count;
            let center = (cell.key.as_vec3() + 0.5) * CELL_SIZE;
            let half = half_cell + cell.reach;
            let containment = view.frustum.classify_box(center, half);
            let offset = (view.eye - center).abs();
            let too_far = (offset - half).max(Vec3::ZERO).length() > view.max_distance;
            if containment == Containment::Outside || too_far {
                stats.cells_outside += 1;
                if too_far {
                    stats.too_far_or_small += count;
                } else {
                    stats.outside_frustum += count;
                }
                continue;
            }
            let mut flags = 0;
            if containment == Containment::Intersects {
                flags |= TEST_PLANES;
            }
            // Every item passes distance and size when even the smallest, at the box's farthest corner, would.
            let farthest = (offset + half).length();
            if farthest > view.max_distance || cell.smallest * view.size_scale < view.min_screen_size * farthest {
                flags |= TEST_SIZE;
            }
            if levels && cell.levels {
                flags |= WITH_LEVELS;
            }
            // Ordered by the cell's own box: its items' centres are in it.
            out.order.push((view.box_key(center, half_cell), index | flags));
        }
        for (index, large) in (0u32..).zip(&self.large) {
            let item = &large.item;
            stats.candidates += 1;
            // A non-finite sphere fails the size test and is culled: the GPU could not place it either.
            if view.frustum.classify_box(item.center, large.half) == Containment::Outside {
                stats.outside_frustum += 1;
            } else if !view.sphere_large_and_near(item.center, item.radius) {
                stats.too_far_or_small += 1;
            } else if keeps::<MODE>(item) {
                let with_levels = if levels && item.class() != 0 { WITH_LEVELS } else { 0 };
                // By the box's nearest point, so a wall is ordered by its near face.
                out.order
                    .push((view.box_key(item.center, large.half), index | LARGE | with_levels));
            }
        }
        // ...then nearest first, their items into the lists: front to back a cell at a time.
        out.order.sort_unstable();
        for i in 0..out.order.len() {
            let (_, place) = out.order[i];
            let index = (place & INDEX) as usize;
            if place & LARGE != 0 {
                if let Some(large) = self.large.get_mut(index) {
                    let item = &mut large.item;
                    if place & WITH_LEVELS != 0 {
                        push_large_with_level(view, item, out);
                    } else {
                        out.push(item.entry, item.list);
                    }
                }
                continue;
            }
            let Some(cell) = self.cells.get_mut(index) else {
                continue;
            };
            let (test_planes, test_size) = (place & TEST_PLANES != 0, place & TEST_SIZE != 0);
            if place & WITH_LEVELS != 0 {
                cull_cell_with_levels::<MODE>(cell, view, (test_planes, test_size), out, &mut stats);
                continue;
            }
            // A cell that has held no round part: exactly the cull without levels.
            for item in &cell.items {
                if !keeps::<MODE>(item) {
                    continue;
                }
                if test_planes && !view.frustum.contains_sphere(item.center, item.radius) {
                    stats.outside_frustum += 1;
                } else if test_size && !view.sphere_large_and_near(item.center, item.radius) {
                    stats.too_far_or_small += 1;
                } else {
                    out.push(item.entry, item.list);
                }
            }
        }
        stats.visible = out.finish();
        stats
    }
}

/// The second pass of [`Grid::cull_filtered`] over a cell that holds round parts: as for any cell, and each visible
/// round item listed at the level `view` draws it at. Out of line, so the loop over the other cells stays as it was
/// before levels of detail.
#[inline(never)]
fn cull_cell_with_levels<const MODE: u8>(
    cell: &mut Cell,
    view: &CullView,
    (test_planes, test_size): (bool, bool),
    out: &mut VisibleSet,
    stats: &mut CullStats,
) {
    let remember = remembers(view);
    for item in &mut cell.items {
        if !keeps::<MODE>(item) {
            continue;
        }
        if test_planes && !view.frustum.contains_sphere(item.center, item.radius) {
            stats.outside_frustum += 1;
        } else if test_size && !view.sphere_large_and_near(item.center, item.radius) {
            stats.too_far_or_small += 1;
        } else if item.class() == 0 {
            out.push(item.entry, item.list);
        } else {
            push_with_level(view, item, remember, out);
        }
    }
}

/// Whether `view` remembers the levels it chooses: a camera's view does, a shadow view's follow from its own
/// distance.
#[inline]
fn remembers(view: &CullView) -> bool {
    view.lod.distance() == 0.0
}

/// [`push_with_level`] for a large round item, out of line like [`cull_cell_with_levels`].
#[inline(never)]
fn push_large_with_level(view: &CullView, item: &mut Item, out: &mut VisibleSet) {
    push_with_level(view, item, remembers(view), out);
}

/// Lists the round `item` at the level `view` draws it at: a camera's view by its distance from the eye, with the
/// hysteresis of the level it last drew it at (and notes the new one, with `remember`); a shadow view by its own
/// distance.
#[inline]
fn push_with_level(view: &CullView, item: &mut Item, remember: bool, out: &mut VisibleSet) {
    let level = if remember {
        let d2 = (item.center - view.eye).length_squared();
        let level = view.lod.choose(item.class(), item.radius * item.radius, d2, item.level);
        // Levels rarely change: leave the item's cache line clean when it keeps its own.
        if level != item.level {
            item.level = level;
        }
        level
    } else {
        view.lod.choose_caster(item.class(), item.radius)
    };
    out.push_level(item.entry, item.list, level);
}

/// The cull modes of [`Grid::cull_filtered`]: a camera's view (every instance), or a shadow view keeping the
/// casters of both regions or of the static region only.
const VIEW: u8 = 0;
const ALL_CASTERS: u8 = 1;
const STATIC_CASTERS: u8 = 2;

/// Whether a cull in `MODE` considers `item` at all.
#[inline(always)]
fn keeps<const MODE: u8>(item: &Item) -> bool {
    match MODE {
        VIEW => true,
        ALL_CASTERS => item.caster(),
        _ => item.caster() && item.entry & DYNAMIC_ENTRY == 0,
    }
}

fn swap_remove<T: Copy>(items: &mut Vec<T>, pos: u32) -> Option<T> {
    let last = items.pop()?;
    let slot = items.get_mut(pos as usize)?;
    *slot = last;
    Some(last)
}

#[cfg(test)]
mod tests {
    use metallic_core::CFrame;
    use metallic_core::glam::Quat;

    use super::super::instance::{MeshKind, affine_rows};
    use super::super::lists::TRANSPARENT_LIST;
    use super::*;
    use crate::cull::CullSettings;
    use crate::parts::Camera;

    fn instance(at: Vec3, size: Vec3) -> GpuInstance {
        GpuInstance {
            rows: affine_rows(CFrame::new(at), size, Vec3::ZERO),
            flags: GpuInstance::LIVE,
            mesh_material: u32::from(MeshKind::BLOCK.0) << 16,
            ..GpuInstance::default()
        }
    }

    fn view(eye: Vec3, target: Vec3) -> CullView {
        let camera = Camera::from_cframe(CFrame::look_at(eye, target, Vec3::Y), 90.0);
        CullView::for_camera(&camera, Vec3::ZERO, 100, 100, &CullSettings::default())
    }

    fn cull(grid: &mut Grid, view: &CullView) -> (VisibleSet, CullStats) {
        let mut set = VisibleSet::new();
        let stats = grid.cull(view, [16usize].into_iter(), 16, &mut set);
        (set, stats)
    }

    #[test]
    fn the_sphere_holds_the_turned_box() {
        let size = Vec3::new(2.0, 4.0, 6.0);
        let turned = GpuInstance {
            rows: affine_rows(
                CFrame::from_parts(Vec3::new(1.0, 2.0, 3.0), Quat::from_rotation_y(0.7)),
                size,
                Vec3::ZERO,
            ),
            ..GpuInstance::default()
        };
        let (center, radius) = bounding_sphere(&turned);
        assert!(center.abs_diff_eq(Vec3::new(1.0, 2.0, 3.0), 1e-6));
        assert!((radius - size.length() / 2.0).abs() < 1e-5);
        // The box: Y is not turned, X and Z mix.
        let half = half_extents(&turned);
        assert!((half.y - 2.0).abs() < 1e-5);
        assert!(half.x > 1.0 && half.z < 3.0 && half.x < 3.7 && half.z > 2.0, "{half}");
    }

    #[test]
    fn items_follow_their_slots_between_cells() {
        let mut grid = Grid::default();
        grid.update_new(Region::Static, 0, &instance(Vec3::new(1.0, 1.0, 1.0), Vec3::ONE), 0);
        grid.update_new(Region::Static, 1, &instance(Vec3::new(2.0, 1.0, 1.0), Vec3::ONE), 0);
        grid.update_new(Region::Dynamic, 0, &instance(Vec3::new(100.0, 1.0, 1.0), Vec3::ONE), 0);
        assert_eq!((grid.cells.len(), grid.len()), (2, 3));
        // Moving within a cell rewrites the item; moving out takes it to another cell.
        grid.update_new(Region::Static, 0, &instance(Vec3::new(3.0, 1.0, 1.0), Vec3::ONE), 0);
        assert_eq!(grid.cells[0].items[0].center.x, 3.0);
        grid.update_new(Region::Static, 0, &instance(Vec3::new(101.0, 1.0, 1.0), Vec3::ONE), 0);
        assert_eq!(grid.cells[0].items.len(), 1);
        assert_eq!(grid.cells[1].items.len(), 2);
        // Slot 1 was swapped into position 0 of the first cell: removing it still finds it.
        grid.update_new(Region::Static, 1, &GpuInstance::FREE, NOT_LISTED);
        assert!(grid.cells[0].items.is_empty());
        assert_eq!(grid.cells[0].reach, 0.0);
        // A large item goes on the large list, and comes back when it shrinks.
        grid.update_new(
            Region::Static,
            0,
            &instance(Vec3::ZERO, Vec3::new(500.0, 1.0, 500.0)),
            0,
        );
        assert_eq!((grid.large.len(), grid.cells[1].items.len()), (1, 1));
        grid.update_new(Region::Static, 0, &instance(Vec3::ZERO, Vec3::ONE), 0);
        assert!(grid.large.is_empty());
        assert_eq!(grid.len(), 2);
        grid.clear();
        assert_eq!(grid.len(), 0);
    }

    #[test]
    fn a_cull_keeps_what_the_view_sees_front_to_back() {
        let mut grid = Grid::default();
        // A row of blocks along −Z every 10 m, a row behind the eye, and one off to the side.
        let mut slot = 0;
        for i in 1..=8 {
            grid.update_new(
                Region::Static,
                slot,
                &instance(Vec3::new(0.0, 0.0, -10.0 * i as f32), Vec3::ONE),
                0,
            );
            slot += 1;
            grid.update_new(
                Region::Static,
                slot,
                &instance(Vec3::new(0.0, 0.0, 10.0 * i as f32), Vec3::ONE),
                0,
            );
            slot += 1;
        }
        grid.update_new(
            Region::Static,
            slot,
            &instance(Vec3::new(500.0, 0.0, -10.0), Vec3::ONE),
            0,
        );
        grid.update_new(
            Region::Dynamic,
            0,
            &instance(Vec3::new(0.0, 0.0, -5.0), Vec3::ONE),
            TRANSPARENT_LIST,
        );
        // A floor under everything: large, and crossing the view.
        grid.update_new(
            Region::Dynamic,
            1,
            &instance(Vec3::new(0.0, -2.0, 0.0), Vec3::new(400.0, 1.0, 400.0)),
            0,
        );
        let v = view(Vec3::ZERO, Vec3::NEG_Z);
        let (set, stats) = cull(&mut grid, &v);
        assert_eq!(stats.candidates, 19);
        assert_eq!(stats.visible, 10, "{stats:?}");
        assert_eq!(stats.outside_frustum, 9);
        assert_eq!(set.transparent(), [entry(Region::Dynamic, 0)]);
        let ahead: Vec<u32> = (0..8).map(|i| entry(Region::Static, i * 2)).collect();
        // The row nearest first (a cell at a time, each in the order its items arrived), and the floor, whose
        // nearest point is under the eye like the first cell's, right after that cell.
        let floor = entry(Region::Dynamic, 1);
        assert_eq!(set.opaque[0][3], floor, "{:?}", set.opaque[0]);
        let row: Vec<u32> = set.opaque[0].iter().copied().filter(|&e| e != floor).collect();
        assert_eq!(row, ahead);
        // Turned round, the other row; and every cell behind is skipped whole.
        let (set, stats) = cull(&mut grid, &view(Vec3::ZERO, Vec3::Z));
        assert_eq!(set.opaque[0].len(), 9);
        assert!(stats.cells_outside > 0, "{stats:?}");
    }

    #[test]
    fn distance_and_size_cull_whole_cells_and_single_items() {
        let mut grid = Grid::default();
        for i in 0..10 {
            let at = Vec3::new(0.0, 0.0, -100.0 * (i + 1) as f32);
            grid.update_new(Region::Static, i, &instance(at, Vec3::ONE), 0);
        }
        let camera = Camera::from_cframe(CFrame::look_at(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y), 90.0);
        let near = CullView::for_camera(
            &camera,
            Vec3::ZERO,
            100,
            100,
            &CullSettings {
                max_distance: 350.0,
                min_screen_size: 0.0,
                ..CullSettings::default()
            },
        );
        let (set, stats) = cull(&mut grid, &near);
        assert_eq!(set.opaque[0].len(), 3);
        assert_eq!(stats.too_far_or_small, 7);
        // A unit cube (radius 0.87) on a 1000-pixel target at 90°: 2 pixels across at about 433 m.
        let small = CullView::for_camera(
            &camera,
            Vec3::ZERO,
            1000,
            1000,
            &CullSettings {
                min_screen_size: 2.0,
                ..CullSettings::default()
            },
        );
        let (set, _) = cull(&mut grid, &small);
        assert_eq!(set.opaque[0].len(), 4);
    }
}
