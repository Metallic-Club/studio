//! `MeshPart`: a part drawn as an imported mesh (glTF or GLB) with a base-colour texture (PNG or JPEG).
//!
//! Everything about where it is and how it looks comes from `BasePart`; `MeshPart` adds `MeshId` and `TextureId`, the
//! ids of the project's assets to draw (32 hex digits, from the project's asset index, ADR 0006), and the
//! `AssetMesh` component, which the renderer implements by drawing the mesh stretched so its bounds fill `Size`.
//! Loading the assets is the engine's job: until they are loaded, or when they are missing, the part draws nothing.

use metallic_core::{ClassDesc, ComponentDesc, PropMeta};

/// The component that draws a part as a mesh asset. Declared here; the renderer implements it.
pub static COMPONENT: ComponentDesc = ComponentDesc::new("AssetMesh")
    .describe(
        "Draws the part as the mesh asset its MeshId names, textured with TextureId, stretched so the mesh's bounds \
         fill its Size.",
    )
    .requires(&["Transform"]);

static PROPS: &[PropMeta] = &[
    PropMeta::string("MeshId", "")
        .visual()
        .group("MeshPart")
        .order(0)
        .desc(
            "The id of the project's mesh asset (a glTF or GLB model) the part is drawn as. Its bounds are \
             stretched to the part's Size. Empty draws nothing.",
        ),
    PropMeta::string("TextureId", "")
        .visual()
        .group("MeshPart")
        .order(1)
        .desc(
            "The id of the project's image asset (PNG or JPEG) the mesh is coloured with, multiplied by Color. \
             Empty uses Color alone.",
        ),
];

/// A part drawn as an imported mesh.
pub static CLASS: ClassDesc = ClassDesc::new("MeshPart", Some("BasePart"))
    .describe("A part drawn as an imported mesh with a texture, which you size, colour and place like any part.")
    .category("Parts")
    .creatable()
    .components(&["AssetMesh"])
    .props(PROPS);

#[cfg(test)]
mod tests {
    use metallic_core::Value;

    use crate::shared_registry;
    use crate::testing::{game, new_in, workspace};

    #[test]
    fn mesh_part_has_an_asset_mesh_and_a_transform() {
        let registry = shared_registry().unwrap();
        let info = registry.class(registry.class_id("MeshPart").unwrap()).unwrap();
        assert!(info.has_component("AssetMesh"));
        assert!(info.has_component("Transform"));
        assert!(!info.has_component("PrimitiveMesh"));
        assert!(info.creatable());
    }

    #[test]
    fn ids_start_empty_and_take_any_text() {
        let mut dm = game();
        let ws = workspace(&dm);
        let part = new_in(&mut dm, "MeshPart", ws, "Statue");
        assert_eq!(dm.get_by_name(part, "MeshId"), Ok(Value::String("".into())));
        assert_eq!(dm.get_by_name(part, "TextureId"), Ok(Value::String("".into())));
        let id = Value::String("0123456789abcdef0123456789abcdef".into());
        dm.set_by_name(part, "MeshId", id.clone()).unwrap();
        assert_eq!(dm.get_by_name(part, "MeshId"), Ok(id));
        // Inherited from BasePart.
        assert!(dm.get_by_name(part, "Size").is_ok());
    }
}
