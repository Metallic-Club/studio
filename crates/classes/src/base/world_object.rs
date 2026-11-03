//! `WorldObject`: an instance with a place in the 3D world, such as a model, part, light or camera.
//!
//! # Pivots
//!
//! Every world object moves and rotates around its *pivot*: a [`CFrame`] in world space, returned by `GetPivot()`.
//! It is the object's base frame with its `PivotOffset` applied (`base * PivotOffset`). The base frame is:
//!
//! - a `BasePart`, `Camera` or `Light`: its world `CFrame` (a light inside a part goes through the part, see
//!   [`light::world_cframe`]);
//! - a `Model`: its `PrimaryPart`'s pivot, when that is set and still inside the model; otherwise the centre of the
//!   axis-aligned box around every part in it, with no rotation;
//! - anything else: there is none, and `GetPivot` / `PivotTo` fail with a message saying so.
//!
//! `PivotTo(target)` moves the object so that its pivot lands on `target`. A model moves rigidly: every object in it
//! with its own world transform (parts, cameras, and lights that are not inside a part) gets the same move, so the
//! distances and angles between them do not change. A model without a `PrimaryPart` has no stored rotation, so its
//! pivot is recomputed from its parts afterwards: a pure move lands exactly, but after a rotation the box around the
//! turned parts is generally different. Set `PrimaryPart` when a model's pivot must be exact.

use metallic_core::{
    CFrame, ClassDesc, DataModel, DmError, Id, MethodCall, MethodDesc, Param, PropMeta, ScriptError, ScriptValue, Ty,
    Value, Vector3,
};

use crate::world::lights::light;

/// Whether `id` is a `class` (or inherits from it). `false` when the registry has no such class.
pub(crate) fn is_a_named(dm: &DataModel, id: Id, class: &str) -> Result<bool, DmError> {
    match dm.registry().class_id(class) {
        Some(class) => dm.is_a(id, class),
        None => Ok(false),
    }
}

/// How an object's base frame is found.
enum Placement {
    /// A part or camera: its `CFrame` is in world space.
    Frame,
    /// A light: its `CFrame` may be relative to the part it sits in.
    Light,
    /// A model: computed from the parts in it.
    Model,
}

/// How `id` is placed, or `None` when it has no geometry of its own.
fn placement(dm: &DataModel, id: Id) -> Result<Option<Placement>, DmError> {
    Ok(if is_a_named(dm, id, "BasePart")? || is_a_named(dm, id, "Camera")? {
        Some(Placement::Frame)
    } else if is_a_named(dm, id, "Light")? {
        Some(Placement::Light)
    } else if is_a_named(dm, id, "Model")? {
        Some(Placement::Model)
    } else {
        None
    })
}

/// The `CFrame` property of `id`.
fn cframe_prop(dm: &DataModel, id: Id, name: &str) -> Result<CFrame, DmError> {
    dm.cframe(id, dm.prop_id(id, name)?)
}

/// The error for an object with no pivot.
fn no_pivot(dm: &DataModel, id: Id, why: &str) -> ScriptError {
    let class = dm.class_name(id).unwrap_or("WorldObject");
    let name = dm.name(id).unwrap_or_default();
    ScriptError::runtime(format!("{class} '{name}' has no pivot: {why}"))
}

/// The world frame of an object that has its own (`Frame` or `Light`).
fn own_world_frame(dm: &DataModel, id: Id, placement: &Placement) -> Result<CFrame, DmError> {
    match placement {
        Placement::Light => light::world_cframe(dm, id),
        Placement::Frame | Placement::Model => cframe_prop(dm, id, "CFrame"),
    }
}

/// The centre of the axis-aligned box around every `BasePart` under `model`, or `None` when there are none.
fn parts_centre(dm: &DataModel, model: Id) -> Result<Option<Vector3>, DmError> {
    let mut bounds: Option<(Vector3, Vector3)> = None;
    for part in dm.descendants(model)? {
        if !is_a_named(dm, part, "BasePart")? {
            continue;
        }
        let cframe = cframe_prop(dm, part, "CFrame")?;
        let half = dm.vector3(part, dm.prop_id(part, "Size")?)? * 0.5;
        for corner in 0..8 {
            let sign = |bit: u32| if corner & bit == 0 { -1.0 } else { 1.0 };
            let point = cframe.transform_point(half * Vector3::new(sign(1), sign(2), sign(4)));
            bounds = Some(match bounds {
                Some((min, max)) => (min.min(point), max.max(point)),
                None => (point, point),
            });
        }
    }
    Ok(bounds.map(|(min, max)| (min + max) * 0.5))
}

/// The base frame of a model, before its `PivotOffset`.
fn model_frame(dm: &DataModel, model: Id) -> Result<CFrame, ScriptError> {
    let primary = dm.reference(model, dm.prop_id(model, "PrimaryPart")?)?;
    if let Some(part) = primary
        && dm.is_descendant_of(part, model)?
    {
        return pivot(dm, part);
    }
    match parts_centre(dm, model)? {
        Some(centre) => Ok(CFrame::new(centre)),
        None => Err(no_pivot(dm, model, "it holds no parts; add a part or set PrimaryPart")),
    }
}

/// The world-space pivot of a world object: its base frame times its `PivotOffset` (see the module docs).
///
/// Fails for an object with no geometry: an empty model, or a world object that is not a part, light, camera or
/// model.
pub fn pivot(dm: &DataModel, id: Id) -> Result<CFrame, ScriptError> {
    let base = match placement(dm, id)? {
        Some(Placement::Model) => model_frame(dm, id)?,
        Some(own) => own_world_frame(dm, id, &own)?,
        None => return Err(no_pivot(dm, id, "it has no position of its own")),
    };
    Ok(base * cframe_prop(dm, id, "PivotOffset")?)
}

/// Writes the world frame of an object that has its own.
fn set_own_world_frame(dm: &mut DataModel, id: Id, placement: &Placement, world: CFrame) -> Result<(), DmError> {
    match placement {
        Placement::Light => light::set_world_cframe(dm, id, world),
        Placement::Frame | Placement::Model => {
            let prop = dm.prop_id(id, "CFrame")?;
            dm.set(id, prop, Value::CFrame(world))
        }
    }
}

/// Moves a world object so that its pivot lands on `target` (see the module docs). A model's contents move
/// rigidly.
pub fn pivot_to(dm: &mut DataModel, id: Id, target: CFrame) -> Result<(), ScriptError> {
    let current = pivot(dm, id)?;
    match placement(dm, id)? {
        Some(Placement::Model) => {
            let delta = target * current.inverse();
            let mut moved = Vec::new();
            for object in dm.descendants(id)? {
                let Some(kind) = placement(dm, object)? else {
                    continue;
                };
                // A light inside a part follows the part, so moving it too would move it twice. A nested model
                // moves with its parts.
                let follows_part = matches!(kind, Placement::Light) && light::host_part(dm, object)?.is_some();
                if !matches!(kind, Placement::Model) && !follows_part {
                    moved.push((object, own_world_frame(dm, object, &kind)?, kind));
                }
            }
            for (object, frame, kind) in moved {
                set_own_world_frame(dm, object, &kind, delta * frame)?;
            }
            Ok(())
        }
        Some(own) => {
            let offset = cframe_prop(dm, id, "PivotOffset")?;
            Ok(set_own_world_frame(dm, id, &own, target * offset.inverse())?)
        }
        // `pivot` has already failed for an object with no placement.
        None => Ok(()),
    }
}

fn get_pivot(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok(ScriptValue::CFrame(pivot(call.dm, call.this)?))
}

fn pivot_to_method(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let target = call.args.cframe(0)?;
    pivot_to(call.dm, call.this, target)?;
    Ok(ScriptValue::Nil)
}

/// The base of everything that has a place in the 3D world.
pub static CLASS: ClassDesc = ClassDesc::new("WorldObject", Some("Instance"))
    .describe("The base of everything that has a position in the 3D world, such as models, parts, lights and cameras.")
    .props(&[PropMeta::cframe("PivotOffset").group("Transform").desc(
        "Where the object's pivot sits, relative to the object. Moving and rotating the object happens around its pivot.",
    )])
    .methods(&[
        MethodDesc::new("GetPivot", get_pivot)
            .params(&[])
            .returns(Ty::CFrame)
            .describe(
                "Returns the point the object moves and rotates around, as a CFrame in world space. For a model \
                 without a PrimaryPart it is the centre of its parts.",
            ),
        MethodDesc::new("PivotTo", pivot_to_method)
            .params(&[Param::new("targetCFrame", Ty::CFrame)])
            .returns_nothing()
            .describe(
                "Moves the object so that its pivot is at the given CFrame. A model moves everything in it together, \
                 keeping their positions relative to each other.",
            ),
    ]);

#[cfg(test)]
mod tests {
    use metallic_core::{CFrame, DataModel, Id, ScriptValue, Value, Vector3};

    use super::{pivot, pivot_to};
    use crate::testing::{call, game, new_in, workspace};

    const EPS: f32 = 1e-4;

    fn place(dm: &mut DataModel, id: Id, cframe: CFrame) {
        dm.set_by_name(id, "CFrame", Value::CFrame(cframe)).unwrap();
    }

    fn cframe(dm: &DataModel, id: Id) -> CFrame {
        match dm.get_by_name(id, "CFrame") {
            Ok(Value::CFrame(cf)) => cf,
            other => panic!("{other:?}"),
        }
    }

    fn at(x: f32, y: f32, z: f32) -> CFrame {
        CFrame::new(Vector3::new(x, y, z))
    }

    #[test]
    fn a_parts_pivot_is_its_cframe_with_the_offset() {
        let mut dm = game();
        let ws = workspace(&dm);
        let part = new_in(&mut dm, "Part", ws, "Part");
        place(&mut dm, part, at(1.0, 2.0, 3.0));
        assert_eq!(
            call(&mut dm, part, "GetPivot", vec![]),
            Ok(ScriptValue::CFrame(at(1.0, 2.0, 3.0)))
        );

        dm.set_by_name(part, "PivotOffset", Value::CFrame(at(0.0, -0.5, 0.0)))
            .unwrap();
        assert!(pivot(&dm, part).unwrap().approx_eq(at(1.0, 1.5, 3.0), EPS));

        let target = at(10.0, 0.0, 0.0) * CFrame::from_orientation_deg(0.0, 90.0, 0.0);
        call(&mut dm, part, "PivotTo", vec![ScriptValue::CFrame(target)]).unwrap();
        assert!(pivot(&dm, part).unwrap().approx_eq(target, EPS));
        // The part's centre sits half a metre above the pivot.
        assert!(cframe(&dm, part).position().abs_diff_eq(Vector3::new(10.0, 0.5, 0.0), EPS));
    }

    #[test]
    fn a_model_without_a_primary_part_pivots_at_the_centre_of_its_parts() {
        let mut dm = game();
        let ws = workspace(&dm);
        let model = new_in(&mut dm, "Model", ws, "Model");
        let a = new_in(&mut dm, "Part", model, "A");
        let folder = new_in(&mut dm, "Folder", model, "Nested");
        let b = new_in(&mut dm, "Part", folder, "B");
        // Size (4, 1, 2): A spans x -2..2, B spans x 8..12, so the box is x -2..12, y -0.5..4.5, z -1..1.
        place(&mut dm, a, at(0.0, 0.0, 0.0));
        place(&mut dm, b, at(10.0, 4.0, 0.0));
        assert!(pivot(&dm, model).unwrap().approx_eq(at(5.0, 2.0, 0.0), EPS));

        // A pure move lands exactly, and the parts keep their offsets.
        call(&mut dm, model, "PivotTo", vec![ScriptValue::CFrame(at(0.0, 10.0, 0.0))]).unwrap();
        assert!(pivot(&dm, model).unwrap().approx_eq(at(0.0, 10.0, 0.0), EPS));
        assert!(cframe(&dm, a).approx_eq(at(-5.0, 8.0, 0.0), EPS));
        assert!(cframe(&dm, b).approx_eq(at(5.0, 12.0, 0.0), EPS));
    }

    #[test]
    fn a_model_with_a_primary_part_pivots_on_it_and_moves_rigidly() {
        let mut dm = game();
        let ws = workspace(&dm);
        let model = new_in(&mut dm, "Model", ws, "Car");
        let body = new_in(&mut dm, "Part", model, "Body");
        let wheel = new_in(&mut dm, "Part", model, "Wheel");
        let lamp = new_in(&mut dm, "PointLight", model, "Lamp");
        let bulb = new_in(&mut dm, "PointLight", body, "Bulb");
        place(&mut dm, body, at(0.0, 1.0, 0.0));
        place(&mut dm, wheel, at(2.0, 0.0, 1.0));
        place(&mut dm, lamp, at(0.0, 3.0, -2.0));
        place(&mut dm, bulb, at(0.0, 0.5, 0.0));
        dm.set_by_name(model, "PrimaryPart", Value::Ref(Some(body))).unwrap();
        assert!(pivot(&dm, model).unwrap().approx_eq(at(0.0, 1.0, 0.0), EPS));

        let target = at(100.0, 5.0, -20.0) * CFrame::from_orientation_deg(0.0, 90.0, 0.0);
        pivot_to(&mut dm, model, target).unwrap();
        assert!(pivot(&dm, model).unwrap().approx_eq(target, EPS));
        assert!(cframe(&dm, body).approx_eq(target, EPS));
        // Everything kept its place relative to the body.
        let body_cf = cframe(&dm, body);
        assert!((body_cf.inverse() * cframe(&dm, wheel)).approx_eq(at(2.0, -1.0, 1.0), EPS));
        assert!((body_cf.inverse() * cframe(&dm, lamp)).approx_eq(at(0.0, 2.0, -2.0), EPS));
        // The bulb is relative to the body and was not moved again.
        assert_eq!(cframe(&dm, bulb), at(0.0, 0.5, 0.0));
    }

    #[test]
    fn a_models_offset_applies_on_top_of_its_base() {
        let mut dm = game();
        let ws = workspace(&dm);
        let model = new_in(&mut dm, "Model", ws, "Model");
        let part = new_in(&mut dm, "Part", model, "Part");
        place(&mut dm, part, at(1.0, 1.0, 1.0));
        dm.set_by_name(model, "PivotOffset", Value::CFrame(at(0.0, 2.0, 0.0)))
            .unwrap();
        assert!(pivot(&dm, model).unwrap().approx_eq(at(1.0, 3.0, 1.0), EPS));
        pivot_to(&mut dm, model, at(0.0, 0.0, 0.0)).unwrap();
        assert!(cframe(&dm, part).approx_eq(at(0.0, -2.0, 0.0), EPS));
    }

    #[test]
    fn a_primary_part_outside_the_model_is_ignored() {
        let mut dm = game();
        let ws = workspace(&dm);
        let model = new_in(&mut dm, "Model", ws, "Model");
        let inside = new_in(&mut dm, "Part", model, "Inside");
        let outside = new_in(&mut dm, "Part", ws, "Outside");
        place(&mut dm, inside, at(3.0, 0.0, 0.0));
        place(&mut dm, outside, at(50.0, 0.0, 0.0));
        dm.set_by_name(model, "PrimaryPart", Value::Ref(Some(outside))).unwrap();
        assert!(pivot(&dm, model).unwrap().approx_eq(at(3.0, 0.0, 0.0), EPS));
    }

    #[test]
    fn lights_and_cameras_pivot_on_their_cframe() {
        let mut dm = game();
        let ws = workspace(&dm);
        let spot = new_in(&mut dm, "SpotLight", ws, "Spot");
        place(&mut dm, spot, at(0.0, 10.0, 0.0));
        assert!(pivot(&dm, spot).unwrap().approx_eq(at(0.0, 10.0, 0.0), EPS));
        pivot_to(&mut dm, spot, at(5.0, 5.0, 5.0)).unwrap();
        assert!(cframe(&dm, spot).approx_eq(at(5.0, 5.0, 5.0), EPS));

        // Inside a part, the pivot is in world space and PivotTo writes the CFrame relative to the part.
        let part = new_in(&mut dm, "Part", ws, "Post");
        place(&mut dm, part, at(20.0, 0.0, 0.0));
        dm.set_parent(spot, Some(part)).unwrap();
        assert!(pivot(&dm, spot).unwrap().approx_eq(at(25.0, 5.0, 5.0), EPS));
        pivot_to(&mut dm, spot, at(20.0, 4.0, 0.0)).unwrap();
        assert!(cframe(&dm, spot).approx_eq(at(0.0, 4.0, 0.0), EPS));
        assert_eq!(cframe(&dm, part), at(20.0, 0.0, 0.0), "the part stays");

        let camera = new_in(&mut dm, "Camera", ws, "Cam");
        call(&mut dm, camera, "PivotTo", vec![ScriptValue::CFrame(at(1.0, 2.0, 3.0))]).unwrap();
        assert_eq!(cframe(&dm, camera), at(1.0, 2.0, 3.0));
    }

    #[test]
    fn objects_without_geometry_have_no_pivot() {
        let mut dm = game();
        let ws = workspace(&dm);
        let model = new_in(&mut dm, "Model", ws, "Empty");
        let error = call(&mut dm, model, "GetPivot", vec![]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Model 'Empty' has no pivot: it holds no parts; add a part or set PrimaryPart"
        );
        let target = vec![ScriptValue::CFrame(CFrame::IDENTITY)];
        assert!(call(&mut dm, model, "PivotTo", target).is_err());
        assert!(call(&mut dm, model, "PivotTo", vec![]).is_err(), "PivotTo needs a CFrame");
    }
}
