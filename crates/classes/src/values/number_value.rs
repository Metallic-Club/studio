//! `NumberValue`: a value object holding a number.

use metallic_core::{ClassDesc, PropMeta};

use super::value_base::CATEGORY;

static PROPS: &[PropMeta] = &[PropMeta::number("Value", 0.0)
    .group("Data")
    .order(0)
    .desc("The stored number. Starts at 0.")];

/// A value object holding a number.
pub static CLASS: ClassDesc = ClassDesc::new("NumberValue", Some("ValueBase"))
    .describe("Stores a number under a name, such as a speed or an amount of health.")
    .category(CATEGORY)
    .creatable()
    .props(PROPS);
