//! The objects an arrange operation works on, built from [`PartSnapshot`]s.
//!
//! [`read_objects`] is the only function here that reads the world (through
//! [`geom::snapshot`](crate::tools::geom::snapshot) plus `Shape`, the stored `Orientation` and the
//! parent). Every operation then works on the returned [`Object`]s alone.
//!
//! A selected `BasePart` is one object. A selected `Model` (or `Folder`) is one object made of
//! every `BasePart` below it; it moves as a rigid unit. An id whose ancestor is also selected is
//! skipped, so selecting a Model and one of its parts moves the Model once. Ids of other classes
//! (scripts, services) are ignored.

use glam::{DMat3, DQuat, DVec3};
use metallic_core::{CoreError, InstanceId, Value, Vec3, World};

use super::math::Pose;
use super::plan::ArrangeError;
use crate::tools::geom::{Aabb, PartSnapshot, box_corners, snapshot};

/// The `Shape` property of a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A box.
    Block,
    /// A sphere (diameter = smallest size axis).
    Ball,
    /// A cylinder along local X.
    Cylinder,
    /// A right triangular prism: profile in local Y-Z, tall edge at +Z, symmetric across local X.
    Wedge,
}

impl Shape {
    /// Parses the property string; unknown names read as `Block`.
    pub fn from_name(name: &str) -> Shape {
        match name {
            "Ball" => Shape::Ball,
            "Cylinder" => Shape::Cylinder,
            "Wedge" => Shape::Wedge,
            _ => Shape::Block,
        }
    }
}

/// One part: its snapshot plus what arrange needs beyond it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartState {
    /// Placement and size.
    pub snap: PartSnapshot,
    /// `Shape`.
    pub shape: Shape,
    /// The `Orientation` value as stored, so a planned change records the exact value it
    /// replaces. Not updated by [`PartState::transformed`].
    pub read_orientation: Vec3,
}

impl PartState {
    /// Builds a part from a snapshot. `read_orientation` is the stored `Orientation`.
    pub fn new(snap: PartSnapshot, shape: Shape, read_orientation: Vec3) -> PartState {
        PartState {
            snap,
            shape,
            read_orientation,
        }
    }

    /// The part's id.
    pub fn id(&self) -> InstanceId {
        self.snap.id
    }

    /// Centre and rotation matrix.
    pub fn pose(&self) -> Pose {
        Pose {
            pos: self.snap.position,
            rot: DMat3::from_quat(self.snap.rotation),
        }
    }

    /// The same part with a new pose (`rot` must be a proper rotation).
    pub fn with_pose(&self, pose: &Pose) -> PartState {
        let mut out = *self;
        out.snap.position = pose.pos;
        out.snap.rotation = DQuat::from_mat3(&pose.rot).normalize();
        out
    }

    /// The same part moved rigidly by `t` (applied after the current pose).
    pub fn transformed(&self, t: &Pose) -> PartState {
        self.with_pose(&t.compose(&self.pose()))
    }

    /// The part's bounding box in `frame` (coordinates along `frame`'s columns).
    pub fn bounds_in(&self, frame: &DMat3) -> Aabb {
        if *frame == DMat3::IDENTITY {
            return self.snap.aabb();
        }
        let to_frame = frame.transpose();
        let corners = box_corners(self.snap.position, self.snap.rotation, self.snap.size);
        Aabb::from_points(corners.iter().map(|&c| to_frame * c)).unwrap_or_else(|| self.snap.aabb())
    }
}

/// Whether an object is a single part or a group of parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// A `BasePart`.
    Part,
    /// A `Model` or `Folder`, moved as one unit.
    Group,
}

/// One selected object and the parts it moves.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    /// The selected instance.
    pub id: InstanceId,
    /// Its parent (where copies go).
    pub parent: Option<InstanceId>,
    /// Part or group.
    pub kind: ObjectKind,
    /// The part itself, or every `BasePart` below the group in tree order. Never empty.
    pub parts: Vec<PartState>,
}

impl Object {
    /// The object's bounding box in `frame`.
    pub fn bounds_in(&self, frame: &DMat3) -> Aabb {
        let mut b = self.parts[0].bounds_in(frame);
        for p in &self.parts[1..] {
            b = b.union(&p.bounds_in(frame));
        }
        b
    }

    /// World bounding box.
    pub fn bounds(&self) -> Aabb {
        self.bounds_in(&DMat3::IDENTITY)
    }

    /// The pivot: a part's own pose; a group's world bounding-box centre with no rotation (the
    /// engine has no `WorldPivot` property yet).
    pub fn pivot(&self) -> Pose {
        match self.kind {
            ObjectKind::Part => self.parts[0].pose(),
            ObjectKind::Group => Pose::translation(centre(&self.bounds())),
        }
    }

    /// Every part moved rigidly by `t`.
    pub fn transformed(&self, t: &Pose) -> Vec<PartState> {
        self.parts.iter().map(|p| p.transformed(t)).collect()
    }
}

/// The centre of a box.
pub fn centre(b: &Aabb) -> DVec3 {
    (b.min + b.max) * 0.5
}

/// Reads the objects for `ids` in selection order (the last one is the active object).
///
/// Errors: [`ArrangeError::Core`] for a destroyed id.
pub fn read_objects(world: &World, ids: &[InstanceId]) -> Result<Vec<Object>, ArrangeError> {
    let mut out: Vec<Object> = Vec::new();
    for (i, &id) in ids.iter().enumerate() {
        if !world.is_alive(id) {
            return Err(ArrangeError::Core(CoreError::InvalidInstance));
        }
        if ids[..i].contains(&id) || has_selected_ancestor(world, id, ids) {
            continue;
        }
        let (kind, part_ids) = if world.is_a(id, "BasePart") {
            (ObjectKind::Part, vec![id])
        } else if world.is_a(id, "Model") || world.is_a(id, "Folder") {
            let mut found = Vec::new();
            let mut stack: Vec<InstanceId> = world.children(id).iter().rev().copied().collect();
            while let Some(cur) = stack.pop() {
                if world.is_a(cur, "BasePart") {
                    found.push(cur);
                }
                stack.extend(world.children(cur).iter().rev());
            }
            (ObjectKind::Group, found)
        } else {
            continue;
        };
        let parts: Vec<PartState> = snapshot(world, &part_ids)
            .into_iter()
            .map(|snap| {
                let shape = match world.get_ref(snap.id, "Shape") {
                    Ok(Value::String(s)) => Shape::from_name(s),
                    _ => Shape::Block,
                };
                let orientation = match world.get_ref(snap.id, "Orientation") {
                    Ok(Value::Vector3(v)) => *v,
                    _ => Vec3::default(),
                };
                PartState::new(snap, shape, orientation)
            })
            .collect();
        if !parts.is_empty() {
            out.push(Object {
                id,
                parent: world.parent(id),
                kind,
                parts,
            });
        }
    }
    Ok(out)
}

/// The bounding box of all objects in `frame`. `objects` must not be empty.
pub fn selection_bounds_in(objects: &[Object], frame: &DMat3) -> Aabb {
    let mut b = objects[0].bounds_in(frame);
    for o in &objects[1..] {
        b = b.union(&o.bounds_in(frame));
    }
    b
}

/// The selection's pivot point: one object's pivot, or the centre of several objects' bounds.
pub fn selection_pivot(objects: &[Object]) -> DVec3 {
    match objects {
        [one] => one.pivot().pos,
        _ => centre(&selection_bounds_in(objects, &DMat3::IDENTITY)),
    }
}

fn has_selected_ancestor(world: &World, id: InstanceId, ids: &[InstanceId]) -> bool {
    let mut cur = world.parent(id);
    while let Some(p) = cur {
        if ids.contains(&p) {
            return true;
        }
        cur = world.parent(p);
    }
    false
}
