//! `BoolValue`: a value object holding true or false.

use metallic_core::{ClassDesc, PropMeta};

use super::value_base::CATEGORY;

static PROPS: &[PropMeta] = &[PropMeta::boolean("Value", false)
    .group("Data")
    .order(0)
    .desc("The stored true or false. Starts false.")];

/// A value object holding true or false.
pub static CLASS: ClassDesc = ClassDesc::new("BoolValue", Some("ValueBase"))
    .describe("Stores true or false under a name, such as whether a door is open.")
    .category(CATEGORY)
    .creatable()
    .props(PROPS);
