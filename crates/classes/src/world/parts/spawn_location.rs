//! `SpawnLocation`: a part that marks where players appear (#198).
//!
//! It is a `Part` in every other way, so it is sized, coloured and placed like one and draws as its `Shape`.
//! `SpawnLocation` adds `Enabled`. Spawning characters on it belongs to the character and player work; teams and
//! `Duration` (the spawn shield) wait for multiplayer.

use metallic_core::{ClassDesc, PropMeta};

static PROPS: &[PropMeta] = &[PropMeta::boolean("Enabled", true).group("Behavior").order(0).desc(
    "Whether players can appear on this spawn location. A disabled one is an ordinary part.",
)];

/// A part players appear on.
pub static CLASS: ClassDesc = ClassDesc::new("SpawnLocation", Some("Part"))
    .describe("A part that marks where players appear when they join or respawn.")
    .category("Parts")
    .creatable()
    .props(PROPS);

#[cfg(test)]
mod tests {
    use metallic_core::Value;

    use crate::shared_registry;
    use crate::testing::{game, new_in, workspace};

    #[test]
    fn a_spawn_location_is_an_enabled_part() {
        let registry = shared_registry().unwrap();
        let info = registry.class(registry.class_id("SpawnLocation").unwrap()).unwrap();
        assert!(info.creatable());
        assert!(info.has_component("PrimitiveMesh"));
        let part = registry.class_id("Part").unwrap();
        assert!(info.ancestors().contains(&part));

        let mut dm = game();
        let ws = workspace(&dm);
        let spawn = new_in(&mut dm, "SpawnLocation", ws, "Spawn");
        assert_eq!(dm.get_by_name(spawn, "Enabled"), Ok(Value::Bool(true)));
        dm.set_by_name(spawn, "Enabled", Value::Bool(false)).unwrap();
        assert_eq!(dm.get_by_name(spawn, "Enabled"), Ok(Value::Bool(false)));
        // Inherited from Part and BasePart.
        assert!(dm.get_by_name(spawn, "Shape").is_ok());
        assert!(dm.get_by_name(spawn, "Size").is_ok());
    }
}
