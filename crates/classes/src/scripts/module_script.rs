//! `ModuleScript`: code shared between scripts through `require`.

use metallic_core::ClassDesc;

use super::source_container::CATEGORY;

/// A script that runs when it is first required.
pub static CLASS: ClassDesc = ClassDesc::new("ModuleScript", Some("SourceContainer"))
    .describe(
        "Code shared between scripts. It runs the first time a script requires it and returns a value, usually \
         a table of functions; later requires return that same value.",
    )
    .category(CATEGORY)
    .creatable();
