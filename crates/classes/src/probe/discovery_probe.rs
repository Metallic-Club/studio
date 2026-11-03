//! Discovery probe: a throwaway class, component and enum in a nested folder. They prove that `build.rs` finds all
//! three kinds of file and mirrors folders as modules, so adding a class needs no other edit.
//!
//! Temporary: #16 deletes this file (and the `probe` folder) when `Object` arrives. Until then `DiscoveryProbe` is
//! the root class.

use metallic_core::{ClassDesc, ComponentDesc, EnumDesc, PropMeta};

/// An enum the probe class uses, proving `ENUMS` lists are found.
pub static ENUMS: &[EnumDesc] = &[EnumDesc::new("DiscoveryProbeMode", &["Off", "On"])];

/// A component the probe class uses, proving `COMPONENT`s are found.
pub static COMPONENT: ComponentDesc =
    ComponentDesc::new("DiscoveryProbeComponent").props(&[PropMeta::boolean("ProbeFlag", false)]);

/// The probe class, proving `CLASS`es are found.
pub static CLASS: ClassDesc = ClassDesc::new("DiscoveryProbe", None)
    .describe("Proves that class discovery works. Removed by #16.")
    .components(&["DiscoveryProbeComponent"])
    .props(&[PropMeta::enumeration("Mode", "DiscoveryProbeMode", "On")]);
