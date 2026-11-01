//! Model: a group of parts with one pivot and scale
//! (docs/design/building-tools.md section 4.4, `metallic_core::transform`).
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Model", Some("PVInstance"))
    .flags(ClassFlags::CREATABLE)
    .category("Base")
    .describe("A group of parts that moves and scales as one, around one pivot.")
    .props(&[
        PropDesc::new("PrimaryPart", ValueType::Ref, || Value::Ref(None)),
        PropDesc::new("WorldPivot", ValueType::CFrame, || {
            Value::CFrame(Box::new(CFrame::IDENTITY))
        }),
        // False until WorldPivot is first set: the pivot is then the bounding-box centre.
        PropDesc::new("WorldPivotSet", ValueType::Bool, || Value::Bool(false)).hidden(),
        // What `GetScale` returns; `ScaleTo` changes it.
        PropDesc::new("ScaleFactor", ValueType::Number, || Value::Number(1.0)).hidden(),
        look_override(),
    ]);
