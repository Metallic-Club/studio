//! `WorldObject`: an instance with a place in the 3D world, such as a model, part, light or camera.
//!
//! `GetPivot` and `PivotTo` arrive with `BasePart` (#17): a model's pivot is computed from the parts it holds.

use metallic_core::{ClassDesc, PropMeta};

/// The base of everything that has a place in the 3D world.
pub static CLASS: ClassDesc = ClassDesc::new("WorldObject", Some("Instance"))
    .describe("The base of everything that has a position in the 3D world, such as models, parts, lights and cameras.")
    .props(&[PropMeta::cframe("PivotOffset").group("Transform").desc(
        "Where the object's pivot sits, relative to the object. Moving and rotating the object happens around its pivot.",
    )]);
