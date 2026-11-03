//! `Model`: a group of world objects that belong together.
//!
//! A model has no `CFrame` of its own: its pivot comes from its `PrimaryPart`, or from the parts in it (see
//! `base::world_object`).

use metallic_core::{ClassDesc, PropMeta};

/// A group of world objects, such as a car or a building.
pub static CLASS: ClassDesc = ClassDesc::new("Model", Some("WorldObject"))
    .describe("A group of world objects that belong together, such as a car or a house, moved and copied as one.")
    .creatable()
    .props(&[PropMeta::reference("PrimaryPart", "BasePart")
        .group("Model")
        .order(0)
        .desc(
            "The part the model moves and rotates around. When it is nil (or not inside the model), the model \
             pivots on the centre of its parts.",
        )]);
