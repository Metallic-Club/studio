//! The API: every built-in class and service, one file per class, grouped by domain.
//!
//! Owns: class declarations (`Object`, `Instance`, `WorldObject`, `Light`, `SpotLight`, `Workspace`, ...), their
//! properties, methods, signals and enums, and per-class behaviour that only touches the data model.
//!
//! Must not depend on: render, physics, scripting or platform crates. A class *names* the components it has
//! (`SpotLightEmitter`); the subsystem that implements a component registers itself under that name.
//! See `docs/adr/0002-object-model.md` and `docs/adr/0007-initial-api.md`.
//!
//! # Adding a class
//!
//! Add one file anywhere under `src/` that exports `pub static CLASS: ClassDesc` (or `COMPONENT`, or `ENUMS`).
//! `build.rs` finds it and adds it to [`input()`]; folders become nested modules, so
//! `src/world/lights/spot_light.rs` is `metallic_classes::world::lights::spot_light`. No other file changes.

use metallic_core::{Registry, RegistryErrors, RegistryInput};

include!(concat!(env!("OUT_DIR"), "/classes.rs"));

/// Every class, component and enum declared under `src/`, as found by `build.rs`.
pub fn input() -> RegistryInput<'static> {
    RegistryInput {
        classes: ALL_CLASSES,
        components: ALL_COMPONENTS,
        enums: ALL_ENUMS,
    }
}

/// Builds the registry of every built-in class, or lists every problem in their descriptors.
pub fn registry() -> Result<Registry, RegistryErrors> {
    Registry::build(input())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_builds_and_finds_the_probe() {
        let registry = registry().unwrap_or_else(|e| panic!("{e}"));
        let probe = registry.class_id("DiscoveryProbe").expect("probe class discovered");
        let info = registry.class(probe).expect("probe class");
        assert!(info.has_component("DiscoveryProbeComponent"));
        assert!(registry.prop_id(probe, "ProbeFlag").is_some());
        assert_eq!(
            registry.enum_item("DiscoveryProbeMode", "On"),
            Some(metallic_core::EnumItem::new("DiscoveryProbeMode", "On"))
        );
    }

    #[test]
    fn folders_become_nested_modules() {
        // `src/probe/discovery_probe.rs` is reachable at the path its folders spell out.
        assert_eq!(crate::probe::discovery_probe::CLASS.name, "DiscoveryProbe");
        assert!(
            input()
                .classes
                .iter()
                .any(|c| std::ptr::eq(*c, &crate::probe::discovery_probe::CLASS))
        );
        assert_eq!(input().components.len(), 1);
        assert_eq!(input().enums.len(), 1);
    }
}
