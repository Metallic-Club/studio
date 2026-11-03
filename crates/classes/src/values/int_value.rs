//! `IntValue`: a value object holding a whole number.

use metallic_core::{ClassDesc, PropMeta};

use super::value_base::CATEGORY;

static PROPS: &[PropMeta] = &[PropMeta::int("Value", 0)
    .group("Data")
    .order(0)
    .desc("The stored whole number. Starts at 0.")];

/// A value object holding a whole number.
pub static CLASS: ClassDesc = ClassDesc::new("IntValue", Some("ValueBase"))
    .describe("Stores a whole number under a name, such as a score or a count.")
    .category(CATEGORY)
    .creatable()
    .props(PROPS);
