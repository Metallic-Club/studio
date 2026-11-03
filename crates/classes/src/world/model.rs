//! `Model`: a group of world objects that belong together.
//!
//! `PrimaryPart` (a reference to a `BasePart`) arrives with `BasePart` in #17; that class does not exist yet.

use metallic_core::ClassDesc;

/// A group of world objects, such as a car or a building.
pub static CLASS: ClassDesc = ClassDesc::new("Model", Some("WorldObject"))
    .describe("A group of world objects that belong together, such as a car or a house, moved and copied as one.")
    .creatable();
