//! What an arrange operation returns: planned property changes and planned copies.
//!
//! A [`Plan`] never touches the world. Studio's editing layer (#10, #213) turns it into one undo
//! step on commit, and draws it as a preview (moved parts and ghost copies) while a popover is
//! open. One plan is one gesture.

use metallic_core::{CoreError, InstanceId, Value, Vec3, World};

use super::scene::{Object, ObjectKind, PartState};

/// Why an operation could not be planned.
#[derive(Debug, thiserror::Error)]
pub enum ArrangeError {
    /// Too few objects selected (after skipping ids inside selected Models).
    #[error("select {needed} or more objects")]
    NeedMore {
        /// Objects the operation needs.
        needed: usize,
        /// Objects found.
        got: usize,
    },
    /// An option is out of range (count, spacing, path).
    #[error("{0}")]
    Invalid(String),
    /// Reading the world failed.
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// One property write on an existing instance, with the value it replaces.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyChange {
    /// The instance.
    pub id: InstanceId,
    /// `Position`, `Orientation` or `Size`.
    pub property: &'static str,
    /// Current value.
    pub before: Value,
    /// Planned value.
    pub after: Value,
}

/// The placement of one part inside a planned copy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CopiedPart {
    /// The part in the source subtree this copy part comes from.
    pub source_part: InstanceId,
    /// `Position` of the copy.
    pub position: Vec3,
    /// `Orientation` of the copy (degrees).
    pub orientation: Vec3,
    /// `Size` of the copy.
    pub size: Vec3,
}

/// A copy of a selected object's subtree (`World::clone_subtree`) with new part placements.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedCopy {
    /// The selected object that is copied.
    pub source: InstanceId,
    /// Where the copy goes: the source's parent, unless [`Plan::group`] is set, in which case the
    /// copy goes inside that new Model.
    pub parent: Option<InstanceId>,
    /// One entry per part of the source object.
    pub parts: Vec<CopiedPart>,
}

/// A new Model that holds all planned copies ("Group result" in the array popover).
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedGroup {
    /// The Model's name.
    pub name: String,
    /// Its parent.
    pub parent: Option<InstanceId>,
}

/// The result of an arrange operation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    /// Undo name, "Verb object (count)".
    pub label: String,
    /// Writes to existing parts. Unchanged values are left out.
    pub changes: Vec<PropertyChange>,
    /// New copies.
    pub copies: Vec<PlannedCopy>,
    /// A Model to put the copies in.
    pub group: Option<PlannedGroup>,
}

impl Plan {
    /// Whether the plan does nothing.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.copies.is_empty()
    }

    /// The planned value of `property` on `id`, if the plan changes it.
    pub fn after(&self, id: InstanceId, property: &str) -> Option<&Value> {
        self.changes
            .iter()
            .find(|c| c.id == id && c.property == property)
            .map(|c| &c.after)
    }

    /// Adds the changes that move `before` to `after` (same part). Values within `1e-12` are
    /// treated as unchanged so a no-op leaves no entry.
    pub fn push_part(&mut self, before: &PartState, after: &PartState) {
        debug_assert_eq!(before.id, after.id);
        let moved = (after.pose.pos - before.pose.pos).magnitude() > 1e-12;
        if moved {
            self.changes.push(PropertyChange {
                id: before.id,
                property: "Position",
                before: Value::Vector3(before.pose.pos),
                after: Value::Vector3(after.pose.pos),
            });
        }
        if after.pose.rot.max_diff(&before.pose.rot) > 1e-12 {
            self.changes.push(PropertyChange {
                id: before.id,
                property: "Orientation",
                before: Value::Vector3(before.read_orientation),
                after: Value::Vector3(after.pose.rot.to_orientation()),
            });
        }
        if (after.size - before.size).magnitude() > 1e-12 {
            self.changes.push(PropertyChange {
                id: before.id,
                property: "Size",
                before: Value::Vector3(before.size),
                after: Value::Vector3(after.size),
            });
        }
    }

    /// Adds the changes for a whole object whose parts became `after` (same order).
    pub fn push_object(&mut self, object: &Object, after: &[PartState]) {
        for (b, a) in object.parts.iter().zip(after) {
            self.push_part(b, a);
        }
    }

    /// Adds a copy of `object` with its parts placed at `after` (same order).
    pub fn push_copy(&mut self, world: &World, object: &Object, after: &[PartState]) {
        self.copies.push(PlannedCopy {
            source: object.id,
            parent: world.parent(object.id),
            parts: object
                .parts
                .iter()
                .zip(after)
                .map(|(src, a)| CopiedPart {
                    source_part: src.id,
                    position: a.pose.pos,
                    orientation: a.pose.rot.to_orientation(),
                    size: a.size,
                })
                .collect(),
        });
    }
}

/// "3 parts", "1 model", "2 objects" for undo names.
pub fn describe(objects: &[Object]) -> String {
    let n = objects.len();
    let all = |k: ObjectKind| objects.iter().all(|o| o.kind == k);
    let noun = if all(ObjectKind::Part) {
        "part"
    } else if all(ObjectKind::Group) {
        "model"
    } else {
        "object"
    };
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}
