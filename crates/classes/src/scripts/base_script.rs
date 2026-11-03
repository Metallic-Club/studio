//! `BaseScript`: what `Script` and `LocalScript` share, a script that runs on its own when the game starts.

use metallic_core::{ClassDesc, PropMeta};

use super::source_container::GROUP;

static PROPS: &[PropMeta] = &[PropMeta::boolean("Enabled", true)
    .group(GROUP)
    .order(1)
    .desc("Whether the script runs. A disabled script does not start; turning it off later stops it.")];

/// The base of the scripts that run on their own.
pub static CLASS: ClassDesc = ClassDesc::new("BaseScript", Some("SourceContainer"))
    .describe("The base of Script and LocalScript: a script that runs on its own when the game starts, if enabled.")
    .props(PROPS);
