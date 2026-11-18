//! SurfaceAppearance (#807): a MeshPart with one inside draws with a material made from it, in place of its own
//! `Material`.
//!
//! The scene gives such a part a material key of its own that says everything the material is made of:
//! [`SURFACE_KEY_PREFIX`] then `color=`, `normal=`, `roughness=` and `metalness=` (the maps' asset ids, empty for
//! none), `tint=` (the `Color`'s three channels) and `alpha=` (the `AlphaMode`), separated by `;`. The engine's
//! material loader reads the key back (`metallic_project::material::Material::from_surface_key`) and loads the maps
//! as it loads a material's, so a change of the appearance is a new key and is drawn at once. The scene watches the
//! appearances: one that changes, appears or goes has its part packed again.

use metallic_core::{ClassId, DataModel, Id, PropId, Registry};

/// What starts the material key of a part drawn with a SurfaceAppearance.
pub const SURFACE_KEY_PREFIX: &str = "surface:";

/// The SurfaceAppearance class and its properties, found once in the registry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfaceProps {
    pub(crate) class: ClassId,
    color_map: PropId,
    normal_map: PropId,
    roughness_map: PropId,
    metalness_map: PropId,
    color: PropId,
    alpha: PropId,
    /// MeshPart: only its instances (and its subclasses') take one.
    mesh_part: ClassId,
}

impl SurfaceProps {
    /// The class and its properties in `registry`, if it has them.
    pub(crate) fn of(registry: &Registry) -> Option<Self> {
        let class = registry.class_id("SurfaceAppearance")?;
        let prop = |name: &str| registry.prop_id(class, name);
        Some(Self {
            class,
            color_map: prop("ColorMap")?,
            normal_map: prop("NormalMap")?,
            roughness_map: prop("RoughnessMap")?,
            metalness_map: prop("MetalnessMap")?,
            color: prop("Color")?,
            alpha: prop("AlphaMode")?,
            mesh_part: registry.class_id("MeshPart")?,
        })
    }

    /// The material key the part `id` draws with: its first SurfaceAppearance's, when it is a MeshPart with one.
    pub(crate) fn key_of(&self, dm: &DataModel, id: Id) -> Option<String> {
        let class = dm.class_of(id).ok()?;
        if !dm.registry().is_a(class, self.mesh_part) {
            return None;
        }
        let surface = dm
            .children(id)
            .ok()?
            .find(|&child| dm.class_of(child).is_ok_and(|class| class == self.class))?;
        let text = |prop: PropId| dm.string(surface, prop).unwrap_or_default().to_owned();
        let tint = dm.color3(surface, self.color).unwrap_or(metallic_core::Color3::WHITE);
        let alpha = dm
            .enum_item(surface, self.alpha)
            .map_or("Overlay", |item| item.name())
            .to_owned();
        Some(format!(
            "{SURFACE_KEY_PREFIX}color={};normal={};roughness={};metalness={};tint={},{},{};alpha={alpha}",
            text(self.color_map),
            text(self.normal_map),
            text(self.roughness_map),
            text(self.metalness_map),
            tint.r,
            tint.g,
            tint.b,
        ))
    }
}

#[cfg(test)]
mod tests {
    use metallic_core::{Color3, Value};

    use super::*;

    #[test]
    fn a_mesh_part_with_a_surface_appearance_has_a_key_that_says_it_all() {
        let mut dm = metallic_classes::new_game().unwrap();
        let props = SurfaceProps::of(dm.registry()).unwrap();
        let part = dm.create(dm.registry().class_id("MeshPart").unwrap()).unwrap();
        assert_eq!(props.key_of(&dm, part), None, "without one");
        let surface = dm.create(props.class).unwrap();
        dm.set_parent(surface, Some(part)).unwrap();
        dm.set_by_name(
            surface,
            "ColorMap",
            Value::String("0000000000000000000000000000000a".into()),
        )
        .unwrap();
        dm.set_by_name(surface, "Color", Value::Color3(Color3::new(1.0, 0.5, 0.25)))
            .unwrap();
        assert_eq!(
            props.key_of(&dm, part).as_deref(),
            Some(
                "surface:color=0000000000000000000000000000000a;normal=;roughness=;metalness=;tint=1,0.5,0.25;\
                 alpha=Overlay"
            )
        );
        // A plain part never takes one.
        let block = dm.create(dm.registry().class_id("Part").unwrap()).unwrap();
        let other = dm.create(props.class).unwrap();
        dm.set_parent(other, Some(block)).unwrap();
        assert_eq!(props.key_of(&dm, block), None);
    }
}
