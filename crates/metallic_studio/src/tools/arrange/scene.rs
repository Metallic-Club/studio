//! Reads the objects an arrange operation works on from a [`World`].
//!
//! A selected `BasePart` is one object. A selected `Model` (or `Folder`) is one object made of
//! every `BasePart` below it; it moves as a rigid unit. An id whose ancestor is also selected is
//! skipped, so selecting a Model and one of its parts moves the Model once. Ids of other classes
//! (scripts, services) are ignored.

use metallic_core::{InstanceId, Value, Vec3, World};

use super::math::{Aabb, Mat3, Pose, box_corners};
use super::plan::ArrangeError;

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

/// One part's placement and size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartState {
    /// The part.
    pub id: InstanceId,
    /// Centre and rotation.
    pub pose: Pose,
    /// `Size`.
    pub size: Vec3,
    /// `Shape`.
    pub shape: Shape,
    /// The `Orientation` value as read from the world (kept so a planned change records the
    /// exact value it replaces; not updated by [`PartState::transformed`]).
    pub read_orientation: Vec3,
}

impl PartState {
    /// Reads a part. Errors if `id` is not a live `BasePart`.
    pub fn read(world: &World, id: InstanceId) -> Result<PartState, ArrangeError> {
        let pos = vec3(world, id, "Position")?;
        let orientation = vec3(world, id, "Orientation")?;
        let size = vec3(world, id, "Size")?;
        let shape = match world.get(id, "Shape")? {
            Value::String(s) => Shape::from_name(&s),
            _ => Shape::Block,
        };
        Ok(PartState {
            id,
            pose: Pose {
                pos,
                rot: Mat3::from_orientation(orientation),
            },
            size,
            shape,
            read_orientation: orientation,
        })
    }

    /// The same part moved rigidly by `t` (applied after the current pose).
    pub fn transformed(&self, t: &Pose) -> PartState {
        PartState {
            pose: t.compose(&self.pose),
            ..*self
        }
    }

    /// The part's bounding box expressed in `frame` (coordinates along `frame`'s columns).
    pub fn bounds_in(&self, frame: &Mat3) -> Aabb {
        let to_frame = frame.transpose();
        let corners = box_corners(&self.pose, self.size);
        let mut b = Aabb::point(to_frame.apply(corners[0]));
        for c in &corners[1..] {
            b.include(to_frame.apply(*c));
        }
        b
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
    /// Part or group.
    pub kind: ObjectKind,
    /// The part itself, or every `BasePart` below the group in tree order.
    pub parts: Vec<PartState>,
}

impl Object {
    /// The object's bounding box in `frame`.
    pub fn bounds_in(&self, frame: &Mat3) -> Aabb {
        let mut b = self.parts[0].bounds_in(frame);
        for p in &self.parts[1..] {
            b = b.union(&p.bounds_in(frame));
        }
        b
    }

    /// World bounding box.
    pub fn bounds(&self) -> Aabb {
        self.bounds_in(&Mat3::IDENTITY)
    }

    /// The pivot: a part's own pose; a group's world bounding-box centre with no rotation (the
    /// engine has no `WorldPivot` property yet).
    pub fn pivot(&self) -> Pose {
        match self.kind {
            ObjectKind::Part => self.parts[0].pose,
            ObjectKind::Group => Pose::translation(self.bounds().centre()),
        }
    }

    /// Every part moved rigidly by `t`.
    pub fn transformed(&self, t: &Pose) -> Vec<PartState> {
        self.parts.iter().map(|p| p.transformed(t)).collect()
    }
}

/// Reads the objects for `ids` in selection order (the last one is the active object).
///
/// Errors: [`ArrangeError::Core`] for a destroyed id or unreadable part property.
pub fn read_objects(world: &World, ids: &[InstanceId]) -> Result<Vec<Object>, ArrangeError> {
    let mut out: Vec<Object> = Vec::new();
    for (i, &id) in ids.iter().enumerate() {
        if !world.is_alive(id) {
            return Err(ArrangeError::Core(
                metallic_core::CoreError::InvalidInstance,
            ));
        }
        if ids[..i].contains(&id) || has_selected_ancestor(world, id, ids) {
            continue;
        }
        if world.is_a(id, "BasePart") {
            out.push(Object {
                id,
                kind: ObjectKind::Part,
                parts: vec![PartState::read(world, id)?],
            });
        } else if world.is_a(id, "Model") || world.is_a(id, "Folder") {
            let mut parts = Vec::new();
            let mut stack: Vec<InstanceId> = world.children(id).iter().rev().copied().collect();
            while let Some(cur) = stack.pop() {
                if world.is_a(cur, "BasePart") {
                    parts.push(PartState::read(world, cur)?);
                }
                stack.extend(world.children(cur).iter().rev());
            }
            if !parts.is_empty() {
                out.push(Object {
                    id,
                    kind: ObjectKind::Group,
                    parts,
                });
            }
        }
    }
    Ok(out)
}

/// The bounding box of all objects in `frame`.
pub fn selection_bounds_in(objects: &[Object], frame: &Mat3) -> Aabb {
    let mut b = objects[0].bounds_in(frame);
    for o in &objects[1..] {
        b = b.union(&o.bounds_in(frame));
    }
    b
}

/// The selection's pivot point: one object's pivot, or the centre of several objects' bounds.
pub fn selection_pivot(objects: &[Object]) -> Vec3 {
    match objects {
        [one] => one.pivot().pos,
        _ => selection_bounds_in(objects, &Mat3::IDENTITY).centre(),
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

fn vec3(world: &World, id: InstanceId, prop: &str) -> Result<Vec3, ArrangeError> {
    match world.get(id, prop)? {
        Value::Vector3(v) => Ok(v),
        _ => Err(ArrangeError::Invalid(format!("{prop} is not a Vector3"))),
    }
}
