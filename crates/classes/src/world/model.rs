//! `Model`: a group of world objects that belong together.
//!
//! A model has no `CFrame` of its own: its pivot comes from its `PrimaryPart`, or else its `WorldPivot`, the centre of
//! the parts in it until first set (#413, see `base::world_object`). `WorldPivot` is kept in two hidden properties,
//! saved with the model: the frame, and whether it was ever set. `Position`, `Rotation` and `Scale` (#439) are the
//! pivot's place and turn and the model's size, for Properties and scripts; the scale is kept in a hidden property.

use metallic_core::{
    CFrame, ClassDesc, DataModel, Id, PropFlags, PropMeta, ScriptError, ScriptValue, Value, Vector3, Widget,
};

use crate::base::world_object;

fn world_pivot_get(dm: &DataModel, id: Id) -> Value {
    Value::CFrame(world_object::world_pivot(dm, id).unwrap_or(CFrame::IDENTITY))
}

fn world_pivot_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    // The data model has checked the type before it runs a setter.
    let Value::CFrame(base) = value else {
        return Err(ScriptError::WrongType {
            expected: "CFrame".into(),
            got: "something else".into(),
        });
    };
    Ok(world_object::set_world_pivot(dm, id, base)?)
}

/// The smallest a model's `Scale` goes.
const MIN_SCALE: f64 = 0.001;

fn vector3_of(value: Value) -> Result<Vector3, ScriptError> {
    match value {
        Value::Vector3(v) => Ok(v),
        other => Err(ScriptError::wrong_type("Vector3", ScriptValue::from(other).type_name())),
    }
}

/// The model's pivot, or none for an empty model.
fn pivot_of(dm: &DataModel, id: Id) -> Option<CFrame> {
    world_object::pivot(dm, id).ok()
}

fn position_get(dm: &DataModel, id: Id) -> Value {
    Value::Vector3(pivot_of(dm, id).map_or(Vector3::ZERO, |pivot| pivot.position()))
}

fn position_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let position = vector3_of(value)?;
    let pivot = world_object::pivot(dm, id)?;
    world_object::pivot_to(dm, id, CFrame::from_parts(position, pivot.rotation()))
}

fn rotation_get(dm: &DataModel, id: Id) -> Value {
    Value::Vector3(pivot_of(dm, id).map_or(Vector3::ZERO, |pivot| pivot.to_orientation_deg()))
}

fn rotation_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let angles = vector3_of(value)?;
    let pivot = world_object::pivot(dm, id)?;
    let rotation = CFrame::from_orientation_deg(angles.x, angles.y, angles.z).rotation();
    world_object::pivot_to(dm, id, CFrame::from_parts(pivot.position(), rotation))
}

fn scale_of(dm: &DataModel, id: Id) -> f64 {
    match dm.get_by_name(id, "ScaleData") {
        Ok(Value::Number(scale)) if scale >= MIN_SCALE => scale,
        _ => 1.0,
    }
}

fn scale_get(dm: &DataModel, id: Id) -> Value {
    Value::Number(scale_of(dm, id))
}

fn scale_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let Value::Number(scale) = value else {
        return Err(ScriptError::wrong_type("number", ScriptValue::from(value).type_name()));
    };
    let scale = scale.max(MIN_SCALE);
    // An empty model has nothing to resize, but keeps its scale.
    if pivot_of(dm, id).is_some() {
        world_object::scale_model(dm, id, (scale / scale_of(dm, id)) as f32)?;
    }
    Ok(dm.set_by_name(id, "ScaleData", Value::Number(scale))?)
}

/// A group of world objects, such as a car or a building.
pub static CLASS: ClassDesc = ClassDesc::new("Model", Some("WorldObject"))
    .describe("A group of world objects that belong together, such as a car or a house, moved and copied as one.")
    .creatable()
    .props(&[
        PropMeta::reference("PrimaryPart", "BasePart")
            .group("Model")
            .order(0)
            .desc(
                "The part the model moves and rotates around. When it is nil (or not inside the model), the model \
                 pivots on its WorldPivot.",
            ),
        PropMeta::cframe("WorldPivot")
            .computed(world_pivot_get, Some(world_pivot_set))
            .derived_from(&["WorldPivotData", "WorldPivotSet"])
            .flags(PropFlags::NO_SAVE)
            .group("Model")
            .order(1)
            .desc(
                "Where the model moves and rotates around when it has no PrimaryPart. Until it is first set (by \
                 PivotTo, Studio's tools or a script) it is the centre of the model's parts; from then on it is kept, \
                 turning with the model.",
            ),
        PropMeta::cframe("WorldPivotData")
            .flags(PropFlags::HIDDEN)
            .desc("Where WorldPivot is kept once set."),
        PropMeta::boolean("WorldPivotSet", false)
            .flags(PropFlags::HIDDEN)
            .desc("Whether WorldPivot was ever set; until it is, it is the centre of the model's parts."),
        PropMeta::vector3("Position", 0.0, 0.0, 0.0)
            .computed(position_get, Some(position_set))
            .derived_from(&["PrimaryPart", "PivotOffset", "WorldPivotData", "WorldPivotSet"])
            .transform()
            .flags(PropFlags::NO_SAVE)
            .group("Transform")
            .order(0)
            .desc("Where the model's pivot is, in metres. Setting it moves the whole model without rotating it."),
        PropMeta::vector3("Rotation", 0.0, 0.0, 0.0)
            .computed(rotation_get, Some(rotation_set))
            .derived_from(&["PrimaryPart", "PivotOffset", "WorldPivotData", "WorldPivotSet"])
            .transform()
            .flags(PropFlags::NO_SAVE)
            .group("Transform")
            .order(1)
            .desc(
                "The model's rotation about its pivot, as angles in degrees about X, Y and Z, applied Z first, then X, \
                 then Y. Setting it turns the whole model in place.",
            ),
        PropMeta::number("Scale", 1.0)
            .computed(scale_get, Some(scale_set))
            .derived_from(&["ScaleData"])
            .range(MIN_SCALE, f64::INFINITY)
            // Dragged in hundredths, as there is no upper end to slide to (#452).
            .widget(Widget::Drag { speed: 0.01 })
            .transform()
            .flags(PropFlags::NO_SAVE)
            .group("Transform")
            .order(2)
            .desc(
                "How big the model is against its size when made, one number for all three axes: 2 is twice as big. \
                 Setting it resizes everything in the model about its pivot.",
            ),
        PropMeta::number("ScaleData", 1.0)
            .flags(PropFlags::HIDDEN)
            .desc("Where Scale is kept."),
    ]);

#[cfg(test)]
mod tests {
    use metallic_core::{CFrame, DataModel, Id, Value, Vector3};

    use crate::testing::{game, new_in, workspace};

    const EPS: f32 = 1e-4;

    fn vector(dm: &DataModel, id: Id, name: &str) -> Vector3 {
        match dm.get_by_name(id, name) {
            Ok(Value::Vector3(v)) => v,
            other => panic!("{name}: {other:?}"),
        }
    }

    fn number(dm: &DataModel, id: Id, name: &str) -> f64 {
        match dm.get_by_name(id, name) {
            Ok(Value::Number(n)) => n,
            other => panic!("{name}: {other:?}"),
        }
    }

    /// A model of two 1 × 2 × 1 parts at x = 0 and x = 4, and a light between them (at its centre, x = 2).
    fn model() -> (DataModel, Id, [Id; 3]) {
        let mut dm = game();
        let ws = workspace(&dm);
        let model = new_in(&mut dm, "Model", ws, "Model");
        let part = |dm: &mut DataModel, x: f32| {
            let id = new_in(dm, "Part", model, "Part");
            dm.set_by_name(id, "Size", Value::Vector3(Vector3::new(1.0, 2.0, 1.0))).unwrap();
            dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(Vector3::new(x, 0.0, 0.0))))
                .unwrap();
            id
        };
        let a = part(&mut dm, 0.0);
        let b = part(&mut dm, 4.0);
        let light = new_in(&mut dm, "PointLight", model, "Light");
        dm.set_by_name(light, "CFrame", Value::CFrame(CFrame::new(Vector3::new(2.0, 0.0, 0.0))))
            .unwrap();
        dm.set_by_name(light, "Range", Value::Number(10.0)).unwrap();
        (dm, model, [a, b, light])
    }

    #[test]
    fn position_and_rotation_read_and_move_the_pivot() {
        let (mut dm, model, [a, b, _]) = model();
        assert!(vector(&dm, model, "Position").abs_diff_eq(Vector3::new(2.0, 0.0, 0.0), EPS));
        assert!(vector(&dm, model, "Rotation").abs_diff_eq(Vector3::ZERO, EPS));
        dm.set_by_name(model, "Position", Value::Vector3(Vector3::new(2.0, 5.0, 0.0)))
            .unwrap();
        assert!(vector(&dm, a, "Position").abs_diff_eq(Vector3::new(0.0, 5.0, 0.0), EPS));
        // A quarter about Y, in place: the parts swing round the pivot.
        dm.set_by_name(model, "Rotation", Value::Vector3(Vector3::new(0.0, 90.0, 0.0)))
            .unwrap();
        assert!(vector(&dm, model, "Rotation").abs_diff_eq(Vector3::new(0.0, 90.0, 0.0), 1e-3));
        assert!(vector(&dm, model, "Position").abs_diff_eq(Vector3::new(2.0, 5.0, 0.0), EPS));
        assert!(vector(&dm, b, "Position").abs_diff_eq(Vector3::new(2.0, 5.0, -2.0), EPS));
        assert!(vector(&dm, b, "Orientation").abs_diff_eq(Vector3::new(0.0, 90.0, 0.0), 1e-3));
    }

    #[test]
    fn scale_resizes_everything_about_the_pivot_and_is_kept() {
        let (mut dm, model, [a, b, light]) = model();
        assert_eq!(number(&dm, model, "Scale"), 1.0);
        dm.set_by_name(model, "Scale", Value::Number(2.0)).unwrap();
        assert_eq!(number(&dm, model, "Scale"), 2.0);
        // Kept in a stored property, so it is saved.
        assert_eq!(number(&dm, model, "ScaleData"), 2.0);
        assert!(vector(&dm, a, "Size").abs_diff_eq(Vector3::new(2.0, 4.0, 2.0), EPS));
        assert!(vector(&dm, a, "Position").abs_diff_eq(Vector3::new(-2.0, 0.0, 0.0), EPS));
        assert!(vector(&dm, b, "Position").abs_diff_eq(Vector3::new(6.0, 0.0, 0.0), EPS));
        assert_eq!(number(&dm, light, "Range"), 20.0);
        // The pivot stays; one number for every axis, from where it is now.
        assert!(vector(&dm, model, "Position").abs_diff_eq(Vector3::new(2.0, 0.0, 0.0), EPS));
        dm.set_by_name(model, "Scale", Value::Number(0.5)).unwrap();
        assert!(vector(&dm, a, "Size").abs_diff_eq(Vector3::new(0.5, 1.0, 0.5), EPS));
        assert!(vector(&dm, b, "Position").abs_diff_eq(Vector3::new(3.0, 0.0, 0.0), EPS));
        assert_eq!(number(&dm, light, "Range"), 5.0);
        // Never 0 or less.
        dm.set_by_name(model, "Scale", Value::Number(-1.0)).unwrap();
        assert!(number(&dm, model, "Scale") > 0.0);
    }

    #[test]
    fn an_empty_model_keeps_its_scale_and_reads_zero() {
        let mut dm = game();
        let ws = workspace(&dm);
        let model = new_in(&mut dm, "Model", ws, "Empty");
        assert_eq!(vector(&dm, model, "Position"), Vector3::ZERO);
        dm.set_by_name(model, "Scale", Value::Number(3.0)).unwrap();
        assert_eq!(number(&dm, model, "Scale"), 3.0);
        assert!(dm.set_by_name(model, "Position", Value::Vector3(Vector3::ONE)).is_err());
    }
}
