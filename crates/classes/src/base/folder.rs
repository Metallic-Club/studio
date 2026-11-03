//! `Folder`: a plain container for organising instances.

use metallic_core::ClassDesc;

/// A container with no behaviour of its own.
pub static CLASS: ClassDesc = ClassDesc::new("Folder", Some("Instance"))
    .describe("A container for organising instances. It has no position and does nothing on its own.")
    .creatable();
