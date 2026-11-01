//! PVInstance: anything with a pivot.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("PVInstance", Some("Instance"))
    .flags(ClassFlags::ABSTRACT)
    .category("Base")
    .describe("Anything with a pivot in the world: parts and models.")
    // Signatures only. `metallic_script/src/api/pivot.rs` still implements them (#465).
    .methods(&[
        MethodDesc::host("GetPivot")
            .returns(Ty::CFrame)
            .describe("The world position and rotation of the pivot."),
        MethodDesc::host("PivotTo")
            .params(&[Param::new("target", Ty::CFrame).manual()])
            .describe("Moves the instance so its pivot is at the target."),
    ]);
