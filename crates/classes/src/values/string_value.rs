//! `StringValue`: a value object holding text.

use metallic_core::{ClassDesc, PropMeta};

use super::value_base::CATEGORY;

static PROPS: &[PropMeta] = &[PropMeta::string("Value", "")
    .group("Data")
    .order(0)
    .desc("The stored text. Starts empty.")];

/// A value object holding text.
pub static CLASS: ClassDesc = ClassDesc::new("StringValue", Some("ValueBase"))
    .describe("Stores text under a name, such as a team name or a message to show.")
    .category(CATEGORY)
    .creatable()
    .props(PROPS);
