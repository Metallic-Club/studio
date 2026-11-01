//! BasePart: everything physical in the world.
use crate::classes::prelude::*;

/// Materials a part may use.
pub const MATERIAL: EnumDesc = EnumDesc::new(
    "Material",
    &[
        "Plastic",
        "SmoothPlastic",
        "Metal",
        "Wood",
        "Concrete",
        "Glass",
        "Grass",
        "Brick",
        "Ice",
        "Sand",
    ],
);

/// Shapes a part may have.
const SHAPE: EnumDesc = EnumDesc::new("Shape", &["Block", "Ball", "Cylinder", "Wedge"]);

pub static CLASS: ClassDesc = ClassDesc::new("BasePart", Some("PVInstance"))
    .flags(ClassFlags::ABSTRACT)
    .category("Parts")
    .describe("A physical object in the world: position, size, look and collision.")
    .enums(&[MATERIAL, SHAPE])
    .props(&[
        PropDesc::new("Position", ValueType::Vector3, || {
            Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
        }),
        PropDesc::new("Orientation", ValueType::Vector3, || {
            Value::Vector3(Vec3::new(0.0, 0.0, 0.0))
        }),
        PropDesc::new("Size", ValueType::Vector3, || {
            Value::Vector3(Vec3::new(4.0, 1.0, 2.0))
        }),
        PropDesc::new("Color", ValueType::Color3, || Value::Color3(GREY)),
        enumerated("Material", || Value::String("Plastic".into()), &MATERIAL),
        PropDesc::new("Anchored", ValueType::Bool, || Value::Bool(false)),
        PropDesc::new("CanCollide", ValueType::Bool, || Value::Bool(true)),
        PropDesc::new("Transparency", ValueType::Number, || Value::Number(0.0)),
        enumerated("Shape", || Value::String("Block".into()), &SHAPE),
        PropDesc::new("PivotOffset", ValueType::CFrame, || {
            Value::CFrame(Box::new(CFrame::IDENTITY))
        }),
        look_override(),
    ]);
