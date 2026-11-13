//! `Part`: the basic building block, a block, ball, cylinder or wedge.
//!
//! Everything about where it is and how it looks comes from `BasePart`; `Part` adds its `Shape` and the
//! `PrimitiveMesh` component, which the renderer implements by drawing the built-in mesh for that shape (M3).

use metallic_core::{ClassDesc, ComponentDesc, EnumDesc, PropMeta};

/// The enums `Part` declares.
pub static ENUMS: &[EnumDesc] = &[EnumDesc::new("PartShape", &["Block", "Ball", "Cylinder", "Wedge"])
    .describe("The shape of a Part: a box, a sphere, a cylinder or a ramp.")];

/// The component that draws a part as one of the built-in shapes. Declared here; the renderer implements it.
pub static COMPONENT: ComponentDesc = ComponentDesc::new("PrimitiveMesh")
    .describe("Draws the part as the built-in mesh for its Shape, stretched to its Size.")
    .requires(&["Transform"]);

static PROPS: &[PropMeta] = &[PropMeta::enumeration("Shape", "PartShape", "Block")
    .visual()
    .physics()
    // First in Appearance, before BasePart's Material (#589).
    .group("Appearance")
    .order(0)
    .desc(
        "The part's shape, stretched to its Size: a Ball whose sides differ is an ellipsoid; a Cylinder runs along X, \
         its ends the ellipse of the Y and Z sizes; a Wedge slopes down towards -Z.",
    )];

/// A basic building block.
pub static CLASS: ClassDesc = ClassDesc::new("Part", Some("BasePart"))
    .describe("The basic building block: a block, ball, cylinder or wedge that you size, colour and place.")
    .category("Parts")
    .creatable()
    .components(&["PrimitiveMesh"])
    .props(PROPS);

#[cfg(test)]
mod tests {
    use metallic_core::{EnumItem, Value};

    use crate::shared_registry;
    use crate::testing::{game, new_in, workspace};

    #[test]
    fn part_shape_has_its_items_and_defaults_to_block() {
        let registry = shared_registry().unwrap();
        let shape = registry.enum_desc("PartShape").unwrap();
        assert_eq!(shape.items, ["Block", "Ball", "Cylinder", "Wedge"]);

        let mut dm = game();
        let ws = workspace(&dm);
        let part = new_in(&mut dm, "Part", ws, "Ball");
        assert_eq!(
            dm.get_by_name(part, "Shape"),
            Ok(Value::Enum(EnumItem::new("PartShape", "Block")))
        );
        let ball = Value::Enum(EnumItem::new("PartShape", "Ball"));
        dm.set_by_name(part, "Shape", ball.clone()).unwrap();
        assert_eq!(dm.get_by_name(part, "Shape"), Ok(ball));
    }

    #[test]
    fn part_has_a_mesh_and_a_transform() {
        let registry = shared_registry().unwrap();
        let info = registry.class(registry.class_id("Part").unwrap()).unwrap();
        assert!(info.has_component("PrimitiveMesh"));
        assert!(info.has_component("Transform"));
    }
}
