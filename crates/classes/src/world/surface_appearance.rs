//! `SurfaceAppearance` (#807): a MeshPart's own physically based look, as Roblox's: put inside a MeshPart, its maps
//! (colour, normal, roughness and metalness pictures, laid on the mesh's own texture coordinates) and its `Color` tint
//! draw the mesh in place of the part's `Material` and `TextureId`. `AlphaMode` says what the colour map's alpha does.
//!
//! The renderer draws it (`metallic_render`'s scene gives the part a material made from it); nothing else reads it.
//! It does nothing anywhere but directly inside a MeshPart.

use metallic_core::{ClassDesc, EnumDesc, PropMeta, Widget};

/// The pictures a map names.
const IMAGES: Widget = Widget::AssetId {
    extensions: &["png", "jpg", "jpeg"],
};

/// The group its properties are listed under.
const GROUP: &str = "Appearance";

static PROPS: &[PropMeta] = &[
    PropMeta::string("ColorMap", "")
        .visual()
        .widget(IMAGES)
        .group(GROUP)
        .order(0)
        .desc("The picture the mesh is coloured with, on its own texture coordinates, multiplied by Color."),
    PropMeta::string("NormalMap", "")
        .visual()
        .widget(IMAGES)
        .group(GROUP)
        .order(1)
        .desc("A tangent-space normal map (the OpenGL convention: green up), which bends the light across the mesh."),
    PropMeta::string("RoughnessMap", "")
        .visual()
        .widget(IMAGES)
        .group(GROUP)
        .order(2)
        .desc("How rough the surface is, from black (smooth, mirror-like) to white (rough), in the picture's red."),
    PropMeta::string("MetalnessMap", "")
        .visual()
        .widget(IMAGES)
        .group(GROUP)
        .order(3)
        .desc("How metallic the surface is, from black (not at all) to white (metal), in the picture's red."),
    PropMeta::color3("Color", 1.0, 1.0, 1.0)
        .visual()
        .group(GROUP)
        .order(4)
        .desc("Tints the colour map: white leaves it as it is."),
    PropMeta::enumeration("AlphaMode", "AlphaMode", "Overlay")
        .visual()
        .group(GROUP)
        .order(5)
        .desc(
            "What the colour map's alpha does. Overlay shows the part's Color where the map is see-through; \
             Transparency makes the mesh see-through there; TintMask tints only where the map is opaque.",
        ),
];

/// The enum `AlphaMode` reads.
pub static ENUMS: &[EnumDesc] = &[EnumDesc::new("AlphaMode", &["Overlay", "Transparency", "TintMask"])
    .describe("What a SurfaceAppearance's colour map's alpha does.")];

/// A MeshPart's own look: maps on the mesh's coordinates in place of its material.
pub static CLASS: ClassDesc = ClassDesc::new("SurfaceAppearance", Some("Instance"))
    .describe(
        "Gives the MeshPart it is in a look of its own: colour, normal, roughness and metalness maps on the mesh's \
         texture coordinates, in place of the part's Material and TextureId.",
    )
    .category("Parts")
    .creatable()
    .props(PROPS);

#[cfg(test)]
mod tests {
    use metallic_core::Value;

    #[test]
    fn a_surface_appearance_goes_inside_a_mesh_part_with_its_maps() {
        let mut dm = crate::new_game().unwrap();
        let class = dm.registry().class_id("SurfaceAppearance").unwrap();
        let surface = dm.create(class).unwrap();
        assert_eq!(dm.get_by_name(surface, "ColorMap"), Ok(Value::String("".into())));
        assert_eq!(
            dm.get_by_name(surface, "Color"),
            Ok(Value::Color3(metallic_core::Color3::WHITE))
        );
        let mode = dm.get_by_name(surface, "AlphaMode").unwrap();
        assert!(matches!(mode, Value::Enum(item) if item.name() == "Overlay"));
        let part = dm.create(dm.registry().class_id("MeshPart").unwrap()).unwrap();
        dm.set_parent(surface, Some(part)).unwrap();
        assert_eq!(dm.parent(surface).unwrap(), Some(part));
    }
}
