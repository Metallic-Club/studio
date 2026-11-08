//! `Model`: a group of world objects that belong together.
//!
//! A model has no `CFrame` of its own: its pivot comes from its `PrimaryPart`, or else its `WorldPivot`, the centre of
//! the parts in it until first set (#413, see `base::world_object`). `WorldPivot` is kept in two hidden properties,
//! saved with the model: the frame, and whether it was ever set.

use metallic_core::{ClassDesc, CFrame, DataModel, Id, PropFlags, PropMeta, ScriptError, Value};

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
    ]);
