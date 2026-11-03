//! The games built into the Player, until projects load from disk (M4).

use std::fmt;
use std::str::FromStr;

use metallic_engine::MemorySources;
use metallic_engine::metallic_core::{DataModel, Value};

use crate::PlayerError;

/// The demo game's script: see the file for what it builds and its controls.
const DEMO: &str = include_str!("scenes/demo.luau");

/// A game built into the Player: what `--scene` picks.
///
/// Only the demo exists so far. The stress scenes of ADR 0008 §10 join it from #78, under their own names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scene {
    /// The M3 demo: every part shape and material, see-through glass, the three kinds of light, parts moved on
    /// `Heartbeat` and a camera driven by `UserInputService` (WASD, Q/E, the right mouse button, the arrow keys).
    #[default]
    Demo,
}

impl Scene {
    /// Every built-in scene, in the order `--help` lists them.
    pub const ALL: &[Scene] = &[Scene::Demo];

    /// The name `--scene` takes.
    pub fn name(self) -> &'static str {
        match self {
            Scene::Demo => "demo",
        }
    }

    /// The scene's script sources, by the path its scripts' `Source` names.
    pub fn sources(self) -> MemorySources {
        match self {
            Scene::Demo => MemorySources::new().with("Demo.luau", DEMO),
        }
    }

    /// Adds the scene's scripts to `dm`, for the engine to start with the game. The scripts build the rest.
    pub fn install(self, dm: &mut DataModel) -> Result<(), PlayerError> {
        match self {
            Scene::Demo => add_script(dm, "Demo", "Demo.luau"),
        }
    }
}

impl fmt::Display for Scene {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A `--scene` name that is not a built-in scene.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown scene `{0}` (scenes: {names})", names = scene_names())]
pub struct UnknownScene(pub String);

fn scene_names() -> String {
    Scene::ALL
        .iter()
        .map(|scene| scene.name())
        .collect::<Vec<_>>()
        .join(", ")
}

impl FromStr for Scene {
    type Err = UnknownScene;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Scene::ALL
            .iter()
            .copied()
            .find(|scene| scene.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| UnknownScene(name.to_owned()))
    }
}

/// Adds a `Script` called `name` running `source` to `ServerScriptService`.
fn add_script(dm: &mut DataModel, name: &str, source: &str) -> Result<(), PlayerError> {
    let registry = dm.registry().clone();
    let script_class = registry.class_id("Script").ok_or(PlayerError::MissingClass("Script"))?;
    let service = registry
        .class_id("ServerScriptService")
        .and_then(|class| dm.service(class))
        .ok_or(PlayerError::MissingClass("ServerScriptService"))?;
    let script = dm.create(script_class)?;
    dm.set_name(script, name.into())?;
    dm.set_by_name(script, "Source", Value::String(source.into()))?;
    dm.set_parent(script, Some(service))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_parse_by_name_whatever_the_case() {
        assert_eq!("demo".parse(), Ok(Scene::Demo));
        assert_eq!("Demo".parse(), Ok(Scene::Demo));
        let error = "stress".parse::<Scene>().unwrap_err();
        assert_eq!(error.to_string(), "unknown scene `stress` (scenes: demo)");
        for scene in Scene::ALL {
            assert_eq!(scene.name().parse(), Ok(*scene));
            assert_eq!(scene.to_string(), scene.name());
        }
    }
}
