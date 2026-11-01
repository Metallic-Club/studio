//! DataModel: the class of `game`.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("DataModel", Some("ServiceProvider"))
    .flags(ClassFlags::ABSTRACT)
    .category("Services")
    // `game` stays a host object for now (design section 17), so no instance of this class exists.
    .describe("The root of a game: what `game` is.");
