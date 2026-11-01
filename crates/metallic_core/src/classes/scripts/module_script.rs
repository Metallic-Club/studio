//! ModuleScript: `require(moduleScript)` runs its `Source` once per script VM. It never runs by
//! itself, so it has no `Enabled`.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ModuleScript", Some("LuaSourceContainer"))
    .flags(ClassFlags::CREATABLE)
    .category("Scripts")
    .insert_order(130)
    .describe("A module scripts load with require; runs once per script VM.");
