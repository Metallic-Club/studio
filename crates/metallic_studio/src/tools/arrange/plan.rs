//! What an arrange operation returns: planned property changes and planned copies.
//!
//! Operations take [`Object`]s (built from `tools::geom::PartSnapshot`s by
//! [`read_objects`](super::scene::read_objects)), so a [`Plan`] never touches the world. Studio's editing layer (#10, #213) turns it into one undo
//! step on commit, and draws it as a preview (moved parts and ghost copies) while a popover is
//! open. One plan is one gesture.

use metallic_core::{CoreError, InstanceId, Value, Vec3};

use super::math::{max_diff, orientation_of, to_core};
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

    /// Errors if any planned value is not finite (huge inputs can overflow to infinity), so
    /// a plan never asks the world to store NaN or infinity.
    pub fn check_finite(self) -> Result<Plan, ArrangeError> {
        let ok = |v: &Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
        let changes_ok = self.changes.iter().all(|c| match &c.after {
            Value::Vector3(v) => ok(v),
            _ => true,
        });
        let copies_ok = self
            .copies
            .iter()
            .flat_map(|c| &c.parts)
            .all(|p| ok(&p.position) && ok(&p.orientation) && ok(&p.size));
        if changes_ok && copies_ok {
            Ok(self)
        } else {
            Err(ArrangeError::Invalid(
                "the result is out of range (a value is too large)".into(),
            ))
        }
    }

    /// The planned value of `property` on `id`, if the plan changes it.
    pub fn after(&self, id: InstanceId, property: &str) -> Option<&Value> {
        self.changes
            .iter()
            .find(|c| c.id == id && c.property == property)
            .map(|c| &c.after)
    }

    /// Adds the changes that move `before` to `after` (same part). Values within `1e-12` are
    /// treated as unchanged so a no-op leaves no entry; a NaN counts as changed so
    /// [`Plan::check_finite`] rejects it.
    pub fn push_part(&mut self, before: &PartState, after: &PartState) {
        debug_assert_eq!(before.id(), after.id());
        let changed = |d: f64| d.is_nan() || d > 1e-12;
        let (b, a) = (before.pose(), after.pose());
        if changed((a.pos - b.pos).length()) {
            self.changes.push(PropertyChange {
                id: before.id(),
                property: "Position",
                before: Value::Vector3(to_core(b.pos)),
                after: Value::Vector3(to_core(a.pos)),
            });
        }
        if changed(max_diff(&a.rot, &b.rot)) {
            self.changes.push(PropertyChange {
                id: before.id(),
                property: "Orientation",
                before: Value::Vector3(before.read_orientation),
                after: Value::Vector3(orientation_of(&a.rot)),
            });
        }
        if changed((after.snap.size - before.snap.size).length()) {
            self.changes.push(PropertyChange {
                id: before.id(),
                property: "Size",
                before: Value::Vector3(to_core(before.snap.size)),
                after: Value::Vector3(to_core(after.snap.size)),
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
    pub fn push_copy(&mut self, object: &Object, after: &[PartState]) {
        self.copies.push(PlannedCopy {
            source: object.id,
            parent: object.parent,
            parts: object
                .parts
                .iter()
                .zip(after)
                .map(|(src, a)| {
                    let pose = a.pose();
                    CopiedPart {
                        source_part: src.id(),
                        position: to_core(pose.pos),
                        orientation: orientation_of(&pose.rot),
                        size: to_core(a.snap.size),
                    }
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
