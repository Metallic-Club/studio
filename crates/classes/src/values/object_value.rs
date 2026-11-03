//! `ObjectValue`: a value object holding a reference to an instance.

use metallic_core::{ClassDesc, PropMeta};

use super::value_base::CATEGORY;

static PROPS: &[PropMeta] = &[PropMeta::reference("Value", "Instance")
    .group("Data")
    .order(0)
    .desc("The instance it points at, or nil. Starts nil, and reads nil once that instance is destroyed.")];

/// A value object holding a reference to an instance.
pub static CLASS: ClassDesc = ClassDesc::new("ObjectValue", Some("ValueBase"))
    .describe("Stores a reference to another instance under a name, such as the door a button opens.")
    .category(CATEGORY)
    .creatable()
    .props(PROPS);
