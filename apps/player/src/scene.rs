//! The games built into the Player (projects on disk are [`PlayerConfig::project`](crate::PlayerConfig::project)).

use std::fmt;
use std::str::FromStr;

use metallic_engine::MemorySources;
use metallic_engine::metallic_core::{DataModel, Value};
use metallic_engine::stress::{StressKind, StressScene};

use crate::PlayerError;

/// The demo game's script: see the file for what it builds and its controls.
const DEMO: &str = include_str!("scenes/demo.luau");

/// The seed the stress scenes are built from, the same as the engine's `stress` example uses by default, so the
/// Player's `--bench` and the example measure the same scene.
pub const STRESS_SEED: u64 = 1;

/// A game built into the Player: what `--scene` picks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scene {
    /// The M3 demo: every part shape and material, see-through glass, the three kinds of light, parts moved on
    /// `Heartbeat` and a camera driven by `UserInputService` (WASD, Q/E, the right mouse button, the arrow keys).
    #[default]
    Demo,
    /// A stress scene of ADR 0008 §10 ([`metallic_engine::stress`]), built by code with no scripts; its moving
    /// parts are moved by the Player every frame.
    Stress(StressKind),
}

impl Scene {
    /// Every built-in scene, in the order `--help` lists them.
    pub const ALL: &[Scene] = &[
        Scene::Demo,
        Scene::Stress(StressKind::Parts10k),
        Scene::Stress(StressKind::Parts50k),
        Scene::Stress(StressKind::Parts100k),
        Scene::Stress(StressKind::ManyLights),
        Scene::Stress(StressKind::Overdraw),
        Scene::Stress(StressKind::Mobile),
        Scene::Stress(StressKind::SeeThrough10k),
        Scene::Stress(StressKind::Calibration),
    ];

    /// The name `--scene` takes.
    pub fn name(self) -> &'static str {
        match self {
            Scene::Demo => "demo",
            Scene::Stress(kind) => kind.name(),
        }
    }

    /// The window the scene opens by default, in logical pixels: a stress scene's own size, so its numbers compare
    /// with the engine's `stress` example.
    pub fn window_size(self) -> (u32, u32) {
        match self {
            Scene::Demo => (1280, 720),
            Scene::Stress(kind) => kind.window_size(),
        }
    }

    /// The scene's script sources, by the path its scripts' `Source` names.
    pub fn sources(self) -> MemorySources {
        match self {
            Scene::Demo => MemorySources::new().with("Demo.luau", DEMO),
            Scene::Stress(_) => MemorySources::new(),
        }
    }

    /// Adds the scene to `dm`: the demo's scripts (which build the rest when the game starts), or a stress scene's
    /// parts, returned so its moving parts can be moved every frame.
    pub fn install(self, dm: &mut DataModel) -> Result<Option<StressScene>, PlayerError> {
        match self {
            Scene::Demo => add_script(dm, "Demo", "Demo.luau").map(|()| None),
            Scene::Stress(kind) => Ok(Some(StressScene::build(dm, kind, STRESS_SEED)?)),
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

/// Every scene's name, comma-separated.
pub(crate) fn scene_names() -> String {
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
        assert_eq!("parts_100k".parse(), Ok(Scene::Stress(StressKind::Parts100k)));
        let error = "stress".parse::<Scene>().unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("unknown scene `stress` (scenes: demo, parts_10k, parts_50k"),
            "{error}"
        );
        for scene in Scene::ALL {
            assert_eq!(scene.name().parse(), Ok(*scene));
            assert_eq!(scene.to_string(), scene.name());
        }
        // Every stress scene is offered.
        for kind in StressKind::ALL {
            assert!(Scene::ALL.contains(&Scene::Stress(kind)), "{}", kind.name());
        }
    }
}
