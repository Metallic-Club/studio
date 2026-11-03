//! `Workspace`: the 3D world, and the camera the screen shows.

use metallic_core::{CFrame, ClassDesc, DataModel, Id, PropMeta, Value, Vector3};

use crate::GameError;

/// Where a new game's camera starts: a few metres up and back from the origin, looking at it.
const START_EYE: Vector3 = Vector3::new(0.0, 5.0, 10.0);

/// The service that holds the 3D world.
pub static CLASS: ClassDesc = ClassDesc::new("Workspace", Some("Instance"))
    .describe("The 3D world: everything placed here is part of the scene. Scripts placed here run.")
    .service(0)
    .persisted()
    .runs_scripts()
    .props(&[PropMeta::reference("CurrentCamera", "Camera")
        .group("Camera")
        .order(0)
        .desc("The camera the screen shows. A new game starts with one, named Camera, inside Workspace.")]);

/// Adds the starting `Camera` to a new game's `Workspace` and makes it the `CurrentCamera`.
pub(crate) fn add_start_camera(dm: &mut DataModel) -> Result<Id, GameError> {
    let class_id = |dm: &DataModel, name: &'static str| dm.registry().class_id(name).ok_or(GameError::MissingClass(name));
    let workspace = class_id(dm, CLASS.name)?;
    let workspace = dm.service(workspace).ok_or(GameError::MissingClass(CLASS.name))?;
    let camera = dm.create(class_id(dm, "Camera")?)?;
    dm.set_name(camera, "Camera".into())?;
    dm.set_by_name(
        camera,
        "CFrame",
        Value::CFrame(CFrame::look_at(START_EYE, Vector3::ZERO, Vector3::Y)),
    )?;
    dm.set_parent(camera, Some(workspace))?;
    dm.set_by_name(workspace, "CurrentCamera", Value::Ref(Some(camera)))?;
    Ok(camera)
}

#[cfg(test)]
mod tests {
    use metallic_core::Value;

    use crate::testing::{game, workspace};

    #[test]
    fn a_new_game_looks_through_a_camera_in_workspace() {
        let dm = game();
        let ws = workspace(&dm);
        let Ok(Value::Ref(Some(camera))) = dm.get_by_name(ws, "CurrentCamera") else {
            panic!("CurrentCamera is not set");
        };
        assert_eq!(dm.class_name(camera), Ok("Camera"));
        assert_eq!(dm.name(camera), Ok("Camera"));
        assert_eq!(dm.parent(camera), Ok(Some(ws)));
        let Ok(Value::CFrame(cframe)) = dm.get_by_name(camera, "CFrame") else {
            panic!("no CFrame");
        };
        assert!(cframe.look_vector().dot(-cframe.position()) > 0.0, "it looks at the origin");
    }

    #[test]
    fn current_camera_only_takes_cameras() {
        let mut dm = game();
        let ws = workspace(&dm);
        let part = crate::testing::new_in(&mut dm, "Part", ws, "Part");
        assert!(dm.set_by_name(ws, "CurrentCamera", Value::Ref(Some(part))).is_err());
    }
}
