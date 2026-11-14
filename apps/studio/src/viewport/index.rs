//! The parts in Workspace by where they are (#596): what picking, the light icons' "is it hidden" test and the camera's
//! reach ask instead of walking every instance in the game on every frame.
//!
//! The index registers a change consumer on the data model ([`DataModel::register_consumer`]) and, once a frame
//! ([`PartIndex::update`]), drains what changed: a part that moved has its box updated in place, a part added or taken
//! out of Workspace joins or leaves. It keeps each part's world box (the axis-aligned box round its `CFrame` and
//! `Size`) in a bounding volume hierarchy, so a ray tests only the parts whose boxes it passes through. A part's
//! actual shape ([`super::shapes`]) is still tested by the caller, for the few the boxes let through.
//!
//! The tree is not rebuilt for every change:
//!
//! - a part that moves keeps its place, and the boxes above it are refitted (one pass up the tree);
//! - a part that joins goes into a short list tested one by one, until there are enough of them to build again;
//! - a part that leaves is marked gone where it is.
//!
//! The tree is built again when the loose list or the parts gone grow past a share of the whole, or when so many have
//! moved that the refitted boxes overlap more than they should. Building is O(n log n), refitting O(n), and a frame
//! with no changes costs one empty drain.
//!
//! The index also keeps the lights in Workspace, which the viewport draws an icon for (#506).

use std::collections::{BTreeSet, HashMap};

use metallic_engine::metallic_core::glam::{Mat3, Vec3};
use metallic_engine::metallic_core::{ConsumerId, DataModel, DirtyBits, Id};

use super::pick::{self, Bounds};
use super::view::Ray;

/// How many parts a leaf of the tree holds at most.
const LEAF: usize = 4;
/// How many parts that joined since the tree was built are tested one by one before it is built again.
const LOOSE: usize = 256;

/// What a ray query does after a part ([`PartIndex::ray`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Go on.
    Go,
    /// Go on, but only as far as this along the ray: a nearer hit was found.
    Limit(f32),
    /// Stop: the answer is known.
    Stop,
}

/// A part's place in the index.
#[derive(Clone, Copy, Debug)]
struct Item {
    /// `None` once the part has left (its place is reused).
    id: Option<Id>,
    min: Vec3,
    max: Vec3,
}

/// A node of the tree. A leaf has `count > 0` parts at `order[start..start + count]`; an inner node has its left child
/// right after it and its right child at `start`.
#[derive(Clone, Copy, Debug)]
struct Node {
    min: Vec3,
    max: Vec3,
    start: u32,
    count: u32,
}

/// The parts and lights in a data model's Workspace, by where they are. See the [module documentation](self).
#[derive(Debug, Default)]
pub struct PartIndex {
    /// The data model followed ([`DataModel::uid`]) and the consumer registered on it.
    tracking: Option<(u64, ConsumerId)>,
    changes: Vec<(Id, DirtyBits)>,
    removed: Vec<Id>,
    /// Each part's place in `items`.
    places: HashMap<Id, u32>,
    items: Vec<Item>,
    /// Places in `items` free for reuse.
    free: Vec<u32>,
    /// The places in the tree, leaf by leaf.
    order: Vec<u32>,
    nodes: Vec<Node>,
    /// Whether each place in `items` is in the tree (rather than loose).
    in_tree: Vec<bool>,
    /// Places that joined since the tree was built, tested one by one.
    loose: Vec<u32>,
    /// Parts moved and places freed since the tree was built.
    moved: usize,
    gone: usize,
    /// The tree's boxes need refitting.
    refit: bool,
    lights: BTreeSet<Id>,
    revision: u64,
}

impl PartIndex {
    /// An empty index; it fills on its first [`PartIndex::update`].
    pub fn new() -> Self {
        Self::default()
    }

    /// An index of `dm`'s Workspace as it is now.
    pub fn of(dm: &mut DataModel) -> Self {
        let mut index = Self::new();
        index.update(dm);
        index
    }

    /// Takes in what changed in `dm` since the last update. The first update of a data model (or of another one than
    /// before) reads every instance in it.
    pub fn update(&mut self, dm: &mut DataModel) {
        let consumer = match self.tracking {
            Some((uid, consumer)) if uid == dm.uid() => consumer,
            _ => {
                *self = Self {
                    revision: self.revision.wrapping_add(1),
                    ..Self::default()
                };
                let interest = DirtyBits::TRANSFORM | DirtyBits::VISUAL | DirtyBits::STRUCTURE | DirtyBits::PROPERTY;
                let Ok(consumer) = dm.register_consumer(interest) else {
                    log::warn!("viewport: no room for the part index's change consumer; picking walks every part");
                    return;
                };
                dm.mark_all(consumer);
                self.tracking = Some((dm.uid(), consumer));
                consumer
            }
        };
        let mut changes = std::mem::take(&mut self.changes);
        let mut removed = std::mem::take(&mut self.removed);
        dm.drain_changes(consumer, &mut changes);
        dm.drain_removed(consumer, &mut removed);
        if !changes.is_empty() || !removed.is_empty() {
            self.revision = self.revision.wrapping_add(1);
            let world = pick::workspace(dm);
            for &id in &removed {
                self.leave(id);
                self.lights.remove(&id);
            }
            for &(id, bits) in &changes {
                self.apply(dm, world, id, bits);
            }
            let live = self.places.len();
            if self.loose.len() > LOOSE.max(live / 8)
                || self.gone > live.max(LOOSE) / 2
                || self.moved > live.max(LOOSE) / 2
            {
                self.build();
            } else if self.refit {
                self.refit_tree();
            }
        }
        changes.clear();
        removed.clear();
        self.changes = changes;
        self.removed = removed;
    }

    /// Changes whenever anything in the data model the index follows changed: what a cache of the parts or lights
    /// can compare.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether this index follows `dm` (it was updated with it): until then, ask the data model itself.
    pub fn follows(&self, dm: &DataModel) -> bool {
        self.tracking.is_some_and(|(uid, _)| uid == dm.uid())
    }

    /// How many parts are in Workspace.
    pub fn len(&self) -> usize {
        self.places.len()
    }

    /// Whether Workspace has no parts.
    pub fn is_empty(&self) -> bool {
        self.places.is_empty()
    }

    /// Every part in Workspace, in no particular order.
    pub fn parts(&self) -> impl Iterator<Item = Id> + '_ {
        self.items.iter().filter_map(|item| item.id)
    }

    /// Every light in Workspace, in the order of their ids.
    pub fn lights(&self) -> impl Iterator<Item = Id> + '_ {
        self.lights.iter().copied()
    }

    /// Calls `visit` with every part whose box `ray` meets no farther than `limit` along it, and how far along the
    /// box starts, nearer boxes roughly first; `visit` says whether to go on, and how far.
    pub fn ray(&self, (origin, dir): Ray, limit: f32, mut visit: impl FnMut(Id, f32) -> Step) {
        let inverse = Vec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut limit = limit;
        let mut meet = |item: &Item, limit: &mut f32| -> bool {
            let Some(id) = item.id else {
                return true;
            };
            let Some(t) = slab(origin, inverse, item.min, item.max, *limit) else {
                return true;
            };
            match visit(id, t) {
                Step::Go => true,
                Step::Limit(t) => {
                    *limit = limit.min(t);
                    true
                }
                Step::Stop => false,
            }
        };
        for &place in &self.loose {
            if let Some(item) = self.items.get(place as usize)
                && !meet(item, &mut limit)
            {
                return;
            }
        }
        if self.nodes.is_empty() {
            return;
        }
        let mut stack: Vec<(u32, f32)> = vec![(0, 0.0)];
        while let Some((at, near)) = stack.pop() {
            if near > limit {
                continue;
            }
            let Some(node) = self.nodes.get(at as usize) else {
                continue;
            };
            if node.count > 0 {
                let start = node.start as usize;
                for &place in self.order.get(start..start + node.count as usize).unwrap_or(&[]) {
                    if let Some(item) = self.items.get(place as usize)
                        && !meet(item, &mut limit)
                    {
                        return;
                    }
                }
                continue;
            }
            let children = [at + 1, node.start].map(|child| {
                let near = self
                    .nodes
                    .get(child as usize)
                    .and_then(|node| slab(origin, inverse, node.min, node.max, limit));
                (child, near)
            });
            // The nearer child last, so it is visited first.
            let [a, b] = children;
            let (first, second) = match (a.1, b.1) {
                (Some(ta), Some(tb)) if tb < ta => (b, a),
                _ => (a, b),
            };
            for (child, near) in [second, first] {
                if let Some(near) = near {
                    stack.push((child, near));
                }
            }
        }
    }

    /// Takes in one changed instance.
    fn apply(&mut self, dm: &DataModel, world: Option<Id>, id: Id, bits: DirtyBits) {
        let placed = bits.intersects(DirtyBits::STRUCTURE | DirtyBits::CREATED);
        let indexed = self.places.contains_key(&id);
        if !placed && !indexed {
            // Not a part in Workspace before, and it has not moved in the tree: still not one (unless it is a light,
            // whose membership only a move changes too).
            return;
        }
        let in_world = |dm: &DataModel| world.is_some_and(|world| dm.is_descendant_of(id, world).unwrap_or(false));
        if placed {
            if pick::is_a(dm, id, "Light") && in_world(dm) {
                self.lights.insert(id);
            } else {
                self.lights.remove(&id);
            }
        }
        let bounds = if placed && !in_world(dm) {
            None
        } else {
            pick::part_box(dm, id)
        };
        match bounds {
            Some(bounds) => {
                let (min, max) = world_box(&bounds);
                self.place(id, min, max);
            }
            None => self.leave(id),
        }
    }

    /// Puts part `id` at its box `min`..`max`: where it was, or in a new place.
    fn place(&mut self, id: Id, min: Vec3, max: Vec3) {
        if let Some(&place) = self.places.get(&id) {
            if let Some(item) = self.items.get_mut(place as usize) {
                item.min = min;
                item.max = max;
            }
            if self.in_tree.get(place as usize).copied().unwrap_or(false) {
                self.moved += 1;
                self.refit = true;
            }
            return;
        }
        let item = Item { id: Some(id), min, max };
        let place = match self.free.pop() {
            Some(place) => {
                if let Some(slot) = self.items.get_mut(place as usize) {
                    *slot = item;
                }
                place
            }
            None => {
                self.items.push(item);
                self.in_tree.push(false);
                u32::try_from(self.items.len() - 1).unwrap_or(u32::MAX)
            }
        };
        // A reused place may still be in the tree, where its leaf's box no longer holds it: refit.
        if self.in_tree.get(place as usize).copied().unwrap_or(false) {
            self.refit = true;
        } else {
            self.loose.push(place);
        }
        self.places.insert(id, place);
    }

    /// Takes part `id` out, if it is in.
    fn leave(&mut self, id: Id) {
        let Some(place) = self.places.remove(&id) else {
            return;
        };
        if let Some(item) = self.items.get_mut(place as usize) {
            item.id = None;
        }
        if self.in_tree.get(place as usize).copied().unwrap_or(false) {
            self.gone += 1;
        } else {
            self.loose.retain(|&loose| loose != place);
        }
        self.free.push(place);
    }

    /// Builds the tree again from every part in.
    fn build(&mut self) {
        self.loose.clear();
        self.moved = 0;
        self.gone = 0;
        self.refit = false;
        self.free.clear();
        // Packs the parts in, so no place is gone.
        let live: Vec<Item> = self.items.iter().copied().filter(|item| item.id.is_some()).collect();
        self.items = live;
        self.places.clear();
        for (place, item) in self.items.iter().enumerate() {
            if let Some(id) = item.id {
                self.places.insert(id, u32::try_from(place).unwrap_or(u32::MAX));
            }
        }
        self.in_tree = vec![true; self.items.len()];
        self.order = (0..u32::try_from(self.items.len()).unwrap_or(u32::MAX)).collect();
        self.nodes.clear();
        if self.items.is_empty() {
            return;
        }
        let centre = |items: &[Item], place: u32| {
            items
                .get(place as usize)
                .map_or(Vec3::ZERO, |item| (item.min + item.max) * 0.5)
        };
        // (start, end, the node whose right child this is).
        let mut stack: Vec<(usize, usize, Option<usize>)> = vec![(0, self.order.len(), None)];
        while let Some((start, end, parent)) = stack.pop() {
            let at = self.nodes.len();
            if let Some(parent) = parent.and_then(|parent| self.nodes.get_mut(parent)) {
                parent.start = u32::try_from(at).unwrap_or(u32::MAX);
            }
            let places = self.order.get(start..end).unwrap_or(&[]);
            let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            let (mut low, mut high) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for &place in places {
                if let Some(item) = self.items.get(place as usize) {
                    min = min.min(item.min);
                    max = max.max(item.max);
                    let c = (item.min + item.max) * 0.5;
                    low = low.min(c);
                    high = high.max(c);
                }
            }
            let spread = high - low;
            let axis = if spread.x >= spread.y && spread.x >= spread.z {
                0
            } else if spread.y >= spread.z {
                1
            } else {
                2
            };
            if end - start <= LEAF || spread[axis] <= 0.0 || !spread[axis].is_finite() {
                self.nodes.push(Node {
                    min,
                    max,
                    start: u32::try_from(start).unwrap_or(u32::MAX),
                    count: u32::try_from(end - start).unwrap_or(u32::MAX),
                });
                continue;
            }
            self.nodes.push(Node {
                min,
                max,
                start: 0,
                count: 0,
            });
            let middle = (start + end) / 2;
            let items = &self.items;
            if let Some(places) = self.order.get_mut(start..end) {
                places.select_nth_unstable_by(middle - start, |&a, &b| {
                    centre(items, a)[axis].total_cmp(&centre(items, b)[axis])
                });
            }
            // The right half waits; the left is built next, so it comes right after this node.
            stack.push((middle, end, Some(at)));
            stack.push((start, middle, None));
        }
    }

    /// Fits every box of the tree round what is under it again, children before parents.
    fn refit_tree(&mut self) {
        self.refit = false;
        for at in (0..self.nodes.len()).rev() {
            let Some(node) = self.nodes.get(at).copied() else {
                continue;
            };
            let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            if node.count > 0 {
                let start = node.start as usize;
                for &place in self.order.get(start..start + node.count as usize).unwrap_or(&[]) {
                    if let Some(item) = self.items.get(place as usize).filter(|item| item.id.is_some()) {
                        min = min.min(item.min);
                        max = max.max(item.max);
                    }
                }
            } else {
                for child in [at + 1, node.start as usize] {
                    if let Some(child) = self.nodes.get(child) {
                        min = min.min(child.min);
                        max = max.max(child.max);
                    }
                }
            }
            if let Some(node) = self.nodes.get_mut(at) {
                node.min = min;
                node.max = max;
            }
        }
    }
}

/// Calls `visit` like [`PartIndex::ray`] for `parts` when it follows `dm`, else by walking every part in `dm`'s
/// Workspace (a data model the index was never updated with).
pub fn each_on_ray(dm: &DataModel, parts: &PartIndex, ray: Ray, limit: f32, mut visit: impl FnMut(Id, f32) -> Step) {
    if parts.follows(dm) {
        parts.ray(ray, limit, visit);
        return;
    }
    let Some(world) = pick::workspace(dm) else {
        return;
    };
    let Ok(all) = dm.descendants(world) else {
        return;
    };
    let inverse = Vec3::new(1.0 / ray.1.x, 1.0 / ray.1.y, 1.0 / ray.1.z);
    let mut limit = limit;
    for id in all {
        let Some(bounds) = pick::part_box(dm, id) else {
            continue;
        };
        let (min, max) = world_box(&bounds);
        let Some(t) = slab(ray.0, inverse, min, max, limit) else {
            continue;
        };
        match visit(id, t) {
            Step::Go => {}
            Step::Limit(t) => limit = limit.min(t),
            Step::Stop => return,
        }
    }
}

/// The world's axis-aligned box round `bounds`.
pub fn world_box(bounds: &Bounds) -> (Vec3, Vec3) {
    let m = Mat3::from_quat(bounds.rot);
    let reach = m.x_axis.abs() * bounds.half.x + m.y_axis.abs() * bounds.half.y + m.z_axis.abs() * bounds.half.z;
    (bounds.center - reach, bounds.center + reach)
}

/// How far along the ray from `origin` (with `inverse`, one over its direction) the box `min`..`max` starts, if the ray
/// meets it no farther than `limit`: 0 from inside.
fn slab(origin: Vec3, inverse: Vec3, min: Vec3, max: Vec3, limit: f32) -> Option<f32> {
    let (mut near, mut far) = (0.0f32, limit);
    for axis in 0..3 {
        let a = (min[axis] - origin[axis]) * inverse[axis];
        let b = (max[axis] - origin[axis]) * inverse[axis];
        // NaN (a ray along a face of the box) leaves the span as it was: `f32::max` and `min` skip it.
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far).then_some(near)
}

#[cfg(test)]
mod tests {
    use metallic_engine::metallic_core::{CFrame, Value, Vector3};

    use super::*;

    fn game() -> (DataModel, Id) {
        let dm = metallic_engine::metallic_classes::new_game().unwrap();
        let world = pick::workspace(&dm).unwrap();
        (dm, world)
    }

    fn part(dm: &mut DataModel, parent: Id, at: Vec3, size: Vec3) -> Id {
        let id = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
        dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(at))).unwrap();
        dm.set_by_name(id, "Size", Value::Vector3(Vector3::from(size))).unwrap();
        dm.set_parent(id, Some(parent)).unwrap();
        id
    }

    /// The parts a ray's boxes meet, as the index finds them.
    fn met(index: &PartIndex, ray: Ray) -> BTreeSet<Id> {
        let mut out = BTreeSet::new();
        index.ray(ray, f32::INFINITY, |id, _| {
            out.insert(id);
            Step::Go
        });
        out
    }

    /// The same by testing every part's box.
    fn met_by_walking(dm: &DataModel, ray: Ray) -> BTreeSet<Id> {
        let world = pick::workspace(dm).unwrap();
        dm.descendants(world)
            .unwrap()
            .filter(|&id| {
                pick::part_box(dm, id).is_some_and(|b| {
                    let (min, max) = world_box(&b);
                    slab(ray.0, ray.1.recip(), min, max, f32::INFINITY).is_some()
                })
            })
            .collect()
    }

    #[test]
    fn a_ray_meets_the_parts_whose_boxes_it_passes_through_as_the_index_follows_changes() {
        let (mut dm, world) = game();
        // A grid of 20 × 20 parts, 4 m apart.
        let mut parts = Vec::new();
        for x in 0..20 {
            for z in 0..20 {
                parts.push(part(
                    &mut dm,
                    world,
                    Vec3::new(x as f32 * 4.0, 0.0, z as f32 * 4.0),
                    Vec3::ONE,
                ));
            }
        }
        let mut index = PartIndex::of(&mut dm);
        assert_eq!(index.len(), 400);
        let rays: Vec<Ray> = vec![
            (Vec3::new(-10.0, 0.0, 8.0), Vec3::X),
            (Vec3::new(40.0, 50.0, 40.0), Vec3::NEG_Y),
            (Vec3::new(-5.0, 0.2, -5.0), Vec3::new(1.0, 0.0, 1.0).normalize()),
            (Vec3::new(100.0, 0.0, 0.0), Vec3::X),
        ];
        let check = |index: &PartIndex, dm: &DataModel| {
            for &ray in &rays {
                assert_eq!(met(index, ray), met_by_walking(dm, ray), "{ray:?}");
            }
        };
        check(&index, &dm);
        assert_eq!(met(&index, rays[0]).len(), 20, "a row");
        assert!(met(&index, rays[3]).is_empty());

        // Moved, added, taken out and destroyed, a few at a time and then many: the same answers.
        let revision = index.revision();
        dm.set_by_name(
            parts[0],
            "CFrame",
            Value::CFrame(CFrame::new(Vec3::new(500.0, 0.0, 0.0))),
        )
        .unwrap();
        let added = part(&mut dm, world, Vec3::new(150.0, 0.0, 0.0), Vec3::splat(2.0));
        dm.set_parent(parts[1], None).unwrap();
        dm.destroy(parts[2]).unwrap();
        index.update(&mut dm);
        assert_ne!(index.revision(), revision);
        check(&index, &dm);
        assert!(met(&index, rays[3]).contains(&added));
        assert!(met(&index, rays[3]).contains(&parts[0]));
        assert_eq!(index.len(), 399);
        for &id in &parts[3..300] {
            let Ok(Value::CFrame(at)) = dm.get_by_name(id, "CFrame") else {
                panic!("a part has a CFrame");
            };
            dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(at.position() + Vec3::Y * 3.0)))
                .unwrap();
        }
        for i in 0..400 {
            part(&mut dm, world, Vec3::new(i as f32, 9.0, 0.0), Vec3::ONE);
        }
        index.update(&mut dm);
        check(&index, &dm);
        assert_eq!(index.len(), 799);
        // Nothing changed: the same revision.
        let revision = index.revision();
        index.update(&mut dm);
        assert_eq!(index.revision(), revision);
    }

    #[test]
    fn a_model_taken_out_of_workspace_takes_its_parts_and_lights_with_it() {
        let (mut dm, world) = game();
        let model = dm.create(dm.registry().class_id("Model").unwrap()).unwrap();
        dm.set_parent(model, Some(world)).unwrap();
        let lamp = part(&mut dm, model, Vec3::ZERO, Vec3::ONE);
        let light = dm.create(dm.registry().class_id("PointLight").unwrap()).unwrap();
        dm.set_parent(light, Some(lamp)).unwrap();
        let mut index = PartIndex::of(&mut dm);
        assert_eq!(index.parts().collect::<Vec<_>>(), [lamp]);
        assert_eq!(index.lights().collect::<Vec<_>>(), [light]);
        dm.set_parent(model, None).unwrap();
        index.update(&mut dm);
        assert!(index.is_empty());
        assert_eq!(index.lights().count(), 0);
        dm.set_parent(model, Some(world)).unwrap();
        index.update(&mut dm);
        assert_eq!(index.len(), 1);
        assert_eq!(index.lights().count(), 1);
    }

    /// #596: a ray that passes none of 20,000 parts tests none of them, and one along a row only that row's.
    #[test]
    fn a_ray_away_from_many_parts_tests_none_of_them() {
        let (mut dm, world) = game();
        for i in 0..20_000 {
            let at = Vec3::new((i % 200) as f32 * 2.0, 0.0, (i / 200) as f32 * -2.0);
            part(&mut dm, world, at, Vec3::ONE);
        }
        let index = PartIndex::of(&mut dm);
        assert_eq!(index.len(), 20_000);
        let mut tested = 0;
        index.ray((Vec3::new(0.0, 10.0, 50.0), Vec3::Y), f32::INFINITY, |_, _| {
            tested += 1;
            Step::Go
        });
        assert_eq!(tested, 0, "looking up");
        index.ray((Vec3::new(0.0, 10.0, 50.0), Vec3::Z), f32::INFINITY, |_, _| {
            tested += 1;
            Step::Go
        });
        assert_eq!(tested, 0, "looking away");
        index.ray((Vec3::new(-10.0, 0.0, -20.0), Vec3::X), f32::INFINITY, |_, _| {
            tested += 1;
            Step::Go
        });
        assert_eq!(tested, 200, "one row");
    }

    #[test]
    fn a_ray_can_stop_and_narrow_its_reach() {
        let (mut dm, world) = game();
        for x in 0..50 {
            part(&mut dm, world, Vec3::new(x as f32 * 2.0, 0.0, 0.0), Vec3::ONE);
        }
        let index = PartIndex::of(&mut dm);
        let ray = (Vec3::new(-5.0, 0.0, 0.0), Vec3::X);
        let mut seen = 0;
        index.ray(ray, f32::INFINITY, |_, _| {
            seen += 1;
            Step::Stop
        });
        assert_eq!(seen, 1);
        // Limited to the nearest found so far, far fewer boxes come out than the 50 in line.
        let mut seen = 0;
        index.ray(ray, f32::INFINITY, |_, t| {
            seen += 1;
            Step::Limit(t + 0.5)
        });
        assert!(seen < 10, "{seen}");
        let mut near = Vec::new();
        index.ray(ray, 10.0, |_, t| {
            near.push(t);
            Step::Go
        });
        assert!(near.iter().all(|&t| t <= 10.0) && near.len() == 3, "{near:?}");
    }
}
