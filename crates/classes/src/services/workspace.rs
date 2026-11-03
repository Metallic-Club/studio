//! `Workspace`: the 3D world.
//!
//! `CurrentCamera` arrives with `Camera` in #17.

use metallic_core::ClassDesc;

/// The service that holds the 3D world.
pub static CLASS: ClassDesc = ClassDesc::new("Workspace", Some("Instance"))
    .describe("The 3D world: everything placed here is part of the scene. Scripts placed here run.")
    .service(0)
    .persisted()
    .runs_scripts();
