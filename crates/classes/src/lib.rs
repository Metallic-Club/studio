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
//!
//! # Starting a game
//!
//! [`new_game()`] returns a [`DataModel`] with the root (`game`) and every service already created.

use std::sync::{Arc, OnceLock};

use metallic_core::{DataModel, DmError, Registry, RegistryErrors, RegistryInput};
use thiserror::Error;

include!(concat!(env!("OUT_DIR"), "/classes.rs"));

/// Every class, component and enum declared under `src/`, as found by `build.rs`.
pub fn input() -> RegistryInput<'static> {
    RegistryInput {
        classes: ALL_CLASSES,
        components: ALL_COMPONENTS,
        enums: ALL_ENUMS,
    }
}

/// Builds a new registry of every built-in class, or lists every problem in their descriptors.
pub fn registry() -> Result<Registry, RegistryErrors> {
    Registry::build(input())
}

/// The registry of every built-in class, built on first use and then shared by every game.
pub fn shared_registry() -> Result<Arc<Registry>, RegistryErrors> {
    static SHARED: OnceLock<Arc<Registry>> = OnceLock::new();
    if let Some(registry) = SHARED.get() {
        return Ok(Arc::clone(registry));
    }
    // Another thread may build one at the same time; whichever is stored first is the one everybody shares.
    let built = Arc::new(registry()?);
    Ok(Arc::clone(SHARED.get_or_init(|| built)))
}

/// Why [`new_game()`] could not start a game.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum GameError {
    /// The built-in class descriptors are inconsistent.
    #[error(transparent)]
    Registry(#[from] RegistryErrors),
    /// The data model refused to create the root or a service.
    #[error(transparent)]
    DataModel(#[from] DmError),
    /// The registry has no root class.
    #[error("there is no `{0}` class to be the root of the game")]
    NoRootClass(&'static str),
    /// The registry lacks a class (or service) that every new game contains.
    #[error("there is no `{0}`, which every new game needs")]
    MissingClass(&'static str),
}

/// A new, empty game: the root (`DataModel`, `game` in scripts) with every service under it, in service order, and
/// a `Camera` in `Workspace` as its `CurrentCamera`.
pub fn new_game() -> Result<DataModel, GameError> {
    let registry = shared_registry()?;
    let root_name = services::data_model::CLASS.name;
    let root = registry.class_id(root_name).ok_or(GameError::NoRootClass(root_name))?;
    let services = registry.services().to_vec();
    let mut dm = DataModel::new(registry);
    dm.create_root(root)?;
    for service in services {
        dm.create_service(service)?;
    }
    services::workspace::add_start_camera(&mut dm)?;
    Ok(dm)
}

/// Helpers the class files' tests share.
#[cfg(test)]
pub(crate) mod testing {
    use metallic_core::{Args, DataModel, Id, MethodCall, ScriptError, ScriptValue};

    /// A new game, or a panic naming what failed.
    pub(crate) fn game() -> DataModel {
        crate::new_game().unwrap_or_else(|e| panic!("{e}"))
    }

    /// The game's `Workspace`.
    pub(crate) fn workspace(dm: &DataModel) -> Id {
        let class = dm.registry().class_id("Workspace").expect("Workspace class");
        dm.service(class).expect("Workspace service")
    }

    /// A new instance of `class` named `name` under `parent`.
    pub(crate) fn new_in(dm: &mut DataModel, class: &str, parent: Id, name: &str) -> Id {
        let class = dm.registry().class_id(class).expect("class exists");
        let id = dm.create(class).expect("create");
        dm.set_name(id, name.into()).expect("name");
        dm.set_parent(id, Some(parent)).expect("parent");
        id
    }

    /// Calls `method` on `id` through its `MethodDesc`, as a script backend would.
    pub(crate) fn call(
        dm: &mut DataModel,
        id: Id,
        method: &str,
        args: Vec<ScriptValue>,
    ) -> Result<ScriptValue, ScriptError> {
        let class = dm.class_of(id)?;
        let desc = dm
            .registry()
            .method(class, method)
            .ok_or_else(|| ScriptError::runtime(format!("{method} is not a method of this instance")))?;
        (desc.func)(&mut MethodCall {
            this: id,
            args: Args::new(args),
            dm,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use metallic_core::{ScriptValue, Value};

    use super::*;
    use crate::testing::{call, game, new_in, workspace};

    #[test]
    fn model_inherits_object_instance_world_object() {
        let registry = shared_registry().unwrap();
        let model = registry.class_id("Model").unwrap();
        let chain: Vec<&str> = registry
            .class(model)
            .unwrap()
            .ancestors()
            .iter()
            .map(|&c| registry.class(c).unwrap().name())
            .collect();
        assert_eq!(chain, ["Object", "Instance", "WorldObject", "Model"]);
        for member in ["ClassName", "Name", "Parent", "Archivable", "PivotOffset"] {
            assert!(registry.prop_id(model, member).is_some(), "{member}");
        }
        for method in ["IsA", "Destroy", "GetChildren", "SetAttribute"] {
            assert!(registry.method(model, method).is_some(), "{method}");
        }
        for signal in ["Changed", "ChildAdded", "Destroying"] {
            assert!(registry.signal(model, signal).is_some(), "{signal}");
        }
    }

    #[test]
    fn is_a_works_up_the_chain() {
        let mut dm = game();
        let ws = workspace(&dm);
        let model = new_in(&mut dm, "Model", ws, "M");
        for (class, expected) in [
            ("Model", true),
            ("WorldObject", true),
            ("Instance", true),
            ("Object", true),
            ("Folder", false),
            ("Workspace", false),
        ] {
            assert_eq!(
                call(&mut dm, model, "IsA", vec![class.into()]),
                Ok(ScriptValue::Bool(expected)),
                "IsA({class})"
            );
        }
        let error = call(&mut dm, model, "IsA", vec!["Foo".into()]).unwrap_err();
        assert_eq!(error.to_string(), "unknown class 'Foo'");
        assert!(call(&mut dm, model, "IsA", vec![]).is_err());
    }

    #[test]
    fn new_game_has_the_services_in_order() {
        let dm = game();
        let root = dm.root().unwrap();
        assert_eq!(dm.class_name(root), Ok("DataModel"));
        let names: Vec<&str> = dm
            .children(root)
            .unwrap()
            .iter()
            .map(|&s| dm.class_name(s).unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "Workspace",
                "Lighting",
                "ReplicatedStorage",
                "ServerScriptService",
                "ServerStorage"
            ]
        );
    }

    #[test]
    fn full_name_starts_at_the_service() {
        let mut dm = game();
        let ws = workspace(&dm);
        let folder = new_in(&mut dm, "Folder", ws, "Folder");
        assert_eq!(dm.full_name(folder).as_deref(), Ok("Workspace.Folder"));
    }

    #[test]
    fn games_share_one_registry() {
        let (a, b) = (game(), game());
        assert!(Arc::ptr_eq(a.registry(), b.registry()));
    }

    #[test]
    fn lighting_has_its_defaults() {
        let dm = game();
        let lighting = dm.registry().class_id("Lighting").unwrap();
        let lighting = dm.service(lighting).unwrap();
        assert_eq!(dm.get_by_name(lighting, "Brightness"), Ok(Value::Number(2.0)));
        assert_eq!(dm.get_by_name(lighting, "ClockTime"), Ok(Value::Number(14.0)));
    }

    #[test]
    fn every_class_and_member_is_described() {
        let registry = shared_registry().unwrap();
        let mut missing = Vec::new();
        for info in registry.classes() {
            let desc = info.desc();
            if desc.description.is_empty() {
                missing.push(format!("class {}", desc.name));
            }
            for prop in desc.props {
                if prop.description.is_empty() {
                    missing.push(format!("property {}.{}", desc.name, prop.name));
                }
            }
            for method in desc.methods {
                if method.description.is_empty() {
                    missing.push(format!("method {}:{}", desc.name, method.name));
                }
            }
            for signal in desc.signals {
                if signal.description.is_empty() {
                    missing.push(format!("signal {}.{}", desc.name, signal.name));
                }
            }
        }
        for component in input().components {
            if component.description.is_empty() {
                missing.push(format!("component {}", component.name));
            }
        }
        for desc in registry.enums() {
            if desc.description.is_empty() {
                missing.push(format!("enum {}", desc.name));
            }
        }
        assert!(missing.is_empty(), "undescribed: {missing:?}");
    }

    #[test]
    fn every_class_file_is_discovered() {
        assert!(
            input()
                .classes
                .iter()
                .any(|c| std::ptr::eq(*c, &crate::world::lights::spot_light::CLASS))
        );
        assert_eq!(input().classes.len(), 24);
        let mut components: Vec<&str> = input().components.iter().map(|c| c.name).collect();
        components.sort_unstable();
        assert_eq!(
            components,
            [
                "DirectionalLightEmitter",
                "PointLightEmitter",
                "PrimitiveMesh",
                "SpotLightEmitter",
                "Transform"
            ]
        );
        assert_eq!(input().enums.concat().len(), 2, "Material and PartShape");
    }
}
