//! Arrange operations: align, distribute, mirror and arrays (building-tools spec sections 6.1,
//! 6.2, 7 and 8.2; issues #217 and #218).
//!
//! This is the engine-side core with no UI. [`read_objects`] turns the selected instance ids (the
//! last id is the active object) into [`Object`]s built from shared
//! [`PartSnapshot`](crate::tools::geom::PartSnapshot)s; every operation then takes those objects
//! and returns a [`Plan`]: property changes on existing parts plus planned copies. Nothing here
//! writes the world. Studio's editing layer (#10, with the preview overlay and batched
//! `SetProperties` of #213) turns a plan into a live preview while a popover is open and into one
//! undo step on commit.
//!
//! A Model (or Folder) in the selection is one object made of all its parts and moves as a rigid
//! unit; a part inside a selected Model is not moved twice. Every numeric option must be finite,
//! and arrays create at most [`array::MAX_CREATED_PARTS`] parts.
//!
//! ```
//! use metallic_core::{Value, Vec3, World};
//! use metallic_studio::tools::arrange::{
//!     AlignOptions, Axis, Edge, RelativeTo, Space, align, read_objects,
//! };
//!
//! let mut world = World::new_with_services();
//! let ws = world.root("Workspace").unwrap();
//! let a = world.create("Part", Some(ws)).unwrap();
//! let b = world.create("Part", Some(ws)).unwrap();
//! world.set(b, "Position", Value::Vector3(Vec3::new(0.0, 6.0, 0.0))).unwrap();
//! let opts = AlignOptions {
//!     axis: Axis::Y,
//!     edge: Edge::Min,
//!     relative: RelativeTo::Active,
//!     space: Space::World,
//! };
//! let plan = align(&read_objects(&world, &[a, b]).unwrap(), &opts).unwrap();
//! assert_eq!(plan.after(a, "Position"), Some(&Value::Vector3(Vec3::new(0.0, 6.0, 0.0))));
//! ```

pub mod align;
pub mod array;
pub mod math;
pub mod mirror;
pub mod path;
pub mod plan;
pub mod scene;

#[cfg(test)]
mod testing;

pub use align::{
    AlignOptions, DistributeMode, DistributeOptions, Edge, RelativeTo, Space, align, distribute,
};
pub use array::{
    AngleField, CountField, GridOptions, LinearOptions, OffsetIs, PathFill, PathOptions,
    RadialCentre, RadialOptions, along_path, grid, linear, radial,
};
pub use math::Axis;
pub use mirror::{MirrorOptions, MirrorPlane, mirror};
pub use plan::{ArrangeError, CopiedPart, Plan, PlannedCopy, PlannedGroup, PropertyChange};
pub use scene::{Object, ObjectKind, PartState, Shape, read_objects};
